use std::error::Error;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Duration, Utc};
use enum_dispatch::enum_dispatch;
use log::{debug, error, info, log, warn};
use users::get_current_uid;
use crate::cgroup::CGroupHook;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::{EmptyHook, HookType, JobHook, JobIntoBox};
use crate::provider::{_LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::CmdJob;
use crate::zfs_hook::ZfsHook;
#[cfg(target_os = "linux")]
use crate::btrfs_snapshot_hook::BtrfsSnapshotHook;
use crate::btrfs_snapshot_hook_nolinux;
use crate::btrfs_snapshot_hook_nolinux::BtrfsSnapshotHook;
use crate::config::{DockerConfig, MirrorConfig};
use crate::exec_post_hook::ExecPostHook;
use crate::loglimit_hook::LogLimiter;


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

    pub(crate) cmd: Option<Arc<RwLock<CmdJob>>>,
    pub(crate) log_file_fd: Option<File>,
    pub(crate) is_running: Option<Arc<AtomicBool>>,

    pub(crate) cgroup: Option<CGroupHook>,
    pub(crate) zfs: Option<ZfsHook>,
    pub(crate) docker: Option<DockerHook>,

    pub(crate) hooks: Vec<Box<dyn JobHook>>
}
impl BaseProvider {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    
    // fn enter_context(&mut self) -> &Mutex<Option<Context<T>>> {
    //     let mut cur_ctx = self.ctx.lock().unwrap();
    //     *cur_ctx = match cur_ctx.to_owned() {
    //         Some(ctx) => Some(ctx.enter()),
    //         None => None,
    //     };
    //     
    //     self.ctx.as_ref()
    // }
    
    // fn exit_context(&mut self) -> Option<&Context<T>> {
    //     if let Some(cur_ctx) = self.context_mut(){
    //         if let Ok(ctx) = cur_ctx.to_owned().exit(){
    //             self.ctx = Some(ctx);
    //             return self.ctx.as_ref();
    //         }else {
    //             self.ctx = None;
    //         }
    //     }
    //     None
    // }
    
    fn context(&self) -> &Mutex<Option<Context>> {
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
    
    fn hooks(&self) -> &Vec<Box<dyn JobHook>>{
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
    
    fn start(&mut self) -> Result<(), Box<dyn Error>>{
        panic!("还未实现")
    }

    pub(crate) fn is_running(&self) -> bool {
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
    
    // 初始化hooks字段
    #[cfg(target_os = "linux")]
    fn btrfs_snapshot(&mut self, snapshot_path: &str, mirror: MirrorConfig){
        let btrfs_snapshot = BtrfsSnapshotHook::new(self.name(), snapshot_path, mirror);
        // self.hooks.push(Box::<T>::new(btrfs_snapshot.into()));
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

    pub(crate) fn prepare_log_file(&mut self, append: bool) -> Result<(), Box<dyn Error>>{
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

            let ctx = self.context().lock().unwrap();
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
}
