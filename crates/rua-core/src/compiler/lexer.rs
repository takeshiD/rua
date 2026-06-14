//! 字句解析（本家 `llex.c` 相当）。担当: **lua-frontend**。
//!
//! Lua 5.1 のトークンを生成する。予約語・記号・名前・数値リテラル（10進/16進）・
//! 文字列（短/長, エスケープ含む）・長括弧コメント（`--[[ .. ]]`, `--[==[ .. ]==]`）を扱う。
//! 行番号は本家 `inclinenumber` と同じく `\n` / `\r` / `\r\n` / `\n\r` を 1 行として数える。
//!
//! エラー文言は本家 `luaX_lexerror` に合わせ `<chunk>:<line>: <msg>` 形式（必要なら ` near '<token>'`）。

use crate::error::{LuaError, LuaResult};

/// Lua 5.1 のトークン種別（本家 `RESERVED` + 記号 + リテラル）。
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // --- リテラル / 名前 ---
    /// 識別子。
    Name(String),
    /// 数値リテラル（Lua 5.1 は整数型を持たず全て double）。
    Number(f64),
    /// 文字列リテラル（バイト列。Lua 文字列は 8bit クリーン）。
    Str(Vec<u8>),

    // --- 予約語 ---
    And,
    Break,
    Do,
    Else,
    Elseif,
    End,
    False,
    For,
    Function,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,

    // --- 記号 ---
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Caret,     // ^
    Hash,      // #
    Eq,        // ==
    Ne,        // ~=
    Le,        // <=
    Ge,        // >=
    Lt,        // <
    Gt,        // >
    Assign,    // =
    LParen,    // (
    RParen,    // )
    LBrace,    // {
    RBrace,    // }
    LBracket,  // [
    RBracket,  // ]
    Semicolon, // ;
    Colon,     // :
    Comma,     // ,
    Dot,       // .
    Concat,    // ..
    Ellipsis,  // ...

    /// 入力終端（本家 `TK_EOS`）。
    Eof,
}

impl Token {
    /// 予約語の文字列から対応するトークンを返す（名前でなければ `None`）。
    fn keyword(s: &str) -> Option<Token> {
        Some(match s {
            "and" => Token::And,
            "break" => Token::Break,
            "do" => Token::Do,
            "else" => Token::Else,
            "elseif" => Token::Elseif,
            "end" => Token::End,
            "false" => Token::False,
            "for" => Token::For,
            "function" => Token::Function,
            "if" => Token::If,
            "in" => Token::In,
            "local" => Token::Local,
            "nil" => Token::Nil,
            "not" => Token::Not,
            "or" => Token::Or,
            "repeat" => Token::Repeat,
            "return" => Token::Return,
            "then" => Token::Then,
            "true" => Token::True,
            "until" => Token::Until,
            "while" => Token::While,
            _ => return None,
        })
    }

    /// エラーメッセージ中の `near '<token>'` 表示（本家 `txtToken` / `luaX_token2str` 相当）。
    pub fn display(&self) -> String {
        match self {
            Token::Name(s) => s.clone(),
            Token::Number(n) => format!("{n}"),
            Token::Str(b) => String::from_utf8_lossy(b).into_owned(),
            Token::And => "and".into(),
            Token::Break => "break".into(),
            Token::Do => "do".into(),
            Token::Else => "else".into(),
            Token::Elseif => "elseif".into(),
            Token::End => "end".into(),
            Token::False => "false".into(),
            Token::For => "for".into(),
            Token::Function => "function".into(),
            Token::If => "if".into(),
            Token::In => "in".into(),
            Token::Local => "local".into(),
            Token::Nil => "nil".into(),
            Token::Not => "not".into(),
            Token::Or => "or".into(),
            Token::Repeat => "repeat".into(),
            Token::Return => "return".into(),
            Token::Then => "then".into(),
            Token::True => "true".into(),
            Token::Until => "until".into(),
            Token::While => "while".into(),
            Token::Plus => "+".into(),
            Token::Minus => "-".into(),
            Token::Star => "*".into(),
            Token::Slash => "/".into(),
            Token::Percent => "%".into(),
            Token::Caret => "^".into(),
            Token::Hash => "#".into(),
            Token::Eq => "==".into(),
            Token::Ne => "~=".into(),
            Token::Le => "<=".into(),
            Token::Ge => ">=".into(),
            Token::Lt => "<".into(),
            Token::Gt => ">".into(),
            Token::Assign => "=".into(),
            Token::LParen => "(".into(),
            Token::RParen => ")".into(),
            Token::LBrace => "{".into(),
            Token::RBrace => "}".into(),
            Token::LBracket => "[".into(),
            Token::RBracket => "]".into(),
            Token::Semicolon => ";".into(),
            Token::Colon => ":".into(),
            Token::Comma => ",".into(),
            Token::Dot => ".".into(),
            Token::Concat => "..".into(),
            Token::Ellipsis => "...".into(),
            Token::Eof => "<eof>".into(),
        }
    }
}

