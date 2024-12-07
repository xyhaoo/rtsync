use std::error::Error;
use std::{fs, io};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use chrono::Utc;
use log::debug;
use crate::hooks::{EmptyHook, JobHook};
use crate::provider::{MirrorProvider, _LOG_FILE_KEY};

pub(crate) struct LogLimiter<T: Clone>{
    empty_hook: EmptyHook<T>,
}

fn new_log_limiter<T: Clone>(provider: Box<dyn MirrorProvider<ContextStoreVal=T>>) -> LogLimiter<T>{
    LogLimiter{
        empty_hook: EmptyHook{
            provider
        }
    }
}

impl JobHook for LogLimiter<String>{
    fn pre_exec(&self) -> Result<(), Box<dyn Error>> {
        debug!("为 {} 执行日志限制器", self.empty_hook.provider.name());
        
        let p = self.empty_hook.provider.as_ref();
        if p.log_file().eq("/dev/null"){
            return Ok(());
        }
        let log_dir = p.log_dir();
        
        // 找到log_dir下的log文件，只保留最新的10个
        let path = Path::new(&log_dir);
        match fs::read_dir(path){
            Err(err) => {
                // 如果目录不存在，则创建目录
                if err.kind() == io::ErrorKind::NotFound {
                    fs::create_dir_all(path)?;
                    fs::metadata(&path)
                        .expect("failed to get metadata").permissions()
                        .set_mode(0o755); // 设置权限
                } else {
                    return Err(err.into());
                }
            }
            Ok(entries) => {
                let mut matched_files = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    let file_name = entry.file_name();
                    // 使用to_string_lossy()处理合法字符
                    if file_name.to_string_lossy().starts_with(p.name().as_str()) {
                        matched_files.push(entry);
                    }
                }
                // 按文件修改时间排序，最新的文件排在前面
                matched_files.sort_by(|a, b| 
                    b.metadata().unwrap().modified().unwrap()
                        .cmp(&a.metadata().unwrap().modified().unwrap()));

                // 保留最新的 10 个文件，删除其余的
                if matched_files.len() > 9 {
                    for file in &matched_files[9..] {
                        let file_path = file.path();
                        debug!("删除旧文件: {:?}", file_path);
                        fs::remove_file(file_path)?; // 删除文件
                    }
                }

            }
        }
        let log_file_name = format!("{}_{}.log", p.name(), Utc::now().format("%Y-%m-%d_%H_%M"));
        let log_file_path = Path::new(&log_dir).join(&log_file_name);
        let log_link = Path::new(&log_dir).join("latest");
        // 如果符号链接存在，删除它
        if log_link.exists() {
            fs::remove_file(&log_link)?;
        }
        // 创建新的符号链接
        symlink(&log_file_name, &log_link)?;
        
        let ctx = p.enter_context();
        ctx.set(_LOG_FILE_KEY.to_string(), log_file_path.display().to_string());
        Ok(())
    }
    fn post_exec(&self) -> Result<(), Box<dyn Error>> {
        self.empty_hook.provider.exit_context()?;
        Ok(())
    }
    fn post_fail(&self) -> Result<(), Box<dyn Error>> {
        let log_file = self.empty_hook.provider.log_file();
        let log_file_fail = format!("{log_file}.fail");
        let log_dir = self.empty_hook.provider.log_dir();
        let log_link = Path::new(&log_dir).join("latest");
        fs::rename(&log_file, &log_file_fail)?;
        fs::remove_file(&log_link)?;
        let log_file_name = Path::new(&log_file_fail)
            .file_name().unwrap()
            .to_string_lossy().to_string();
        symlink(&log_file_name, log_link)?;
        
        self.empty_hook.provider.exit_context()?;
        Ok(())
    }
    
}

