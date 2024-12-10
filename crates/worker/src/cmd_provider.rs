use std::collections::HashMap;
use std::error::Error;
use std::{fs, io, thread};
use std::ffi::{OsStr, OsString};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crossbeam_channel::bounded;
use crossbeam_channel::Sender;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::Ordering;
use anymap::AnyMap;
use chrono::{DateTime, Duration, Utc};
use libc::{getgid, getuid};
use log::{debug, error, info, warn};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use regex::Regex;
use shlex::Shlex;
use crate::base_provider::{BaseProvider};
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use internal::util::{find_all_submatches_in_file, extract_size_from_log};
use crate::hooks::HookType;
use crate::runner::{err_process_not_started, CmdJob};

#[derive(Clone)]
pub(crate) struct CmdConfig{
    pub(crate) name: String,

    pub(crate) upstream_url: String,
    pub(crate) command: String,

    pub(crate) working_dir: String,
    pub(crate) log_dir: String,
    pub(crate) log_file: String,

    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
    pub(crate) env: HashMap<String, String>,
    pub(crate) fail_on_match: String,
    pub(crate) size_pattern: String,
}

pub(crate) struct CmdProvider{
    pub(crate) base_provider: BaseProvider,
    cmd_config: CmdConfig,
    command: Vec<String>,
    data_size: String,
    fail_on_match: Option<Regex>,
    size_pattern: Option<Regex>,

}
// 
impl CmdProvider{
    pub(crate) fn new(mut c: CmdConfig) -> Result<Self, Box<dyn Error>>{
        // TODO: 检查config选项
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }
        let mut provider = CmdProvider{
            base_provider: BaseProvider{
                name: c.name.clone(),
                ctx: Arc::new(Mutex::new(Some(Context::new()))),
                interval: c.interval,
                retry: c.retry,
                timeout: c.timeout,

                is_master: false,
                cmd: None,
                log_file_fd: None,
                is_running: None,
                cgroup: None,
                zfs: None,
                docker: None,
                hooks: vec![],
            },

            cmd_config: c.clone(),
            command: vec![],
            data_size: "".to_string(),
            fail_on_match: None,
            size_pattern: None,
        };
        if let Some(ctx) = provider.base_provider.ctx.lock().unwrap().as_mut(){
            let mut value = AnyMap::new();
            value.insert(c.working_dir);
            ctx.set(_WORKING_DIR_KEY.to_string(), value);

            let mut value = AnyMap::new();
            value.insert(c.log_dir);
            ctx.set(_LOG_DIR_KEY.to_string(), value);

            let mut value = AnyMap::new();
            value.insert(c.log_file);
            ctx.set(_LOG_FILE_KEY.to_string(), value);
        }
        let cmd: Vec<String> = Shlex::new(&*c.command).collect();
        if cmd.len() == 0 {
            return Err("未检测到命令".into())
        }
        provider.command = cmd;
        if c.fail_on_match.len() > 0{
            match Regex::new(&*c.fail_on_match) {
                Err(e) => {
                    return Err(format!("匹配正则表达式fail_on_match失败：{}", e).into())
                }
                Ok(fail_on_match) => {
                    provider.fail_on_match = Some(fail_on_match);
                }
            }
        }
        if c.size_pattern.len() > 0{
            match Regex::new(&*c.size_pattern) {
                Err(e) => {
                    return Err(format!("匹配正则表达式size_pattern失败：{}", e).into())
                }
                Ok(size_pattern) => {
                    provider.size_pattern = Some(size_pattern);
                }
            }
        }

