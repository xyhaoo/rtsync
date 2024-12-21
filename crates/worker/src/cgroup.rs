use std::env;
use std::fs::File;
use std::io::{self, BufRead, Read};
use std::process::{Command, exit};
use std::os::unix::io::FromRawFd;
use std::os::unix::process::CommandExt;
use std::ffi::CString;

use crate::hooks::{EmptyHook, JobHook};
use crate::config::{CGroupConfig, MemBytes};


use std::{fs, io::{ Write}, path::Path};
use std::error::Error;
use std::sync::{Arc, Mutex};
use log::{debug, info, log, trace};
use crate::context::Context;
// use cgroups_rs::hierarchies;

#[derive(Debug, Clone)]
pub struct CGroupHook{
    cg_cfg: CGroupConfig,
    mem_limit: MemBytes,
    // cg_mgr_v1: cgroups_rs::Cgroup,
    // cg_mgr_v2: cgroups_rs::Cgroup,
}

#[derive(Debug)]
enum ExecCmd {
    Cont,
    Abrt,
}

const CMD_CONT: &str = "cont";
const CMD_ABRT: &str = "abrt";

fn exec_cmd(command: &str) -> Option<ExecCmd> {
    match command {
        CMD_CONT => Some(ExecCmd::Cont),
        CMD_ABRT => Some(ExecCmd::Abrt),
        _ => None,
    }
}
fn wait_exec() -> io::Result<()> {
    // 1. Check for the command passed as an argument
    let binary = env::args().nth(1).expect("No command specified");

    // 2. Open the pipe (file descriptor 3)
    let mut pipe = unsafe { File::from_raw_fd(3) };

    // 3. Read from the pipe if available
    let mut cmd_bytes = Vec::new();
    if let Ok(_) = pipe.read_to_end(&mut cmd_bytes) {
        let cmd_str = String::from_utf8_lossy(&cmd_bytes);
        if let Some(cmd) = exec_cmd(&cmd_str) {
            match cmd {
                ExecCmd::Abrt => {
                    eprintln!("Aborting as requested");
                    exit(1);
                }
                ExecCmd::Cont => {
                    // Continue execution (no-op here, can be expanded)
                    println!("Continuing execution...");
                }
            }
        }
    }

    // 4. Execute the binary command
    let args: Vec<String> = env::args().skip(1).collect();
    let env_vars = env::vars().collect::<Vec<_>>();

    // Run the external command using Command
    let mut child = Command::new(&binary)
        .args(&args)
        .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .spawn();

    // In case exec fails (it doesn't return on success), we panic
    match child {
        Err(e) => {
            panic!("Exec failed: {}", e);
        }
        Ok(_) => {}
    }
    Ok(())
}

fn init() {
    // Here, we would put any setup or initialization code you need
    println!("Initialization code goes here...");
}

fn get_self_cgroup_path() -> Result<String, io::Error> {
    let file_path = "/proc/self/cgroup"; // 当前进程的 CGroup 信息文件
    let file = fs::File::open(file_path)?; // 打开文件
    let reader = io::BufReader::new(file);

    for line in reader.lines() {
        let line = line?; // 读取每一行
        // 每行格式通常为: <subsystem>:<cgroup_id>:<path>
        let parts: Vec<&str> = line.split(':').collect();

        // 如果读取到有效的 CGroup 行
        if parts.len() >= 3 {
            let cgroup_path = parts[2]; // CGroup 路径
            return Ok(cgroup_path.to_string())
        }
    }
    Err(io::ErrorKind::InvalidData.into())
}

impl CGroupHook{
    fn new(cg_cfg: CGroupConfig, mem_limit: MemBytes) -> Self {
        CGroupHook{
            cg_cfg,
            mem_limit,
        }
    }
    
    /*
    fn init_cgroup(&mut self) -> io::Result<()> {
        let os = env::consts::OS;
        if os != "linux" {
            panic!("Only linux is supported");
        }
        
        debug!("初始化cgroup");
    
        // 如果 base_group 为空，表示使用当前进程的 cgroup，否则指定一个绝对路径
        let mut base_group = self.group.clone();
        if let Some(group) = base_group{
            if !group.is_empty(){
                base_group = Some(format!("/{}", group)); // Ensure absolute path
            }else { 
                base_group = None;
            }
        }
        
        // 检查现在是v1还是v2，is_unified为true表示v2
        let is_unified = hierarchies::is_cgroup2_unified_mode();
        self.is_unified = Some(is_unified); // You can check cgroup mode here
        
        // v2的处理
        if is_unified {
            debug!("监测到Cgroup V2");
            let mut group_path = base_group.clone();
            if group_path.is_none() {
                debug!("检测我的cgroup路径");
                match get_self_cgroup_path() {
                    Ok(cgroup_path) => {
                        group_path = Some(cgroup_path);
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
            info!("使用cgroup路径：{}", group_path.unwrap());
            
            let group_path = if base_group.is_empty() {
                self.detect_cgroup_path()?
            } else {
                base_group
            };
            println!("Using cgroup path: {}", group_path);
            self.cg_mgr_v2 = Some(group_path);
        } else {
            println!("Cgroup V1 detected");
            let group_path = if base_group.is_empty() {
                "/sys/fs/cgroup" // Default path
            } else {
                &base_group
            };
            self.cg_mgr_v1 = Some(group_path.to_string());
        }
    
        // Further logic for creating subgroups and moving processes (simplified)
        self.create_and_move_processes()
    }
     */

    fn detect_cgroup_path(&self) -> io::Result<String> {
        // Simulate detection of the current cgroup path for the process
        Ok("/sys/fs/cgroup/unified".to_string())
    }

    // fn create_and_move_processes(&self) -> io::Result<()> {
    //     // Logic to create subgroups and move processes into it (simplified)
    //     println!("Creating subgroups and moving processes...");
    // 
    //     if let Some(group_path) = &self.cg_mgr_v2 {
    //         println!("Creating sub group in cgroup v2: {}", group_path);
    //         // Here you could interact with the cgroup v2 filesystem to create subgroups
    //     }
    // 
    //     Ok(())
    // }
}

impl JobHook for CGroupHook  {

    fn pre_exec(&self,
                _provider_name: String,
                _log_dir: String,
                _log_file: String,
                _working_dir: String,
                _context: Arc<Mutex<Option<Context>>>)
        -> Result<(), Box<dyn Error>> 
    {
        todo!()
    }
    fn post_exec(&self, context: Arc<Mutex<Option<Context>>>, provider_name: String) -> Result<(), Box<dyn Error>> {
        todo!()
    }
}



