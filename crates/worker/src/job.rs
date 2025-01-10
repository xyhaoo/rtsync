use std::fmt::{Display, Formatter};
use anyhow::{anyhow, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio::sync::Mutex;
use log::{debug, error, info, warn};
use crate::common::Empty;
use crate::hooks::JobHook;
use crate::provider::MirrorProvider;
use internal::status::SyncStatus;
use scopeguard::guard;
use tokio::sync::mpsc::{Receiver, Sender};
use crate::context::Context;
// 这个文件描述一个mirror job的工作流

// 控制动作枚举
#[derive(Debug, Clone)]
pub(crate) enum JobCtrlAction {
    Start,
    Stop,      // 停止同步但保持job
    Disable,   // 禁用job(停止goroutine)
    Restart,   // 重启同步
    Ping,      // 确保goroutine存活
    Halt,      // worker停止
    ForceStart, // 忽略并发限制
}

// Job消息结构
#[derive(Debug)]
pub(crate) struct JobMessage {
    pub(crate) status: SyncStatus,
    pub(crate) name: String,
    pub(crate) msg: String,
    // 是否进行下次同步，将会作为信号发送给worker来安排该任务的调度时间
    // 任务同步成功（且未被其他信号影响）或失败时为true，正在同步时为false
    pub(crate) schedule: bool,  
}

// Job状态枚举
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum JobState {
    None = 0,      // 空状态
    Ready = 1,     // 准备运行,可调度
    Paused = 2,    // 被jobStop暂停
    Disabled = 3,  // 被jobDisable禁用
    Halting = 4,   // worker正在停止
}

impl Display for JobState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "JobState::{}", self)
    }
}

impl JobState {
    fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(JobState::None),
            1 => Some(JobState::Ready),
            2 => Some(JobState::Paused),
            3 => Some(JobState::Disabled),
            4 => Some(JobState::Halting),
            _ => None
        }
    }
}

enum HookAction {
    PreJob,
    PreExec,
    PostExec,
    PostSuccess,
    PostFail,
}

async fn send_err(e: anyhow::Error, 
                  hook_name: &str, 
                  provider_name: &str, 
                  manager_chan: &Sender<JobMessage>) -> Result<()>
{
    error!("在 {} 执行 {} hooks时失败：{}", provider_name, hook_name, e);
    manager_chan.send(JobMessage {
        status: SyncStatus::Failed,
        name: provider_name.parse()?,
        msg: format!("在执行 hook {} 时失败: {}", hook_name, e),
        schedule: true,
    }).await?;
    Err(anyhow!(e))
}

// Mirror Job结构
#[derive(Clone, Debug)]
pub struct MirrorJob {
    pub(crate) provider: Arc<Box<dyn MirrorProvider>>,
    pub(crate) ctrl_chan_tx: Sender<JobCtrlAction>,
    ctrl_chan_rx: Arc<Mutex<Receiver<JobCtrlAction>>>,
    disabled_tx: Arc<Mutex<Option<Sender<Empty>>>>,
    pub(crate) disabled_rx: Arc<Mutex<Option<Receiver<Empty>>>>,
    state: Arc<AtomicU32>,
    pub(crate) size: Arc<Mutex<String>>,
}
impl PartialEq for MirrorJob {
    fn eq(&self, other: &Self) -> bool {
        self.provider.name() == other.provider.name()
    }
}

