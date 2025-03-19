use crate::value::Value;

pub struct VM {
    stack: Vec<Value>,
    // 他のVMの状態を追加
}

impl VM {
    pub fn new() -> Self {
        VM {
            stack: Vec::new(),
        }
    }
    
    pub fn interpret(&mut self, source: &str) -> Result<(), String> {
        // ソースコードの解釈を実装します
        // 現在は何もせずに成功を返すだけ
        Ok(())
    }
}
