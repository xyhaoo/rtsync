use std::cmp::Ordering;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use std::fmt;
use std::collections::HashMap;
use crate::status::SyncStatus;

// 当一个worker完成同步时，MirrorStatus表示一个msg
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct MirrorStatus {
    pub name: String,
    pub worker: String,
    pub is_master: bool,
    pub status: SyncStatus,
    pub last_update: DateTime<Utc>,
    pub last_started: DateTime<Utc>,
    pub last_ended: DateTime<Utc>,
    #[serde(rename = "next_schedule")]
    pub scheduled: DateTime<Utc>,
    pub upstream: String,
    pub size: String,
    pub error_msg: String,
}
impl Ord for MirrorStatus{
    fn cmp(&self, other: &MirrorStatus) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for MirrorStatus {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for MirrorStatus{
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for MirrorStatus {}


// WorkerStatus是描述worker的信息结构体，从manager发送给客户端。
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct WorkerStatus {
    pub id: String,
    pub url: String,    // worker url
    pub token: String,  // session token
    pub last_online: DateTime<Utc>, // last seen
    pub last_register: DateTime<Utc>,   // last register time
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MirrorSchedules {
    pub schedules: Vec<MirrorSchedule>,
}


#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MirrorSchedule {
    pub mirror_name: String,
    pub next_schedule: DateTime<Utc>,
}

// CmdVerb是对worker或其job的操作
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum CmdVerb {
    Start,  // 开始同步
    Stop,   // 停止同步，但仍保持job协程存在
    Disable,    // 禁用同步作业（停止协程）
    Restart,    //重新启动同步job
    Ping,   //  确保同步协程存活
    Reload, // 通知worker重载mirror config
}
impl fmt::Display for CmdVerb {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            CmdVerb::Start => "start",
            CmdVerb::Stop => "stop",
            CmdVerb::Disable => "disable",
            CmdVerb::Restart => "restart",
            CmdVerb::Ping => "ping",
            CmdVerb::Reload => "reload",
        };
        write!(f, "{}", s)
    }
}
impl CmdVerb {
    fn from_str(s: &str) -> Result<CmdVerb, &'static str> {
        match s {
            "start" => Ok(CmdVerb::Start),
            "stop" => Ok(CmdVerb::Stop),
            "disable" => Ok(CmdVerb::Disable),
            "restart" => Ok(CmdVerb::Restart),
            "ping" => Ok(CmdVerb::Ping),
            "reload" => Ok(CmdVerb::Reload),
            _ => Err("Invalid CmdVerb value"),
        }
    }
}
// CmdVerb 自定义的序列化和反序列化实现
impl Serialize for CmdVerb {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl<'de> Deserialize<'de> for CmdVerb {
    fn deserialize<D>(deserializer: D) -> Result<CmdVerb, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        CmdVerb::from_str(&s).map_err(de::Error::custom)
    }
}





// WorkerCmd是从manager发送给worker的命令消息
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerCmd {
    pub cmd: CmdVerb,
    pub mirror_id: String,
    pub args: Vec<String>,
    pub options: HashMap<String, bool>,
}

// 自定义 WorkerCmd 的 Display 实现
impl fmt::Display for WorkerCmd {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.args.is_empty() {
            write!(f, "{} ({}, {:?})", self.cmd, self.mirror_id, self.args)
        } else {
            write!(f, "{} ({})", self.cmd, self.mirror_id)
        }
    }
}

// ClientCmd是从客户端发送到manager的命令消息
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientCmd {
    pub cmd: CmdVerb,
    pub mirror_id: String,
    pub worker_id: String,
    pub args: Vec<String>,
    pub options: HashMap<String, bool>,
}
impl Default for ClientCmd {
    fn default() -> Self {
        ClientCmd{
            cmd: CmdVerb::Start,
            mirror_id: "".to_string(),
            worker_id: "".to_string(),
            args: Vec::new(),
            options: HashMap::new(),
        }
    }
}


