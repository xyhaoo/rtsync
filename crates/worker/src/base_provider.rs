use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::future::IntoFuture;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::Mutex;
use std::sync::{Arc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::mpsc::channel;
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

// baseProvider是providers的基本混合
// Mutex<BaseProvider>
#[derive(Default, Debug)]
pub(crate) struct BaseProvider {
    pub(crate) ctx: Arc<Mutex<Option<Context>>>,
    pub(crate) name: String,
    // 任务从同步完成到下一次同步开始的间隔时间
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

    pub(crate) async fn enter_context(&self) -> Arc<Mutex<Option<Context>>>{
        let mut context = self.ctx.lock().await;
        if let Some(ctx) = context.take() {
            *context = Some(ctx.enter());
        };
        drop(context);
        self.context()
    }

    pub(crate) async fn exit_context(&self) -> Arc<Mutex<Option<Context>>> {
        let mut context = self.ctx.lock().await;
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
    
    pub(crate) async fn add_hook(&mut self, hook: HookType) {
        match hook {
            HookType::Cgroup(ref cgroup) => {
                self.cgroup = Arc::from(Some(cgroup.clone()));
            },
            HookType::Zfs(ref zfs) => {
                self.zfs = Arc::from(Some(zfs.clone()));
            },
            HookType::Docker(ref docker) => {
                // println!("debug: 添加docker hook {:?}", docker);
                self.docker = Arc::from(Some(docker.clone()));
            },
            _ => {}
        }
        self.hooks.lock().await.push(HookType::into_box(hook));
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

    fn close_log_file(&self) -> Result<()>{
        // File会在超出作用域后自动关闭
        Ok(())
    }

    fn run(&mut self) -> Result<()>{
        panic!("还未实现")
    }
    

    pub(crate) fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Acquire)
    }


    // 初始化zfs字段
    fn zfs(&mut self, z_pool: String){
        self.zfs = Arc::from(Some(ZfsHook::new(z_pool)));
    }

    // 初始化docker字段
    fn docker(&mut self, g_cfg: DockerConfig, m_cfg: MirrorConfig){
        self.docker = Arc::from(Some(DockerHook::new(g_cfg, m_cfg)));
    }
    
    pub(crate) async fn working_dir(&self) -> String {
        if let Some(ctx) = self.ctx.lock().await.as_ref() {
            if let Some(v) = ctx.get(_WORKING_DIR_KEY){
                v.get::<String>().cloned().unwrap()
            }else {
                panic!("工作目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }

    pub(crate) async fn log_dir(&self) -> String {
        if let Some(ctx) = self.ctx.lock().await.as_ref() {
            if let Some(v) = ctx.get(_LOG_DIR_KEY){
                v.get::<String>().cloned().unwrap()
            }else {
                panic!("日志目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }

    pub(crate) async fn log_file(&self) -> String {
        if let Some(ctx) = self.ctx.lock().await.as_ref() {
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
    pub(crate) async fn prepare_log_file(&mut self, append: bool) -> Result<()>{
        let log_file = self.log_file().await;
        if log_file.eq("/dev/null"){
            self.cmd.as_ref().unwrap().set_log_file(None).await;
            return Ok(())
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true);

        if append {
            options.append(true); // 追加模式
        }

        // 打开文件，或创建新文件，设置权限为0644
        match options.open(&log_file) {
            Err(e) => {
                error!("无法打开log文件：{} : {}", log_file, e);
                return Err(anyhow!(Box::new(e)))
            }
            Ok(log_file) => {
                self.log_file_fd = Some(log_file.try_clone()?);
                self.cmd.as_ref().unwrap().set_log_file(Some(log_file)).await;
            }
        }
        Ok(())
    }

    // DockerHook的log_file方法
    pub(crate) async fn docker_log_file(&self) -> String{
        if let Some(ctx) = self.context().lock().await.as_ref() {
            if let Some(value) = ctx.get(&format!("{_LOG_FILE_KEY}:docker")){
                return value.get::<String>().cloned().unwrap()
            }
        }
        self.log_file().await
    }

    // DockerHook的volumes方法
    // volumes返回已配置的卷和运行时需要的卷
    // 包括mirror dirs和log file
    pub(crate) async fn docker_volumes(&self) -> Vec<String>{
        if let Some(docker) = self.docker_ref().as_ref(){
            let mut vols = Vec::with_capacity(docker.volumes.len());
            vols.extend(docker.volumes.iter().cloned());

            if let Some(ctx)  = self.context().lock().await.as_ref() {
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
    pub(crate) async fn start(&mut self) -> Result<()>{
        let cmd_job = self.cmd.as_mut().unwrap();
        let mut cmd_lock = cmd_job.cmd.lock().await;
        debug!("命令启动：{:?}", *cmd_lock);
        let (mut tx_lock, mut rx_lock) = 
            (cmd_job.finished_tx.lock().await, cmd_job.finished_rx.lock().await);
        let (tx, rx) = channel(1);
        *tx_lock = Some(tx);
        *rx_lock = Some(rx);
        drop((tx_lock, rx_lock));
        
        
        let spawn_result = cmd_lock.spawn();
        // drop(cmd_lock);
        match spawn_result {
            Err(e) => {
                // 检测不到命令所在的路径
                return Err(anyhow!(Box::new(e)))
            }
            Ok(child) => {
                let mut child_lock = cmd_job.result.lock().await;
                // 输入了错误的参数，命令正确，不会马上检测到错误，也会进入这个分支
                cmd_job.pid = Some(child.id().unwrap() as i32);
                *child_lock = Some(child);
                drop(child_lock);
            }
        }
        drop(cmd_lock);
        Ok(())
    }

    // 等待命令结束
    pub(crate) async fn wait(&self) -> Result<i32>{
        debug!("调用了wait函数，调用者：{}", self.name());
        
        let ret = self.cmd.as_ref().unwrap()
            .wait().await;
        
        debug!("将is_running字段设置为false，调用者：{}", self.name());
        self.is_running.store(false, Ordering::Release);
        // println!("debug: 将is_running字段设置为false，调用者：{}", self.name());
        ret
    }

    pub(crate) async fn terminate(&self) -> Result<()>{
        // println!("debug: 进入terminate函数");
        debug!("正在终止provider：{}", self.name());
        if !self.is_running(){
            warn!("调用了终止函数，但是此时没有检测到 {} 正在运行", self.name());
            return Ok(())
        }
        /////////////////////////////////
        let cmd_job = self.cmd.as_ref().unwrap();
        let pid = cmd_job.pid;
        let cmd_lock = cmd_job.cmd.lock().await;
        let mut finished_rx_lock = cmd_job.finished_rx.lock().await;
        // cmd的terminate方法
        if pid.is_none() {
            return Err(err_process_not_started())
        }
        
        if let Some(docker) = self.docker.as_ref() {
            // println!("debug: 终止docker");
            let name = docker.name(self.name());
            let _ = Command::new("docker").arg("stop")
                .arg("-t").arg("2")
                .arg(name)
                .output().await?;
            // println!("debug: output是 {output:?}");
            return Ok(());
        }
        
        // println!("debug: 发送 SIGTERM 信号");
        let pid = Pid::from_raw(pid.unwrap());
        // 发送 SIGTERM 信号
        if let Err(e) = kill(pid, Signal::SIGTERM){
            return Err(e.into())
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {        
                // 如果子进程仍在运行，则发送 SIGKILL 信号强制退出
                match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(status) if status.eq(&WaitStatus::StillAlive) => {
                        kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
                        warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
                    },
                    _ => {}
                }
            }
            _ = finished_rx_lock.as_mut().unwrap().recv() => {}
        }
        drop(finished_rx_lock);
        drop(cmd_lock);
        Ok(())
    }
}
