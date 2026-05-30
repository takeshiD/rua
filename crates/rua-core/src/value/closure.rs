//! 関数（本家 `lfunc.c` / `Closure`・`Proto` 相当）。
//!
//! Lua の `function` 型は 2 種: **Lua クロージャ**（バイトコード [`Proto`] + upvalue）と
//! **ネイティブ関数**（本家 C function、rua では Rust の [`NativeFn`]）。
//!
//! # upvalue のオープン/クローズ（本家 `UpVal`）
//! upvalue は、捕捉元のローカル変数がまだスタック上に生きている間は **open**
//! （[`UpvalueState::Open`]、スタックスロットの絶対インデックスを指す）であり、
//! その変数がスコープを抜けると **closed**（[`UpvalueState::Closed`]、値を自身へコピー）になる。
//! 同一スコープの同一ローカルを捕捉する複数クロージャは同じ [`Upvalue`]（`Rc<RefCell<..>>`）を共有し、
//! 一方の書き換えが他方へ反映される（本家のセマンティクスに一致）。

use std::cell::RefCell;
use std::rc::Rc;

use crate::gc::{Trace, Tracer};
use crate::state::NativeFn;
use crate::value::Value;
use crate::vm::proto::Proto;

/// upvalue の状態（open/closed）。
#[derive(Debug)]
pub enum UpvalueState {
    /// 捕捉元ローカルがまだスタック上にある。値はスタックの**絶対**インデックスで参照する。
    Open(usize),
    /// スコープを抜けて閉じられた upvalue。値を自身が保持する。
    Closed(Value),
}

/// 共有される upvalue セル。複数クロージャ間で同一ローカルの捕捉を共有する。
pub type Upvalue = Rc<RefCell<UpvalueState>>;

/// 関数オブジェクト。
#[derive(Debug)]
pub enum Closure {
    /// Lua バイトコードのクロージャ。
    Lua(LuaClosure),
    /// ネイティブ（Rust）関数。本家の C closure 相当。
    Native(NativeClosure),
}

/// Lua バイトコードのクロージャ。
#[derive(Debug)]
pub struct LuaClosure {
    /// 実行するプロトタイプ（命令列・定数表・子 proto・デバッグ情報）。
    proto: Rc<Proto>,
    /// 捕捉した upvalue 群（`proto.num_upvalues` 個）。
    upvalues: Vec<Upvalue>,
}

impl LuaClosure {
    /// プロトタイプからクロージャを作る（upvalue は呼び出し側が [`Self::push_upvalue`] で束ねる）。
    pub fn new(proto: Rc<Proto>) -> Self {
        let cap = proto.num_upvalues as usize;
        LuaClosure {
            proto,
            upvalues: Vec::with_capacity(cap),
        }
    }

    /// プロトタイプへの参照。
    pub fn proto(&self) -> &Rc<Proto> {
        &self.proto
    }

    /// 束ねた upvalue 群。
    pub fn upvalues(&self) -> &[Upvalue] {
        &self.upvalues
    }

    /// `i` 番目の upvalue を取得（`GETUPVAL`/`SETUPVAL` 用）。
    pub fn upvalue(&self, i: usize) -> Option<&Upvalue> {
        self.upvalues.get(i)
    }

    /// upvalue を 1 つ束ねる（`CLOSURE` 直後の捕捉疑似命令処理から呼ぶ）。
    pub fn push_upvalue(&mut self, uv: Upvalue) {
        self.upvalues.push(uv);
    }
}

/// ネイティブ関数クロージャ（Rust 実装の組み込み関数）。
pub struct NativeClosure {
    /// 呼び出される Rust 関数。
    func: NativeFn,
    /// C closure と同様の upvalue（本家 `lua_pushcclosure` の upvalue 群）。
    upvalues: Vec<Value>,
}

impl NativeClosure {
    pub fn new(func: NativeFn) -> Self {
        NativeClosure {
            func,
            upvalues: Vec::new(),
        }
    }

    pub fn with_upvalues(func: NativeFn, upvalues: Vec<Value>) -> Self {
        NativeClosure { func, upvalues }
    }

    pub fn func(&self) -> NativeFn {
        self.func
    }

    /// upvalue スライス全体を返す。
    pub fn upvalues(&self) -> &[Value] {
        &self.upvalues
    }

    /// `i` 番目の upvalue（0-origin）を返す。範囲外は `None`。
    ///
    /// 本家 `lua_upvalueindex` の内部実装補助として [`crate::state::LuaState::current_upvalue`] から呼ぶ。
    pub fn upvalue(&self, i: usize) -> Option<&Value> {
        self.upvalues.get(i)
    }

    /// upvalue の個数を返す。
    pub fn upvalue_count(&self) -> usize {
        self.upvalues.len()
    }

    /// upvalue を末尾に追加する（`alloc_closure` 後に後付けする場合に使用）。
    ///
    /// `lua_pushcclosure` は `NativeClosure::with_upvalues` でまとめて渡すのが主流だが、
    /// capi が個別に積む場合はこちらを使える。
    pub fn push_upvalue(&mut self, v: Value) {
        self.upvalues.push(v);
    }

    /// `i` 番目の upvalue を書き換える（0-origin）。範囲外は何もしない。
    ///
    /// 本家 `lua_setupvalue` の内部実装補助。
    pub fn set_upvalue(&mut self, i: usize, v: Value) {
        if let Some(slot) = self.upvalues.get_mut(i) {
            *slot = v;
        }
    }
}

// 関数ポインタは Debug を導出できないため手動実装。
impl std::fmt::Debug for NativeClosure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeClosure")
            .field("func", &(self.func as *const ()))
            .field("upvalues", &self.upvalues)
            .finish()
    }
}

impl Trace for Closure {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Closure::Lua(c) => {
                // proto の定数表（インターン文字列を含む）を mark。
                c.proto.trace_constants(tracer);
                // closed upvalue が保持する値を mark（open は捕捉元スタックが別途ルート）。
                for uv in &c.upvalues {
                    if let UpvalueState::Closed(v) = &*uv.borrow() {
                        tracer.mark_value(v);
                    }
                }
            }
            Closure::Native(c) => {
                for v in &c.upvalues {
                    tracer.mark_value(v);
                }
            }
        }
    }
}
