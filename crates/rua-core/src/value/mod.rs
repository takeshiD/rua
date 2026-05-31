//! 値モデル（本家 `lobject.c` の `TValue` 相当, ARCHITECTURE.md §4）。
//!
//! Lua 5.1 の値型は `nil / boolean / number(double) / string / table / function /
//! userdata / lightuserdata / thread`。このうち string/table/function/userdata/thread は
//! GC 管理オブジェクトであり、[`Value::GcRef`] が型別アリーナ上のハンドルで参照する。
//!
//! 型の **骨格** は lua-runtime が用意し、各 GC オブジェクトの本実装（テーブルのハッシュ部、
//! 文字列メタ情報、クロージャの命令列など）は lua-vm / lua-frontend が埋める。

pub mod closure;
pub mod convert;
pub mod string;
pub mod table;
pub mod thread;
pub mod userdata;

use std::os::raw::c_void;

use crate::gc::GcHandle;

/// Lua の値（本家 `TValue`）。
///
/// `f64`・`bool`・ポインタ・[`GcHandle`] はいずれも `Copy` なので `Value` も `Copy`。
/// これにより VM スタック上を安価に移動できる（本家の値セマンティクスに一致）。
///
/// # 等価性
/// `PartialEq` は Lua の生（raw）等価性に対応させる予定だが、GC 文字列の内容比較は
/// インターン（同値 → 同一ハンドル）で吸収されるため、ハンドル一致で内容一致を判定できる。
/// メタメソッド `__eq` を考慮した等価性は lua-vm が別途実装する。
#[derive(Debug, Clone, Copy, Default)]
pub enum Value {
    /// `nil`
    #[default]
    Nil,
    /// `boolean`
    Boolean(bool),
    /// `number` — Lua 5.1 は整数型を持たず全数値が double。
    Number(f64),
    /// `lightuserdata` — GC 管理外の生ポインタ。値として比較・運搬される。
    LightUserData(*mut c_void),
    /// GC 管理オブジェクト（string/table/function/userdata/thread）への参照。
    GcRef(GcHandle),
}

/// Lua の基本型（本家 `LUA_T*` 定数, `lua_type` の戻り値に相当）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaType {
    Nil,
    Boolean,
    Number,
    String,
    Table,
    Function,
    Userdata,
    Thread,
    LightUserData,
}

impl LuaType {
    /// 本家 `lua_typename` 相当の型名（エラーメッセージで使用）。
    pub fn name(self) -> &'static str {
        match self {
            LuaType::Nil => "nil",
            LuaType::Boolean => "boolean",
            LuaType::Number => "number",
            LuaType::String => "string",
            LuaType::Table => "table",
            LuaType::Function => "function",
            LuaType::Userdata => "userdata",
            LuaType::Thread => "thread",
            // 本家では lightuserdata も "userdata" と表示される。
            LuaType::LightUserData => "userdata",
        }
    }
}

impl Value {
    /// 本家 `lua_type` 相当。GcRef はハンドルの判別子だけで型が分かる（本体デリファレンス不要）。
    pub fn type_of(&self) -> LuaType {
        match self {
            Value::Nil => LuaType::Nil,
            Value::Boolean(_) => LuaType::Boolean,
            Value::Number(_) => LuaType::Number,
            Value::LightUserData(_) => LuaType::LightUserData,
            Value::GcRef(h) => match h {
                GcHandle::Str(_) => LuaType::String,
                GcHandle::Table(_) => LuaType::Table,
                GcHandle::Closure(_) => LuaType::Function,
                GcHandle::Userdata(_) => LuaType::Userdata,
                GcHandle::Thread(_) => LuaType::Thread,
            },
        }
    }

    /// Lua の真偽（false と nil のみ偽, それ以外は真）。本家 `lua_toboolean` のコア規則。
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Boolean(false))
    }

    /// GC 参照ならそのハンドルを返す（GC ルート収集や型別アクセスの入口）。
    pub fn as_gc(&self) -> Option<GcHandle> {
        match self {
            Value::GcRef(h) => Some(*h),
            _ => None,
        }
    }
}

impl PartialEq for Value {
    /// Lua の raw 等価（`rawequal`）相当。
    /// number は値比較、GcRef はハンドル比較（文字列はインターンにより内容一致 ⇔ ハンドル一致）。
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::LightUserData(a), Value::LightUserData(b)) => std::ptr::eq(*a, *b),
            (Value::GcRef(a), Value::GcRef(b)) => a == b,
            _ => false,
        }
    }
}
