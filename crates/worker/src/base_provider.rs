use std::error::Error;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use chrono::{DateTime, Duration, Utc};
use crossbeam_channel::bounded;
use log::{debug, error, info, log, warn};
use nix::sys::signal::{kill, Signal};
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
// mutex
#[derive(Default)]
pub(crate) struct BaseProvider {
    pub(crate) ctx: Arc<Mutex<Option<Context>>> ,
    pub(crate) name: String,
    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
    pub(crate) is_master: bool,

    pub(crate) cmd: Option<CmdJob>,
    pub(crate) log_file_fd: Option<File>,
    pub(crate) is_running: Option<Arc<AtomicBool>>,

    pub(crate) cgroup: Option<CGroupHook>,
    pub(crate) zfs: Option<ZfsHook>,
    pub(crate) docker: Option<DockerHook>,

    pub(crate) hooks: Vec<Box<dyn JobHook>>
}


impl BaseProvider {
    pub(crate) fn name(&self) -> String {
        self.name.clone()
    }

    pub(crate) fn enter_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        let mut cur_ctx = self.ctx.lock().unwrap();
        *cur_ctx = match cur_ctx.take() {
            Some(ctx) => Some(ctx.enter()),
            None => None,
        };
        self.ctx.clone()
    }

    pub(crate) fn exit_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        let mut cur_ctx = self.ctx.lock().unwrap();
        *cur_ctx = match cur_ctx.take(){
            Some(ctx) => {
                match ctx.exit() {
                    Ok(ctx) => Some(ctx),
                    Err(_) => None,
                }
            },
            None => None,
        };
        self.ctx.clone()
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
                self.cgroup = Some(cgroup.clone());
            },
            HookType::Zfs(ref zfs) => {
                self.zfs = Some(zfs.clone());
            },
            HookType::Docker(ref docker) => {
                self.docker = Some(docker.clone());
            },
            _ => {}
        }
        self.hooks.push(HookType::into_box(hook));
    }
    
    pub(crate) fn hooks(&self) -> &Vec<Box<dyn JobHook>>{
        self.hooks.as_ref()
    }
    
    fn cgroup_ref(&self) -> &Option<CGroupHook> {
        &self.cgroup
    }
    
    fn zfs_ref(&self) -> &Option<ZfsHook>{
        &self.zfs
    }
    
    pub(crate) fn docker_ref(&self) -> &Option<DockerHook>{
        &self.docker
    }

    fn close_log_file(&mut self) -> Result<(), Box<dyn Error>>{
        // File会在超出作用域后自动关闭
        Ok(())
    }

    fn run(&mut self) -> Result<(), Box<dyn Error>>{
        panic!("还未实现")
    }
    
    /// 启动配置好的命令
    pub(crate) fn start(&self) -> Result<(), Box<dyn Error>>{
        let cmd_job = self.cmd.as_ref().expect("没有cmd字段");
        let mut cmd_lock =  cmd_job.cmd.write().unwrap();

        let x = format!("命令 {:?} 启动：{:?} \n 环境变量: {:?}", cmd_lock.get_program(), cmd_lock.get_args().collect::<Vec<&OsStr>>(), cmd_lock.get_envs());
        println!("{x}");
        debug!("命令启动：{:?}", cmd_lock.get_args().collect::<Vec<&OsStr>>());

        let mut finished_lock = cmd_job.finished.write().unwrap();
        *finished_lock = Some(bounded(1).1);
        drop(finished_lock);

        let spawn_result = cmd_lock.spawn();
        drop(cmd_lock);
        match spawn_result {
            Err(e) => {
                // 检测不到命令所在的路径
                return Err(Box::new(e))
            }
            Ok(child) => {
                // 输入了错误的参数，命令正确，不会马上检测到错误，也会进入这个分支
                let mut child_lock = cmd_job.result.write().unwrap();
                *child_lock = Some(child);
                drop(child_lock);
            }
        }
        Ok(())
    }

    pub(crate) fn is_running(&self) -> bool {
        match self.is_running.as_ref() {
            Some(running) => running.load(Ordering::SeqCst),
            None => panic!("没有is_running字段")
        }
    }
    
    // 等待命令结束
    pub(crate) fn wait(&mut self) -> Result<i32, Box<dyn Error>>{
        debug!("调用了wait函数，调用者：{}", self.name());
        let ret = self.cmd.as_mut().expect("没有cmd字段")
            .wait();
        
        debug!("将is_running字段设置为false，调用者：{}", self.name());
        if let Some(is_running) = &self.is_running {
            is_running.store(false, Ordering::SeqCst);
        }
        println!("debug: 将is_running字段设置为false，调用者：{}", self.name());
        
        ret
    }
    
    fn data_size(&self) -> String{
        "".to_string()
    }

    // 初始化zfs字段
    fn zfs(&mut self, z_pool: String){
        self.zfs = Some(ZfsHook::new(z_pool));
    }

    // 初始化docker字段
    fn docker(&mut self, g_cfg: DockerConfig, m_cfg: MirrorConfig){
        self.docker = Some(DockerHook::new(g_cfg, m_cfg));
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
            self.cmd.as_mut().expect("没有cmd字段")
                .set_log_file(None);
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
                self.cmd.as_mut().expect("没有cmd字段")
                    .set_log_file(Some(log_file));
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
        if let Some(docker) = self.docker_ref(){
            let mut vols = Vec::with_capacity(docker.volumes.len());
            vols.extend(docker.volumes.iter().cloned());

            let context = self.context();
            let ctx = context.lock().unwrap();
            if let Some(ctx)  = ctx.as_ref(){
                if let Some(ivs) = ctx.get("volumes"){
                    vols.extend(ivs.get::<Vec<String>>().cloned().unwrap());
                }
            }
            vols

        }else {
            Vec::new()
        }
    }
    
    pub(crate) fn terminate(&self) -> Result<(), Box<dyn Error>>{
        println!("debug: 进入terminate函数");
        let cmd = self.cmd.as_ref().expect("没有cmd字段");

        debug!("正在终止provider：{}", self.name());
        if !self.is_running(){
            warn!("调用了终止函数，但是此时没有检测到 {} 正在运行", self.name());
            return Ok(())
        }

        /////////////////////////////////
        // cmd的terminate方法
        if cmd.cmd.read().unwrap().get_program().eq("") || cmd.result.read().unwrap().is_none(){
            return Err(err_process_not_started())
        }
        if self.docker_ref().is_some(){
            Command::new("docker")
                .arg("stop")
                .arg("-t")
                .arg("2")
                .arg(self.name())
                .output()?;
            return Ok(())
        }
        if let Some(child) = cmd.result.write().unwrap().as_mut(){
            // 发送 SIGTERM 信号
            let pid = Pid::from_raw(child.id() as i32); // 获取子进程的 PID
            if let Err(e) = kill(pid, Signal::SIGTERM){
                return Err(e.into())
            }
            // 等待 2 秒
            thread::sleep(core::time::Duration::from_secs(2));

            // 如果子进程仍在运行，则发送 SIGKILL 信号强制退出
            if let None = child.try_wait().ok().flatten() {
                kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
                warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
            }
            // if let Some(finished) = cmd.finished.read().unwrap().as_ref(){
            //     if finished.try_recv().is_err() {
            //         kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
            //         warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
            //     }
            // }else {
            //     panic!("无法获得finished字段")
            // }
        }

        Ok(())
    }
}
