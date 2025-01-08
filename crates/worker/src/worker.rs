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
    cfg: Arc<Mutex<Config>>,
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
        if let Some(ca_cert) = cfg.manager.ca_cert.as_ref(){
            if !ca_cert.is_empty(){
                match create_http_client(Some(ca_cert)){
                    Ok(client) => {
                        http_client = client;
                    }
                    Err(e) => {
                        error!("初始化http 客户端失败: {}", e);
                        return None;
                    }
                }
            }
        }
        let (tx, rx) = channel(1);
        let tx = Some(tx);
        let worker_manager = WorkerManager::new(32, concurrent as usize);
        let w = Worker{
            cfg: Arc::new(Mutex::new(cfg)),
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
        let jobs_lock = self.worker_manager.jobs.write().await;
        info!("Stopping all the jobs");
        for job in jobs_lock.values() {
            if job.state() != JobState::Disabled{
                job.ctrl_chan_tx.send(JobCtrlAction::Halt).await.unwrap();
            }
        }
        info!("All the jobs are stopped");
        drop(jobs_lock);
        self.exit.0 = None;
    }

    // ReloadMirrorConfig refresh the providers and jobs
    // from new mirror configs
    // TODO: deleted job should be removed from manager-side mirror list
    pub async fn reload_mirror_config(&self, new_mirrors: Vec<MirrorConfig>) {
        let mut jobs_lock = self.worker_manager.jobs.write().await;
        info!("Reloading mirror configs");

        let old_mirrors = self.cfg.lock().await.mirrors.clone();
        let difference = diff_mirror_config(&old_mirrors, &new_mirrors);
        drop(old_mirrors);

        // first deal with deletion and modifications
        for op in difference.iter() {
            if op.diff_op == Diff::Add{
                continue;
            }
            let name = op.mir_cfg.name.as_ref().unwrap();
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
                            let provider = new_mirror_provider(op.mir_cfg.clone(), self.cfg.lock().await.clone()).await;
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
                            info!("Reloaded job {}", name);
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
            let provider = new_mirror_provider(op.mir_cfg, self.cfg.lock().await.clone()).await;
            let job = MirrorJob::new(provider);
            let job_name = job.name();
            jobs_lock.insert(job_name.clone(), job.clone());

            job.set_state(JobState::None);
            let job_clone = job.clone();
            let manager_chan = self.worker_manager.manager_chan.0.clone();
            let semaphore = Arc::clone(&self.worker_manager.semaphore);
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await.unwrap();
            });
            self.worker_manager.schedule.add_job(Utc::now(), job).await;
            info!("New job {}", job_name);
        }

        self.cfg.lock().await.mirrors = new_mirrors;
    }

    async fn init_jobs(&self) {
        let cfg_lock = self.cfg.lock().await;
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
            .mount("/" ,routes![post]);
        s
    }

    async fn run_http_server(&self) -> Rocket<Build> {
        let mut rocket = self.http_engine.lock().await.take().unwrap();
        let mut figment = rocket.figment().clone();
        let cfg_lock = self.cfg.lock().await;
        if let (Some(addr), Some(port)) = (cfg_lock.server.listen_addr.as_ref(), cfg_lock.server.listen_port.as_ref()){
            figment = figment
                .merge((rocket::Config::PORT, port))
                .merge((rocket::Config::ADDRESS, addr))
        }
        if let (Some(cert), Some(key)) = (cfg_lock.server.ssl_cert.as_ref(), cfg_lock.server.ssl_key.as_ref()){
            figment = figment
                .merge(("tls.certs", cert))
                .merge(("tls.key", key));
        }
        rocket = rocket.configure(figment);
        rocket
    }

    async fn run_schedule(&self) {
        let jobs_lock = self.worker_manager.jobs.write().await;
        let mirror_list = self.fetch_job_status().await;
        let mut unset = HashMap::new();
        for name in jobs_lock.keys() {
            unset.insert(name, true);
        }
        // Fetch mirror list stored in the manager
        // put it on the scheduled time
        // if it's disabled, ignore it
        for m in mirror_list{
            if let Some(job) = jobs_lock.get(&m.name){
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
                        let stime = m.last_update + job.provider.interval().await;
                        debug!("Scheduling job {} @{}", job.name(), stime.format("%d-%m-%Y %H:%M:%S"));
                        self.worker_manager.schedule.add_job(stime, job.clone()).await;
                    }
                }
            }
        }
        // some new jobs may be added
        // which does not exist in the
        // manager's mirror list
        for name in unset.keys() {
            let job = jobs_lock.get(*name).unwrap();
            job.set_state(JobState::None);
            let manager_chan = self.worker_manager.manager_chan.0.clone();
            let semaphore = Arc::clone(&self.worker_manager.semaphore);
            let job_clone = job.clone();
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await.unwrap();
            });
            self.worker_manager.schedule.add_job(Utc::now(), job.clone()).await;
        }

        drop(jobs_lock);

        let sched_info = self.worker_manager.schedule.get_jobs().await;
        self.update_sched_info(sched_info).await;

        let mut interval = time::interval(time::Duration::from_secs(5));
        let mut manager_chan_rx = self.worker_manager.manager_chan.1.lock().await;
        let mut exit_lock = self.exit.1.lock().await;
        let mut i = 1;
        loop {
            tokio::select! {
                Some(job_msg) = manager_chan_rx.recv() => {
			        // got status update from job
                    match self.worker_manager.jobs.read().await.get(&job_msg.name){
                        None => {
                            warn!("Job {} not found", job_msg.name);
                            continue;
                        },
                        Some(job) => {
                            let job_state = job.state();
                            println!("hehehe{i}");
                            i+=1;
                            if job_state != JobState::Ready && job_state != JobState::Halting{
                                info!("Job {} state is not ready, skip adding new schedule", job_msg.name);
                                continue;
                            }
                            // syncing status is only meaningful when job
                            // is running. If it's paused or disabled
                            // a sync failure signal would be emitted
                            // which needs to be ignored
                            self.update_status(job, &job_msg).await;

                            // only successful or the final failure msg
			                // can trigger scheduling
                            if job_msg.schedule {
                                let schedule_time = Utc::now() + job.provider.interval().await;
                                info!("Next scheduled time for {}: {}",
                                    job.name(),
                                    schedule_time.format("%d-%m-%Y %H:%M:%S"));
                                self.worker_manager.schedule.add_job(schedule_time, job.clone()).await;
                            }

                            let sched_info = self.worker_manager.schedule.get_jobs().await;
                            self.update_sched_info(sched_info).await;
                        }
                    }
                },

                _ = interval.tick() => {
			        // check schedule every 5 seconds
                    if let Some(job) = self.worker_manager.schedule.pop().await{
                        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
                    }
                },
                None = exit_lock.recv() => {
			        // flush status update messages
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
        self.cfg.lock().await.global.name.clone().unwrap()
    }

    // url返回worker的http服务器的url
    async fn url(&self) -> String {
        let cfg_lock = self.cfg.lock().await;
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

        let cfg_lock = self.cfg.lock().await;
        for root in cfg_lock.manager.api_base_list(){
            let url = format!("{}/workers", root);
            debug!("register on manager url: {}", url);
            let mut retry = 10;
            while retry > 0 {
                if let Err(e) = post_json(&url, &msg, Some(self.http_client.clone())).await{
                    error!("Failed to register worker");
                    retry -= 1;
                    if retry > 0 {
                        tokio::time::sleep(time::Duration::from_secs(1)).await;
                        info!("Retrying... ({})", retry);
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
            worker: self.cfg.lock().await.global.name.clone().unwrap(),
            is_master: job.provider.is_master(),
            status: job_msg.status,
            upstream: job.provider.upstream(),
            size: "unknown".to_string(),
            error_msg: job_msg.msg.clone(),
            ..MirrorStatus::default()
        };

        println!("debug: {:?}", smsg);

        //某些提供商（例如rsync）可能知道镜像的大小
        // 所以我们在这里报告给Manager
        let size_lock = job.size.lock().await;
        if size_lock.len() != 0 {
            smsg.size = size_lock.clone();
        }
        drop(size_lock);

        let name = self.name().await;
        let cfg_lock = self.cfg.lock().await;
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
        let cfg_lock = self.cfg.lock().await;
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
        let api_base = &self.cfg.lock().await.manager.api_base_list()[0];

        let url = format!("{}/workers/{}/jobs", api_base, name);

        match get_json::<Vec<MirrorStatus>>(&url, Some(self.http_client.clone())).await{
            Ok(jobs) => {
                mirror_list = jobs;
            }
            Err(e) => {
                error!("Failed to fetch job status: {}", e);
            }
        }
        mirror_list
    }
}


#[post("/", format = "application/json", data = "<cmd>")]
async fn post(cmd: Json<WorkerCmd>, w: &State<WorkerManager>) -> (Status, Json<Response>) {
    let mut list_lock = w.schedule.list.lock().await;
    let mut jobs_lock = w.schedule.jobs.lock().await;


    let cmd = cmd.into_inner();
    info!("Received command: {}", cmd);
    if cmd.mirror_id.is_empty(){
        // worker-level commands
        match cmd.cmd{
            CmdVerb::Reload => {
                // send myself a SIGHUP
                let pid = unsafe { getpid() };
                kill(Pid::from_raw(pid), Signal::SIGHUP).unwrap();
            },
            _ => {
                return (Status::BadRequest, Json(Response {msg: "Invalid Command".to_string()}));
            }
        }
    }

    // job level commands
    // FIXME: client发送post请求后等待结果，然而请求的处理过程因为等待w.jobs.lock()而阻塞，超过了client等待结果返回的时间
    // FIXME: 现在把锁改为RwLock，避免死锁，但因每个函数上锁种类不同，可能会导致其他问题
    match w.jobs.read().await.get(&cmd.mirror_id){
        None => {
            (Status::NotFound, Json(Response {msg: format!("Mirror '{}' not found", cmd.mirror_id)}))
        }
        Some(job) => {
            // No matter what command, the existing job
            // schedule should be flushed
            w.schedule.remove(&*job.name(), &mut list_lock, &mut jobs_lock);

            // if job disabled, start them first
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
                    // if job is disabled, no goroutine would be there
                    // receiving this signal
                    if job.state() != JobState::Disabled{
                        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();
                    }
                },
                CmdVerb::Disable => {
                    w.disable_job(&job, &mut list_lock, &mut jobs_lock).await;
                },
                CmdVerb::Ping => {
                    // empty
                },
                _ => {
                    return (Status::NotAcceptable, Json(Response {msg: "Invalid Command".to_string()}));
                }
            }

            (Status::Ok, Json(Response {msg: "Ok".to_string()}))
        }
    }

}
