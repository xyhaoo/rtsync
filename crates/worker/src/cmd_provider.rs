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
use std::sync::atomic::{AtomicBool, Ordering};
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

pub(crate) struct CmdProvider{
    pub(crate) base_provider: BaseProvider,
    pub(crate) cmd_config: CmdConfig,
    command: Vec<String>,
    data_size: String,
    fail_on_match: Option<Regex>,
    size_pattern: Option<Regex>,

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
            base_provider: BaseProvider{
                name: c.name.clone(),
                ctx: Arc::new(Mutex::new(Some(Context::new()))),
                interval: c.interval,
                retry: c.retry,
                timeout: c.timeout,

                is_master: false,
                cmd: None,
                log_file_fd: None,
                is_running: Some(Arc::new(AtomicBool::new(false))),
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
        if let Some(ctx) = provider.context().lock().unwrap().as_mut(){
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

        println!("provider.cmd: {:?}", cmd);

        provider.command = cmd;
        if c.fail_on_match.len() > 0{
            match Regex::new(&*c.fail_on_match) {
                Err(e) => {
                    return Err(format!("匹配正则表达式fail_on_match失败：{}", e).into())
                }
                Ok(fail_on_match) => {
                    println!("debug：初始化regix成功：{:?}", fail_on_match);
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
                    println!("debug: size_pattern regex初始化成功{:?}", size_pattern);
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
                             // "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
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
                cmd: RwLock::new(Command::new(c)),
                result: RwLock::new(None),
                working_dir: working_dir.clone(),
                env: env.clone(),
                log_file: None,
                finished: RwLock::new(None),
                ret_err: None,
            };
            { cmd_job.cmd.write().unwrap().args(&args); }

        }else {
            if self.command.len() == 1{
                cmd_job = CmdJob{
                    cmd: RwLock::new(Command::new(&self.command[0])),
                    result: RwLock::new(None),
                    working_dir: self.working_dir().clone(),
                    env: env.clone(),
                    log_file: None,
                    finished: RwLock::new(None),
                    ret_err: None,
                };
            }else if self.command.len() > 1 {
                let c = self.command[0].clone();
                let args = self.command[1..].to_vec();
                cmd_job = CmdJob{
                    cmd: RwLock::new(Command::new(c)),
                    result: RwLock::new(None),
                    working_dir: self.working_dir().clone(),
                    env: env.clone(),
                    log_file: None,
                    finished: RwLock::new(None),
                    ret_err: None,
                };
                { cmd_job.cmd.write().unwrap().args(&args); }
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
                cmd_job.cmd.write().unwrap()
                    .current_dir(&working_dir)
                    .envs(crate::runner::new_environ(env, true));
            }
        }

        self.base_provider.cmd = Some(cmd_job);
        
    }

    /// 启动配置好的命令
    fn start(&mut self) -> Result<(), Box<dyn Error>>{
        self.base_provider.start()
    }
    
}


impl MirrorProvider for CmdProvider{
    fn name(&self) -> String {
        self.base_provider.name()
    }

    fn upstream(&self) -> String {
        self.cmd_config.upstream_url.clone()
    }

    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Command
    }

    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = "".to_string();

        println!("debug: 等待启动。。。");

        MirrorProvider::start(self)?;
        
        started.send(()).expect("发送失败");

        println!("debug: 等待结束。。。");

        match self.base_provider.wait() {
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
                .unwrap_or_default();
            println!("debug: 找到的匹配项：{:?}", self.data_size);
        }
        Ok(())
    }
    fn start(&mut self) -> Result<(), Box<dyn Error>> {
        if self.is_running(){
            return Err("provider现在正在运行".into())
        }
        self.cmd();
        
        self.base_provider.prepare_log_file(false)?;

        self.start()?;
        
        self.base_provider.is_running.as_mut().unwrap().store(true, Ordering::SeqCst);
        
        println!("debug: 将is_running字段设置为true :{}", self.base_provider.name());
        
        debug!("将is_running字段设置为true :{}", self.base_provider.name());

        Ok(())
        
    }

    fn terminate(&self) -> Result<(), Box<dyn Error>> {
        println!("debug: 进入terminate！");
        self.base_provider.terminate()
    }

    fn is_running(&self) -> bool {
        self.base_provider.is_running()
    }
    fn add_hook(&mut self, hook: HookType) {
        self.base_provider.add_hook(hook);
    }

    fn hooks(&self) -> &Vec<Box<dyn JobHook>> {
        self.base_provider.hooks()
    }

    fn interval(&self) -> Duration {
        self.base_provider.interval()
    }

    fn timeout(&self) -> Duration {
        self.base_provider.timeout()
    }

    fn working_dir(&self) -> String {
        self.base_provider.working_dir()
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

    fn enter_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.enter_context()
    }

    fn exit_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.exit_context()
    }

    fn context(&self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.context()
    }

    fn zfs(&self) -> Option<&ZfsHook> {
        self.base_provider.zfs.as_ref()
    }
}