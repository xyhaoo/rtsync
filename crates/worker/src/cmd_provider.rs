use std::collections::HashMap;
use std::error::Error;
use std::{fs, io, thread};
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crossbeam_channel::bounded;
use crossbeam_channel::Sender;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use anymap::AnyMap;
use chrono::{DateTime, Duration, Utc};
use libc::{getgid, getuid};
use log::{debug, error, info, warn};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use regex::Regex;
use scopeguard::defer;
use shlex::Shlex;
use crate::base_provider::{BaseProvider};
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use internal::util::{find_all_submatches_in_file, extract_size_from_log};
use crate::docker::DockerHook;
use crate::hooks::{HookType, JobHook};
use crate::runner::{err_process_not_started, CmdJob};
use crate::zfs_hook::ZfsHook;

#[derive(Clone, Default)]
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

#[derive(Clone, Default)]
pub(crate) struct CmdProvider{
    pub(crate) base_provider: Arc<RwLock<BaseProvider>>,
    pub(crate) cmd_config: Arc<CmdConfig>,
    command: Arc<Vec<String>>,
    data_size: Arc<String>,
    fail_on_match: Arc<Option<Regex>>,
    size_pattern: Arc<Option<Regex>>,
}
unsafe impl Send for CmdProvider {}
unsafe impl Sync for CmdProvider {}

//
impl CmdProvider{
    pub(crate) fn new(mut c: CmdConfig) -> Result<Self, Box<dyn Error>>{
        // TODO: 检查config选项
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }
        let mut provider = CmdProvider{
            base_provider: Arc::new(RwLock::new(BaseProvider::new(c.name.clone(), c.interval, c.retry, c.timeout))),
            cmd_config: Arc::from(c.clone()),
            ..CmdProvider::default()
        };
        let base_provider_lock = provider.base_provider.write().unwrap();
        if let Some(ctx) = base_provider_lock.ctx.lock().unwrap().as_mut(){
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
        drop(base_provider_lock);
        
        let cmd: Vec<String> = Shlex::new(&*c.command).collect();
        if cmd.len() == 0 {
            return Err("未检测到命令".into())
        }

        println!("debug: provider.cmd: {:?}", cmd);

        provider.command = Arc::from(cmd);
        if c.fail_on_match.len() > 0{
            match Regex::new(&*c.fail_on_match) {
                Err(e) => {
                    return Err(format!("匹配正则表达式fail_on_match失败：{}", e).into())
                }
                Ok(fail_on_match) => {
                    println!("debug：初始化regex成功：{:?}", fail_on_match);
                    provider.fail_on_match = Arc::from(Some(fail_on_match));
                }
            }
        }
        if c.size_pattern.len() > 0{
            match Regex::new(&*c.size_pattern) {
                Err(e) => {
                    return Err(format!("匹配正则表达式size_pattern失败：{}", e).into())
                }
                Ok(size_pattern) => {
                    println!("debug: size_pattern regex初始化成功{:?}", size_pattern);
                    provider.size_pattern = Arc::from(Some(size_pattern));
                }
            }
        }
        Ok(provider)
    }
    
    /// cmd配置base_provider字段中的cmd字段
    fn cmd(&self){
        let working_dir = self.working_dir();
        let mut base_provider_lock = self.base_provider.write().unwrap();
        let mut env: HashMap<String, String> = [
            ("RTSYNC_MIRROR_NAME".to_string(), base_provider_lock.name().to_string()),
            ("RTSYNC_WORKING_DIR".to_string(), base_provider_lock.working_dir()),
            ("RTSYNC_UPSTREAM_URL".to_string(), self.cmd_config.upstream_url.clone()),
            ("RTSYNC_LOG_DIR".to_string(), base_provider_lock.log_dir()),
            ("RTSYNC_LOG_FILE".to_string(), base_provider_lock.log_file())
        ].into();

        for (k, v) in self.cmd_config.env.iter(){
            env.insert(k.to_string(), v.to_string());
        }

        let mut cmd_job: CmdJob;
        let mut args: Vec<String> = Vec::new();
        let use_docker = base_provider_lock.docker_ref().is_some();
        
        if let Some(d) = base_provider_lock.docker_ref().as_ref(){
            let c = "docker";
            args.extend(vec!["run".to_string(), "--rm".to_string(),
                             // "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                             "--name".to_string(), base_provider_lock.name().parse().unwrap(),
                             "-w".to_string(), working_dir.clone()]);
            // 指定用户
            unsafe {
                args.extend(vec!["-u".to_string(),
                                 format!("{}:{}",getuid().to_string(), getgid().to_string())]);
            }
            // 添加卷
            for vol in d.volumes.iter(){
                println!("debug: 数据卷 {}", &vol);
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
                args.extend(vec!["-m".to_string(), format!("{}b", d.memory_limit.value())])
            }
            // 添加选项
            args.extend(d.options.iter().cloned());
            // 添加镜像和command
            args.push(d.image.clone());
            // 添加command
            args.extend(self.command.iter().cloned());
            
            cmd_job = CmdJob::new(Command::new(c), working_dir.clone(), env.clone());
            { cmd_job.cmd.lock().unwrap().args(&args); }
            
        }else {
            if self.command.len() == 1{
                cmd_job = CmdJob::new(Command::new(&self.command[0]), working_dir.clone(), env.clone());
                
            }else if self.command.len() > 1 {
                let c = self.command[0].clone();
                let args = self.command[1..].to_vec();
                
                cmd_job = CmdJob::new(Command::new(c), working_dir.clone(), env.clone());
                { cmd_job.cmd.lock().unwrap().args(&args); }
            }else {
                panic!("命令的长度最少是1！")
            }
        }

        if !use_docker {
            debug!("在 {} 位置执行 {} 命令", &working_dir, self.command[0]);
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
            {
                cmd_job.cmd
                    .lock().unwrap()
                    .current_dir(&working_dir)
                    .envs(crate::runner::new_environ(env, true));
            }

        }
        base_provider_lock.cmd = Some(cmd_job);
        drop(base_provider_lock);
    }

    
    
}


