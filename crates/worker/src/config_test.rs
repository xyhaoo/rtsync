#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::fs::File;
    use crate::config::*;

    const CFG_BLOB: &str = r#"
[global]
name = "test_worker"
log_dir = "/var/log/rtsync/{{.Name}}"
mirror_dir = "/data/mirrors"
concurrent = 10
interval = 240
retry = 3
timeout = 86400

[manager]
api_base = "https://127.0.0.1:5000"
token = "some_token"

[server]
hostname = "worker1.example.com"
listen_addr = "127.0.0.1"
listen_port = 6000
ssl_cert = "/etc/rtsync.d/worker1.cert"
ssl_key = "/etc/rtsync.d/worker1.key"

[[mirrors]]
name = "AOSP"
provider = "command"
upstream = "https://aosp.google.com/"
interval = 720
retry = 2
timeout = 3600
mirror_dir = "/data/git/AOSP"
exec_on_success = [
	"bash -c 'echo ${RTSYNC_JOB_EXIT_STATUS} > ${RTSYNC_WORKING_DIR}/exit_status'"
]
	[mirrors.env]
	REPO = "/usr/local/bin/aosp-repo"

[[mirrors]]
name = "debian"
provider = "two-stage-rsync"
stage1_profile = "debian"
upstream = "rsync://ftp.debian.org/debian/"
use_ipv6 = true
memory_limit = "256MiB"

[[mirrors]]
name = "fedora"
provider = "rsync"
upstream = "rsync://ftp.fedoraproject.org/fedora/"
use_ipv6 = true
memory_limit = "128M"

