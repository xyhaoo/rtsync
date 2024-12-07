use std::collections::HashMap;
use chrono::{DateTime, Duration, Utc};
use crate::base_provider::BaseProvider;

pub(crate) struct CmdConfig{
    pub(crate) name: String,

    pub(crate) upstream_url: String,
    pub(crate) command: String,

    pub(crate) working_dir: String,
    pub(crate) log_dir: String,
    pub(crate) log_file: String,

    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
    pub(crate) env: HashMap<String, String>,
    pub(crate) fail_on_match: String,
    pub(crate) size_pattern: String,
}

pub(crate) struct CmdProvider{
    base_provider: BaseProvider<()>,
    cmd_config: CmdConfig,
    command: Vec<String>,
    data_size: String,
    
}
// 
// // impl 