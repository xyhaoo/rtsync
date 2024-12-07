use crate::provider::_LOG_FILE_KEY;
use std::cell::RefCell;
use std::{fs, io, thread, time::Duration};
use std::error::Error;
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::rc::Rc;
use log::{debug, error, warn};
use crate::config::{DockerConfig, MemBytes, MirrorConfig};

use crate::context::Context;
use crate::hooks::{EmptyHook, JobHook};
use crate::provider::MirrorProvider;

pub(crate) struct DockerHook<T: Clone> {
    pub(crate) empty_hook: EmptyHook<T>,
    pub(crate) image: String,
    pub(crate) volumes: Vec<String>,
    pub(crate) options: Vec<String>,
    pub(crate) memory_limit: MemBytes,
}

fn new_docker_hook<T: Clone>(
    p: Box<dyn MirrorProvider<ContextStoreVal=T>>, 
    g_cfg: DockerConfig, m_cfg: MirrorConfig) 
    -> DockerHook<T>
{
    let mut volumes: Vec<String> = vec![];
    if let Some(v) = &g_cfg.volumes{
        volumes.extend(v.iter().cloned());
    }
    if let Some(v) = &m_cfg.docker_volumes{
        volumes.extend(v.iter().cloned());
    }
    match &m_cfg.exclude_file {
        Some(file) if file.len()>0 => {
            let arg = format!("{}:{}:ro", file, file);
            volumes.push(arg);
        },
        _ => {},
    }
    let mut options: Vec<String> = vec![];
    if let Some(opts) = &g_cfg.options{
        options.extend(opts.iter().cloned());
    }
    if let Some(opts) = &m_cfg.docker_options{
        options.extend(opts.iter().cloned());
    }

    DockerHook{
        empty_hook: EmptyHook{
            provider: p,
        },
        image: m_cfg.docker_image.unwrap_or_default(),
        volumes,
        options,
        memory_limit: m_cfg.memory_limit.unwrap_or_default(),
    }
}

impl JobHook for DockerHook<Vec<String>>{
    fn pre_exec(&self) -> Result<(), Box<dyn Error>>{
        let p = self.empty_hook.provider.as_ref();
        let log_dir = p.log_dir();
        let log_file = p.log_file();
        let working_dir = p.working_dir();

        // 如果目录不存在，则创建目录
        if let Err(err) = fs::read_dir(&working_dir) {
            if err.kind() == io::ErrorKind::NotFound {
                debug!("创建文件夹：{}", &working_dir);
                fs::create_dir_all(&working_dir)?;
                if let Err(e) = fs::set_permissions(&working_dir, Permissions::from_mode(0o755)){
                    return Err(format!("创建文件夹 {} 失败: {}",&working_dir, e).into())
                }
            }
        }
        
        // 重写working_dir
        let ctx = p.enter_context();
        ctx.set(
            "volumes".to_string(), vec![
            format!("{}:{}", &log_dir, log_dir),
            format!("{}:{}", &log_file, &log_file),
            format!("{}:{}", &working_dir, &working_dir)]);

        Ok(())
    }

    fn post_exec(&self) -> Result<(), Box<dyn Error>>{
        // Command::new("docker")
        //  .args(["rm", "-f", self.name()])
        //  .status()
        
        let name = self.name();
        let mut retry = 10;
        loop{
            if retry == 0 { break }
            let output = Command::new("docker")
                .args(["ps", "-a",
                    "--filter", format!("name=^{}$", name).as_str(),
                    "--format", "{{.Status}}"])
                .output();
            match output {
                Ok(output) if !output.stdout.is_empty() => {
                    debug!("container {} 仍存在:{}",name, String::from_utf8_lossy(&output.stdout));
                }
                Ok(output) if output.stdout.is_empty() => {
                    break
                }
                Ok(output) if !output.stderr.is_empty() => {
                    error!("docker ps 命令失败: {}", String::from_utf8_lossy(&output.stderr));
                }
                _ => {}
            }
            thread::sleep(Duration::from_secs(1));  // 一秒
            retry -= 1;
        }
        if retry == 0{
            warn!("容器 {} 未自动删除，下一次同步可能失败", name)
        }
        self.empty_hook.provider.exit_context()?;
        Ok(())
    }
    
}
impl DockerHook<Vec<String>> {
    // volumes返回已配置的卷和运行时需要的卷
    // 包括mirror dirs和log file
    pub(crate) fn volumes(&self) -> Vec<String>{
        let mut vols = Vec::with_capacity(self.volumes.len());
        vols.extend(self.volumes.iter().cloned());

        let p = self.empty_hook.provider.as_ref();
        let ctx = p.context();
        if let Some(ivs) = ctx.get("volumes") {
            vols.extend(ivs.iter().cloned());
        }
        vols
    }
}

impl DockerHook<String> {
    pub(crate) fn log_file(&self) -> String{
        let p = self.empty_hook.provider.as_ref();
        let ctx = p.context();
        if let Some(value) = ctx.get(&format!("{_LOG_FILE_KEY}:docker")){
            return value
        }
        p.log_file()
    }
    
}
impl<T: Clone> DockerHook<T>{
    pub(crate) fn name(&self) -> String {
        let p = self.empty_hook.provider.as_ref();
        format!("rtsync-job-{}", p.name())
    }
}


