use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use chrono::Utc;
use internal::msg::{WorkerStatus, MirrorStatus};
use serde_json;
use std::sync::{Arc, RwLock, RwLockWriteGuard};
use internal::status::SyncStatus;
use std::path::Path;
use crate::db_rocksdb::RocksDbAdapter;
use crate::db_redis::RedisAdapter;
use crate::db_leveldb::LeveldbAdapter;

use rusty_leveldb;
use redis;
use rocksdb;

pub(crate) trait DbAdapter: Send + Sync {
    fn init(&self) -> Result<(), Box<dyn Error>>;
    fn list_workers(&self) -> Result<Vec<WorkerStatus>, Box<dyn Error>>;
    fn get_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>>;
    fn delete_worker(&self, worker_id: &str) -> Result<(), Box<dyn Error>>;
    fn create_worker(&self, w: WorkerStatus) -> Result<WorkerStatus, Box<dyn Error>>;
    fn refresh_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>>;
    fn update_mirror_status(&self, worker_id: &str, mirror_id: &str, status: MirrorStatus)
        -> Result<MirrorStatus, Box<dyn Error>>;
    fn get_mirror_status(&self, worker_id: &str, mirror_id: &str) -> Result<MirrorStatus, Box<dyn Error>>;
    fn list_mirror_states(&self, worker_id: &str) -> Result<Vec<MirrorStatus>, Box<dyn Error>>;
    fn list_all_mirror_states(&self) -> Result<Vec<MirrorStatus>, Box<dyn Error>>;
    fn flush_disabled_jobs(&self) -> Result<(), Box<dyn Error>>;
    fn close(&self) -> Result<(), Box<dyn Error>>;
}

pub(crate) trait KvAdapter {
    fn init_bucket(&self, bucket: &str) -> Result<(), Box<dyn Error>>;
    fn get(&self, bucket: &str, key: &str) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
    fn get_all(&self, bucket: &str) -> Result<HashMap<String, Vec<u8>>, Box<dyn Error>>;
    fn put(&self, bucket: &str, key: &str, value: Vec<u8>) -> Result<(), Box<dyn Error>>;
    fn delete(&self, bucket: &str, key: &str) -> Result<(), Box<dyn Error>>;
    fn close(&self) -> Result<(), Box<dyn Error>>;
}

const _WORKER_BUCKET_KEY: &str = "worker";
const _STATUS_BUCKET_KEY: &str = "mirror_status";

pub(crate) fn make_db_adapter(db_type: &str, db_file: &str) -> Result<impl DbAdapter, Box<dyn Error>> {
    if db_type.eq("leveldb"){
        let path = Path::new(db_file);
        let mut options = rusty_leveldb::Options::default();
        options.create_if_missing = true;
        
        return match rusty_leveldb::DB::open(path, options) {
            Ok(inner_db) => { 
                let db = LeveldbAdapter{db: RefCell::new(inner_db)}; 
                let kv = KvDbAdapter{db: Arc::new(RwLock::new(db))};
                kv.init()?;
                Ok(kv)
            },
            Err(e) => {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{}, 不能打开这个leveldb '{}'", e, db_file),
                )))
            }
        };
    }else if db_type.eq("redis") {
        return match redis::Client::open(format!("redis://{}/0", db_file)) {
            Ok(inner_db) => {
                let db = RedisAdapter { db: inner_db };
                let kv = KvDbAdapter { db: Arc::new(RwLock::new(db)) };
                kv.init()?;
                Ok(kv)
            }
            Err(e) => {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{}, 错误的redis url '{}'", e, db_file),
                )))
            }
        }
    }else if db_type.eq("rocksdb") {
        // let mut opts = rocksdb::Options::default();
        // opts.create_if_missing(true);
        return match rocksdb::DB::open_default(db_file) {
            Ok(inner_db) => {
                let db = RocksDbAdapter { db: inner_db };
                let kv = KvDbAdapter{db: Arc::new(RwLock::new(db))};
                kv.init()?; // init()创建bucket，在go版本中只有boltdb有这个操作，所以实际上重写版本无需这个函数
                Ok(kv)
            }
            Err(e) => {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{}, 不能打开这个leveldb '{}'", e, db_file),
                )))
            }
        }
    }
    // 不支持的数据库类型
    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("不支持的数据库类型： {}", db_type),
    )))
}


