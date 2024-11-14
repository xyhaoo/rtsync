use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncStatus {
    #[default]
    None,
    Failed,
    Success,
    Syncing,
    PreSyncing,
    Paused,
    Disabled,
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            SyncStatus::None => "none",
            SyncStatus::Failed => "failed",
            SyncStatus::Success => "success",
            SyncStatus::Syncing => "syncing",
            SyncStatus::PreSyncing => "pre-syncing",
            SyncStatus::Paused => "paused",
            SyncStatus::Disabled => "disabled",
        };
        write!(f, "{}", status_str)
    }
}

impl Serialize for SyncStatus {
    // 序列化
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SyncStatus {
    // 反序列化
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "none" => Ok(SyncStatus::None),
            "failed" => Ok(SyncStatus::Failed),
            "success" => Ok(SyncStatus::Success),
            "syncing" => Ok(SyncStatus::Syncing),
            "pre-syncing" => Ok(SyncStatus::PreSyncing),
            "paused" => Ok(SyncStatus::Paused),
            "disabled" => Ok(SyncStatus::Disabled),
            _ => Err(serde::de::Error::custom(format!("Invalid status value: {}", s))),
        }
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn test_sync_status() {
        // 测试序列化
        let status = SyncStatus::PreSyncing;
        let serialized = serde_json::to_string(&status).unwrap();
        assert_eq!(serialized, r#""pre-syncing""#);

        // 测试反序列化
        let deserialized: SyncStatus = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(deserialized, SyncStatus::Failed);
        
    }
}