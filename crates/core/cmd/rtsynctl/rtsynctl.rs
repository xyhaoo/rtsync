use std::{env, fs};
use std::collections::HashMap;
use std::process::exit;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use anyhow::{anyhow, Result};
use clap::ArgMatches;
use log::{debug, error, info};
use internal as rtsync;
use lazy_static::lazy_static;
use reqwest::{Client, StatusCode};
use tokio::sync::mpsc::channel;
use tera::{Context, Tera};
use serde_json::{json, to_string_pretty};


mod cli;
mod build;

const LIST_JOBS_PATH: &str = "/jobs";
const LIST_WORKERS_PATH: &str = "/workers";
const FLUSH_DISABLED_PATH: &str = "/jobs/disabled";
const CMD_PATH: &str = "/cmd";
const SYSTEM_CFG_FILE: &str = "/etc/rtsync/ctl.conf";   // 系统级的配置文件地址
const  USER_CFG_FILE: &str = "$HOME/.config/rtsync/ctl.conf";   // 用户级别的配置文件地址

lazy_static! {
    static ref BASE_URL: RwLock<String> = RwLock::new(String::new());
    static ref CLIENT: RwLock<Client> = RwLock::new(Client::new());
}

// 配置文件的序列化结构
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(default)]
struct Config {
    manager_addr: String,
    manager_port: u32,
    ca_cert: String,
}

fn load_config(cfg_file: &str, cfg: &mut Config) -> Result<()>{
    if !cfg_file.is_empty(){
        info!("加载配置文件: {}", cfg_file);
        let config_contents = fs::read_to_string(cfg_file)?;
        *cfg = toml::de::from_str(&config_contents)?;
    }
    Ok(())
}

async fn initialize(c: &ArgMatches) -> Result<()>{
    // init logger
    // rtsync::logger::init_logger(true, true, false);
    rtsync::logger::init_logger(c.get_flag("verbose"),
                                c.get_flag("debug"),
                                false);

    let mut cfg = Config::default();

    // default configs
    cfg.manager_addr = "localhost".to_string();
    cfg.manager_port = 14242;

    // 找到配置文件地址并导入。如果用户在命令行中使用参数指定，则使用这个配置文件
    if fs::exists(SYSTEM_CFG_FILE).is_ok_and(|b| b){
        load_config(SYSTEM_CFG_FILE, &mut cfg)?
    }
    
    let path = USER_CFG_FILE
        .replace("$HOME", &env::var("HOME").unwrap_or_default());
    debug!("用户的配置文件: {}", path);
    if fs::exists(path.as_str()).is_ok_and(|b| b){
        load_config(path.as_str(), &mut cfg)?;
    }
    
    if let Some(config) = c.get_one::<String>("config"){
        load_config(config.as_str(), &mut cfg)?;
    }

    // 使用命令行参数重写其他配置项
    if let Some(manager) = c.get_one::<String>("manager"){
        cfg.manager_addr = manager.clone();
    }
    if let Some(port) = c.get_one::<String>("port"){
        cfg.manager_port = port.parse::<u32>()?;
    }

    if let Some(ca_cert) = c.get_one::<String>("ca-cert"){
        cfg.ca_cert = ca_cert.clone();
    }

    // 解析 manager server 的 base url 
    let mut url_lock = BASE_URL.write().await;
    if !cfg.ca_cert.is_empty(){
        *url_lock = format!("https://{}:{}", &cfg.manager_addr, &cfg.manager_port);
    }else {
        *url_lock = format!("http://{}:{}", &cfg.manager_addr, &cfg.manager_port);
    }
    info!("使用manager地址: {}", *url_lock);
    drop(url_lock);

    // 创建 HTTP 客户端
    let mut client_lock = CLIENT.write().await;
    let ca_cert = match &cfg.ca_cert {
        ca_cert if !ca_cert.is_empty() => Some(ca_cert.clone()),
        _ => None,
    };
    match rtsync::util::create_http_client(ca_cert.as_deref()){
        Ok(client) => {
            *client_lock = client;
        }
        Err(e) => {
            error!("初始化 HTTP 服务器失败: {e}");
            return Err(anyhow!(e));
        }
    }

    Ok(())
}

