//! Rust 型 ⇔ 高レベル [`Value`] の変換トレイト（本家 `mlua` の `IntoLua`/`FromLua` 相当）。
//!
//! - [`IntoLua`]: Rust 値を Lua 値へ。文字列のインターン等で [`Lua`] を要する。
//! - [`FromLua`]: Lua 値から Rust 値へ。失敗時は型不一致エラー。
//! - [`IntoLuaMulti`]/[`FromLuaMulti`]: 関数の多値（引数列/戻り値列）に対応する。

use crate::error::{LuaError, LuaResult};

use super::value::{Function, Table, Value};
use super::Lua;

/// 変換失敗（型不一致）の実行時エラーを作る。
pub(crate) fn conv_error(lua: &mut Lua, expected: &str, got: &str) -> LuaError {
    lua.runtime_error(format!("expected {expected}, got {got}"))
}

/// Rust 値 → 高レベル [`Value`]。
pub trait IntoLua {
    /// Lua 値へ変換する（文字列のインターン等で `lua` を用いることがある）。
    fn into_lua(self, lua: &mut Lua) -> LuaResult<Value>;
}

/// 高レベル [`Value`] → Rust 値。
pub trait FromLua: Sized {
    /// Lua 値から変換する。型が合わなければ `Err`。
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self>;
}

// ---- IntoLua 実装 ---------------------------------------------------------

impl IntoLua for Value {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(self)
    }
}

impl IntoLua for () {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::Nil)
    }
}

impl IntoLua for bool {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::Boolean(self))
    }
}

macro_rules! into_lua_number {
    ($($t:ty),*) => {$(
        impl IntoLua for $t {
            fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
                Ok(Value::Number(self as f64))
            }
        }
    )*};
}
into_lua_number!(f64, f32, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl IntoLua for &str {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::String(self.as_bytes().to_vec()))
    }
}

impl IntoLua for String {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::String(self.into_bytes()))
    }
}

impl IntoLua for &[u8] {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::String(self.to_vec()))
    }
}

impl IntoLua for Vec<u8> {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::String(self))
    }
}

impl IntoLua for Table {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::Table(self))
    }
}

impl IntoLua for Function {
    fn into_lua(self, _lua: &mut Lua) -> LuaResult<Value> {
        Ok(Value::Function(self))
    }
}

impl<T: IntoLua> IntoLua for Option<T> {
    fn into_lua(self, lua: &mut Lua) -> LuaResult<Value> {
        match self {
            Some(v) => v.into_lua(lua),
            None => Ok(Value::Nil),
        }
    }
}

// ---- FromLua 実装 ---------------------------------------------------------

impl FromLua for Value {
    fn from_lua(value: Value, _lua: &mut Lua) -> LuaResult<Self> {
        Ok(value)
    }
}

impl FromLua for () {
    fn from_lua(_value: Value, _lua: &mut Lua) -> LuaResult<Self> {
        Ok(())
    }
}

impl FromLua for bool {
    fn from_lua(value: Value, _lua: &mut Lua) -> LuaResult<Self> {
        // Lua の慣用に合わせ、任意値の真偽として解釈する（false/nil のみ偽）。
        Ok(value.is_truthy())
    }
}

macro_rules! from_lua_number {
    ($($t:ty),*) => {$(
        impl FromLua for $t {
            fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
                match value {
                    Value::Number(n) => Ok(n as $t),
                    // 本家同様、数値に見える文字列は数値へ強制変換。
                    Value::String(ref b) => match crate::value::convert::str_to_number(b) {
                        Some(n) => Ok(n as $t),
                        None => Err(conv_error(lua, "number", value.type_name())),
                    },
                    other => Err(conv_error(lua, "number", other.type_name())),
                }
            }
        }
    )*};
}
from_lua_number!(f64, f32, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl FromLua for String {
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
        match value {
            Value::String(b) => {
                String::from_utf8(b).map_err(|_| lua.runtime_error("string is not valid UTF-8"))
            }
            Value::Number(n) => Ok(crate::value::convert::number_to_string(n)),
            other => Err(conv_error(lua, "string", other.type_name())),
        }
    }
}

