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
    
    #[test]
    fn test_exit(){
        let mut ctx = Context::new();
        ctx.set(String::from("logdir_value_1"), Map::new());
        ctx.set(String::from("logdir_value_2"), Map::new());
        ctx = ctx.enter();
        ctx.set(String::from("new_value_1"), Map::new());
        ctx.get("logdir_value_1").unwrap();
    }
}





