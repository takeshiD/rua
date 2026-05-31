//! エラー型と制御フロー（本家 `ldo.c` の `setjmp/longjmp` 相当を Rust の `Result` で表現）。
//!
//! 本家 Lua は `lua_error` で `longjmp` し、最寄りの `lua_pcall`/保護フレームまで巻き戻す。
//! rua では保護境界を [`LuaResult`] の `Err` 伝播で表現し、`state::call` の保護呼び出しで捕捉する。
//! パニックは FFI 境界（将来の rua-capi）に漏らさない方針（ARCHITECTURE.md §6）。

use crate::value::Value;

/// Lua 実行時／コンパイル時に発生しうるエラー。
///
/// Lua のエラーオブジェクトは任意の値（多くは文字列）を取りうるため、実行時エラーは
/// [`Value`] を保持する。フロントエンドや内部エラーは Rust 側メッセージで表現する。
#[derive(Debug, Clone)]
pub enum LuaError {
    /// `error(obj)` 等で送出された Lua 値（実行時エラー）。本家のエラーオブジェクトに相当。
    Runtime(Value),
    /// 構文エラー（lexer/parser）。本家の `LUA_ERRSYNTAX`。
    Syntax(String),
    /// メモリ確保失敗。本家の `LUA_ERRMEM`。
    Memory,
    /// エラーハンドラ実行中のエラー。本家の `LUA_ERRERR`。
    ErrorInError,
    /// 内部実装エラー（rua の bug。本来到達しない経路）。
    Internal(String),
    /// コルーチン yield（制御フロー専用、通常のエラーではない）。
    /// `pcall` を透過して `coroutine.resume` まで伝播する。
    Yield(Vec<Value>),
}

impl LuaError {
    /// 文字列メッセージから実行時エラーを作る簡易コンストラクタ。
    ///
    /// 注意: Lua セマンティクス上、本来エラーオブジェクトはインターン済み Lua 文字列
    /// （[`Value::GcRef`]）であるべき。ヒープへアクセスできない箇所での暫定用途に限り、
    /// ここでは Rust 文字列を `Syntax`/`Internal` 系で包む。Lua 文字列値へ昇格する責務は
    /// 呼び出し側（ヒープを持つ `state`）にある。TODO(lua-vm): 実行時メッセージの Lua 文字列化。
    pub fn runtime_msg(msg: impl Into<String>) -> Self {
        LuaError::Internal(msg.into())
    }
}

impl core::fmt::Display for LuaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LuaError::Runtime(v) => write!(f, "runtime error: {v:?}"),
            LuaError::Syntax(s) => write!(f, "syntax error: {s}"),
            LuaError::Memory => write!(f, "not enough memory"),
            LuaError::ErrorInError => write!(f, "error in error handling"),
            LuaError::Internal(s) => write!(f, "internal error: {s}"),
            LuaError::Yield(_) => write!(f, "attempt to yield"),
        }
    }
}

impl std::error::Error for LuaError {}

/// rua-core 全体で用いる結果型。
pub type LuaResult<T> = Result<T, LuaError>;
