use std::collections::HashMap;
use std::error::Error;
use std::{fs, io, thread};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs::Permissions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crossbeam_channel::{Sender, Receiver, bounded};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use anymap::AnyMap;
use chrono::Duration;
use libc::{getgid, getuid};
use log::{debug, error, warn};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use internal;
use internal::util::extract_size_from_rsync_log;
use crate::base_provider::{BaseProvider};
use crate::cgroup::CGroupHook;
use crate::common;
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::{HookType, JobHook};
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::{err_process_not_started, CmdJob};
use crate::zfs_hook::ZfsHook;

#[derive(Clone, Default)]
pub(crate) struct RsyncConfig{
    pub(crate) name: String,
    pub(crate) rsync_cmd: String,

    pub(crate) upstream_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) exclude_file: String,

    pub(crate) extra_options: Vec<String>,
    pub(crate) overridden_options: Vec<String>,
    pub(crate) rsync_never_timeout: bool,
    pub(crate) rsync_timeout_value: i64,
    pub(crate) rsync_env: HashMap<String, String>,

    pub(crate) working_dir: String,
    pub(crate) log_dir: String,
    pub(crate) log_file: String,

    pub(crate) use_ipv6: bool,
    pub(crate) use_ipv4: bool,

    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
}

// RsyncProvider提供了基于rsync的同步作业的实现
#[derive(Clone, Default)]
pub(crate) struct RsyncProvider {
    pub(crate) base_provider: Arc<RwLock<BaseProvider>>,
    pub(crate) rsync_config: Arc<RsyncConfig>,
    options: Arc<Vec<String>>,
    data_size: Arc<String>,
}

unsafe impl Send for RsyncProvider{}
unsafe impl Sync for RsyncProvider{}

impl RsyncProvider {
    pub(crate) fn new(mut c: RsyncConfig) -> Result<Self, Box<dyn Error>> {
        // TODO: 检查config选项
        if !c.upstream_url.ends_with("/"){
            return Err("rsync上游URL应该以'/'结尾".into());
        }
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }
        
        // FIXME: ubuntu22.04上如果直接使用脚本文件运行Command则会Text file busy，用bash来运行，使用.arg()添加脚本文件路径来避免文件阻塞
        // FIXME: 这是ubuntu的问题，根本原因是用来设置script_file权限的文件句柄在ubuntu上没有正常关闭，导致Command使用这个文件运行时显示为已被占用
        if c.rsync_cmd.len() == 0 {
            c.rsync_cmd = "rsync".to_string();
        }
        if c.username.len() != 0 {
            c.rsync_env.insert("USER".to_string(), c.username.clone());
        }
        if c.password.len() != 0{
            c.rsync_env.insert("RSYNC_PASSWORD".to_string(), c.password.clone());
        }

        let mut provider = RsyncProvider{
            base_provider: Arc::new(RwLock::new(BaseProvider::new(c.name.clone(), c.interval, c.retry, c.timeout))),
            rsync_config: Arc::from(c.clone()),
            ..RsyncProvider::default()
        };
        
        // 根据配置填充provider的options字段，作为rsync命令的参数
        let mut options: Vec<String> = vec![
            "-aHvh", "--no-o", "--no-g", "--stats",
            "--filter" , "risk .~tmp~/", "--exclude", ".~tmp~/",
            "--delete", "--delete-after", "--delay-updates",
            "--safe-links",
        ].iter().map(|option|option.to_string()).collect();

        if !c.overridden_options.is_empty(){
            options = c.overridden_options
        }
        if !c.rsync_never_timeout{
            let mut timeo = 120;
            if c.rsync_timeout_value > 0{
                timeo = c.rsync_timeout_value
            }
            options.push(format!("--timeout={}", timeo));
        }
        if c.use_ipv6{
            options.push("-6".to_string());
        }else if c.use_ipv4{
            options.push("-4".to_string());
        }
        if !c.exclude_file.is_empty(){
            options.push("--exclude-from".to_string());
            options.push(c.exclude_file);
        }
        if !c.extra_options.is_empty(){
            println!("debug: 添加extra_options {:?}", c.extra_options.clone());
            options.extend(c.extra_options)
        }
        provider.options = Arc::from(options);
        if let Some(ctx) = provider.base_provider.write().unwrap().context().lock().unwrap().as_mut(){
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

        Ok(provider)
    }
    
