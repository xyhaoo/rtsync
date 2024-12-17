use crate::provider::_LOG_FILE_KEY;
use std::cell::RefCell;
use std::{fs, io, thread, time::Duration};
use std::error::Error;
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use log::{debug, error, warn};
use crate::config::{DockerConfig, MemBytes, MirrorConfig};
use anymap::AnyMap;
use crate::context::Context;
use crate::hooks::{EmptyHook, JobHook};
use crate::provider::MirrorProvider;

#[derive(Debug, Clone)]
pub(crate) struct DockerHook {
    // pub(crate) empty_hook: EmptyHook<T>,
    pub(crate) image: String,
    pub(crate) volumes: Vec<String>,
    pub(crate) options: Vec<String>,
    pub(crate) memory_limit: MemBytes,
}

impl DockerHook{
    pub(crate) fn new(g_cfg: DockerConfig, m_cfg: MirrorConfig) -> Self
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
            image: m_cfg.docker_image.unwrap_or_default(),
            volumes,
            options,
            memory_limit: m_cfg.memory_limit.unwrap_or_default(),
        }
    }

    pub(crate) fn name(&self, provider_name: String) -> String {
        format!("rtsync-job-{}", provider_name)
    }
    

}

impl JobHook for DockerHook {
    fn pre_exec(&self,
                _provider_name: String,
                log_dir: String, 
                log_file: String, 
                working_dir: String,
                context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>>
    {
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
        let mut cur_ctx = context.lock().unwrap();
        
        *cur_ctx = match cur_ctx.take(){
            Some(ctx) => Some(ctx.enter()),
            None => None,
        };
        
        if let Some(ctx) = cur_ctx.as_mut(){
            let mut value = AnyMap::new();
            value.insert(vec![
                format!("{}:{}", &log_dir, log_dir),
                format!("{}:{}", &log_file, &log_file),
                format!("{}:{}", &working_dir, &working_dir)]);
            ctx.set("volumes".to_string(), value);
        }
        

        Ok(())
    }

    fn post_exec(&self, 
                 context: &Arc<Mutex<Option<Context>>>, 
                 provider_name: String) 
        -> Result<(), Box<dyn Error>>
    {
        // Command::new("docker")
        //     .args(["rm", "-f", &provider_name])
        //     .output()
        //     .unwrap();
        
        let name = self.name(provider_name);
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

        // 😅
        let mut cur_ctx = context.lock().unwrap();
        *cur_ctx = match cur_ctx.take(){
            Some(ctx) => {
                match ctx.exit() {
                    Ok(ctx) => Some(ctx),
                    Err(_) => None,
                }
            },
            None => None,
        };
        Ok(())
    }
    
}



/*
impl DockerHook<Vec<String>> {
    // volumes返回已配置的卷和运行时需要的卷
    // 包括mirror dirs和log file
    // pub(crate) fn volumes(&self) -> Vec<String>{
    //     let mut vols = Vec::with_capacity(self.volumes.len());
    //     vols.extend(self.volumes.iter().cloned());
    // 
    //     let p = self.empty_hook.provider.as_ref();
    //     if let Some(ctx) = p.context(){
    //         if let Some(ivs) = ctx.get("volumes") {
    //             vols.extend(ivs.iter().cloned());
    //         }
    //     }
    //     
    //     vols
    // }
}

impl DockerHook<String> {
    // pub(crate) fn log_file(&self) -> String{
    //     let p = self.empty_hook.provider.as_ref();
    //     if let Some(ctx) = p.context(){
    //         if let Some(value) = ctx.get(&format!("{_LOG_FILE_KEY}:docker")){
    //             return value
    //         }
    //     }
    //     p.log_file()
    // }
    
}
impl DockerHook{
    
    
}
 */


