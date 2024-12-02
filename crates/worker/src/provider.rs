use std::error::Error;
use std::path::PathBuf;
use crate::config::{ProviderEnum, MirrorConfig, Config};
use crate::common;
use crate::cgroup::CGroupHook;
use crate::zfs_hook::ZfsHook;
use crate::hooks::JobHook;
use crate::context::Context;
use crate::docker::DockerHook;
use std::sync::mpsc;
use chrono::{DateTime, Utc};
use regex::Regex;
use log::warn;

// mirror provider是mirror job的包装器

const _WORKING_DIR_KEY: &str = "working_dir";
const _LOG_DIR_KEY: &str = "log_dir";
const _LOG_FILE_KEY: &str = "log_file";


// MirrorProvider trait
pub trait MirrorProvider: /*Clone+Sized*/{
    // name
    fn name(&self) -> String;
    fn upstream(&self) -> String;
    fn r#type(&self) -> ProviderEnum;

    // 开始后等待
    fn run(&self, started: mpsc::Sender<common::Empty>) -> Result<(), Box<dyn Error>>;
    // job开始
    fn start(&self) -> Result<(), Box<dyn Error>>;
    // 等待job结束
    fn wait(&self) -> Result<(), Box<dyn Error>>;
    // 终止mirror job
    fn terminate(&self) -> Result<(), Box<dyn Error>>;
    // job hooks
    fn is_running(&self) -> bool;
    // Cgroup
    fn c_group(&self) -> Option<CGroupHook>;
    // ZFS
    // fn zfs(&self) -> ZfsHook;
    // docker
    fn docker(&self) -> Option<DockerHook<Self>> where Self: Sized;

    fn add_hook(&self, hook: Box<dyn JobHook>);
    fn hooks(&self) -> Vec<Box<dyn JobHook>>;

    fn interval(&self)-> DateTime<Utc>;
    fn retry(&self) -> u64;
    fn timeout(&self) -> DateTime<Utc>;

    fn working_dir(&self) -> String;
    fn log_dir(&self) -> String;
    fn log_file(&self) -> String;
    fn is_master(&self) -> bool;
    fn data_size(&self) -> String;

    // // enter context
    // fn enter_context<T: Clone>(&self) -> Context<T>;
    // // exit context
    // fn exit_context<T: Clone>(&self) -> Context<T>;
    // // return context
    // fn context<T: Clone>(&self) -> Context<T>;
}


// fn new_mirror_provider<T>(mirror: &mut MirrorConfig, cfg: &Config) -> T 
// where T: MirrorProvider
// {
//     let format_log_dir = |log_dir: String, m: &MirrorConfig|-> String{
//         // 使用正则替换模板中的占位符
//         let re = Regex::new(r"\{\{\.Name}}").unwrap(); // 匹配 {{.Name}}
// 
//         // 使用正则替换 name 为 MirrorConfig 中的 name
//         let formatted_log_dir = re.replace_all(&*log_dir, m.name.clone().unwrap());
// 
//         // 返回替换后的字符串
//         formatted_log_dir.to_string()
//     };
//     let log_dir = mirror.log_dir.clone()
//         .unwrap_or_else(|| cfg.global.log_dir.clone()
//             .unwrap_or_else(|| "".to_string()));
//     let mirror_dir = mirror.mirror_dir.clone().unwrap_or_else(|| 
//         PathBuf::new()
//             .join(cfg.global.mirror_dir.clone().unwrap_or_else(|| "".to_string()))
//             .join(mirror.mirror_sub_dir.clone().unwrap_or_else(|| "".to_string()))
//             .join(mirror.name.clone().unwrap_or_else(|| "".to_string()))
//             .display().to_string());
// 
//     mirror.interval = match mirror.interval{
//         None => cfg.global.interval.clone(),
//         Some(0) => cfg.global.interval.clone(),
//         Some(other) => Some(other),
//     };
// 
//     mirror.retry = match mirror.retry{
//         None => cfg.global.retry.clone(),
//         Some(0) => cfg.global.retry.clone(),
//         Some(other) => Some(other),
//     };
// 
//     mirror.timeout = match mirror.timeout{
//         None => cfg.global.timeout.clone(),
//         Some(0) => cfg.global.timeout.clone(),
//         Some(other) => Some(other),
//     };
//     
//     let log_dir = format_log_dir(log_dir, mirror);
//     
//     //is master
//     let mut is_master = true;
//     match &mirror.role{
//         Some(slave) if slave.eq("slave") => is_master = false,
//         Some(other) if !other.eq("master") =>  {
//             warn!("{} 的role配置无效", mirror.name.clone().unwrap())
//         },
//         _ => {},
//     }
//     
//     match &mirror.provider{
//         Some(provider) if provider.eq(&ProviderEnum::Command) => {},
//         Some(provider) if provider.eq(&ProviderEnum::Rsync) => {},
//         Some(provider) if provider.eq(&ProviderEnum::TwoStageRsync) => {},
//         _ => {panic!("mirror的provider字段无效")}
//     }
//     
//     // add logging hook
//     
//     
//     
//     
//     
//     
//     unimplemented!()
// }