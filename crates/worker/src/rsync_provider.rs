use std::collections::HashMap;
use std::error::Error;
use std::io::Write;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use chrono::Duration;
use log::debug;
use internal;
use internal::util::extract_size_from_rsync_log;
use crate::base_provider::BaseProvider;
use crate::cgroup::CGroupHook;
use crate::common;
use crate::common::{Empty, DEFAULT_MAX_RETRY};
use crate::config::ProviderEnum;
use crate::context::Context;
use crate::docker::DockerHook;
use crate::hooks::JobHook;
use crate::provider::{MirrorProvider, _LOG_DIR_KEY, _LOG_FILE_KEY, _WORKING_DIR_KEY};
use crate::runner::CmdJob;
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
struct RsyncProvider<T: Clone> {
    base_provider: BaseProvider<T>,
    rsync_config: RsyncConfig,
    options: Vec<String>,
    data_size: String,
}

impl RsyncProvider<String> {
    fn new(mut c: RsyncConfig) -> Result<Self, String> {
        // TODO: 检查config选项
        if !c.upstream_url.ends_with("/"){
            return Err("rsync上游URL应该以'/'结尾".to_string());
        }
        if c.retry == 0{
            c.retry = DEFAULT_MAX_RETRY;
        }

        let mut provider = RsyncProvider{
            base_provider: BaseProvider{
                name: c.name.clone(),
                ctx: Some(Context::new()),
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
        if let Some(ctx) = provider.base_provider.ctx.as_mut(){
            ctx.set(_WORKING_DIR_KEY.to_string(), c.working_dir);
            ctx.set(_LOG_DIR_KEY.to_string(), c.log_dir);
            ctx.set(_LOG_FILE_KEY.to_string(), c.log_file);
        }

        Ok(provider)
    }
}

impl<T: Clone> MirrorProvider for RsyncProvider<T>  {
    type ContextStoreVal = ();
    
    fn name(&self) -> String {
        todo!()
    }
    fn upstream(&self) -> String {
        self.rsync_config.upstream_url.clone()
    }
    fn r#type(&self) -> ProviderEnum {
        ProviderEnum::Rsync
    }
    // fn start(&mut self) -> Result<(), Box<dyn Error>> {
    //     if self.is_running(){
    //         return Err("provider现在正在运行".into())
    //     }
    //     let mut command = Vec::new();
    //     command.push(self.rsync_config.rsync_cmd.clone());
    //     command.extend(self.options.clone());
    //     command.push(self.rsync_config.upstream_url.clone());
    //     command.push(self.working_dir());
    //     self.base_provider.cmd = 
    //         Some(Arc::new(RwLock::new(
    //             CmdJob::new(Box::<T>::new(self.into()), 
    //                         command,
    //                         &*self.working_dir(),
    //                         &self.rsync_config.rsync_env))));
    // 
    //     Ok(())
    // }

    fn run(&mut self, started: Sender<Empty>) -> Result<(), Box<dyn Error>> {
        self.data_size = "".to_string();
        if let Err(e) = self.start(){
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
        if let Some(size) = extract_size_from_rsync_log(&*self.log_file()) {
            self.data_size = size
        };
        Ok(())
    }

    fn start(&mut self) -> Result<(), Box<dyn Error>> {
        todo!()
    }

    fn wait(&self) -> Result<(), Box<dyn Error>> {
        todo!()
    }

    fn terminate(&self) -> Result<(), Box<dyn Error>> {
        todo!()
    }

    fn is_running(&self) -> bool {
        todo!()
    }

    fn c_group(&self) -> Option<&CGroupHook<Self::ContextStoreVal>> {
        todo!()
    }

    fn zfs(&self) -> Option<ZfsHook<Self::ContextStoreVal>> {
        todo!()
    }

    fn docker(&self) -> Option<&DockerHook<Self::ContextStoreVal>> {
        todo!()
    }

    fn add_hook(&self, hook: Box<dyn JobHook>) {
        todo!()
    }

    fn hooks(&self) -> Vec<Box<dyn JobHook>> {
        todo!()
    }

    fn interval(&self) -> Duration {
        todo!()
    }

    fn retry(&self) -> u64 {
        todo!()
    }

    fn timeout(&self) -> Duration {
        todo!()
    }

    fn working_dir(&self) -> String {
        todo!()
    }

    fn log_dir(&self) -> String {
        todo!()
    }

    fn log_file(&self) -> String {
        todo!()
    }

    fn is_master(&self) -> bool {
        todo!()
    }

    fn data_size(&self) -> String {
        self.data_size.clone()
    }

    fn enter_context(&self) -> &mut Context<Self::ContextStoreVal> {
        todo!()
    }

    fn exit_context(&self) -> Result<Context<Self::ContextStoreVal>, Box<dyn Error>> {
        todo!()
    }

    fn context(&self) -> Context<Self::ContextStoreVal> {
        todo!()
    }
}
