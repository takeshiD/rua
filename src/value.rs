use std::fmt;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(Rc<String>),
    Table(Rc<RefCell<HashMap<Value, Value>>>),
    Function(/* 関数の表現は後で実装 */),
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Boolean(b) => write!(f, "{}", b),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Table(_) => write!(f, "table"),
            Value::Function(_) => write!(f, "function"),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(a, b),
            (Value::Function(_), Value::Function(_)) => false, // 関数の比較は後で実装
            _ => false,
        }
    }
}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Nil => 0.hash(state),
            Value::Boolean(b) => b.hash(state),
            Value::Number(n) => {
                let bits = n.to_bits();
                bits.hash(state);
            },
            Value::String(s) => s.hash(state),
            Value::Table(t) => Rc::as_ptr(t).hash(state),
            Value::Function(_) => {
                // 関数のハッシュは後で実装
                0.hash(state);
            },
        }
    }
}
