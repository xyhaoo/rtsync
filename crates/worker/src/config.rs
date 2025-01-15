use std::cell::RefCell;
use std::str::FromStr;
use anyhow::Result;

#[derive(Debug, PartialEq, Deserialize, Clone, Eq)]
pub enum ProviderEnum {
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
#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub(crate) global: GlobalConfig,
    pub(crate) manager: ManagerConfig,
    pub(crate) server: ServerConfig,
    pub(crate) c_group: CGroupConfig,
    pub(crate) zfs: ZFSConfig,
    pub(crate) btrfs_snapshot: BtrfsSnapshotConfig,
    pub(crate) docker: DockerConfig,
    pub(crate) include: IncludeConfig,
    #[serde(rename = "mirrors")]
    pub(crate) mirrors_config: Vec<MirrorConfig>,
    pub mirrors: Vec<MirrorConfig>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct GlobalConfig {
    pub(crate) name: Option<String>,
    pub(crate) log_dir: Option<String>,
    pub(crate) mirror_dir: Option<String>,
    pub(crate) concurrent: Option<u64>,
    pub(crate) interval: Option<i64>,
    pub(crate) retry: Option<i64>,
    pub(crate) timeout: Option<i64>,

    pub(crate) exec_on_success: Option<Vec<String>>,
    pub(crate) exec_on_failure: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct ManagerConfig{
    pub(crate) api_base: Option<String>,
    //该选项覆盖APIBase
    pub(crate) api_list: Option<Vec<String>>,
    pub(crate) ca_cert: Option<String>,
    // token: Option<String>,
}
impl ManagerConfig {
    //获取api_list，如果为空，就获取api_base，将其放入vec中返回
    pub(crate) fn api_base_list(&self) -> Vec<String> {
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

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct ServerConfig {
    pub(crate) hostname: Option<String>,
    pub(crate) listen_addr: Option<String>,
    pub(crate) listen_port: Option<usize>,
    pub(crate) ssl_cert: Option<String>,
    pub(crate) ssl_key: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct CGroupConfig {
    pub(crate) enable: Option<bool>,
    base_path: Option<String>,
    pub(crate) group: Option<String>,
    #[serde(rename = "subsystem")]
    sub_system: Option<String>,
    #[serde(rename = "isUnified")]
    pub(crate) is_unified: Option<bool>,
    #[serde(rename = "cgMgrV1")]
    cg_mgr_v1: Option<String>,
    #[serde(rename = "cgMgrV2")]
    pub(crate) cg_mgr_v2: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct ZFSConfig {
    pub(crate) enable: Option<bool>,
    #[serde(rename = "zpool")]
    pub(crate) z_pool: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct BtrfsSnapshotConfig {
    pub(crate) enable: Option<bool>,
    pub(crate) snapshot_path: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct DockerConfig {
    pub(crate) enable: Option<bool>,
    pub(crate) volumes: Option<Vec<String>>,
    pub(crate) options: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub(crate) struct IncludeConfig {
    include_mirrors: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IncludeMirrorConfig {
    mirrors: Vec<MirrorConfig>,
}


#[derive(Debug, Default, Deserialize, Clone, Eq, PartialEq)]
pub struct MemBytes(pub(crate) i64);
impl MemBytes {
    #[warn(dead_code)]
    fn r#type(&self) -> String{
        "bytes".to_string()
    }
    pub(crate) fn value(&self) -> i64{
        self.0
    }
}
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
                ("k", 10u64.pow(3)), ("kb", 10u64.pow(3)), ("kib", 1 << 10),
                ("m", 10u64.pow(6)), ("mb", 10u64.pow(6)), ("mib", 1 << 20),
                ("g", 10u64.pow(9)), ("gb", 10u64.pow(9)), ("gib", 1 << 30),
                ("t", 10u64.pow(12)), ("tb", 10u64.pow(12)), ("tib", 1 << 40),
                ("p", 10u64.pow(15)), ("pb", 10u64.pow(15)), ("pib", 1 << 50),
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

#[derive(Debug, Deserialize, Default, Clone, Eq, PartialEq)]
#[serde(default)]
pub struct MirrorConfig {
    pub(crate) name: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_enum")]
    pub(crate) provider: Option<ProviderEnum>,
    pub(crate) upstream: Option<String>,
    pub(crate) interval: Option<i64>,
    pub(crate) retry: Option<i64>,
    pub(crate) timeout: Option<i64>,
    pub(crate) mirror_dir: Option<String>,
    #[serde(rename = "mirror_subdir")]
    pub(crate) mirror_sub_dir: Option<String>,
    pub(crate) log_dir: Option<String>,
    pub(crate) env: Option<HashMap<String, String>>,
    pub(crate) role: Option<String>,

    //这两个选项覆盖全局选项
    pub(crate) exec_on_success: Option<Vec<String>>,
    pub(crate) exec_on_failure: Option<Vec<String>>,

    //
    pub(crate) exec_on_success_extra: Option<Vec<String>>,
    pub(crate) exec_on_failure_extra: Option<Vec<String>>,

    pub(crate) command: Option<String>,
    pub(crate) fail_on_match: Option<String>,
    pub(crate) size_pattern: Option<String>,
    pub(crate) use_ipv4: Option<bool>,
    pub(crate) use_ipv6: Option<bool>,
    pub(crate) exclude_file: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    #[serde(rename = "rsync_no_timeout")]
    pub(crate) rsync_no_timeo: Option<bool>,
    pub(crate) rsync_timeout: Option<i64>,
    pub(crate) rsync_options: Option<Vec<String>>,
    pub(crate) rsync_override: Option<Vec<String>>,
    pub(crate) stage1_profile: Option<String>,

    #[serde(deserialize_with = "deserialize_mem_bytes", default = "memory_limit_default")]
    pub(crate) memory_limit: Option<MemBytes>,

    pub(crate) docker_image: Option<String>,
    pub(crate) docker_volumes: Option<Vec<String>>,
    pub(crate) docker_options: Option<Vec<String>>,

    pub(crate) snapshot_path: Option<String>,

    #[serde(rename = "mirrors")]
    pub(crate) child_mirrors: Option<Vec<MirrorConfig>>
}

macro_rules! merge {
    ($self:ident, $other:ident, { $($field:ident),* }) => {
        $(
            $self.$field = $other.$field.or($self.$field.take());
        )*
    };
}


impl MirrorConfig {
    
    fn merge(&mut self, other: Self) {
        merge!(self, other, {
            name, provider, upstream, interval, retry, timeout, mirror_dir, 
            mirror_sub_dir, log_dir, env, role,
            exec_on_success, exec_on_failure, exec_on_success_extra, exec_on_failure_extra,
            command, fail_on_match, size_pattern, use_ipv4, use_ipv6, exclude_file,
            username, password, rsync_no_timeo, rsync_timeout, rsync_options, rsync_override,
            stage1_profile, memory_limit, 
            docker_image, docker_volumes, docker_options, snapshot_path, child_mirrors
        });
    }
}


use glob::glob;
// load_config加载配置
pub fn load_config(cfg_file: &str) -> Result<Config> {
    // 使用配置文件初始化Config实例
    // fs::metadata(file)?;  // 检查配置文件是否存在
    let config_contents = fs::read_to_string(cfg_file)?;
    let mut cfg: Config = toml::de::from_str(&config_contents)?;

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
    Ok(cfg)
}

// 依据mirror_config配置cfg的mirror字段
fn recursive_mirrors(cfg: &mut Config, parent: Rc<RefCell<Option<MirrorConfig>>>, mirror: MirrorConfig) -> Result<()> {
    let cur_mir = match &*parent.borrow() {
        Some(_mirror_config) => parent.clone(),
        None => Rc::new(RefCell::new(Some(MirrorConfig::default()))),
    };
    if let Some(ref mut mirror_config) = *cur_mir.borrow_mut() {
        // 初始化 child_mirrors 为 None
        mirror_config.child_mirrors = None;
        // 合并 mirror 到 cur_mir，用mirror中的全部非空字段重写cur_mir
        mirror_config.merge(mirror.clone());
        
    };
    
    // 检查是否有子镜像
    if let Some(child_mirrors) = mirror.child_mirrors {
        for child_mir in child_mirrors {
            // 递归处理每个子镜像
            // 当child_mirrors内有多个元素，即存在一个以上的同级镜像时
            // 应确保处理第一个镜像的操作不会影响处理第二个镜像时传入的parent
            // 所以应该在递归开始前拷贝cur_mir，传入复制体
            // recursive_mirrors(cfg, Rc::clone(&cur_mir), child_mir)?;
            let parent_copy = Rc::new(RefCell::new(cur_mir.borrow().clone())); 
            recursive_mirrors(cfg, parent_copy, child_mir)?;
        }
    } else {
        // 如果没有子镜像，添加当前镜像到配置中
        cfg.mirrors.push(cur_mir.take().unwrap());
    }
    Ok(())
}