impl FromLua for Vec<u8> {
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
        match value {
            Value::String(b) => Ok(b),
            Value::Number(n) => Ok(crate::value::convert::number_to_string(n).into_bytes()),
            other => Err(conv_error(lua, "string", other.type_name())),
        }
    }
}

impl FromLua for Table {
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
        match value {
            Value::Table(t) => Ok(t),
            other => Err(conv_error(lua, "table", other.type_name())),
        }
    }
}

impl FromLua for Function {
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
        match value {
            Value::Function(f) => Ok(f),
            other => Err(conv_error(lua, "function", other.type_name())),
        }
    }
}

impl<T: FromLua> FromLua for Option<T> {
    fn from_lua(value: Value, lua: &mut Lua) -> LuaResult<Self> {
        match value {
            Value::Nil => Ok(None),
            other => Ok(Some(T::from_lua(other, lua)?)),
        }
    }
}

// ---- 多値（Multi）--------------------------------------------------------

/// 関数引数列／戻り値列へ展開できる Rust 値（タプル等）。
pub trait IntoLuaMulti {
    fn into_lua_multi(self, lua: &mut Lua) -> LuaResult<Vec<Value>>;
}

/// 関数戻り値列から復元できる Rust 値（タプル等）。
pub trait FromLuaMulti: Sized {
    fn from_lua_multi(values: Vec<Value>, lua: &mut Lua) -> LuaResult<Self>;
}

impl IntoLuaMulti for () {
    fn into_lua_multi(self, _lua: &mut Lua) -> LuaResult<Vec<Value>> {
        Ok(Vec::new())
    }
}

impl IntoLuaMulti for Vec<Value> {
    fn into_lua_multi(self, _lua: &mut Lua) -> LuaResult<Vec<Value>> {
        Ok(self)
    }
}

impl FromLuaMulti for () {
    fn from_lua_multi(_values: Vec<Value>, _lua: &mut Lua) -> LuaResult<Self> {
        Ok(())
    }
}

impl FromLuaMulti for Vec<Value> {
    fn from_lua_multi(values: Vec<Value>, _lua: &mut Lua) -> LuaResult<Self> {
        Ok(values)
    }
}

/// 単一の `IntoLua` 値は 1 要素の多値として扱う。
impl<T: IntoLua> IntoLuaMulti for (T,) {
    fn into_lua_multi(self, lua: &mut Lua) -> LuaResult<Vec<Value>> {
        Ok(vec![self.0.into_lua(lua)?])
    }
}

macro_rules! tuple_multi {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($name: IntoLua),+> IntoLuaMulti for ($($name,)+ ) {
            fn into_lua_multi(self, lua: &mut Lua) -> LuaResult<Vec<Value>> {
                let ($($name,)+) = self;
                Ok(vec![$($name.into_lua(lua)?),+])
            }
        }

        #[allow(non_snake_case)]
        impl<$($name: FromLua),+> FromLuaMulti for ($($name,)+ ) {
            fn from_lua_multi(values: Vec<Value>, lua: &mut Lua) -> LuaResult<Self> {
                let mut it = values.into_iter();
                Ok(($(
                    $name::from_lua(it.next().unwrap_or(Value::Nil), lua)?,
                )+))
            }
        }
    };
}
// 2..=5 要素のタプルに対応（(T,) は上で個別実装）。
tuple_multi!(A, B);
tuple_multi!(A, B, C);
tuple_multi!(A, B, C, D);
tuple_multi!(A, B, C, D, E);

/// 単一値の戻り値復元（`FromLuaMulti` の単数版）。先頭値を変換し、無ければ `nil`。
impl<T: FromLua> FromLuaMulti for (T,) {
    fn from_lua_multi(values: Vec<Value>, lua: &mut Lua) -> LuaResult<Self> {
        let mut it = values.into_iter();
        Ok((T::from_lua(it.next().unwrap_or(Value::Nil), lua)?,))
    }
}
