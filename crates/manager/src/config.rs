use std::{fmt, fs};
use clap::builder::Str;
use serde::de::Error;
use serde::Deserialize;
use crate::{Cli};


#[derive(Debug)]
enum ConfigError {
    IoError(std::io::Error),
    TomlError(toml::de::Error),
    // Add other error types as needed
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::IoError(err) => write!(f, "IO error: {}", err),
            ConfigError::TomlError(err) => write!(f, "TOML error: {}", err),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> ConfigError {
        ConfigError::IoError(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> ConfigError {
        ConfigError::TomlError(err)
    }
}

// Config是顶级的可序列化配置结构
#[derive(Debug, Default, Deserialize, Clone)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) debug: bool,
    server: ServerConfig,
    pub(crate) files: FileConfig,
}

// ServerConfig表示HTTP服务器的配置
#[derive(Debug, Default, Deserialize, Clone)]
struct ServerConfig {
    #[serde(default)]
    addr: Option<String>,
    #[serde(default)]
    port: Option<u32>,
    #[serde(default)]
    ssl_cert: Option<String>,
    #[serde(default)]
    ssl_key: Option<String>,
}

// FileConfig包含特殊文件的路径
#[derive(Debug, Default, Deserialize, Clone)]
pub(crate) struct  FileConfig {
    #[serde(default)]
    status_file: Option<String>,
    #[serde(default)]
    pub(crate) db_file: Option<String>,
    #[serde(default)]
    pub(crate) db_type: Option<String>,
    #[serde(default)]
    pub(crate) ca_cert: Option<String>,
}

// LoadConfig从指定文件加载配置
fn load_config(cfg_file: Option<String>, c:Option<Cli>)->Result<Config, ConfigError>{
    let mut cfg = Config::default();
    cfg.server.addr = Some("127.0.0.1".to_string());
    cfg.server.port = Some(14242);
    cfg.debug = false;
    cfg.files.status_file = Some("/var/lib/rtsync/rtsync.json".to_string());
    cfg.files.db_file = Some("bolt".to_string());
    
    if let Some(file) = cfg_file {
        let config_contents = fs::read_to_string(file)?;
        cfg = toml::de::from_str(&config_contents)?
    }
    
    if let Some(c) = c {
        if let Some(addr) = c.addr {
            cfg.server.addr = Some(addr);
        }
        if let Some(port) = c.port {
            if port < 65536{
                cfg.server.port = Some(port);
            }
            
        }
        if let (Some(cert),Some(key)) = (c.cert, c.key) {
            cfg.server.ssl_cert = Some(cert);
            cfg.server.ssl_key = Some(key);
        }
        if let  Some(status_file) = c.status_file {
            cfg.files.status_file = Some(status_file);
        }
        if let Some(db_file) = c.db_file {
            cfg.files.db_file = Some(db_file);
        }
        if let Some(db_type) = c.db_type {
            cfg.files.db_type = Some(db_type);
        }
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    const CFG_BLOB: &str = r#"
	debug = true
	[server]
	addr = "0.0.0.0"
	port = 5000

	[files]
	status_file = "/tmp/rtsync.json"
	db_file = "/var/lib/rtsync/rtsync.db"
	"#;
    
    // 解码toml文件，和用toml文件初始化Config对象的测试
    #[test]
    fn test_toml_decode() {
        let mut _conf = Config::default();
        
        _conf = toml::from_str(CFG_BLOB).expect("toml decode error");
        assert_eq!(_conf.server.addr.unwrap(), "0.0.0.0".to_string());
        assert_eq!(_conf.server.port.unwrap(), 5000);
        assert_eq!(_conf.files.status_file.unwrap(), "/tmp/rtsync.json".to_string());
        assert_eq!(_conf.files.db_file.unwrap(), "/var/lib/rtsync/rtsync.db".to_string());
    }


    use tempfile::Builder;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt; // 引入用于设置权限的扩展
    use clap::{Parser, Subcommand};
    use std::env;
    //使用命令行参数初始化Config的测试
    #[test]
    fn test_load_config() {
        //创建临时文件，将配置文件字符串写入文件并设置文件权限的测试
        let mut tmp_file = Builder::new()
            .prefix("rtsync")   // 文件前缀
            .tempfile().expect("failed to create tmp file");
        // 将 CFG_BLOB 写入临时文件
        writeln!(tmp_file, "{}", CFG_BLOB).expect("failed to write to tmp file");

        // 获取临时文件的路径
        let path = tmp_file.path();

        // 设置文件权限为 0644
        let mut perms = fs::metadata(path).expect("failed to get metadata").permissions();
        perms.set_mode(0o644); // 设置权限
        fs::set_permissions(path, perms).expect("failed to set permissions");


        // 用从命令行中（实例化Cli对象）读取的config（文件）和命令行中的其他参数来初始化Config struct
        //当命令中没有制定配置文件所在地址
        let args = vec![
            "test".to_string(), //第一个参数是程序名
        ];
        let cli = Cli::parse_from(args);
        let cfg_file = cli.config.clone();
        let cfg: Config = load_config(cfg_file, Some(cli)).expect("failed to create cfg when giving nothing");
        assert_eq!(cfg.server.addr.unwrap(), "127.0.0.1".to_string());

        // 当命令参数提供了配置文件的地址
        let args = vec![
            "test".to_string(), 
            "-c".to_string(), path.to_str().expect("failed to parse file path").to_string(),
        ];
        let cli = Cli::parse_from(args);
        assert!(cli.config.is_some());   //此时提供了配置文件地址， cli变量内此字段应有值
        let cfg_file = cli.config.clone();
        let cfg: Config = load_config(cfg_file, Some(cli)).expect("failed to create cfg when only giving file_path");
        assert_eq!(cfg.server.addr.unwrap(), "0.0.0.0".to_string());
        assert_eq!(cfg.server.port.unwrap(), 5000);
        assert_eq!(cfg.files.status_file.unwrap(), "/tmp/rtsync.json".to_string());
        assert_eq!(cfg.files.db_file.unwrap(), "/var/lib/rtsync/rtsync.db".to_string());
        
        // 当提供除配置文件地址外的其他命令行参数
        let args = vec![
            "test".to_string(),
            "--addr".to_string(), "0.0.0.0".to_string(),
            "--port".to_string(), "5001".to_string(),
            "--cert".to_string(), "/ssl.cert".to_string(),
            "--key".to_string(), "/ssl.key".to_string(),
            "--status-file".to_string(), "/rtsync.json".to_string(),
            "--db-file".to_string(), "/rtsync.db".to_string(),
        ];
        let cli = Cli::parse_from(args);
        assert!(cli.config.is_none());   //此时并未提供配置文件地址， cli变量内此字段应为None
        let cfg_file = cli.config.clone();
        let cfg: Config = load_config(cfg_file, Some(cli)).expect("failed to create cfg when giving options except config addr");
        assert_eq!(cfg.server.addr.unwrap(), "0.0.0.0".to_string());
        assert_eq!(cfg.server.port.unwrap(), 5001);
        assert_eq!(cfg.server.ssl_cert.unwrap(), "/ssl.cert".to_string());
        assert_eq!(cfg.server.ssl_key.unwrap(), "/ssl.key".to_string());
        assert_eq!(cfg.files.status_file.unwrap(), "/rtsync.json".to_string());
        assert_eq!(cfg.files.db_file.unwrap(), "/rtsync.db".to_string());

        // 当提供除url外的命令行参数
        let args = vec![
            "test".to_string(),
            "-c".to_string(), path.to_str().expect("failed to parse file path").to_string(),
            "--cert".to_string(), "/ssl.cert".to_string(),
            "--key".to_string(), "/ssl.key".to_string(),
            "--status-file".to_string(), "/rtsync.json".to_string(),
            "--db-file".to_string(), "/rtsync.db".to_string(),
        ];
        let cli = Cli::parse_from(args);
        assert!(cli.config.is_some());   //此时提供了配置文件地址， cli变量内此字段应有值
        let cfg_file = cli.config.clone();
        let cfg: Config = load_config(cfg_file, Some(cli)).expect("failed to create cfg when giving options except url");
        assert_eq!(cfg.server.addr.unwrap(), "0.0.0.0".to_string());
        assert_eq!(cfg.server.port.unwrap(), 5000);
        assert_eq!(cfg.server.ssl_cert.unwrap(), "/ssl.cert".to_string());
        assert_eq!(cfg.server.ssl_key.unwrap(), "/ssl.key".to_string());
        assert_eq!(cfg.files.status_file.unwrap(), "/rtsync.json".to_string());
        assert_eq!(cfg.files.db_file.unwrap(), "/rtsync.db".to_string());
    }
}
