use std::cell::RefCell;
use std::rc::Rc;
use chrono::{DateTime, Utc};
use crate::context::Context;

// baseProvider是providers的基本混合
struct BaseProvider<T: Clone> {
    //mutex
    ctx: Rc<RefCell<Context<T>>> ,
    name: String,
    interval: DateTime<Utc>,
    retry: u64,
    timeout: DateTime<Utc>,
    is_master: bool,
    
    // cmd: 
}