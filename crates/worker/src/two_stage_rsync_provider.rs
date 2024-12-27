use std::collections::HashMap;
use anyhow::{anyhow, Result};
use std::{fs, io};
use std::fs::Permissions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use tokio::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::Sender;
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;
use std::sync::Arc;
use anymap::AnyMap;
use chrono::Duration;
use crate::base_provider::BaseProvider;
use lazy_static::lazy_static;
use libc::{getgid, getuid};
use log::{debug, error};
use internal::util::extract_size_from_rsync_log;
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::{HookType, JobHook};
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::CmdJob;

#[derive(Clone, Default)]
pub(crate) struct TwoStageRsyncConfig {
    pub(crate) name: String,
    pub(crate) rsync_cmd: String,
    pub(crate) stage1_profile: String,

    pub(crate) upstream_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) exclude_file: String,

    pub(crate) extra_options: Vec<String>,
    pub(crate) rsync_never_timeout: bool,
    pub(crate) rsync_timeout_value: i64,
    pub(crate) rsync_env: HashMap<String, String>,

    pub(crate) working_dir: String,
    pub(crate) log_dir: String,
    pub(crate) log_file: String,

    pub(crate) use_ipv4: bool,
    pub(crate) use_ipv6: bool,

    pub(crate) interval: Duration,
    pub(crate) retry: i64,
    pub(crate) timeout: Duration,
}

// RsyncProvider提供了基于rsync的同步作业的实现
#[derive(Clone, Default)]
pub(crate) struct TwoStageRsyncProvider {
    pub(crate) base_provider: Arc<RwLock<BaseProvider>>,
    pub(crate) two_stage_rsync_config: Arc<TwoStageRsyncConfig>,
    stage1_options: Arc<Vec<String>>,
    stage2_options: Arc<Vec<String>>,
    data_size: Arc<String>,
}

unsafe impl Send for TwoStageRsyncProvider{}
unsafe impl Sync for TwoStageRsyncProvider{}

// 引用自: https://salsa.debian.org/mirror-team/archvsync/-/blob/master/bin/ftpsync#L431
lazy_static! {
    static ref  rsync_stage1_profiles: HashMap<&'static str, Vec<&'static str>> = [
        ("debian", vec![
            "--include=*.diff/",
            "--include=by-hash/",
            "--exclude=*.diff/Index",
            "--exclude=Contents*",
            "--exclude=Packages*",
            "--exclude=Sources*",
            "--exclude=Release*",
            "--exclude=InRelease",
            "--exclude=i18n/*",
            "--exclude=dep11/*",
            "--exclude=installer-*/current",
            "--exclude=ls-lR*",]),
        ("debian-oldstyle", vec![
            "--exclude=Packages*", "--exclude=Sources*", "--exclude=Release*",
		    "--exclude=InRelease", "--exclude=i18n/*", "--exclude=ls-lR*", "--exclude=dep11/*",]),
    ]
        .iter()
        .cloned()
        .collect();
}

impl TwoStageRsyncProvider {
    pub(crate) async fn new(mut c: TwoStageRsyncConfig) -> Result<Self>{
        // TODO: 检查config选项
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }
        if !c.username.is_empty(){
            c.rsync_env.insert("USER".to_string(), c.username.clone());
        }
        if c.password.is_empty(){
            c.rsync_env.insert("RSYNC_PASSWORD".to_string(), c.password.clone());
        }
        if c.rsync_cmd.is_empty(){
            c.rsync_cmd = "rsync".to_string();
        }
        let mut provider = TwoStageRsyncProvider{
            base_provider: Arc::new(RwLock::new(BaseProvider::new(c.name.clone(), c.interval, c.retry, c.timeout))),
            two_stage_rsync_config: Arc::from(c.clone()),
            stage1_options: Arc::from(["-aHvh", "--no-o", "--no-g", "--stats",
                "--filter", "risk .~tmp~/", "--exclude", ".~tmp~/",
                "--safe-links"]
                .iter().map(|i|i.to_string()).collect::<Vec<_>>()),
            stage2_options: Arc::from(["-aHvh", "--no-o", "--no-g", "--stats",
                "--filter", "risk .~tmp~/", "--exclude", ".~tmp~/",
                "--delete", "--delete-after", "--delay-updates",
                "--safe-links"]
                .iter().map(|i|i.to_string()).collect::<Vec<_>>()),
            ..TwoStageRsyncProvider::default()
        };
        let base_provider_lock = provider.base_provider.write().await;
        if let Some(ctx) = base_provider_lock.ctx.lock().await.as_mut(){
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
        Ok(provider)
    }

