//! 構文解析（本家 `lparser.c` 相当）。担当: **lua-frontend**。
//!
//! Lua 5.1 文法の再帰下降パーサ。[`Lexer`] からトークンを 1 つ先読み付きで取得し、
//! [`ast::Block`] を構築する。演算子優先順位・右結合（`..`/`^`）・単項演算子は
//! 本家 `subexpr` の優先度表を忠実に再現する。エラー文言は本家 `lparser.c` に合わせる。

use crate::compiler::ast::*;
use crate::compiler::lexer::{Lexer, Spanned, Token};
use crate::error::{LuaError, LuaResult};

/// 本家 `LUAI_MAXCCALLS`。再帰の深さ上限（Rust スタック保護も兼ねる）。
const MAX_LEVELS: u32 = 200;

/// 再帰下降パーサ。
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    /// 現在のトークン。
    tok: Spanned,
    /// 1 トークン先読み（未取得なら `None`）。
    ahead: Option<Spanned>,
    /// 再帰の深さ（`enterlevel`/`leavelevel` 相当）。
    level: u32,
}

impl<'a> Parser<'a> {
    /// ソースと（短縮済み）チャンク名からパーサを構築し、チャンクを解析する。
    pub fn parse(src: &'a [u8], chunk: impl Into<String>) -> LuaResult<Block> {
        let mut lexer = Lexer::new(src, chunk);
        let first = lexer.next_token()?;
        let mut p = Parser {
            lexer,
            tok: first,
            ahead: None,
            level: 0,
        };
        let block = p.chunk()?;
        p.expect_eof()?;
        Ok(block)
    }

    // ---- トークン操作 ----

    fn line(&self) -> u32 {
        self.tok.line
    }

    /// 現在トークンを次へ進める。
    fn advance(&mut self) -> LuaResult<()> {
        self.tok = match self.ahead.take() {
            Some(t) => t,
            None => self.lexer.next_token()?,
        };
        Ok(())
    }

    /// 1 トークン先読み（現在トークンは変えない）。
    fn lookahead(&mut self) -> LuaResult<&Token> {
        if self.ahead.is_none() {
            self.ahead = Some(self.lexer.next_token()?);
        }
        Ok(&self.ahead.as_ref().unwrap().tok)
    }

    fn check(&self, t: &Token) -> bool {
        &self.tok.tok == t
    }

