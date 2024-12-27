#[cfg(test)]
mod tests {
    use chrono::Duration;
    use tempfile::{Builder, TempDir};
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::hooks::JobHook;
    use crate::provider::MirrorProvider;
    use crate::zfs_hook::*;
    #[tokio::test]
    async fn test_zfs_hook() {
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let temp_file_path = tmp_dir_path.join("log_file");

        let c = CmdConfig{
            name: "rt_zfs_hook_test".into(),
            upstream_url: "http://mirrors.tuna.moe/".into(),
            command: "ls".into(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            interval: Duration::seconds(1),
            ..CmdConfig::default()
        };
        let provider = CmdProvider::new(c.clone()).await.unwrap();

        // working_dir_do_not_exist(provider, tmp_dir).await
        working_dir_is_not_a_mount_point(provider, tmp_dir).await
    }
    async fn working_dir_do_not_exist(provider: CmdProvider, tmp_dir: TempDir){
        tmp_dir.close().unwrap();
        let hook = ZfsHook::new("test_pool".into());
        let err = hook.pre_job(provider.working_dir().await, provider.name().await);
        assert!(err.is_err());
    }
    async fn working_dir_is_not_a_mount_point(provider: CmdProvider, tmp_dir: TempDir){
        let hook = ZfsHook::new("test_pool".into());
        let err = hook.pre_job(provider.working_dir().await, provider.name().await);
        assert!(err.is_err());
    }

}