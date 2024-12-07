use std::collections::HashMap;
use crate::hooks::{EmptyHook, JobHook};
use crate::provider::MirrorProvider;
use std::error::Error;
use std::process::Command;
use shlex::Shlex;

// hook在同步后执行命令
// 通常设置时间戳等

pub(crate) enum ExecOn{
    Success,
    Failure,
}

pub(crate) struct ExecPostHook<T: Clone> {
    empty_hook: EmptyHook<T>,
    exec_on: ExecOn,
    command: Vec<String>,
}

fn new_exec_post_hook<T: Clone>(provider: Box<dyn MirrorProvider<ContextStoreVal=T>>, 
                                exec_on: ExecOn, command: &str) 
    -> Result<ExecPostHook<T>, Box<dyn Error>>
{
    let cmd: Vec<String> = Shlex::new(command).collect();
    if cmd.len() == 0 { 
        return Err("未检测到命令".into())
    }
    Ok(ExecPostHook{
        empty_hook: EmptyHook{
            provider,
        },
        exec_on,
        command: cmd,
    })
}
impl<T: Clone> JobHook for ExecPostHook<T>{
    fn post_success(&self) -> Result<(), Box<dyn Error>> {
        if let ExecOn::Success = self.exec_on{
            return self.r#do()
        }
        Ok(())
    }
    fn post_fail(&self) -> Result<(), Box<dyn Error>> {
        if let ExecOn::Failure = self.exec_on{
            return self.r#do()
        }
        Ok(())
    }
} 


impl<T: Clone> ExecPostHook<T> {
    fn r#do(&self) -> Result<(), Box<dyn Error>>{
        let p = self.empty_hook.provider.as_ref();
        let exit_status = match self.exec_on {
            ExecOn::Success => "success",
            ExecOn::Failure => "failure",
        };
        let env: HashMap<&str, String> = [
            ("RTSYNC_MIRROR_NAME", p.name()),
            ("RTSYNC_WORKING_DIR", p.working_dir()),
            ("RTSYNC_UPSTREAM_URL", p.upstream()),
            ("RTSYNC_LOG_DIR", p.log_dir()),
            ("RTSYNC_LOG_FILE", p.log_file()),
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
                return Err("不合法的命令".into())
            },
        };
        let mut cmd = Command::new(cmd);
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.args(args).status()?;

        Ok(())
    }
}