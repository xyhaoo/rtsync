use rocket::{Request, Response};
use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Status;
use log::error;
use rocket::request::{FromRequest, Outcome};
use rocket::serde::json::Json;
use crate::db::DbAdapter;
use crate::server;

#[derive(Default)]
pub(crate) struct ContextErrorLogger;

#[rocket::async_trait]
impl Fairing for ContextErrorLogger {
    fn info(&self) -> Info {
        Info {
            name: "Error Logger",
            kind: Kind::Response,
        }
    }
    async fn on_response<'r>(&self, _request: &'r Request<'_>, response: &mut Response<'r>) {
        // Here we can check for errors after request handling
        if response.status() != Status::Ok {
            // Log errors in the response status or other conditions
            error!(
                "请求: {} {}发生错误，状态码: {}",
                _request.method(),
                _request.uri(),
                response.status()
            );
        }
    }
}

#[derive(Debug)]
pub(crate) struct CheckWorkerId;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for CheckWorkerId {
    type Error = Json<server::Response>;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let id = request.param::<&str>(1).unwrap().unwrap();
        match request.rocket().state::<Box<dyn DbAdapter>>(){
            Some(adapter) => {
                if let Err(_) = adapter.get_worker(id) {
                    // 这个worker不存在
                    let error = format!("无效的worker_id: {}", id);
                    error!("{}", error);
                    return Outcome::Error((Status::BadRequest, Json(server::Response::Error (error))));
                }
            }
            None => {
                let error = "没有找到adapter".to_string();
                error!("{}", error);
                return Outcome::Error((Status::BadRequest, Json(server::Response::Error (error))));
            }
        }
        Outcome::Success(CheckWorkerId)
    }
}