async fn list_workers(c: &ArgMatches) -> Result<()>{
    let mut workers: Vec<rtsync::msg::WorkerStatus> = vec![];
    let client = CLIENT.read().await.clone();
    match rtsync::util::get_json(&format!("{}{}",
                                          *BASE_URL.read().await,
                                          LIST_WORKERS_PATH),
                                 Some(client)).await
    {
        Ok(resp) => {
            workers = resp;
        }
        Err(e) => {
            eprintln!("不能正确地从manager服务器获得信息: {}", e);
            exit(1);
        }
    }

    let data = json!(workers);
    let pretty_json = to_string_pretty(&data)?;
    println!("{}", pretty_json);
    Ok(())
}

async fn list_jobs(c: &ArgMatches) -> Result<()>{
    let mut generic_jobs_ms: Vec<rtsync::msg::MirrorStatus> = Vec::new();
    let mut generic_jobs_wms: Vec<rtsync::status_web::WebMirrorStatus> = Vec::new();
    
    if c.get_flag("all"){
        let mut jobs: Vec<rtsync::status_web::WebMirrorStatus> = vec![];
        let client = CLIENT.read().await.clone();
        match rtsync::util::get_json(&format!("{}{}",
                                              *BASE_URL.read().await,
                                              LIST_JOBS_PATH),
                                     Some(client)).await
        {
            Ok(resp) => {
                jobs = resp;
            }
            Err(e) => {
                eprintln!("不能正确地从 manager 服务器获得所有同步任务的信息: {}", e);
                exit(1);
            }
        }
        if let Some(status_str) = c.get_one::<String>("status"){
            let mut filtered_jobs: Vec<rtsync::status_web::WebMirrorStatus> =
                Vec::with_capacity(10);
            let mut statuses: Vec<rtsync::status::SyncStatus> = vec![];
            for s in status_str.split(","){
                match serde_json::from_str(format!("\"{}\"", s.trim()).as_str()) {
                    Ok(s) => {
                        let status: rtsync::status::SyncStatus = s;
                        statuses.push(status);
                    },
                    Err(e) => {
                        eprintln!("解析状态失败: {}", e);
                        exit(1);
                    }
                }
            }
            for job in jobs.iter(){
                for s in statuses.iter(){
                    if job.status == *s{
                        filtered_jobs.push(job.clone());
                        break;
                    }
                }
            }
            generic_jobs_wms.extend(filtered_jobs);
        }else {
            generic_jobs_wms.extend(jobs);
        }
    }else {
        let mut jobs: Vec<rtsync::msg::MirrorStatus> = vec![];
        let worker_ids: Vec<String> = c.get_many::<String>("WORKERS").unwrap()
            .into_iter()
            .map(String::from)
            .collect();
        let (ans_tx, mut ans_rx) =
            channel::<Vec<rtsync::msg::MirrorStatus>>(worker_ids.len());
        for worker_id in worker_ids.iter(){
            let ans_tx = ans_tx.clone();
            let client = CLIENT.read().await.clone();
            let worker_id = worker_id.clone();
            tokio::spawn(async move {
                let mut worker_jobs: Vec<rtsync::msg::MirrorStatus> = vec![];
                match rtsync::util::get_json(&format!("{}/workers/{}/jobs",
                                                      *BASE_URL.read().await,
                                                      worker_id),
                                             Some(client)).await
                {
                    Ok(resp) => {
                        worker_jobs = resp;
                    },
                    Err(e) => {
                        error!("获取 WORKER {} 的同步任务失败: {}",worker_id, e);
                    }
                }
                ans_tx.send(worker_jobs).await.unwrap();
            });
        }
        for _ in worker_ids.iter(){
            let job = ans_rx.recv().await.unwrap();
            if job.is_empty(){
                eprintln!("不能从至少一个manager正确获取同步作业信息");
                exit(1);
            }
            jobs.extend(job);
        }
        generic_jobs_ms.extend(jobs);
    }

    if let Some(format) = c.get_one::<String>("format"){
        let mut tera = Tera::default();
        if !generic_jobs_wms.is_empty(){
            for w in generic_jobs_wms.iter(){
                let mut context = Context::new();
                context.insert("status", &w.status);
                context.insert("name", &w.name);
                context.insert("size", &w.size);
                context.insert("last_update", &w.last_update);
                context.insert("upstream", &w.upstream);
                context.insert("is_master", &w.is_master);
                context.insert("last_ended", &w.last_ended);
                context.insert("last_started", &w.last_started);
                context.insert("scheduled", &w.scheduled);
                match tera.render_str(format, &context){
                    Ok(output) => {
                        println!("{}", output);
                    }
                    Err(e) => {
                        eprintln!("输出信息失败: {}", e);
                        exit(1);
                    }
                }
            }
        }else if !generic_jobs_ms.is_empty() {
            for m in generic_jobs_ms.iter(){
                let mut context = Context::new();
                context.insert("status", &m.status);
                context.insert("status", &m.name);
                context.insert("status", &m.worker);
                context.insert("status", &m.is_master);
                context.insert("status", &m.last_started);
                context.insert("status", &m.last_update);
                context.insert("status", &m.last_ended);
                context.insert("status", &m.scheduled);
                context.insert("status", &m.upstream);
                context.insert("status", &m.size);
                context.insert("status", &m.error_msg);
                match tera.render_str(format, &context){
                    Ok(output) => {
                        println!("{}", output);
                    }
                    Err(e) => {
                        eprintln!("输出信息失败: {}", e);
                        exit(1)
                    }
                }
            }
        }
    }else {
        // XXX: 😓
        if !generic_jobs_wms.is_empty(){
            let data = json!(generic_jobs_wms);
            let pretty_json = to_string_pretty(&data)?;
            println!("{}", pretty_json);
        }else if !generic_jobs_ms.is_empty() {
            let data = json!(generic_jobs_ms);
            let pretty_json = to_string_pretty(&data)?;
            println!("{}", pretty_json);
        }
    }
    Ok(())
}


