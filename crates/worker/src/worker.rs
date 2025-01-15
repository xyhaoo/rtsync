use std::collections::HashMap;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use nix::unistd::Pid;
use rocket::{post, routes, Build, Rocket, State};
use reqwest::Client;
use rocket::serde::json::Json;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{Mutex, MutexGuard, RwLock, Semaphore};
use internal::msg::{CmdVerb, MirrorSchedule, MirrorSchedules, MirrorStatus, WorkerCmd, WorkerStatus};
use internal::util::{create_http_client, get_json, post_json};
use libc::getpid;
use nix::sys::signal::{kill, Signal};
use rocket::http::Status;
use rocket::serde::Serialize;
use scopeguard::defer;
use skiplist::SkipMap;
use tokio::time;
use internal::status::SyncStatus;
use crate::common::DEFAULT_MAX_RETRY;
use crate::config::{Config, MirrorConfig};
use crate::config_diff::{diff_mirror_config, Diff};
use crate::job::{JobCtrlAction, JobMessage, MirrorJob, JobState};
use crate::provider::new_mirror_provider;
use crate::schedule::{JobScheduleInfo, ScheduleQueue};

#[derive(Serialize)]
pub(crate) struct Response {
    msg: String,
}

#[derive(Clone)]
struct WorkerManager{
    l: Arc<Mutex<()>>,
    jobs: Arc<RwLock<HashMap<String, MirrorJob>>>,
    manager_chan: (Sender<JobMessage>, Arc<Mutex<Receiver<JobMessage>>>),
    semaphore: Arc<Semaphore>,
    schedule: ScheduleQueue,
}
impl WorkerManager{
    fn new(manager_chan_buffer: usize, semaphore_permits: usize)->Self{
        let (tx, rx) = channel(manager_chan_buffer);
        let rx = Arc::new(Mutex::new(rx));
        WorkerManager{
            l: Arc::new(Mutex::new(())),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            manager_chan: (tx, rx),
            semaphore: Arc::new(Semaphore::new(semaphore_permits)),
            schedule: ScheduleQueue::new(),
        }
    }

    async fn disable_job(&self,
                         job: &MirrorJob,
                         list_lock: &mut MutexGuard<'_, SkipMap<DateTime<Utc>, MirrorJob>>,
                         jobs_lock: &mut MutexGuard<'_, HashMap<String, bool>>)
    {
        self.schedule.remove(&job.name(), list_lock, jobs_lock);
        if job.state() != JobState::Disabled{
            job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
            let mut disabled_lock = job.disabled_rx.lock().await;
            let disabled_rx = disabled_lock.as_mut().unwrap();
            let _ = disabled_rx.recv().await;
        }
    }
}

// Worker是rtsync worker的一个实例
#[derive(Clone)]
pub struct Worker {
    cfg: Arc<RwLock<Config>>,
    exit: (Option<Sender<()>>, Arc<Mutex<Receiver<()>>>),
    http_engine: Arc<Mutex<Option<Rocket<Build>>>>,
    http_client: Client,
    worker_manager: WorkerManager,  // 将要被manage到rocket的数据
}


impl Worker {
    pub async fn new(mut cfg: Config) -> Option<Self> {
        if cfg.global.retry.is_none() || cfg.global.retry.is_some_and(|retry| retry == 0){
            cfg.global.retry = Some(DEFAULT_MAX_RETRY);
        }
        let mut http_client = Client::new();
        let concurrent = cfg.global.concurrent.unwrap();

        let ca_cert = cfg.manager.ca_cert.as_deref().filter(|s| !s.is_empty());
        let client_result = match ca_cert {
            Some(cert) => create_http_client(Some(cert)),
            None => create_http_client(None),
        };
        match client_result {
            Ok(client) => {
                http_client = client;
            }
            Err(e) => {
                let err = format!("初始化http 客户端失败: {}", e);
                log::error!("{}", err);
                return None;
            }
        }
        
        let (tx, rx) = channel(1);
        let tx = Some(tx);
        let worker_manager = WorkerManager::new(32, concurrent as usize);
        let w = Worker{
            cfg: Arc::new(RwLock::new(cfg)),
            exit: (tx, Arc::new(Mutex::new(rx))),
            http_engine: Arc::new(Mutex::new(Some(Self::make_http_server(worker_manager.clone())))),
            http_client,
            worker_manager,
        };
        w.init_jobs().await;
        Some(w)
    }