    /// cmd配置base_provider字段中的cmd字段
    fn cmd(&self){
        let working_dir = self.working_dir();
        let mut command = Vec::new();
        command.push(self.rsync_config.rsync_cmd.clone());
        println!("debug：添加了rsync_cmd：{}", self.rsync_config.rsync_cmd.clone());
        command.extend(self.options.as_ref().clone());
        println!("debug：添加了options：{:?}", self.options.clone());
        command.push(self.rsync_config.upstream_url.clone());
        println!("debug：添加了upstream_url：{:?}", self.rsync_config.upstream_url.clone());
        command.push(working_dir.clone());
        println!("debug：添加了working_dir：{:?}", working_dir.clone());
        let env = self.rsync_config.rsync_env.clone();
        println!("debug：rsync_env ：{:?}", &env);
        
        let cmd_job: CmdJob;
        let mut args: Vec<String> = Vec::new();
        let mut base_provider_lock = self.base_provider.write().unwrap();
        let use_docker = base_provider_lock.docker_ref().is_some();

        if let Some(d) = base_provider_lock.docker_ref().as_ref(){
            let c = "docker";
            // --rm 容器在执行完任务后会自动删除
            // -a 
            // --name 容器命名
            // -w 容器内部工作目录
            args.extend(vec!["run".to_string(), "--rm".to_string(),
                             // "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                             "--name".to_string(), d.name(base_provider_lock.name().parse().unwrap()),
                             "-w".to_string(), working_dir.clone()]);
            // -u 设置容器运行时的用户:用户组
            unsafe {
                args.extend(vec!["-u".to_string(),
                                 format!("{}:{}",getuid().to_string(), getgid().to_string())]);
            }
            // -v 添加卷: 把主机的文件或目录挂载到容器内
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
                args.extend(vec!["-m".to_string(), format!("{}b", d.memory_limit.value())])
            }
            // 添加选项
            args.extend(d.options.iter().cloned());
            // 添加镜像和command
            args.push(d.image.clone());
            // 添加command
            args.extend(command.iter().cloned());
            
            cmd_job = CmdJob::new(Command::new(c), working_dir.clone(), env.clone());
            { cmd_job.cmd.lock().unwrap().args(&args); }

        }else {
            if command.len() == 1{
                // 已证实：ubuntu22.04上如果直接使用脚本文件运行Command则会Text file busy，用bash来运行避免文件阻塞
                // 正常运行时cmd: Command::new(&command[0])，把下面cmd_job.cmd.arg(&command[0]);删除
                cmd_job = CmdJob::new(Command::new("bash"), working_dir.clone(), env.clone());
                { cmd_job.cmd.lock().unwrap().arg(&command[0]); }
                
            }else if command.len() > 1 {
                // 已证实：ubuntu22.04上如果直接使用脚本文件运行Command则会Text file busy，用bash来运行避免文件阻塞
                // 正常运行时cmd: Command::new(c)，把下面cmd_job.cmd.arg(c)删除
                let c = command[0].clone();
                let args = command[1..].to_vec();

                cmd_job = CmdJob::new(Command::new("bash"), working_dir.clone(), env.clone());
                {
                    cmd_job.cmd
                        .lock().unwrap()
                        .arg(c)
                        .args(&args);
                }
                
            }else {
                panic!("命令的长度最少是1！")
            }
        }

        if !use_docker {
            debug!("在 {} 位置执行 {} 命令", &working_dir, command[0]);

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
        drop(base_provider_lock)
    }
    
    
}

impl MirrorProvider for RsyncProvider  {
    fn name(&self) -> String {
        self.base_provider.read().unwrap().name()
    }

    fn upstream(&self) -> String {
        self.rsync_config.upstream_url.clone()
    }
    
    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Rsync
    }
    
    // 运行rsync命令并等待结果，记录同步文件大小。如果rsync异常，则解析错误并将错误返回
    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = Arc::from(String::new());
        
        println!("debug: 等待启动。。。");
        
        MirrorProvider::start(self)?;
        
        started.send(()).expect("发送失败");
        
        println!("debug: 等待结束。。。");

        // 获取锁并执行 wait() 操作
        let base_provider = self.base_provider.read().unwrap();
        let result = base_provider.wait();
        
        // 处理等待的结果
        match result {
            Err(err) => {
                return Err(err);
            },
            Ok(exit_code) if exit_code != 0 => {
                println!("捕获到非正常退出！{exit_code}");
                
                return if let Some(msg) = internal::util::translate_rsync_error_code(exit_code) {
                    debug!("rsync 异常终止： {} ({})", exit_code, msg);

                    // 释放锁后再处理 log_file_fd
                    drop(base_provider);
                    if let Some(log_file_fd) = self.base_provider.write().unwrap().log_file_fd.as_mut() {
                        log_file_fd.write(format!("{}\n", msg).as_bytes())?;
                    }
                    Err(msg.into())
                } else {
                    Err("解析错误状态码失败".into())
                }
            }
            _ => {
                drop(base_provider);
            }
        }
        if let Some(size) = extract_size_from_rsync_log(&*self.log_file()) {
            self.data_size = size.into()
        };
        Ok(())
    }

    // 根据配置生成rsync命令，启动一个新进程来运行
    fn start(&self) -> Result<(), Box<dyn Error>> {
        if self.is_running(){
            return Err("provider现在正在运行".into())
        }
        
        self.cmd();
        
        let mut base_provider_lock = self.base_provider.write().unwrap();
        
        if let Err(e) = base_provider_lock.prepare_log_file(false){
            error!("prepare_log_file 失败{}", e);
            return Err(e);
        }
        
        if let Err(e) = base_provider_lock.start(){
            error!("start失败: {}", e);
            return Err(e);
        }
        
        base_provider_lock.is_running.store(true, Ordering::SeqCst);
        
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
