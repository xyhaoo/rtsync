// Context对象的目的是存储运行时配置

// Context对象是一个分层的键值存储
// 当进入一个上下文时，对存储的更改将被存储在一个新的层中
// 当退出时，顶层弹出，存储返回到进入该上下文之前的状态

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::rc::Rc;

pub struct Context<T: Clone> {
    parent: Option<Rc<RefCell<Context<T>>>>,
    store: HashMap<String, T>,
}
impl<T: Clone> Context<T> {
    fn new() -> Rc<RefCell<Context<T>>> {
        Rc::new(RefCell::new(
            Context{
                parent: None,
                store: HashMap::new(),
            }
        ))
    }
    
    //enter生成一个新的context层
    fn enter(ctx: Rc<RefCell<Context<T>>>) -> Rc<RefCell<Context<T>>> {
        Rc::new(RefCell::new(
            Context{
                parent: Some(Rc::clone(&ctx)),
                store: HashMap::new(),
            }
        ))
    }

    
    //exit返回上一层context
    fn exit(ctx: Rc<RefCell<Context<T>>>) 
        -> Result<Rc<RefCell<Context<T>>>, Box<dyn std::error::Error>> 
    {
        match ctx.borrow_mut().parent {
            None => Err("Cannot exit the bottom layer context".to_string().into()),
            Some(ref Context) => Ok(Rc::clone(Context)),
        }
    }


    // get返回key对应的值，如果在当前层中没有找到，则返回较低层的context的值
    pub(crate) fn get(ctx: Rc<RefCell<Context<T>>>, key: &str) -> Option<T> {
        // 检查当前层的store
        if let Some(value) = ctx.borrow().store.get(key) {
            return Some(value.clone()); 
        }

        // 如果有下层context，检查其store
        if let Some(parent) = &ctx.borrow().parent {
            return Self::get(Rc::clone(parent), key); 
        }
        
        None
    }
    

    
    // set设置当前层的key
    pub(crate) fn set(ctx: Rc<RefCell<Context<T>>>, key: &str, val: T) {
        ctx.borrow_mut().store.insert(key.to_string(), val);
    }

}
