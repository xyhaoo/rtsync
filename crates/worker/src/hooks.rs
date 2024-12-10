use std::error::Error;
use std::sync::{Arc, Mutex};
use crate::context::Context;
use enum_dispatch::enum_dispatch;
#[cfg(target_os = "linux")]
use crate::btrfs_snapshot_hook;
use crate::{btrfs_snapshot_hook, btrfs_snapshot_hook_nolinux};
use crate::cgroup::CGroupHook;
use crate::docker::DockerHook;
use crate::exec_post_hook::ExecPostHook;
use crate::loglimit_hook::LogLimiter;
use crate::zfs_hook::ZfsHook;
/*
hooks to exec before/after syncing
                                                                        failed
                              +------------------ post-fail hooks -------------------+
                              |                                                      |
 job start -> pre-job hooks --v-> pre-exec hooks --> (syncing) --> post-exec hooks --+---------> post-success --> end
                                                                                       success
*/
#[enum_dispatch]
pub(crate) trait JobHook{
    fn per_job(&self, 
               _working_dir: String, 
               _provider_name: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn pre_exec(&self, 
                _provider_name: String,
                _log_dir: String, 
                _log_file: String, 
                _working_dir: String, 
                _context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_exec(&self, 
                 _context: &Arc<Mutex<Option<Context>>>, 
                 _provider_name: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_success(&self, 
                    _provider_name: String,
                    _working_dir: String,
                    _upstream: String,
                    _log_dir: String,
                    _log_file: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_fail(&self,
                 _provider_name: String,
                 _working_dir: String,
                 _upstream: String,
                 _log_dir: String,
                 _log_file: String,
                 _context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>> {Ok(())}
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
impl_into_box_for!(btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook);
impl_into_box_for!(CGroupHook);
impl_into_box_for!(DockerHook);
impl_into_box_for!(ExecPostHook);
impl_into_box_for!(LogLimiter);
impl_into_box_for!(ZfsHook);

#[enum_dispatch(JobHook, JobIntoBox)]
pub(crate) enum HookType{
    #[cfg(target_os = "linux")]
    Btrfs(btrfs_snapshot_hook::BtrfsSnapshotHook),
    BtrfsNoLinux(btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook),
    Cgroup(CGroupHook),
    Docker(DockerHook),
    // Empty(EmptyHook),
    ExecPost(ExecPostHook),
    LogLimiter(LogLimiter),
    Zfs(ZfsHook),
}

/*
impl JobIntoBox for DockerHook {
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
impl JobIntoBox for ZfsHook{
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
impl JobIntoBox for CGroupHook {
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
impl JobIntoBox for LogLimiter{
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
impl JobIntoBox for ExecPostHook {
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
impl JobIntoBox for btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook{
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
#[cfg(target_os = "linux")]
impl JobIntoBox for BtrfsSnapshotHook{
    fn into_box(self) -> Box<dyn JobHook> {
        Box::new(self)
    }
}
 */