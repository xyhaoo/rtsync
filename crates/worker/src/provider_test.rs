#[cfg(test)]
mod tests{
    use std::{fs, thread};
    use std::cell::RefCell;
    use std::error::Error;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, RwLock};
    use anymap::AnyMap;
    use chrono::Duration;
    use tempfile::Builder;
    use crossbeam_channel::{Sender, Receiver, bounded};

    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::common::Empty;
    use crate::config::{Config, DockerConfig, GlobalConfig, MirrorConfig, ProviderEnum};
    use crate::provider::*;
    use crate::rsync_provider::{RsyncConfig, RsyncProvider};
    use crate::two_stage_rsync_provider::{TwoStageRsyncConfig, TwoStageRsyncProvider};

    fn resolve_symlink(mut path: PathBuf) -> Result<PathBuf, std::io::Error> {
        loop {
            // 尝试读取符号链接
            match fs::read_link(&path) {
                Ok(target) => {
                    // 如果链接解析成功，更新当前路径为目标路径
                    path = target;
                }
                Err(_) => {
                    // 如果不是符号链接，退出循环
                    break;
                }
            }
        }
        Ok(path)
    }

    #[test]
    fn rsync_provider_test(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();

        let script_file_path = tmp_dir_path.join("myrsync");
        let script_file = File::create(&script_file_path)
            .expect("failed to create tmp file");
        fs::set_permissions(&script_file_path, fs::Permissions::from_mode(0o755)).unwrap();

        let tmp_file_path = tmp_dir_path.join("log_file");
        let temp_file = File::create(&tmp_file_path)
            .expect("failed to create tmp file");

        let c = RsyncConfig{
            name: "rt".to_string(),
            upstream_url: "rsync://mirror.tuna.tsinghua.edu.cn/apache/README.html".to_string(),
            rsync_cmd: script_file_path.display().to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: tmp_file_path.display().to_string(),
            use_ipv6: true,
            timeout: Duration::seconds(100),
            interval: Duration::seconds(600),
            ..RsyncConfig::default()
        };
        let provider = RsyncProvider::new(c.clone()).unwrap();

        assert_eq!(provider.r#type(), ProviderEnum::Rsync);
        assert_eq!(provider.name(), c.name);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);
        assert_eq!(provider.timeout(), c.timeout);


        // test_entering_and_exiting_a_context(provider, c);
        test_run(provider, c, script_file);

    }
    fn test_entering_and_exiting_a_context(mut provider: RsyncProvider, c: RsyncConfig){
        let mut func = || {
            let ctx = provider.enter_context();
            assert_eq!(provider.working_dir(), c.working_dir);
            let new_working_dir = "/srv/mirror/working/tuna".to_string();
            let mut value = AnyMap::new();
            value.insert(new_working_dir.clone());
            if let Ok(mut ctx) = ctx.lock(){
                if let Some(context) = ctx.as_mut(){
                    context.set(_WORKING_DIR_KEY.parse().unwrap(), value);
                }else { 
                    panic!("没有ctx字段");
                }
            }else { 
                panic!("加锁失败");
            }
            assert_eq!(provider.working_dir(), new_working_dir);
            provider.exit_context();
        };
        func();
        assert_eq!(provider.working_dir(), c.working_dir);
    }

    // 使用一个生成的脚本文件作为rsync_provider要执行的命令，模拟rsync命令运行
    fn test_run(mut provider: RsyncProvider, c: RsyncConfig, mut script_file: File){
        let script_content = r#"#!/bin/bash
echo "syncing to $(pwd)"
echo $RSYNC_PASSWORD $@
sleep 1
echo "Total file size: 1.33T bytes"
echo "Done"
exit 0
			"#;
        script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");

        let target_dir = resolve_symlink(PathBuf::from(provider.working_dir())).unwrap();
        
        let s1 = "-aHvh --no-o --no-g --stats --filter risk .~tmp~/ --exclude .~tmp~/ ";
        let s2 = "--delete --delete-after --delay-updates --safe-links ";
        let s3 = "--timeout=120 -6";
        let expected_output = format!("syncing to {}\n{}\nTotal file size: 1.33T bytes\nDone\n",
                                      target_dir.to_string_lossy(),
                                      format!("{s1}{s2}{s3} {} {}", 
                                              provider.rsync_config.upstream_url, 
                                              provider.working_dir()));


        provider.run(bounded(1).0).unwrap();
        let logged_content = fs::read_to_string(provider.log_file()).unwrap();
        assert_eq!(expected_output, logged_content);
        assert_eq!(provider.data_size(), "1.33T".to_string());
    }

    #[test]
    // 测试当rsync参数错误时，能否将错误信息写入log_file
    fn test_rsync_fails(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let log_file_path = tmp_dir_path.join("log_file");
        let log_file = File::create(&log_file_path)
            .expect("failed to create tmp file");
        
        let c = RsyncConfig{
            name: "rt".to_string(),
            upstream_url: "rsync://mirror.tuna.tsinghua.edu.cn/apache/README.html".to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: log_file_path.display().to_string(),
            extra_options: vec!["--somethine-invalid".to_string()],
            interval: Duration::seconds(600),
            ..Default::default()
        };

        // let x = translate_rsync_error_code(1).expect("failed to translate rsync error");

        let mut provider = RsyncProvider::new(c.clone()).unwrap();
        assert!(provider.run(bounded(1).0).is_err());
        let logged_content = fs::read_to_string(provider.log_file()).unwrap();
        assert!(logged_content.contains("Syntax or usage error"));

    }
    
    #[test]
    fn test_rsync_provider_with_password(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();

        let script_file_path = tmp_dir_path.join("myrsync");
        let mut script_file = File::create(&script_file_path)
            .expect("failed to create tmp file");
        fs::set_permissions(&script_file_path, fs::Permissions::from_mode(0o755)).unwrap();

        let log_file_path = tmp_dir_path.join("log_file");
        let log_file = File::create(&log_file_path)
            .expect("failed to create tmp file");

        let proxy_addr = "127.0.0.1:1233".to_string();
        
        let c = RsyncConfig{
            name: "rt".to_string(),
            upstream_url: "rsync://mirror.tuna.tsinghua.edu.cn/apache/README.html".to_string(),
            rsync_cmd: script_file_path.display().to_string(),
            username: "rtsync".to_string(),
            password: "rtsyncpassword".to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            extra_options: vec!["--delete-excluded".to_string()],
            rsync_timeout_value: 30,
            rsync_env: [("RSYNC_PROXY".to_string(), proxy_addr.clone())].into(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: log_file_path.display().to_string(),
            use_ipv4: true,
            interval: Duration::seconds(600),
            ..Default::default()
        };
        let mut provider = RsyncProvider::new(c.clone()).unwrap();
        assert_eq!(provider.name(), c.name);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);
        
        // test run
        let script_content = r#"#!/bin/bash
echo "syncing to $(pwd)"
echo $USER $RSYNC_PASSWORD $RSYNC_PROXY $@
sleep 1
echo "Done"
exit 0
			"#;

        script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");

        let target_dir = resolve_symlink(PathBuf::from(provider.working_dir())).unwrap();

        let s1 = "-aHvh --no-o --no-g --stats --filter risk .~tmp~/ --exclude .~tmp~/ ";
        let s2 = "--delete --delete-after --delay-updates --safe-links ";
        let s3 = "--timeout=30 -4 --delete-excluded";
        let expected_output = format!("syncing to {}\n{}\nDone\n",
                                      target_dir.to_string_lossy(),
                                      format!("{} {} {} {s1}{s2}{s3} {} {}",
                                              provider.rsync_config.username,
                                              provider.rsync_config.password,
                                              proxy_addr,
                                              provider.rsync_config.upstream_url,
                                              provider.working_dir()));

        //////////////////////////////////
//         let fff = "/root/documents/rust/fff";
//         let mut file = File::create(fff).unwrap();
//         fs::set_permissions(fff, fs::Permissions::from_mode(0o755)).unwrap();
//         let script_echo = r#"#!/bin/bash
// echo "syncing to $(pwd)"
// echo $USER $RSYNC_PASSWORD $RSYNC_PROXY $@
// sleep 1
// echo "Done"
// exit 0
// 			"#;
//         file.write_all(script_echo.as_bytes()).expect("failed to write to tmp file");
//         let cc = std::process::Command::new(fff)
//             .env("USER", "rtsync")
//             .env("RSYNC_PASSWORD", "rtsyncpassword")
//             .env("RSYNC_PROXY", "127.0.0.1:1233")
//             .output().expect("failed to run echo");
//         println!("{:?}", cc);
//         //////////////////////////////////


        provider.run(bounded(1).0).unwrap();
        let logged_content = fs::read_to_string(provider.log_file()).unwrap();
        assert_eq!(expected_output, logged_content);
    }
    
    #[test]
    fn test_rsync_provider_with_overridden_options(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let script_file_path = tmp_dir_path.join("myrsync");
        let mut script_file = File::create(&script_file_path)
            .expect("failed to create tmp file");
        let log_file_path = tmp_dir_path.join("log_file");
        let log_file = File::create(&log_file_path)
            .expect("failed to create tmp file");
        
        let c = RsyncConfig{
            name: "rt".to_string(),
            upstream_url: "rsync://rsync.tuna.moe/tuna/".to_string(),
            rsync_cmd: script_file_path.display().to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            rsync_never_timeout: true,
            overridden_options: vec!["-aHvh".into(), "--no-o".into(), "--no-g".into(), "--stats".into()],
            extra_options: vec!["--delete-excluded".to_string()],
            log_dir: tmp_dir_path.display().to_string(),
            log_file: log_file_path.display().to_string(),
            use_ipv6: true,
            interval: Duration::seconds(600),
            ..Default::default()
        };
        let mut provider = RsyncProvider::new(c.clone()).unwrap();
        assert_eq!(provider.name(), c.name);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);

        // test run
        let script_content = r#"#!/bin/bash