/// 行番号付きトークン。
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub tok: Token,
    pub line: u32,
}

/// 字句解析器。バイト列を走査して 1 トークンずつ生成する。
pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    /// 現在行（1 始まり）。
    line: u32,
    /// エラー表示用のチャンク短縮名（本家 `luaO_chunkid` 済み）。
    chunk: String,
    /// 文字列/数値リテラル組み立て用バッファ。
    buff: Vec<u8>,
}

impl<'a> Lexer<'a> {
    /// 入力と（短縮済みの）チャンク名から字句解析器を作る。
    pub fn new(src: &'a [u8], chunk: impl Into<String>) -> Self {
        let mut lx = Lexer {
            src,
            pos: 0,
            line: 1,
            chunk: chunk.into(),
            buff: Vec::new(),
        };
        // 本家 luaL_loadfile 相当のシバン行スキップ（先頭が `#`）。
        if lx.src.first() == Some(&b'#') {
            while let Some(c) = lx.cur() {
                if c == b'\n' || c == b'\r' {
                    break;
                }
                lx.pos += 1;
            }
        }
        lx
    }

    /// 現在行。
    pub fn line(&self) -> u32 {
        self.line
    }

    /// チャンク名。
    pub fn chunk(&self) -> &str {
        &self.chunk
    }

    /// `<chunk>:<line>: <msg>` 形式の構文エラーを作る。
    pub fn error(&self, msg: impl AsRef<str>) -> LuaError {
        LuaError::Syntax(format!("{}:{}: {}", self.chunk, self.line, msg.as_ref()))
    }

    /// `<chunk>:<line>: <msg> near '<near>'` 形式。
    fn error_near(&self, msg: &str, near: &str) -> LuaError {
        LuaError::Syntax(format!(
            "{}:{}: {} near '{}'",
            self.chunk, self.line, msg, near
        ))
    }