struct KvDbAdapter {
    db: Arc<RwLock<dyn KvAdapter>>,
}
// 静态分析工具：可以使用像 Clippy、Miri 等工具来检测潜在的线程安全问题。
// Miri 是一个执行 Rust 程序的工具，它可以帮助发现内存和线程安全问题。
unsafe impl Send for KvDbAdapter {}
unsafe impl Sync for KvDbAdapter {}

impl DbAdapter for KvDbAdapter {
    fn init(&self) -> Result<(), Box<dyn Error>> {
        // 创建 Bucket 的过程
        let create_bucket = |bucket_key: &str| -> Result<(), Box<dyn Error>> {
            return 
                match self.db.write().map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("获取锁失败: {}", e),
                    )) as Box<dyn Error>}) 
                {
                    Ok(db) => {
                        match db.init_bucket(bucket_key).map_err(|e| {
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("创建 bucket {} 失败: {}", bucket_key, e),
                            ))}) 
                        {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => {
                        Err(e)
                    }
                };
            
        };
        
        // 尝试创建 _WORKER_BUCKET_KEY
        create_bucket(_WORKER_BUCKET_KEY)?;

        // 尝试创建 _STATUS_BUCKET_KEY
        create_bucket(_STATUS_BUCKET_KEY)
        
    }

    fn list_workers(&self) -> Result<Vec<WorkerStatus>, Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>})
        {
            Ok(db) => {
                let mut workers = Vec::new();
                
                // 从数据库中获取所有 _WORKER_BUCKET_KEY 桶中的数据
                let all_workers = db.get_all(_WORKER_BUCKET_KEY)?;
                // 遍历每个存储的 worker 数据
                for (_key, value) in all_workers {
                    // 尝试将二进制数据反序列化为 WorkerStatus
                    match serde_json::from_slice::<WorkerStatus>(&value) {
                        Ok(worker_status) => {
                            // 如果成功，添加到 workers 列表
                            workers.push(worker_status);
                        }
                        Err(e) => {
                            // 如果反序列化失败，记录错误并继续下一个 worker 数据
                            eprintln!("反序列化 WorkerStatus 失败: {}", e);
                        }
                    }
                }
                Ok(workers)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn get_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}){
            Ok(db) => {
                // 从 _WORKER_BUCKET_KEY 桶中获取指定 worker_id 的数据
                let value = db.get(_WORKER_BUCKET_KEY, worker_id)?;

                // 如果没有找到对应的 worker 数据，则返回错误
                if value.is_none() {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("没有这个worker_id： {}", worker_id),
                    )));
                }

                // 将数据反序列化为 WorkerStatus
                let worker: WorkerStatus = serde_json::from_slice(&value.unwrap())?;
                Ok(worker)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn delete_worker(&self, worker_id: &str) -> Result<(), Box<dyn Error>> {
        match self.db.write().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}){
            Ok(db) => {
                let result = db.get(_WORKER_BUCKET_KEY, worker_id)?;
                if result.is_none() {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("没有这个worker_id: {}", worker_id),
                    )) as Box<dyn Error>);
                }
                db.delete(_WORKER_BUCKET_KEY, worker_id)?;
                Ok(())
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn create_worker(&self, w: WorkerStatus) -> Result<WorkerStatus, Box<dyn Error>> {
        match self.db.write().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                // 将 WorkerStatus 序列化为 JSON
                let value = serde_json::to_vec(&w)?;
                // 将序列化后的数据存储到数据库
                db.put(_WORKER_BUCKET_KEY, &*w.id.clone(), value)?;
                Ok(w)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    // 将worker的last_online字段设置为当前时刻
    fn refresh_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>> {
        // 获取现有的 WorkerStatus
        let mut worker = self.get_worker(worker_id)?;
        // 更新 LastOnline 字段
        worker.last_online = Utc::now();
        // 将更新后的 WorkerStatus 存储到数据库
        self.create_worker(worker.clone())?;
        Ok(worker)
    }

    fn update_mirror_status(&self, worker_id: &str, mirror_id: &str, status: MirrorStatus) -> Result<MirrorStatus, Box<dyn Error>> {
        match self.db.write().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                let id = format!("{}/{}", mirror_id, worker_id);
                // 将 MirrorStatus 序列化为 JSON
                let value = serde_json::to_vec(&status)?;
                // 将序列化后的数据存储到数据库
                db.put(_STATUS_BUCKET_KEY, &*id, value)?;
                Ok(status)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn get_mirror_status(&self, worker_id: &str, mirror_id: &str) -> Result<MirrorStatus, Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                let id = format!("{}/{}", mirror_id, worker_id);
                // 从数据库获取数据
                let value = db.get(_STATUS_BUCKET_KEY, &*id)?;

                // 如果数据不存在，则返回错误
                if value.is_none() {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("在worker '{}' 里没有镜像任务 '{}' ", worker_id, mirror_id),
                    )));
                }

                // 反序列化为 MirrorStatus
                let status: MirrorStatus = serde_json::from_slice(&value.unwrap())?;
                Ok(status)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn list_mirror_states(&self, worker_id: &str) -> Result<Vec<MirrorStatus>, Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                let all_vals = db.get_all(_STATUS_BUCKET_KEY)?;
                let mut statuses = Vec::new();

                for (key, value) in all_vals {
                    // 仅匹配指定 worker_id 的条目
                    if let Some(w_id) = key.split('/').nth(1) {
                        if w_id == worker_id {
                            // 反序列化并添加到结果列表
                            if let Ok(status) = serde_json::from_slice::<MirrorStatus>(&value) {
                                statuses.push(status);
                            }
                        }
                    }
                }

                Ok(statuses)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn list_all_mirror_states(&self) -> Result<Vec<MirrorStatus>, Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                let all_vals = db.get_all(_STATUS_BUCKET_KEY)?;
                let mut statuses = Vec::new();

                for (_key, value) in all_vals {
                    // 反序列化并添加到结果列表
                    if let Ok(status) = serde_json::from_slice::<MirrorStatus>(&value) {
                        statuses.push(status);
                    }
                }

                Ok(statuses)
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }
    

    fn flush_disabled_jobs(&self) -> Result<(), Box<dyn Error>> {
        match self.db.write().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                // 从 _STATUS_BUCKET_KEY 桶中获取所有数据
                let all_vals = db.get_all(_STATUS_BUCKET_KEY)?;

                for (key, value) in all_vals {
                    // 尝试将每个数据反序列化为 MirrorStatus
                    match serde_json::from_slice::<MirrorStatus>(&value) {
                        Ok(status) => {
                            // 检查状态是否为 Disabled 或 Name 为空
                            if status.status == SyncStatus::Disabled || status.name.is_empty() {
                                // 删除不需要的条目
                                if let Err(delete_err) = db.delete(_STATUS_BUCKET_KEY, &*key.clone()) {
                                    eprintln!("删除'{}'失败：{}", key, delete_err);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("反序列化 MirrorStatus 失败: {}", e);
                        }
                    }
                }

                Ok(())
            }
            Err(e) => {
                Err(e)
            }
        }
        
    }

    fn close(&self) -> Result<(), Box<dyn Error>> {
        match self.db.read().map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("获取锁失败: {}", e),
            )) as Box<dyn Error>}) {
            Ok(db) => {
                // 关闭数据库连接（如果存在）
                db.close()?;
                Ok(())
            }
            Err(e) => {
                Err(e)
            }
        }
    }
}





