#[cfg(test)]
mod tests {
    use chrono::Duration;
    use tempfile::Builder;
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::exec_post_hook::*;
    use crate::hooks::HookType;
    use crate::provider::MirrorProvider;

    #[tokio::test]
    async fn test_exec_post_hook() {
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();

        let script_file_path = tmp_dir_path.join("cmd.sh");

        let c = CmdConfig{
            name: "rt-exec-post".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: script_file_path.display().to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: tmp_dir_path.join("latest.log").display().to_string(),
            interval: Duration::seconds(600),
            ..CmdConfig::default()
        };

        let mut provider = CmdProvider::new(c.clone()).await.unwrap();

        test_on_success(&mut provider).await;
    }

    async fn test_on_success(provider: &mut CmdProvider) {
        let command = r#"bash -c 'echo ${RTSYNC_JOB_EXIT_STATUS} > ${RTSYNC_WORKING_DIR}/exit_status'"#;
        let hook = ExecPostHook::new(ExecOn::Success, command).unwrap();
        provider.add_hook(HookType::ExecPost(hook)).await;

    }
}