/*
use std::collections::HashMap;
use once_cell::sync::Lazy;
use std::fs;
use std::sync::Arc;
use reqwest::{Client, Response};
use rustls::{ClientConfig, RootCertStore};
use rustls::internal::pemfile;
use tokio::time::Duration;
use std::io::{self, BufReader, ErrorKind};
use std::error::Error;
use serde::de::DeserializeOwned;
use serde::Serialize;
use regex::Regex;
use std::process::ExitStatus;


static RSYNC_EXIT_VALUES: Lazy<HashMap<i32, String>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert(0, String::from("成功。"));
    m.insert(1, String::from("语法或用法错误。"));
    m.insert(2, String::from("协议不兼容。"));
    m.insert(3, String::from("I/O文件、文件夹选择错误。"));
    m.insert(4, String::from("请求的操作不支持：试图在不支持64位文件的平台上操作64位文件；或者指定了客户端支持而服务器不支持的选项。"));
    m.insert(5, String::from("启动客户端-服务器协议错误。"));
    m.insert(6, String::from("守护进程无法追加日志文件。"));
    m.insert(10, String::from("socket I/O错误。"));
    m.insert(11, String::from("file I/O错误。"));
    m.insert(12, String::from("rsync协议数据流错误。"));
    m.insert(13, String::from("程序诊断错误。"));
    m.insert(14, String::from("IPC码错误。"));
    m.insert(20, String::from("收到SIGUSR1或SIGINT。"));
    m.insert(21, String::from("waitpid()返回了某些错误。"));
    m.insert(22, String::from("分配核心内存缓冲区错误。"));
    m.insert(23, String::from("由于错误导致了部分传输。"));
    m.insert(24, String::from("由于消失的源文件导致了部分传输。"));
    m.insert(25, String::from("-max-delete限制停止删除。"));
    m.insert(30, String::from("数据传送/接收超时。"));
    m.insert(35, String::from("等待守护进程连接超时。"));
    m
});

// get_tle_config通过ca_file生成tls.config
fn get_tls_config(ca_file: &str) -> Result<Arc<ClientConfig>, Box<dyn Error>> {
    // 读取CA文件内容
    let ca_cert = fs::read(ca_file)?;

    // 创建证书池
    let mut root_cert_store = RootCertStore::empty();
    let mut buf = BufReader::new(&ca_cert[..]);

    // 将CA证书添加到证书池中
    let certs = pemfile::certs(&mut buf)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to parse CA"))?;
    if certs.is_empty() {
        return Err(Box::new(io::Error::new(io::ErrorKind::InvalidData, "No certificates found in CA file")));
    }
    root_cert_store.add_parsable_certificates(&certs);

    // 创建并配置TLS配置
    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth(); // 无客户端证书

    Ok(Arc::new(config))
}


async fn create_http_client(ca_file: Option<&str>) -> Result<Client, Box<dyn Error>> {
    // 加载自定义TLS配置
    let mut client_builder = Client::builder()
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(20);

    if let Some(ca_path) = ca_file {
        let tls_config = get_tls_config(ca_path)?; // 使用之前定义的get_tls_config函数
        client_builder = client_builder.use_preconfigured_tls(tls_config);
    }

    let client = client_builder.build()?;
    Ok(client)
}

async fn post_json<T: Serialize>(url: &str, obj: &T, client: Option<&Client>) -> Result<Response, Box<dyn Error>> {
    let client = match client {
        Some(c) => c.clone(),
        None => create_http_client(None).await?,  // 使用之前定义的 create_http_client 函数
    };

    let resp = client
        .post(url)
        .json(obj)
        .send()
        .await?;

    Ok(resp)
}

async fn get_json<T: DeserializeOwned>(url: &str, client: Option<&Client>) -> Result<T, Box<dyn Error>> {
    let client = match client {
        Some(c) => c.clone(),
        None => create_http_client(None).await?,
    };

    let resp = client
        .get(url)
        .send()
        .await?;

    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("HTTP status code is not 200: {}", resp.status()).into());
    }

    let obj = resp.json::<T>().await?;
    Ok(obj)
}

fn find_all_submatch_in_file(file_name: &str, re: &Regex) -> Result<Vec<Vec<Vec<u8>>>, io::Error> {
    if file_name == "/dev/null" {
        return Err(io::Error::new(ErrorKind::InvalidInput, "Invalid log file"));
    }

    let content = fs::read(file_name)?;
    let mut matches = Vec::new();

    for cap in re.captures_iter(&content) {
        let submatches: Vec<Vec<u8>> = cap.iter().skip(1)
            .map(|m| m.map_or(vec![], |m| m.as_bytes().to_vec()))
            .collect();
        matches.push(submatches);
    }

    Ok(matches)
}

fn extract_size_from_log(log_file: &str, re: &Regex) -> Option<String> {
    match find_all_submatch_in_file(log_file, re) {
        Ok(matches) if !matches.is_empty() => {
            if let Some(last_match) = matches.last() {
                if last_match.len() > 1 {
                    return Some(String::from_utf8_lossy(&last_match[1]).into_owned());
                }
            }
            None
        },
        _ => None,
    }
}

fn extract_size_from_rsync_log(log_file: &str) -> Option<String> {
    let re = Regex::new(r"(?m)^Total file size: ([0-9.]+[KMGTP]?) bytes").unwrap();
    extract_size_from_log(log_file, &re)
}

fn translate_rsync_error_code(status: ExitStatus) -> (i32, String) {
    let exit_code = status.code().unwrap_or(-1);

    if let Some(err_msg) = RSYNC_EXIT_VALUES.get(&exit_code) {
        (exit_code, format!("rsync error: {}", err_msg))
    } else {
        (exit_code, String::from("Unknown rsync error"))
    }
}
 */

