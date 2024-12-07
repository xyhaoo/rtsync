use std::error::Error;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::{DateTime, Duration, Utc};
use log::{debug, error, log, warn};
use crate::cgroup::CGroupHook;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::JobHook;
use crate::provider::{_LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::CmdJob;
use crate::zfs_hook::ZfsHook;


enum HookType<T: Clone>{
    Cgroup(CGroupHook<T>),
    Zfs(ZfsHook<T>),
    Docker(DockerHook<T>),
}

// baseProvider是providers的基本混合
// mutex
#[derive(Default)]
pub(crate) struct BaseProvider<T: Clone> {
    pub(crate) ctx: Option<Context<T>> ,
    pub(crate) name: String,
    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
    pub(crate) is_master: bool,

    pub(crate) cmd: Option<Arc<RwLock<CmdJob<T>>>>,
    pub(crate) log_file_fd: Option<File>,
    pub(crate) is_running: Option<Arc<AtomicBool>>,

    pub(crate) cgroup: Option<CGroupHook<T>>,
    pub(crate) zfs: Option<ZfsHook<T>>,
    pub(crate) docker: Option<DockerHook<T>>,

    pub(crate) hooks: Vec<Box<dyn JobHook>>
}
impl<T: Clone> BaseProvider<T> {
    fn name(&self) -> &str {
        &self.name
    }
    fn enter_context(&mut self) -> Option<&Context<T>> {
        if let Some(ctx) = self.ctx.as_mut(){
            self.ctx = Some(ctx.enter());
            return self.ctx.as_ref();
        }
        None
    }
    fn exit_context(&mut self) -> Option<&Context<T>> {
        if let Some(cur_ctx) = self.ctx.as_mut(){
            if let Ok(ctx) = cur_ctx.to_owned().exit(){
                self.ctx = Some(ctx);
                return self.ctx.as_ref();
            }else {
                self.ctx = None;
            }
        }
        None
    }
    fn context(&self) -> Option<&Context<T>> {
        self.ctx.as_ref()
    }
    fn interval(&self) -> Duration {
        self.interval.clone()
    }
    fn retry(&self) -> i64 {
        self.retry
    }
    fn timeout(&self) -> Duration {
        self.timeout.clone()
    }
    fn is_master(&self) -> bool {
        self.is_master
    }
    fn add_hook(&mut self, hook: HookType<T>) {
        match hook {
            HookType::Cgroup(cgroup) => {self.cgroup = Some(cgroup);},
            HookType::Zfs(zfs) => {self.zfs = Some(zfs);},
            HookType::Docker(docker) => {self.docker = Some(docker);},
        }
    }
    fn hooks(&self) -> &Vec<Box<dyn JobHook>>{
        self.hooks.as_ref()
    }
    fn cgroup(&self) -> &Option<CGroupHook<T>> {
        &self.cgroup
    }
    fn zfs(&self) -> &Option<ZfsHook<T>>{
        &self.zfs
    }
    fn docker(&self) -> &Option<DockerHook<T>>{
        &self.docker
    }

    fn close_log_file(&mut self) -> Result<(), Box<dyn Error>>{
        // File会在超出作用域后自动关闭
        Ok(())
    }

    fn run(&mut self) -> Result<(), Box<dyn Error>>{
        panic!("还未实现")
    }
    fn start(&mut self) -> Result<(), Box<dyn Error>>{
        panic!("还未实现")
    }

    fn is_running(&self) -> bool {
        self.is_running.as_ref().expect("没有is_running字段")
            .load(Ordering::SeqCst)
    }
    pub(crate) fn wait(&mut self) -> Result<i32, Box<dyn Error>>{
        debug!("调用了wait函数，调用者：{}", self.name());
        debug!("将is_running字段设置为false，调用者：{}", self.name());
        if let Some(is_running) = &self.is_running {
            is_running.store(false, Ordering::SeqCst);
        }
        self.cmd.as_ref().expect("没有cmd字段")
            .write().expect("无法在BaseProvider的cmd字段上获得锁")
            .wait()
        
    }
    fn terminate(&mut self) -> Result<(), Box<dyn Error>>{
        if let Ok(mut cmd) = self.cmd.as_ref().expect("没有cmd字段").write(){
            debug!("正在终止provider：{}", self.name());
            if !self.is_running(){
                warn!("调用了终止函数，但是此时没有检测到 {} 正在运行", self.name());
                return Ok(())
            }
            cmd.terminate()
        }else { 
            panic!("在调用terminate时，无法在BaseProvider的cmd字段上获得锁")
        }
    }
    fn data_size(&self) -> String{
        "".to_string()
    }

}


impl BaseProvider<String> {
    fn working_dir(&self) -> String {
        if let Some(ctx) = self.ctx.as_ref() {
            if let Some(v) = ctx.get(_WORKING_DIR_KEY){
                v
            }else {
                panic!("工作目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }
    fn log_dir(&self) -> String {
        if let Some(ctx) = self.ctx.as_ref() {
            if let Some(v) = ctx.get(_LOG_DIR_KEY){
                v
            }else {
                panic!("日志目录不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }
    fn log_file(&self) -> String {
        if let Some(ctx) = self.ctx.as_ref() {
            if let Some(v) = ctx.get(_LOG_FILE_KEY){
                v
            }else {
                panic!("日志文件不应该不存在")
            }
        }else {
            panic!("没有ctx字段")
        }
    }
    fn prepare_log_file(&mut self, append: bool) -> Result<(), Box<dyn Error>>{
        if let Ok(mut cmd) = self.cmd.as_ref().expect("没有cmd字段").write() {
            if self.log_file().eq("/dev/null"){
                cmd.set_log_file(None);
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
                    cmd.set_log_file(Some(log_file));
                }
            }
        }else {
            panic!("在调用prepare_log_file时，无法在BaseProvider的cmd字段上获得锁")
        }

        Ok(())
    }

}