use std::collections::HashMap;
use crate::hooks::JobHook;
use anyhow::{anyhow, Result};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use async_trait::async_trait;
use shlex::Shlex;
use crate::context::Context;
// hook在同步后执行命令
// 通常设置时间戳等

#[derive(Debug, Clone)]
pub(crate) enum ExecOn{
    Success,
    Failure,
}
impl ExecOn {
    pub(crate) fn from_u8(exec_on: u8) -> Self {
        match exec_on {
            0 => Self::Success,
            1 => Self::Failure,
            _ => unreachable!("exec_on的无效选项"),
        }
    }
    pub(crate) fn as_u8(&self) -> u8 {
        match self {
            ExecOn::Success => 0,
            ExecOn::Failure => 1,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct ExecPostHook {
    exec_on: ExecOn,
    command: Vec<String>,
}

impl ExecPostHook {
    pub(crate) fn new(exec_on: ExecOn, command: &str) -> Result<ExecPostHook>
    {
        let cmd: Vec<String> = Shlex::new(command).collect();
        // if cmd.len() == 0 {
        //     return Err("未检测到命令".into())
        // }
        Ok(ExecPostHook{
            exec_on,
            command: cmd,
        })
    }

    fn r#do(&self,
            provider_name: String,
            working_dir: String,
            upstream: String,
            log_dir: String,
            log_file: String)
            -> Result<()>
    {
        let exit_status = match self.exec_on {
            ExecOn::Success => "success",
            ExecOn::Failure => "failure",
        };
        let env: HashMap<&str, String> = [
            ("RTSYNC_MIRROR_NAME", provider_name),
            ("RTSYNC_WORKING_DIR", working_dir),
            ("RTSYNC_UPSTREAM_URL", upstream),
            ("RTSYNC_LOG_DIR", log_dir),
            ("RTSYNC_LOG_FILE", log_file),
            ("RTSYNC_JOB_EXIT_STATUS", exit_status.to_string())]
            .iter().cloned().collect();
        let mut args = Vec::new();
        let cmd = match self.command.len() {
            1 => {
                self.command[0].as_str()
            },
            num if num > 1 => {
                args.extend_from_slice(&self.command[1..]);
                self.command[0].as_str()
            },
            _ => {
                return Err(anyhow!("不合法的命令"))
            },
        };
        // FIXME：改动与rsync_provider相同
        let mut command = Command::new(cmd);
        // command.arg(cmd);
        for (key, value) in env {
            command.env(key, value);
        }
        // println!("debug cmd: {:?}\n参数 {:?}", command.get_program(), command.get_args());
        command.args(args).status()?;

        Ok(())
    }
}

#[async_trait]
impl JobHook for ExecPostHook{

    async fn post_success(&self,
                    _context: Arc<Mutex<Option<Context>>>,
                    provider_name: String,
                    working_dir: String,
                    upstream: String,
                    log_dir: String,
                    log_file: String) 
        -> Result<()> 
    {
        if let ExecOn::Success = self.exec_on{
            return self.r#do(provider_name, working_dir, upstream, log_dir, log_file);
        }
        Ok(())
    }
    
    async fn post_fail(&self,
                 provider_name: String,
                 working_dir: String,
                 upstream: String,
                 log_dir: String,
                 log_file: String, 
                 _context: Arc<Mutex<Option<Context>>>) 
        -> Result<()> 
    {
        if let ExecOn::Failure = self.exec_on{
            return self.r#do(provider_name, working_dir, upstream, log_dir, log_file)
        }
        Ok(())
    }
}



