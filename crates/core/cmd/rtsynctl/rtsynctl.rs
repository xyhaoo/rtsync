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
const SYSTEM_CFG_FILE: &str = "/etc/rtsync/ctl.conf";   // system-wide conf
const  USER_CFG_FILE: &str = "$HOME/.config/rtsync/ctl.conf";   // user-specific conf

lazy_static! {
    static ref BASE_URL: RwLock<String> = RwLock::new(String::new());
    static ref CLIENT: RwLock<Client> = RwLock::new(Client::new());
}

fn initialize_wrapper(){

}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(default)]
struct Config {
    manager_addr: String,
    manager_port: u32,
    ca_cert: String,
}

fn load_config(cfg_file: &str, cfg: &mut Config) -> Result<()>{
    if !cfg_file.is_empty(){
        info!("Loading config: {}", cfg_file);
        let config_contents = fs::read_to_string(cfg_file)?;
        *cfg = toml::de::from_str(&config_contents)?;
    }
    Ok(())
}

async fn initialize(c: &ArgMatches) -> Result<()>{
    // init logger
    rtsync::logger::init_logger(c.get_flag("verbose"),
                                c.get_flag("debug"),
                                false);

    let mut cfg = Config::default();

    // default configs
    cfg.manager_addr = "localhost".to_string();
    cfg.manager_port = 14242;

    // find config file and load config
    load_config(SYSTEM_CFG_FILE, &mut cfg)?;
    let path = USER_CFG_FILE
        .replace("$HOME", &env::var("HOME").unwrap_or_default());
    debug!("user config file: {}", path);
    if let Some(config) = c.get_one::<String>("config"){
        load_config(config.as_str(), &mut cfg)?;
    }

    // override config using the command-line arguments
    if let Some(manager) = c.get_one::<String>("manager"){
        cfg.manager_addr = manager.clone();
    }
    if let Some(port) = c.get_one::<String>("manager_port"){
        cfg.manager_port = port.parse::<u32>()?;
    }

    if let Some(ca_cert) = c.get_one::<String>("ca_cert"){
        cfg.ca_cert = ca_cert.clone();
    }

    // parse base url of the manager server
    let mut url_lock = BASE_URL.write().await;
    if !cfg.ca_cert.is_empty(){
        *url_lock = format!("https://{}:{}", &cfg.manager_addr, &cfg.manager_port);
    }else {
        *url_lock = format!("http://{}:{}", &cfg.manager_addr, &cfg.manager_port);
    }
    info!("Use manager address: {}", *url_lock);
    drop(url_lock);

    // create HTTP client
    let mut client_lock = CLIENT.write().await;
    match rtsync::util::create_http_client(Some(&cfg.ca_cert)){
        Ok(client) => {
            *client_lock = client;
        }
        Err(e) => {
            error!("error initializing HTTP client: {e}");
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
            eprintln!("Filed to correctly get information from manager server: {}", e);
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
                eprintln!("Failed to correctly get information of all jobs from manager server: {}", e);
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
                        eprintln!("Error parsing status: {}", e);
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
                        error!("Failed to correctly get jobs for WORKER {}: {}",worker_id, e);
                    }
                }
                ans_tx.send(worker_jobs).await.unwrap();
            });
        }
        for _ in worker_ids.iter(){
            let job = ans_rx.recv().await.unwrap();
            if job.is_empty(){
                eprintln!("Failed to correctly get information of jobs from at least one manager");
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
                        eprintln!("Error printing out information: {}", e);
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
                        eprintln!("Error printing out information: {}", e);
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
            eprintln!("Failed to send request to manager: {}", e);
            exit(1);
        },
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("Manager failed to update mirror size: {:?}", resp);
                exit(1);
            }
            // FIXME: 因为manager返回的json和原版的tunasync不同，这里解析可能会出错
            let status: rtsync::msg::MirrorStatus = resp.json().await.expect("无法解析成MirrorStatus");
            if status.size != *mirror_size {
                eprintln!("Mirror size error, expecting {}, manager returned {}", mirror_size, status.size);
                exit(1);
            }
            println!("Successfully updated mirror size to {}", mirror_size);
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
        eprintln!("Failed to correctly send: command: HTTP status code is not 200: {:?}", resp);
        exit(1);
    }
    // FIXME: 因为manager返回的json和原版的tunasync不同，这里解析可能会出错
    let res: HashMap<String, String> = resp.json().await.expect("无法解析成HashMap");
    if let Some(msg) = res.get("message"){
        if msg == "deleted"{
            println!("Successfully removed the worker");
        }else {
            eprintln!("Failed to remove the worker");
            exit(1);
        }
    }else {
        eprintln!("Failed to remove the worker, 没有解析到message字段");
        exit(1);
    }
    Ok(())
}

async fn flush_disabled_jobs(_c: &ArgMatches) -> Result<()>{
    let url = format!("{}{}", *BASE_URL.read().await, FLUSH_DISABLED_PATH);
    let client = CLIENT.read().await.clone();
    let resp = client.delete(&url).send().await?;
    if resp.status() != StatusCode::OK{
        eprintln!("Failed to correctly send: command: HTTP status code is not 200: {:?}", resp);
        exit(1);
    }
    println!("Successfully flushed disabled jobs");
    Ok(())
}

async fn cmd_job(cmd: rtsync::msg::CmdVerb, c: &ArgMatches){
    let worker_id = c.get_one::<String>("WORKER").unwrap().clone();
    let mirror_id = c.get_one::<String>("MIRROR").unwrap().clone();

    let mut options: HashMap<String, bool> = HashMap::new();
    // force针对start
    if c.get_flag("force"){
        options.insert("force".to_string(), true);
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
            eprintln!("Failed to correctly send command: {}", e);
            exit(1);
        }
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("Failed to correctly send command: HTTP status code is not 200: {:?}", resp);
                exit(1);
            }
            println!("Successfully send the command");
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
            eprintln!("Failed to correctly send command: {}", e);
            exit(1);
        },
        Ok(resp) => {
            if resp.status() != StatusCode::OK{
                eprintln!("Failed to correctly send command: HTTP status code is not 200: {:?}", resp);
                exit(1);
            }
            println!("Successfully send the command");
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
                cmd_job(rtsync::msg::CmdVerb::Start, &sub_matches).await;
            }
        },
        Some(("stop", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Stop, &sub_matches).await;
            }
        },
        Some(("disable", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Disable, &sub_matches).await;
            }
        },
        Some(("restart", sub_matches)) => {
            if sub_matches.args_present() {
                if let Err(e) = initialize(sub_matches).await {
                    eprintln!("{}", e);
                    exit(1);
                }
                cmd_job(rtsync::msg::CmdVerb::Restart, &sub_matches).await;
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
                cmd_job(rtsync::msg::CmdVerb::Ping, &sub_matches).await;
            }
        },
        _ => unreachable!()
    }

    Ok(())
}