async fn update_mirror_size(c: &ArgMatches) -> Result<()>{
    let worker_id = c.get_one::<String>("WORKER").unwrap();
    let mirror_id = c.get_one::<String>("MIRROR").unwrap();
    let mirror_size = c.get_one::<String>("SIZE").unwrap();

    #[derive(Serialize, Deserialize)]
    struct Msg<'a>{
        name: &'a str,
        size: usize,
    }
    let msg = Msg{
        name: mirror_id,
        size: mirror_size.parse::<usize>()?,
    };
    let url = format!("{}/workers/{}/jobs/{}/size",
                      *BASE_URL.read().await,
                      worker_id,
                      mirror_id);
    let client = CLIENT.read().await.clone();
    match rtsync::util::post_json(&url, &msg, Some(client)).await{
        Err(e) => {
            eprintln!("向manager发送请求失败: {}", e);
            exit(1);
        },
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("Manager 更新镜像大小失败: {:?}", resp);
                exit(1);
            }
            let status: rtsync::msg::MirrorStatus = resp.json().await.expect("无法解析成MirrorStatus");
            if status.size != *mirror_size {
                eprintln!("镜像大小错误, 应该为 {}, 但manager返回 {}", mirror_size, status.size);
                exit(1);
            }
            println!("成功将镜像的大小设置为 {}", mirror_size);
        }
    }

    Ok(())
}

