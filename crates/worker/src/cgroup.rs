use crate::hooks::EmptyHook;
use crate::config::{CGroupConfig, MemBytes};


pub struct CGroupHook{
    // empty_hook: EmptyHook<T>,
    cg_cfg: CGroupConfig,
    mem_limit: MemBytes,
    // cg_mgr_v1: cgroups_rs::Cgroup,
    // cg_mgr_v2: cgroups_rs::Cgroup,
}

type ExecCmd = &'static str;
const CMD_CONT: ExecCmd = "cont";
const CMD_ABRT: ExecCmd = "abrt";