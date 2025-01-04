#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use rocket::serde::json::Json;
    use rocket::{get, post, routes, Build, Rocket, State};
    use rocket::http::Status;
    use tokio::sync::mpsc::{channel, Receiver, Sender};
    use internal::msg::{CmdVerb, MirrorSchedules, MirrorStatus, WorkerCmd, WorkerStatus};
    use chrono::{DateTime, Utc};
    use log::{debug, info};
    use internal::logger::init_logger;
    use internal::status::SyncStatus;
    use internal::util::{create_http_client, post_json};
    use crate::config::{Config, GlobalConfig, ManagerConfig, MirrorConfig, ProviderEnum, ServerConfig};
    use crate::worker::*;

    const MANAGER_PORT: u16 = 5001;
    const WORKER_PORT: u16 = 5002;

    #[allow(unused)]
    enum SendType{
        WorkerStatus(WorkerStatus),
        MirrorSchedules(MirrorSchedules),
        MirrorStatus(MirrorStatus),
    }
    #[derive(rocket::serde::Serialize)]
    struct Response{
        _info_key: String,
    }

    #[get("/ping")]
    fn ping() -> (Status, Json<Response>) {
        (Status::Ok, Json(Response {_info_key: "pong".to_string()}))
    }

    #[post("/workers", format = "application/json", data = "<worker>")]
    async fn send_worker_status(mut worker: Json<WorkerStatus>, recv_data: &State<Sender<SendType>>) -> (Status, Json<WorkerStatus>) {
        worker.last_online = Utc::now();
        worker.last_register = Utc::now();
        let worker = worker.into_inner();
        recv_data.send(SendType::WorkerStatus(worker.clone())).await.unwrap();
        (Status::Ok, Json(worker))
    }

    #[post("/workers/dut/schedules", format = "application/json", data = "<sch>")]
    async fn send_mirror_schedules(sch: Json<MirrorSchedules>, recv_data: &State<Sender<SendType>>) -> (Status, Json<()>) {
        recv_data.send(SendType::MirrorSchedules(sch.into_inner())).await.unwrap();
        (Status::Ok, Json(()))
    }

    #[post("/workers/dut/jobs/<job>", format = "application/json", data = "<status>")]
    async fn send_mirror_status(job: &str, status: Json<MirrorStatus>, recv_data: &State<Sender<SendType>>) -> (Status, Json<MirrorStatus>) {
        let status = status.into_inner();
        recv_data.send(SendType::MirrorStatus(status.clone())).await.unwrap();
        (Status::Ok, Json(status))
    }

    #[get("/workers/dut/jobs")]
    fn get_mirror_status() -> (Status, Json<Vec<MirrorStatus>>) {
        let mirror_status_list = vec![];
        (Status::Ok, Json(mirror_status_list))
    }

    fn make_mock_manager_server(recv_data_tx: Sender<SendType>) -> Rocket<Build> {
        let r = Rocket::build()
            .mount("/",
                   routes![ping, send_worker_status, send_mirror_status, send_mirror_schedules, get_mirror_status])
            .manage(recv_data_tx);
        r
    }

    async fn send_command_to_worker(worker_url: &str,
                              http_client: Arc<Option<reqwest::Client>>,
                              cmd: CmdVerb,
                              mirror_id: String)
    {
        let worker_cmd = WorkerCmd{
            cmd,
            mirror_id,
            args: vec![],
            options: Default::default(),
        };
        debug!("POST to {} with cmd {}", worker_url, cmd);
        assert!(post_json(worker_url, &worker_cmd, http_client).await.is_ok());
    }


    #[tokio::test]
    async fn test_worker(){
        init_logger(false, true, false);

        let (recv_data_chan_tx, recv_data_chan_rx) = channel(1);
        let mut s = make_mock_manager_server(recv_data_chan_tx);
        let figment = s.figment().clone()
            .merge((rocket::Config::PORT, MANAGER_PORT))
            .merge((rocket::Config::ADDRESS, "127.0.0.1"));

        s = s.configure(figment);
        tokio::spawn(async move {
            s.launch().await.unwrap();
        });
        // Wait for http server starting
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let http_client = create_http_client(None).unwrap();
        let http_client = Arc::new(Some(http_client));

        let worker_port = WORKER_PORT + 1;

        let worker_cfg = Config{
            global: GlobalConfig{
                name: Some("dut".to_string()),
                log_dir: Some("/tmp".to_string()),
                mirror_dir: Some("/tmp".to_string()),
                concurrent: Some(2),
                interval: Some(1),
                ..GlobalConfig::default()
            },
            server: ServerConfig{
                hostname: Some("localhost".to_string()),
                listen_addr: Some("127.0.0.1".to_string()),
                listen_port: Some(worker_port as usize),
                ..ServerConfig::default()
            },
            manager: ManagerConfig{
                api_base: Some(format!("http://localhost:{}", MANAGER_PORT)),
                ..ManagerConfig::default()
            },
            ..Config::default()
        };
        debug!("worker port {}", worker_port);

        // with_no_job(worker_cfg, http_client, recv_data_chan_rx).await
        // with_one_job(worker_cfg, http_client, recv_data_chan_rx).await
        with_several_jobs(worker_cfg, http_client, recv_data_chan_rx).await
    }

    async fn with_no_job(cfg: Config, http_client: Arc<Option<reqwest::Client>>, mut recv_data_chan_rx: Receiver<SendType>) {
        let mut exited_chan = channel::<i32>(1);
        let mut w = Worker::new(cfg).await.unwrap();
        let w_clone = w.clone();
        tokio::spawn(async move {
            w_clone.run().await;
        });
        exited_chan.0.send(1).await.unwrap();

        /////////////////////////////////////
        let mut registered = false;
        loop {
            tokio::select! {
                Some(data) = recv_data_chan_rx.recv() => {
                    match data {
                        SendType::WorkerStatus(worker_status) => {
                            assert_eq!(worker_status.id, "dut");
                            registered = true;
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            send_command_to_worker(&*worker_status.url, http_client.clone(), CmdVerb::Start, "foobar".to_string()).await;
                        },
                        SendType::MirrorSchedules(mirror_schedules) => {
                            assert_eq!(mirror_schedules.schedules.len(), 0);
                        },
                        _ => {}
                    }
                },
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    assert_eq!(registered, true);
                    break;
                }

            }
        }
        /////////////////////////////////////
        w.halt().await;
        tokio::select! {
            Some(exited) = exited_chan.1.recv() => {
                assert_eq!(exited, 1);
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                assert_eq!(0, 1);
            }
        }
    }

    async fn with_one_job(mut cfg: Config, http_client: Arc<Option<reqwest::Client>>, mut recv_data_chan_rx: Receiver<SendType>){
        cfg.mirrors = vec![MirrorConfig{
            name: Some("job-ls".to_string()),
            provider: Some(ProviderEnum::Command),
            command: Some("ls".to_string()),
            ..MirrorConfig::default()
        }];
        
        let mut exited_chan = channel::<i32>(1);
        let mut w = Worker::new(cfg).await.unwrap();
        let w_clone = w.clone();
        tokio::spawn(async move {
            w_clone.run().await;
        });
        exited_chan.0.send(1).await.unwrap();
        /////////////////////////////////////
        let mut url = "".to_string();
        let mut job_running = false;
        let mut last_status = SyncStatus::None;
        loop {
            tokio::select! {
                Some(data) = recv_data_chan_rx.recv() => {
                    match data {
                        SendType::WorkerStatus(worker_status) => {
                            assert_eq!(worker_status.id, "dut");
                            url = worker_status.url.clone();
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            send_command_to_worker(&url, http_client.clone(), CmdVerb::Start, "job-ls".to_string()).await;
                        },
                        SendType::MirrorSchedules(mirror_schedules) => {
                            if !job_running {
                                assert_eq!(mirror_schedules.schedules.len(), 1);
                                assert_eq!(mirror_schedules.schedules[0].mirror_name, "job-ls");
                                let time = Utc::now();
                                assert!(mirror_schedules.schedules[0].next_schedule > time - chrono::Duration::seconds(2));
                                assert!(mirror_schedules.schedules[0].next_schedule < time + chrono::Duration::minutes(1));
                            }
                        },
                        SendType::MirrorStatus(mirror_status) => {
                            info!("Job {} status {}", mirror_status.name, mirror_status.status);
                            if mirror_status.status == SyncStatus::PreSyncing || mirror_status.status == SyncStatus::Syncing{
                                job_running = true;
                            }else{
                                job_running = false;
                            }
                            assert_ne!(mirror_status.status, SyncStatus::Failed);
                            last_status = mirror_status.status;
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    assert_ne!(url, "");
                    assert_eq!(job_running, false);
                    assert_eq!(last_status, SyncStatus::Success);
                    break;
                }
            }
        }
        /////////////////////////////////////
        w.halt().await;
        tokio::select! {
            Some(exited) = exited_chan.1.recv() => {
                assert_eq!(exited, 1);
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                assert_eq!(0, 1);
            }
        }
    }
    
    async fn with_several_jobs(mut cfg: Config, http_client: Arc<Option<reqwest::Client>>, mut recv_data_chan_rx: Receiver<SendType>){
        cfg.mirrors = vec![
            MirrorConfig{
                name: Some("job-ls-1".to_string()),
                provider: Some(ProviderEnum::Command),
                command: Some("ls".to_string()),
                ..MirrorConfig::default()
            },
            MirrorConfig{
                name: Some("job-fail".to_string()),
                provider: Some(ProviderEnum::Command),
                command: Some("non-existent-command-xxxx".to_string()),
                ..MirrorConfig::default()
            },
            MirrorConfig{
                name: Some("job-ls-2".to_string()),
                provider: Some(ProviderEnum::Command),
                command: Some("ls".to_string()),
                ..MirrorConfig::default()
            },
        ];
        let mut exited_chan = channel::<i32>(1);
        let mut w = Worker::new(cfg).await.unwrap();
        let w_clone = w.clone();
        tokio::spawn(async move {
            w_clone.run().await;
        });
        exited_chan.0.send(1).await.unwrap();
        /////////////////////////////////////
        let mut url = "".to_string();
        let mut last_status: HashMap<String, SyncStatus> = HashMap::new();
        let mut next_sch:HashMap<String, DateTime<Utc>> = HashMap::new();
        loop {
            tokio::select! {
                Some(data) = recv_data_chan_rx.recv() => {
                    match data {
                        SendType::WorkerStatus(worker_status) => {
                            assert_eq!(worker_status.id, "dut");
                            url = worker_status.url.clone();
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            send_command_to_worker(&url, http_client.clone(), CmdVerb::Start, "job-fail".to_string()).await;
                            send_command_to_worker(&url, http_client.clone(), CmdVerb::Start, "job-ls-1".to_string()).await;
                            send_command_to_worker(&url, http_client.clone(), CmdVerb::Start, "job-ls-2".to_string()).await;
                        },
                        SendType::MirrorSchedules(mirror_schedules) => {
                            for item in mirror_schedules.schedules.iter() {
                                next_sch.insert(item.mirror_name.clone(), item.next_schedule);
                            }
                        },
                        SendType::MirrorStatus(mirror_status) => {
                            info!("Job {} status {}", mirror_status.name, mirror_status.status);
                            let job_running;
                            if mirror_status.status == SyncStatus::PreSyncing || mirror_status.status == SyncStatus::Syncing{
                                job_running = true;
                            }else{
                                job_running = false;
                            }
                            if !job_running {
                                if mirror_status.name == "job-fail" {
                                    assert_eq!(mirror_status.status, SyncStatus::Failed);
                                }else{
                                    assert_ne!(mirror_status.status, SyncStatus::Failed);
                                }
                            }
                            last_status.insert(mirror_status.name.clone(), mirror_status.status);
                        }
                    }
                },
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    assert_eq!(last_status.len(), 3);
                    assert_eq!(next_sch.len(), 3);
                    break;
                }
            }
        }
        /////////////////////////////////////
        w.halt().await;
        tokio::select! {
            Some(exited) = exited_chan.1.recv() => {
                assert_eq!(exited, 1);
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                assert_eq!(0, 1);
            }
        }
    }

}