use std::collections::HashMap;
use std::{fs};
use std::fs::File;
use std::io::{self, Read, ErrorKind, BufReader};
use std::sync::Arc;
use reqwest::{Certificate, Client, Error};
use reqwest::header::CONTENT_TYPE;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use lazy_static::lazy_static;
use rocket::data::N;

lazy_static! {
    static ref  rsync_exit_values: HashMap<i32, &'static str> = [
        (0, "Success"),
        (1, "Syntax or usage error"),
        (2, "Protocol incompatibility"),
        (3, "Errors selecting input/output files, dirs"),
        (4, "Requested action not supported"),
        (5, "Error starting client-server protocol"),
        (6, "Daemon unable to append to log-file"),
        (10, "Error in socket I/O"),
        (11, "Error in file I/O"),
        (12, "Error in rsync protocol data stream"),
        (13, "Errors with program diagnostics"),
        (14, "Error in IPC code"),
        (20, "Received SIGUSR1 or SIGINT"),
        (21, "Some error returned by waitpid()"),
        (22, "Error allocating core memory buffers"),
        (23, "Partial transfer due to error"),
        (24, "Partial transfer due to vanished source files"),
        (25, "The --max-delete limit stopped deletions"),
        (30, "Timeout in data send/receive"),
        (35, "Timeout waiting for daemon connection"),
    ]
        .iter()
        .cloned()
        .collect();
}

pub fn get_tls_config(ca_file: &str) -> Result<Vec<Certificate>, reqwest::Error> {
    let mut buf = Vec::new();
    File::open(ca_file)
        .expect("打开文件失败")
        .read_to_end(&mut buf).expect("不能读取该文件");
    let cert = Certificate::from_pem_bundle(&buf)?;
    Ok(cert)

/*
    // 打开 CA 文件
    let ca_file = File::open(ca_file)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("Failed to open CA file: {}", e)))?;
    let mut reader = BufReader::new(ca_file);
    
    // 使用 rustls-pemfile 解析证书
    let certs = certs(&mut reader);
    
    let root = Certificate::from_pem()
    
    // 创建证书池并添加证书
    let mut root_cert_store = RootCertStore::empty();
    for cert in certs {
        root_cert_store
            // .add_parsable_certificates(cert)
            .add(cert.map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?)
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "Failed to add certificate to pool"))?;
    }
    
    // 构建 TLS 配置
    let config = ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();
        
    
    Ok(config)
*/

}

pub fn create_http_client(ca_file: Option<&str>) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder();
    if let Some(ca_file) = ca_file {
        let tls_config = get_tls_config(ca_file).map_err(|e|reqwest::Error::from(e.into()))?;

        // 设置根证书
        for cert in tls_config {
            builder = builder.add_root_certificate(cert);
        }
        
    }

    // 配置 HTTP 客户端
    let client = builder
        .pool_max_idle_per_host(20) // 设置空闲时的最大连接数
        .timeout(Duration::new(5, 0)) // 设置 5 秒超时
        .build()?;
    Ok(client)
}

// Post JSON data to a URL
pub async fn post_json<T: Serialize + Send>(url: &str, obj: &T, client: Arc<Option<Client>>) -> Result<reqwest::Response, reqwest::Error> {
    let response = match client.as_ref() {
        Some(client) => {
            client
                .post(url)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .json(obj)
                .send()
                .await?
        }
        None => {
            let client = create_http_client(None)?;
            client
                .post(url)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .json(obj)
                .send()
                .await?
        }
    };
    Ok(response)
}

