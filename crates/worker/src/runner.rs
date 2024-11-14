use crate::common::Empty;
use std::collections::HashMap;
use std::fs::{File, Permissions};
use std::{fs, process};
use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;
use std::sync::Mutex;
use crate::provider::MirrorProvider;
use libc::{getuid, getgid};
use log::debug;
// runner运行操作系统命令，提供命令行，env和log file
// 它是python-sh或go-sh的替代品

fn err_process_not_started(){
    panic!("Process Not Started");
}

struct CmdJob<T, E>
where T: MirrorProvider, E: std::error::Error
{
    // mutex
    cmd: Option<process::Command>,
    working_dir: String,    //PathBuf?
    env: HashMap<String, String>,
    log_file: Option<File>,
    finished: mpsc::Sender<Empty>,
    provider: T,
    ret_err: Result<(), E>,
}

fn new_cmd_job<'a, T, E>(provider: &T,
                         cmd_and_args: &Vec<String>,
                         working_dir: &String,
                         env: &HashMap<String, String>) -> Mutex<CmdJob<T, E>>
where T: MirrorProvider, E: std::error::Error
{
    let mut args:Vec<String> = Vec::new();
    let mut cmd = None;
    if let Some(d) = provider.docker(){
        let c = "docker";
        args.extend(vec!["run".to_string(), "--rm".to_string(),
                         "-a".to_string(), "STDOUT".to_string(), "-a".to_string(), "STDERR".to_string(),
                         "--name".to_string(), d.name(),
                         "-w".to_string(), working_dir.clone()]);
        // 指定用户
        unsafe {
            args.extend(vec!["-u".to_string(),
                             getuid().to_string(), getgid().to_string()]);
        }
        // 添加卷
        for vol in d.volumes::<Vec<String>>(){
            debug!("volume: {}", &vol);
            args.extend(vec!["-v".to_string(), vol.clone()])
        }
        // 设置环境变量
        for (key, value) in env{
            let kv = format!("{}={}", key, value);
            args.extend(vec!["-e".to_string(), kv.clone()])
        }
        // 设置内存限制
        if d.memory_limit.0 != 0{
            args.extend(vec!["-m".to_string(), format!("{}", d.memory_limit.value())])
        }
        // 添加options
        args.extend(d.options.clone());
        // 添加镜像和command
        args.push(d.image.clone());
        // 添加command
        args.extend(cmd_and_args.clone());
        
        let mut tmp_cmd = process::Command::new(c);
        tmp_cmd.args(&args);
        cmd = Some(tmp_cmd);
        
    }else if let Some(_) = provider.c_group(){
        let mut tmp_cmd = process::Command::new("rtsync-exec");
        tmp_cmd.args(cmd_and_args);
        cmd = Some(tmp_cmd);
    }else { 
        if cmd_and_args.len() == 1{
            cmd = Some(process::Command::new(&cmd_and_args[0]));
        }else if cmd_and_args.len() > 1 {
            let c = cmd_and_args[0].clone();
            args.extend_from_slice(&cmd_and_args[1..].to_vec());
            let mut tmp_cmd = process::Command::new(c);
            tmp_cmd.args(&args);
            cmd = Some(tmp_cmd);
        }else if cmd_and_args.len() == 0 {
            panic!("Command的长度最少是1！")
        }
    }
    if let None = provider.docker(){
        debug!("在{}执行{}命令", cmd_and_args[0], working_dir);
        if let Ok(_) = fs::create_dir_all(&working_dir) {
            debug!("创建了文件夹：{}", &working_dir);
            if let Err(e) = fs::set_permissions(&working_dir, Permissions::from_mode(0o755)){
                Err(e).expect(format!("创建文件夹:{} 失败",&working_dir).as_str())
            }
        }
        let mut tmp_cmd = process::Command::new("");
        tmp_cmd.current_dir(working_dir).envs(env.clone());
        cmd = Some(tmp_cmd)
            
    }
    
    Mutex::new(
        CmdJob{
            cmd,
            working_dir: working_dir.to_string(),
            env: env.clone(),
            log_file: None,
            finished: mpsc::channel::<()>().0,
            provider: provider.clone(),
            ret_err: Ok(()),
        }
    )
}

impl<T:MirrorProvider, E:std::error::Error> CmdJob<T, E> {
    fn start(&mut self) -> Result<(), E>{
        let cg = self.provider.c_group();
        let pip_r: File;
        let pip_w: File;
        if let Some(_) = cg{
            debug!("为job：{}准备cgroup同步管道", self.provider.name());
            
        }
        
        
        Ok(())
    }
}