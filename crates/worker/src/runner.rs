use crate::common::Empty;
use std::collections::HashMap;
use std::fs::{File, Permissions};
use std::{env, fs, io, process, thread};
use std::error::Error;
use std::ops::Deref;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::raw::pid_t;
use std::process::{Child, Command, Stdio};
use crossbeam_channel::{Sender, Receiver};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Duration;
use crate::provider::MirrorProvider;
use libc::{getuid, getgid};
use log::{debug, error, warn};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
// runner运行操作系统命令，提供命令行，env和log file
// 它是python-sh或go-sh的替代品
pub(crate) fn err_process_not_started() -> Box<dyn Error> {
    "进程未启动".into()
}

// Mutex<CmdJob>

pub(crate) struct CmdJob
{
    pub(crate) cmd: Mutex<Command>,
    pub(crate) result: Mutex<Option<Child>>,
    pub(crate) pid: Option<i32>,
    pub(crate) working_dir: String,
    pub(crate) env: HashMap<String, String>,
    pub(crate) log_file: Option<File>,
    pub(crate) finished: Option<Receiver<Empty>>,
    pub(crate) ret_err: Mutex<String>,
}

impl CmdJob {

    
    pub(crate) fn new(cmd: Command, working_dir: String, env: HashMap<String, String>) -> Self {
        CmdJob{
            cmd: Mutex::new(cmd),
            result: Mutex::new(None),
            pid: None,
            working_dir: String::new(),
            env: HashMap::new(),
            log_file: None,
            finished: None,
            ret_err: Mutex::from(String::new()),
        }
    }
    
    pub(crate) fn set_log_file(&self, log_file: Option<File>){
        let mut cmd_lock = self.cmd.lock().unwrap();
        match log_file {
            Some(log_file) => {
                cmd_lock.stdout(Stdio::from(log_file.try_clone().expect("复制文件句柄时失败")));
                cmd_lock.stderr(Stdio::from(log_file));
            }
            None => {
                // 重定向到空的输出流，相当于丢弃输出。命令的输出不会显示在终端中。
                cmd_lock.stdout(Stdio::null());
                cmd_lock.stderr(Stdio::null());
            }
        }
        drop(cmd_lock);
    }

    pub(crate) fn wait(&self) -> Result<i32, Box<dyn Error>>{
        let mut ret_err_lock = self.ret_err.lock().unwrap();
        if let Some(finished) = self.finished.as_ref() {
            if finished.recv().is_ok(){
                return match &ret_err_lock {
                    err if ret_err_lock.is_empty() => Ok(0),
                    err => Err(ret_err_lock.clone().into()),
                }
                // return if let Some(err) = self.ret_err.as_ref() {
                //     Err(err.clone().into())
                // } else {
                //     Ok(0)
                // };
            }
        }
        
        if let Some(child) = self.result.lock().unwrap().as_mut() {
            return match child.wait() {
                Err(err) => {
                    *ret_err_lock = err.to_string();
                    Err(format!("命令异常终止: {}", err.to_string()).into())
                }
                Ok(exit_status) => {
                    println!("debug: exit status: {}", exit_status);
                    if exit_status.code() != Some(0) {
                        *ret_err_lock = format!("退出状态: {}", exit_status.to_string())
                    }
                    // 命令正确，运行时是否出错都会进入这个分支，通过返回的状态码来判断是否正常退出
                    println!("{}", format!("退出状态: {}", exit_status.to_string()));
                    Ok(exit_status.code().expect("无法获取状态码"))
                }
            };
        }

        Ok(0)
    }
}

pub(crate) fn new_environ(env: HashMap<String, String>, inherit: bool) -> HashMap<String, String> {
    let mut environ = HashMap::with_capacity(env.len());

    // 如果 inherit 为 true，继承当前进程的环境变量
    if inherit {
        // 获取当前进程的环境变量
        for (key, value) in env::vars() {
            // 如果当前环境变量已在传入的 env 中，跳过
            if !env.contains_key(&key) {
                environ.insert(key, value);
            }
        }
    }

    println!("现在的环境变量：");
    // 添加传入的 env 变量
    for (key, value) in env.iter() {
        println!("{}={}", key, value);
        environ.insert(key.clone(), value.clone());
    }
    environ
}

