use std::collections::HashMap;
use chrono::Duration;

pub(crate) struct TwoStageRsyncConfig {
    pub(crate) name: String,
    pub(crate) rsync_cmd: String,
    pub(crate) stage1_profile: String,

    pub(crate) upstream_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) exclude_file: String,

    pub(crate) extra_options: Vec<String>,
    pub(crate) rsync_never_timeout: bool,
    pub(crate) rsync_timeout_value: i64,
    pub(crate) rsync_env: HashMap<String, String>,

    pub(crate) working_dir: String,
    pub(crate) log_dir: String,
    pub(crate) log_file: String,

    pub(crate) use_ipv4: bool,
    pub(crate) use_ipv6: bool,

    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
}