impl MirrorJob {
    pub fn new(provider: Box<dyn MirrorProvider>) -> Self {
        let (tx, rx) = mpsc::channel(1);
        Self {
            provider: Arc::new(provider),
            ctrl_chan_tx: tx,
            ctrl_chan_rx: Arc::new(Mutex::new(rx)),
            disabled_tx: Arc::new(Mutex::new(None)),
            disabled_rx: Arc::new(Mutex::new(None)),
            state: Arc::new(AtomicU32::new(JobState::None as u32)),
            size: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn name(&self) -> String {
        self.provider.name()
    }

    pub fn state(&self) -> JobState {
        let state = self.state.load(Ordering::SeqCst);
        match state {
            0 => JobState::None,
            1 => JobState::Ready,
            2 => JobState::Paused,
            3 => JobState::Disabled,
            4 => JobState::Halting,
            _ => unreachable!()
        }
    }

    pub fn set_state(&self, state: JobState) {
        self.state.store(state as u32, Ordering::SeqCst);
    }

    pub fn set_provider(&mut self, provider: Box<dyn MirrorProvider>) -> Result<()> {
        let s = self.state();
        if s != JobState::None && s != JobState::Disabled {
            return Err(anyhow!(format!("Provider cannot be switched when job state is {}", s)));
        }
        self.provider = Arc::new(provider);
        Ok(())
    }

/*
    async fn run_hooks(
        &self,
        hooks: Arc<Mutex<Vec<Box<dyn JobHook>>>>,
        action: impl Fn(&Box<dyn JobHook>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>,
        hook_name: &str,
        manager_chan: &mpsc::Sender<JobMessage>,
    ) -> Result<()> 
    {
        for hook in hooks.lock().await.iter() {
            if let Err(e) = action(hook).await {
                error!(
                    "failed at {} hooks for {}: {}",
                    hook_name, self.name().await, e
                );
                manager_chan.send(JobMessage {
                    status: SyncStatus::Failed,
                    name: self.name().await,
                    msg: format!("error exec hook {}: {}", hook_name, e),
                    schedule: true,
                }).await?;
                return Err(e.into());
            }
        }
        Ok(())
    }
 */
    // 在每个同步阶段都会依次运行provider内不同的job_hook中的方法
    async fn run_hooks(
        &self,
        hooks: Arc<Mutex<Vec<Box<dyn JobHook>>>>,
        action: HookAction,
        manager_chan: &Sender<JobMessage>,
    ) -> Result<()>
    {
        match action {
            HookAction::PreJob => {
                for hook in hooks.lock().await.iter() {
                    if let Err(e) = hook.pre_job(self.provider.working_dir().await, 
                                                 self.name()) {
                        return send_err(e, "pre-job", &self.name(), manager_chan).await
                    }
                }
            },
            HookAction::PreExec => {
                for hook in hooks.lock().await.iter() {
                    if let Err(e) = hook.pre_exec(self.name(), 
                                                  self.provider.log_dir().await, 
                                                  self.provider.log_file().await, 
                                                  self.provider.working_dir().await, 
                                                  self.provider.context().await).await {
                        return send_err(e, "pre-exec", &self.name(), manager_chan).await
                    }
                }
            },
            // 需要倒序
            HookAction::PostExec => {
                for hook in hooks.lock().await.iter().rev() {
                    if let Err(e) = hook.post_exec(self.provider.context().await, 
                                                   self.name()).await {
                        return send_err(e, "post-exec", &self.name(), manager_chan).await
                    }
                }
            },
            // 需要倒序
            HookAction::PostSuccess => {
                for hook in hooks.lock().await.iter().rev() {
                    if let Err(e) = hook.post_success(self.provider.context().await, 
                                                      self.name(), 
                                                      self.provider.working_dir().await, 
                                                      self.provider.upstream(), 
                                                      self.provider.log_dir().await, 
                                                      self.provider.log_file().await).await {
                        return send_err(e, "post-success", &self.name(), manager_chan).await
                    }
                }
            },
            // 需要倒序
            HookAction::PostFail => {
                for hook in hooks.lock().await.iter().rev() {
                    if let Err(e) = hook.post_fail(self.name(), 
                                                   self.provider.working_dir().await, 
                                                   self.provider.upstream(), 
                                                   self.provider.log_dir().await, 
                                                   self.provider.log_file().await, 
                                                   self.provider.context().await).await {
                        return send_err(e, "post-fail", &self.name(), manager_chan).await
                    }
                }
            },
        }
        Ok(())
    }

    // 整个同步流程
    async fn run_job_wrapper(
        &self,
        mut kill: Receiver<Empty>,
        job_done: Sender<Empty>,
        manager_chan: Sender<JobMessage>,
    ) -> Result<()> 
    {
        scopeguard::defer!{
            drop(job_done);
        }

        // 发送PreSyncing状态
        manager_chan.send(JobMessage {
            status: SyncStatus::PreSyncing,
            name: self.name(),
            msg: String::new(),
            schedule: false,
        }).await?;
        info!("开始同步: {}", self.name());

        let hooks = self.provider.hooks().await;

        // Pre-job hooks
        debug!("hooks: pre-job");

        self.run_hooks(Arc::clone(&hooks), HookAction::PreJob, &manager_chan).await?;

        // 设置retry和timeout后进行同步，同步成功或失败都会发送信号
        // 非强制终止类型的同步失败发生时，会根据retry再次尝试同步
        for retry in 0..self.provider.retry().await {
            let mut stop_asap = false;  // stop job as soon as possible

            if retry > 0 {
                info!("重试同步: {}, 重试次数: {}", self.name(), retry);
            }

            // Pre-exec hooks
            self.run_hooks(Arc::clone(&hooks), HookAction::PreExec, &manager_chan).await?;

            // 开始同步
            manager_chan.send(JobMessage {
                status: SyncStatus::Syncing,
                name: self.name(),
                msg: String::new(),
                schedule: false,
            }).await?;

            let (sync_done_tx, mut sync_done_rx) = mpsc::channel(1);
            let (started_tx, mut started_rx) = mpsc::channel(10); // we may receive "started" more than one time (e.g. two_stage_rsync)
            
            // 启动provider
            let provider = self.provider.clone();
            tokio::spawn(async move {
                if let Err(e) = provider.run(started_tx).await {
                    sync_done_tx.send(Err(anyhow!(e))).await.unwrap();
                } else {
                    sync_done_tx.send(Ok(())).await.unwrap();
                }
            });

            // 等待provider启动或出错
            let mut sync_err = Ok(());
            tokio::select! {
                Some(result) = sync_done_rx.recv() => {
                    if let Err(e) = result {
                        error!("provider {} 启动失败: {}", self.name(), e);
                        sync_err = Err(e.root_cause().to_string());  // it will be read again later
                    }
                }
                Some(_) = started_rx.recv() => {
                    debug!("provider 已启动");
                }
            }
            // Now terminating the provider is feasible

            // 处理超时和终止
            let mut timeout = self.provider.timeout().await;
            let timeout = if timeout.is_zero() {
                Duration::from_secs(3600 * 100000) // 永远不会超时
            } else {
                Duration::from_secs(timeout.num_seconds() as u64)
            };

            let mut term_err = None;
            tokio::select! {
                Some(result) = sync_done_rx.recv() => {
                    if let Err(e) = result {
                        sync_err = Err(e.root_cause().to_string());
                    }
                    debug!("同步完成");
                }
                _ = tokio::time::sleep(timeout) => {
                    warn!("provider 超时");
                    term_err = self.provider.terminate().await.err();
                    sync_err = Err(format!("{} 超时，等待时间 {:?}", self.name(), timeout));
                }
                None = kill.recv() => {
                    debug!("收到终止信号");
                    stop_asap = true;
                    term_err = self.provider.terminate().await.err();
                    sync_err = Err("被manager终止".to_string());
                }
            }

            if let Some(e) = term_err {
                error!("终止provider {} 失败: {}", self.name(), e);
                return Err(e.into());
            }

            // post-exec hooks
            self.run_hooks(Arc::clone(&hooks), HookAction::PostExec, &manager_chan).await?;

            if sync_err.is_ok() {
                // 同步成功
                info!("成功同步 {}", self.name());
                debug!("post-success hooks");
                
                self.run_hooks(Arc::clone(&hooks), HookAction::PostSuccess, &manager_chan).await?;

                { *self.size.lock().await = self.provider.data_size().await; }
                manager_chan.send(JobMessage {
                    status: SyncStatus::Success,
                    name: self.name(),
                    msg: String::new(),
                    schedule: self.state() == JobState::Ready,
                }).await?;
                return Ok(());
            } else {
                // 同步失败，包括被终止
                warn!("同步 {} 失败: {:?}", self.name(), sync_err);
                debug!("post-fail hooks");

                self.run_hooks(Arc::clone(&hooks), HookAction::PostFail,&manager_chan).await?;

                manager_chan.send(JobMessage {
                    status: SyncStatus::Failed,
                    name: self.name(),
                    msg: sync_err.err().unwrap().to_string(),
                    schedule: (retry == self.provider.retry().await - 1) && (self.state() == JobState::Ready),
                }).await?;

                //
                if stop_asap {
                    debug!("不再重试，直接退出");
                    return Ok(());
                }
                // 尝试进行下一次同步
            }

        } // retry循环

        Ok(())
    }

    async fn run_job(
        &self,
        mut kill: Receiver<Empty>,
        job_done: Sender<Empty>,
        manager_chan: Sender<JobMessage>,
        semaphore: Arc<Semaphore>,
        bypass_semaphore_rx: Arc<Mutex<Receiver<Empty>>>,
    ) 
    {
        let mut bypass_semaphore_lock = bypass_semaphore_rx.lock().await;
        
        tokio::select! {
            Ok(permit) = semaphore.acquire() => {
                drop(bypass_semaphore_lock);
                let _ = self.run_job_wrapper(kill, job_done, manager_chan).await;
                scopeguard::defer! {
                    drop(permit);
                }
            }
            Some(_) = bypass_semaphore_lock.recv() => {
                drop(bypass_semaphore_lock);
                warn!("{} 忽略了并发限制", self.name());
                let _ = self.run_job_wrapper(kill, job_done, manager_chan).await;
            }
            None = kill.recv() => {
                drop(bypass_semaphore_lock);
                // drop kill通道的发送端后，kill通道接收端会收到None，表示结束任务
                let _ = job_done.send(()).await;
            }
        }
    }
    
    
    pub async fn run(
        &self,
        manager_chan: Sender<JobMessage>,
        semaphore: Arc<Semaphore>,
    ) -> Result<()>
    {

        // disabled标记一个MirrorJob是否运行完毕，同步过后，其发送端被丢弃，接收端接收到None
        {
            let (disabled_tx, disabled_rx) = mpsc::channel(1);
            *self.disabled_tx.lock().await = Some(disabled_tx);
            *self.disabled_rx.lock().await = Some(disabled_rx);
        }

        let mut disabled_tx_lock = self.disabled_tx.lock().await;
        scopeguard::defer!{
            *disabled_tx_lock = None;
            drop(disabled_tx_lock);
        }

        
        let (bypass_semaphore_tx, bypass_semaphore_rx) = mpsc::channel(1);
        let bypass_semaphore_rx = Arc::new(Mutex::new(bypass_semaphore_rx));
        
        // 一次同步的整个流程
        'whole_syncing: loop {
            // 刚被初始化的MirrorJob此时的状态是JobState::None
            // 它等待接收从ctrl_chan发送的状态信号来选择接下来的动作
            if self.state() == JobState::Ready {
                // kill的发送端是否被丢弃，决定任务是否被强制结束，当任务被强制结束，接收端接收到None
                // 如果任务开始后马上被结束，会在run_job中收到None，如果在同步中结束，会在run_job_wrapper中收到None
                let (kill_tx, kill_rx) = mpsc::channel(1);
                // job_done来标记同步任务是否结束
                // 当完成run_job_wrapper后，它的发送端会被丢弃，在接收端会收到None
                let (job_done_tx, mut job_done_rx) = mpsc::channel(1);

                // manager_chan用来在同步中发送状态信息
                let manager_chan = manager_chan.clone();
                
                // bypass_semaphore_rx用来忽略并发限制
                // 如果ctrl_chan_rx接收到ForceStart，如果当前的同步任务数已达上限，下一个同步任务也会开始
                let bypass_semaphore_rx = Arc::clone(&bypass_semaphore_rx);

                // semaphore是进行同步时的最大并发任务数
                let semaphore = semaphore.clone();
                
                let mirror_job = self.clone();
                tokio::spawn(async move {
                    mirror_job.run_job(kill_rx, job_done_tx, 
                                       manager_chan, 
                                       semaphore, bypass_semaphore_rx).await;
                });

                let mut ctrl_lock = self.ctrl_chan_rx.lock().await;
                'wait_for_job: loop {
                    tokio::select! {
                        _ = job_done_rx.recv() => {
                            // 接收到Some(())表示job在run_job时接受到kill，被终止
                            // 接收到None表示job在run_job_wrapper中结束，完成所有同步过程
                            debug!("job done");
                            break 'wait_for_job;
                        }
                        Some(ctrl) = ctrl_lock.recv() => {
                            match ctrl {
                                JobCtrlAction::Stop => {
                                    self.set_state(JobState::Paused);
                                    drop(kill_tx);
                                    // 阻塞，等待已运行的同步工作结束
                                    let _ = job_done_rx.recv().await;
                                    break 'wait_for_job;
                                }
                                JobCtrlAction::Disable => {
                                    self.set_state(JobState::Disabled);
                                    drop(kill_tx);
                                    let _ = job_done_rx.recv().await;
                                    return Ok(());
                                }
                                JobCtrlAction::Restart => {
                                    self.set_state(JobState::Ready);
                                    drop(kill_tx);
                                    let _ = job_done_rx.recv().await;
                                    // 等待 1 秒，防止重启时任务还未完全退出。
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    continue 'whole_syncing;
                                }
                                JobCtrlAction::ForceStart => {
                                    let _ = bypass_semaphore_tx.try_send(());
                                    self.set_state(JobState::Ready);
                                    continue 'wait_for_job;
                                }
                                JobCtrlAction::Start => {
                                    self.set_state(JobState::Ready);
                                    continue 'wait_for_job;
                                }
                                JobCtrlAction::Halt => {
                                    self.set_state(JobState::Halting);
                                    drop(kill_tx);
                                    let _ = job_done_rx.recv().await;
                                    return Ok(());
                                }
                                _ => {
                                    drop(kill_tx);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }// wait_for_job 循环
            }

            // 一次wait_for_job完成后，将阻塞等待ctrl_chan的下一次的动作信号，没有信号不会开始
            if let Some(ctrl) = self.ctrl_chan_rx.lock().await.recv().await {
                match ctrl {
                    JobCtrlAction::Stop => {
                        self.set_state(JobState::Paused);
                    }
                    JobCtrlAction::Disable => {
                        self.set_state(JobState::Disabled);
                        return Ok(());
                    }
                    JobCtrlAction::ForceStart => {
                        //non-blocking
                        let _ = bypass_semaphore_tx.try_send(());
                        self.set_state(JobState::Ready);
                    }
                    JobCtrlAction::Restart | JobCtrlAction::Start => {
                        self.set_state(JobState::Ready);
                    }
                    _ => return Ok(()),
                }
            }
        }

    }
}