        Ok(provider)
    }
    
    /// cmd配置base_provider字段中的cmd字段
    fn cmd(&mut self){
        let mut env: HashMap<String, String> = [
            ("RTSYNC_MIRROR_NAME".to_string(), self.base_provider.name().to_string()),
            ("RTSYNC_WORKING_DIR".to_string(), self.base_provider.working_dir()),
            ("RTSYNC_UPSTREAM_URL".to_string(), self.cmd_config.upstream_url.clone()),
            ("RTSYNC_LOG_DIR".to_string(), self.base_provider.log_dir()),
            ("RTSYNC_LOG_FILE".to_string(), self.base_provider.log_file())]
            .into();

        for (k, v) in self.cmd_config.env.iter(){
            env.insert(k.to_string(), v.to_string());
        }

        let mut cmd_job: CmdJob;
        let mut args: Vec<String> = Vec::new();
        let use_docker = self.base_provider.docker_ref().is_some();

        let working_dir = self.base_provider.working_dir();
        
        if let Some(d) = self.base_provider.docker_ref(){
            let c = "docker";
            args.extend(vec!["run".to_string(), "--rm".to_string(),
                             "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                             "--name".to_string(), self.base_provider.name().parse().unwrap(),
                             "-w".to_string(), working_dir.clone()]);
            // 指定用户
            unsafe {
                args.extend(vec!["-u".to_string(),
                                 format!("{}:{}",getuid().to_string(), getgid().to_string())]);
            }
            // 添加卷
            for vol in d.volumes.iter(){
                debug!("数据卷: {}", &vol);
                args.extend(vec!["-v".to_string(), vol.clone()])
            }
            // 设置环境变量
            for (key, value) in env.iter(){
                let kv = format!("{}={}", key, value);
                args.extend(vec!["-e".to_string(), kv.clone()])
            }
            // 设置内存限制
            if d.memory_limit.0 != 0{
                args.extend(vec!["-m".to_string(), format!("{}", d.memory_limit.value())])
            }
            // 添加选项
            args.extend(d.options.iter().cloned());
            // 添加镜像和command
            args.push(d.image.clone());
            // 添加command
            args.extend(self.command.iter().cloned());


            cmd_job = CmdJob{
                cmd: Command::new(c),
                result: None,
                working_dir: working_dir.clone(),
                env: env.clone(),
                log_file: None,
                finished: Mutex::new(bounded(1).1),
                ret_err: None,
            };
            cmd_job.cmd.args(&args);

        }else {
            if self.command.len() == 1{
                cmd_job = CmdJob{
                    cmd: Command::new(&self.command[0]),
                    result: None,
                    working_dir: self.working_dir().clone(),
                    env: env.clone(),
                    log_file: None,
                    finished: Mutex::new(bounded(1).1),
                    ret_err: None,
                };
            }else if self.command.len() > 1 {
                let c = self.command[0].clone();
                let args = self.command[1..].to_vec();
                cmd_job = CmdJob{
                    cmd: Command::new(c),
                    result: None,
                    working_dir: self.working_dir().clone(),
                    env: env.clone(),
                    log_file: None,
                    finished: Mutex::new(bounded(1).1),
                    ret_err: None,
                };
                cmd_job.cmd.args(&args);
            }else {
                panic!("命令的长度最少是1！")
            }
        }

        if !use_docker {
            debug!("在 {} 位置执行 {} 命令", self.command[0], &working_dir);

            // 如果目录不存在，则创建目录
            if let Err(err) = fs::read_dir(&working_dir) {
                if err.kind() == io::ErrorKind::NotFound {
                    debug!("创建文件夹：{}", &working_dir);
                    if fs::create_dir_all(&working_dir).is_ok(){
                        if let Err(e) = fs::set_permissions(&working_dir, Permissions::from_mode(0o755)) {
                            error!("更改文件夹 {} 权限失败: {}",&working_dir, err)
                        }
                    }else {
                        error!("创建文件夹 {} 失败: {}", &working_dir, err)
                    }
                }
            }
            cmd_job.cmd.current_dir(&working_dir);
            cmd_job.cmd.envs(crate::runner::new_environ(&env, true));
        }

        self.base_provider.cmd = Some(Arc::new(RwLock::new(cmd_job)));
        
    }
    
    /// start操作base_provider字段中的cmd字段，使用的是非阻塞的spawn
    fn start(&mut self) -> Result<(), Box<dyn Error>>{
        if let Ok(mut c) = self.base_provider.cmd.as_mut().unwrap().write(){
            debug!("命令启动：{:?}", c.cmd.get_args().collect::<Vec<&OsStr>>());
            c.finished = Mutex::new(bounded(1).1);
            
            if let Err(e) = c.cmd.spawn(){
                return Err(Box::new(e))
            }
        }
        Ok(())
    }
    
    fn terminate(&mut self) -> Result<(), Box<dyn Error>>{
        if let Ok(mut cmd) = self.base_provider.cmd
            .as_ref().expect("没有cmd字段").write()
        {
            debug!("正在终止provider：{}", self.base_provider.name());
            if !self.base_provider.is_running(){
                warn!("调用了终止函数，但是此时没有检测到 {} 正在运行", self.base_provider.name());
                return Ok(())
            }

            /////////////////////////////////
            // cmd的terminate方法
            if cmd.cmd.get_program().eq("") || cmd.result.is_none(){
                return Err(err_process_not_started())
            }
            if let Some(d) = self.base_provider.docker_ref(){
                Command::new("docker")
                    .arg("stop")
                    .arg("-t")
                    .arg("2")
                    .arg(self.base_provider.name())
                    .output()?;
                return Ok(())
            }
            if let Some(child) = cmd.result.as_mut(){
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
            }

            Ok(())
            /////////////////////////////////
        }else {
            panic!("在调用terminate时，无法在BaseProvider的cmd字段上获得锁")
        }
    }
    
}


impl MirrorProvider for CmdProvider{

    fn upstream(&self) -> String {
        self.cmd_config.upstream_url.clone()
    }
    
    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Command
    }
    
    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = "".to_string();
        if let Err(e) = MirrorProvider::start(self){
            return Err(e);
        }
        started.send(()).expect("发送失败");
        if let Err(e) = self.base_provider.wait(){
            return Err(e);
        }
        if let Some(fail_on_match) = self.fail_on_match.as_ref(){
            match find_all_submatches_in_file(&*self.base_provider.log_file(), fail_on_match){
                Ok(mut submatches) => {
                    info!("在文件中找到所有的子匹配项：{:?}", submatches);
                    if submatches.len() != 0{
                        debug!("匹配失败{:?}", submatches);
                        return Err(format!("匹配正则表达式失败，找到 {} 个匹配项", submatches.len()).into());
                    }
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
        if let Some(size_pattern) = self.size_pattern.as_ref(){
            self.data_size = extract_size_from_log(&*self.base_provider.log_file(), size_pattern)
                .unwrap_or_default();
        }
        Ok(())
    }
    
    fn start(&mut self) -> Result<(), Box<dyn Error>> {
        if self.is_running(){
            return Err("provider现在正在运行".into())
        }
        self.cmd();
        if let Err(e) = self.base_provider.prepare_log_file(false){
            return Err(e);
        }
        if let Err(e) = self.start(){
            return Err(e);
        }
        self.base_provider.is_running.as_mut().unwrap().store(true, Ordering::SeqCst);
        debug!("将is_running字段设置为true :{}", self.base_provider.name());

        Ok(())
        
    }
    
    fn add_hook(&mut self, hook: HookType) {
        self.base_provider.add_hook(hook);
    }

    fn log_dir(&self) -> String {
        self.base_provider.log_dir()
    }

    fn log_file(&self) -> String {
        self.base_provider.log_file()
    }

    fn data_size(&self) -> String {
        self.data_size.clone()
    }
}