    /// 現在トークンが `t` なら消費して true。
    fn test_next(&mut self, t: &Token) -> LuaResult<bool> {
        if self.check(t) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `t` を期待し、消費する。違えば構文エラー。
    fn expect(&mut self, t: &Token) -> LuaResult<()> {
        if self.check(t) {
            self.advance()
        } else {
            Err(self.error_expected(&t.display()))
        }
    }

    /// 名前トークンを期待し、その文字列を返す。
    fn expect_name(&mut self) -> LuaResult<String> {
        match &self.tok.tok {
            Token::Name(s) => {
                let s = s.clone();
                self.advance()?;
                Ok(s)
            }
            _ => Err(self.error_expected("<name>")),
        }
    }

    fn expect_eof(&mut self) -> LuaResult<()> {
        if self.check(&Token::Eof) {
            Ok(())
        } else {
            Err(self.error_near("'<eof>' expected"))
        }
    }

    /// `what` の開きと `who` 行に対応する閉じトークンを期待する（本家 `check_match`）。
    fn check_match(&mut self, close: &Token, open: &Token, open_line: u32) -> LuaResult<()> {
        if self.check(close) {
            return self.advance();
        }
        let msg = if open_line == self.line() {
            format!("'{}' expected", close.display())
        } else {
            format!(
                "'{}' expected (to close '{}' at line {})",
                close.display(),
                open.display(),
                open_line
            )
        };
        Err(self.error_near_msg(&msg))
    }

    // ---- エラー生成（本家 luaX_syntaxerror 形式） ----

    fn error_near_msg(&self, msg: &str) -> LuaError {
        let near = self.tok.tok.display();
        LuaError::Syntax(format!(
            "{}:{}: {} near '{}'",
            self.lexer.chunk(),
            self.line(),
            msg,
            near
        ))
    }

    fn error_near(&self, msg: &str) -> LuaError {
        self.error_near_msg(msg)
    }

    fn error_expected(&self, what: &str) -> LuaError {
        self.error_near_msg(&format!("'{what}' expected"))
    }

    fn enter_level(&mut self) -> LuaResult<()> {
        self.level += 1;
        if self.level > MAX_LEVELS {
            return Err(self.error_near("chunk has too many syntax levels"));
        }
        Ok(())
    }

    fn leave_level(&mut self) {
        self.level -= 1;
    }

    // ---- ブロック / 文 ----

    /// `block_follow`: ブロックを終える可能性のあるトークンか。
    fn block_follow(&self) -> bool {
        matches!(
            self.tok.tok,
            Token::Else | Token::Elseif | Token::End | Token::Until | Token::Eof
        )
    }

    /// `chunk -> { stat [';'] }`。`return`/`break` は最後の文。
    fn chunk(&mut self) -> LuaResult<Block> {
        self.enter_level()?;
        let mut stmts = Vec::new();
        let mut is_last = false;
        while !is_last && !self.block_follow() {
            let (stmt, last) = self.statement()?;
            is_last = last;
            stmts.push(stmt);
            // セミコロンは任意。
            self.test_next(&Token::Semicolon)?;
        }
        self.leave_level();
        Ok(Block { stmts })
    }

    /// ブロック（`chunk` と同義。スコープは codegen 側で管理）。
    fn block(&mut self) -> LuaResult<Block> {
        self.chunk()
    }

    /// 1 文を解析。戻り値の bool は「最後の文か（return/break）」。
    fn statement(&mut self) -> LuaResult<(Stmt, bool)> {
        let line = self.line();
        let (kind, is_last) = match &self.tok.tok {
            Token::If => (self.if_stat()?, false),
            Token::While => (self.while_stat()?, false),
            Token::Do => {
                self.advance()?;
                let b = self.block()?;
                self.check_match(&Token::End, &Token::Do, line)?;
                (StmtKind::Do(b), false)
            }
            Token::For => (self.for_stat()?, false),
            Token::Repeat => (self.repeat_stat()?, false),
            Token::Function => (self.func_stat()?, false),
            Token::Local => {
                self.advance()?;
                if self.test_next(&Token::Function)? {
                    (self.local_function()?, false)
                } else {
                    (self.local_stat()?, false)
                }
            }
            Token::Return => (self.return_stat()?, true),
            Token::Break => {
                self.advance()?;
                (StmtKind::Break, true)
            }
            _ => (self.expr_stat()?, false),
        };
        Ok((Stmt { kind, line }, is_last))
    }

    /// `if cond then block {elseif cond then block} [else block] end`。
    fn if_stat(&mut self) -> LuaResult<StmtKind> {
        let if_line = self.line();
        self.advance()?; // if
        let mut arms = Vec::new();
        let cond = self.expr()?;
        self.expect(&Token::Then)?;
        let body = self.block()?;
        arms.push((cond, body));
        while self.check(&Token::Elseif) {
            self.advance()?;
            let c = self.expr()?;
            self.expect(&Token::Then)?;
            let b = self.block()?;
            arms.push((c, b));
        }
        let else_block = if self.test_next(&Token::Else)? {
            Some(self.block()?)
        } else {
            None
        };
        self.check_match(&Token::End, &Token::If, if_line)?;
        Ok(StmtKind::If { arms, else_block })
    }

    /// `while cond do block end`。
    fn while_stat(&mut self) -> LuaResult<StmtKind> {
        let line = self.line();
        self.advance()?; // while
        let cond = self.expr()?;
        self.expect(&Token::Do)?;
        let body = self.block()?;
        self.check_match(&Token::End, &Token::While, line)?;
        Ok(StmtKind::While { cond, body })
    }

    /// `repeat block until cond`。
    fn repeat_stat(&mut self) -> LuaResult<StmtKind> {
        let line = self.line();
        self.advance()?; // repeat
        let body = self.block()?;
        self.check_match(&Token::Until, &Token::Repeat, line)?;
        let cond = self.expr()?;
        Ok(StmtKind::Repeat { body, cond })
    }

    /// `for` 文（数値 / 汎用）。
    fn for_stat(&mut self) -> LuaResult<StmtKind> {
        let for_line = self.line();
        self.advance()?; // for
        let first = self.expect_name()?;
        match &self.tok.tok {
            Token::Assign => {
                // 数値 for
                self.advance()?;
                let start = self.expr()?;
                self.expect(&Token::Comma)?;
                let limit = self.expr()?;
                let step = if self.test_next(&Token::Comma)? {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect(&Token::Do)?;
                let body = self.block()?;
                self.check_match(&Token::End, &Token::For, for_line)?;
                Ok(StmtKind::NumericFor {
                    var: first,
                    start,
                    limit,
                    step,
                    body,
                })
            }
            Token::Comma | Token::In => {
                // 汎用 for
                let mut names = vec![first];
                while self.test_next(&Token::Comma)? {
                    names.push(self.expect_name()?);
                }
                self.expect(&Token::In)?;
                let exprs = self.expr_list()?;
                self.expect(&Token::Do)?;
                let body = self.block()?;
                self.check_match(&Token::End, &Token::For, for_line)?;
                Ok(StmtKind::GenericFor { names, exprs, body })
            }
            _ => Err(self.error_near("'=' or 'in' expected")),
        }
    }

    /// `function funcname funcbody`。funcname -> NAME {'.' NAME} [':' NAME]。
    fn func_stat(&mut self) -> LuaResult<StmtKind> {
        self.advance()?; // function
        let base = self.expect_name()?;
        let mut fields = Vec::new();
        while self.test_next(&Token::Dot)? {
            fields.push(self.expect_name()?);
        }
        let method = if self.test_next(&Token::Colon)? {
            Some(self.expect_name()?)
        } else {
            None
        };
        let is_method = method.is_some();
        let name = FuncName {
            base,
            fields,
            method,
        };
        let body = self.func_body(is_method)?;
        Ok(StmtKind::Function { name, body })
    }

    /// `local function Name funcbody`。
    fn local_function(&mut self) -> LuaResult<StmtKind> {
        let name = self.expect_name()?;
        let body = self.func_body(false)?;
        Ok(StmtKind::LocalFunction { name, body })
    }

    /// `local Name {',' Name} ['=' explist]`。
    fn local_stat(&mut self) -> LuaResult<StmtKind> {
        let mut names = vec![self.expect_name()?];
        while self.test_next(&Token::Comma)? {
            names.push(self.expect_name()?);
        }
        let exprs = if self.test_next(&Token::Assign)? {
            self.expr_list()?
        } else {
            Vec::new()
        };
        Ok(StmtKind::Local { names, exprs })
    }

    /// `return [explist] [';']`。
    fn return_stat(&mut self) -> LuaResult<StmtKind> {
        self.advance()?; // return
        let exprs = if self.block_follow() || self.check(&Token::Semicolon) {
            Vec::new()
        } else {
            self.expr_list()?
        };
        Ok(StmtKind::Return(exprs))
    }

    /// 式文: 関数呼び出し または 代入。
    fn expr_stat(&mut self) -> LuaResult<StmtKind> {
        let first = self.suffixed_expr()?;
        if self.check(&Token::Assign) || self.check(&Token::Comma) {
            // 代入。最初のターゲットを含めた左辺リストを収集。
            let mut targets = vec![self.check_assignable(first)?];
            while self.test_next(&Token::Comma)? {
                let e = self.suffixed_expr()?;
                targets.push(self.check_assignable(e)?);
            }
            self.expect(&Token::Assign)?;
            let exprs = self.expr_list()?;
            Ok(StmtKind::Assign { targets, exprs })
        } else {
            // 関数呼び出しでなければ構文エラー（本家 "syntax error"）。
            match first.kind {
                ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                    Ok(StmtKind::ExprStat(first))
                }
                _ => Err(self.error_near("syntax error")),
            }
        }
    }

    /// 代入対象が左辺値（Name か Index）であることを検査。
    fn check_assignable(&self, e: Expr) -> LuaResult<Expr> {
        match e.kind {
            ExprKind::Name(_) | ExprKind::Index { .. } => Ok(e),
            _ => Err(self.error_near("syntax error")),
        }
    }

    // ---- 関数本体 ----

    /// `funcbody -> '(' [parlist] ')' block end`。
    /// `is_method` が true なら暗黙の `self` を先頭パラメータに加える。
    fn func_body(&mut self, is_method: bool) -> LuaResult<FuncBody> {
        let line = self.line();
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        if is_method {
            params.push("self".to_string());
        }
        let mut is_vararg = false;
        if !self.check(&Token::RParen) {
            loop {
                match &self.tok.tok {
                    Token::Ellipsis => {
                        self.advance()?;
                        is_vararg = true;
                        break; // '...' は仮引数リストの最後
                    }
                    Token::Name(n) => {
                        params.push(n.clone());
                        self.advance()?;
                    }
                    _ => return Err(self.error_near("<name> or '...' expected")),
                }
                if !self.test_next(&Token::Comma)? {
                    break;
                }
            }
        }
        self.expect(&Token::RParen)?;
        let body = self.block()?;
        let last_line = self.line();
        self.check_match(&Token::End, &Token::Function, line)?;
        Ok(FuncBody {
            params,
            is_vararg,
            body,
            line,
            last_line,
        })
    }

    // ---- 式 ----

    /// `explist -> expr {',' expr}`。
    fn expr_list(&mut self) -> LuaResult<Vec<Expr>> {
        let mut list = vec![self.expr()?];
        while self.test_next(&Token::Comma)? {
            list.push(self.expr()?);
        }
        Ok(list)
    }

    /// 完全な式（優先度 0）。
    fn expr(&mut self) -> LuaResult<Expr> {
        let (e, _) = self.subexpr(0)?;
        Ok(e)
    }

    /// 本家 `subexpr`: 単項演算子・二項演算子の優先度に従って式を構築。
    /// 戻り値の `Option<BinOp>` は未処理の次の二項演算子（呼び出し側で再利用）。
    fn subexpr(&mut self, limit: u8) -> LuaResult<(Expr, Option<BinOp>)> {
        self.enter_level()?;
        let line = self.line();
        let mut left = if let Some(uop) = unop_of(&self.tok.tok) {
            self.advance()?;
            let (operand, _) = self.subexpr(UNARY_PRIORITY)?;
            // 定数畳み込み（`-5` 等）は codegen の constfolding に委ねる（luac と定数表を一致させるため）。
            Expr {
                kind: ExprKind::UnOp {
                    op: uop,
                    expr: Box::new(operand),
                },
                line,
            }
        } else {
            self.simple_expr()?
        };

        // 優先度が limit より高い二項演算子を展開。
        let mut op = binop_of(&self.tok.tok);
        while let Some(o) = op {
            let (lp, rp) = o.priority();
            if lp <= limit {
                break;
            }
            let op_line = self.line();
            self.advance()?;
            let (right, next_op) = self.subexpr(rp)?;
            left = Expr {
                kind: ExprKind::BinOp {
                    op: o,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                line: op_line,
            };
            op = next_op;
        }
        self.leave_level();
        Ok((left, op))
    }

    /// `simpleexp`: 単純式（リテラル・テーブル・関数・suffixedexp）。
    fn simple_expr(&mut self) -> LuaResult<Expr> {
        let line = self.line();
        let kind = match &self.tok.tok {
            Token::Number(n) => {
                let n = *n;
                self.advance()?;
                ExprKind::Number(n)
            }
            Token::Str(s) => {
                let s = s.clone();
                self.advance()?;
                ExprKind::Str(s)
            }
            Token::Nil => {
                self.advance()?;
                ExprKind::Nil
            }
            Token::True => {
                self.advance()?;
                ExprKind::True
            }
            Token::False => {
                self.advance()?;
                ExprKind::False
            }
            Token::Ellipsis => {
                self.advance()?;
                ExprKind::Vararg
            }
            Token::LBrace => return self.table_constructor(),
            Token::Function => {
                self.advance()?;
                let body = self.func_body(false)?;
                ExprKind::Function(body)
            }
            _ => return self.suffixed_expr(),
        };
        Ok(Expr { kind, line })
    }

    /// `primaryexp -> NAME | '(' expr ')'`。
    fn primary_expr(&mut self) -> LuaResult<Expr> {
        let line = self.line();
        match &self.tok.tok {
            Token::LParen => {
                self.advance()?;
                let e = self.expr()?;
                self.check_match(&Token::RParen, &Token::LParen, line)?;
                Ok(Expr {
                    kind: ExprKind::Paren(Box::new(e)),
                    line,
                })
            }
            Token::Name(n) => {
                let n = n.clone();
                self.advance()?;
                Ok(Expr {
                    kind: ExprKind::Name(n),
                    line,
                })
            }
            _ => Err(self.error_near("unexpected symbol")),
        }
    }

    /// `suffixedexp -> primaryexp { '.' NAME | '[' exp ']' | ':' NAME funcargs | funcargs }`。
    fn suffixed_expr(&mut self) -> LuaResult<Expr> {
        let mut e = self.primary_expr()?;
        loop {
            let line = self.line();
            match &self.tok.tok {
                Token::Dot => {
                    self.advance()?;
                    let name = self.expect_name()?;
                    e = Expr {
                        kind: ExprKind::Index {
                            obj: Box::new(e),
                            key: Box::new(Expr {
                                kind: ExprKind::Str(name.into_bytes()),
                                line,
                            }),
                        },
                        line,
                    };
                }
                Token::LBracket => {
                    self.advance()?;
                    let key = self.expr()?;
                    self.expect(&Token::RBracket)?;
                    e = Expr {
                        kind: ExprKind::Index {
                            obj: Box::new(e),
                            key: Box::new(key),
                        },
                        line,
                    };
                }
                Token::Colon => {
                    self.advance()?;
                    let method = self.expect_name()?;
                    let args = self.func_args()?;
                    e = Expr {
                        kind: ExprKind::MethodCall {
                            obj: Box::new(e),
                            method,
                            args,
                        },
                        line,
                    };
                }
                Token::LParen | Token::Str(_) | Token::LBrace => {
                    let args = self.func_args()?;
                    e = Expr {
                        kind: ExprKind::Call {
                            func: Box::new(e),
                            args,
                        },
                        line,
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    /// `funcargs -> '(' [explist] ')' | tableconstructor | STRING`。
    fn func_args(&mut self) -> LuaResult<Vec<Expr>> {
        match &self.tok.tok {
            Token::LParen => {
                let line = self.line();
                self.advance()?;
                let args = if self.check(&Token::RParen) {
                    Vec::new()
                } else {
                    self.expr_list()?
                };
                self.check_match(&Token::RParen, &Token::LParen, line)?;
                Ok(args)
            }
            Token::Str(s) => {
                let line = self.line();
                let s = s.clone();
                self.advance()?;
                Ok(vec![Expr {
                    kind: ExprKind::Str(s),
                    line,
                }])
            }
            Token::LBrace => {
                let t = self.table_constructor()?;
                Ok(vec![t])
            }
            _ => Err(self.error_near("function arguments expected")),
        }
    }

    /// `tableconstructor -> '{' [fieldlist] '}'`。
    /// `fieldlist -> field { fieldsep field } [fieldsep]`、`fieldsep -> ',' | ';'`。
    fn table_constructor(&mut self) -> LuaResult<Expr> {
        let line = self.line();
        self.expect(&Token::LBrace)?;
        let mut fields = Vec::new();
        while !self.check(&Token::RBrace) {
            // NAME の直後が '=' なら名前付きフィールド。借用が重ならないよう先に判定する。
            let is_named = matches!(self.tok.tok, Token::Name(_))
                && *self.lookahead()? == Token::Assign;
            if self.check(&Token::LBracket) {
                // [exp] = exp
                self.advance()?;
                let key = self.expr()?;
                self.expect(&Token::RBracket)?;
                self.expect(&Token::Assign)?;
                let val = self.expr()?;
                fields.push(Field::Keyed(key, val));
            } else if is_named {
                // NAME = exp
                let name = self.expect_name()?;
                self.expect(&Token::Assign)?;
                let val = self.expr()?;
                fields.push(Field::Named(name, val));
            } else {
                // exp （位置フィールド）
                let val = self.expr()?;
                fields.push(Field::Positional(val));
            }
            // フィールド区切り ',' または ';'。なければ終了。
            if !self.test_next(&Token::Comma)? && !self.test_next(&Token::Semicolon)? {
                break;
            }
        }
        self.check_match(&Token::RBrace, &Token::LBrace, line)?;
        Ok(Expr {
            kind: ExprKind::Table(fields),
            line,
        })
    }
}

/// トークン → 二項演算子。
fn binop_of(t: &Token) -> Option<BinOp> {
    Some(match t {
        Token::Plus => BinOp::Add,
        Token::Minus => BinOp::Sub,
        Token::Star => BinOp::Mul,
        Token::Slash => BinOp::Div,
        Token::Percent => BinOp::Mod,
        Token::Caret => BinOp::Pow,
        Token::Concat => BinOp::Concat,
        Token::Eq => BinOp::Eq,
        Token::Ne => BinOp::Ne,
        Token::Lt => BinOp::Lt,
        Token::Le => BinOp::Le,
        Token::Gt => BinOp::Gt,
        Token::Ge => BinOp::Ge,
        Token::And => BinOp::And,
        Token::Or => BinOp::Or,
        _ => return None,
    })
}

/// トークン → 単項演算子。
fn unop_of(t: &Token) -> Option<UnOp> {
    Some(match t {
        Token::Minus => UnOp::Neg,
        Token::Not => UnOp::Not,
        Token::Hash => UnOp::Len,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> LuaResult<Block> {
        Parser::parse(src.as_bytes(), "test")
    }

    fn parse_ok(src: &str) -> Block {
        parse(src).unwrap_or_else(|e| panic!("parse failed: {e}\nsrc: {src}"))
    }

    #[test]
    fn empty_chunk() {
        assert_eq!(parse_ok(""), Block { stmts: vec![] });
    }

    #[test]
    fn local_assignment() {
        let b = parse_ok("local a, b = 1, 2");
        assert_eq!(b.stmts.len(), 1);
        match &b.stmts[0].kind {
            StmtKind::Local { names, exprs } => {
                assert_eq!(names, &["a", "b"]);
                assert_eq!(exprs.len(), 2);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn assignment_targets() {
        let b = parse_ok("a, t.x, t[1] = 1, 2, 3");
        match &b.stmts[0].kind {
            StmtKind::Assign { targets, exprs } => {
                assert_eq!(targets.len(), 3);
                assert_eq!(exprs.len(), 3);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn non_lvalue_assignment_errors() {
        assert!(parse("1 = 2").is_err());
        assert!(parse("f() = 2").is_err());
    }

    #[test]
    fn call_statement() {
        let b = parse_ok("print('hi')");
        assert!(matches!(b.stmts[0].kind, StmtKind::ExprStat(_)));
    }

    #[test]
    fn method_call() {
        let b = parse_ok("obj:method(1, 2)");
        match &b.stmts[0].kind {
            StmtKind::ExprStat(Expr {
                kind: ExprKind::MethodCall { method, args, .. },
                ..
            }) => {
                assert_eq!(method, "method");
                assert_eq!(args.len(), 2);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn operator_precedence() {
        // 1 + 2 * 3  ==> 1 + (2 * 3)
        let b = parse_ok("return 1 + 2 * 3");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => match &es[0].kind {
                ExprKind::BinOp { op, lhs, rhs } => {
                    assert_eq!(*op, BinOp::Add);
                    assert!(matches!(lhs.kind, ExprKind::Number(_)));
                    assert!(matches!(rhs.kind, ExprKind::BinOp { op: BinOp::Mul, .. }));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn pow_right_assoc() {
        // 2 ^ 3 ^ 2 ==> 2 ^ (3 ^ 2)
        let b = parse_ok("return 2 ^ 3 ^ 2");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => match &es[0].kind {
                ExprKind::BinOp { op: BinOp::Pow, rhs, .. } => {
                    assert!(matches!(rhs.kind, ExprKind::BinOp { op: BinOp::Pow, .. }));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn concat_right_assoc() {
        // 'a' .. 'b' .. 'c' ==> 'a' .. ('b' .. 'c')
        let b = parse_ok("return 'a' .. 'b' .. 'c'");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => match &es[0].kind {
                ExprKind::BinOp { op: BinOp::Concat, rhs, .. } => {
                    assert!(matches!(rhs.kind, ExprKind::BinOp { op: BinOp::Concat, .. }));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unary_neg_not_folded_in_parser() {
        // 定数畳み込みは codegen 側で行うため、parser では UnOp のまま。
        let b = parse_ok("return -5");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => {
                assert!(matches!(es[0].kind, ExprKind::UnOp { op: UnOp::Neg, .. }))
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unary_vs_binary() {
        // -2 ^ 2 ==> -(2 ^ 2) because unary priority (8) < pow left (10)
        let b = parse_ok("return -2 ^ 2");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => {
                assert!(matches!(
                    es[0].kind,
                    ExprKind::UnOp { op: UnOp::Neg, .. }
                ));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn if_elseif_else() {
        let b = parse_ok("if a then return 1 elseif b then return 2 else return 3 end");
        match &b.stmts[0].kind {
            StmtKind::If { arms, else_block } => {
                assert_eq!(arms.len(), 2);
                assert!(else_block.is_some());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn numeric_for() {
        let b = parse_ok("for i = 1, 10, 2 do end");
        match &b.stmts[0].kind {
            StmtKind::NumericFor { var, step, .. } => {
                assert_eq!(var, "i");
                assert!(step.is_some());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn generic_for() {
        let b = parse_ok("for k, v in pairs(t) do end");
        match &b.stmts[0].kind {
            StmtKind::GenericFor { names, exprs, .. } => {
                assert_eq!(names, &["k", "v"]);
                assert_eq!(exprs.len(), 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn while_repeat() {
        assert!(matches!(
            parse_ok("while true do end").stmts[0].kind,
            StmtKind::While { .. }
        ));
        assert!(matches!(
            parse_ok("repeat until x").stmts[0].kind,
            StmtKind::Repeat { .. }
        ));
    }

    #[test]
    fn function_statement_method() {
        let b = parse_ok("function a.b:c(x) return x end");
        match &b.stmts[0].kind {
            StmtKind::Function { name, body } => {
                assert_eq!(name.base, "a");
                assert_eq!(name.fields, &["b"]);
                assert_eq!(name.method.as_deref(), Some("c"));
                // 暗黙の self が先頭に入る
                assert_eq!(body.params, &["self", "x"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn local_function() {
        let b = parse_ok("local function f(...) return ... end");
        match &b.stmts[0].kind {
            StmtKind::LocalFunction { name, body } => {
                assert_eq!(name, "f");
                assert!(body.is_vararg);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn table_constructor() {
        let b = parse_ok("return { 1, 2, x = 3, [4] = 5; 6 }");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => match &es[0].kind {
                ExprKind::Table(fields) => {
                    assert_eq!(fields.len(), 5);
                    assert!(matches!(fields[0], Field::Positional(_)));
                    assert!(matches!(fields[2], Field::Named(_, _)));
                    assert!(matches!(fields[3], Field::Keyed(_, _)));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn return_must_be_last() {
        // return の後に文が続くとエラー（block_follow でないため）。
        assert!(parse("return 1 print(2)").is_err());
        // ただし ';' は許される。
        assert!(parse("return 1;").is_ok());
    }

    #[test]
    fn nested_index_and_call() {
        let b = parse_ok("a.b.c[d]:e(f).g = 1");
        assert!(matches!(b.stmts[0].kind, StmtKind::Assign { .. }));
    }

    #[test]
    fn paren_expr() {
        let b = parse_ok("return (f())");
        match &b.stmts[0].kind {
            StmtKind::Return(es) => assert!(matches!(es[0].kind, ExprKind::Paren(_))),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unclosed_block_error() {
        assert!(parse("if a then").is_err());
        assert!(parse("do").is_err());
        assert!(parse("function f()").is_err());
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        // 深いネストでもパニックせずエラーになること（スタック保護）。
        let src = "return ".to_string() + &"(".repeat(1000) + "1" + &")".repeat(1000);
        assert!(parse(&src).is_err());
    }
}
