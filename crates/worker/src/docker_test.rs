#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use crate::docker::*;
    use std::process::Command;
    use std::sync::{Arc, RwLock};
    use chrono::Duration;
    use tempfile::Builder;
    use tokio::sync::mpsc::channel;
    use crate::cmd_provider::{CmdConfig, CmdProvider};
    use crate::common::Empty;
    use crate::config::MemBytes;
    use crate::hooks::{HookType, JobHook};
    use crate::provider::MirrorProvider;

    fn cmd_run(p: String, args: Vec<&str>) {
        match Command::new(p).args(args).output(){
            Ok(output) => {
                println!("cmd启动, 标准输出：\n{} 标准错误：\n{} ",
                         std::str::from_utf8(&output.stdout).unwrap(),
                         std::str::from_utf8(&output.stderr).unwrap());
            }
            Err(e) =>{
                eprintln!("cmd启动失败：{}", e);
                return;
            }
        }
    }

    fn get_docker_by_name(name: String) -> Result<String, Box<dyn std::error::Error>>{
        println!("debug: name是{name}");
        match Command::new("docker")
            // .arg("docker")
            .arg("ps").arg("-a")
            .arg("--filter").arg(format!("name={}", name))
            .arg("--format").arg("{{.Names}}")
            .output()
        {
            Ok(output) => {
                println!("debug: output是{output:?}");
                Ok(std::str::from_utf8(&output.stdout).unwrap().into())
            }
            Err(e) =>{
                Err(e.into())
            }
        }
    }

    #[tokio::test]
    async fn test_docker() {
        let tmp_dir = Builder::new()
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let cmd_script_path = tmp_dir_path.join("cmd.sh");
        let temp_file_path = tmp_dir_path.join("log_file");

        let expect_output = "HELLO_WORLD".to_string();

        let c = CmdConfig{
            name: "rt-docker".to_string(),
            upstream_url: "http://mirrors.tuna.moe/".to_string(),
            // 在docker里运行脚本最好显示加上sh
            command: format!("sh {}", "/bin/cmd.sh"),
            working_dir: tmp_dir_path.display().to_string(),
            log_dir: tmp_dir_path.display().to_string(),
            log_file: temp_file_path.display().to_string(),
            interval: Duration::seconds(600),
            env: [("TEST_CONTENT".to_string(), expect_output.clone())].into(),
            ..CmdConfig::default()
        };



        let cmd_script_content = r#"
#!/bin/sh
echo ${TEST_CONTENT}
sleep 30
"#;

        {
            let mut script_file = File::create(&cmd_script_path)
                .expect("failed to create tmp file");
            script_file.write_all(cmd_script_content.as_bytes()).expect("failed to write to tmp file");
            fs::metadata(&cmd_script_path).expect("failed to get metadata")
                .permissions().set_mode(0o755);
        }


        let mut provider = CmdProvider::new(c.clone()).await.unwrap();

        let d = DockerHook{
            image: "alpine:3.8".to_string(),
            volumes: vec![format!("{}:/bin/cmd.sh", cmd_script_path.as_path().display().to_string())].into(),
            memory_limit: MemBytes(512*1024_i64.pow(2)),   //512 MiB
            ..DockerHook::default()
        };
        let d = HookType::Docker(d);
        provider.add_hook(d).await;
        assert!(provider.docker().await.is_some());

        // 😥
        provider.docker().await.as_ref().clone().unwrap()
            .pre_exec(provider.name().await,
                      provider.log_dir().await,
                      provider.log_file().await,
                      provider.working_dir().await,
                      provider.context().await).await
            .unwrap();

        cmd_run("docker".to_string(), vec!["images"]);
        let (exit_err_tx, mut exit_err_rx) = channel::<String>(1);

        let provider_clone = provider.clone();
        let handler = tokio::spawn(async move {
            let mut provider = provider_clone;
            if let Err(e) = provider.run(channel(1).0).await {
                println!("provider.run() 错误: {e}");
                exit_err_tx.send(e.to_string()).await.unwrap();
            }else {
                exit_err_tx.send("".to_string()).await.unwrap();
            }
            println!("provider.run() 退出了"); 
        });

        // Wait for docker running
        for _ in 0..8{
            // 😅
            let names = get_docker_by_name(provider.docker().await.as_ref().clone().unwrap().name(provider.name().await)).unwrap();
            if !names.len() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        // assert container running
        let names = get_docker_by_name(provider.docker().await.as_ref().clone().unwrap().name(provider.name().await)).unwrap();
        assert_eq!(names, format!("{}\n", provider.docker().await.as_ref().clone().unwrap().name(provider.name().await)));

        provider.terminate().await.unwrap();
        exit_err_rx.recv().await.unwrap();

        let names = get_docker_by_name(provider.docker().await.as_ref().clone().unwrap().name(provider.name().await)).unwrap();
        assert!(names.eq(""));

        // check log content
        let logged_content = fs::read_to_string(provider.log_file().await).unwrap();
        assert_eq!(logged_content, format!("{}\n", expect_output));

        provider.docker().await.as_ref().clone().unwrap()
            .post_exec(provider.context().await,
                       provider.name().await).await
            .unwrap();
    }

    #[test]
    fn t(){
        let output = Command::new("docker")
            .args(vec!["run", "--name", "rt-docker", "-u", "501:20", "-v", "/Users/xiaoyh/Documents/Code/测试/test_ruts/11/hello/tests/cmd.sh:/bin/cmd.sh", "-e", "TEST_CONTENT=HELLO_WORLD", "-m", "536870912b", "alpine:3.8", "/bin/cmd.sh"])
            .output().unwrap();
        println!("{}", std::str::from_utf8(&output.stdout).unwrap());
    }
}