#[cfg(test)]
mod tests {
    use core::time;
    use std::fmt::format;
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use chrono::Duration;
    use tempfile::Builder;
    use tokio::sync::mpsc::{channel, Receiver, Sender};
    use tokio::sync::Semaphore;
    use crate::job::*;
    use internal::logger::init_logger;
    use internal::status::SyncStatus;
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::common::DEFAULT_MAX_RETRY;
    use crate::exec_post_hook::{ExecOn, ExecPostHook};
    use crate::hooks::HookType;
    use crate::provider::MirrorProvider;

    #[tokio::test]
    async fn test_mirror_job(){
        init_logger(true, true, false);
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let script_file_path = tmp_dir_path.join("cmd.sh");
        let temp_file_path = tmp_dir_path.join("log_file");

        let c = CmdConfig{
            name: "rt-cmd-jobtest".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: format!("bash {}", script_file_path.display().to_string()),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            interval: Duration::seconds(1),
            timeout: Duration::seconds(7),
            ..CmdConfig::default()
        };

        let provider = CmdProvider::new(c.clone()).await.unwrap();
        assert_eq!(provider.name().await, c.name);
        assert_eq!(provider.working_dir().await, c.working_dir);
        assert_eq!(provider.log_dir().await, c.log_dir);
        assert_eq!(provider.log_file().await, c.log_file);
        assert_eq!(provider.interval().await, c.interval);
        assert_eq!(provider.timeout().await, c.timeout);

        test_a_normal_job(provider, script_file_path).await;
        // test_running_long_jobs_with_post_fail_hook(provider, tmp_dir_path, script_file_path).await;
        // test_running_long_jobs(provider, script_file_path).await;
        // test_a_job_timeout(provider, script_file_path).await;

    }
    async fn test_a_normal_job(provider: CmdProvider, script_file_path: PathBuf) {
        let script_content = r#"#!/bin/bash
			echo $RTSYNC_WORKING_DIR
			echo $RTSYNC_MIRROR_NAME
			echo $RTSYNC_UPSTREAM_URL
			echo $RTSYNC_LOG_FILE
			"#;

        let expected_output = format!("{}\n{}\n{}\n{}\n",
                                      provider.working_dir().await,
                                      provider.name().await,
                                      provider.cmd_config.upstream_url,
                                      provider.log_file().await);
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let read_ed_script_content = fs::read_to_string(&script_file_path).unwrap();
        assert_eq!(read_ed_script_content, script_content);

        // if we run several times
        let mut manage_chan = channel::<JobMessage>(10);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));
        let job_clone = job.clone();
        let manager_chan_clone = manage_chan.0.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan_clone, semaphore).await
        });
        tokio::select! {
            _ = manage_chan.1.recv() => assert!(false),
            _ = tokio::time::sleep(time::Duration::from_secs(1)) => {assert!(true);}
        }

        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        for _ in 0..2{
            let msg = manage_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::PreSyncing);

            let msg = manage_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Syncing);

            let msg = manage_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Success);

            let logged_content = fs::read_to_string(job.provider.log_file().await).unwrap();
            assert_eq!(logged_content, expected_output);
            job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        }
        tokio::select! {
            Some(msg) = manage_chan.1.recv() => {
                assert_eq!(msg.status, SyncStatus::PreSyncing);

                let msg = manage_chan.1.recv().await.unwrap();
                assert_eq!(msg.status, SyncStatus::Syncing);

                let msg = manage_chan.1.recv().await.unwrap();
                assert_eq!(msg.status, SyncStatus::Success);
            },
            _ = tokio::time::sleep(time::Duration::from_secs(2)) => {assert!(false);}
        }

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        tokio::select! {
            _ = manage_chan.1.recv() => assert!(false),
            _ = disabled_rx.recv() => assert!(true),
        }

    }

    async fn test_running_long_jobs_with_post_fail_hook(mut provider: CmdProvider, tmp_dir_path: &Path, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
echo '++++++'
echo $RTSYNC_WORKING_DIR
echo $0 sleeping
sleep 3
echo $RTSYNC_WORKING_DIR
echo '------'
			"#;

        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let hook_script_file_path = tmp_dir_path.join("hook.sh");
        {
            let mut script_file = File::create(&hook_script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }
        let h = ExecPostHook::new(ExecOn::Failure, hook_script_file_path.to_str().unwrap()).unwrap();
        provider.add_hook(HookType::ExecPost(h)).await;

        let manager_chan = channel::<JobMessage>(10);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

        // test_kill_it(job, manager_chan, semaphore).await;
        test_kill_it_then_start_it(job, manager_chan, semaphore).await;
    }
    async fn test_kill_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>) {
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();

        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }
    async fn test_kill_it_then_start_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();

        tokio::time::sleep(time::Duration::from_secs(2)).await;
        println!("now starting...");
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();

        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }

    async fn test_running_long_jobs(provider: CmdProvider, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
sleep 5
echo $RTSYNC_WORKING_DIR
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let manager_chan = channel::<JobMessage>(10);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

        // test_kill_it_(job, manager_chan, semaphore).await;
        // test_do_not_kill_it(job, manager_chan, semaphore).await;
        // test_restart_it(job, manager_chan, semaphore).await;
        // test_disabled_it(job, manager_chan, semaphore).await;
        test_stop_it_twice_then_start_it(job, manager_chan, semaphore).await;
    }
    async fn test_kill_it_(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap(); // 应该被忽略

        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);

        let expected_output = format!("{}\n", job.provider.working_dir().await);
        let logged_content = fs::read_to_string(&job.provider.log_file().await).unwrap();
        assert_eq!(expected_output, logged_content);
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;

    }
    async fn test_do_not_kill_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Success);

        let expected_output = format!("{}\n{}\n",
                                      job.provider.working_dir().await,
                                      job.provider.working_dir().await);
        let logged_content = fs::read_to_string(&job.provider.log_file().await).unwrap();
        assert_eq!(expected_output, logged_content);
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }
    async fn test_restart_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Restart).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);
        assert_eq!(msg.msg, "被manager终止");

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Success);

        let expected_output = format!("{}\n{}\n",
                                      job.provider.working_dir().await,
                                      job.provider.working_dir().await);
        let logged_content = fs::read_to_string(&job.provider.log_file().await).unwrap();
        assert_eq!(expected_output, logged_content);
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }
    async fn test_disabled_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);
        assert_eq!(msg.msg, "被manager终止");

        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }
    async fn test_stop_it_twice_then_start_it(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);
        assert_eq!(msg.msg, "被manager终止");

        job.ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap(); // 应该被忽略

        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Success);

        let expected_output = format!("{}\n{}\n",
                                      job.provider.working_dir().await,
                                      job.provider.working_dir().await);

        let logged_content = fs::read_to_string(&job.provider.log_file().await).unwrap();
        assert_eq!(expected_output, logged_content);

        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }

    async fn test_a_job_timeout(provider: CmdProvider, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
sleep 10
echo $RTSYNC_WORKING_DIR
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let manager_chan = channel::<JobMessage>(10);
        let semaphore = Arc::new(Semaphore::new(1));
        let job = MirrorJob::new(Box::new(provider));

        // test_it_should_be_automatically_terminated(job, manager_chan, semaphore).await;
        test_it_should_be_retried(job, manager_chan, semaphore).await;
    }
    async fn test_it_should_be_automatically_terminated(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Syncing);

        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap(); // 应该被忽略

        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::Failed);

        let expected_output = format!("{}\n", job.provider.working_dir().await);
        let logged_content = fs::read_to_string(&job.provider.log_file().await).unwrap();
        assert_eq!(expected_output, logged_content);
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }
    async fn test_it_should_be_retried(job: MirrorJob, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        let job_clone = job.clone();
        tokio::spawn(async move {
            job_clone.run(manager_chan.0, semaphore).await
        });
        job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        tokio::time::sleep(time::Duration::from_secs(1)).await;
        let msg = manager_chan.1.recv().await.unwrap();
        assert_eq!(msg.status, SyncStatus::PreSyncing);

        for i in 0..DEFAULT_MAX_RETRY{
            let msg = manager_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Syncing);

            job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();

            let msg = manager_chan.1.recv().await.unwrap();
            assert_eq!(msg.status, SyncStatus::Failed);
            assert!(msg.msg.contains("等待时间"));
            // re-schedule after last try
            assert_eq!(msg.schedule, i == DEFAULT_MAX_RETRY-1)
        }
        job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
        let mut disabled_lock = job.disabled_rx.lock().await;
        let disabled_rx = disabled_lock.as_mut().unwrap();
        let _ = disabled_rx.recv().await;
    }

    
    

    async fn counting_jobs(manager_chan_rx: &mut Receiver<JobMessage>,
                           total_jobs: usize,
                           concurrent_check: usize)
                           -> (usize, i32)
    {
        let mut counter_ended = 0;      // 已结束的任务数
        let mut counter_running = 0;    // 正在运行的任务数
        let mut peak_concurrent = 0;    // 并发峰值
        let mut counter_failed = 0;       // 失败的任务数  
        while counter_ended < total_jobs{
            let msg = manager_chan_rx.recv().await.unwrap();
            match msg.status {
                SyncStatus::PreSyncing => { 
                    counter_running += 1; 
                },
                SyncStatus::Syncing => {},
                SyncStatus::Failed => {
                    counter_failed += 1;
                    counter_ended += 1;
                    counter_running -= 1;
                },
                SyncStatus::Success => {
                    counter_ended += 1;
                    counter_running -= 1;
                },
                _ => assert!(false),
            }
            // 检查并发限制是否正常
            assert!(counter_running <= concurrent_check);
            if counter_running > peak_concurrent {
                peak_concurrent = counter_running;
            }
        }
        (peak_concurrent, counter_failed)
    }
    const CONCURRENT: usize = 5;
    
    
    #[tokio::test]
    async fn test_concurrent_mirror_jobs(){
        init_logger(true, true, false);
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        
        let mut jobs: Vec<MirrorJob> = Vec::with_capacity(CONCURRENT);
        for i in 0..CONCURRENT{
            let c = CmdConfig{
                name: format!("job-{i}"),
                upstream_url: "http://mirrors.tuna.moe/".to_string(),
                command: "sleep 2".to_string(),
                working_dir: tmp_dir_path.display().to_string(),
                log_dir: tmp_dir_path.display().to_string(),
                log_file: "/dev/null".to_string(),
                interval: Duration::seconds(10),
                ..CmdConfig::default()
            };
            let provider = CmdProvider::new(c).await.unwrap();
            jobs.push(MirrorJob::new(Box::new(provider)));
        }

        let manager_chan = channel::<JobMessage>(10);
        let semaphore = Arc::new(Semaphore::new(CONCURRENT-2));
        
        // test_run_them_all(jobs, manager_chan, semaphore).await;
        // test_cancel_one_job(jobs, manager_chan, semaphore).await;
        test_override_the_concurrent_limit(jobs, manager_chan, semaphore).await;
    }
    async fn test_run_them_all(jobs: Vec<MirrorJob>, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        for job in jobs.iter() {
            let job_clone = job.clone();
            let manager_chan = manager_chan.0.clone();
            let semaphore = Arc::clone(&semaphore);
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await
            });
            job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        }
        let (peak_concurrent, counter_failed) = 
            counting_jobs(&mut manager_chan.1, CONCURRENT, CONCURRENT-2).await;
        
        assert_eq!(peak_concurrent, CONCURRENT - 2);
        assert_eq!(counter_failed, 0);
        
        for job in jobs {
            job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
            let mut disabled_lock = job.disabled_rx.lock().await;
            let disabled_rx = disabled_lock.as_mut().unwrap();
            let _ = disabled_rx.recv().await;
        }
    }
    async fn test_cancel_one_job(jobs: Vec<MirrorJob>, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        for job in jobs.iter() {
            let job_clone = job.clone();
            let manager_chan = manager_chan.0.clone();
            let semaphore = Arc::clone(&semaphore);
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await
            });
            job.ctrl_chan_tx.send(JobCtrlAction::Restart).await.unwrap();
            tokio::time::sleep(time::Duration::from_millis(200)).await;
        }
        
        // 停止一个正在等待信号量的job
        jobs[jobs.len() - 1].ctrl_chan_tx.send(JobCtrlAction::Stop).await.unwrap();
        
        let (peak_concurrent, counter_failed) = counting_jobs(&mut manager_chan.1, CONCURRENT-1, CONCURRENT-2).await;
        
        assert_eq!(peak_concurrent, CONCURRENT-2);
        assert_eq!(counter_failed, 0);
        for job in jobs {
            job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
            let mut disabled_lock = job.disabled_rx.lock().await;
            let disabled_rx = disabled_lock.as_mut().unwrap();
            let _ = disabled_rx.recv().await;
        }
    }
    async fn test_override_the_concurrent_limit(jobs: Vec<MirrorJob>, mut manager_chan: (Sender<JobMessage>, Receiver<JobMessage>), semaphore: Arc<Semaphore>){
        for job in jobs.iter() {
            let job_clone = job.clone();
            let manager_chan = manager_chan.0.clone();
            let semaphore = Arc::clone(&semaphore);
            tokio::spawn(async move {
                job_clone.run(manager_chan, semaphore).await
            });
            job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
            tokio::time::sleep(time::Duration::from_millis(200)).await;
        }
        
        jobs[jobs.len() - 1].ctrl_chan_tx.send(JobCtrlAction::ForceStart).await.unwrap();
        jobs[jobs.len() - 2].ctrl_chan_tx.send(JobCtrlAction::ForceStart).await.unwrap();
        
        let (peak_concurrent, counter_failed) = counting_jobs(&mut manager_chan.1, CONCURRENT, CONCURRENT).await;
        
        assert_eq!(peak_concurrent, CONCURRENT);
        assert_eq!(counter_failed, 0);
        
        tokio::time::sleep(time::Duration::from_secs(1)).await;
        
        for job in jobs.iter() {
            job.ctrl_chan_tx.send(JobCtrlAction::Start).await.unwrap();
        }

        let (peak_concurrent, counter_failed) = counting_jobs(&mut manager_chan.1, CONCURRENT, CONCURRENT-2).await;
        assert_eq!(peak_concurrent, CONCURRENT-2);
        assert_eq!(counter_failed, 0);
        
        for job in jobs {
            job.ctrl_chan_tx.send(JobCtrlAction::Disable).await.unwrap();
            let mut disabled_lock = job.disabled_rx.lock().await;
            let disabled_rx = disabled_lock.as_mut().unwrap();
            let _ = disabled_rx.recv().await;
        }
        
    }
}



