    pub async fn run(&self) {
        self.register_worker().await;
        let rocket = self.run_http_server().await;
        // 现在Worker里的服务器被取出，放入异步运行时运行， self.http_engine == None
        tokio::spawn(async move {
            rocket.launch().await.unwrap();
        });
        self.run_schedule().await;
    }

    pub async fn halt(&mut self) {
        let lock  = self.worker_manager.l.lock().await;
        info!("停止所有镜像任务");
        for job in self.worker_manager.jobs.read().await.values() {
            if job.state() != JobState::Disabled{
                job.ctrl_chan_tx.send(JobCtrlAction::Halt).await.unwrap();
            }
        }
        info!("所有镜像任务已停止");
        drop(lock);
        self.exit.0 = None;
    }

    // ReloadMirrorConfig refresh the providers and jobs
    // from new mirror configs
    // TODO: deleted job should be removed from manager-side mirror list
    pub async fn reload_mirror_config(&self, new_mirrors: Vec<MirrorConfig>) {
        let lock = self.worker_manager.l.lock().await;
        defer!{drop(lock);}
        info!("重载镜像任务配置");

        let old_mirrors = self.cfg.read().await.mirrors.clone();
        let difference = diff_mirror_config(&old_mirrors, &new_mirrors);
        drop(old_mirrors);

        // 首先处理删除和修改
        for op in difference.iter() {
            if op.diff_op == Diff::Add{
                continue;
            }
            let name = op.mir_cfg.name.as_ref().unwrap();
            let mut jobs_lock = self.worker_manager.jobs.write().await;
            match jobs_lock.get_mut(name) {
                None => {
                    warn!("Job {} not found", name);
                    continue;
                },
                Some(job) => {
                    let mut list_lock = self.worker_manager.schedule.list.lock().await;
                    let mut job_lock = self.worker_manager.schedule.jobs.lock().await;
                    match op.diff_op {
                        Diff::Delete => {
                            self.disable_job(job, &mut list_lock, &mut job_lock).await;
                            drop((list_lock, job_lock));
                            jobs_lock.remove(name);
                            info!("Deleted job {}", name);
                        },
                        Diff::Modify => {
                            let job_state = job.state();
                            self.disable_job(job, &mut list_lock, &mut job_lock).await;
                            drop((list_lock, job_lock));
                            // set new provider
                            let provider = new_mirror_provider(op.mir_cfg.clone(), self.cfg.read().await.clone()).await;
                            if let Err(e) = job.set_provider(provider){
                                error!("Error setting job provider of {}: {}", name, e);
                                continue;
                            }

                            // re-schedule job according to its previous state
                            match job_state {
                                JobState::Disabled => {
                                    job.set_state(JobState::Disabled)
                                },
                                JobState::Paused => {
                                    job.set_state(JobState::Paused);
                                    let job_clone = job.clone();
                                    let manager_chan = self.worker_manager.manager_chan.0.clone();
                                    let semaphore = Arc::clone(&self.worker_manager.semaphore);
                                    tokio::spawn(async move {
                                        job_clone.run(manager_chan, semaphore).await.unwrap();
                                    });
                                },
                                _ => {
                                    job.set_state(JobState::None);
                                    let job_clone = job.clone();
                                    let manager_chan = self.worker_manager.manager_chan.0.clone();
                                    let semaphore = Arc::clone(&self.worker_manager.semaphore);
                                    tokio::spawn(async move {
                                        job_clone.run(manager_chan, semaphore).await.unwrap();
                                    });
                                    self.worker_manager.schedule.add_job(Utc::now(), job.clone()).await;
                                }
                            }
                            info!("重载任务 {}", name);
                        },
                        _ => { drop((list_lock, job_lock)); }
                    }
                }
            }
        } // for

        // for added new jobs, just start new jobs
        for op in difference {
            if op.diff_op != Diff::Add{
                continue;
            }
            let provider = new_mirror_provider(op.mir_cfg, self.cfg.read().await.clone()).await;
            let job = MirrorJob::new(provider);
            let job_name = job.name();
            { self.worker_manager.jobs.write().await.insert(job_name.clone(), job.clone()); }

            job.set_state(JobState::None);
            let job_clone = job.clone();
            let manager_chan = self.worker_manager.manager_chan.0.clone();
            let semaphore = Arc::clone(&self.worker_manager.semaphore);
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await.unwrap();
            });
            self.worker_manager.schedule.add_job(Utc::now(), job).await;
            info!("新任务 {}", job_name);
        }

