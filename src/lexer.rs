#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // 単一文字トークン
    LeftParen, RightParen, LeftBrace, RightBrace,
    Comma, Dot, Minus, Plus, Semicolon, Slash, Star,
    
    // 1〜2文字トークン
    Bang, BangEqual,
    Equal, EqualEqual,
    Greater, GreaterEqual,
    Less, LessEqual,
    
    // リテラル
    Identifier, String, Number,
    
    // キーワード
    And, Break, Do, Else, Elseif, End,
    False, For, Function, If, In, Local,
    Nil, Not, Or, Repeat, Return, Then,
    True, Until, While,
    
    // その他
    Eof
}

#[derive(Debug, Clone)]
pub struct Token {
    pub token_type: TokenType,
    pub lexeme: String,
    pub line: usize,
}

pub struct Lexer {
    source: Vec<char>,
    tokens: Vec<Token>,
    start: usize,
    current: usize,
    line: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            source: source.chars().collect(),
            tokens: Vec::new(),
            start: 0,
            current: 0,
            line: 1,
        }
    }
    
    pub fn scan_tokens(&mut self) -> Result<Vec<Token>, String> {
        // トークンのスキャンを実装します
        // 現在は空のトークンリストを返すだけ
        Ok(self.tokens.clone())
    }
}