// Get JSON response from a URL
pub async fn get_json<T: for<'de> Deserialize<'de>>(url: &str, client: Arc<Option<Client>>) -> Result<T, reqwest::Error> {
    match client.as_ref() {
        None => {
            let client = create_http_client(None)?;
            let resp = client.get(url).send().await?;
            resp.json::<T>().await
            
        }
        Some(client) => {
            let resp = client.get(url).send().await?;
            resp.json::<T>().await
        }
    }
}

// Extract matches from a file using regex 
pub fn find_all_submatches_in_file(file_name: &str, re: &Regex) -> Result<Vec<Vec<String>>, io::Error> {
    if file_name == "/dev/null" {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "不合法的log文件"));
    }

    let content = fs::read_to_string(file_name)?;
    println!("debug: content是: {:?}", content);
    Ok(re.captures_iter(&*content)
           .map(|cap| {
               (0..cap.len())  // 捕获组的数量
                   .map(|i| {
                       // 获取捕获组的内容，如果没有匹配则填充空字符串
                       cap.get(i).map_or("".to_string(), |m| m.as_str().to_string())
                   })
                   .collect()
           })
           .collect())
    
    /*
    "(\d+)-(\d+)-(\d+)"
    "2024-12-17\n2025-01-01\n2023-11-30";
    ["2024-12-17", "2024", "12", "17"]
    ["2025-01-01", "2025", "01", "01"]
    ["2023-11-30", "2023", "11", "30"]
     */
}

// Extract size from a log file using regex 
pub fn extract_size_from_log(log_file: &str, re: &Regex) -> Option<String> {
    match find_all_submatches_in_file(log_file, re) {
        Ok(matches) if !matches.is_empty() => { 
            println!("debug: matches: {:?}", matches);
            // 最后一个匹配项的第一个子捕获组
            matches.last().and_then(|m| m.get(1).cloned()) 
        },
        _ => None,
    }
}

// Extract size specifically from rsync log
pub fn extract_size_from_rsync_log(log_file: &str) -> Option<String> {
    let re = Regex::new(r"(?m)^Total file size: ([0-9.]+[KMGTP]?) bytes").unwrap();
    extract_size_from_log(log_file, &re)
}

// Test rsync command error handling
pub fn translate_rsync_error_code(exit_code: i32) -> Option<String>{
    if let Some(msg) = rsync_exit_values.get(&exit_code) {
        let error = format!("rsync error: {}", msg.to_string());
        // 直接返回一个&&str可能得不到结果， 而且不写return为啥不回返回。。
        // Some(msg.to_string());
        return Some(error)
    }
    None
}


mod tests{
    use std::fs::File;
    use std::io::Write;
    use tempfile::Builder;
    use super::*;
    const READ_LOG_CONTENT: &str = r#"
Number of files: 998,470 (reg: 925,484, dir: 58,892, link: 14,094)
Number of created files: 1,049 (reg: 1,049)
Number of deleted files: 1,277 (reg: 1,277)
Number of regular files transferred: 5,694
Total file size: 1.33T bytes
Total transferred file size: 2.86G bytes
Literal data: 780.62M bytes
Matched data: 2.08G bytes
File list size: 37.55M
File list generation time: 7.845 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 7.55M
Total bytes received: 823.25M

sent 7.55M bytes  received 823.25M bytes  5.11M bytes/sec
total size is 1.33T  speedup is 1,604.11
"#;
    
    #[test]
    fn test_extract_size_from_rsync_log(){
        //生成一个包含在临时目录（前缀为rtsync）中的文件rtsync
        let tmp_dir = Builder::new()    // 使用tempfile生成的临时目录
            .prefix("rtsync")
            .tempdir().expect("failed to create tmp dir");
        let tmp_dir_path = tmp_dir.path();
        let tmp_file_path = tmp_dir_path.join("rs.log");
        //使用File生成的文件，包含在临时目录内，会随其一起被删除，且文件名后面没有英文字母后缀
        let mut tmp_file = File::create(&tmp_file_path)
            .expect("failed to create tmp file");
        
        // 写入临时文件
        tmp_file.write_all(READ_LOG_CONTENT.as_bytes()).expect("failed to write to tmp file");
        
        let result = extract_size_from_rsync_log(tmp_file_path.to_str().unwrap()).unwrap();
        assert_eq!(result, "1.33T");
    }
    
}















