

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::sync::{Arc, RwLock, RwLockReadGuard};
    use std::{thread, time};
    use std::sync::atomic::{AtomicU32, Ordering};
    use chrono::{Duration, TimeZone, Utc};
    use internal::msg::{ClientCmd, CmdVerb, MirrorSchedule, MirrorSchedules, MirrorStatus, WorkerCmd, WorkerStatus};
    use crate::db::DbAdapter;
    use log::{debug, error, info};
    use rocket::http::Status;
    use rocket::local::asynchronous::{Client, LocalResponse};
    use rocket::{tokio, Build, Rocket, State};
    use rocket::serde::json::Json;
    use rocket::yansi::Paint;
    use tokio::sync::mpsc;
    use tracing_subscriber::fmt::format;
    // use rocket::local::blocking::Client;
    // use tokio::task;
    use internal::logger::init_logger;
    use internal::status::SyncStatus;
    use internal::status_web::WebMirrorStatus;
    use internal::util::{get_json, post_json};
    use crate::config::Config;
    use crate::server::*;

    const _MAGIC_BAD_WORKER_ID: &'static str = "magic_bad_worker_id";
    struct MockDbAdapter{
        worker_store: Arc<RwLock<HashMap<String, WorkerStatus>>>,
        status_store: Arc<RwLock<HashMap<String, MirrorStatus>>>,
    }

    impl DbAdapter for MockDbAdapter {
        fn init(&self) -> Result<(), Box<dyn Error>> {
            Ok(())
        }

        fn list_workers(&self) -> Result<Vec<WorkerStatus>, Box<dyn Error>> {
            let mut workers: Vec<WorkerStatus> = Vec::with_capacity(self.worker_store.read().unwrap().len());
            for (_k, v) in self.worker_store.read().unwrap().iter() {
                workers.push(v.clone());
            }
            Ok(workers)
        }

        fn get_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>> {
            match self.worker_store.read().unwrap().get(worker_id) {
                Some(status) => Ok(status.clone()),
                None => {
                    error!("无效的worker_id");
                    Err("无效的worker_id".into())
                },
            }
        }

        fn delete_worker(&self, worker_id: &str) -> Result<(), Box<dyn Error>> {
            self.worker_store.write().unwrap().remove(worker_id);
            Ok(())
        }

        fn create_worker(&self, w: WorkerStatus) -> Result<WorkerStatus, Box<dyn Error>> {
            self.worker_store.write().unwrap().insert(w.id.clone(), w.clone());
            Ok(w)
        }

        fn refresh_worker(&self, worker_id: &str) -> Result<WorkerStatus, Box<dyn Error>> {
            match self.get_worker(worker_id){
                Ok(mut w) => {
                    w.last_online = Utc::now();
                    let w = self.create_worker(w)?;
                    Ok(w)
                }
                Err(e) => {
                    Err(e)
                }
            }
        }

        fn update_mirror_status(&self, worker_id: &str, mirror_id: &str, status: MirrorStatus) -> Result<MirrorStatus, Box<dyn Error>> {
            let id = format!("{}/{}", mirror_id, worker_id);
            self.status_store.write().unwrap().insert(id, status.clone());
            Ok(status)
        }

        fn get_mirror_status(&self, worker_id: &str, mirror_id: &str) -> Result<MirrorStatus, Box<dyn Error>> {
            let id = format!("{}/{}", mirror_id, worker_id);
            match self.status_store.read().unwrap().get(&id) {
                None => {
                    let err = format!("在worker {} 中不存在镜像 {}", worker_id, mirror_id);
                    error!("{}", err);
                    Err(err.into())
                },
                Some(status) => {
                    Ok(status.clone())
                }
            }
        }

        fn list_mirror_states(&self, worker_id: &str) -> Result<Vec<MirrorStatus>, Box<dyn Error>> {
            let mut mirror_status_list: Vec<MirrorStatus> = Vec::default();
            // 模拟数据库故障
            if worker_id.eq(_MAGIC_BAD_WORKER_ID){
                error!("数据库故障");
                return Err("数据库故障".into());
            }
            for (k, v) in self.status_store.read().unwrap().iter() {
                let w_id = k.split('/').collect::<Vec<&str>>()[1];
                if w_id.eq(worker_id){
                    mirror_status_list.push(v.clone());
                }
            }
            Ok(mirror_status_list)
        }

        fn list_all_mirror_states(&self) -> Result<Vec<MirrorStatus>, Box<dyn Error>> {
            let mut mirror_status_list: Vec<MirrorStatus> = Vec::default();
            for (_, v) in self.status_store.read().unwrap().iter() {
                mirror_status_list.push(v.clone());
            }
            Ok(mirror_status_list)
        }

        fn flush_disabled_jobs(&self) -> Result<(), Box<dyn Error>> {
            Ok(())
        }

        fn close(&self) -> Result<(), Box<dyn Error>> {
            Ok(())
        }
    }

    
    fn make_mock_worker_server(cmd_chan: mpsc::Sender<WorkerCmd>) -> Rocket<Build>{
        let mut engine = Rocket::build();
        engine = engine.manage(cmd_chan);
        engine = engine.mount("/", routes![ping, cmd]);
        engine
    }

    #[post("/cmd", format = "application/json", data = "<cmd>")]
    async fn cmd(sender: &State<mpsc::Sender<WorkerCmd>>, cmd: Json<WorkerCmd>){
        sender.send(cmd.into_inner()).await.unwrap();
    }
    

    #[rocket::async_test]
    async fn test_http_server() {
        let mut listen_port = 5000;
        listen_port += 1;
        let port = listen_port;
        let addr = "127.0.0.1".to_string();
        let base_url = format!("{}/{}", addr, port);
        // init_logger(true, true, false);
        let mut s = get_rtsync_manager(&Config{
            debug: true,
            ..Config::default()
        }).unwrap();
        s.cfg.server.addr = Some(addr);
        s.cfg.server.port = Some(port);
        let worker_status_map = Arc::new(RwLock::new(HashMap::default()));
        worker_status_map.write().unwrap().insert(
            _MAGIC_BAD_WORKER_ID.to_string(),
            WorkerStatus{ id: _MAGIC_BAD_WORKER_ID.to_string(), ..WorkerStatus::default() });

        s.engine = s.engine.manage(Box::new(MockDbAdapter{
            worker_store: worker_status_map,
            status_store: Arc::new(RwLock::new(HashMap::new())),
        }) as Box<dyn DbAdapter>);

        s.engine = s.engine.manage(reqwest::Client::new());


        let client = Client::tracked(s.run()).await.expect("valid rocket instance");
        let response = client.get("/ping").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        // assert_eq!(response.body().take(), "pong");
        assert_eq!(response.headers().get("Content-Type").next().unwrap(), "application/json");

        let mut p: HashMap<String, String> = HashMap::default();
        p = response.into_json::<HashMap<String, String>>().await.unwrap();
        assert_eq!(p.get("message").unwrap(), "pong");


        // test_database_fail(&client).await;

        // test_register_multiple_workers(Arc::new(tokio::sync::RwLock::new(client))).await

        test_register_worker(Arc::new(tokio::sync::RwLock::new(client))).await
    }

    async fn test_database_fail(client: &Client) {
        let response = client.get(format!("/workers/{}/jobs", _MAGIC_BAD_WORKER_ID)).dispatch().await;
        assert_eq!(response.status(), Status::InternalServerError);
        
        // let raw_response = response.into_string().unwrap();
        // assert_eq!(raw_response, "")
        let msg: HashMap<String, HashMap<String, String>> = response.into_json().await.unwrap();
        assert!(msg.get("Err").unwrap().get("error").unwrap().eq("在列出worker_id为magic_bad_worker_id的worker的所有job时失败：数据库故障"));

    }

    async fn test_register_multiple_workers(arc_client: Arc<tokio::sync::RwLock<Client>>) {
        let n = 10;
        let cnt = Arc::new(AtomicU32::new(0));
        // let arc_client = Arc::new(tokio::sync::RwLock::new(client));
        let mut handles = vec![];
        for i in 0..n {
            let cnt = Arc::clone(&cnt);
            let arc_client = Arc::clone(&arc_client);
            // 在异步任务中运行请求
            let handle = tokio::spawn(async move {
                let worker = WorkerStatus {
                    id: format!("worker{}", i),
                    ..WorkerStatus::default()
                };

                // 发送 POST 请求
                let binding = arc_client.read().await;
                let response = binding
                    .post("/workers")
                    .json(&worker)
                    .dispatch().await;

                // 检查工人注册是否成功
                assert_eq!(response.status().to_string(), Status::Ok.to_string());
                cnt.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        // 等待所有异步任务完成
        for handle in handles {
            handle.await.unwrap();
        }

        // 确保所有工人都已注册
        assert_eq!(cnt.load(Ordering::SeqCst), n);

        // 测试获取所有工人列表
        let arc_client = Arc::clone(&arc_client);
        let binding = arc_client.read().await;
        let response = binding.get("/workers").dispatch().await;
        assert_eq!(response.status(), Status::Ok);

        let workers: Vec<WorkerStatus> = response.into_json::<HashMap<String, Vec<WorkerStatus>>>()
            .await.unwrap()
            .get("Ok").expect("valid JSON response")
            .to_vec();

        // 确保工人数量是预期的
        assert_eq!(workers.len(), (n + 1) as usize);  // 可能有一个默认工人

    }

    async fn test_register_worker(arc_client: Arc<tokio::sync::RwLock<Client>>){
        let w = WorkerStatus{
            id: "test_worker1".to_string(),
            ..WorkerStatus::default()
        };

        let binding = arc_client.read().await;
        let resp = binding.post("/workers")
            .json(&w)
            .dispatch()
            .await;
        assert_eq!(resp.status(), Status::Ok);


        // 测试列出所有worker
        // test_list_all_workers(&binding).await;

        // 测试删除存在的worker
        // test_delete_existent_worker(&binding, &w.id).await;

        // 测试删除不存在的worker
        let invalid_worker = "test_worker_haha";
        // test_delete_nonexistent_worker(&binding, invalid_worker).await;

        // 测试刷新禁用的jobs
        // test_flush_disabled_jobs(&binding).await;


        // 测试更新现有worker的mirror状态
        let status = MirrorStatus{
            name: "arch-sync1".to_string(),
            worker: "test_worker1".to_string(),
            is_master: true,
            status: SyncStatus::Success,
            upstream: "mirrors.tuna.tsinghua.edu.cn".to_string(),
            size: "unknown".to_string(),
            ..MirrorStatus::default()
        };
        // test_update_mirror_status_of_a_existed_worker(&binding, status).await;


        // 测试更新不存在的worker的mirror状态
        let invalid_worker = "test_worker2".to_string();
        let status = MirrorStatus{
            name: "arch-sync2".to_string(),
            worker: invalid_worker,
            is_master: true,
            status: SyncStatus::Success,
            last_update: Utc::now(),
            last_started: Utc::now(),
            last_ended: Utc::now(),
            upstream: "mirrors.tuna.tsinghua.edu.cn".to_string(),
            size: "4GB".to_string(),
            ..MirrorStatus::default()
        };
        // test_update_mirror_status_of_a_nonexistent_worker(&binding, status).await;
        
        // 更新不存在的worker的时间表
        let invalid_worker = "test_worker2".to_string();
        let mut schedules = vec![];
        schedules.push(MirrorSchedule{
            mirror_name: "arch-sync1".to_string(),
            next_schedule: Utc::now() + Duration::minutes(10),
            ..MirrorSchedule::default()
        });
        schedules.push(MirrorSchedule{
            mirror_name: "arch-sync2".to_string(),
            next_schedule: Utc::now() + Duration::minutes(7),
            ..MirrorSchedule::default()
        });
        let sch = MirrorSchedules{
            schedules
        };
        // test_update_schedule_of_an_nonexistent_worker(&binding, invalid_worker, sch).await
    

    }

    async fn test_list_all_workers(binding: &tokio::sync::RwLockReadGuard<'_, Client>){
        let resp = binding.get("/workers").dispatch().await;
        let workers: Vec<WorkerStatus> = resp.into_json::<HashMap<String, Vec<WorkerStatus>>>()
            .await.unwrap()
            .get("Ok").expect("序列化失败")
            .to_vec();
        assert_eq!(workers.len(), 2);
    }

    async fn test_delete_existent_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>, worker_id: &str){
        let resp = binding.delete(format!("/workers/{}", worker_id)).dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let res = resp.into_json::<HashMap<String, String>>()
            .await.expect("序列化失败");

        assert_eq!(res.get("message").unwrap(), "deleted");
    }

    async fn test_delete_nonexistent_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>, invalid_worker: &str) {
        let resp = binding.delete(format!("/workers/{}", invalid_worker)).dispatch().await;
        assert_eq!(resp.status(), Status::BadRequest);

        // let ret = resp.into_string().await.unwrap();
        // assert_eq!(ret, "");
        let res = resp.into_json::<HashMap<String, String>>()
            .await.expect("序列化失败");

        assert_eq!(res.get("error").unwrap(), &format!("无效的worker_id: {}", invalid_worker));
    }

    async fn test_flush_disabled_jobs(binding: &tokio::sync::RwLockReadGuard<'_, Client>) {
        let resp = binding.delete("/jobs/disabled").dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let res = resp.into_json::<HashMap<String, HashMap<String, String>>>()
            .await.expect("序列化失败");
        assert_eq!(res.get("Ok").unwrap().get("message").unwrap(), "flushed");
    }

    async fn test_update_mirror_status_of_a_existed_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>, mut status: MirrorStatus){
        let resp = binding.post(format!("/workers/{}/jobs/{}", status.worker, status.name))
            .json(&status)
            .dispatch().await;

        assert_eq!(resp.status(), Status::Ok);

        // 测试列出现有worker的镜像状态
        // list_status_of_an_existed_worker(&binding, &status).await;

        // 开始同步
        status.status = SyncStatus::PreSyncing;
        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let resp = binding.post(format!("/workers/{}/jobs/{}", status.worker, status.name))
            .json(&status)
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        
        // 更新镜像状态为PreSync -开始同步
        // update_mirror_status_to_presync_starting_sync(&binding, &status).await;
        
        // 列出所有worker的status
        // list_all_job_status_of_all_workers(&binding, &status).await;
        
        // 更新有效镜像的大小
        // update_size_of_a_valid_mirror(&binding, &status).await;
        
        // 更新有效镜像的时间表
        // update_schedule_of_valid_mirrors(&binding, &status).await;
        
        // 更新无效镜像的大小
        // update_size_of_invalid_mirrors(&binding, &status).await;
        
        // 如果status改变失败？
        status.status = SyncStatus::Failed;
        tokio::time::sleep(time::Duration::from_secs(3)).await;
        let resp = binding.post(format!("/workers/{}/jobs/{}", status.worker, status.name))
            .json(&status)
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        
        // 如果同步job失败？
        syncing_job_failed(&binding, &status).await;
        
    }

    async fn list_status_of_an_existed_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let resp = binding.get("/workers/test_worker1/jobs")
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let ms = resp.into_json::<HashMap<String, Vec<MirrorStatus>>>()
            .await.unwrap()
            .get("Ok").unwrap().clone();
        let m = ms.get(0).unwrap();
        assert_eq!(m.name, status.name);
        assert_eq!(m.worker, status.worker);
        assert_eq!(m.status, status.status);
        assert_eq!(m.upstream, status.upstream);
        assert_eq!(m.size, status.size);
        assert_eq!(m.is_master, status.is_master);
        assert!((Utc::now() - m.last_update) < Duration::seconds(1));
        assert_eq!(m.last_started, Utc.timestamp(0, 0));
        assert!((Utc::now() - m.last_ended) < Duration::seconds(1));
    }
    async fn update_mirror_status_to_presync_starting_sync(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let resp = binding.get("/workers/test_worker1/jobs")
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let ms = resp.into_json::<HashMap<String, Vec<MirrorStatus>>>()
            .await.unwrap()
            .get("Ok").unwrap().clone();
        let m = ms.get(0).unwrap();
        assert_eq!(m.name, status.name);
        assert_eq!(m.worker, status.worker);
        assert_eq!(m.status, status.status);
        assert_eq!(m.upstream, status.upstream);
        assert_eq!(m.size, status.size);
        assert_eq!(m.is_master, status.is_master);
        assert!((Utc::now() - m.last_update) < Duration::seconds(3));
        assert!((Utc::now() - m.last_update) > Duration::seconds(1));
        assert!((Utc::now() - m.last_started) < Duration::seconds(2));
        assert!((Utc::now() - m.last_ended) < Duration::seconds(3));
        assert!((Utc::now() - m.last_ended) > Duration::seconds(1));
    }
    async fn list_all_job_status_of_all_workers(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let resp = binding.get("/jobs").dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let ms = resp.into_json::<HashMap<String, Vec<WebMirrorStatus>>>()
            .await.unwrap()
            .get("Ok").unwrap().clone();
        let m = ms.get(0).unwrap();
        assert_eq!(m.name, status.name);
        assert_eq!(m.status, status.status);
        assert_eq!(m.upstream, status.upstream);
        assert_eq!(m.size, status.size);
        assert_eq!(m.is_master, status.is_master);
        assert!((Utc::now() - m.last_update.0) < Duration::seconds(3));
        assert!((Utc::now() - m.last_started.0) < Duration::seconds(2));
        assert!((Utc::now() - m.last_ended.0) < Duration::seconds(3));
        
        
    }
    async fn update_size_of_a_valid_mirror(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let msg = SizeMsg{
            name: status.name.clone(),
            size: "5GB".to_string(),
        };
        let resp = binding.post(format!("/workers/{}/jobs/{}/size", status.worker, status.name))
            .json(&msg)
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        
        // 获得镜像的新大小 
        let get_new_size_of_a_mirror = async {  
            let resp = binding.get("/workers/test_worker1/jobs")
                .dispatch().await;
            assert_eq!(resp.status(), Status::Ok);
            let ms = resp.into_json::<HashMap<String, Vec<MirrorStatus>>>()
                .await.unwrap()
                .get("Ok").unwrap().clone();
            let m = ms.get(0).unwrap();
            assert_eq!(m.name, status.name);
            assert_eq!(m.worker, status.worker);
            assert_eq!(m.status, status.status);
            assert_eq!(m.upstream, status.upstream);
            assert_eq!(m.size, "5GB".to_string());
            assert_eq!(m.is_master, status.is_master);
            assert!((Utc::now() - m.last_update) < Duration::seconds(3));
            assert!((Utc::now() - m.last_started) < Duration::seconds(2));
            assert!((Utc::now() - m.last_ended) < Duration::seconds(3));
            
        };
        
        get_new_size_of_a_mirror.await
        
        
    }
    async fn update_schedule_of_valid_mirrors(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let mut schedules = vec![];
        schedules.push(MirrorSchedule{
            mirror_name: "arch-sync1".to_string(),
            next_schedule: Utc::now() + Duration::minutes(10),
            ..MirrorSchedule::default()
        });
        schedules.push(MirrorSchedule{
            mirror_name: "arch-sync2".to_string(),
            next_schedule: Utc::now() + Duration::minutes(7),
            ..MirrorSchedule::default()
        });
        let msg = MirrorSchedules{
            schedules
        };
        let resp = binding.post(format!("/workers/{}/schedules", status.worker))
            .json(&msg)
            .dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
    }
    async fn update_size_of_invalid_mirrors(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let msg = SizeMsg{
            name: "Invalid mirror".to_string(),
            size: "5GB".to_string(),
        };
        let resp = binding.post(format!("/workers/{}/jobs/{}/size", status.worker, status.name))
            .json(&msg)
            .dispatch().await;
        assert_eq!(resp.status(), Status::InternalServerError);
    }
    async fn syncing_job_failed(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: &MirrorStatus){
        let resp = binding.get("/workers/test_worker1/jobs").dispatch().await;
        assert_eq!(resp.status(), Status::Ok);
        let ms = resp.into_json::<HashMap<String, Vec<MirrorStatus>>>()
            .await.unwrap()
            .get("Ok").unwrap().clone();
        let m = ms.get(0).unwrap();
        assert_eq!(m.name, status.name);
        assert_eq!(m.worker, status.worker);
        assert_eq!(m.status, status.status);
        assert_eq!(m.upstream, status.upstream);
        assert_eq!(m.size, status.size);
        assert_eq!(m.is_master, status.is_master);
        assert!((Utc::now() - m.last_update) > Duration::seconds(3));
        assert!((Utc::now() - m.last_started) > Duration::seconds(2));
        assert!((Utc::now() - m.last_ended) < Duration::seconds(1));
    }

    async fn test_update_mirror_status_of_a_nonexistent_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>, status: MirrorStatus){
        let resp = binding.post(format!("/workers/{}/jobs/{}", status.worker, status.name))
            .json(&status)
            .dispatch().await;
        assert_eq!(resp.status(), Status::BadRequest);
        // let msg = resp.into_string().await.unwrap();
        // assert_eq!(msg, "");
        let msg = resp.into_json::<HashMap<String, HashMap<String, String>>>()
            .await.unwrap()
            .get("Err").unwrap().clone();
        assert_eq!(msg.get("error").unwrap(), &format!("无效的worker_id: {}", status.worker));
    }
    
    async fn test_update_schedule_of_an_nonexistent_worker(binding: &tokio::sync::RwLockReadGuard<'_, Client>,invalid_worker: String, sch: MirrorSchedules){
        let resp = binding.post(format!("/workers/{invalid_worker}/schedules"))
            .json(&sch)
            .dispatch().await;
        assert_eq!(resp.status(), Status::BadRequest);
        let msg = resp.into_json::<HashMap<String, HashMap<String, String>>>()
            .await.unwrap()
            .get("Err").unwrap().clone();
        
        assert_eq!(msg.get("error").unwrap(), &format!("无效的worker_id: {invalid_worker}"));
    }


    use rand;
    use rand::Rng;
    use tokio::sync::mpsc::Receiver;

    // 处理客户端命令
    // 用rocket::local::asynchronous::Client启动的服务器不能接收reqwest客户端发出的请求
    // 所以使用Rocket::new().lunch()正常启动服务器
    #[rocket::async_test]
    async fn test_handle_client_command(){
        let mut listen_port = 5000;
        listen_port += 1;
        let port = listen_port;
        let addr = "127.0.0.1".to_string();
        let base_url = format!("http://{}:{}", addr, port);
        init_logger(true, true, false);
        let mut s = get_rtsync_manager(&Config{
            debug: true,
            ..Config::default()
        }).unwrap();
        s.cfg.server.addr = Some(addr);
        s.cfg.server.port = Some(port);
        let worker_status_map = Arc::new(RwLock::new(HashMap::default()));
        worker_status_map.write().unwrap().insert(
            _MAGIC_BAD_WORKER_ID.to_string(),
            WorkerStatus{ id: _MAGIC_BAD_WORKER_ID.to_string(), ..WorkerStatus::default() });

        s.engine = s.engine.manage(Box::new(MockDbAdapter{
            worker_store: worker_status_map,
            status_store: Arc::new(RwLock::new(HashMap::new())),
        }) as Box<dyn DbAdapter>);

        s.engine = s.engine.manage(reqwest::Client::new());
        let rocket_handle = tokio::spawn(async move {
            s.run().launch().await.expect("Rocket launch failed");
        });
        tokio::time::sleep(time::Duration::from_secs(1)).await;

        let resp: HashMap<String, String> = get_json(&format!("{base_url}/ping"), Arc::new(None)).await.unwrap();
        assert_eq!(resp.get("message").unwrap(), "pong");
        
        let w = WorkerStatus{
            id: "test_worker1".to_string(),
            ..WorkerStatus::default()
        };
        
        let resp = post_json(&format!("{base_url}/workers"), &w, Arc::new(None)).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        

        let (tx, rx) = mpsc::channel::<WorkerCmd>(1);
        let mut worker_server = make_mock_worker_server(tx);
        let mut figment = worker_server.figment().clone();
        let addr = "127.0.0.1";
        let port = rand::thread_rng().gen_range(0..10000) + 30000;
        let worker_base_url = format!("http://{}:{}", addr, port);
        figment = figment
            .merge((rocket::Config::ADDRESS, addr))
            .merge((rocket::Config::PORT, port));
        worker_server = worker_server.configure(figment);
        
        let w = WorkerStatus{
            id: "test_worker_cmd".to_string(),
            url: format!("{worker_base_url}/cmd"),
            ..WorkerStatus::default()
        };
        let resp = post_json(&format!("{base_url}/workers"), &w, Arc::new(None)).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        
        let rocket_handle2 = tokio::spawn(async move {
            worker_server.launch().await.expect("Rocket launch failed");
        });
        let resp: HashMap<String, String> = get_json(&format!("{worker_base_url}/ping"), Arc::new(None)).await.unwrap();
        assert_eq!(resp.get("message").unwrap(), "pong");
        

        let url = format!("{base_url}/cmd");
        
        // 当客户端发送了错误命令
        // client_send_wrong_cmd(&url).await;
        
        // 当客户端发送正确命令
        client_send_correct_cmd(&url, rx).await;
    }
    async fn client_send_wrong_cmd(url: &str){
        let client_cmd = ClientCmd{
            cmd: CmdVerb::Start,
            mirror_id: "ubuntu-sync".to_string(),
            worker_id: "no_exist_worker".to_string(),
            ..ClientCmd::default()
        };
        let resp = post_json(url, &client_cmd, Arc::new(None)).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    
    async fn client_send_correct_cmd(url: &str, mut rx: Receiver<WorkerCmd>){
        let client_cmd = ClientCmd{
            cmd: CmdVerb::Start,
            mirror_id: "ubuntu-sync".to_string(),
            worker_id: "test_worker_cmd".to_string(),
            ..ClientCmd::default()
        };
        let resp = post_json(url, &client_cmd, Arc::new(None)).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        match rx.recv().await{
            Some(cmd) => {
                assert_eq!(cmd.cmd, client_cmd.cmd);
                assert_eq!(cmd.mirror_id, client_cmd.mirror_id);
            }
            None => {
                panic!("没有收到");
            }
        }
        
    }
    
}

