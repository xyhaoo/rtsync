use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;
use async_trait::async_trait;
use log::{error, info};
use crate::hooks::JobHook;
use users;
use users::get_current_uid;

#[derive(Debug, Clone)]
pub(crate) struct ZfsHook {
    pub(crate) z_pool: String,
}

impl ZfsHook {
    pub(crate) fn new(z_pool: String) -> Self {
        ZfsHook{
            z_pool,
        }
    }
    fn print_help_message(&self, working_dir: String, provider_name: String) {
        let zfs_dataset = format!("{}/{}", self.z_pool, provider_name).to_lowercase();
        // let working_dir = self.empty_hook.provider.working_dir();
        info!("你可能正在使用以下数据创建ZFS数据集：");
        info!("  zfs create '{}'", zfs_dataset);
        info!("  zfs挂载点='{}' '{}'", working_dir, zfs_dataset);
        match users::get_user_by_uid(get_current_uid()){
            Some(usr) if usr.uid() == 0 => { return },
            None => {return},
            Some(usr) => {
                info!("  chown {} '{}'", usr.uid(), working_dir);
            }
        }
    }
}

#[async_trait]
impl JobHook for ZfsHook {
    // 检查工作目录是否为ZFS数据集
    fn pre_job(&self, working_dir: String, provider_name: String) -> Result<()> {
        // let working_dir = self.empty_hook.provider.working_dir();
        if !Path::new(&working_dir).exists() {
            let err = format!("目录 {} 不存在", working_dir);
            error!("{err}");
            self.print_help_message(working_dir, provider_name);
            return Err(anyhow!(err));
        }
        let output = Command::new("sh")
            .arg("-c")
            .arg("mountpoint")
            .arg("-q")
            .arg(&working_dir)
            .output()?;

        if !output.status.success() {
            let err = format!("{} 不是挂载点", working_dir);
            error!("{err}");
            self.print_help_message(working_dir, provider_name);
            return Err(anyhow!(err));
        }
        Ok(())
    }
}



// #[cfg(test)]
// mod tests {
//     use tempfile::Builder;
//     use crate::cmd_provider::CmdConfig;
//     use super::*;
//     #[test]
//     fn test_zfs_hook() {
//         let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
//             .prefix("rtsync")
//             .tempdir().expect("failed to create tmp dir");
//         let tmp_dir_path = tmp_dir.path();
//         let tmp_file_path = tmp_dir_path.join("log_file");
//         // let c = CmdConfig{
//         //     
//         // }
//     }
// }