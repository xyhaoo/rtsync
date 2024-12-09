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
use std::sync::Mutex;
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

// mutex
pub(crate) struct CmdJob
{
    pub(crate) cmd: Command,
    pub(crate) result: Option<Child>,
    pub(crate) working_dir: String,
    pub(crate) env: HashMap<String, String>,
    pub(crate) log_file: Option<File>,
    pub(crate) finished: Mutex<Receiver<Empty>>,
    pub(crate) ret_err: Option<String>,
}
// 把逻辑弄到MirrorProvider的实现类里
impl CmdJob {
    // pub(crate) fn new(cmd_and_args: Vec<String>,
    //                   working_dir: &str,
    //                   env: &HashMap<String, String>) -> Self
    // {
    //     let mut cmd_job: CmdJob;
    //     let mut args: Vec<String> = Vec::new();
    //     let use_docker = provider.docker().is_some();
    //     if let Some(d) = provider.docker(){
    //         let c = "docker";
    //         args.extend(vec!["run".to_string(), "--rm".to_string(),
    //                          "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
    //                          "--name".to_string(), d.name(),
    //                          "-w".to_string(), working_dir.to_string()]);
    //         // 指定用户
    //         unsafe {
    //             args.extend(vec!["-u".to_string(),
    //                              format!("{}:{}",getuid().to_string(), getgid().to_string())]);
    //         }
    //         // 添加卷
    //         for vol in d.volumes.iter(){
    //             debug!("数据卷: {}", &vol);
    //             args.extend(vec!["-v".to_string(), vol.clone()])
    //         }
    //         // 设置环境变量
    //         for (key, value) in env.iter(){
    //             let kv = format!("{}={}", key, value);
    //             args.extend(vec!["-e".to_string(), kv.clone()])
    //         }
    //         // 设置内存限制
    //         if d.memory_limit.0 != 0{
    //             args.extend(vec!["-m".to_string(), format!("{}", d.memory_limit.value())])
    //         }
    //         // 添加选项
    //         args.extend(d.options.iter().cloned());
    //         // 添加镜像和command
    //         args.push(d.image.clone());
    //         // 添加command
    //         args.extend(cmd_and_args.iter().cloned());
    // 
    // 
    //         cmd_job = CmdJob{
    //             cmd: Command::new(c),
    //             result: None,
    //             working_dir: working_dir.to_string(),
    //             env: env.clone(),
    //             log_file: None,
    //             finished: Mutex::new(mpsc::channel().1),
    //             ret_err: None,
    //         };
    //         cmd_job.cmd.args(&args);
    // 
    //     // }else if provider.c_group().is_some() {
    //     //     let mut tmp_cmd = process::Command::new("rtsync-exec");
    //     //     tmp_cmd.args(cmd_and_args);
    //     //     cmd = Some(tmp_cmd);
    //     }else {
    //         if cmd_and_args.len() == 1{
    //             cmd_job = CmdJob{
    //                 cmd: Command::new(&cmd_and_args[0]),
    //                 result: None,
    //                 working_dir: working_dir.to_string(),
    //                 env: env.clone(),
    //                 log_file: None,
    //                 finished: Mutex::new(mpsc::channel().1),
    //                 provider,
    //                 ret_err: None,
    //             };
    //         }else if cmd_and_args.len() > 1 {
    //             let c = cmd_and_args[0].clone();
    //             let args = cmd_and_args[1..].to_vec();
    //             cmd_job = CmdJob{
    //                 cmd: Command::new(c),
    //                 result: None,
    //                 working_dir: working_dir.to_string(),
    //                 env: env.clone(),
    //                 log_file: None,
    //                 finished: Mutex::new(mpsc::channel().1),
    //                 provider,
    //                 ret_err: None,
    //             };
    //             cmd_job.cmd.args(&args);
    //         }else {
    //             panic!("命令的长度最少是1！")
    //         }
    //     }
    // 
    //     if !use_docker {
    //         debug!("在 {} 位置执行 {} 命令", cmd_and_args[0], working_dir);
    // 
    //         // 如果目录不存在，则创建目录
    //         if let Err(err) = fs::read_dir(&working_dir) {
    //             if err.kind() == io::ErrorKind::NotFound {
    //                 debug!("创建文件夹：{}", working_dir);
    //                 if fs::create_dir_all(working_dir).is_ok(){
    //                     if let Err(e) = fs::set_permissions(working_dir, Permissions::from_mode(0o755)) {
    //                         error!("更改文件夹 {} 权限失败: {}",working_dir, err)
    //                     }
    //                 }else {
    //                     error!("创建文件夹 {} 失败: {}",working_dir, err)
    //                 }
    //             }
    //         }
    //         cmd_job.cmd.current_dir(&working_dir);
    //         cmd_job.cmd.envs(new_environ(&env, true));
    //     }
    //     cmd_job
    // 
    // }

    // 非阻塞 spawn
    // pub(crate) fn start(&self) -> Result<(), Box<dyn Error>>{
    //     let cg = self.provider.c_group();
    //     unimplemented!();
    //     Ok(())
    // }


