use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use crate::config::{ProviderEnum, MirrorConfig, Config};
use crate::common;
use crate::cgroup::CGroupHook;
use crate::hooks::{HookType, JobHook};
use crate::context::Context;
// use std::sync::mpsc;
use crossbeam_channel::{Sender, Receiver};
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use log::{error, warn};
use tera::Tera;
use crate::cmd_provider::{CmdConfig, CmdProvider};
use crate::docker::DockerHook;
use crate::rsync_provider::{RsyncConfig, RsyncProvider};
use crate::two_stage_rsync_provider::{TwoStageRsyncConfig, TwoStageRsyncProvider};
use crate::zfs_hook::ZfsHook;

#[cfg(not(target_os = "linux"))]
use crate::btrfs_snapshot_hook_nolinux;
#[cfg(target_os = "linux")]
use crate::btrfs_snapshot_hook;
use crate::exec_post_hook::{ExecOn, ExecPostHook};
use crate::loglimit_hook::LogLimiter;

// mirror provider是mirror job的包装器

pub(crate) const _WORKING_DIR_KEY: &str = "working_dir";
pub(crate) const _LOG_DIR_KEY: &str = "log_dir";
pub(crate) const _LOG_FILE_KEY: &str = "log_file";


// MirrorProvider trait
pub trait MirrorProvider: /*Clone+Sized*/{
    // name
    fn name(&self) -> String;
    fn upstream(&self) -> String;
    fn r#type(&self) -> ProviderEnum;

    // 开始后等待
    fn run(&mut self, started: Sender<common::Empty>) -> Result<(), Box<dyn Error>>;
    // job开始
    fn start(&mut self) -> Result<(), Box<dyn Error>> {Ok(())}
    // 等待job结束
    fn wait(&self) -> Result<(), Box<dyn Error>> {Ok(())}
    // 终止mirror job
    fn terminate(&self) -> Result<(), Box<dyn Error>>;
    // job hooks
    fn is_running(&self) -> bool;
    // Cgroup
    fn c_group(&self) -> Option<&CGroupHook> {None}
    // ZFS
    fn zfs(&self) -> Option<&ZfsHook> {None}
    // docker
    fn docker(&self) -> Option<&DockerHook> {None}

    fn add_hook(&mut self, hook: HookType);
    fn hooks(&self) -> &Vec<Box<dyn JobHook>>;

    fn interval(&self)-> Duration;
    fn retry(&self) -> u64 {0}
    fn timeout(&self) -> Duration;

    fn working_dir(&self) -> String;
    fn log_dir(&self) -> String;
    fn log_file(&self) -> String;
    fn is_master(&self) -> bool {false}
    fn data_size(&self) -> String;

    // enter context
    fn enter_context(&mut self) -> Arc<Mutex<Option<Context>>>;
    // exit context
    fn exit_context(&mut self) -> Arc<Mutex<Option<Context>>>;
    // return context
    fn context(&self) -> Arc<Mutex<Option<Context>>>;
}