    fn options(&self, stage: u8) -> Result<Vec<String>>{
        let mut options = Vec::new();
        match stage {
            1 => {
                options.extend(self.stage1_options.as_ref().clone());
                match rsync_stage1_profiles.get(&*self.two_stage_rsync_config.stage1_profile){
                    Some(profiles) => {
                        for exc in profiles{
                            options.push(exc.to_string());
                        }
                    }
                    None => {
                        return Err(anyhow!("stage 1配置文件无效"));
                    }
                }
            }
            2 => {
                options.extend(self.stage2_options.as_ref().clone());
                if self.two_stage_rsync_config.extra_options.len() > 0{
                    options.extend(self.two_stage_rsync_config.extra_options.clone());
                }
            }
            _ => {
                return Err(anyhow!(format!("不合法的stage：{}", stage)));
            }
        }

        if !self.two_stage_rsync_config.rsync_never_timeout{
            let timeo = match self.two_stage_rsync_config.rsync_timeout_value {
                value if value > 0 => value,
                _ => 120,
            };
            options.push(format!("--timeout={}", timeo))
        }

        if self.two_stage_rsync_config.use_ipv6{
            options.push("-6".to_string());
        }else if self.two_stage_rsync_config.use_ipv4 {
            options.push("-4".to_string());
        }

        if !self.two_stage_rsync_config.exclude_file.is_empty(){
            options.push("--exclude-from".to_string());
            options.push(self.two_stage_rsync_config.exclude_file.clone());
        }
        Ok(options)
    }

    /// cmd配置base_provider字段中的cmd字段
    async fn cmd(&self, cmd_and_args: Vec<String>, working_dir: String, env: HashMap<String, String>){
        let mut base_provider_lock = self.base_provider.write().await;
        let mut cmd_job: CmdJob;
        let mut args: Vec<String> = Vec::new();
        let use_docker = base_provider_lock.docker_ref().is_some();

        if let Some(d) = base_provider_lock.docker_ref().as_ref(){
            let c = "docker";
            args.extend(vec!["run".to_string(), "--rm".to_string(),
                             "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                             "--name".to_string(), d.name(base_provider_lock.name().parse().unwrap()),
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
                args.extend(vec!["-m".to_string(), format!("{}b", d.memory_limit.value())])
            }
            // 添加选项
            args.extend(d.options.iter().cloned());
            // 添加镜像和command
            args.push(d.image.clone());
            // 添加command
            args.extend(cmd_and_args.iter().cloned());

            cmd_job = CmdJob::new(Command::new(c), working_dir.clone(), env.clone());
            { cmd_job.cmd.lock().await.args(&args); }

        }else {
            if cmd_and_args.len() == 1{
                // cmd修改与rsync_provider相同
                cmd_job = CmdJob::new(Command::new(&cmd_and_args[0]), working_dir.clone(), env.clone());
                // { cmd_job.cmd.lock().await.arg(&cmd_and_args[0]); }

            }else if cmd_and_args.len() > 1 {
                // cmd修改与rsync_provider相同
                let c = cmd_and_args[0].clone();
                let args = cmd_and_args[1..].to_vec();
                cmd_job = CmdJob::new(Command::new(c), working_dir.clone(), env.clone());
                { 
                    cmd_job.cmd.lock().await
                        // .arg(c)
                        .args(&args); 
                }

            }else {
                panic!("命令的长度最少是1！")
            }
        }

        if !use_docker {
            debug!("在 {} 位置执行 {} 命令", &working_dir, cmd_and_args[0]);

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
                    .lock().await
                    .current_dir(&working_dir)
                    .envs(crate::runner::new_environ(env, true));
            }
        }

        base_provider_lock.cmd = Some(cmd_job);
        drop(base_provider_lock);
    }

}

