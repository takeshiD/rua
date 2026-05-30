//! 関数プロトタイプ（本家 `lobject.c` の `Proto` 相当）。担当: **lua-vm**（frontend と共有）。
//!
//! [`Proto`] は 1 つの Lua 関数のコンパイル結果（命令列・定数表・ネスト関数・デバッグ情報）を
//! 保持する不変オブジェクトである。codegen（lua-frontend）が生成し、`CLOSURE` 命令で
//! [`crate::value::closure::LuaClosure`] が参照する。
//!
//! # GC との関係
//! `Proto` は GC アリーナではなく [`Rc`] で共有する（不変・循環なし）。ただし
//! [`Proto::constants`] にはインターン済み Lua 文字列（[`Value::GcRef`]）が含まれうるため、
//! Proto を保持するクロージャの [`Trace`](crate::gc::Trace) 実装が
//! [`Proto::trace_constants`] を呼んで定数表を mark する（さもないと誤回収される）。
//!
//! # frontend との契約
//! `CLOSURE Bx` は `protos[Bx]` を実体化し、続く `num_upvalues` 個の **疑似命令**
//! （`MOVE B` = 親レジスタ R(B) を捕捉 / `GETUPVAL B` = 親 upvalue[B] を捕捉）で
//! upvalue を束ねる（本家 Lua 5.1 と同一方式）。

use std::rc::Rc;

use crate::gc::Tracer;
use crate::value::Value;

use super::opcode::Instruction;

/// ローカル変数のデバッグ情報（本家 `LocVar`）。
#[derive(Debug, Clone)]
pub struct LocalVar {
    /// 変数名。
    pub name: String,
    /// 有効範囲の開始 pc。
    pub start_pc: u32,
    /// 有効範囲の終了 pc。
    pub end_pc: u32,
}

/// Lua 関数プロトタイプ（本家 `Proto`）。
#[derive(Debug, Default)]
pub struct Proto {
    /// 命令列。
    pub code: Vec<Instruction>,
    /// 定数表（`Kst(i)`）。数値・真偽・nil・インターン済み文字列を含む。
    pub constants: Vec<Value>,
    /// ネストした子プロトタイプ（`CLOSURE Bx` の対象）。
    pub protos: Vec<Rc<Proto>>,
    /// 仮引数の数。
    pub num_params: u8,
    /// 可変長引数（`...`）を取るか。
    pub is_vararg: bool,
    /// 必要なレジスタ数（本家 `maxstacksize`）。
    pub max_stack_size: u8,
    /// upvalue の数（`CLOSURE` 後に続く捕捉疑似命令の個数）。
    pub num_upvalues: u8,
    /// ソース名（チャンク名, デバッグ/エラー表示用）。
    pub source: Option<String>,
    /// 関数定義の開始行。
    pub line_defined: u32,
    /// 関数定義の終了行。
    pub last_line_defined: u32,
    /// 命令ごとのソース行番号（`code` と同じ長さ。エラー位置表示用）。
    pub line_info: Vec<u32>,
    /// upvalue 名（デバッグ用）。
    pub upvalue_names: Vec<String>,
    /// ローカル変数情報（デバッグ用）。
    pub local_vars: Vec<LocalVar>,
}

impl Proto {
    pub fn new() -> Self {
        Proto::default()
    }

    /// `pc` 番目の命令に対応するソース行を返す（無ければ 0）。
    pub fn line_at(&self, pc: usize) -> u32 {
        self.line_info.get(pc).copied().unwrap_or(0)
    }

    /// 定数表と子 proto の定数を再帰的に mark する。
    ///
    /// このメソッドはクロージャの [`Trace`](crate::gc::Trace) 実装から呼ばれ、
    /// 定数表中のインターン文字列が GC で誤回収されないようにする。
    pub fn trace_constants(&self, tracer: &mut Tracer) {
        for v in &self.constants {
            tracer.mark_value(v);
        }
        for p in &self.protos {
            p.trace_constants(tracer);
        }
    }
}