// new_mirror_provider使用一个MirrorConfig和全局的Config创建一个MirrorProvider实例
pub(crate) fn new_mirror_provider(mut mirror: MirrorConfig, cfg: Config) -> Box<dyn MirrorProvider>
{
    // 使用MirrorConfig中的name字段匹配log_dir中的占位符
    let format_log_dir = |log_dir: String, m: &MirrorConfig|-> String{
        let mut tera = Tera::default(); // 创建一个空的模板引擎
        let mut context = tera::Context::new();
        context.insert("name", &m.name); // 将结构体中的值插入模板上下文

        let formatted = tera
            .render_str(&*log_dir, &context) // 渲染模板
            .unwrap_or_else(|_| panic!("渲染模板失败"));

        formatted
    };

    let log_dir = mirror.log_dir.clone()
        .unwrap_or( cfg.global.log_dir.clone()
            .unwrap_or_default());
    let mirror_dir = mirror.mirror_dir.clone().unwrap_or_else(||
        PathBuf::new()
            .join(cfg.global.mirror_dir.clone().unwrap_or_default())
            .join(mirror.mirror_sub_dir.clone().unwrap_or_default())
            .join(mirror.name.clone().unwrap_or_default())
            .display().to_string());
    
    if let Some(0) = mirror.interval{
        mirror.interval = cfg.global.interval.clone()
    }

    if let Some(0) = mirror.retry{
        mirror.retry = cfg.global.retry.clone()
    }

    if let Some(0) = mirror.timeout {
        mirror.timeout = cfg.global.timeout.clone()
    }
    
    let log_dir = format_log_dir(log_dir, &mirror);

    //is master
    let mut is_master = true;
    match &mirror.role{
        Some(slave) if slave.eq("slave") => is_master = false,
        Some(other) if !other.eq("master") =>  {
            warn!("{} 的role配置无效", mirror.name.clone().unwrap())
        },
        _ => {},
    }

    let mut provider: Box<dyn MirrorProvider>;
    
    match &mirror.provider{
        Some(_provider) if _provider.eq(&ProviderEnum::Command) => {
            let pc = CmdConfig{
                name: mirror.name.clone().unwrap_or_default(),
                upstream_url: mirror.upstream.clone().unwrap_or_default(),
                command: mirror.command.clone().unwrap_or_default(),
                working_dir: mirror_dir,
                fail_on_match: mirror.fail_on_match.clone().unwrap_or_default(),
                size_pattern: mirror.size_pattern.clone().unwrap_or_default(),
                log_dir: log_dir.clone(),
                log_file: PathBuf::new().join(log_dir).join("latest.log").display().to_string(),
                interval: Duration::minutes(mirror.interval.unwrap_or_default()),
                retry: mirror.retry.unwrap_or_default(),
                timeout: Duration::seconds(mirror.timeout.unwrap_or_default()),
                env: mirror.env.clone().unwrap_or_default(),
            };
            match CmdProvider::new(pc) {
                Ok(mut p) => {
                    p.base_provider.is_master = is_master;
                    provider = Box::new(p);
                },
                Err(e) => {
                    panic!("{}", e);
                }
            }
        },
        Some(_provider) if _provider.eq(&ProviderEnum::Rsync) => {
            let rc = RsyncConfig{
                name: mirror.name.clone().unwrap_or_default(),
                upstream_url: mirror.upstream.clone().unwrap_or_default(),
                rsync_cmd: mirror.command.clone().unwrap_or_default(),
                username: mirror.username.clone().unwrap_or_default(),
                password: mirror.password.clone().unwrap_or_default(),
                exclude_file: mirror.exclude_file.clone().unwrap_or_default(),
                extra_options: mirror.rsync_options.clone().unwrap_or_default(),
                rsync_never_timeout: mirror.rsync_no_timeo.unwrap_or_default(),
                rsync_timeout_value: mirror.rsync_timeout.clone().unwrap_or_default(),
                overridden_options: mirror.rsync_override.clone().unwrap_or_default(),
                rsync_env: mirror.env.clone().unwrap_or_default(),
                working_dir: mirror_dir,
                log_dir: log_dir.clone(),
                log_file: PathBuf::new().join(log_dir).join("latest.log").display().to_string(),
                use_ipv4: mirror.use_ipv4.unwrap_or_default(),
                use_ipv6: mirror.use_ipv6.unwrap_or_default(),
                interval: Duration::minutes(mirror.interval.unwrap_or_default()),
                retry: mirror.retry.unwrap_or_default(),
                timeout: Duration::seconds(mirror.timeout.unwrap_or_default()),
            };
            match RsyncProvider::new(rc) {
                Ok(mut p) => {
                    p.base_provider.is_master = is_master;
                    provider = Box::new(p);
                },
                Err(e) => {
                    panic!("{}", e);
                }
            }
        },
        Some(_provider) if _provider.eq(&ProviderEnum::TwoStageRsync) => {
            let rc = TwoStageRsyncConfig{
                name: mirror.name.clone().unwrap_or_default(),
                stage1_profile: mirror.stage1_profile.clone().unwrap_or_default(),
                upstream_url: mirror.upstream.clone().unwrap_or_default(),
                rsync_cmd: mirror.command.clone().unwrap_or_default(),
                username: mirror.username.clone().unwrap_or_default(),
                password: mirror.password.clone().unwrap_or_default(),
                exclude_file: mirror.exclude_file.clone().unwrap_or_default(),
                extra_options: mirror.rsync_options.clone().unwrap_or_default(),
                rsync_never_timeout: mirror.rsync_no_timeo.unwrap_or_default(),
                rsync_timeout_value: mirror.rsync_timeout.unwrap_or_default(),
                rsync_env: mirror.env.clone().unwrap_or_default(),
                working_dir: mirror_dir,
                log_dir: log_dir.clone(),
                log_file: PathBuf::new().join(log_dir).join("latest.log").display().to_string(),
                use_ipv4: mirror.use_ipv4.unwrap_or_default(),
                use_ipv6: mirror.use_ipv6.unwrap_or_default(),
                interval: Duration::minutes(mirror.interval.unwrap_or_default()),
                retry: mirror.retry.unwrap_or_default(),
                timeout: Duration::seconds(mirror.timeout.unwrap_or_default()),
            };
            match TwoStageRsyncProvider::new(rc) {
                Ok(mut p) => {
                    p.base_provider.is_master = is_master;
                    provider = Box::new(p);
                },
                Err(e) => {
                    panic!("{}", e);
                }
            }

        },
        _ => {panic!("mirror的provider字段无效")}
    }

    // add logging hook
    provider.add_hook(HookType::LogLimiter(LogLimiter::new()));

    // add zfs hook
    if let Some(true) = cfg.zfs.enable{
        if let Some(z_pool) = cfg.zfs.z_pool{
            provider.add_hook(HookType::Zfs(ZfsHook::new(z_pool)));
        }
    }

    // add btrfs snapshot hook
    if let Some(true) = cfg.btrfs_snapshot.enable{
        if let Some(snapshot_path) = cfg.btrfs_snapshot.snapshot_path{
            #[cfg(not(target_os = "linux"))]
            provider.add_hook(HookType::BtrfsNoLinux(btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook::new(&*snapshot_path, mirror.clone())));
            #[cfg(target_os = "linux")]
            provider.add_hook(HookType::Btrfs(btrfs_snapshot_hook::BtrfsSnapshotHook::new(&*provider.name(), &*snapshot_path, mirror.clone())));
        }
    }

    // add docker hook
    if let Some(true) = cfg.docker.enable{
        if mirror.docker_image.is_some(){
            provider.add_hook(HookType::Docker(DockerHook::new(cfg.docker.clone(), mirror.clone())));
        }
    }
    // else if cfg.c_group.enable.unwrap() {
    //     // add cgroup hook
    //     provider.add_hook(HookType::Cgroup(CGroupHook::new()))
    // }

    let mut add_hook_from_cmd_list = |cmd_list: Vec<String>, exec_on: u8| {
        for cmd in cmd_list {
            match ExecPostHook::new(ExecOn::from_u8(exec_on), &*cmd) {
                Ok(hook) => provider.add_hook(HookType::ExecPost(hook)),
                Err(e) => {
                    let err = format!("初始化mirror {} 失败：{}", mirror.name.clone().unwrap(), e);
                    error!("{}", err);
                    panic!("{}", err);
                }
            }
        }
    };

    // ExecOnSuccess hook
    match mirror.exec_on_success.as_ref() {
        Some(exec_on_success) if !exec_on_success.is_empty() => {
            add_hook_from_cmd_list(exec_on_success.clone(), ExecOn::Success.as_u8());
        }
        _ => {
            add_hook_from_cmd_list(cfg.global.exec_on_success.clone().unwrap_or_default(), 
                                   ExecOn::Success.as_u8());
        }
    }
    add_hook_from_cmd_list(mirror.exec_on_success_extra.clone().unwrap_or_default(),
                           ExecOn::Success.as_u8());

    // ExecOnFailure hook
    match mirror.exec_on_failure.as_ref() {
        Some(exec_on_failure) if !exec_on_failure.is_empty() => {
            add_hook_from_cmd_list(exec_on_failure.clone(), ExecOn::Failure.as_u8());
        }
        _ => {
            add_hook_from_cmd_list(cfg.global.exec_on_failure.clone().unwrap_or_default(),
                                   ExecOn::Failure.as_u8());
        }
    }
    add_hook_from_cmd_list(mirror.exec_on_failure_extra.clone().unwrap_or_default(),
                           ExecOn::Failure.as_u8());

    
    provider
}