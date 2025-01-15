use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::msg::MirrorStatus;
use crate::status::SyncStatus;

#[derive(Debug, Clone, Default)]
pub struct TextTime(pub DateTime<Utc>);

// 实现自定义时间格式 "2006-01-02 15:04:05 -0700" 的序列化和反序列化
impl Serialize for TextTime {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let formatted = self.0.format("%Y-%m-%d %H:%M:%S %z").to_string();
        serializer.serialize_str(&formatted)
    }
}

impl<'de> Deserialize<'de> for TextTime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let dt = DateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S %z")
            .map_err(serde::de::Error::custom)?;
        Ok(TextTime(DateTime::from(dt)))
    }
}

#[derive(Debug, Clone, Default)]
pub struct StampTime(DateTime<Utc>);

// 实现 Unix 时间戳的序列化和反序列化
impl Serialize for StampTime {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(self.0.timestamp())
    }
}

impl<'de> Deserialize<'de> for StampTime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ts = i64::deserialize(deserializer)?;
        Ok(StampTime(Utc.timestamp(ts, 0)))
    }
}

// WebMirrorStatus是在web页面中显示的镜像的状态
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct WebMirrorStatus {
    pub name: String,
    pub is_master: bool,
    pub status: SyncStatus,
    pub last_update: TextTime,
    pub last_update_ts: StampTime,
    pub last_started: TextTime,
    pub last_started_ts: StampTime,
    pub last_ended: TextTime,
    pub last_ended_ts: StampTime,
    #[serde(rename = "next_schedule")]
    pub scheduled: TextTime,
    #[serde(rename = "next_schedule_ts")]
    pub scheduled_ts: StampTime,
    pub upstream: String,
    pub size: String,
}


