// use std::collections::HashMap;
// use once_cell::sync::Lazy;
// use std::fs;
// use std::sync::Arc;
// use reqwest::{Client, Response};
// use rustls::{ClientConfig, RootCertStore};
// use rustls::internal::pemfile;
// use tokio::time::Duration;
// use std::io::{self, BufReader, ErrorKind};
// use std::error::Error;
// use serde::de::DeserializeOwned;
// use serde::Serialize;
// use regex::Regex;
// use std::process::ExitStatus;
// 
// 
// static RSYNC_EXIT_VALUES: Lazy<HashMap<i32, String>> = Lazy::new(|| {
//     let mut m = HashMap::new();
//     m.insert(0, String::from("成功。"));
//     m.insert(1, String::from("语法或用法错误。"));
//     m.insert(2, String::from("协议不兼容。"));
//     m.insert(3, String::from("I/O文件、文件夹选择错误。"));
//     m.insert(4, String::from("请求的操作不支持：试图在不支持64位文件的平台上操作64位文件；或者指定了客户端支持而服务器不支持的选项。"));
//     m.insert(5, String::from("启动客户端-服务器协议错误。"));
//     m.insert(6, String::from("守护进程无法追加日志文件。"));
//     m.insert(10, String::from("socket I/O错误。"));
//     m.insert(11, String::from("file I/O错误。"));
//     m.insert(12, String::from("rsync协议数据流错误。"));
//     m.insert(13, String::from("程序诊断错误。"));
//     m.insert(14, String::from("IPC码错误。"));
//     m.insert(20, String::from("收到SIGUSR1或SIGINT。"));
//     m.insert(21, String::from("waitpid()返回了某些错误。"));
//     m.insert(22, String::from("分配核心内存缓冲区错误。"));
//     m.insert(23, String::from("由于错误导致了部分传输。"));
//     m.insert(24, String::from("由于消失的源文件导致了部分传输。"));
//     m.insert(25, String::from("-max-delete限制停止删除。"));
//     m.insert(30, String::from("数据传送/接收超时。"));
//     m.insert(35, String::from("等待守护进程连接超时。"));
//     m
// });
// 
// // get_tle_config通过ca_file生成tls.config
// fn get_tls_config(ca_file: &str) -> Result<Arc<ClientConfig>, Box<dyn Error>> {
//     // 读取CA文件内容
//     let ca_cert = fs::read(ca_file)?;
// 
//     // 创建证书池
//     let mut root_cert_store = RootCertStore::empty();
//     let mut buf = BufReader::new(&ca_cert[..]);
// 
//     // 将CA证书添加到证书池中
//     let certs = pemfile::certs(&mut buf)
//         .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to parse CA"))?;
//     if certs.is_empty() {
//         return Err(Box::new(io::Error::new(io::ErrorKind::InvalidData, "No certificates found in CA file")));
//     }
//     root_cert_store.add_parsable_certificates(&certs);
// 
//     // 创建并配置TLS配置
//     let config = ClientConfig::builder()
//         .with_safe_defaults()
//         .with_root_certificates(root_cert_store)
//         .with_no_client_auth(); // 无客户端证书
// 
//     Ok(Arc::new(config))
// }
// 
// 
// async fn create_http_client(ca_file: Option<&str>) -> Result<Client, Box<dyn Error>> {
//     // 加载自定义TLS配置
//     let mut client_builder = Client::builder()
//         .timeout(Duration::from_secs(5))
//         .pool_max_idle_per_host(20);
// 
//     if let Some(ca_path) = ca_file {
//         let tls_config = get_tls_config(ca_path)?; // 使用之前定义的get_tls_config函数
//         client_builder = client_builder.use_preconfigured_tls(tls_config);
//     }
// 
//     let client = client_builder.build()?;
//     Ok(client)
// }
// 
// async fn post_json<T: Serialize>(url: &str, obj: &T, client: Option<&Client>) -> Result<Response, Box<dyn Error>> {
//     let client = match client {
//         Some(c) => c.clone(),
//         None => create_http_client(None).await?,  // 使用之前定义的 create_http_client 函数
//     };
// 
//     let resp = client
//         .post(url)
//         .json(obj)
//         .send()
//         .await?;
// 
//     Ok(resp)
// }
// 
// async fn get_json<T: DeserializeOwned>(url: &str, client: Option<&Client>) -> Result<T, Box<dyn Error>> {
//     let client = match client {
//         Some(c) => c.clone(),
//         None => create_http_client(None).await?,
//     };
// 
//     let resp = client
//         .get(url)
//         .send()
//         .await?;
// 
//     if resp.status() != reqwest::StatusCode::OK {
//         return Err(format!("HTTP status code is not 200: {}", resp.status()).into());
//     }
// 
//     let obj = resp.json::<T>().await?;
//     Ok(obj)
// }
// 
// fn find_all_submatch_in_file(file_name: &str, re: &Regex) -> Result<Vec<Vec<Vec<u8>>>, io::Error> {
//     if file_name == "/dev/null" {
//         return Err(io::Error::new(ErrorKind::InvalidInput, "Invalid log file"));
//     }
// 
//     let content = fs::read(file_name)?;
//     let mut matches = Vec::new();
// 
//     for cap in re.captures_iter(&content) {
//         let submatches: Vec<Vec<u8>> = cap.iter().skip(1)
//             .map(|m| m.map_or(vec![], |m| m.as_bytes().to_vec()))
//             .collect();
//         matches.push(submatches);
//     }
// 
//     Ok(matches)
// }
// 
// fn extract_size_from_log(log_file: &str, re: &Regex) -> Option<String> {
//     match find_all_submatch_in_file(log_file, re) {
//         Ok(matches) if !matches.is_empty() => {
//             if let Some(last_match) = matches.last() {
//                 if last_match.len() > 1 {
//                     return Some(String::from_utf8_lossy(&last_match[1]).into_owned());
//                 }
//             }
//             None
//         },
//         _ => None,
//     }
// }
// 
// fn extract_size_from_rsync_log(log_file: &str) -> Option<String> {
//     let re = Regex::new(r"(?m)^Total file size: ([0-9.]+[KMGTP]?) bytes").unwrap();
//     extract_size_from_log(log_file, &re)
// }
// 
// fn translate_rsync_error_code(status: ExitStatus) -> (i32, String) {
//     let exit_code = status.code().unwrap_or(-1);
// 
//     if let Some(err_msg) = RSYNC_EXIT_VALUES.get(&exit_code) {
//         (exit_code, format!("rsync error: {}", err_msg))
//     } else {
//         (exit_code, String::from("Unknown rsync error"))
//     }
// }
// 
// 
// 
// 
// 
// 
// 
// 











