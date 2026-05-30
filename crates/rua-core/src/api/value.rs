//! 高レベル Rust API の値型（`mlua`/`rlua` 風）。
//!
//! コアの [`crate::value::Value`] は `Copy` な GC ハンドル参照であり、文字列内容を取り出すには
//! [`crate::gc::Heap`] が要る。高レベル API ではユーザが扱いやすいよう、文字列は**所有バイト列**
//! として持ち、テーブル/関数は GC ハンドルの薄いラッパ（[`Table`]/[`Function`]）で表す。
//!
//! # GC 安全性（重要）
//! 第二マイルストーン時点では GC は **明示起動のみ**（[`crate::state::LuaState::collect_garbage`]）で、
//! 高レベル API 経路では自動回収を行わない。よって [`Table`]/[`Function`] が保持する
//! [`GcHandle`] は対象 [`Lua`](super::Lua) が生きている限り有効である。
//! 自動 GC（インクリメンタル化）を導入する際は、これらの参照をレジストリへアンカーして
//! 寿命を守る必要がある（TODO: 性能フェーズ, ARCHITECTURE.md §5）。

use std::os::raw::c_void;

use crate::gc::GcHandle;

/// 高レベル API のテーブル参照（GC 上のテーブルへの薄いハンドル）。
///
/// 不変条件: 内部 [`GcHandle`] は必ず [`GcHandle::Table`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table(pub(crate) GcHandle);

impl Table {
    /// 内部 GC ハンドル。
    pub fn handle(self) -> GcHandle {
        self.0
    }
}

/// 高レベル API の関数参照（GC 上のクロージャ／ネイティブ関数への薄いハンドル）。
///
/// 不変条件: 内部 [`GcHandle`] は必ず [`GcHandle::Closure`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Function(pub(crate) GcHandle);

impl Function {
    /// 内部 GC ハンドル。
    pub fn handle(self) -> GcHandle {
        self.0
    }
}

/// 高レベル API の Lua 値。
///
/// 文字列は所有バイト列（Lua 文字列は任意のバイト列を取りうる）。`number` は Lua 5.1 仕様どおり
/// すべて `f64`。テーブル/関数は GC ハンドルラッパで参照する。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `nil`
    Nil,
    /// `boolean`
    Boolean(bool),
    /// `number`（Lua 5.1 は全数値が double）
    Number(f64),
    /// `string`（バイト列）
    String(Vec<u8>),
    /// `table`
    Table(Table),
    /// `function`
    Function(Function),
    /// `lightuserdata`
    LightUserData(*mut c_void),
}

impl Value {
    /// `nil` か。
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Lua の真偽（`false`/`nil` のみ偽）。
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Boolean(false))
    }

    /// 本家 `lua_typename` 相当の型名。
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::LightUserData(_) => "userdata",
        }
    }
}
