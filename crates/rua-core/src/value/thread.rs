//! Lua コルーチン（スレッド）型（本家 `lua_State` のコルーチン表現に相当）。
//!
//! コルーチンは `coroutine.create` で生成され GC ヒープ（[`crate::gc::Heap`]）に格納される。
//! 実行状態（スタック・コールフレーム）をスライスとして保存し、`coroutine.resume` で
//! メインスレッドの `LuaState` へ積み戻して再開する（スタック・スライシング方式）。

use crate::state::CallInfo;
use crate::value::Value;

/// コルーチンの実行状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    /// 生成直後、または yield で中断中。
    Suspended,
    /// 現在実行中（resume されている）。
    Running,
    /// 正常終了または未捕捉エラーで終了済み。
    Dead,
    /// 別のコルーチンを resume している（中断ではないが実行もしていない）。
    Normal,
}

/// GC 管理のコルーチンオブジェクト（本家 `lua_State` のコルーチン部分に相当）。
#[derive(Debug)]
pub struct LuaThread {
    /// 現在の実行状態。
    pub status: ThreadStatus,
    /// 初回 resume で呼ぶ関数（`coroutine.create(f)` の `f`）。
    /// 初回 resume 後は `None` に。
    pub body: Option<Value>,
    /// 中断時のスタック保存領域（`state.stack` のコルーチン部分）。
    pub saved_stack: Vec<Value>,
    /// 中断時のコールフレーム保存領域（`state.call_info` のコルーチン部分）。
    pub saved_call_info: Vec<CallInfo>,
}

impl LuaThread {
    /// 関数 `body` を持つ新規サスペンド状態のコルーチンを作る。
    pub fn new(body: Value) -> Self {
        LuaThread {
            status: ThreadStatus::Suspended,
            body: Some(body),
            saved_stack: Vec::new(),
            saved_call_info: Vec::new(),
        }
    }
}