        { self.cfg.write().await.mirrors = new_mirrors; }
    }

    async fn init_jobs(&self) {
        let cfg_lock = self.cfg.read().await;
        let cfg = cfg_lock.clone();
        for mirror in cfg_lock.mirrors.iter(){
            let provider = new_mirror_provider(mirror.clone(), cfg.clone()).await;
            self.worker_manager.jobs.write().await.insert(provider.name(), MirrorJob::new(provider));
        }
        drop((cfg_lock, cfg));
    }

    async fn disable_job(&self,
                         job: &MirrorJob,
                         list_lock: &mut MutexGuard<'_, SkipMap<DateTime<Utc>, MirrorJob>>,
                         jobs_lock: &mut MutexGuard<'_, HashMap<String, bool>>)
    {
        self.worker_manager.disable_job(job, list_lock, jobs_lock).await;
    }

    // Ctrl server receives commands from the manager
    fn make_http_server(worker_manager: WorkerManager) -> Rocket<Build> {
        let s = Rocket::build()
            .manage(worker_manager)    // Arc
            .mount("/" ,routes![handle_cmd_from_manager]);
        s
    }

    async fn run_http_server(&self) -> Rocket<Build> {
        let mut rocket = self.http_engine.lock().await.take().unwrap();
        let mut figment = rocket.figment().clone();
        let cfg_lock = self.cfg.read().await;
        if let (Some(addr), Some(port)) = (cfg_lock.server.listen_addr.as_ref(), cfg_lock.server.listen_port.as_ref()){
            if !addr.is_empty(){
                figment = figment
                    .merge((rocket::Config::PORT, port))
                    .merge((rocket::Config::ADDRESS, addr))
            }
        }
        if let (Some(cert), Some(key)) = (cfg_lock.server.ssl_cert.as_ref(), cfg_lock.server.ssl_key.as_ref()){
            if !cert.is_empty() && !key.is_empty(){
                figment = figment
                    .merge(("tls.certs", cert))
                    .merge(("tls.key", key));
            }
        }
        rocket = rocket.configure(figment);
        rocket
    }

    async fn run_schedule(&self) {
        let lock = self.worker_manager.l.lock().await;
        let mirror_list = self.fetch_job_status().await;
        let mut unset = HashMap::new();
        for name in self.worker_manager.jobs.read().await.keys() {
            unset.insert(name.clone(), true);
        }

        // 获取存储在manager服务器数据库中的镜像状态列表
        // 为其分配新的协程来准备启动该镜像的同步（未开始）
        // 设置其同步开始时间后，放入worker的调度列表
        // 如果这个镜像是被禁用的状态，就跳过它
        for m in mirror_list{
            if let Some(job) = self.worker_manager.jobs.read().await.get(&m.name){
                unset.remove(&m.name);
                match m.status {
                    SyncStatus::Disabled => {
                        job.set_state(JobState::Disabled);
                        continue;
                    },
                    SyncStatus::Paused => {
                        job.set_state(JobState::Paused);
                        let manager_chan = self.worker_manager.manager_chan.0.clone();
                        let semaphore = Arc::clone(&self.worker_manager.semaphore);
                        let job_clone = job.clone();
                        tokio::spawn(async move {
                            job_clone.run(manager_chan, semaphore).await.unwrap();
                        });
                        continue;
                    },
                    _ => {
                        job.set_state(JobState::None);
                        let manager_chan = self.worker_manager.manager_chan.0.clone();
                        let semaphore = Arc::clone(&self.worker_manager.semaphore);
                        let job_clone = job.clone();
                        tokio::spawn(async move {
                            job_clone.run(manager_chan, semaphore).await.unwrap();
                        });
                        let stime = m.last_update + job.provider.interval();
                        debug!("安排了 job {} @{}", job.name(), stime.format("%d-%m-%Y %H:%M:%S"));
                        self.worker_manager.schedule.add_job(stime, job.clone()).await;
                    }
                }
            }
        }

        // 可能会启动一些新添加的同步任务，原来不存在于mirror_list
        // 它们也会被安排进worker的调度列表
        for name in unset.keys() {
            let job = self.worker_manager.jobs.read().await.get(name).cloned().unwrap();
            job.set_state(JobState::None);
            let manager_chan = self.worker_manager.manager_chan.0.clone();
            let semaphore = Arc::clone(&self.worker_manager.semaphore);
            let job_clone = job.clone();
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await.unwrap();
            });
            self.worker_manager.schedule.add_job(Utc::now(), job.clone()).await;
        }
        drop(lock);

        let sched_info = self.worker_manager.schedule.get_jobs().await;
        info!("sched_info: {:?}", sched_info);
        self.update_sched_info(sched_info).await;

        let mut interval = time::interval(time::Duration::from_secs(5));
        let mut manager_chan_rx = self.worker_manager.manager_chan.1.lock().await;
        let mut exit_lock = self.exit.1.lock().await;
        loop {
            tokio::select! {
                Some(job_msg) = manager_chan_rx.recv() => {
			        // 获取不断在更新的任务状态
                    // 向manager服务器报告状态
                    let lock = self.worker_manager.l.lock().await;
                    let job = self.worker_manager.jobs.read().await.get(&job_msg.name).cloned();
                    drop(lock);
                    if job.is_none() {
                        warn!("任务 {} 未找到", job_msg.name);
                        continue;
                    }
                    let job = job.unwrap();
                    let job_state = job.state();

                    if (job_state != JobState::Ready) && (job_state != JobState::Halting){
                        info!("任务 {} 不是就绪状态，直到其状态被改变，不再为其安排新的同步时间", job_msg.name);
                        continue;
                    }
                    // 任务的同步状态只有在它运行时才有意义。
                    // 如果任务被暂停或禁用，则会发出同步失败信号，需要忽略该信号
                    self.update_status(&job, &job_msg).await;

                    // 只有同步任务成功或失败时发送的schedule信号（都为true）才会触发该任务的下一次同步调度
                    if job_msg.schedule {
                        let schedule_time = Utc::now() + job.provider.interval();
                        info!("{} 的下一次同步时间: {}",
                            job.name(),
                            schedule_time.format("%Y-%m-%d %H:%M:%S"));
                        self.worker_manager.schedule.add_job(schedule_time, job.clone()).await;
                    }
                    let sched_info = self.worker_manager.schedule.get_jobs().await;
                    self.update_sched_info(sched_info).await;
                },

                _ = interval.tick() => {
			        // 每隔五秒检查调度队列
                    if let Some(job) = self.worker_manager.schedule.pop().await{
                        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
                    }
                },
                None = exit_lock.recv() => {
			        // flush status update messages
                    let lock = self.worker_manager.l.lock().await;
                    defer!{drop(lock);}
                    loop {
                        if let Ok(job_msg) = manager_chan_rx.try_recv() {
                            debug!("status update from {}", job_msg.name);
                            match self.worker_manager.jobs.read().await.get(&job_msg.name){
                                None => {continue;},
                                Some(job) => {
                                    if job_msg.status == SyncStatus::Failed || job_msg.status == SyncStatus::Success{
                                        self.update_status(job, &job_msg).await;
                                    }
                                }
                            }
                        }else{
                            return;
                        }
                    }
                }
            }
        }

    }

    async fn name(&self) -> String {
        self.cfg.read().await.global.name.clone().unwrap()
    }

    // url返回worker的http服务器的url
    async fn url(&self) -> String {
        let cfg_lock = self.cfg.read().await;
        let mut proto = "https";
        if cfg_lock.server.ssl_cert.is_none() || cfg_lock.server.ssl_key.is_none(){
            proto = "http";
        }
        format!("{}://{}:{}/", proto, cfg_lock.server.hostname.clone().unwrap(),
                cfg_lock.server.listen_port.clone().unwrap())
    }

    async fn register_worker(&self) {
        let msg = WorkerStatus{
            id: self.name().await,
            url: self.url().await,
            ..WorkerStatus::default()
        };

        let cfg_lock = self.cfg.read().await;
        for root in cfg_lock.manager.api_base_list(){
            let url = format!("{}/workers", root);
            debug!("向 manager url: {} 注册 worker", url);
            let mut retry = 10;
            while retry > 0 {
                if post_json(&url, &msg, Some(self.http_client.clone())).await.is_err() {
                    error!("注册worker失败");
                    retry -= 1;
                    if retry > 0 {
                        tokio::time::sleep(time::Duration::from_secs(1)).await;
                        info!("重试注册... ({})", retry);
                    }
                }else{
                    break;
                }
            }
        }
    }

    async fn update_status(&self, job: &MirrorJob, job_msg: &JobMessage) {
        let mut smsg = MirrorStatus{
            name: job_msg.name.clone(),
            worker: self.cfg.read().await.global.name.clone().unwrap(),
            is_master: job.provider.is_master(),
            status: job_msg.status,
            upstream: job.provider.upstream(),
            size: "unknown".to_string(),
            error_msg: job_msg.msg.clone(),
            ..MirrorStatus::default()
        };
        
        //某些提供商（例如rsync）可能知道镜像的大小
        // 所以我们在这里报告给Manager
        let size_lock = job.size.lock().await;
        if size_lock.len() != 0 {
            smsg.size = size_lock.clone();
        }
        drop(size_lock);

        let name = self.name().await;
        let cfg_lock = self.cfg.read().await;
        for root in cfg_lock.manager.api_base_list(){
            let url = format!("{}/workers/{}/jobs/{}", root, name, job_msg.name);
            debug!("reporting on manager url: {}", url);
            if let Err(e) = post_json(&url, &smsg, Some(self.http_client.clone())).await{
                error!("Failed to update mirror({}) status: {}", job_msg.name, e);
            }
        }
    }

    async fn update_sched_info(&self, sched_info: Vec<JobScheduleInfo>) {
        let mut s: Vec<MirrorSchedule> = vec![];
        for sched in sched_info {
            s.push(MirrorSchedule{
                mirror_name: sched.job_name,
                next_schedule: sched.next_scheduled,
                ..MirrorSchedule::default()
            });
        }
        let msg = MirrorSchedules{
            schedules: s,
        };

        let name = self.name().await;
        let cfg_lock = self.cfg.read().await;
        for root in cfg_lock.manager.api_base_list(){
            let url = format!("{}/workers/{}/schedules", root, name);
            debug!("reporting on manager url: {}", url);
            if let Err(e) = post_json(&url, &msg, Some(self.http_client.clone())).await {
                error!("Failed to upload schedules: {}", e);
            }
        }
    }

    async fn fetch_job_status(&self) -> Vec<MirrorStatus> {
        let mut mirror_list = vec![];
        let name = self.name().await;
        let api_base = &self.cfg.read().await.manager.api_base_list()[0];

        let url = format!("{}/workers/{}/jobs", api_base, name);

        match get_json::<Vec<MirrorStatus>>(&url, Some(self.http_client.clone())).await{
            Ok(jobs) => {
                mirror_list = jobs;
            }
            Err(e) => {
                error!("获取任务状态失败: {}", e);
            }
        }
        mirror_list
    }
}


