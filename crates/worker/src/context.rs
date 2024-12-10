use std::collections::HashMap;
use std::error::Error;
use anymap::{self, Map};
// Context对象的目的是存储运行时配置
// Context 是一个分层的键值存储
// 当进入一个新的上下文时，修改将被存储在新的层次上
// 当退出时，最上层的上下文被弹出，存储回到退出之前的状态
#[derive(Debug, Default)]
pub struct Context {
    parent: Option<Box<Context>>,  // 上一个 Context
    store: HashMap<String, Map>, // 当前层的键值存储
}

impl Context {
    // 创建一个新的 Context
    pub fn new() -> Self {
        Context {
            parent: None,
            store: HashMap::new(),
        }
    }

    // 生成一个新的层次的 Context
    pub fn enter(self) -> Self {
        Context {
            parent: Some(Box::new(self)),
            store: HashMap::new(),
        }
    }

    // 退出当前 Context，返回上层的 Context
    pub fn exit(self) -> Result<Self, Box<dyn Error>> {
        match self.parent {
            Some(parent) => Ok(*parent),
            None => Err("无法从底层context退出".into()),
        }
    }

    // 获取当前层或下层 Context 中的值
    pub fn get(&self, key: &str) -> Option<&Map> {
        // 尝试从当前层获取
        let current = self.store.get(key);
        if self.parent.is_none() {
            if let Some(value) = current {
               return Some(value)
            }
            None
        }else {
            if let Some(value) = current {
                Some(value)
            }else {
                self.parent.as_ref().and_then(|parent| parent.get(key))
            }
        }
    }

    // 在当前层设置值
    pub fn set(&mut self, key: String, value: Map) {
        self.store.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use anymap::AnyMap;
    use super::*;
    #[test]
    fn test_context() {
        let mut ctx = Context::new();
        assert!(ctx.parent.is_none());
        let mut l1 = AnyMap::new();
        l1.insert("logdir_value_1");
        let mut l2 = AnyMap::new();
        l2.insert("logdir_value_2");

        ctx.set("logdir1".into(), l1);
        ctx.set("logdir2".into(), l2);
        assert_eq!(ctx.get("logdir1").unwrap().get::<&str>(), Some(&"logdir_value_1"));

        test_entering_a_new_context(ctx)
    }
    fn test_entering_a_new_context(mut ctx: Context) {
        ctx = ctx.enter();
        assert_eq!(ctx.get("logdir1").unwrap().get::<&str>(), Some(&"logdir_value_1"));

        let mut l1 = AnyMap::new();
        l1.insert("new_value_1");
        ctx.set("logdir1".into(), l1);
        assert_eq!(ctx.get("logdir1").unwrap().get::<&str>(), Some(&"new_value_1"));

        assert_eq!(ctx.get("logdir2").unwrap().get::<&str>(), Some(&"logdir_value_2"));

        test_accessing_invalid_context(&ctx);
        test_accessing_new_context(ctx);
    }
    fn test_accessing_invalid_context(ctx: &Context) {
        assert!(ctx.get("invalid_key").is_none());
        
    }
    fn test_accessing_new_context(mut ctx: Context) {
        ctx = ctx.exit().unwrap();

        assert_eq!(ctx.get("logdir1").unwrap().get::<&str>(), Some(&"logdir_value_1"));
        assert_eq!(ctx.get("logdir2").unwrap().get::<&str>(), Some(&"logdir_value_2"));

        test_exiting_from_bottom_context(ctx);
    }
    fn test_exiting_from_bottom_context(ctx: Context) {
        let result = ctx.exit();
        assert_eq!(result.is_err(), true);
    }
}


/*


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


 */


