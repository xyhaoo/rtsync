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
    use internal::logger::init_logger;
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::hooks::HookType;
    use crate::loglimit_hook::*;
    use crate::provider::MirrorProvider;
    use glob::glob;
    use tokio::sync::mpsc::channel;
    use tokio::sync::Semaphore;
    use internal::status::SyncStatus;
    use crate::job::{JobCtrlAction, JobMessage, MirrorJob};

    #[tokio::test]
    async fn test_log_limiter() {
        init_logger(true, true, false);
        
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        
        let tmp_log_dir = Builder::new()
            .prefix("rtsync-log")
            .tempdir().expect("failed to create tmp dir");
        let tmp_log_dir_path = tmp_log_dir.path();
        
        let script_file_path = tmp_dir_path.join("cmd.sh");

        let c = CmdConfig{
            name: "rt-loglimit".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: format!("bash {}", script_file_path.display().to_string()),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_log_dir_path.display().to_string(),
            log_file: tmp_log_dir_path.join("latest.log").display().to_string(),
            interval: Duration::seconds(600),
            ..CmdConfig::default()
        };

        let mut provider = CmdProvider::new(c).await.unwrap();
        let limiter = LogLimiter::new();
        provider.add_hook(HookType::LogLimiter(limiter)).await;

        // test_logs_are_created_simply(provider, tmp_log_dir_path, script_file_path).await;
        test_job_failed_simply(provider, tmp_log_dir_path, script_file_path).await;
    }
    async fn test_logs_are_created_simply(provider: CmdProvider, tmp_log_dir_path: &Path, script_file_path: PathBuf){
        for i in 0..15{
            let f_n = tmp_log_dir_path.join(format!("{}-{}.log", provider.name(), i));
            {
                let script_file = File::create(&f_n)
                    .expect("failed to create tmp file");
            }
        }
        let matches: Vec<PathBuf> = glob(tmp_log_dir_path.join("*.log").to_str().unwrap())
            .unwrap()
            .map(|path| {path.unwrap()})
            .collect();
        assert_eq!(matches.len(), 15);
        
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
        let mut manager_chan = channel::<JobMessage>(1);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

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
        let log_file = job.provider.log_file().await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Success);

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();

        let matches: Vec<PathBuf> = glob(tmp_log_dir_path.join("*.log").to_str().unwrap())
            .unwrap()
            .map(|path| {path.unwrap()})
            .collect();
        assert_eq!(matches.len(), 10);

        let expected_output = format!("{}\n{}\n{}\n{}\n",
                                      job.provider.working_dir().await,
                                      job.provider.name(),
                                      job.provider.upstream(),
                                      log_file);

        let logged_content = fs::read_to_string(Path::new(&job.provider.log_dir().await).join("latest"))
            .expect("failed to read log file");
        
        assert_eq!(logged_content, expected_output);
    }
    async fn test_job_failed_simply(provider: CmdProvider, tmp_log_dir_path: &Path, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
echo $RTSYNC_MIRROR_NAME
echo $RTSYNC_UPSTREAM_URL
echo $RTSYNC_LOG_FILE
sleep 5
			"#;

        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let mut manager_chan = channel::<JobMessage>(1);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

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
        let log_file = job.provider.log_file().await;
        
        tokio::time::sleep(core::time::Duration::from_secs(1)).await;
        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();
        
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);
        
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
        
        assert_ne!(log_file, job.provider.log_file().await);
        
        let expected_output = format!("{}\n{}\n{}\n{}\n", 
                                      job.provider.working_dir().await, 
                                      job.provider.name(), 
                                      job.provider.upstream(), 
                                      log_file);
        
        let logged_content = fs::read_to_string(Path::new(&job.provider.log_dir().await).join("latest")).unwrap();
        assert_eq!(logged_content, expected_output);
        let logged_content = fs::read_to_string(format!("{log_file}.fail")).unwrap();
        assert_eq!(logged_content, expected_output);
    }
    
}