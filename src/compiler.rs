use crate::parser::{Expr, Stmt, LiteralValue};
use crate::value::Value;
use std::rc::Rc;

// 命令セット
#[derive(Debug, Clone, Copy)]
pub enum OpCode {
    Constant,
    Nil,
    True,
    False,
    Pop,
    GetLocal,
    SetLocal,
    GetGlobal,
    DefineGlobal,
    SetGlobal,
    GetUpvalue,
    SetUpvalue,
    GetProperty,
    SetProperty,
    GetIndex,
    SetIndex,
    Equal,
    Greater,
    Less,
    Add,
    Subtract,
    Multiply,
    Divide,
    Not,
    Negate,
    Print,
    Jump,
    JumpIfFalse,
    Loop,
    Call,
    Closure,
    CloseUpvalue,
    Return,
    Table,
}

// チャンク（バイトコード）
#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub constants: Vec<Value>,
    pub lines: Vec<usize>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
        }
    }
    
    pub fn write(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
    }
    
    pub fn add_constant(&mut self, value: Value) -> usize {
        self.constants.push(value);
        self.constants.len() - 1
    }
}

// コンパイラ
pub struct Compiler {
    chunk: Chunk,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            chunk: Chunk::new(),
        }
    }
    
    pub fn compile(&mut self, statements: Vec<Stmt>) -> Result<Chunk, String> {
        // 各文をコンパイル
        for stmt in statements {
            self.compile_statement(&stmt)?;
        }
        
        // 終了命令を追加
        self.emit_return();
        
        Ok(self.chunk.clone())
    }
    
    fn compile_statement(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expression { expression } => {
                self.compile_expression(expression)?;
                self.emit_byte(OpCode::Pop as u8, 0);
            },
            // 他の文の種類は後で実装
            _ => return Err("未実装の文の種類です".to_string()),
        }
        
        Ok(())
    }
    
    fn compile_expression(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Literal { value } => {
                match value {
                    LiteralValue::Nil => self.emit_byte(OpCode::Nil as u8, 0),
                    LiteralValue::Boolean(true) => self.emit_byte(OpCode::True as u8, 0),
                    LiteralValue::Boolean(false) => self.emit_byte(OpCode::False as u8, 0),
                    LiteralValue::Number(num) => {
                        let constant = self.make_constant(Value::Number(*num));
                        self.emit_bytes(OpCode::Constant as u8, constant, 0);
                    },
                    LiteralValue::String(s) => {
                        let constant = self.make_constant(Value::String(Rc::new(s.clone())));
                        self.emit_bytes(OpCode::Constant as u8, constant, 0);
                    },
                }
            },
            // 他の式の種類は後で実装
            _ => return Err("未実装の式の種類です".to_string()),
        }
        
        Ok(())
    }
    
    fn emit_byte(&mut self, byte: u8, line: usize) {
        self.chunk.write(byte, line);
    }
    
    fn emit_bytes(&mut self, byte1: u8, byte2: u8, line: usize) {
        self.emit_byte(byte1, line);
        self.emit_byte(byte2, line);
    }
    
    fn emit_return(&mut self) {
        self.emit_byte(OpCode::Return as u8, 0);
    }
    
    fn make_constant(&mut self, value: Value) -> u8 {
        let constant = self.chunk.add_constant(value);
        if constant > u8::MAX as usize {
            panic!("定数が多すぎます");
        }
        constant as u8
    }
}
