use std::cell::{Ref, RefCell};
use std::str::FromStr;
// use cgroups_rs::Cgroup;

#[derive(Debug)]
enum ConfigError {
    IoError(std::io::Error),
    TomlError(toml::de::Error),
    // Add other error types as needed
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
enum ProviderEnum {
    Rsync,
    TwoStageRsync,
    Command,
}

use serde::de::{self, Deserializer};
// 自定义反序列化函数
fn deserialize_provider_enum<'de, D>(deserializer: D) -> Result<Option<ProviderEnum>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s.as_deref() {
        Some("command") => Ok(Some(ProviderEnum::Command)),
        Some("rsync") => Ok(Some(ProviderEnum::Rsync)),
        Some("two-stage-rsync") => Ok(Some(ProviderEnum::TwoStageRsync)),
        None => Ok(None),
        _ => Err(de::Error::custom("invalid provider value")),
    }
}

//Config代表worker配置选项
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Config {
    global: GlobalConfig,
    manager: ManagerConfig,
    server: ServerConfig,
    c_group: CGroupConfig,
    zfs: ZFSConfig,
    btrfs_snapshot: BtrfsSnapshotConfig,
    docker: DockerConfig,
    include: IncludeConfig,
    mirrors_config: Vec<MirrorConfig>,
    mirrors: Vec<MirrorConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GlobalConfig {
    name: Option<String>,
    log_dir: Option<String>,
    mirror_dir: Option<String>,
    concurrent: Option<usize>,
    interval: Option<usize>,
    retry: Option<usize>,
    timeout: Option<usize>,

    exec_on_success: Option<Vec<String>>,
    exec_on_failure: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ManagerConfig{
    api_base: Option<String>,
    //该选项覆盖APIBase
    api_list: Option<Vec<String>>,
    ca_cert: Option<String>,
    // Token: String   绑定 worker.conf 文件的"token"
    token: Option<String>,
}
impl ManagerConfig {
    //获取api_list，如果为空，就获取api_base，将其放入vec中返回
    fn api_base_list(&self) -> Vec<String> {
        if let Some(apis) = &self.api_list {
            // 如果 api_list 不为空，直接返回它的克隆
            apis.clone()
        } else {
            // 如果 api_list 为空，则返回 api_base 的值（如果存在）
            self.api_base
                .as_ref()
                .map_or_else(Vec::new, |base| vec![base.clone()])
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ServerConfig {
    hostname: Option<String>,
    listen_addr: Option<String>,
    listen_port: Option<usize>,
    ssl_cert: Option<String>,
    ssl_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CGroupConfig {
    enabled: Option<bool>,
    base_path: Option<String>,
    group: Option<String>,
    sub_system: Option<String>,
    is_unified: Option<bool>,
    // cg_mgr_v1: cgroups_rs::Cgroup,
    // cg_mgr_v2: cgroups_rs::Cgroup,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZFSConfig {
    enable: Option<bool>,
    z_pool: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BtrfsSnapshotConfig {
    enable: Option<bool>,
    snapshot_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DockerConfig {
    enable: Option<bool>,
    volume: Option<Vec<String>>,
    options: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IncludeConfig {
    include_mirrors: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IncludeMirrorConfig {
    mirrors: Vec<MirrorConfig>,
}


#[derive(Debug, Default, Deserialize, Clone)]
struct MemBytes(i64);
fn memory_limit_default() -> Option<MemBytes> {
    Some(MemBytes(0))
}
// 自定义反序列化函数
fn deserialize_mem_bytes<'de, D>(deserializer: D) -> Result<Option<MemBytes>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s.as_deref() {
        Some(raw) => {
            // 这里不用检查字符串是否为空，如果toml文件里等号右边是""，解析结果会是None
            // 因为MemBytes作为成员变量使用了Option包裹

            // 传入值为负值时的处理
            if raw.trim().starts_with("-") {
                return Ok(None);
            }

            let mut result = None;

            // 原始字符串不带单位
            if let Ok(result) = raw.parse::<i64>() {
                return Ok(Some(MemBytes(result)));
            }
            // 原始字符串带单位
            let trimmed = raw.trim().to_lowercase();
            let suffixes = [
                ("b", 1),
                ("k", 1 << 10), ("kb", 1 << 10),
                ("m", 1 << 20), ("mb", 1 << 20), ("mib", 1 << 20),
                ("g", 1 << 30), ("gb", 1 << 30), ("gib", 1 << 30),
                ("t", 1 << 40), ("tb", 1 << 40), ("tib", 1 << 40),
                ("p", 1 << 50), ("pb", 1 << 50), ("pib", 1 << 50),
            ];
            for (suffix, multiplier) in &suffixes {
                if trimmed.ends_with(suffix) {
                    let number_str = &trimmed[..trimmed.len() - suffix.len()];
                    match u64::from_str(number_str) {
                        Ok(value) if value <= i64::MAX as u64 => {
                            result = Some(MemBytes((value * multiplier) as i64))
                        },
                        Err(_) => {},   // 应该不会解析错误
                        _ => {
                            result = None;  // 值超出i64的范围
                        },
                    }
                }
            }

            Ok(result)
        },
        None => Ok(None),
    }
}

use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
use serde::Deserialize;

use merge::Merge;
#[derive(Debug, Deserialize, Default, Merge, Clone)]
#[serde(default)]
struct MirrorConfig {
    name: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_enum")]
    provider: Option<ProviderEnum>,
    upstream: Option<String>,
    interval: Option<u64>,
    retry: Option<u64>,
    timeout: Option<u64>,
    mirror_dir: Option<String>,
    mirror_sub_dir: Option<String>,
    log_dir: Option<String>,
    env: Option<HashMap<String, String>>,
    role: Option<String>,

    //这两个选项覆盖全局选项
    exec_on_success: Option<Vec<String>>,
    exec_on_failure: Option<Vec<String>>,

    //
    exec_on_success_extra: Option<Vec<String>>,
    exec_on_failure_extra: Option<Vec<String>>,

    command: Option<Vec<String>>,
    fail_on_match: Option<String>,
    size_pattern: Option<String>,
    use_ipv4: Option<bool>,
    use_ipv6: Option<bool>,
    exclude_file: Option<String>,
    username: Option<String>,
    password: Option<String>,
    rsync_no_timeo: Option<bool>,
    rsync_timeout: Option<i32>,
    rsync_options: Option<Vec<String>>,
    rsync_override: Option<Vec<String>>,
    stage_1_profile: Option<String>,

    #[serde(deserialize_with = "deserialize_mem_bytes", default = "memory_limit_default")]
    memory_limit: Option<MemBytes>,

    docker_image: Option<String>,
    docker_volumes: Option<Vec<String>>,
    docker_options: Option<Vec<String>>,

    snapshot_path: Option<String>,

    #[serde(rename = "mirrors")]
    child_mirrors: Option<Vec<MirrorConfig>>
}

impl MirrorConfig {
    
}

use glob::glob;
// load_config加载配置
fn load_config(cfg_file: Option<&str>) -> Result<Config, Box<dyn std::error::Error>> {
    let mut cfg = Config::default();

    // 使用配置文件初始化Config实例
    if let Some(file) = cfg_file {
        // fs::metadata(file)?;  // 检查配置文件是否存在
        let config_contents = fs::read_to_string(file)?;
        cfg = toml::de::from_str(&config_contents)?;
    }

    // 如果有下层镜像，提取其文件所在位置，解析并插入cfg
    let include_mirrors = cfg.include.include_mirrors.clone();
    if include_mirrors.as_ref().is_some_and(|mirror_paths| !mirror_paths.is_empty()) {
        let include_files = glob(&*include_mirrors.unwrap())?;
        let mut inc_mir_cfg = IncludeMirrorConfig::default();
        for file in include_files {
            let config_contents = fs::read_to_string(file.unwrap())?;
            inc_mir_cfg = toml::de::from_str(&config_contents)?;
            cfg.mirrors_config.append(&mut inc_mir_cfg.mirrors);
        }
    }
    let mirrors_config = cfg.mirrors_config.clone();

    for m in mirrors_config{
        recursive_mirrors(&mut cfg, Rc::new(RefCell::new(None)), m)?
    }
    println!("{:#?}",cfg.mirrors);
    
    Ok(cfg)
}


fn recursive_mirrors(cfg: &mut Config, parent: Rc<RefCell<Option<MirrorConfig>>>, mirror: MirrorConfig) -> Result<(), Box<dyn std::error::Error>> {
    let cur_mir = match &*parent.borrow() {
        Some(mirror_config) => parent.clone(),
        None => Rc::new(RefCell::new(Some(MirrorConfig::default()))),
    };
    if let Some(ref mut mirror_config) = *cur_mir.borrow_mut() {
        // 初始化 child_mirrors 为 None
        mirror_config.child_mirrors = None;
        // 合并 mirror 到 cur_mir，仅补cur_mir中的空值（如果mirror中对应字段有值）
        mirror_config.merge(mirror.clone());
    } 
    
    // 检查是否有子镜像
    if let Some(child_mirrors) = mirror.child_mirrors {
        for child_mir in child_mirrors {
            // 递归处理每个子镜像
            recursive_mirrors(cfg, Rc::clone(&cur_mir), child_mir)?;
        }
    } else {
        // 如果没有子镜像，添加当前镜像到配置中
        cfg.mirrors.push(cur_mir.clone().take().unwrap());
    }

    Ok(())
}



#[cfg(test)]
mod tests {
    use std::fs::File;
    use super::*;
    
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
        assert_eq!(m.stage_1_profile, Some("debian".to_string()));

        let m = cfg.mirrors[4].clone();
        assert_eq!(m.name, Some("ubuntu".to_string()));
        assert_eq!(m.mirror_dir, None);
        assert_eq!(m.provider, Some(TwoStageRsync));
        assert_eq!(m.use_ipv6, Some(true));
        assert_eq!(m.stage_1_profile, Some("ubuntu".to_string()));

        let m = cfg.mirrors[5].clone();
        assert_eq!(m.name, Some("debian-cd".to_string()));
        assert_eq!(m.use_ipv6, Some(true));
        assert_eq!(m.provider, Some(Rsync));

        assert_eq!(cfg.mirrors.len(), 6);
    }

}

