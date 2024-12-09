use std::collections::HashMap;
use std::error::Error;
use std::{fs, io, thread};
use std::ffi::OsStr;
use std::fs::Permissions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crossbeam_channel::{Sender, Receiver, bounded};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::Ordering;
use chrono::Duration;
use libc::{getgid, getuid};
use log::{debug, error, warn};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use internal;
use internal::util::extract_size_from_rsync_log;
use crate::base_provider::{BaseProvider, HookType};
use crate::cgroup::CGroupHook;
use crate::common;
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::JobHook;
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::{err_process_not_started, CmdJob};
use crate::zfs_hook::ZfsHook;

#[derive(Clone)]
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
pub(crate) struct RsyncProvider<T: Clone> {
    pub(crate) base_provider: BaseProvider<T>,
    rsync_config: RsyncConfig,
    options: Vec<String>,
    data_size: String,
}

impl RsyncProvider<String> {
    pub(crate) fn new(mut c: RsyncConfig) -> Result<Self, Box<dyn Error>> {
        // TODO: 检查config选项
        if !c.upstream_url.ends_with("/"){
            return Err("rsync上游URL应该以'/'结尾".into());
        }
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }

        let mut provider = RsyncProvider{
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
                hooks: Vec::new(),
            },
            rsync_config: c.clone(),
            options: Vec::new(),
            data_size: String::default(),
        };
        if c.rsync_cmd.len() == 0 {
            provider.rsync_config.rsync_cmd = "rsync".to_string();
        }
        if c.rsync_env.len() == 0 {
            provider.rsync_config.rsync_env = HashMap::new();
        }
        if c.username.len() == 0 {
            provider.rsync_config.rsync_env.insert("USER".to_string(), c.username);
        }
        if c.password.len() == 0{
            provider.rsync_config.rsync_env.insert("RSYNC_PASSWORD".to_string(), c.password);
        }
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
            options.extend(c.extra_options)
        }
        provider.options = options;
        if let Some(ctx) = provider.base_provider.ctx.lock().unwrap().as_mut(){
            ctx.set(_WORKING_DIR_KEY.to_string(), c.working_dir);
            ctx.set(_LOG_DIR_KEY.to_string(), c.log_dir);
            ctx.set(_LOG_FILE_KEY.to_string(), c.log_file);
        }

        Ok(provider)
    }
    
    /// cmd配置base_provider字段中的cmd字段
    fn cmd(&mut self){
        let working_dir = self.base_provider.working_dir();

        let mut command = Vec::new();
        command.push(self.rsync_config.rsync_cmd.clone());
        command.extend(self.options.clone());
        command.push(self.rsync_config.upstream_url.clone());
        command.push(working_dir.clone());

        ///////////////////////////////////////
        let mut cmd_job: CmdJob;
        let mut args: Vec<String> = Vec::new();
        let use_docker = self.docker().is_some();

        if let Some(d) = self.docker(){
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
            for (key, value) in self.rsync_config.rsync_env.iter(){
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
            args.extend(command.iter().cloned());


            cmd_job = CmdJob{
                cmd: Command::new(c),
                result: None,
                working_dir: working_dir.clone(),
                env: self.rsync_config.rsync_env.clone(),
                log_file: None,
                finished: Mutex::new(bounded(1).1),
                ret_err: None,
            };
            cmd_job.cmd.args(&args);

        }else {
            if command.len() == 1{
                cmd_job = CmdJob{
                    cmd: Command::new(&command[0]),
                    result: None,
                    working_dir: self.working_dir().clone(),
                    env: self.rsync_config.rsync_env.clone(),
                    log_file: None,
                    finished: Mutex::new(bounded(1).1),
                    ret_err: None,
                };
            }else if command.len() > 1 {
                let c = command[0].clone();
                let args = command[1..].to_vec();
                cmd_job = CmdJob{
                    cmd: Command::new(c),
                    result: None,
                    working_dir: self.working_dir().clone(),
                    env: self.rsync_config.rsync_env.clone(),
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
            debug!("在 {} 位置执行 {} 命令", command[0], &working_dir);

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
            cmd_job.cmd.envs(crate::runner::new_environ(&self.rsync_config.rsync_env, true));
        }

        self.base_provider.cmd = Some(Arc::new(RwLock::new(cmd_job)));
        ///////////////////////////////////////

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
            if let Some(d) = self.docker(){
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
impl<T: Clone> RsyncProvider<T>  {
    
}

impl MirrorProvider for RsyncProvider<String>  {
    type ContextStoreVal = String;

    fn upstream(&self) -> String {
        self.rsync_config.upstream_url.clone()
    }
    
    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Rsync
    }
    
    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = "".to_string();
        if let Err(e) = MirrorProvider::start(self){
            return Err(e);
        }
        started.send(()).expect("发送失败");
        match self.base_provider.wait() {
            Err(err) =>{
                return Err(err);
            }
            Ok(exit_code) if exit_code != 0 => {
                if let Some(msg) = internal::util::translate_rsync_error_code(exit_code){
                    debug!("rsync 异常终止： {} ({})", exit_code, msg);
                    if let Some(log_file_fd) = self.base_provider.log_file_fd.as_mut(){
                        log_file_fd.write(format!("{}\n",msg).as_bytes())?;
                    }
                    return Err(msg.into());
                }
            }
            _ => {}
        }
        if let Some(size) = extract_size_from_rsync_log(&*self.base_provider.log_file()) {
            self.data_size = size
        };
        Ok(())
    }

    fn start(&mut self) -> Result<(), Box<dyn Error>> {
        if self.is_running(){
            return Err("provider现在正在运行".into())
        }
        self.cmd();
    
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

    fn data_size(&self) -> String {
        self.data_size.clone()
    }
    
}