    pub(crate) fn wait(&mut self) -> Result<i32, Box<dyn Error>>{
        let finished = self.finished.lock().unwrap();
        if finished.recv().is_ok(){
            return if let Some(err) = self.ret_err.as_ref() {
                Err(err.clone().into())
            } else {
                Ok(0)
            }
        }else {
            if let Some(child) = self.result.as_mut(){
                let exit_status = child.wait().unwrap();
                match child.wait(){
                    Err(err) => {
                        return Err(format!("命令异常终止: {}", err.to_string()).into());
                    },
                    Ok(exit_status) => {
                        return Ok(exit_status.code().expect("无法获取状态码"))
                    }
                }
                
            }
        }
        Ok(0)
    }

    pub(crate) fn set_log_file(&mut self, log_file: Option<File>){
        match log_file {
            Some(log_file) => {
                self.cmd.stdout(Stdio::from(log_file.try_clone().expect("复制文件句柄时失败")));
                self.cmd.stderr(Stdio::from(log_file));
            }
            None => {
                // 重定向到一个“空的”输出流，相当于丢弃输出。这样，命令的输出不会显示在终端中。
                self.cmd.stdout(Stdio::null());
                self.cmd.stderr(Stdio::null());
            }
        }
    }

    // pub(crate) fn terminate(&mut self) -> Result<(), Box<dyn Error>>{
    //     if self.cmd.get_program().eq("") || self.result.is_none(){
    //         return Err(err_process_not_started())
    //     }
    //     if let Some(d) = self.provider.docker(){
    //         Command::new("docker")
    //             .arg("stop")
    //             .arg("-t")
    //             .arg("2")
    //             .arg(d.name())
    //             .output()?;
    //         return Ok(())
    //     }
    //     if let Some(child) = self.result.as_mut(){
    //         // 发送 SIGTERM 信号
    //         let pid = Pid::from_raw(child.id() as i32); // 获取子进程的 PID
    //         if let Err(e) = kill(pid, Signal::SIGTERM){
    //             return Err(e.into())
    //         }
    //         // 等待 2 秒
    //         thread::sleep(Duration::from_secs(2));
    // 
    //         // 如果子进程仍在运行，则发送 SIGKILL 信号强制退出
    //         if let None = child.try_wait().ok().flatten() {
    //             kill(pid, Signal::SIGKILL).expect("发送SIGKILL信号失败");
    //             warn!("对进程发送了SIGTERM信号，但是其未在两秒内退出，所以发送了SIGKILL信号");
    //         } 
    //     }
    // 
    //     Ok(())
    // }
}

pub(crate) fn new_environ(env: &HashMap<String, String>, inherit: bool) -> HashMap<String, String> {
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

    // 添加传入的 env 变量
    for (key, value) in env.iter() {
        environ.insert(key.clone(), value.clone());
    }
    environ
}

/*
fn new_cmd_job<T: Clone>(provider: Box<dyn MirrorProvider<ContextStoreVal=T>>,
                         cmd_and_args: Vec<String>,
                         working_dir: String,
                         env: HashMap<String, String>) -> Mutex<CmdJob<T>>
{
    let mut args: Vec<String> = Vec::new();

    let mut cmd: &mut process::Command;
    if let Some(d) = provider.docker(){
        let c = "docker";
        args.extend(vec!["run".to_string(), "--rm".to_string(),
                         "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                         "--name".to_string(), d.name(),
                         "-w".to_string(), working_dir]);
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
            args.extend(vec!["-m".to_string(), format!("{}", d.memory_limit.value())])
        }
        // 添加选项
        args.extend(d.options.iter().cloned());
        // 添加镜像和command
        args.push(d.image.clone());
        // 添加command
        args.extend(cmd_and_args.iter().cloned());

        cmd = process::Command::new(c)
            .args(&args);

    // }else if provider.c_group().is_some() {
    //     let mut tmp_cmd = process::Command::new("rtsync-exec");
    //     tmp_cmd.args(cmd_and_args);
    //     cmd = Some(tmp_cmd);
    }else {
        if cmd_and_args.len() == 1{
            cmd = &mut process::Command::new(&cmd_and_args[0]);
        }else if cmd_and_args.len() > 1 {
            let c = cmd_and_args[0].clone();
            let args = cmd_and_args[1..].to_vec();

            cmd = process::Command::new(c)
                .args(&args);
        }else {
            panic!("命令的长度最少是1！")
        }
    }

    if provider.docker().is_none() {
        debug!("在 {} 位置执行 {} 命令", cmd_and_args[0], &working_dir);

        // 如果目录不存在，则创建目录
        if let Err(err) = fs::read_dir(&working_dir) {
            if err.kind() == io::ErrorKind::NotFound {
                debug!("创建文件夹：{}", &working_dir);
                if fs::create_dir_all(&working_dir).is_ok(){
                    if let Err(e) = fs::set_permissions(&working_dir, Permissions::from_mode(0o755)) {
                        error!("更改文件夹 {} 权限失败: {}",&working_dir, err)
                    }
                }else {
                    error!("创建文件夹 {} 失败: {}",&working_dir, err)
                }
            }
        }
        cmd = cmd.current_dir(&working_dir)
            .envs(env.clone());
    }
    Mutex::new(
        CmdJob{
            // cmd: Some(cmd.deref().clone()),
            working_dir: working_dir.to_string(),
            env,
            log_file: None,
            finished: mpsc::channel::<()>().0,
            provider,
            ret_err: None,
        }
    )
}
 */
