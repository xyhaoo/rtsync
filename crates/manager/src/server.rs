use std::sync::RwLock;
use chrono::Utc;
use reqwest::Client;
use crate::config::Config;
use rocket::{Build, Rocket, State};
use internal::util::{create_http_client, post_json};
use crate::db::{make_db_adapter, DbAdapter};

use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::serde::{Serialize, Deserialize};
use internal::{
    msg::MirrorStatus,
    status_web::WebMirrorStatus,
};
use internal::msg::{ClientCmd, CmdVerb, MirrorSchedules, WorkerCmd, WorkerStatus};
use internal::status::SyncStatus::{Disabled, Failed, Paused, PreSyncing, Success, Syncing};
use internal::status_web::build_web_mirror_status;
use crate::middleware::{CheckWorkerId, ContextErrorLogger};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Response{
    Message(String),
    Error(String),
}


// 一个Manager代表一个manager服务器
pub struct Manager{
    pub(crate) cfg: Config,
    pub(crate) engine: Rocket<Build>,
}
impl Manager {
    pub async fn run(mut self){
        let mut figment = self.engine.figment().clone();
        if let (Some(addr), Some(port)) = (self.cfg.server.addr, self.cfg.server.port){
            figment = figment
                .merge((rocket::Config::PORT, port))
                .merge((rocket::Config::ADDRESS, addr))
        }
        if let (Some(cert), Some(key)) = (self.cfg.server.ssl_cert, self.cfg.server.ssl_key){
            figment = figment
                .merge(("tls.certs", cert))
                .merge(("tls.key", key));
        }
        self.engine = self.engine.configure(figment);
        
        self.engine.launch().await.unwrap();
    }

}

pub fn get_rtsync_manager(cfg: &Config) -> Result<Manager, String>{
    let mut s = Manager{
        cfg: cfg.clone(),
        engine: Rocket::build(),
    };
    if let Some(ca_cert) = &cfg.files.ca_cert{
        if !ca_cert.is_empty(){
            match create_http_client(Some(ca_cert)){
                Ok(client) => {
                    s.engine = s.engine.manage(client);
                }
                Err(e) => {
                    let err = format!("初始化http 客户端失败: {}", e);
                    log::error!("{}", err);
                    return Err(err);
                }
            }
        }
    }
    if let (Some(db_type), Some(db_file)) = (&cfg.files.db_type, &cfg.files.db_file){
        if !db_file.is_empty() && !db_type.is_empty(){
            match make_db_adapter(db_type, db_file) {
                Ok(adapter) => {
                    s.engine = s.engine.manage(Box::new(adapter) as Box<dyn DbAdapter>);
                }
                Err(e) => {
                    let err = format!("初始化数据库适配器(db adapter)失败: {}", e);
                    log::error!("{}", err);
                    return Err(err);
                }
            }
        }
    }

    s.engine = s.engine.attach(ContextErrorLogger);

    s.engine = s.engine.mount("/", routes![
        ping,
        list_all_jobs,
        flush_disabled_jobs,
        list_workers,
        register_worker,
        delete_worker,
        list_jobs_of_worker,
        update_job_of_worker,
        update_mirror_size,
        update_schedules_of_worker,
        handle_client_cmd,
    ]);

    Ok(s)
}

#[get("/ping")]
pub(crate) async fn ping() -> Json<Response> {
    Json(Response::Message("pong".into()))
}