impl MirrorProvider for CmdProvider{
    fn name(&self) -> String {
        self.base_provider.read().unwrap().name()
    }

    fn upstream(&self) -> String {
        self.cmd_config.upstream_url.clone()
    }

    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Command
    }

    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = Arc::from(String::new());

        println!("debug: 等待启动。。。");

        MirrorProvider::start(self)?;
        
        started.send(()).expect("发送失败");

        println!("debug: 等待结束。。。");

        match self.base_provider.read().unwrap().wait() {
            Err(err) => {
                return Err(err);
            }
            Ok(exit_code) if exit_code != 0 => {
                println!("捕获到非正常退出！状态码：{exit_code}");
                return Err(format!("错误状态码： {}", exit_code).into());
            }
            // 正常退出
            _ => {}
        }
        
        if let Some(fail_on_match) = self.fail_on_match.as_ref(){
            match find_all_submatches_in_file(&*self.log_file(), fail_on_match){
                Ok(sub_matches) => {
                    println!("debug: 找到的匹配项：{:?}", sub_matches);
                    info!("在文件中找到所有的子匹配项：{:?}", sub_matches);
                    if sub_matches.len() != 0{
                        debug!("匹配失败{:?}", sub_matches);
                        return Err(format!("匹配正则表达式失败，找到 {} 个匹配项", sub_matches.len()).into());
                    }
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
        if let Some(size_pattern) = self.size_pattern.as_ref(){
            self.data_size = extract_size_from_log(&*self.log_file(), size_pattern)
                .unwrap_or_default()
                .into();
            println!("debug: 找到的匹配项：{:?}", self.data_size);
        }
        Ok(())
    }
    
    fn start(&self) -> Result<(), Box<dyn Error>> {
        if self.is_running(){
            return Err("provider现在正在运行".into())
        }
        self.cmd();
        let mut base_provider_lock = self.base_provider.write().unwrap();
        
        base_provider_lock.prepare_log_file(false)?;
        base_provider_lock.start()?;
        base_provider_lock.is_running.store(true, Ordering::SeqCst);
        
        println!("debug: 将is_running字段设置为true :{}", base_provider_lock.name());
        debug!("将is_running字段设置为true :{}", base_provider_lock.name());
        
        drop(base_provider_lock);
        Ok(())
    }

    fn terminate(&self) -> Result<(), Box<dyn Error>> {
        println!("debug: 进入terminate！");
        self.base_provider.read().unwrap().terminate()
    }

    fn is_running(&self) -> bool {
        self.base_provider.read().unwrap().is_running()
    }
    // fn zfs(&self) -> Option<&ZfsHook> {
    //     self.base_provider.zfs.as_ref()
    // }
    // 
    fn docker(&self) -> Arc<Option<DockerHook>> {
        self.base_provider.read().unwrap().docker_ref()
    }

    fn add_hook(&mut self, hook: HookType) {
        self.base_provider.write().unwrap().add_hook(hook);
    }

    fn hooks(&self) -> Arc<Mutex<Vec<Box<dyn JobHook>>>> {
        self.base_provider.read().unwrap()
            .hooks()
    }

    fn interval(&self) -> Duration {
        self.base_provider.read().unwrap().interval()
    }

    fn timeout(&self) -> Duration {
        self.base_provider.read().unwrap().timeout()
    }

    fn working_dir(&self) -> String {
        self.base_provider.read().unwrap().working_dir()
    }

    fn log_dir(&self) -> String {
        self.base_provider.read().unwrap().log_dir()
    }

    fn log_file(&self) -> String {
        self.base_provider.read().unwrap().log_file()
    }

    fn data_size(&self) -> String {
        self.data_size.to_string()
    }

    fn enter_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().unwrap()
            .enter_context()
    }

    fn exit_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().unwrap()
            .exit_context()
    }

    fn context(&self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().unwrap()
            .context()
    }
}