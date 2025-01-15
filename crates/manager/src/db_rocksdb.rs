use std::collections::HashMap;
use rocksdb::DB;
use std::error::Error;
use crate::db::KvAdapter;

pub struct RocksDbAdapter {
    pub(crate) db: DB,
}

impl KvAdapter for RocksDbAdapter {
    fn init_bucket(&self, _bucket: &str) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn get(&self, bucket: &str, key: &str) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let full_key = format!("{}{}", bucket, key);

        // 查询数据库
        let result = self.db.get(full_key.as_bytes())?;
        
        // 返回查询结果
        Ok(result)
    }

    fn get_all(&self, bucket: &str) -> Result<HashMap<String, Vec<u8>>, Box<dyn Error>> {
        let prefix = format!("{}", bucket);
        let mut results = HashMap::new();

        // 使用前缀过滤器遍历所有匹配键值
        let iter = self.db.prefix_iterator(prefix.as_bytes());
        for item in iter {
            if let Ok((key, value)) = item {
                let key_str = String::from_utf8(key.to_vec())?;
                let actual_key = key_str.trim_start_matches(&prefix).to_string();
                results.insert(actual_key, value.to_vec());
            }
        }

        Ok(results)
    }

    fn put(&self, bucket: &str, key: &str, value: Vec<u8>) -> Result<(), Box<dyn Error>> {
        // 拼接 bucket 和 key
        let full_key = format!("{}{}", bucket, key);
        
        // 写入键值对
        self.db.put(full_key.as_bytes(), value).expect("插入失败");
        Ok(())
    }

    fn delete(&self, bucket: &str, key: &str) -> Result<(), Box<dyn Error>> {
        // 拼接 bucket 和 key
        let full_key = format!("{}{}", bucket, key);

        // 删除键
        self.db.delete(full_key.as_bytes())?;
        Ok(())
    }

    fn close(&self) -> Result<(), Box<dyn Error>> {
        // RocksDB 在 Rust 中不需要显式 close，资源会在对象释放时自动释放
        Ok(())
    }
}