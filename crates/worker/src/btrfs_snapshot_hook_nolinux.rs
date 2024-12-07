use std::error::Error;
use crate::config::MirrorConfig;
use crate::hooks::JobHook;
use crate::provider::MirrorProvider;

#[cfg(not(target_os = "linux"))]
pub(crate) struct BtrfsSnapshotHook{}

#[cfg(not(target_os = "linux"))]
fn new_btrfs_snapshot_hook<T: Clone>(
    provider: Box<dyn MirrorProvider<ContextStoreVal=T>>, 
    snapshot_path: &str, 
    mirror_config: MirrorConfig) 
    -> BtrfsSnapshotHook
{
    BtrfsSnapshotHook{}
}

#[cfg(not(target_os = "linux"))]
impl JobHook for BtrfsSnapshotHook{
    fn per_job(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    fn pre_exec(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    fn post_exec(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    fn post_success(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    fn post_fail(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}