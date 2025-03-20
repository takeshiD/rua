use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use crate::value::Value;

// Luaのテーブル実装
#[derive(Debug, Clone)]
pub struct Table {
    map: HashMap<Value, Value>,
    metatable: Option<Rc<RefCell<Table>>>,
}

impl Table {
    pub fn new() -> Self {
        Table {
            map: HashMap::new(),
            metatable: None,
        }
    }
    
    pub fn get(&self, key: &Value) -> Option<Value> {
        match self.map.get(key) {
            Some(value) => Some(value.clone()),
            None => {
                // メタテーブルがあれば__indexメソッドを探す
                if let Some(metatable) = &self.metatable {
                    let metatable = metatable.borrow();
                    let index_key = Value::String(Rc::new("__index".to_string()));
                    
                    if let Some(index_fn) = metatable.map.get(&index_key) {
                        // __indexがテーブルの場合は再帰的に検索
                        if let Value::Table(table) = index_fn {
                            let table = table.borrow();
                            return table.get(key);
                        }
                        // 関数の場合は後で実装
                    }
                }
                None
            }
        }
    }
    
    pub fn set(&mut self, key: Value, value: Value) {
        self.map.insert(key, value);
    }
    
    pub fn set_metatable(&mut self, metatable: Rc<RefCell<Table>>) {
        self.metatable = Some(metatable);
    }
    
    pub fn get_metatable(&self) -> Option<Rc<RefCell<Table>>> {
        self.metatable.clone()
    }
}
