use lazy_static::lazy_static;
use std::sync::{Arc, RwLock};
use rocket::{log, Build, Rocket};
use crate::config::Config;
use crate::db::{make_db_adapter, DbAdapter};

const _ERROR_KEY: &str = "error";
const _INFO_KEY: &str = "message";

// 一个Manager代表一个manager服务器
pub(crate) struct Manager{
    cfg: Config,
    // engine: ,
    adapter: Option<Box<dyn DbAdapter>>,
    // rwmu: ,// 使用Arc<RwLock<>>包裹Manager达到相同操作
    http_client: Option<reqwest::Client>,
}
impl Manager {
    fn set_db_adapter(&mut self, adapter: Box<dyn DbAdapter>){
        self.adapter = Some(adapter);
    }
}

lazy_static! {
    static ref MANAGER: Arc<RwLock<Option<Manager>>> = Arc::new(RwLock::new(None));
}

fn get_rtsync_manager(cfg: &Config) -> &'static MANAGER{
    let instance = &MANAGER;
    match instance.read() {
        Ok(manager) => {
            if manager.is_some() {
                return instance
            }else {
                if !cfg.debug{} // rust不需要根据debug字段确定debug和release
                let mut s = Manager{
                    cfg: cfg.clone(),
                    adapter: None,
                    http_client: None,
                };
                /*
                这里捕获panic异常，使程序恢复
                如果是debug，初始化logger
                 */
                if cfg.files.ca_cert.is_some() {

                }

                if cfg.files.db_file.is_some(){
                    let (db_type, db_file) = (&*cfg.files.db_type.clone().unwrap(),
                                              &*cfg.files.db_file.clone().unwrap());
                    let adapter = make_db_adapter(db_type, db_file);
                    match adapter {
                        Err(e) => {
                            error!("初始化DB adapter失败: {}", e);
                            return instance;
                        }
                        Ok(adapter) => {
                            s.set_db_adapter(Box::new(adapter));
                        }
                    }
                }
                
            }
        },
        Err(e) => {
            eprintln!("获取MANAGER锁失败：{}", e);
        }
    }

    instance
}


#[launch]
fn rocket() -> _ {
    rocket::build().manage(MANAGER.clone())
}