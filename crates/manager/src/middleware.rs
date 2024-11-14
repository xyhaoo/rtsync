// use rocket::{Request, Data, Response, outcome, State};
// use rocket::http::Status;
// use rocket::fairing::{Fairing, Info, Kind};
// use rocket::tokio::sync::Mutex;
// use std::sync::Arc;
// use std::fmt;
// use crate::server::Manager;
// // 错误日志中间件
// struct ErrorLogger;
// 
// #[rocket::async_trait]
// 
// impl Fairing for ErrorLogger {
//     fn info(&self) -> Info {
//         Info {
//             name: "Error Logger",
//             kind: Kind::Request | Kind::Response,
//         }
//     }
// 
//     async fn on_request(&self, _req: &rocket::Request, _data: Data<'_>) {
//         // 可以在此记录请求的详细信息，如果需要
//     }
// 
//     async fn on_response(&self, _req: &rocket::Request, res: &mut Response<'_>) {
//         if let Some(err) = res.get_error() {
//             // 记录错误信息
//             println!("Request: {} {} - Error: {}", _req.method(), _req.uri(), err);
//         }
//     }
// }
// 
// // workerID 校验中间件
// #[rocket::async_trait]
// impl<'r> rocket::request::FromParam<'r> for Manager {
//     async fn from_param(param: rocket::Request<'r>, worker_id: &str) -> rocket::Outcome<'r, Self> {
//         // 模拟验证 workerID
//         let manager = Manager { adapter: Adapter };
//         if let Err(err) = manager.adapter.get_worker(worker_id) {
//             let err_message = format!("Invalid workerID {}: {}", worker_id, err);
//             return Outcome::Failure((Status::BadRequest, err_message));
//         }
//         Outcome::Success(manager)
//     }
// }
// 
