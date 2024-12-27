use crate::config::MirrorConfig;
use crate::hooks::{JobHook};

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone)]
pub(crate) struct BtrfsSnapshotHook{}
#[cfg(not(target_os = "linux"))]
impl BtrfsSnapshotHook {
    pub(crate) fn new(
        _snapshot_path: &str,
        _mirror_config: MirrorConfig)
        -> BtrfsSnapshotHook
    {
        BtrfsSnapshotHook{}
    }
}


#[cfg(not(target_os = "linux"))]
impl JobHook for BtrfsSnapshotHook{}

