use std::cell::RefCell;
use std::{fs, thread, time::Duration};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::rc::Rc;
use log::{debug, error};
use crate::config::{DockerConfig, MemBytes, MirrorConfig};

use crate::context::Context;
use crate::hooks::EmptyHook;
use crate::provider::MirrorProvider;

pub(crate) struct DockerHook<T>
where T: MirrorProvider
{
    empty_hook: EmptyHook<T>,
    pub(crate) image: String,
    volumes: Vec<String>,
    pub(crate) options: Vec<String>,
    pub(crate) memory_limit: MemBytes,
}

fn new_docker_hook<T>(p: &T, g_cfg: &DockerConfig, m_cfg: &MirrorConfig) -> DockerHook<T>
where T: MirrorProvider
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
            provider: p.clone(),
        },
        image: m_cfg.docker_image.clone().unwrap_or_default(),
        volumes,
        options,
        memory_limit: m_cfg.memory_limit.clone().unwrap(),
    }
}

impl<T: MirrorProvider> DockerHook<T>{
    fn pre_exec(&self) -> Result<(), Box<dyn std::error::Error>>{
        let p = self.empty_hook.provider.clone();
        let log_dir = p.log_dir();
        let log_file = p.log_file();
        let working_dir = p.working_dir();


        fs::create_dir_all(&working_dir)?;
        debug!("创建了文件夹：{}", &working_dir);
        if let Err(e) = fs::set_permissions(&working_dir, Permissions::from_mode(0o755)){
            Err(e).expect(format!("创建文件夹:{} 失败",&working_dir).as_str())
        }

        // 重写working_dir
        let ctx = Rc::new(RefCell::new(p.enter_context()));
        Context::set(
            Rc::clone(&ctx),
            "volumes",
            vec![
                format!("{}:{}", &log_dir, log_dir),
                format!("{}:{}", &log_file, &log_file),
                format!("{}:{}", &working_dir, &working_dir)]);

        Ok(())
    }

    fn post_exec<U: Clone>(&self) -> Result<(), Box<dyn std::error::Error>>{
        // sh.Command(
        // 	"docker", "rm", "-f", d.Name(),
        // ).Run()
        let name = self.name();
        let mut retry = 10;
        loop{
            if retry == 0 { break }
            let output = Command::new("docker")
                .args(["ps", "-a",
                    "filter", format!("name=^{}$", name).as_str(),
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
        self.empty_hook.provider.exit_context::<U>();
        Ok(())
    }
    
    // volumes返回已配置的卷和运行时需要的卷
    // 包括mirror dirs和log file
    pub(crate) fn volumes<U: Clone + Into<Vec<String>>>(&self) -> Vec<String>{
        let mut vols = Vec::with_capacity(self.volumes.len());
        vols.extend(self.volumes.iter().cloned());
        
        let p = self.empty_hook.provider.clone();
        let ctx = Rc::new(RefCell::new(p.context::<U>()));
        if let Some(values) = Context::get(Rc::clone(&ctx), "volumes"){
            vols.extend(values.into())
        }
        vols
    }
    
    fn log_file<U: Clone + Into<String>>(&self) -> String{
        let p = self.empty_hook.provider.clone();
        let ctx = Rc::new(RefCell::new(p.context::<U>()));
        if let Some(value) = Context::get(Rc::clone(&ctx), ":docker"){
            return value.into()
        }
        p.log_file()
    }
    
    pub(crate) fn name(&self) -> String {
        format!("rtsync-job-{}", self.empty_hook.provider.name())
    }
    
}


