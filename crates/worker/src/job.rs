// use std::error::Error;
// use std::future::Future;
// use tokio::sync::mpsc;
// use std::sync::atomic::{AtomicU32, Ordering};
// use log::error;
// use internal::status::SyncStatus;
// use crate::common::Empty;
// use crate::hooks::JobHook;
// use crate::provider::MirrorProvider;
// 
// // 这个文件包含了mirror job的工作流
// pub(crate) enum JobCtrlAction {
//     Start,
//     /// 停止同步，但保留job
//     Stop,
//     /// 清除job（终止线程）
//     Disable,
//     /// 重启同步
//     Restart,
//     /// 查看线程是否存活
//     Ping,
//     /// worker停止
//     Halt,
//     /// 忽略并发限制
//     ForceStart,
// }
// 
// struct JobMessage {
//     status: SyncStatus,
//     name: String,
//     msg: String,
//     schedule: bool,
// }
// 
// #[derive(Clone)]
// enum WorkerState{
//     /// 空state
//     None = 0,
//     /// 已经准备好运行，可以规划
//     Ready = 1,
//     /// 被JobCtrlAction::Stop暂停
//     Paused = 2,
//     /// 被JobCtrlAction::Disable清除
//     Disabled = 3,
//     /// worker是停止状态
//     Halting = 4,
// }
// impl WorkerState {
//     fn as_u32(&self) -> u32{
//         self.clone() as u32
//     }
//     fn from_u32(value: u32) -> Option<Self> {
//         match value {
//             0 => Some(WorkerState::None),
//             1 => Some(WorkerState::Ready),
//             2 => Some(WorkerState::Paused),
//             3 => Some(WorkerState::Disabled),
//             4 => Some(WorkerState::Halting),
//             _ => None,
//         }
//     }
// }
// struct MirrorJob<T: Clone>{
//     provider: Box<dyn MirrorProvider<ContextStoreVal=T>>,
//     ctrl_chan: mpsc::Receiver<JobCtrlAction>,
//     disabled: Option<mpsc::Receiver<Empty>>,
//     state: AtomicU32,
//     size: String,
// }
// 
// impl<T: Clone> MirrorJob<T> {
//     fn new(provider: Box<dyn MirrorProvider<ContextStoreVal=T>>) -> Self {
//         MirrorJob{
//             provider,
//             ctrl_chan: mpsc::channel::<JobCtrlAction>(1).1,
//             state: AtomicU32::new(WorkerState::None.as_u32()),
//             
//             disabled: None,
//             size: "".to_string(),
//         }
//     }
//     
//     fn name(&self) -> String {
//         self.provider.name()
//     }
//     
//     fn state(&self) -> u32 {
//         self.state.load(Ordering::SeqCst)
//     }
//     fn set_state(&self, state: u32) {
//         self.state.store(state, Ordering::SeqCst);
//     }
//     fn set_provider(&mut self, provider: Box<dyn MirrorProvider<ContextStoreVal=T>>) -> Result<(), Box<dyn Error>>{
//         let s = self.state();
//         if (s != WorkerState::None.as_u32()) && (s != WorkerState::Disabled.as_u32()) {
//             return Err(format!("在job状态为 {} 时，不能更换provider", s).into());
//         }
//         self.provider = provider;
//         Ok(())
//     }
//     fn run(&mut self, mut manager_chan: mpsc::Sender<JobMessage>, semaphore: mpsc::Receiver<Empty>) -> Result<(), Box<dyn Error>> {
//         self.disabled = Some(mpsc::channel(1).1);
//         
//         let provider = self.provider.as_ref();
//         let run_hooks = 
//              |hooks: Vec<Box<dyn JobHook>>,
//                    action: fn(h: Box<dyn JobHook>) -> Result<(), Box<dyn Error>>,
//                    hook_name: String|
//                    -> Result<(), Box<dyn Error>>{
//                 for hook in hooks {
//                     if let Err(e) = action(hook) {
//                         error!("在 {} 的钩子 {} 发生错误：{}", self.name(), &hook_name, e);
//                         manager_chan.send(JobMessage {
//                             status: SyncStatus::Failed,
//                             name: self.name(),
//                             msg: format!("执行hook {} 失败: {}", &hook_name, e),
//                             schedule: true,
//                         });
//                         return Err(e);
//                     }
//                 }
//                 Ok(())
//             };
//         
//         let run_job_wrapper = 
//             |kill: mpsc::Sender<Empty>, 
//              job_name: mpsc::Sender<Empty>| 
//                 -> Result<(), Box<dyn Error>> {
//             Ok(())
//         };
//         
//         unimplemented!()
//     }
//     
//     
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
// 
// 