echo "syncing to $(pwd)"
echo $@
sleep 1
echo "Done"
exit 0
			"#;


        script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
        let target_dir = resolve_symlink(PathBuf::from(provider.working_dir())).unwrap();
        
        let expected_output = format!("syncing to {}\n-aHvh --no-o --no-g --stats -6 --delete-excluded {} {}\nDone\n", 
                                      target_dir.to_string_lossy(), 
                                      provider.rsync_config.upstream_url, 
                                      provider.working_dir());
        
        provider.run(bounded(1).0).unwrap();
        let logged_content = fs::read_to_string(provider.log_file()).unwrap();
        assert_eq!(logged_content, expected_output);

    }

    #[test]
    // 创建一个临时容器，将本机临时文件目录挂载到容器指定位置，然后在容器内运行目录内的脚本
    fn test_rsync_in_docker(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();

        let script_file_path = tmp_dir_path.join("myrsync");
        let exclude_file_path = tmp_dir_path.join("exclude.txt");

        // 还是这个问题，在linux上运行时文件句柄不能正常关闭，导致text file busy，现在限制文件句柄的生命周期
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");
            fs::set_permissions(&script_file_path, fs::Permissions::from_mode(0o755)).unwrap();

            let mut exclude_file = File::create(&exclude_file_path)
                .expect("failed to create tmp file");
            fs::set_permissions(&exclude_file_path, fs::Permissions::from_mode(0o755)).unwrap();

            // 遍历传递给脚本的命令行参数，如果遇到 --exclude-from 选项，就打印出紧随其后的文件内容。
            let cmd_script_content = r#"#!/bin/sh
#echo "$@"
while [[ $# -gt 0 ]]; do
if [[ "$1" = "--exclude-from" ]]; then
	cat "$2"
	shift
fi
shift
done
"#;
            script_file.write_all(cmd_script_content.as_bytes()).expect("failed to write to tmp file");
            exclude_file.write_all("__some_pattern".as_bytes()).expect("failed to write to tmp file");
        }
        
        let g = Config{
            global: GlobalConfig{
                retry: Some(2),
                ..GlobalConfig::default()
            },
            docker: DockerConfig{
                enable: Some(true),
                volumes: vec![format!("{}:/bin/myrsync", script_file_path.display().to_string()), 
                              "/etc/gai.conf:/etc/gai.conf:ro".into()].into(),
                ..DockerConfig::default()
            },
            ..Config::default()
        };
        let c = MirrorConfig{
            name: "rt".to_string().into(),
            provider: ProviderEnum::Rsync.into(),
            upstream: "rsync://rsync.tuna.moe/tuna/".to_string().into(),
            command: "/bin/myrsync".to_string().into(),
            exclude_file: exclude_file_path.display().to_string().into(),
            docker_image: "alpine:3.8".to_string().into(),
            log_dir: tmp_dir_path.display().to_string().into(),
            mirror_dir: tmp_dir_path.display().to_string().into(),
            use_ipv6: true.into(),
            timeout: 100.into(),
            interval: 600.into(),
            ..Default::default()
        };
        
        let mut provider = new_mirror_provider(c.clone(), g);
        assert_eq!(provider.r#type(), ProviderEnum::Rsync);
        assert_eq!(provider.name(), c.name.unwrap());
        assert_eq!(provider.working_dir(), c.mirror_dir.unwrap());
        assert_eq!(provider.log_dir(), c.log_dir.unwrap());



        let provider_name = provider.name();
        let log_dir = provider.log_dir();
        let log_file = provider.log_file();
        let working_dir = provider.working_dir();
        let context = provider.context();

        for hook in provider.hooks() {
            hook.pre_exec(provider_name.clone(),
                          log_dir.clone(),
                          log_file.clone(),
                          working_dir.clone(),
                          &context).unwrap()
        }
        provider.run(bounded(1).0).unwrap();
        for hook in provider.hooks() {
            hook.post_exec(&context, provider_name.clone()).unwrap()
        }
        println!("{}", provider.log_file());
        let logged_content = fs::read_to_string(provider.log_file()).unwrap();
        assert_eq!(logged_content, "__some_pattern".to_string());
    }
    
    
    #[test]
    fn cmd_provider_test(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let script_file_path = tmp_dir_path.join("cmd.sh");
        let mut script_file = File::create(&script_file_path)
            .expect("failed to create tmp file");
        let temp_file_path = tmp_dir_path.join("log_file");
        let mut temp_file = File::create(&temp_file_path)
            .expect("failed to create tmp file");

        let c = CmdConfig{
            name: "rt-cmd".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: format!("bash {}", script_file_path.display().to_string()),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            interval: Duration::seconds(600),
            env: [("AOSP_REPO_BIN".into(), "/usr/local/bin/repo".into())].into(),
            ..CmdConfig::default()
        };
        let provider = CmdProvider::new(c.clone()).unwrap();
        assert_eq!(provider.r#type(), ProviderEnum::Command);
        assert_eq!(provider.name(), c.name);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);

        // test_run_simple_command(provider, script_file, script_file_path);
        // test_command_fails(provider, script_file, script_file_path);
        test_killing_a_long_job(provider, script_file, script_file_path);

    }
    fn test_run_simple_command(mut provider: CmdProvider, mut script_file: File, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
echo $RTSYNC_WORKING_DIR
echo $RTSYNC_MIRROR_NAME
echo $RTSYNC_UPSTREAM_URL
echo $RTSYNC_LOG_FILE
echo $AOSP_REPO_BIN
"#;
        {
            fs::set_permissions(&script_file_path, fs::Permissions::from_mode(0o755)).unwrap();
            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
        }


        let expected_output = format!("{}\n{}\n{}\n{}\n{}\n",
                                      provider.working_dir(),
                                      provider.name(),
                                      provider.cmd_config.upstream_url, provider.log_file(),
                                      "/usr/local/bin/repo");

        let ridden_script_content = fs::read(&script_file_path).unwrap();
        assert_eq!(script_content.as_bytes(), ridden_script_content);

        provider.run(bounded(1).0).unwrap();

        let logged_content = fs::read_to_string(&provider.log_file()).unwrap();
        assert_eq!(logged_content, expected_output);
    }

    fn test_command_fails(mut provider: CmdProvider, mut script_file: File, script_file_path: PathBuf){
        let script_content = r#"exit 1"#;
        {
            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        let ridden_script_content = fs::read(&script_file_path).unwrap();
        assert_eq!(script_content.as_bytes(), ridden_script_content);

        assert!(provider.run(bounded(1).0).is_err());
    }

    // 开启一个线程执行provider.run()，在主线程将其结束
    fn test_killing_a_long_job(mut provider: CmdProvider, mut script_file: File, script_file_path: PathBuf){
        let script_content = r#"#!/bin/bash
sleep 7
			"#;
        {
            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        // let (start, receive) = bounded::<Empty>(1);
        //
        // provider.run(start).unwrap();
        //
        // thread::sleep(std::time::Duration::from_secs(1));
        // receive.recv().unwrap();
        //
        // assert_eq!(provider.is_running(), true);
        // provider.terminate().unwrap();

        let provider = Arc::new(RwLock::new(provider));
        let provider_clone = provider.clone();
        let (start, receive) = bounded::<Empty>(1);
        let handler = thread::spawn(move || {

            let mut provider = provider_clone.try_write().unwrap();
            provider.run(start).unwrap();
            drop(provider);
        });
        receive.recv().unwrap();
        // thread::sleep(std::time::Duration::from_secs(1));

        let provider = provider.try_read().unwrap();
        assert_eq!(provider.is_running(), true);
        thread::sleep(std::time::Duration::from_secs(1));
        drop(provider);
        // assert_eq!(provider.try_read().unwrap().is_running(), true);
        // let provider = provider.try_write().unwrap();
        // provider.terminate().unwrap();

        handler.join().unwrap();
    }

    #[test]
    fn test_cmd_provider_without_log_file() {
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();

        let c = CmdConfig{
            name: "run-ls".into(),
            upstream_url: "http://mirrors.tuna.moe/".into(),
            command: "ls".into(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: "/dev/null".to_string(),
            interval: Duration::seconds(600),
            ..CmdConfig::default()
        };

        let mut provider = CmdProvider::new(c.clone()).unwrap();
        assert_eq!(provider.is_master(), false);
        assert!(provider.zfs().is_none());
        assert_eq!(provider.r#type(), ProviderEnum::Command);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);

        provider.run(bounded(1).0).unwrap();
    }

    #[test]
    fn test_cmd_provider_with_reg_exprs(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let temp_file_path = tmp_dir_path.join("log_file");

        let c = CmdConfig{
            name: "run-uptime".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            command: "uptime".to_string(),
            fail_on_match: "".to_string(),
            size_pattern: "".to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            interval: Duration::seconds(600),
            ..CmdConfig::default()
        };
        test_fail_on_match_regex_matches(c.clone());
        test_fail_on_match_regex_does_not_matches(c.clone());
        test_fail_on_match_regex_meets_dev_null(c.clone());
        test_size_pattern_regex_matches(c.clone());
        test_size_pattern_regex_does_not_matches(c.clone());
        test_size_pattern_regex_meets_dev_null(c.clone());
    }
    fn test_fail_on_match_regex_matches(mut c: CmdConfig) {
        c.fail_on_match = "[a-z]+".to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        assert!(provider.run(bounded(1).0).is_err());
        assert_eq!(provider.data_size(), "");
    }

    fn test_fail_on_match_regex_does_not_matches(mut c: CmdConfig) {
        c.fail_on_match = "load average_".to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        provider.run(bounded(1).0).unwrap();
    }

    fn test_fail_on_match_regex_meets_dev_null(mut c: CmdConfig) {
        c.fail_on_match = "load average".to_string();
        c.log_file = "/dev/null".to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        assert!(provider.run(bounded(1).0).is_err());
    }

    fn test_size_pattern_regex_matches(mut c: CmdConfig) {
        c.size_pattern = r#"load averages: ([\d\.]+)"#.to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        provider.run(bounded(1).0).unwrap();

        assert!(!provider.data_size().is_empty());
        let datasize = provider.data_size().parse::<f32>().unwrap();
    }

    fn test_size_pattern_regex_does_not_matches(mut c: CmdConfig) {
        c.size_pattern = r#"load ave: ([\d\.]+)"#.to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        provider.run(bounded(1).0).unwrap();

        assert!(provider.data_size().is_empty());
    }

    fn test_size_pattern_regex_meets_dev_null(mut c: CmdConfig) {
        c.size_pattern = r#"load ave: ([\d\.]+)"#.to_string();
        c.log_file = "/dev/null".to_string();
        let mut provider = CmdProvider::new(c).unwrap();
        // FIXME: 源代码里判断run失败，但是run里在解析size_pattern时忽略了find_all_submatches_in_file返回的错误，所以不应该为失败
        assert!(provider.run(bounded(1).0).is_ok());
        assert!(provider.data_size().is_empty());
    }

    #[test]
    fn test_two_stage_rsync_provider(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let script_file_path = tmp_dir_path.join("myrsync");
        let temp_file_path = tmp_dir_path.join("log_file");

        let c = TwoStageRsyncConfig{
            name: "rt-two-stage-rsync".to_string(),
            upstream_url: "rsync://mirrors.tuna.moe/".to_string(),
            stage1_profile: "debian".to_string(),
            rsync_cmd: script_file_path.display().to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            use_ipv6: true,
            exclude_file: temp_file_path.display().to_string(),
            rsync_timeout_value: 30,
            extra_options: vec!["--delete-excluded".into(), "--cache".into()],
            username: "hello".to_string(),
            password: "world".to_string(),
            ..TwoStageRsyncConfig::default()
        };

        let mut provider = TwoStageRsyncProvider::new(c.clone()).unwrap();
        assert_eq!(provider.r#type(), ProviderEnum::TwoStageRsync);
        assert_eq!(provider.name(), c.name);
        assert_eq!(provider.working_dir(), c.working_dir);
        assert_eq!(provider.log_dir(), c.log_dir);
        assert_eq!(provider.log_file(), c.log_file);
        assert_eq!(provider.interval(), c.interval);

        // test_a_command(provider, script_file_path)
        test_terminating(provider, script_file_path)
    }

    fn test_a_command(mut provider: TwoStageRsyncProvider, script_file_path: PathBuf) {
        let script_content = r#"#!/bin/bash
echo "syncing to $(pwd)"
echo $@
sleep 1
echo "Done"
exit 0
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }

        provider.run(bounded(2).0).unwrap();

        let target_dir = resolve_symlink(PathBuf::from(provider.working_dir())).unwrap();

        let s1 = "-aHvh --no-o --no-g --stats --filter risk .~tmp~/ --exclude .~tmp~/ --safe-links ";
        let s2 = "--include=*.diff/ --include=by-hash/ --exclude=*.diff/Index --exclude=Contents* --exclude=Packages* --exclude=Sources* --exclude=Release* --exclude=InRelease --exclude=i18n/* --exclude=dep11/* --exclude=installer-*/current --exclude=ls-lR* --timeout=30 -6 ";
        let s3 = "--exclude-from ";
        let s4 = "-aHvh --no-o --no-g --stats --filter risk .~tmp~/ --exclude .~tmp~/ ";
        let s5 = "--delete --delete-after --delay-updates --safe-links ";
        let s6 = "--delete-excluded --cache --timeout=30 -6 --exclude-from ";
        let expected_output = format!("syncing to {}\n{}\nDone\nsyncing to {}\n{}\nDone\n",
                                      target_dir.display().to_string(),
                                      format!("{s1}{s2}{s3}{} {} {}",
                                              provider.two_stage_rsync_config.exclude_file, 
                                              provider.two_stage_rsync_config.upstream_url, 
                                              provider.working_dir()), 
                                      target_dir.display().to_string(), 
                                      format!("{s4}{s5}{s6}{} {} {}",
                                              provider.two_stage_rsync_config.exclude_file,
                                              provider.two_stage_rsync_config.upstream_url,
                                              provider.working_dir()));
        
        let logged_content = fs::read_to_string(provider.log_file()).expect("failed to read logged file");
        assert_eq!(logged_content, expected_output);
    }

    fn test_terminating(mut provider: TwoStageRsyncProvider, script_file_path: PathBuf) {
        let script_content = r#"#!/bin/bash
echo $@
sleep 10
exit 0
			"#;
        {
            let mut script_file = File::create(&script_file_path)
                .expect("failed to create tmp file");

            script_file.write_all(script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&script_file_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }
    }
    
    
    #[test]
    fn test_rsync_program_fails(){
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let temp_file_path = tmp_dir_path.join("log_file");
        
        let c = TwoStageRsyncConfig{
            name: "rt-two-stage-rsync".to_string(),
            upstream_url: "rsync://0.0.0.1/".to_string(),
            stage1_profile: "debian".to_string(),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            exclude_file:temp_file_path.display().to_string(),
            ..TwoStageRsyncConfig::default()
        };
        
        let mut provider = TwoStageRsyncProvider::new(c.clone()).unwrap();
        
        assert!(provider.run(bounded(2).0).is_err());
        
        let logged_content = fs::read_to_string(provider.log_file()).expect("failed to read logged file");
        assert!(logged_content.contains("Error in socket I/O"))
    }
}











