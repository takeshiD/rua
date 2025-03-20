use std::rc::Rc;
use std::cell::RefCell;
use std::fmt;
use crate::value::Value;

// オブジェクトの種類を表す列挙型
pub enum ObjectType {
    String,
    Function,
    NativeFunction,
    Closure,
    Upvalue,
}

// オブジェクトのトレイト
pub trait Object: fmt::Debug {
    fn object_type(&self) -> ObjectType;
    fn is_equal(&self, other: &dyn Object) -> bool;
}

// 文字列オブジェクト
#[derive(Debug, Clone)]
pub struct StringObj {
    pub value: String,
}

impl Object for StringObj {
    fn object_type(&self) -> ObjectType {
        ObjectType::String
    }
    
    fn is_equal(&self, other: &dyn Object) -> bool {
        if let Some(other_string) = other.as_any().downcast_ref::<StringObj>() {
            self.value == other_string.value
        } else {
            false
        }
    }
}

// 関数オブジェクト（後で実装）
#[derive(Debug, Clone)]
pub struct FunctionObj {
    // 関数の実装
}

// Any型へのダウンキャストを可能にする拡張トレイト
pub trait AsAny {
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: Object + std::any::Any> AsAny for T {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