    fn cur(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn is_newline(c: u8) -> bool {
        c == b'\n' || c == b'\r'
    }

    /// 本家 `inclinenumber`: `\n` / `\r` / `\r\n` / `\n\r` を 1 行として進める。
    /// 呼び出し時 `cur()` は改行文字であること。
    fn inc_line(&mut self) -> LuaResult<()> {
        let old = self.cur().unwrap();
        self.advance(); // skip \n or \r
        if let Some(c) = self.cur()
            && Self::is_newline(c)
            && c != old
        {
            self.advance(); // skip the paired \n\r or \r\n
        }
        self.line = self
            .line
            .checked_add(1)
            .ok_or_else(|| self.error("chunk has too many lines"))?;
        Ok(())
    }

    /// 次のトークンを行番号付きで返す（空白・コメントは読み飛ばす）。
    pub fn next_token(&mut self) -> LuaResult<Spanned> {
        self.scan()
    }

    /// 空白とコメントを読み飛ばしつつ 1 トークンを取得。
    fn scan(&mut self) -> LuaResult<Spanned> {
        loop {
            let c = match self.cur() {
                None => {
                    return Ok(Spanned {
                        tok: Token::Eof,
                        line: self.line,
                    });
                }
                Some(c) => c,
            };

            match c {
                b'\n' | b'\r' => {
                    self.inc_line()?;
                }
                b' ' | b'\t' | b'\x0b' | b'\x0c' => {
                    // space, tab, vertical tab, form feed
                    self.advance();
                }
                b'-' => {
                    self.advance();
                    if self.cur() != Some(b'-') {
                        return self.spanned(Token::Minus);
                    }
                    // コメント
                    self.advance();
                    if self.cur() == Some(b'[') {
                        let sep = self.skip_sep();
                        if sep >= 0 {
                            self.read_long_string(sep, false)?;
                            continue;
                        }
                    }
                    // 短いコメント: 行末まで
                    while let Some(ch) = self.cur() {
                        if Self::is_newline(ch) {
                            break;
                        }
                        self.advance();
                    }
                }
                b'[' => {
                    let start_line = self.line;
                    let sep = self.skip_sep();
                    if sep >= 0 {
                        let s = self.read_long_string(sep, true)?;
                        return Ok(Spanned {
                            tok: Token::Str(s),
                            line: start_line,
                        });
                    } else if sep == -1 {
                        return self.spanned(Token::LBracket);
                    } else {
                        return Err(self.error("invalid long string delimiter"));
                    }
                }
                b'=' => {
                    self.advance();
                    if self.cur() == Some(b'=') {
                        self.advance();
                        return self.spanned(Token::Eq);
                    }
                    return self.spanned(Token::Assign);
                }
                b'<' => {
                    self.advance();
                    if self.cur() == Some(b'=') {
                        self.advance();
                        return self.spanned(Token::Le);
                    }
                    return self.spanned(Token::Lt);
                }
                b'>' => {
                    self.advance();
                    if self.cur() == Some(b'=') {
                        self.advance();
                        return self.spanned(Token::Ge);
                    }
                    return self.spanned(Token::Gt);
                }
                b'~' => {
                    self.advance();
                    if self.cur() == Some(b'=') {
                        self.advance();
                        return self.spanned(Token::Ne);
                    }
                    // 単独の '~' は不正（本家は "=" を期待）。
                    return Err(self.error_near("unexpected symbol", "~"));
                }
                b'"' | b'\'' => {
                    let start_line = self.line;
                    let s = self.read_string(c)?;
                    return Ok(Spanned {
                        tok: Token::Str(s),
                        line: start_line,
                    });
                }
                b'.' => {
                    // ".", "..", "...", もしくは数値 ".5"
                    if self.peek(1) == Some(b'.') {
                        if self.peek(2) == Some(b'.') {
                            self.pos += 3;
                            return self.spanned(Token::Ellipsis);
                        }
                        self.pos += 2;
                        return self.spanned(Token::Concat);
                    }
                    if matches!(self.peek(1), Some(d) if d.is_ascii_digit()) {
                        let n = self.read_numeral()?;
                        return self.spanned(Token::Number(n));
                    }
                    self.advance();
                    return self.spanned(Token::Dot);
                }
                b'+' => {
                    self.advance();
                    return self.spanned(Token::Plus);
                }
                b'*' => {
                    self.advance();
                    return self.spanned(Token::Star);
                }
                b'/' => {
                    self.advance();
                    return self.spanned(Token::Slash);
                }
                b'%' => {
                    self.advance();
                    return self.spanned(Token::Percent);
                }
                b'^' => {
                    self.advance();
                    return self.spanned(Token::Caret);
                }
                b'#' => {
                    self.advance();
                    return self.spanned(Token::Hash);
                }
                b'(' => {
                    self.advance();
                    return self.spanned(Token::LParen);
                }
                b')' => {
                    self.advance();
                    return self.spanned(Token::RParen);
                }
                b'{' => {
                    self.advance();
                    return self.spanned(Token::LBrace);
                }
                b'}' => {
                    self.advance();
                    return self.spanned(Token::RBrace);
                }
                b']' => {
                    self.advance();
                    return self.spanned(Token::RBracket);
                }
                b';' => {
                    self.advance();
                    return self.spanned(Token::Semicolon);
                }
                b':' => {
                    self.advance();
                    return self.spanned(Token::Colon);
                }
                b',' => {
                    self.advance();
                    return self.spanned(Token::Comma);
                }
                c if c.is_ascii_digit() => {
                    let n = self.read_numeral()?;
                    return self.spanned(Token::Number(n));
                }
                c if c == b'_' || c.is_ascii_alphabetic() => {
                    let start = self.pos;
                    while let Some(ch) = self.cur() {
                        if ch == b'_' || ch.is_ascii_alphanumeric() {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    // 識別子は ASCII のみ（Lua 5.1 の名前は ASCII 英数字 + '_'）。
                    let word = std::str::from_utf8(&self.src[start..self.pos])
                        .expect("identifier bytes are ASCII");
                    let tok = Token::keyword(word).unwrap_or_else(|| Token::Name(word.to_string()));
                    return self.spanned(tok);
                }
                other => {
                    // 制御文字や非対応バイト。本家 "unexpected symbol near '<char>'".
                    let near = if other.is_ascii_graphic() {
                        (other as char).to_string()
                    } else {
                        format!("char({other})")
                    };
                    return Err(self.error_near("unexpected symbol", &near));
                }
            }
        }
    }

    fn spanned(&self, tok: Token) -> LuaResult<Spanned> {
        Ok(Spanned {
            tok,
            line: self.line,
        })
    }

    /// 本家 `skip_sep`: 長括弧の `=` 個数を数える。
    /// `[` または `]` の上で呼ばれ、同種括弧で閉じれば `=` 個数（>=0）、
    /// そうでなければ `(-count) - 1`（<0）を返す。呼んだ分だけ `pos` を進める。
    fn skip_sep(&mut self) -> i32 {
        let s = self.cur().unwrap(); // '[' or ']'
        let mut count: i32 = 0;
        self.advance();
        while self.cur() == Some(b'=') {
            self.advance();
            count += 1;
        }
        if self.cur() == Some(s) {
            count
        } else {
            -count - 1
        }
    }

    /// 本家 `read_long_string`: `[[`/`[==[` で始まる長文字列・長コメントを読む。
    /// `keep` が true なら内容を返す（文字列）。false なら捨てる（コメント）。
    /// 呼び出し時、開きの `[`・`=`…は消費済みで `cur()` は 2 つ目の `[`。
    fn read_long_string(&mut self, sep: i32, keep: bool) -> LuaResult<Vec<u8>> {
        self.buff.clear();
        self.advance(); // skip 2nd '['
        // 開いた直後の改行は捨てる。
        if let Some(c) = self.cur()
            && Self::is_newline(c)
        {
            self.inc_line()?;
        }
        loop {
            match self.cur() {
                None => {
                    let msg = if keep {
                        "unfinished long string"
                    } else {
                        "unfinished long comment"
                    };
                    return Err(self.error(msg));
                }
                Some(b']') => {
                    // 閉じ括弧 `]` + sep 個の `=` + `]` にマッチするか先読みで判定。
                    let mut k = 1; // skip first ']'
                    while self.peek(k) == Some(b'=') {
                        k += 1;
                    }
                    let count = (k - 1) as i32;
                    if count == sep && self.peek(k) == Some(b']') {
                        self.pos += k + 1; // 閉じ括弧全体を消費
                        break;
                    }
                    // 閉じに失敗。`]` 1 文字だけ保存して次へ（`=` 群は次ループで拾う）。
                    if keep {
                        self.buff.push(b']');
                    }
                    self.advance();
                }
                Some(b'\n') | Some(b'\r') => {
                    if keep {
                        self.buff.push(b'\n');
                    }
                    self.inc_line()?;
                }
                Some(c) => {
                    if keep {
                        self.buff.push(c);
                    }
                    self.advance();
                }
            }
        }
        Ok(std::mem::take(&mut self.buff))
    }

    /// 本家 `read_string`: 短い文字列（`'...'` / `"..."`）を読む。
    fn read_string(&mut self, delim: u8) -> LuaResult<Vec<u8>> {
        self.buff.clear();
        self.advance(); // skip opening delimiter
        loop {
            let c = match self.cur() {
                None => return Err(self.error("unfinished string")),
                Some(c) => c,
            };
            if c == delim {
                self.advance();
                break;
            }
            match c {
                b'\n' | b'\r' => return Err(self.error("unfinished string")),
                b'\\' => {
                    self.advance();
                    let e = match self.cur() {
                        None => continue, // 次ループで unfinished string
                        Some(e) => e,
                    };
                    match e {
                        b'a' => {
                            self.buff.push(0x07);
                            self.advance();
                        }
                        b'b' => {
                            self.buff.push(0x08);
                            self.advance();
                        }
                        b'f' => {
                            self.buff.push(0x0c);
                            self.advance();
                        }
                        b'n' => {
                            self.buff.push(b'\n');
                            self.advance();
                        }
                        b'r' => {
                            self.buff.push(b'\r');
                            self.advance();
                        }
                        b't' => {
                            self.buff.push(b'\t');
                            self.advance();
                        }
                        b'v' => {
                            self.buff.push(0x0b);
                            self.advance();
                        }
                        b'\n' | b'\r' => {
                            // `\` 直後の改行 → '\n' を 1 つ保存し行を進める。
                            self.buff.push(b'\n');
                            self.inc_line()?;
                        }
                        d if d.is_ascii_digit() => {
                            // `\ddd` 10進エスケープ（最大 3 桁, <= 255）。
                            let mut val: u32 = 0;
                            let mut i = 0;
                            while i < 3 {
                                match self.cur() {
                                    Some(dd) if dd.is_ascii_digit() => {
                                        val = val * 10 + (dd - b'0') as u32;
                                        self.advance();
                                        i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if val > 255 {
                                return Err(self.error("escape sequence too large"));
                            }
                            self.buff.push(val as u8);
                        }
                        other => {
                            // 本家: それ以外は文字そのものを保存（`\\`, `\"`, `\'`, `\?` …）。
                            self.buff.push(other);
                            self.advance();
                        }
                    }
                }
                _ => {
                    self.buff.push(c);
                    self.advance();
                }
            }
        }
        Ok(std::mem::take(&mut self.buff))
    }

    /// 本家 `read_numeral`: 数値リテラルを読み f64 に変換する。
    /// 16進整数 `0x..` は strtoul 相当、10進/小数/指数は strtod 相当。
    fn read_numeral(&mut self) -> LuaResult<f64> {
        let start = self.pos;
        // 本家の収集規則: 数字と '.' を読み、E/e の後に符号、その後 alnum/'_' を読む。
        while matches!(self.cur(), Some(c) if c.is_ascii_digit() || c == b'.') {
            self.advance();
        }
        if matches!(self.cur(), Some(b'e') | Some(b'E')) {
            self.advance();
            if matches!(self.cur(), Some(b'+') | Some(b'-')) {
                self.advance();
            }
        }
        while matches!(self.cur(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            self.advance();
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.error("malformed number"))?;
        parse_number(text).ok_or_else(|| self.error_near("malformed number", text))
    }
}

/// 本家 `luaO_str2d` 相当の数値変換。16進整数 (`0x..`) と 10進実数を扱う。
pub fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // 16進整数: 0x / 0X プレフィックス（Lua 5.1 は 16進浮動小数を持たない）。
    let hex = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"));
    if let Some(h) = hex {
        if h.is_empty() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // strtoul 相当: u64 でラップしながら解釈。
        let mut acc: u64 = 0;
        for b in h.bytes() {
            let d = (b as char).to_digit(16).unwrap() as u64;
            acc = acc.wrapping_mul(16).wrapping_add(d);
        }
        return Some(acc as f64);
    }
    // 10進。Rust の f64 パーサは "inf"/"nan"/16進等も受けるため、Lua の文法に合わせて拒否する。
    let bytes = t.as_bytes();
    let mut ok = false;
    for &b in bytes {
        match b {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => {}
            _ => return None,
        }
        if b.is_ascii_digit() {
            ok = true;
        }
    }
    if !ok {
        return None;
    }
    // オーバーフロー（例 `1e400`）は本家 Lua 5.1 同様 inf を返す。
    // "inf"/"nan" 文字列は上の文字検査（数字・`.`・`eE`・符号のみ許可）で既に排除済み。
    t.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_all(src: &str) -> Vec<Token> {
        let mut lx = Lexer::new(src.as_bytes(), "test");
        let mut out = Vec::new();
        loop {
            let s = lx.next_token().expect("lex error");
            if s.tok == Token::Eof {
                break;
            }
            out.push(s.tok);
        }
        out
    }

    #[test]
    fn keywords_and_names() {
        assert_eq!(
            lex_all("local x = function end if then"),
            vec![
                Token::Local,
                Token::Name("x".into()),
                Token::Assign,
                Token::Function,
                Token::End,
                Token::If,
                Token::Then,
            ]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            lex_all("+ - * / % ^ # == ~= <= >= < > = ( ) { } [ ] ; : , . .. ..."),
            vec![
                Token::Plus,
                Token::Minus,
                Token::Star,
                Token::Slash,
                Token::Percent,
                Token::Caret,
                Token::Hash,
                Token::Eq,
                Token::Ne,
                Token::Le,
                Token::Ge,
                Token::Lt,
                Token::Gt,
                Token::Assign,
                Token::LParen,
                Token::RParen,
                Token::LBrace,
                Token::RBrace,
                Token::LBracket,
                Token::RBracket,
                Token::Semicolon,
                Token::Colon,
                Token::Comma,
                Token::Dot,
                Token::Concat,
                Token::Ellipsis,
            ]
        );
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.1416 はパース検証用の入力であり π ではない
    fn numbers() {
        assert_eq!(lex_all("3"), vec![Token::Number(3.0)]);
        assert_eq!(lex_all("3.0"), vec![Token::Number(3.0)]);
        assert_eq!(lex_all("3.1416"), vec![Token::Number(3.1416)]);
        assert_eq!(lex_all("314.16e-2"), vec![Token::Number(3.1416)]);
        assert_eq!(lex_all("0.31416E1"), vec![Token::Number(3.1416)]);
        assert_eq!(lex_all("0xff"), vec![Token::Number(255.0)]);
        assert_eq!(lex_all("0x1A"), vec![Token::Number(26.0)]);
        assert_eq!(lex_all(".5"), vec![Token::Number(0.5)]);
    }

    #[test]
    fn malformed_number() {
        let mut lx = Lexer::new(b"3.3.3", "test");
        assert!(lx.next_token().is_err());
    }

    #[test]
    fn short_string_escapes() {
        assert_eq!(
            lex_all(r#" "a\tb\n" "#),
            vec![Token::Str(b"a\tb\n".to_vec())]
        );
        assert_eq!(
            lex_all(r#" "\65\66\67" "#),
            vec![Token::Str(b"ABC".to_vec())]
        );
        assert_eq!(lex_all(r#" '\\' "#), vec![Token::Str(b"\\".to_vec())]);
        assert_eq!(
            lex_all(r#" "quote: \"" "#),
            vec![Token::Str(b"quote: \"".to_vec())]
        );
    }

    #[test]
    fn line_continuation_in_string() {
        let v = lex_all("\"a\\\nb\"");
        assert_eq!(v, vec![Token::Str(b"a\nb".to_vec())]);
    }

    #[test]
    fn unfinished_string() {
        let mut lx = Lexer::new(b"\"abc", "test");
        assert!(lx.next_token().is_err());
        let mut lx2 = Lexer::new(b"\"ab\nc\"", "test");
        assert!(lx2.next_token().is_err());
    }

    #[test]
    fn long_string_basic() {
        assert_eq!(lex_all("[[hello]]"), vec![Token::Str(b"hello".to_vec())]);
        assert_eq!(lex_all("[==[a]]b]==]"), vec![Token::Str(b"a]]b".to_vec())]);
    }

    #[test]
    fn long_string_first_newline_skipped() {
        assert_eq!(lex_all("[[\nhello]]"), vec![Token::Str(b"hello".to_vec())]);
        assert_eq!(
            lex_all("[[\nline1\nline2]]"),
            vec![Token::Str(b"line1\nline2".to_vec())]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(lex_all("-- a comment\n42"), vec![Token::Number(42.0)]);
        assert_eq!(
            lex_all("--[[ long\ncomment ]] 42"),
            vec![Token::Number(42.0)]
        );
        assert_eq!(lex_all("--[==[ x ]] y ]==] 7"), vec![Token::Number(7.0)]);
    }

    #[test]
    fn lonely_bracket_is_token() {
        // '[' に続いて長括弧でない → '[' トークン。
        assert_eq!(
            lex_all("a[1]"),
            vec![
                Token::Name("a".into()),
                Token::LBracket,
                Token::Number(1.0),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn line_tracking() {
        let mut lx = Lexer::new(b"a\nb\r\nc", "test");
        let a = lx.next_token().unwrap();
        assert_eq!(a.line, 1);
        let b = lx.next_token().unwrap();
        assert_eq!(b.line, 2);
        let c = lx.next_token().unwrap();
        assert_eq!(c.line, 3);
    }

    #[test]
    fn shebang_skipped() {
        assert_eq!(lex_all("#!/usr/bin/lua\nreturn"), vec![Token::Return]);
    }

    #[test]
    fn invalid_long_delimiter() {
        // "[=" の後に '[' が来ない → invalid long string delimiter
        let mut lx = Lexer::new(b"[=x", "test");
        assert!(lx.next_token().is_err());
    }
}
