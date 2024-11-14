use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use rusty_leveldb::{DB, DBIterator, LdbIterator, Options};
use std::sync::Arc;

use crate::db::KvAdapter;


pub struct LeveldbAdapter{
    pub(crate) db: RefCell<DB>,
}

impl KvAdapter for LeveldbAdapter {
    fn init_bucket(&self, bucket: &str) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn get(&self, bucket: &str, key: &str) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        // 拼接 bucket 和 key 作为数据库查询的键
        let full_key = format!("{}{}", bucket, key);

        // let read_options = ReadOptions::new();
        // 查询数据库
        match self.db.borrow_mut().get(full_key.as_ref()) {
            // 返回查询结果
            Some(value) => Ok(Some(value)),
            None => Ok(None),
        }

    }

    fn get_all(&self, bucket: &str) -> Result<HashMap<String, Vec<u8>>, Box<dyn Error>> {
        let mut results = HashMap::new();
        let prefix = bucket.as_bytes();
        // let read_options = ReadOptions::new();

        // 创建一个迭代器，以 bucket 的字节前缀作为前缀进行查找
        let mut iterator = self.db.borrow_mut().new_iter()?;

        while let Some((key, value)) = iterator.next() {
            if key.starts_with(prefix) {
                let actual_key = String::from_utf8_lossy(&key[prefix.len()..]).to_string();
                results.insert(actual_key, value.to_vec());
            }
        }
        Ok(results)
    }

    fn put(&self, bucket: &str, key: &str, value: Vec<u8>) -> Result<(), Box<dyn Error>> {
        // 拼接 bucket 和 key 作为数据库的存储键
        let full_key = format!("{}{}", bucket, key);
        // let write_opts = WriteOptions::new();

        // 将键值对写入数据库
        self.db.borrow_mut().put(full_key.as_bytes(), &value)?;

        Ok(())
    }

    fn delete(&self, bucket: &str, key: &str) -> Result<(), Box<dyn Error>> {
        // 拼接 bucket 和 key 作为数据库的删除键
        let full_key = format!("{}{}", bucket, key);
        // let write_opts = WriteOptions::new();

        // 从数据库中删除该键
        self.db.borrow_mut().delete(full_key.as_bytes())?;

        Ok(())
    }

    fn close(&self) -> Result<(), Box<dyn Error>> {
        // `leveldb` 在 Rust 中没有直接的 close 方法，通常可以通过 drop 来释放资源。
        // 可以使用 std::mem::drop(self.db.clone()) 手动关闭或将 db 设置为 Option 类型。

        // 手动 drop 以释放数据库资源
        // Arc::try_unwrap(self.db.clone()).map(|db| drop(db)).map_err(|_| {
        //     "Failed to close the database: multiple references to the database remain".into()
        // })?;

        Ok(())
    }
}