exclude_file = "/etc/rtsync.d/fedora-exclude.txt"
exec_on_failure = [
	"bash -c 'echo ${RTSYNC_JOB_EXIT_STATUS} > ${RTSYNC_WORKING_DIR}/exit_status'"
]
    "#;


    // 当给定错误的文件地址
    #[test]
    fn test_wrong_file(){
        let cfg = load_config(Some("/path/to/invalid/file"));
        assert!(cfg.is_err());
    }

    use tempfile::{Builder, TempDir};
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use crate::config::ProviderEnum::{Command, Rsync, TwoStageRsync};

    // 当配置文件有效
    #[test]
    fn test_valid_file(){
        const INC_BLOB1: &str = r#"
[[mirrors]]
name = "debian-cd"
provider = "two-stage-rsync"
stage1_profile = "debian"
use_ipv6 = true

[[mirrors]]
name = "debian-security"
provider = "two-stage-rsync"
stage1_profile = "debian"
use_ipv6 = true
    "#;

        const INC_BLOB2: &str = r#"
[[mirrors]]
name = "ubuntu"
provider = "two-stage-rsync"
stage1_profile = "debian"
use_ipv6 = true
    "#;
        //生成一个包含在临时目录（前缀为rtsync）中的文件rtsync
        let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let tmp_file_path = tmp_dir_path.join("rtsync");
        //使用File生成的文件，包含在临时目录内，会随其一起被删除，且文件名后面没有英文字母后缀
        let mut tmp_file = File::create(&tmp_file_path)
            .expect("failed to create tmp file");

        //构建配置文件内容
        let inc_section = format!(
            "\n[include]\ninclude_mirrors = \"{}/*.conf\"",
            tmp_dir_path.display()
        );
        let cur_cfg_blob = format!("{}\n{}", CFG_BLOB, inc_section);

        // 写入临时文件
        tmp_file.write_all(cur_cfg_blob.as_bytes()).expect("failed to write to tmp file");

        // 配置debian.conf
        let file_debian = tmp_dir_path.join("debian.conf");
        fs::write(&file_debian, INC_BLOB1)
            .expect("failed to write to tmp file debian.conf");
        // 设置文件权限为 0644
        let mut perms = fs::metadata(&file_debian)
            .expect("failed to get metadata").permissions();
        perms.set_mode(0o644); // 设置权限
        fs::set_permissions(file_debian, perms).expect("failed to set permissions");

        // 配置ubuntu.conf
        let file_ubuntu = tmp_dir_path.join("ubuntu.conf");
        fs::write(&file_ubuntu, INC_BLOB2)
            .expect("failed to write to tmp file ubuntu.conf");
        let mut perms = fs::metadata(&file_ubuntu)
            .expect("failed to get metadata").permissions();
        perms.set_mode(0o644); // 设置权限
        fs::set_permissions(file_ubuntu, perms).expect("failed to set permissions");

        let cfg = load_config(Some(tmp_file_path.to_str().unwrap())).unwrap();
        assert_eq!(cfg.global.name, Some("test_worker".to_string()));
        assert_eq!(cfg.global.interval, Some(240));
        assert_eq!(cfg.global.retry, Some(3));
        assert_eq!(cfg.global.mirror_dir, Some("/data/mirrors".to_string()));

        assert_eq!(cfg.manager.api_base, Some("https://127.0.0.1:5000".to_string()));
        assert_eq!(cfg.server.hostname, Some("worker1.example.com".to_string()));

        let m = cfg.mirrors[0].clone();
        assert_eq!(m.name, Some("AOSP".to_string()));
        assert_eq!(m.mirror_dir, Some("/data/git/AOSP".to_string()));
        assert_eq!(m.provider, Some(Command));
        assert_eq!(m.interval, Some(720));
        assert_eq!(m.retry, Some(2));
        assert_eq!(*m.env.unwrap().get("REPO").unwrap(), "/usr/local/bin/aosp-repo".to_string());

        let m = cfg.mirrors[1].clone();
        assert_eq!(m.name, Some("debian".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));

        let m = cfg.mirrors[2].clone();
        assert_eq!(m.name, Some("fedora".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(Rsync));
        assert_eq!(m.exclude_file, Some("/etc/rtsync.d/fedora-exclude.txt".to_string()));

        let m = cfg.mirrors[3].clone();
        assert_eq!(m.name, Some("debian-cd".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));
        assert_eq!(m.memory_limit.unwrap().0, 0);

        let m = cfg.mirrors[4].clone();
        assert_eq!(m.name, Some("debian-security".to_string()));

        let m = cfg.mirrors[5].clone();
        assert_eq!(m.name, Some("ubuntu".to_string()));

        assert_eq!(cfg.mirrors.len(), 6);
    }

    // 当配置文件嵌套
    #[test]
    fn test_nested_file(){
        const INC_BLOB1: &str = r#"
[[mirrors]]
name = "ipv6s"
use_ipv6 = true
	[[mirrors.mirrors]]
	name = "debians"
	mirror_subdir = "debian"
	provider = "two-stage-rsync"
	stage1_profile = "debian"

		[[mirrors.mirrors.mirrors]]
		name = "debian-security"
		upstream = "rsync://test.host/debian-security/"
		[[mirrors.mirrors.mirrors]]
		name = "ubuntu"
		stage1_profile = "ubuntu"
		upstream = "rsync://test.host2/ubuntu/"
	[[mirrors.mirrors]]
	name = "debian-cd"
	provider = "rsync"
	upstream = "rsync://test.host3/debian-cd/"
        "#;

        //生成一个包含在临时目录（前缀为rtsync）中的文件rtsync
        let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let tmp_file_path = tmp_dir_path.join("rtsync");
        //使用File生成的文件，包含在临时目录内，会随其一起被删除，且文件名后面没有英文字母后缀
        let mut tmp_file = File::create(&tmp_file_path)
            .expect("failed to create tmp file");

        //构建配置文件内容
        let inc_section = format!(
            "\n[include]\ninclude_mirrors = \"{}/*.conf\"",
            tmp_dir_path.display()
        );
        let cur_cfg_blob = format!("{}\n{}", CFG_BLOB, inc_section);

        // 写入临时文件
        tmp_file.write_all(cur_cfg_blob.as_bytes()).expect("failed to write to tmp file");

        // 配置nest.conf
        let file_nest = tmp_dir_path.join("nest.conf");
        fs::write(&file_nest, INC_BLOB1)
            .expect("failed to write to tmp file nest.conf");
        // 设置文件权限为 0644
        let mut perms = fs::metadata(&file_nest)
            .expect("failed to get metadata").permissions();
        perms.set_mode(0o644); // 设置权限
        fs::set_permissions(file_nest, perms).expect("failed to set permissions");

        let cfg = load_config(Some(tmp_file_path.to_str().unwrap())).unwrap();
        assert_eq!(cfg.global.name, Some("test_worker".to_string()));
        assert_eq!(cfg.global.interval, Some(240));
        assert_eq!(cfg.global.retry, Some(3));
        assert_eq!(cfg.global.mirror_dir, Some("/data/mirrors".to_string()));

        assert_eq!(cfg.manager.api_base, Some("https://127.0.0.1:5000".to_string()));
        assert_eq!(cfg.server.hostname, Some("worker1.example.com".to_string()));

        let m = cfg.mirrors[0].clone();
        assert_eq!(m.name, Some("AOSP".to_string()));
        assert_eq!(m.mirror_dir, Some("/data/git/AOSP".to_string()));
        assert_eq!(m.provider, Some(Command));
        assert_eq!(m.interval, Some(720));
        assert_eq!(m.retry, Some(2));
        assert_eq!(*m.env.unwrap().get("REPO").unwrap(), "/usr/local/bin/aosp-repo".to_string());

        let m = cfg.mirrors[1].clone();
        assert_eq!(m.name, Some("debian".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));

        let m = cfg.mirrors[2].clone();
        assert_eq!(m.name, Some("fedora".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(Rsync));
        assert_eq!(m.exclude_file, Some("/etc/rtsync.d/fedora-exclude.txt".to_string()));

        let m = cfg.mirrors[3].clone();
        assert_eq!(m.name, Some("debian-security".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));
        assert_eq!(m.use_ipv6, Some(true));
        assert_eq!(m.stage1_profile, Some("debian".to_string()));

        let m = cfg.mirrors[4].clone();
        assert_eq!(m.name, Some("ubuntu".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));
        assert_eq!(m.use_ipv6, Some(true));
        assert_eq!(m.stage1_profile, Some("ubuntu".to_string()));

        let m = cfg.mirrors[5].clone();
        assert_eq!(m.name, Some("debian-cd".to_string()));
        assert_eq!(m.use_ipv6, Some(true));
        assert_eq!(m.provider, Some(Rsync));

        assert_eq!(cfg.mirrors.len(), 6);
    }

    use crate::provider::{new_mirror_provider, MirrorProvider};
    #[test]
    fn test_valid_provider(){
        //生成一个包含在临时目录（前缀为rtsync）中的文件rtsync
        let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let tmp_file_path = tmp_dir_path.join("rtsync");
        //使用File生成的文件，包含在临时目录内，会随其一起被删除，且文件名后面没有英文字母后缀
        let mut tmp_file = File::create(&tmp_file_path)
            .expect("failed to create tmp file");

        // 写入临时文件
        tmp_file.write_all(CFG_BLOB.as_bytes()).expect("failed to write to tmp file");
        let cfg = load_config(Some(tmp_file_path.to_str().unwrap())).unwrap();

        let mut providers: HashMap<String, Box<dyn MirrorProvider>> = HashMap::new();
        for m in &cfg.mirrors {
            let p = new_mirror_provider(m.clone(), cfg.clone());
            providers.insert(p.name(), p);
        }
        let p = providers.get("AOSP").unwrap();
        assert_eq!(p.name(), "AOSP".to_string());
        assert_eq!(p.log_dir(), "/var/log/rtsync/AOSP".to_string());
        assert_eq!(p.log_file(), "/var/log/rtsync/AOSP/latest.log".to_string())

    }
}