#[cfg(test)]
mod tests {
    use std::fs::File;
    use chrono::Duration;
    use tempfile::Builder;
    use super::*;
    
    fn sort_mirror_status(status: &mut Vec<MirrorStatus>) {
        status.sort();
    }

    fn db_adapter_test_create(db: impl DbAdapter){
        let test_worker_ids = vec!["test_worker1", "test_worker2"];
        
        // 测试创建worker
        for id in &test_worker_ids {
            let w = WorkerStatus{
                id: id.parse().unwrap(),
                token: format!("token_{}", id),
                last_online: Utc::now(),
                last_register: Utc::now(),
                ..Default::default()
            };
            let w = db.create_worker(w).unwrap();
        }
        
        // 测试get_worker，worker_id合法
        db.get_worker(test_worker_ids[0]).unwrap();
        
        // 测试list_worker
        let ws = db.list_workers().unwrap();
        assert_eq!(ws.len(), 2);
        
        // 测试get_worker，worker_id不合法
        let result = db.get_worker("invalid worker_id");
        assert!(result.is_err());
        
        // 测试delete_worker worker_id合法
        let result = db.delete_worker(test_worker_ids[0]);
        assert!(result.is_ok());
        let result = db.get_worker(test_worker_ids[0]);
        assert!(result.is_err());
        let ws = db.list_workers().unwrap();
        assert_eq!(ws.len(), 1);
        
        // 测试delete_worker worker_id不合法
        let result = db.delete_worker("invalid worker_id");
        assert!(result.is_err());
        let ws = db.list_workers().unwrap();
        assert_eq!(ws.len(), 1);
    }
    fn db_adapter_test_update(db: impl DbAdapter){
        let test_worker_ids = vec!["test_worker1", "test_worker2"];
        let mut status = vec![
            MirrorStatus{
                name: "arch-sync1".parse().unwrap(),
                worker: test_worker_ids[0].parse().unwrap(),
                is_master: true,
                status: SyncStatus::Success,
                last_update: Utc::now(),
                last_started: Utc::now() - Duration::minutes(1),
                last_ended: Utc::now(),
                upstream: "mirrors.tuna.tsinghua.edu.cn".parse().unwrap(),
                size: "3GB".parse().unwrap(),
                ..Default::default()
            }, 
            MirrorStatus{
                name: "arch-sync2".parse().unwrap(),
                worker: test_worker_ids[1].parse().unwrap(),
                is_master: true,
                status: SyncStatus::Disabled,
                last_update: Utc::now() - Duration::hours(1),
                last_started: Utc::now() - Duration::minutes(1),
                last_ended: Utc::now(),
                upstream: "mirrors.tuna.tsinghua.edu.cn".parse().unwrap(),
                size: "4GB".parse().unwrap(),
                ..Default::default()
            }, 
            MirrorStatus{
                name: "arch-sync3".parse().unwrap(),
                worker: test_worker_ids[1].parse().unwrap(),
                is_master: true,
                status: SyncStatus::Success,
                last_update: Utc::now() - Duration::minutes(1),
                last_started: Utc::now() - Duration::seconds(1),
                last_ended: Utc::now(),
                upstream: "mirrors.tuna.tsinghua.edu.cn".parse().unwrap(),
                size: "4GB".parse().unwrap(),
                ..Default::default()
            }];
        sort_mirror_status(&mut status);
        
        for s in &status {
            let result = db.update_mirror_status(&s.worker, &s.name, s.clone());
            assert!(result.is_ok());
        }
        
        // 测试get_mirror_status
        let m = db.get_mirror_status(test_worker_ids[0], status[0].name.clone().as_str()).unwrap();
        let expected_json = serde_json::to_string(&status[0]).unwrap();   
        let actual_json = serde_json::to_string(&m).unwrap();
        assert_eq!(expected_json, actual_json);
        
        // 测试list_mirror_status
        let ms = db.list_mirror_states(test_worker_ids[0]).unwrap();
        let expected_json = serde_json::to_string(&vec![status[0].clone()]).unwrap();
        let actual_json = serde_json::to_string(&ms).unwrap();
        assert_eq!(expected_json, actual_json);
        
        // 测试list_all_mirror_status
        let mut ms = db.list_all_mirror_states().unwrap();
        ms.sort();
        let expected_json = serde_json::to_string(&status).unwrap();
        let actual_json = serde_json::to_string(&ms).unwrap();
        assert_eq!(expected_json, actual_json);
        
        // 测试flush_disabled_jobs
        let ms = db.list_all_mirror_states().unwrap();
        assert_eq!(ms.len(), 3);
        db.flush_disabled_jobs().unwrap();
        let ms = db.list_all_mirror_states().unwrap();
        assert_eq!(ms.len(), 2);
    }
    #[test]
    fn test_leveldb_adapter(){
        //生成一个包含在临时目录（前缀为rtsync）中的文件rtsync
        {
            let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
                .prefix("rtsync")
                .tempdir().expect("failed to create tmp dir");
            let tmp_dir_path = tmp_dir.path();
            let db_dir_path = tmp_dir_path.join("leveldb.db");
            //生成的目录，包含在临时目录内，会随其一起被删除，且该目录名后面没有英文字母后缀
            std::fs::create_dir_all(&db_dir_path)
                .expect("failed to create db directory");

            // println!("{:?}",db_dir_path);
            let leveldb_db = make_db_adapter("leveldb", db_dir_path.to_str().unwrap()).unwrap();
            db_adapter_test_create(leveldb_db);
        }
        {
            let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
                .prefix("rtsync")
                .tempdir().expect("failed to create tmp dir");
            let tmp_dir_path = tmp_dir.path();
            let db_dir_path = tmp_dir_path.join("leveldb.db");
            //生成的目录，包含在临时目录内，会随其一起被删除，且该目录名后面没有英文字母后缀
            std::fs::create_dir_all(&db_dir_path)
                .expect("failed to create db directory");

            // println!("{:?}",db_dir_path);
            let leveldb_db = make_db_adapter("leveldb", db_dir_path.to_str().unwrap()).unwrap();
            db_adapter_test_update(leveldb_db);
        }
        
    }
    #[test]
    fn test_redis_adapter(){
        {
            let redis_addr = "localhost:6379";
            let redis_db = make_db_adapter("redis", redis_addr).unwrap();
            db_adapter_test_create(redis_db);
        }
        {
            let redis_addr = "localhost:6379";
            let redis_db = make_db_adapter("redis", redis_addr).unwrap();
            db_adapter_test_update(redis_db);
        }
        
    }
    #[test]
    fn test_rocksdb_adapter(){
        {
            let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
                .prefix("rtsync")
                .tempdir().expect("failed to create tmp dir");
            let tmp_dir_path = tmp_dir.path();
            let db_dir_path = tmp_dir_path.join("rocksdb.db");
            //生成的目录，包含在临时目录内，会随其一起被删除，且该目录名后面没有英文字母后缀
            std::fs::create_dir_all(&db_dir_path)
                .expect("failed to create db directory");
        
            // println!("{:?}",db_dir_path);
            let rocksdb_db = make_db_adapter("rocksdb", db_dir_path.to_str().unwrap()).unwrap();
            db_adapter_test_create(rocksdb_db);
        }
        {
            let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
                .prefix("rtsync")
                .tempdir().expect("failed to create tmp dir");
            let tmp_dir_path = tmp_dir.path();
            let db_dir_path = tmp_dir_path.join("rocksdb.db");
            //生成的目录，包含在临时目录内，会随其一起被删除，且该目录名后面没有英文字母后缀
            std::fs::create_dir_all(&db_dir_path)
                .expect("failed to create db directory");
        
            // println!("{:?}",db_dir_path);
            let rocksdb_db = make_db_adapter("rocksdb", db_dir_path.to_str().unwrap()).unwrap();
            db_adapter_test_update(rocksdb_db);
        }
    }
}




























