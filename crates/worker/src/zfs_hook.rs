use std::error::Error;
use std::path::Path;
use std::process::Command;
use log::{error, info, log};
use crate::hooks::{EmptyHook, JobHook};
use crate::provider::MirrorProvider;
use users;
use users::get_current_uid;

pub(crate) struct ZfsHook<T: Clone> {
    empty_hook: EmptyHook<T>,
    z_pool: String,
}
pub(crate) fn new_zfs_hook<T: Clone>(provider: Box<dyn MirrorProvider<ContextStoreVal=T>>, z_pool: String) -> ZfsHook<T> {
    ZfsHook{
        empty_hook: EmptyHook{
            provider,
        },
        z_pool,
    }
}
impl<T: Clone> ZfsHook<T> {
    fn print_help_message(&self) {
        let zfs_dataset = format!("{}/{}", self.z_pool, self.empty_hook.provider.name()).to_lowercase();
        let working_dir = self.empty_hook.provider.working_dir();
        info!("你可能正在使用以下数据创建ZFS数据集：");
        info!("  zfs create '{}'", zfs_dataset);
        info!("  zfs挂载点='{}' '{}'", working_dir, zfs_dataset);
        let usr = users::get_user_by_uid(get_current_uid());
        if let None = usr {return}
        let usr = usr.unwrap();
        if usr.uid() == 0 {return}
        info!("  chown {} '{}'", usr.uid(), working_dir);
    }
}
impl<T: Clone> JobHook for ZfsHook<T> {
    // 检查工作目录是否为ZFS数据集
    fn per_job(&self) -> Result<(), Box<dyn Error>> {
        let working_dir = self.empty_hook.provider.working_dir();
        if !Path::new(&working_dir).exists() {
            let err = format!("目录 {} 不存在", working_dir);
            error!("{err}");
            self.print_help_message();
            return Err(err.into());
        }
        let output = Command::new("mountpoint")
            .arg("-q")
            .arg(&working_dir)
            .output()?;

        if !output.status.success() {
            let err = format!("{} 不是挂载点", working_dir);
            error!("{err}");
            self.print_help_message();
            return Err(err.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::Builder;
    use crate::cmd_provider::CmdConfig;
    use super::*;
    #[test]
    fn test_zfs_hook() {
        let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let tmp_file_path = tmp_dir_path.join("log_file");
        // let c = CmdConfig{
        //     
        // }
    }
}