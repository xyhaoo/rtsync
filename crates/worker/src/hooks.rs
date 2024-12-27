use anyhow::{anyhow, Result};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::context::Context;
use enum_dispatch::enum_dispatch;
#[cfg(target_os = "linux")]
use crate::btrfs_snapshot_hook;
#[cfg(not(target_os = "linux"))]
use crate::{btrfs_snapshot_hook_nolinux};
use crate::cgroup::CGroupHook;
use crate::docker::DockerHook;
use crate::exec_post_hook::ExecPostHook;
use crate::loglimit_hook::LogLimiter;
use crate::zfs_hook::ZfsHook;
use async_trait::async_trait;
/*
hooks to exec before/after syncing
                                                                        failed
                              +------------------ post-fail hooks -------------------+
                              |                                                      |
 job start -> pre-job hooks --v-> pre-exec hooks --> (syncing) --> post-exec hooks --+---------> post-success --> end
                                                                                       success
*/
#[async_trait]
#[enum_dispatch]
pub(crate) trait JobHook: Debug + Send + Sync{
    fn pre_job(&self, 
               _working_dir: String, 
               _provider_name: String) 
        -> Result<()> {Ok(())}
    
    async fn pre_exec(&self, 
                _provider_name: String,
                _log_dir: String, 
                _log_file: String, 
                _working_dir: String, 
                _context: Arc<Mutex<Option<Context>>>) 
        -> Result<()> {Ok(())}
    
    async fn post_exec(&self, 
                 _context: Arc<Mutex<Option<Context>>>, 
                 _provider_name: String) 
        -> Result<()> {Ok(())}
    
    async fn post_success(&self,
                    _context: Arc<Mutex<Option<Context>>>, 
                    _provider_name: String,
                    _working_dir: String,
                    _upstream: String,
                    _log_dir: String,
                    _log_file: String) 
        -> Result<()> {Ok(())}
    
    async fn post_fail(&self,
                 _provider_name: String,
                 _working_dir: String,
                 _upstream: String,
                 _log_dir: String,
                 _log_file: String,
                 _context: Arc<Mutex<Option<Context>>>) 
        -> Result<()> {Ok(())}
}
pub(crate) struct EmptyHook{
    // pub(crate) provider: Box<dyn MirrorProvider<ContextStoreVal=T>>,
}

#[enum_dispatch]
pub(crate) trait JobIntoBox: JobHook{
    fn into_box(self) -> Box<dyn JobHook>;
}

macro_rules! impl_into_box_for {
    ($t:ty) => {
        impl JobIntoBox for $t {
            fn into_box(self) -> Box<dyn JobHook> {
                Box::new(self)
            }
        }
    };
}
#[cfg(target_os = "linux")]
impl_into_box_for!(btrfs_snapshot_hook::BtrfsSnapshotHook);
#[cfg(not(target_os = "linux"))]
impl_into_box_for!(btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook);
impl_into_box_for!(CGroupHook);
impl_into_box_for!(DockerHook);
impl_into_box_for!(ExecPostHook);
impl_into_box_for!(LogLimiter);
impl_into_box_for!(ZfsHook);

#[enum_dispatch(JobHook, JobIntoBox)]
#[derive(Debug)]
pub(crate) enum HookType{
    #[cfg(target_os = "linux")]
    Btrfs(btrfs_snapshot_hook::BtrfsSnapshotHook),
    #[cfg(not(target_os = "linux"))]
    BtrfsNoLinux(btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook),
    Cgroup(CGroupHook),
    Docker(DockerHook),
    // Empty(EmptyHook),
    ExecPost(ExecPostHook),
    LogLimiter(LogLimiter),
    Zfs(ZfsHook),
}