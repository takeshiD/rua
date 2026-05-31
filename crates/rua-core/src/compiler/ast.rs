//! 抽象構文木（AST）。担当: **lua-frontend**。
//!
//! 本家 `lparser.c` はワンパスで直接コード生成するが、rua では保守性・テスト容易性・
//! `Result` ベースのエラー処理との親和性のため、parser が AST を構築し codegen が
//! それを消費する 2 段構成を採る（最終出力は本家 `luac` 互換バイトコードを目標とし、
//! codegen 段で本家の評価順・レジスタ割付・定数畳み込みを忠実に再現する）。

/// 文の並び（本家の `chunk` / `block`）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

/// 行番号付きの文。
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `local a, b = e1, e2`（右辺は省略可）。
    Local {
        names: Vec<String>,
        exprs: Vec<Expr>,
    },
    /// `local function f() ... end`。
    LocalFunction { name: String, body: FuncBody },
    /// `lhs1, lhs2 = e1, e2`（`lhs` は Name か Index のみ）。
    Assign {
        targets: Vec<Expr>,
        exprs: Vec<Expr>,
    },
    /// 式文（関数呼び出し）。`Expr` は Call / MethodCall のみ。
    ExprStat(Expr),
    /// `do ... end`。
    Do(Block),
    /// `while cond do ... end`。
    While { cond: Expr, body: Block },
    /// `repeat ... until cond`。
    Repeat { body: Block, cond: Expr },
    /// `if c1 then b1 elseif c2 then b2 ... [else be] end`。
    If {
        arms: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    /// `for v = start, limit [, step] do ... end`。
    NumericFor {
        var: String,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Block,
    },
    /// `for n1, n2 in explist do ... end`。
    GenericFor {
        names: Vec<String>,
        exprs: Vec<Expr>,
        body: Block,
    },
    /// `function a.b.c:m() ... end`。
    Function { name: FuncName, body: FuncBody },
    /// `return [explist]`。
    Return(Vec<Expr>),
    /// `break`。
    Break,
}

/// 関数文の名前 `a.b.c:m`。
#[derive(Debug, Clone, PartialEq)]
pub struct FuncName {
    pub base: String,
    /// `.field` の連なり（`a.b.c` なら `["b", "c"]`）。
    pub fields: Vec<String>,
    /// `:method`（あれば暗黙の `self` 引数を持つ）。
    pub method: Option<String>,
}

/// 関数本体（パラメータ・可変長・本体ブロック・行情報）。
#[derive(Debug, Clone, PartialEq)]
pub struct FuncBody {
    pub params: Vec<String>,
    pub is_vararg: bool,
    pub body: Block,
    /// 定義行（`function` キーワードの行）。
    pub line: u32,
    /// 終了行（`end` の行）。
    pub last_line: u32,
}

/// 行番号付きの式。
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Nil,
    True,
    False,
    Number(f64),
    Str(Vec<u8>),
    /// `...`（可変長引数）。
    Vararg,
    /// 変数参照（codegen で local/upvalue/global に解決される）。
    Name(String),
    /// `obj[key]`（`obj.field` は key=Str(field) に正規化）。
    Index {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    /// `func(args)`。
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `obj:method(args)`。
    MethodCall {
        obj: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    /// 関数リテラル `function(...) ... end`。
    Function(FuncBody),
    /// テーブルコンストラクタ `{ ... }`。
    Table(Vec<Field>),
    /// 二項演算。
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// 単項演算。
    UnOp {
        op: UnOp,
        expr: Box<Expr>,
    },
    /// 括弧式 `(e)`。多値を 1 値へ切り詰める意味を持つ（本家 VRELOCABLE 調整）。
    Paren(Box<Expr>),
}

/// テーブルコンストラクタの要素。
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    /// 位置フィールド（`{ v }`）。配列部に順に格納。
    Positional(Expr),
    /// 名前付き（`{ name = v }`）。キーは文字列。
    Named(String, Expr),
    /// 計算キー（`{ [k] = v }`）。
    Keyed(Expr, Expr),
}

/// 二項演算子（本家 `BinOpr` の順序に対応）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// 単項演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-`（符号反転）。
    Neg,
    /// `not`。
    Not,
    /// `#`（長さ）。
    Len,
}

impl BinOp {
    /// 本家 `priority[]` の (left, right) 優先度。右結合（`^`, `..`）は right < left。
    pub fn priority(self) -> (u8, u8) {
        match self {
            BinOp::Add | BinOp::Sub => (6, 6),
            BinOp::Mul | BinOp::Div | BinOp::Mod => (7, 7),
            BinOp::Pow => (10, 9),
            BinOp::Concat => (5, 4),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => (3, 3),
            BinOp::And => (2, 2),
            BinOp::Or => (1, 1),
        }
    }
}

/// 単項演算子の優先度（本家 `UNARY_PRIORITY`）。
pub const UNARY_PRIORITY: u8 = 8;
