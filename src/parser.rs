use crate::lexer::{Token, TokenType};

// 抽象構文木のノード
#[derive(Debug, Clone)]
pub enum Expr {
    Binary {
        left: Box<Expr>,
        operator: Token,
        right: Box<Expr>,
    },
    Grouping {
        expression: Box<Expr>,
    },
    Literal {
        value: LiteralValue,
    },
    Unary {
        operator: Token,
        right: Box<Expr>,
    },
    Variable {
        name: Token,
    },
    Assign {
        name: Token,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        paren: Token,
        arguments: Vec<Expr>,
    },
    Table {
        fields: Vec<(Option<Expr>, Expr)>, // キーが None の場合は配列形式
    },
    Index {
        table: Box<Expr>,
        key: Box<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum LiteralValue {
    Nil,
    Boolean(bool),
    Number(f64),
    String(String),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expression {
        expression: Expr,
    },
    Print {
        expression: Expr,
    },
    Var {
        name: Token,
        initializer: Option<Expr>,
    },
    Block {
        statements: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
    },
    Function {
        name: Token,
        params: Vec<Token>,
        body: Vec<Stmt>,
    },
    Return {
        keyword: Token,
        value: Option<Expr>,
    },
}

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            current: 0,
        }
    }
    
    pub fn parse(&mut self) -> Result<Vec<Stmt>, String> {
        let mut statements = Vec::new();
        
        while !self.is_at_end() {
            if let Ok(stmt) = self.declaration() {
                statements.push(stmt);
            } else {
                // エラー回復処理
                self.synchronize();
            }
        }
        
        Ok(statements)
    }
    
    fn declaration(&mut self) -> Result<Stmt, String> {
        // 宣言の解析（変数、関数など）
        // 現在は単純な式文を返す
        self.statement()
    }
    
    fn statement(&mut self) -> Result<Stmt, String> {
        // 文の解析
        // 現在は単純な式文を返す
        let expr = self.expression()?;
        Ok(Stmt::Expression { expression: expr })
    }
    
    fn expression(&mut self) -> Result<Expr, String> {
        // 式の解析
        // 現在は単純なリテラルを返す
        Ok(Expr::Literal { value: LiteralValue::Nil })
    }
    
    fn is_at_end(&self) -> bool {
        self.current >= self.tokens.len() || 
        self.tokens[self.current].token_type == TokenType::Eof
    }
    
    fn synchronize(&mut self) {
        // エラー回復のための同期処理
        self.advance();
        
        while !self.is_at_end() {
            if self.previous().token_type == TokenType::Semicolon {
                return;
            }
            
            match self.peek().token_type {
                TokenType::Function | 
                TokenType::Var | 
                TokenType::For | 
                TokenType::If | 
                TokenType::While | 
                TokenType::Return => return,
                _ => (),
            }
            
            self.advance();
        }
    }
    
    fn advance(&mut self) -> Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }
    
    fn peek(&self) -> Token {
        self.tokens[self.current].clone()
    }
    
    fn previous(&self) -> Token {
        self.tokens[self.current - 1].clone()
    }
}