// 实现 BuildWebMirrorStatus 函数
pub fn build_web_mirror_status(m: MirrorStatus) -> WebMirrorStatus {
    WebMirrorStatus {
        name: m.name,
        is_master: m.is_master,
        status: m.status,
        last_update: TextTime(m.last_update),
        last_update_ts: StampTime(m.last_update),
        last_started: TextTime(m.last_started),
        last_started_ts: StampTime(m.last_started),
        last_ended: TextTime(m.last_ended),
        last_ended_ts: StampTime(m.last_ended),
        scheduled: TextTime(m.scheduled),
        scheduled_ts: StampTime(m.scheduled),
        upstream: m.upstream,
        size: m.size,
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    use chrono::{FixedOffset, Duration};
    #[test]
    fn test_status_ser_de(){
        // 设置测试时区为 Asia/Tokyo
        let tz = FixedOffset::east_opt(9 * 3600).expect("valid offset");
        let t = tz
            .with_ymd_and_hms(2016, 4, 16, 23, 8, 10).unwrap();
        let t_utc: DateTime<Utc> = t.with_timezone(&Utc);

        let m = WebMirrorStatus {
            name: String::from("tunalinux"),
            status: SyncStatus::Success,
            last_update: TextTime(DateTime::from(t)),
            last_update_ts: StampTime(t.with_timezone(&Utc)),
            last_started: TextTime(DateTime::from(t)),
            last_started_ts: StampTime(t.with_timezone(&Utc)),
            last_ended: TextTime(DateTime::from(t)),
            last_ended_ts: StampTime(t.with_timezone(&Utc)),
            scheduled: TextTime(DateTime::from(t)),
            scheduled_ts: StampTime(t.with_timezone(&Utc)),
            size: String::from("5GB"),
            upstream: String::from("rsync://mirrors.tuna.tsinghua.edu.cn/tunalinux/"),
            ..WebMirrorStatus::default()
        };
        // 序列化 WebMirrorStatus 结构体
        let serialized = serde_json::to_string(&m).expect("serialization should succeed");

        // 反序列化为新的 WebMirrorStatus 结构体
        let m2: WebMirrorStatus = serde_json::from_str(&serialized).expect("deserialization should succeed");

        // 验证序列化和反序列化后的数据是否一致
        assert_eq!(m2.name, m.name);
        assert_eq!(m2.status, m.status);
        assert_eq!(m2.last_update.0.timestamp(), m.last_update.0.timestamp());
        assert_eq!(m2.last_update_ts.0.timestamp(), m.last_update_ts.0.timestamp());
        assert_eq!(m2.last_update.0.timestamp_nanos_opt().unwrap(), m.last_update.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_update_ts.0.timestamp_nanos_opt().unwrap(), m.last_update_ts.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_started.0.timestamp(), m.last_started.0.timestamp());
        assert_eq!(m2.last_started_ts.0.timestamp(), m.last_started_ts.0.timestamp());
        assert_eq!(m2.last_started.0.timestamp_nanos_opt().unwrap(), m.last_started.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_started_ts.0.timestamp_nanos_opt().unwrap(), m.last_started_ts.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_ended.0.timestamp(), m.last_ended.0.timestamp());
        assert_eq!(m2.last_ended_ts.0.timestamp(), m.last_ended_ts.0.timestamp());
        assert_eq!(m2.last_ended.0.timestamp_nanos_opt().unwrap(), m.last_ended.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_ended_ts.0.timestamp_nanos_opt().unwrap(), m.last_ended_ts.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.scheduled.0.timestamp(), m.scheduled.0.timestamp());
        assert_eq!(m2.scheduled_ts.0.timestamp(), m.scheduled_ts.0.timestamp());
        assert_eq!(m2.scheduled.0.timestamp_nanos_opt().unwrap(), m.scheduled.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.scheduled_ts.0.timestamp_nanos_opt().unwrap(), m.scheduled_ts.0.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.size, m.size);
        assert_eq!(m2.upstream, m.upstream);
    }

    #[test]
    fn test_build_web_mirror_status(){
        let m = MirrorStatus{
            name: String::from("arch-sync3"),
            worker: String::from("testWorker"),
            is_master: true,
            status: SyncStatus::Failed,
            last_update: Utc::now() - Duration::minutes(30),
            last_started: Utc::now() - Duration::minutes(1),
            last_ended: Utc::now(),
            scheduled: Utc::now() + Duration::minutes(5),
            upstream: String::from("mirrors.tuna.tsinghua.edu.cn"),
            size: String::from("4GB"),
            ..Default::default()
        };

        let m2 = build_web_mirror_status(m.clone());
        assert_eq!(m2.name, m.name);
        assert_eq!(m2.status, m.status);
        assert_eq!(m2.last_update.0.timestamp(), m.last_update.timestamp());
        assert_eq!(m2.last_update_ts.0.timestamp(), m.last_update.timestamp());
        assert_eq!(m2.last_update.0.timestamp_nanos_opt().unwrap(), m.last_update.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_update_ts.0.timestamp_nanos_opt().unwrap(), m.last_update.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_started.0.timestamp(), m.last_started.timestamp());
        assert_eq!(m2.last_started_ts.0.timestamp(), m.last_started.timestamp());
        assert_eq!(m2.last_started.0.timestamp_nanos_opt().unwrap(), m.last_started.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_started_ts.0.timestamp_nanos_opt().unwrap(), m.last_started.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_ended.0.timestamp(), m.last_ended.timestamp());
        assert_eq!(m2.last_ended_ts.0.timestamp(), m.last_ended.timestamp());
        assert_eq!(m2.last_ended.0.timestamp_nanos_opt().unwrap(), m.last_ended.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.last_ended_ts.0.timestamp_nanos_opt().unwrap(), m.last_ended.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.scheduled.0.timestamp(), m.scheduled.timestamp());
        assert_eq!(m2.scheduled_ts.0.timestamp(), m.scheduled.timestamp());
        assert_eq!(m2.scheduled.0.timestamp_nanos_opt().unwrap(), m.scheduled.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.scheduled_ts.0.timestamp_nanos_opt().unwrap(), m.scheduled.timestamp_nanos_opt().unwrap());
        assert_eq!(m2.size, m.size);
        assert_eq!(m2.upstream, m.upstream);
        
    }
}

