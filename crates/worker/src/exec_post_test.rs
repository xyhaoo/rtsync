#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use chrono::Duration;
    use tempfile::Builder;
    use tokio::sync::mpsc::channel;
    use tokio::sync::Semaphore;
    use internal::logger::init_logger;
    use internal::status::SyncStatus;
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::common::DEFAULT_MAX_RETRY;
    use crate::exec_post_hook::*;
    use crate::hooks::HookType;
    use crate::job::{JobCtrlAction, JobMessage, MirrorJob};
    use crate::provider::MirrorProvider;

    #[tokio::test]
    async fn test_exec_post_hook() {
        init_logger(true, true, false);
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let script_file_path = tmp_dir_path.join("cmd.sh");

        let c = CmdConfig{
            name: "rt-exec-post".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: format!("bash {}", script_file_path.display().to_string()),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: tmp_dir_path.join("latest.log").display().to_string(),
            interval: Duration::seconds(600),
            ..CmdConfig::default()
        };

        let provider = CmdProvider::new(c.clone()).await.unwrap();

        // test_on_success(provider, script_file_path).await;
        test_on_failure(provider, script_file_path).await;
    }

    async fn test_on_success(mut provider: CmdProvider, script_file_path: PathBuf) {
        let command = r#"bash -c 'echo ${RTSYNC_JOB_EXIT_STATUS} > ${RTSYNC_WORKING_DIR}/exit_status'"#;
        let hook = ExecPostHook::new(ExecOn::Success, command).unwrap();
        provider.add_hook(HookType::ExecPost(hook)).await;
        let mut manager_chan = channel::<JobMessage>(1);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));
        
        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
echo $RTSYNC_MIRROR_NAME
echo $RTSYNC_UPSTREAM_URL
echo $RTSYNC_LOG_FILE
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }


        let job_clone = job.clone();
        let manager_chan_clone = manager_chan.0.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan_clone, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Success);
    
        tokio::time::sleep(core::time::Duration::from_millis(200)).await;
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
        
        let expected_output = "success\n";
        
        let output_content = fs::read_to_string(Path::new(&job.provider.working_dir().await).join("exit_status")).unwrap();
        assert_eq!(output_content, expected_output);
        
    }
    async fn test_on_failure(mut provider: CmdProvider, script_file_path: PathBuf) {
        let command = r#"bash -c 'echo ${RTSYNC_JOB_EXIT_STATUS} > ${RTSYNC_WORKING_DIR}/exit_status'"#;
        let hook = ExecPostHook::new(ExecOn::Failure, command).unwrap();
        provider.add_hook(HookType::ExecPost(hook)).await;
        let mut manager_chan = channel::<JobMessage>(1);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
echo $RTSYNC_MIRROR_NAME
echo $RTSYNC_UPSTREAM_URL
echo $RTSYNC_LOG_FILE
exit 1
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }
        let job_clone = job.clone();
        let manager_chan_clone = manager_chan.0.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan_clone, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        for _ in 0..DEFAULT_MAX_RETRY{
            let msg = manager_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Syncing);
            let msg = manager_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Failed);
        }
        
        tokio::time::sleep(core::time::Duration::from_millis(200)).await;
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
        
        let expected_output = "failure\n";
        
        let output_content = fs::read_to_string(Path::new(&job.provider.working_dir().await).join("exit_status")).unwrap();
        assert_eq!(output_content, expected_output);
    }
}