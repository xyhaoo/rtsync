// use std::error::Error;
// use std::sync::atomic::{AtomicU32, Ordering};
// use std::time::Duration;
// use tokio::sync::mpsc;
// use log::{debug, error, info, warn};
// use crate::common::Empty;
// use crate::hooks::JobHook;
// use crate::provider::MirrorProvider;
// use internal::status::SyncStatus;
// use scopeguard::guard;
// 
// // 控制动作枚举
// #[derive(Debug, Clone)]
// pub(crate) enum JobCtrlAction {
//     Start,
//     Stop,      // 停止同步但保持job
//     Disable,   // 禁用job(停止goroutine)
//     Restart,   // 重启同步
//     Ping,      // 确保goroutine存活
//     Halt,      // worker停止
//     ForceStart, // 忽略并发限制
// }
// 
// // Job消息结构
// #[derive(Debug)]
// struct JobMessage {
//     status: SyncStatus,
//     name: String,
//     msg: String,
//     schedule: bool,
// }
// 
// // Worker状态枚举
// #[derive(Clone, Copy, PartialEq)]
// enum WorkerState {
//     None = 0,      // 空状态
//     Ready = 1,     // 准备运行,可调度
//     Paused = 2,    // 被jobStop暂停
//     Disabled = 3,  // 被jobDisable禁用
//     Halting = 4,   // worker正在停止
// }
// 
// impl WorkerState {
//     fn from_u32(val: u32) -> Option<Self> {
//         match val {
//             0 => Some(WorkerState::None),
//             1 => Some(WorkerState::Ready),
//             2 => Some(WorkerState::Paused),
//             3 => Some(WorkerState::Disabled),
//             4 => Some(WorkerState::Halting),
//             _ => None
//         }
//     }
// }
// 
// // Mirror Job结构
// pub struct MirrorJob {
//     provider: Box<dyn MirrorProvider>,
//     ctrl_chan: mpsc::Receiver<JobCtrlAction>,
//     disabled: Option<mpsc::Receiver<Empty>>,
//     state: AtomicU32,
//     size: String,
// }
// 
// impl MirrorJob {
//     pub fn new(provider: Box<dyn MirrorProvider>) -> Self {
//         let (_, rx) = mpsc::channel(1);
//         Self {
//             provider,
//             ctrl_chan: rx,
//             disabled: None,
//             state: AtomicU32::new(WorkerState::None as u32),
//             size: String::new(),
//         }
//     }
// 
//     pub fn name(&self) -> String {
//         self.provider.name()
//     }
// 
//     pub fn state(&self) -> u32 {
//         self.state.load(Ordering::SeqCst)
//     }
// 
//     pub fn set_state(&self, state: u32) {
//         self.state.store(state, Ordering::SeqCst);
//     }
// 
//     pub fn set_provider(&mut self, provider: Box<dyn MirrorProvider>) -> Result<(), Box<dyn Error>> {
//         let s = self.state();
//         if s != WorkerState::None as u32 && s != WorkerState::Disabled as u32 {
//             return Err(format!("Provider cannot be switched when job state is {}", s).into());
//         }
//         self.provider = provider;
//         Ok(())
//     }
// 
//     async fn run_hooks(
//         &self,
//         hooks: &[Box<dyn JobHook>],
//         action: impl Fn(&Box<dyn JobHook>) -> Result<(), Box<dyn Error>>,
//         hook_name: &str,
//         manager_chan: &mpsc::Sender<JobMessage>,
//     ) -> Result<(), Box<dyn Error>> 
//     {
//         for hook in hooks {
//             if let Err(e) = action(hook) {
//                 error!(
//                     "failed at {} hooks for {}: {}",
//                     hook_name, self.name(), e
//                 );
//                 manager_chan.send(JobMessage {
//                     status: SyncStatus::Failed,
//                     name: self.name(),
//                     msg: format!("error exec hook {}: {}", hook_name, e),
//                     schedule: true,
//                 }).await?;
//                 return Err(e.into());
//             }
//         }
//         Ok(())
//     }
// 
//     async fn run_job_wrapper(
//         &mut self,
//         mut kill: mpsc::Receiver<Empty>,
//         job_done: mpsc::Sender<Empty>,
//         manager_chan: mpsc::Sender<JobMessage>,
//     ) -> Result<(), Box<dyn Error>> 
//     {
//         let _guard = scopeguard::guard((), |_| {
//             let _ = job_done.send(());
//         });
// 
//         // 发送PreSyncing状态
//         manager_chan.send(JobMessage {
//             status: SyncStatus::PreSyncing,
//             name: self.name(),
//             msg: String::new(),
//             schedule: false,
//         }).await?;
//         info!("start syncing: {}", self.name());
// 
//         let hooks = self.provider.hooks();
//         let r_hooks: Vec<Box<dyn JobHook>> = hooks.iter()
//             .rev()
//             .map(|h| h.clone())
//             .collect();
// 
//         // Pre-job hooks
//         debug!("hooks: pre-job");
//         self.run_hooks(&hooks, 
//                 |h| h.pre_job(self.provider.working_dir(), self.name()), 
//                 "pre-job", 
//                 &manager_chan
//         ).await?;
// 
//         for retry in 0..self.provider.retry() {
//             let mut stop_asap = false;
// 
//             if retry > 0 {
//                 info!("retry syncing: {}, retry: {}", self.name(), retry);
//             }
// 
//             // Pre-exec hooks
//             self.run_hooks(&hooks, 
//                            |h| h.pre_exec(self.name(), 
//                                           self.provider.log_dir(), 
//                                           self.provider.log_file(), 
//                                           self.provider.working_dir(), 
//                                           self.provider.context()), 
//                            "pre-exec", 
//                            &manager_chan
//             ).await?;
// 
//             // 开始同步
//             manager_chan.send(JobMessage {
//                 status: SyncStatus::Syncing,
//                 name: self.name(),
//                 msg: String::new(),
//                 schedule: false,
//             }).await?;
// 
//             let (sync_done_tx, mut sync_done_rx) = mpsc::channel(1);
//             let (started_tx, mut started_rx) = mpsc::channel(10);
//             
//             // 启动provider
//             let provider = self.provider.as_mut();
//             tokio::spawn(async move {
//                 if let Err(e) = provider.run(started_tx).await {
//                     let _ = sync_done_tx.send(Err(e)).await;
//                 } else {
//                     let _ = sync_done_tx.send(Ok(())).await;
//                 }
//             });
// 
//             // 等待provider启动或出错
//             let mut sync_err = None;
//             tokio::select! {
//                 Some(result) = sync_done_rx.recv() => {
//                     if let Err(e) = result {
//                         error!("failed to start provider {}: {}", self.name(), e);
//                         sync_err = Some(e);
//                     }
//                 }
//                 Some(_) = started_rx.recv() => {
//                     debug!("provider started");
//                 }
//             }
// 
//             // 处理超时和终止
//             let timeout = self.provider.timeout();
//             let timeout = if timeout.is_zero() {
//                 Duration::from_secs(360000) // 100小时
//             } else {
//                 Duration::from_secs(timeout.num_seconds() as u64)
//             };
// 
//             let mut term_err = None;
//             tokio::select! {
//                 Some(result) = sync_done_rx.recv() => {
//                     sync_err = result.err();
//                     debug!("syncing done");
//                 }
//                 _ = tokio::time::sleep(timeout) => {
//                     warn!("provider timeout");
//                     term_err = self.provider.terminate().err();
//                     sync_err = Some(format!("{} timeout after {:?}", self.name(), timeout).into());
//                 }
//                 Some(_) = kill.recv() => {
//                     debug!("received kill");
//                     stop_asap = true;
//                     term_err = self.provider.terminate().err();
//                     sync_err = Some("killed by manager".into());
//                 }
//             }
// 
//             if let Some(e) = term_err {
//                 error!("failed to terminate provider {}: {}", self.name(), e);
//                 return Err(e.into());
//             }
// 
//             // Post-exec hooks
//             self.run_hooks(&r_hooks, |h| h.post_exec(self.provider.context(), self.name()), "post-exec", &manager_chan).await?;
// 
//             if sync_err.is_none() {
//                 // 同步成功
//                 info!("succeeded syncing {}", self.name());
//                 self.run_hooks(&r_hooks, |h| h.post_success(self.provider.context(), self.name(), self.provider.working_dir(), self.provider.upstream(), self.provider.log_dir(), self.provider.log_file()), "post-success", &manager_chan).await?;
//                 
//                 self.size = self.provider.data_size();
//                 manager_chan.send(JobMessage {
//                     status: SyncStatus::Success,
//                     name: self.name(),
//                     msg: String::new(),
//                     schedule: self.state() == WorkerState::Ready as u32,
//                 }).await?;
//                 return Ok(());
//             } else {
//                 // 同步失败
//                 warn!("failed syncing {}: {:?}", self.name(), sync_err);
//                 self.run_hooks(&r_hooks, |h| h.post_fail(self.name(), self.provider.working_dir(), self.provider.upstream(), self.provider.log_dir(), self.provider.log_file(), self.provider.context()), "post-fail", &manager_chan).await?;
// 
//                 manager_chan.send(JobMessage {
//                     status: SyncStatus::Failed,
//                     name: self.name(),
//                     msg: sync_err.unwrap().to_string(),
//                     schedule: (retry == self.provider.retry() - 1) && (self.state() == WorkerState::Ready as u32),
//                 }).await?;
// 
//                 if stop_asap {
//                     debug!("No retry, exit directly");
//                     return Ok(());
//                 }
//             }
//         }
//         Ok(())
//     }
// 
//     async fn run_job(
//         &mut self,
//         mut kill: mpsc::Receiver<Empty>,
//         job_done: mpsc::Sender<Empty>,
//         manager_chan: mpsc::Sender<JobMessage>,
//         semaphore: mpsc::Sender<Empty>,
//         mut bypass_semaphore: mpsc::Receiver<Empty>,
//     ) 
//     {
//         tokio::select! {
//             Ok(_) = semaphore.send(()) => {
//                 let _ = self.run_job_wrapper(kill, job_done, manager_chan).await;
//                 let _ = semaphore.send(());
//             }
//             Some(_) = bypass_semaphore.recv() => {
//                 info!("Concurrent limit ignored by {}", self.name());
//                 let _ = self.run_job_wrapper(kill, job_done, manager_chan).await;
//             }
//             Some(_) = kill.recv() => {
//                 let _ = job_done.send(());
//             }
//         }
//     }
// 
//     pub async fn run(
//         &mut self,
//         manager_chan: mpsc::Sender<JobMessage>,
//         semaphore: mpsc::Sender<Empty>,
//     ) -> Result<(), Box<dyn Error>> 
//     {
//         let (disabled_tx, disabled_rx) = mpsc::channel(1);
//         self.disabled = Some(disabled_rx);
// 
//         let (bypass_tx, bypass_rx) = mpsc::channel(1);
// 
//         loop {
//             if self.state() == WorkerState::Ready as u32 {
//                 let (kill_tx, kill_rx) = mpsc::channel(1);
//                 let (job_done_tx, mut job_done_rx) = mpsc::channel(1);
// 
//                 let mut this = self;
//                 let manager_chan = manager_chan.clone();
//                 let semaphore = semaphore.clone();
//                 
//                 tokio::spawn(async move {
//                     this.run_job(
//                         kill_rx,
//                         job_done_tx,
//                         manager_chan,
//                         semaphore,
//                         bypass_rx,
//                     ).await;
//                 });
// 
//                 loop {
//                     tokio::select! {
//                         Some(_) = job_done_rx.recv() => {
//                             debug!("job done");
//                             break;
//                         }
//                         Some(ctrl) = self.ctrl_chan.recv() => {
//                             match ctrl {
//                                 JobCtrlAction::Stop => {
//                                     self.set_state(WorkerState::Paused as u32);
//                                     drop(kill_tx);
//                                     let _ = job_done_rx.recv().await;
//                                 }
//                                 JobCtrlAction::Disable => {
//                                     self.set_state(WorkerState::Disabled as u32);
//                                     drop(kill_tx);
//                                     let _ = job_done_rx.recv().await;
//                                     return Ok(());
//                                 }
//                                 JobCtrlAction::Restart => {
//                                     self.set_state(WorkerState::Ready as u32);
//                                     drop(kill_tx);
//                                     let _ = job_done_rx.recv().await;
//                                     tokio::time::sleep(Duration::from_secs(1)).await;
//                                     continue;
//                                 }
//                                 JobCtrlAction::ForceStart => {
//                                     let _ = bypass_tx.send(()).await;
//                                 }
//                                 JobCtrlAction::Start => {
//                                     self.set_state(WorkerState::Ready as u32);
//                                 }
//                                 JobCtrlAction::Halt => {
//                                     self.set_state(WorkerState::Halting as u32);
//                                     drop(kill_tx);
//                                     let _ = job_done_rx.recv().await;
//                                     return Ok(());
//                                 }
//                                 _ => {
//                                     drop(kill_tx);
//                                     return Ok(());
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
// 
//             if let Some(ctrl) = self.ctrl_chan.recv().await {
//                 match ctrl {
//                     JobCtrlAction::Stop => {
//                         self.set_state(WorkerState::Paused as u32);
//                     }
//                     JobCtrlAction::Disable => {
//                         self.set_state(WorkerState::Disabled as u32);
//                         return Ok(());
//                     }
//                     JobCtrlAction::ForceStart => {
//                         let _ = bypass_tx.send(()).await;
//                     }
//                     JobCtrlAction::Restart | JobCtrlAction::Start => {
//                         self.set_state(WorkerState::Ready as u32);
//                     }
//                     _ => return Ok(()),
//                 }
//             }
//         }
//     }
// }
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
// 