async fn remove_worker(c: &ArgMatches) -> Result<()>{
    let worker_id = c.get_one::<String>("worker").unwrap();
    let url = format!("{}/workers/{}", *BASE_URL.read().await, worker_id);
    let client = CLIENT.read().await.clone();
    let resp = client.delete(&url).send().await?;
    if resp.status() != StatusCode::OK{
        eprintln!("发送命令失败，HTTP状态码不是200: {:?}", resp);
        exit(1);
    }
    let res: HashMap<String, String> = resp.json().await.expect("解析相应失败");
    if let Some(msg) = res.get("message"){
        if msg == "deleted"{
            println!("成功删除worker");
        }else {
            eprintln!("删除worker失败");
            exit(1);
        }
    }else {
        eprintln!("删除worker失败, 没有解析到message字段");
        exit(1);
    }
    Ok(())
}

async fn flush_disabled_jobs(_c: &ArgMatches) -> Result<()>{
    let url = format!("{}{}", *BASE_URL.read().await, FLUSH_DISABLED_PATH);
    let client = CLIENT.read().await.clone();
    let resp = client.delete(&url).send().await?;
    if resp.status() != StatusCode::OK{
        eprintln!("发送命令失败，HTTP状态码不是200: {:?}", resp);
        exit(1);
    }
    println!("成功刷新已禁用的任务");
    Ok(())
}

async fn cmd_job(cmd: rtsync::msg::CmdVerb, c: &ArgMatches, is_start: bool){
    let worker_id = c.get_one::<String>("WORKER").unwrap().clone();
    let mirror_id = c.get_one::<String>("MIRROR").unwrap().clone();

    let mut options: HashMap<String, bool> = HashMap::new();
    // force针对start
    // XXX: 由于clap不能在命令没有设置一个flag参数时尝试获得该flag值
    // 且force flag参数只在start时使用,所以为该方法签名添加is_start参数来确定子命令
    if is_start{
        if c.get_flag("force"){
            options.insert("force".to_string(), true);
        }
    }
    
    let client_cmd = rtsync::msg::ClientCmd{
        cmd,
        mirror_id,
        worker_id,
        options,
        ..Default::default()
    };
    let url = format!("{}{}", *BASE_URL.read().await, CMD_PATH);
    let client = CLIENT.read().await.clone();
    match rtsync::util::post_json(&url, &client_cmd, Some(client)).await{
        Err(e) => {
            eprintln!("发送命令失败: {}", e);
            exit(1);
        }
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("发送命令失败，HTTP状态码不是200: {:?}", resp);
                exit(1);
            }
            println!("成功发送命令");
        }
    }
}

async fn cmd_worker(cmd: rtsync::msg::CmdVerb, c: &ArgMatches){
    let worker_id = c.get_one::<String>("WORKER").unwrap().clone();
    let cmd = rtsync::msg::ClientCmd{
        cmd,
        worker_id,
        ..Default::default()
    };
    let url = format!("{}{}", *BASE_URL.read().await, CMD_PATH);
    let client = CLIENT.read().await.clone();
    match rtsync::util::post_json(&url, &cmd, Some(client)).await {
        Err(e) => {
            eprintln!("发送命令失败: {}", e);
            exit(1);
        },
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("发送命令失败，HTTP状态码不是200: {:?}", resp);
                exit(1);
            }
            println!("成功发送命令");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let matches = cli::build_cli()
        .get_matches();

    match matches.subcommand() {
        Some(("list", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                list_jobs(sub_matches).await?;
            }
        },
        Some(("flush", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                flush_disabled_jobs(sub_matches).await?;
            }
        },
        Some(("workers", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                list_workers(sub_matches).await?;
            }
        },
        Some(("rm-worker", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                remove_worker(sub_matches).await?;
            }
        },
        Some(("set-size", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                update_mirror_size(&sub_matches).await?;
            }
        },
        Some(("start", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Start, &sub_matches, true).await;
            }
        },
        Some(("stop", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Stop, &sub_matches, false).await;
            }
        },
        Some(("disable", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Disable, &sub_matches, false).await;
            }
        },
        Some(("restart", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Restart, &sub_matches, false).await;
            }
        },
        Some(("reload", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_worker(rtsync::msg::CmdVerb::Reload, &sub_matches).await;
            }
        },
        Some(("ping", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Ping, &sub_matches, false).await;
            }
        },
        _ => unreachable!()
    }

    Ok(())
}