// list_all_jobs返回指定worker的所有job
#[get("/jobs")]
async fn list_all_jobs(engine: &State<Box<dyn DbAdapter>>)
    -> Result<Json<Vec<WebMirrorStatus>>, (Status, Json<Response>)>
{
    match engine.list_all_mirror_states(){
        Ok(mirror_status_list) => {
            let mut web_mir_status_list: Vec<WebMirrorStatus> = vec![];
            for m in mirror_status_list{
                web_mir_status_list.push(build_web_mirror_status(m))
            }
            Ok(Json(web_mir_status_list))
        },
        Err(e) => {
            let error = format!("在列出所有的镜像的过程中失败：{}", e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }
}

// flush_disabled_jobs删除所有被标记为deleted的job
#[delete("/jobs/disabled")]
async fn flush_disabled_jobs(adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<Response>, (Status, Json<Response>)>
{
    if let Err(e) = adapter.flush_disabled_jobs(){
        let error = format!("未能刷新已禁用的jobs：{}", e);
        error!("{}", error);
        return Err((Status::InternalServerError, Json(Response::Error(error))))
    }
    Ok(Json(Response::Message ("flushed".into())))
}

// list_workers使用所有worker的信息进行响应
#[get("/workers")]
async fn list_workers(adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<Vec<WorkerStatus>>, (Status, Json<Response>)>
{
    let mut worker_infos: Vec<WorkerStatus> = vec![];
    match adapter.list_workers() {
        Ok(workers) => {
            for w in workers{
                worker_infos.push(WorkerStatus{
                    id: w.id,
                    url: w.url,
                    token: "REDACTED".to_string(),
                    last_online: w.last_online,
                    last_register: w.last_register,
                });
            };
            Ok(Json(worker_infos))
        }
        Err(e) => {
            let error = format!("在列出所有的worker的过程中失败：{}", e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }
}

// register_worker注册一个新在线的worker
#[post("/workers", format = "application/json", data = "<worker>")]
async fn register_worker(mut worker: Json<WorkerStatus>, adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<WorkerStatus>, (Status, Json<Response>)>
{
    worker.last_online = Utc::now();
    worker.last_register = Utc::now();
    match adapter.create_worker(worker.into_inner()){
        Ok(new_worker) => {
            info!("注册了Worker: {}",new_worker.id);
            Ok(Json(new_worker))
        }
        Err(e) => {
            let error = format!("注册worker失败：{}", e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }
}

// delete_worker根据worker_id删除一个worker
#[delete("/workers/<id>")]
async fn delete_worker(id: &str,
                       guard: Result<CheckWorkerId, Json<Response>>,
                       adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<Response>, (Status, Json<Response>)>
{
    if let Err(e) = guard{
        return Err((Status::BadRequest, e))
    }
    match adapter.delete_worker(id) {
        Ok(_) => {
            info!("删除了worker，id为{}",id);
            Ok(Json(Response::Message ("deleted".to_owned())))
        }
        Err(e) => {
            let error = format!("删除worker失败：{}", e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }
}

// list_jobs_of_worker返回指定worker的所有同步任务
#[get("/workers/<id>/jobs")]
async fn list_jobs_of_worker(id: &str,
                             guard: Result<CheckWorkerId, Json<Response>>,
                             adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<Vec<MirrorStatus>>, (Status, Json<Response>)>
{
    if let Err(e) = guard{
        return Err((Status::BadRequest, e))
    }
    match adapter.list_mirror_states(id) {
        Ok(mirror_status_list) => {
            Ok(Json(mirror_status_list))
        },
        Err(e) => {
            let error = format!("在列出worker_id为{}的worker的所有job时失败：{}", id, e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }
}

#[post("/workers/<id>/jobs/<_job>", format = "application/json", data = "<status>")]
async fn update_job_of_worker(id: &str,
                              _job: &str,
                              guard: Result<CheckWorkerId, Json<Response>>,
                              mut status: Json<MirrorStatus>,
                              adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<MirrorStatus>, (Status, Json<Response>)>
{
    if let Err(e) = guard{
        return Err((Status::BadRequest, e))
    }
    
    let mirror_name = status.name.clone();
    if mirror_name.len() == 0{
        return Err((Status::BadRequest, Json(Response::Error ("镜像名为空".to_string()))))
    }
    let _ = adapter.refresh_worker(id);
    let cur_status = adapter.get_mirror_status(id, &mirror_name).unwrap_or_default();

    let cur_time = Utc::now();
    if status.status == PreSyncing && cur_status.status != PreSyncing {
        status.last_started = cur_time;
    }else{
        status.last_started = cur_status.last_started;
    }
    // 只有同步成功时才需要更新last_update
    if status.status == Success{
        status.last_update = cur_time;
    }else {
        status.last_update = cur_status.last_update;
    }
    if status.status == Success || status.status == Failed{
        status.last_ended = cur_time;
    } else {
        status.last_ended = cur_status.last_ended;
    }

    // 只有大小有意义的消息才会更新镜像大小
    if cur_status.size.len() > 0 && cur_status.size.ne("unknown"){
        if status.size.len() == 0 || status.size.eq("unknown"){
            status.size = cur_status.size;
        }
    }

    // 打印日志
    match status.status {
        Syncing => {
            info!("job [{}] @<{}> 开始同步", status.name, status.worker);
        }
        _ => {
            info!("job [{}] @<{}> {}", status.name, status.worker, status.status);
        }
    }

    match adapter.update_mirror_status(id, &mirror_name, status.into_inner()){
        Ok(new_status) => {
            Ok(Json(new_status))
        }
        Err(e) => {
            let error = format!("更新任务 {} 失败，所属worker {} :{}", mirror_name, id, e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
    }

}

#[derive(Serialize, Deserialize)]
pub(crate) struct SizeMsg{
    pub(crate) name: String,
    pub(crate) size: String,
}
#[post("/workers/<id>/jobs/<_job>/size", format = "application/json", data = "<msg>")]
async fn update_mirror_size(id: &str,
                            _job: &str,
                            guard: Result<CheckWorkerId, Json<Response>>,
                            msg: Json<SizeMsg>,
                            adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<MirrorStatus>, (Status, Json<Response>)>
{
    if let Err(e) = guard{
        return Err((Status::BadRequest, e))
    }
    let mirror_name = msg.name.clone();
    let _ = adapter.refresh_worker(id);
    match adapter.get_mirror_status(id, &mirror_name){
        Err(e) => {
            let error = format!("获取镜像{} @<{}>的状态失败:{}", mirror_name, id, e);
            error!("{}", error);
            Err((Status::InternalServerError, Json(Response::Error(error))))
        }
        Ok(mut status) => {
            // 只有大小有意义的消息才会更新镜像大小
            if msg.size.len() >0 || msg.size.ne("unknown"){
                status.size = msg.size.clone();
            }
            info!("镜像[{}] @<{}> 大小: {}", status.name, status.worker, status.size);

            match adapter.update_mirror_status(id, &mirror_name, status) {
                Ok(new_status) => {
                    Ok(Json(new_status))
                }
                Err(e) => {
                    let error = format!("更新任务 {} 失败，所属worker {} :{}", mirror_name, id, e);
                    error!("{}", error);
                    Err((Status::InternalServerError, Json(Response::Error(error))))
                }
            }
        }

    }
}

// 更新worker同步任务的同步时间
#[post("/workers/<id>/schedules", format = "application/json", data = "<schedules>")]
async fn update_schedules_of_worker(id: &str,
                                    guard: Result<CheckWorkerId, Json<Response>>,
                                    schedules: Json<MirrorSchedules>,
                                    adapter: &State<Box<dyn DbAdapter>>)
    -> Result<Json<()>, (Status, Json<Response>)>
{
    if let Err(e) = guard{
        return Err((Status::BadRequest, e))
    }
    for schedule in schedules.into_inner().schedules{
        let mirror_name = schedule.mirror_name;
        if mirror_name.len() == 0{
            let error = "镜像名为空".to_string();
            error!("{}", error);
            return Err((Status::BadRequest, Json(Response::Error(error))))
        }

        let _ = adapter.refresh_worker(id);
        match adapter.get_mirror_status(id, &mirror_name){
            Err(e) => {
                error!("获取job {} 失败，所属worker {} : {}", mirror_name, id, e);
                continue;
            }
            Ok(mut cur_status) => {
                if cur_status.scheduled == schedule.next_schedule{
                    // 无需改变，跳过更新
                    continue;
                }

                cur_status.scheduled = schedule.next_schedule;

                if let Err(e) = adapter.update_mirror_status(id, &mirror_name, cur_status){
                    let error = format!("更新任务 {} 失败，所属worker {} :{}", mirror_name, id, e);
                    error!("{}", error);
                    return Err((Status::InternalServerError, Json(Response::Error(error))))
                }
            }
        }
    }

    Ok(Json(()))
}

#[post("/cmd", format = "application/json", data = "<client_cmd>")]
async fn handle_client_cmd(client_cmd: Json<ClientCmd>,
                           adapter: &State<Box<dyn DbAdapter>>,
                           client: &State<Client>)
    -> Result<Json<Response>, (Status, Json<Response>)>
{
    let client_cmd = client_cmd.into_inner();
    let worker_id = client_cmd.worker_id.clone();
    if worker_id.len() == 0{
        // TODO：当worker_id为空字符串时，决定哪个worker应该执行此镜像
        let error = "worker_id为空的情况还未实现".to_string();
        error!("{}", error);
        return Err((Status::InternalServerError, Json(Response::Error(error))))
    }
    
    let w = adapter.get_worker(&worker_id);
    if let Err(e) = w {
        let error = format!("worker{}还未注册", worker_id);
        error!("{}", error);
        return Err((Status::BadRequest, Json(Response::Error(error))))
    }
    
    let w = w.unwrap();
    let worker_url = w.url;
    // 把client cmd解析为worker cmd
    let worker_cmd = WorkerCmd{
        cmd: client_cmd.cmd,
        mirror_id: client_cmd.mirror_id.clone(),
        args: client_cmd.args,
        options: client_cmd.options,
    };
    // 更新作业状态，即使作业没有成功禁用, 此状态也应设置为禁用
    let mut cur_stat = adapter.get_mirror_status(&client_cmd.worker_id, &client_cmd.mirror_id).unwrap_or_default();
    let mut changed = false;
    match client_cmd.cmd {
        CmdVerb::Disable => {
            cur_stat.status = Disabled;
            changed = true;
        },
        CmdVerb::Stop => {
            cur_stat.status = Paused;
            changed = true;
        },
        _ => {},
    }
    if changed{
        let _ = adapter.update_mirror_status(&client_cmd.worker_id, &client_cmd.mirror_id, cur_stat);
    }

    info!("对<{}>发送命令'{} {}'", client_cmd.worker_id, client_cmd.cmd, client_cmd.mirror_id);
    let http_client = client.inner().clone();
    if let Err(e) = post_json(&worker_url, &worker_cmd, Some(http_client)).await{
        let error = format!("为worker {}({}) 发送命令失败：{}", worker_id, worker_url, e.to_string());
        error!("{}", error);
        return Err((Status::InternalServerError, Json(Response::Error(error))))
    }
    // TODO: 检查响应是否成功
    Ok(Json(Response::Message(format!("成功发送命令到worker {}", worker_id))))
}



