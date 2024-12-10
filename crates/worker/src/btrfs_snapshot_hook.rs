
use std::error::Error;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use log::{error, info};
use crate::config::MirrorConfig;
use crate::context::Context;
use crate::hooks::{JobHook};

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) struct BtrfsSnapshotHook {
    pub(crate) mirror_snapshot_path: String,
}

#[cfg(target_os = "linux")]
impl BtrfsSnapshotHook {
    // 运行作业的用户（通常是 rtsync）应该被授予运行BTRFS命令的权限
    // TODO: 检查文件系统是否为Btrfs
    pub(crate) fn new(provider_name: &str,
                      snapshot_path: &str,
                      mirror: MirrorConfig) -> BtrfsSnapshotHook
    {
        let mirror_snapshot_path = match mirror.snapshot_path.as_ref(){
            Some(path) => path.clone(),
            None => Path::new(snapshot_path).join(provider_name).display().to_string(),
        };
        BtrfsSnapshotHook{
            mirror_snapshot_path,
        }
    }
}




#[cfg(target_os = "linux")]
impl JobHook for BtrfsSnapshotHook {

    // 检查路径 snapshot_path/provider_name 是否存在
    // 情况1：不存在 => 创建一个新的子卷
    // 情况2：作为子卷存在 => 不处理
    // 情况3：作为目录存在 => 错误
    fn per_job(&self, working_dir: String, _provider_name: String) -> Result<(), Box<dyn Error>> {
        let path = working_dir;
        if !Path::new(&path).exists(){
            // 创建子卷
            if let Err(e) = create_btrfs_sub_volume(&path){
                error!("{e}");
                return Err(e.into())
            }
            info!("创建了新的Btrfs子卷 {path}");
        }else {
            match is_btrfs_sub_volume(&path) {
                Ok(true) => {},
                Ok(false) => {
                    let err = format!("该路径存在：{path} 但不是Btrfs子卷的路径");
                    error!("{err}");
                    return Err(err.into())
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn pre_exec(&self,
                _provider_name: String,
                _log_dir: String,
                _log_file: String,
                _working_dir: String,
                _context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>> 
    {
        Ok(())
    }

    fn post_exec(&self,
                 _context: &Arc<Mutex<Option<Context>>>, 
                 _provider_name: String) 
        -> Result<(), Box<dyn Error>> 
    {
        Ok(())
    }

    // 创建新的快照，如果旧快照存在，将其删除
    fn post_success(&self,
                    _provider_name: String,
                    working_dir: String,
                    _upstream: String,
                    _log_dir: String,
                    _log_file: String) -> Result<(), Box<dyn Error>> 
    {
        let snapshot_path = &self.mirror_snapshot_path;
        if !Path::new(snapshot_path).exists() {
            match is_btrfs_sub_volume(snapshot_path) {
                Err(e) => return Err(e),
                Ok(false) => {
                    let err = format!("该路径存在：{snapshot_path} 但不是Btrfs子卷的路径");
                    error!("{err}");
                    return Err(err.into())
                },
                Ok(true) => {},
            }
            // snapshot_path是旧快照的地址，删除它
            if let Err(e) = delete_btrfs_sub_volume(snapshot_path){
                error!("删除旧的Btrfs快照，其路径是：{snapshot_path}");
                return Err(e.into())
            }
            info!("删除了旧的快照，其路径是：{snapshot_path}")
        }
        // 创建一个新的可写快照
        // 快照是可写的，因此可以很容易地删除
        if let Err(e) = create_btrfs_snapshot(&*working_dir, snapshot_path){
            error!("创建新的Btrfs快照失败，快照路径是：{snapshot_path}");
            return Err(e.into())
        }
        info!("创建了新的Btrfs快照，其路径是：{snapshot_path}");
        Ok(())
    }
}



#[cfg(target_os = "linux")]
fn create_btrfs_sub_volume(path: &str) -> Result<(), String> {
    let output = Command::new("btrfs")
        .arg("subvolume")
        .arg("create")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "在 {} 创建Btrfs子卷失败: {}",
            path,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(target_os = "linux")]
fn is_btrfs_sub_volume(path: &str) -> Result<bool, Box<dyn Error>> {
    let output = Command::new("btrfs")
        .arg("subvolume")
        .arg("show")
        .arg(path)
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(target_os = "linux")]
fn delete_btrfs_sub_volume(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err(format!("路径 {} 不存在", path));
    }

    let output = Command::new("btrfs")
        .arg("subvolume")
        .arg("delete")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "删除子卷失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

// 创建可写btrfs快照
#[cfg(target_os = "linux")]
fn create_btrfs_snapshot(sub_volume_path: &str, snapshot_path: &str) -> Result<(), String> {
    // 构建创建快照的命令
    let output = Command::new("btrfs")
        .arg("subvolume")
        .arg("snapshot")
        .arg(sub_volume_path)
        .arg(snapshot_path)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "创建Btrfs快照失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}