#[async_trait]
impl MirrorProvider for TwoStageRsyncProvider {
    async fn name(&self) -> String {
        self.base_provider.read().await.name()
    }

    fn upstream(&self) -> String {
        self.two_stage_rsync_config.upstream_url.clone()
    }

    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::TwoStageRsync
    }

    async fn run(&mut self, started: Sender<Empty>) -> Result<()> {
        if self.is_running().await {
            return Err(anyhow!("provider现在正在运行"))
        }
        let base_provider_lock = self.base_provider.write().await;
        self.data_size = Arc::from(String::new());
        drop(base_provider_lock);
        
        let stages = vec![1,2];
        for stage in stages{
            let started = started.clone();
            
            let mut command = vec![self.two_stage_rsync_config.rsync_cmd.clone()];
            match self.options(stage){
                Err(e) => return Err(e),
                Ok(options) => {
                    command.extend(options);
                    command.push(self.two_stage_rsync_config.upstream_url.clone());
                    command.push(self.working_dir().await);
                }
            }
            self.cmd(command, self.working_dir().await, self.two_stage_rsync_config.rsync_env.clone()).await;
            
            started.send(()).await.expect("无法发送");
            
            let mut base_provider_lock = self.base_provider.write().await;
            base_provider_lock.prepare_log_file(stage>1).await?;
            base_provider_lock.start().await?;
            
            base_provider_lock.is_running.store(true, Ordering::Release);
            
            println!("debug: 将is_running字段设置为true :{}", base_provider_lock.name());
            debug!("将is_running字段设置为true :{}", base_provider_lock.name());

            // { started.send(()).await.expect("无法发送"); }
        
            drop(base_provider_lock);
            let base_provider_lock = self.base_provider.read().await;
            let result = base_provider_lock.wait().await;
            match result {
                Err(err) =>{
                    return Err(anyhow!(err));
                }
                Ok(exit_code) if exit_code != 0 => {
                    if let Some(msg) = internal::util::translate_rsync_error_code(exit_code){
                        debug!("rsync 异常终止： {} ({})", exit_code, msg);
                        drop(base_provider_lock);
                        if let Some(log_file_fd) = self.base_provider.write().await.log_file_fd.as_mut(){
                            log_file_fd.write(format!("{}\n",msg).as_bytes())?;
                        }
                        return Err(anyhow!(msg));
                    }
                }
                _ => {
                    drop(base_provider_lock);
                }
            }
        }
        
        if let Some(size) = extract_size_from_rsync_log(&*self.log_file().await) {
            self.data_size = size.into()
        };
        Ok(())
    }

    async fn terminate(&self) -> Result<()> {
        println!("debug: 进入terminate！");
        self.base_provider.read().await.terminate().await
    }

    async fn is_running(&self) -> bool {
        self.base_provider.read().await.is_running()
    }


    async fn docker(&self) -> Arc<Option<DockerHook>> {
        self.base_provider.read().await.docker_ref()
    }

    async fn add_hook(&mut self, hook: HookType) {
        self.base_provider.write().await.add_hook(hook).await;
    }

    async fn hooks(&self) -> Arc<Mutex<Vec<Box<dyn JobHook>>>> {
        self.base_provider.read().await
            .hooks()
    }

    async fn interval(&self) -> Duration {
        self.base_provider.read().await.interval()
    }

    async fn retry(&self) -> i64 {
        self.base_provider.read().await.retry
    }

    async fn timeout(&self) -> Duration {
        self.base_provider.read().await.timeout()
    }

    async fn working_dir(&self) -> String {
        self.base_provider.read().await.working_dir().await
    }

    async fn log_dir(&self) -> String {
        self.base_provider.read().await.log_dir().await
    }

    async fn log_file(&self) -> String {
        self.base_provider.read().await.log_file().await
    }

    fn data_size(&self) -> String {
        self.data_size.to_string()
    }

    async fn enter_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().await
            .enter_context().await
    }

    async fn exit_context(&mut self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().await
            .exit_context().await
    }

    async fn context(&self) -> Arc<Mutex<Option<Context>>> {
        self.base_provider.write().await
            .context()
    }
}