// 处理从manager服务器发送来的控制信号
#[post("/", format = "application/json", data = "<cmd>")]
async fn handle_cmd_from_manager(cmd: Json<WorkerCmd>, w: &State<WorkerManager>) 
    -> Result<Json<Response>, (Status, Json<Response>)> 
{
    let lock = w.l.lock().await;
    defer!{
        drop(lock);
    }
    let cmd = cmd.into_inner();
    info!("收到命令: {}", cmd);

    // 对于worker的命令
    if cmd.mirror_id.is_empty(){
        match cmd.cmd{
            CmdVerb::Reload => {
                // 给自身发送 SIGHUP， 用于重载config
                let pid = unsafe { getpid() };
                kill(Pid::from_raw(pid), Signal::SIGHUP).unwrap();
            },
            _ => {
                return Err((Status::NotAcceptable, Json(Response {msg: "无效的命令".to_string()})));
            }
        }
    }

    // 对于job的命令
    match w.jobs.read().await.get(&cmd.mirror_id){
        None => {
            Err((Status::NotFound, Json(Response {msg: format!("镜像 '{}' 未找到", cmd.mirror_id)})))
        }
        Some(job) => {
            // 不管收到什么信号，首先要刷新同步队列
            let mut list_lock = w.schedule.list.lock().await;
            let mut jobs_lock = w.schedule.jobs.lock().await;
            w.schedule.remove(&*job.name(), &mut list_lock, &mut jobs_lock);
            drop((list_lock, jobs_lock));

            // 如果收到的是开始同步信号，且这个job当前被禁用，先将其启动
            match cmd.cmd {
                CmdVerb::Start | CmdVerb::Restart => {
                    if job.state() == JobState::Disabled{
                        let job_clone = job.clone();
                        let manager_chan = w.manager_chan.0.clone();
                        let semaphore = Arc::clone(&w.semaphore);
                        tokio::spawn(async move {
                            job_clone.run(manager_chan, semaphore).await.unwrap();
                        });
                    }
                },
                _ => {},
            }
            match cmd.cmd {
                CmdVerb::Start => {
                    if cmd.options.get("force").is_some_and(|force|*force == true){
                        job.ctrl_chan_tx.send(JobCtrlAction::ForceStart).await.unwrap();
                    }else {
                        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
                    }
                },
                CmdVerb::Restart => {
                    job.ctrl_chan_tx.send(JobCtrlAction::Restart).await.unwrap();
                },
                CmdVerb::Stop => {
                    // 如果收到停止信号，而job当前是禁用状态，则忽略该信号
                    if job.state() != JobState::Disabled{
                        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();
                    }
                },
                CmdVerb::Disable => {
                    let mut list_lock = w.schedule.list.lock().await;
                    let mut jobs_lock = w.schedule.jobs.lock().await;
                    w.disable_job(&job, &mut list_lock, &mut jobs_lock).await;
                    drop((list_lock, jobs_lock));
                },
                CmdVerb::Ping => {
                    // empty
                },
                _ => {
                    return Err((Status::NotAcceptable, Json(Response {msg: "无效的命令".to_string()})));
                }
            }

            Ok(Json(Response {msg: "OK".to_string()}))
        }
    }

}
