use redis::{Commands, Client};
use std::collections::HashMap;
use std::error::Error;
use crate::db::KvAdapter;
pub struct RedisAdapter {
    pub(crate) db: Client,
}


impl KvAdapter for RedisAdapter {
    fn init_bucket(&self, _bucket: &str) -> Result<(), Box<dyn Error>> {
        // Redis 不需要创建 bucket，哈希表在插入时会自动生成
        Ok(())
    }

    fn get(&self, bucket: &str, key: &str) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let mut conn = self.db.get_connection()?;
        let result: Option<String> = conn.hget(bucket, key)?;

        match result {
            Some(val) => Ok(Some(val.into_bytes())),
            None => Ok(None),
        }
    }

    fn get_all(&self, bucket: &str) -> Result<HashMap<String, Vec<u8>>, Box<dyn Error>> {
        let mut conn = self.db.get_connection()?;
        let result: HashMap<String, String> = conn.hgetall(bucket)?;

        // 转换成 Vec<u8> 值的 HashMap
        let byte_map = result
            .into_iter()
            .map(|(k, v)| (k, v.into_bytes()))
            .collect();

        Ok(byte_map)
    }

    fn put(&self, bucket: &str, key: &str, value: Vec<u8>) -> Result<(), Box<dyn Error>> {
        let mut conn = self.db.get_connection()?;
        
        // 将 Vec<u8> 转换为 String,错误时将错误类型转为 Box<dyn Error>
        let value_str = String::from_utf8(value)
            .map_err(|e| Box::new(e) as Box<dyn Error>)?; 
        
        conn.hset(bucket, key, value_str)?;
        Ok(())
    }

    fn delete(&self, bucket: &str, key: &str) -> Result<(), Box<dyn Error>> {
        let mut conn = self.db.get_connection()?;
        conn.hdel(bucket, key)?;
        Ok(())
    }

    fn close(&self) -> Result<(), Box<dyn Error>> {
        // 在 Rust Redis 客户端中，不需要显式关闭连接；连接会在对象被销毁时自动关闭
        Ok(())
    }
}