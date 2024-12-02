use std::collections::HashMap;
use chrono::{DateTime, Utc};

struct CmdConfig{
    name: String,
    
    upstream_uri: String,
    command: String,
    
    work_dir: String,
    log_dir: String,
    log_file: String,
    
    interval: DateTime<Utc>,
    retry: u64,
    timeout: DateTime<Utc>,
    env: HashMap<String, String>,
    fail_on_match: String,
    size_pattern: String,
}

struct CmdProvider{
    
}

// impl 