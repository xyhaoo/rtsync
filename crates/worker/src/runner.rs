use crate::common::Empty;
use std::collections::HashMap;
use std::fs::{File, Permissions};
use std::{env, fs, io, process, thread};
use std::os::unix::process::ExitStatusExt;
use anyhow::{anyhow, Error, Result};
use tokio::process::{Child, Command};
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::mpsc::{Sender, Receiver};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::Mutex;

// runner运行操作系统命令，提供命令行，env和log file
// 它是python-sh或go-sh的替代品
pub(crate) fn err_process_not_started() -> Error {
    anyhow!("进程未启动")
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
    pub(crate) finished_tx: Mutex<Option<Sender<()>>>,
    pub(crate) finished_rx: Mutex<Option<Receiver<()>>>,
    pub(crate) ret_err: Mutex<String>,
}

impl CmdJob {
    pub(crate) fn new(cmd: Command, working_dir: String, env: HashMap<String, String>) -> Self {
        CmdJob{
            cmd: Mutex::new(cmd),
            result: Mutex::new(None),
            pid: None,
            working_dir,
            env,
            log_file: None,
            finished_tx: Mutex::new(None),
            finished_rx: Mutex::new(None),
            ret_err: Mutex::from(String::new()),
        }
    }
    
    pub(crate) async fn set_log_file(&self, log_file: Option<File>){
        let mut cmd_lock = self.cmd.lock().await;
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

    pub(crate) async fn wait(&self) -> Result<i32>{
        let mut ret_err_lock = self.ret_err.lock().await;
        let mut finished_rx_lock = self.finished_rx.lock().await;
        if let Err(e) = finished_rx_lock.as_mut().unwrap().try_recv(){
            drop(finished_rx_lock);
            match e{
                // 发送端被drop之后，也就是命令已经停止
                TryRecvError::Disconnected => {
                    if !ret_err_lock.is_empty(){ return Err(anyhow!(ret_err_lock.clone())); }
                },
                // 命令正在运行。这个分支只能进入一次，分支内将drop发送端
                TryRecvError::Empty => {
                    if let Some(child) = self.result.lock().await.as_mut() {
                        let result = child.wait().await;
                        drop(self.finished_tx.lock().await.take().unwrap());
                        match result {
                            Err(err) => {
                                *ret_err_lock = err.to_string();
                                return Err(anyhow!(format!("命令异常终止: {}", err.to_string())))
                            }
                            // 命令正确，运行时是否出错都会进入这个分支，通过返回的状态码来判断是否正常退出
                            Ok(exit_status) => {
                                match exit_status.code() {
                                    // 正常退出
                                    Some(0) => {},
                                    // 被中断
                                    None => {
                                        *ret_err_lock = format!("退出状态: {:?}", exit_status.signal());
                                        return Err(anyhow!("命令被信号中断: {}", exit_status))
                                    }
                                    // 非正常退出
                                    _ => {
                                        *ret_err_lock = format!("退出状态: {}", exit_status.to_string());
                                        return Ok(exit_status.code().expect("无法获取状态码"))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        drop(ret_err_lock);
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

    // println!("现在的环境变量：");
    // 添加传入的 env 变量
    for (key, value) in env.iter() {
        // println!("{}={}", key, value);
        environ.insert(key.clone(), value.clone());
    }
    environ
}

