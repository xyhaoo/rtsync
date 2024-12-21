use std::cell::RefCell;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{process, thread};
use chrono::{DateTime, Duration, Utc};
use crossbeam_channel::bounded;
use libc::free;
use log::{debug, error, info, log, warn};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use crate::cgroup::CGroupHook;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::{EmptyHook, HookType, JobHook, JobIntoBox};
use crate::provider::{_LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::{err_process_not_started, CmdJob};
use crate::zfs_hook::ZfsHook;
#[cfg(target_os = "linux")]
use crate::btrfs_snapshot_hook::BtrfsSnapshotHook;
use crate::config::{DockerConfig, MirrorConfig};
use scopeguard::defer;
use serde::de::Unexpected::Map;

// baseProvider是providers的基本混合
// Mutex<BaseProvider>
#[derive(Default)]
pub(crate) struct BaseProvider {
    pub(crate) ctx: Arc<Mutex<Option<Context>>>,
    pub(crate) name: String,
    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
    pub(crate) is_master: bool,
    
    pub(crate) cmd: Option<CmdJob>,
    pub(crate) log_file_fd: Option<File>,
    pub(crate) is_running: AtomicBool,

    pub(crate) cgroup: Arc<Option<CGroupHook>>,
    pub(crate) zfs: Arc<Option<ZfsHook>>,
    pub(crate) docker: Arc<Option<DockerHook>>,

    pub(crate) hooks: Arc<Mutex<Vec<Box<dyn JobHook>>>>
}


impl BaseProvider {
    pub(crate) fn new(name: String, interval: Duration, retry: i64, timeout: Duration) -> Self {
        BaseProvider{
            name,
            ctx: Arc::new(Mutex::new(Some(Context::new()))),
            interval,
            retry,
            timeout,
            ..BaseProvider::default()
        }
    }

    pub(crate) fn name(&self) -> String {
        self.name.clone()
    }

    pub(crate) fn enter_context(&self) -> Arc<Mutex<Option<Context>>>{
        let mut context = self.ctx.lock().unwrap();
        if let Some(ctx) = context.take() {
            *context = Some(ctx.enter());
        };
        drop(context);
        self.context()
    }

    pub(crate) fn exit_context(&self) -> Arc<Mutex<Option<Context>>> {
        let mut context = self.ctx.lock().unwrap();
        if let Some(ctx) = context.take() {
            *context = match ctx.exit() {
                Ok(ctx) => Some(ctx),
                Err(_) => None,
            }
        };
        drop(context);
        self.context()
    }
    
    pub(crate) fn context(&self) -> Arc<Mutex<Option<Context>>> {
        self.ctx.clone()
    }
    
    
    pub(crate) fn interval(&self) -> Duration {
        self.interval.clone()
    }
    
    fn retry(&self) -> i64 {
        self.retry
    }
    
    pub(crate) fn timeout(&self) -> Duration {
        self.timeout.clone()
    }
    
    fn is_master(&self) -> bool {
        self.is_master
    }
    
    pub(crate) fn add_hook(&mut self, hook: HookType) {
        match hook {
            HookType::Cgroup(ref cgroup) => {
                self.cgroup = Arc::from(Some(cgroup.clone()));
            },
            HookType::Zfs(ref zfs) => {
                self.zfs = Arc::from(Some(zfs.clone()));
            },
            HookType::Docker(ref docker) => {
                println!("debug: 添加docker hook {:?}", docker);
                self.docker = Arc::from(Some(docker.clone()));
            },
            _ => {}
        }
        self.hooks.lock().unwrap().push(HookType::into_box(hook));
    }
    
    pub(crate) fn hooks(&self) -> Arc<Mutex<Vec<Box<dyn JobHook>>>>{
        self.hooks.clone()
    }
    
    fn cgroup_ref(&self) -> Arc<Option<CGroupHook>> {
        self.cgroup.clone()
    }
    
    fn zfs_ref(&self) -> Arc<Option<ZfsHook>>{
        self.zfs.clone()
    }
    
    pub(crate) fn docker_ref(&self) -> Arc<Option<DockerHook>> {
        self.docker.clone()
    }

    fn close_log_file(&self) -> Result<(), Box<dyn Error>>{
        // File会在超出作用域后自动关闭
        Ok(())
    }

    fn run(&mut self) -> Result<(), Box<dyn Error>>{
        panic!("还未实现")
    }
    

    pub(crate) fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }


    // 初始化zfs字段
    fn zfs(&mut self, z_pool: String){
        self.zfs = Arc::from(Some(ZfsHook::new(z_pool)));
    }

    // 初始化docker字段
    fn docker(&mut self, g_cfg: DockerConfig, m_cfg: MirrorConfig){
        self.docker = Arc::from(Some(DockerHook::new(g_cfg, m_cfg)));
    }
    
    pub(crate) fn working_dir(&self) -> String {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            if let Some(v) = ctx.get(_WORKING_DIR_KEY){
                v.get::<String>().cloned().unwrap()
            }else {
                panic!("工作目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }

    pub(crate) fn log_dir(&self) -> String {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            if let Some(v) = ctx.get(_LOG_DIR_KEY){
                v.get::<String>().cloned().unwrap()
            }else {
                panic!("日志目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }

    pub(crate) fn log_file(&self) -> String {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            if let Some(v) = ctx.get(_LOG_FILE_KEY){
                v.get::<String>().cloned().unwrap()
            }else {
                panic!("日志文件不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }

    // 配置self.log_file_fd字段和运行rsync命令时的日志输出路径，如果指定为/dev/null，则忽略任何日志输出
    pub(crate) fn prepare_log_file(&mut self, append: bool) -> Result<(), Box<dyn Error>>{
        if self.log_file().eq("/dev/null"){
            self.cmd.as_ref().unwrap().set_log_file(None);
            return Ok(())
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true);

        if append {
            options.append(true); // 追加模式
        }

        // 打开文件，或创建新文件，设置权限为0644
        match options.open(self.log_file()) {
            Err(e) => {
                error!("无法打开log文件：{} : {}", self.log_file(), e);
                return Err(Box::new(e))
            }
            Ok(log_file) => {
                self.log_file_fd = Some(log_file.try_clone()?);
                self.cmd.as_ref().unwrap().set_log_file(Some(log_file));
            }
        }
        Ok(())
    }

    // DockerHook的log_file方法
    pub(crate) fn docker_log_file(&self) -> String{
        if let Some(ctx) = self.context().lock().unwrap().as_ref() {
            if let Some(value) = ctx.get(&format!("{_LOG_FILE_KEY}:docker")){
                return value.get::<String>().cloned().unwrap()
            }
        }
        self.log_file()
    }

    // DockerHook的volumes方法
    // volumes返回已配置的卷和运行时需要的卷
    // 包括mirror dirs和log file
    pub(crate) fn docker_volumes(&self) -> Vec<String>{
        if let Some(docker) = self.docker_ref().as_ref(){
            let mut vols = Vec::with_capacity(docker.volumes.len());
            vols.extend(docker.volumes.iter().cloned());

            if let Some(ctx)  = self.context().lock().unwrap().as_ref() {
                if let Some(ivs) = ctx.get("volumes"){
                    vols.extend(ivs.get::<Vec<String>>().cloned().unwrap());
                }
            }
            vols
        }else {
            Vec::new()
        }
    }


    /// 启动配置好的命令
    pub(crate) fn start(&mut self) -> Result<(), Box<dyn Error>>{
        let cmd_job = self.cmd.as_mut().unwrap();
        let mut cmd_lock = cmd_job.cmd.lock().unwrap();
        let x = format!("debug: 命令 {:?}\n参数：{:?}\n环境变量: {:?}",
                        cmd_lock.get_program(),
                        cmd_lock.get_args().collect::<Vec<&OsStr>>(),
                        cmd_lock.get_envs());
        println!("{x}");
        debug!("命令启动：{:?} {:?}", cmd_lock.get_program(), cmd_lock.get_args().collect::<Vec<&OsStr>>());

        cmd_job.finished = Some(bounded(1).1);
        let spawn_result = cmd_lock.spawn();
        // drop(cmd_lock);
        match spawn_result {
            Err(e) => {
                // 检测不到命令所在的路径
                return Err(Box::new(e))
            }
            Ok(child) => {
                let mut child_lock = cmd_job.result.lock().unwrap();
                // 输入了错误的参数，命令正确，不会马上检测到错误，也会进入这个分支
                cmd_job.pid = Some(child.id() as i32);
                *child_lock = Some(child);
                drop(child_lock);
            }
        }
        drop(cmd_lock);
        Ok(())
    }

    // 等待命令结束
    pub(crate) fn wait(&self) -> Result<i32, Box<dyn Error>>{
        debug!("调用了wait函数，调用者：{}", self.name());

        let ret = self.cmd.as_ref().unwrap()
            .wait();

        debug!("将is_running字段设置为false，调用者：{}", self.name());
        self.is_running.store(false, Ordering::SeqCst);
        println!("debug: 将is_running字段设置为false，调用者：{}", self.name());
        ret
    }

    pub(crate) fn terminate(&self) -> Result<(), Box<dyn Error>>{
        println!("debug: 进入terminate函数");
        debug!("正在终止provider：{}", self.name());
        if !self.is_running(){
            warn!("调用了终止函数，但是此时没有检测到 {} 正在运行", self.name());
            return Ok(())
        }
        /////////////////////////////////
        let has_docker = self.docker_ref().is_some();
        let cmd_job = self.cmd.as_ref().unwrap();
        let pid = cmd_job.pid;
        let cmd_lock = cmd_job.cmd.lock().unwrap();
        // cmd的terminate方法
        if cmd_lock.get_program().eq("") || pid.is_none() {
            return Err(err_process_not_started())
        }
        let pid = Pid::from_raw(pid.unwrap());
        if has_docker {
            Command::new("docker").arg("stop")
                .arg("-t").arg("2")
                .arg(self.name())
                .output()?;
            return Ok(());
        }

        // 发送 SIGTERM 信号
        if let Err(e) = kill(pid, Signal::SIGTERM){
            return Err(e.into())
        }
        // 等待 2 秒
        thread::sleep(core::time::Duration::from_secs(2));

        // 如果子进程仍在运行，则发送 SIGKILL 信号强制退出
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(status) if status.eq(&WaitStatus::StillAlive) => {
                kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
                warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
            },
            _ => {}
        }
        // if let None = child.try_wait().ok().flatten() {
        //     kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
        //     warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
        // }
        // if let Some(finished) = cmd.finished.read().unwrap().as_ref(){
        //     if finished.try_recv().is_err() {
        //         kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
        //         warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
        //     }
        // }else {
        //     panic!("无法获得finished字段")
        // }
        drop(cmd_lock);
        Ok(())
    }
}
