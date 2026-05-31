//! ergonomic な Rust 組み込み API（`mlua`/`rlua` 風）。担当: **lua-capi**。
//!
//! [`crate::state::LuaState`] 等の低レベル API を安全・簡潔に包む高レベル層。
//! 文字列/テーブル/関数を Rust の型（[`Value`]/[`Table`]/[`Function`]）として扱い、
//! [`IntoLua`]/[`FromLua`] で Rust 値と相互変換する。
//!
//! ```
//! use rua_core::api::Lua;
//! let mut lua = Lua::new();
//! // 式を評価して結果を Rust 値へ。
//! let n: f64 = lua.load("return 1 + 2").eval().unwrap();
//! assert_eq!(n, 3.0);
//! // グローバルの設定/取得。
//! lua.set_global("answer", 42.0).unwrap();
//! let a: i64 = lua.get_global("answer").unwrap();
//! assert_eq!(a, 42);
//! ```
//!
//! # 設計と安全性
//! - [`Lua`] は [`LuaState`](crate::state::LuaState) を**所有**する（第二マイルストーン時点）。
//! - [`Table`]/[`Function`] は GC ハンドルの薄いラッパで、対象 [`Lua`] が生きている間有効
//!   （GC は明示起動のみ。詳細は [`value`] のモジュールコメント参照）。
//! - すべての実行は [`pcall`](crate::state::call::pcall) 境界で保護し、エラー時はスタックを巻き戻す。

pub mod convert;
pub mod value;

use std::rc::Rc;

pub use convert::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti};
pub use value::{Function, Table, Value};

use crate::error::{LuaError, LuaResult};
use crate::gc::GcHandle;
use crate::state::{LuaState, NativeFn};
use crate::value::Value as CoreValue;
use crate::value::closure::{Closure, LuaClosure, NativeClosure};
use crate::value::table::Table as CoreTable;

/// 高レベル API のエントリポイント。1 つの Lua 実行環境（状態 + 標準ライブラリ）を持つ。
pub struct Lua {
    state: LuaState,
}

impl Default for Lua {
    fn default() -> Self {
        Lua::new()
    }
}

impl Lua {
    /// 標準ライブラリを開いた新しい Lua 環境を作る（本家 `luaL_newstate` + `luaL_openlibs` 相当）。
    pub fn new() -> Self {
        let mut state = LuaState::new();
        crate::stdlib::open_libs(&mut state);
        Lua { state }
    }

    /// 標準ライブラリを開かない素の Lua 環境を作る。
    pub fn new_bare() -> Self {
        Lua {
            state: LuaState::new(),
        }
    }

    /// 内部の [`LuaState`] への参照（低レベル API へのエスケープハッチ）。
    pub fn state(&self) -> &LuaState {
        &self.state
    }

    /// 内部の [`LuaState`] への可変参照（低レベル API へのエスケープハッチ）。
    pub fn state_mut(&mut self) -> &mut LuaState {
        &mut self.state
    }

    /// 文字列メッセージから実行時エラー（Lua 文字列値を保持）を作る。
    pub fn runtime_error(&mut self, msg: impl Into<String>) -> LuaError {
        let v = self.state.new_string(msg.into().as_bytes());
        LuaError::Runtime(v)
    }

    // ---- 値の変換（高レベル ⇔ コア）------------------------------------

    /// 高レベル [`Value`] をコア [`CoreValue`] へ（文字列はインターン）。
    ///
    /// `to_*` は通常 `&self` を取るが、文字列インターンに `&mut self` が必要なため
    /// clippy の `wrong_self_convention` を抑制する。
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_core(&mut self, v: Value) -> CoreValue {
        match v {
            Value::Nil => CoreValue::Nil,
            Value::Boolean(b) => CoreValue::Boolean(b),
            Value::Number(n) => CoreValue::Number(n),
            Value::String(bytes) => self.state.new_string(&bytes),
            Value::Table(t) => CoreValue::GcRef(t.handle()),
            Value::Function(f) => CoreValue::GcRef(f.handle()),
            Value::LightUserData(p) => CoreValue::LightUserData(p),
        }
    }

    /// コア [`CoreValue`] を高レベル [`Value`] へ（文字列内容をコピー）。
    ///
    /// `from_*` は通常 `self` を取らないが、ヒープアクセスに `&self` が必要なため
    /// clippy の `wrong_self_convention` を抑制する。
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn from_core(&self, v: CoreValue) -> Value {
        match v {
            CoreValue::Nil => Value::Nil,
            CoreValue::Boolean(b) => Value::Boolean(b),
            CoreValue::Number(n) => Value::Number(n),
            CoreValue::LightUserData(p) => Value::LightUserData(p),
            CoreValue::GcRef(h) => match h {
                GcHandle::Str(k) => {
                    let bytes = self
                        .state
                        .global
                        .heap
                        .get_str(k)
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_default();
                    Value::String(bytes)
                }
                GcHandle::Table(_) => Value::Table(Table(h)),
                GcHandle::Closure(_) => Value::Function(Function(h)),
                // full userdata は高レベル API v1 では未サポート。
                // 失わないよう lightuserdata 風プレースホルダにフォールバックする。
                // TODO(lua-capi): 高レベル Userdata 型を追加する。
                GcHandle::Userdata(_) => Value::LightUserData(std::ptr::null_mut()),
            },
        }
    }

    // ---- テーブル ------------------------------------------------------

    /// 新しい空テーブルを作る（本家 `lua_newtable`）。
    pub fn create_table(&mut self) -> Table {
        let h = self.state.global.heap.alloc_table(CoreTable::new());
        Table(h)
    }

    /// グローバル環境テーブル `_G`（本家 `LUA_GLOBALSINDEX`）。
    pub fn globals(&self) -> Table {
        Table(self.state.global.globals)
    }

    /// テーブルへ `key = value` を代入する（raw 代入, `__newindex` 非経由）。
    pub fn set<K: IntoLua, V: IntoLua>(&mut self, table: Table, key: K, value: V) -> LuaResult<()> {
        let k = key.into_lua(self)?;
        let v = value.into_lua(self)?;
        let ck = self.to_core(k);
        let cv = self.to_core(v);
        let GcHandle::Table(tk) = table.handle() else {
            return Err(self.runtime_error("not a table"));
        };
        match self.state.global.heap.get_table_mut(tk) {
            Some(t) => t.set(ck, cv).map_err(|_| {
                // nil/NaN キーは本家でエラー。
                LuaError::Runtime(self.state.new_string(b"table index is nil or NaN"))
            }),
            None => Err(self.runtime_error("invalid table")),
        }
    }

    /// テーブルから `key` を取得する（raw 取得, `__index` 非経由）。
    pub fn get<K: IntoLua, R: FromLua>(&mut self, table: Table, key: K) -> LuaResult<R> {
        let k = key.into_lua(self)?;
        let ck = self.to_core(k);
        let GcHandle::Table(tk) = table.handle() else {
            return Err(self.runtime_error("not a table"));
        };
        let cv = self
            .state
            .global
            .heap
            .get_table(tk)
            .map(|t| t.get(&ck))
            .unwrap_or(CoreValue::Nil);
        let v = self.from_core(cv);
        R::from_lua(v, self)
    }

    /// グローバル変数 `name` を設定する。
    pub fn set_global<V: IntoLua>(&mut self, name: &str, value: V) -> LuaResult<()> {
        let g = self.globals();
        self.set(g, name, value)
    }

    /// グローバル変数 `name` を取得する。
    pub fn get_global<R: FromLua>(&mut self, name: &str) -> LuaResult<R> {
        let g = self.globals();
        self.get(g, name)
    }

    // ---- 関数 ----------------------------------------------------------

    /// ネイティブ（Rust）関数を関数値として確保する（本家 `lua_pushcfunction` 相当）。
    ///
    /// `f` は低レベルの [`NativeFn`]（`fn(&mut LuaState) -> LuaResult<i32>`）。引数・戻り値は
    /// VM スタック経由（[`crate::stdlib::aux`] のヘルパが扱いやすくする）。
    ///
    /// TODO(lua-capi): キャプチャ付きクロージャや型付き引数/戻り値の ergonomic な登録は後続拡張。
    pub fn create_function(&mut self, f: NativeFn) -> Function {
        let h = self
            .state
            .global
            .heap
            .alloc_closure(Closure::Native(NativeClosure::new(f)));
        Function(h)
    }

    /// グローバルにネイティブ関数を登録する簡易ヘルパ。
    pub fn register_fn(&mut self, name: &str, f: NativeFn) -> LuaResult<()> {
        let func = self.create_function(f);
        self.set_global(name, func)
    }

    /// 関数を呼び出す（本家 `lua_pcall` 相当・保護付き）。引数列・戻り値列は多値変換に対応。
    pub fn call<A: IntoLuaMulti, R: FromLuaMulti>(
        &mut self,
        func: Function,
        args: A,
    ) -> LuaResult<R> {
        let arg_vals = args.into_lua_multi(self)?;
        let core_args: Vec<CoreValue> = arg_vals.into_iter().map(|v| self.to_core(v)).collect();
        let fval = CoreValue::GcRef(func.handle());
        let results =
            crate::state::call::pcall(&mut self.state, |s| crate::vm::call(s, fval, &core_args))?;
        let high: Vec<Value> = results.into_iter().map(|v| self.from_core(v)).collect();
        R::from_lua_multi(high, self)
    }

    // ---- ロード/評価 ---------------------------------------------------

    /// ソースをチャンクとして読み込む（本家 `luaL_loadstring`/`load` 相当）。
    ///
    /// 返る [`Chunk`] に対し [`Chunk::exec`]/[`Chunk::eval`]/[`Chunk::into_function`] を呼ぶ。
    pub fn load<'lua, S: AsRef<[u8]>>(&'lua mut self, source: S) -> Chunk<'lua> {
        let src = source.as_ref().to_vec();
        // 既定のチャンク名はソース文字列自身（本家 `luaL_loadstring` と同様、
        // `[string "..."]` 形式に整形される）。
        let name = String::from_utf8_lossy(&src).into_owned();
        Chunk {
            lua: self,
            source: src,
            name,
        }
    }

    /// 内部: ソースをコンパイルして Lua 関数値（クロージャ）を確保する。
    fn compile_closure(&mut self, source: &[u8], chunkname: &str) -> LuaResult<Function> {
        let proto = crate::compiler::compile(&mut self.state.global.heap, source, chunkname)?;
        let h = self
            .state
            .global
            .heap
            .alloc_closure(Closure::Lua(LuaClosure::new(Rc::new(proto))));
        Ok(Function(h))
    }
}

/// [`Lua::load`] が返すチャンクビルダ。チャンク名の設定後に実行/評価する。
pub struct Chunk<'lua> {
    lua: &'lua mut Lua,
    source: Vec<u8>,
    name: String,
}

impl<'lua> Chunk<'lua> {
    /// チャンク名を設定する（エラー/トレースバック表示に使われる）。
    ///
    /// 本家 `lua_load` 同様、`@file`（ファイル）・`=name`（表示名そのまま）・その他
    /// （`[string "..."]` 形式）の規約に従う。
    pub fn set_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// コンパイルのみ行い、実行可能な関数値を返す（実行はしない）。
    pub fn into_function(self) -> LuaResult<Function> {
        self.lua.compile_closure(&self.source, &self.name)
    }

    /// チャンクを実行し、戻り値を捨てる（本家 `dofile`/`dostring` の値無視版）。
    pub fn exec(self) -> LuaResult<()> {
        let _: Vec<Value> = self.call(())?;
        Ok(())
    }

    /// チャンクを実行し、最初の戻り値を Rust 値へ変換して返す。
    pub fn eval<R: FromLua>(self) -> LuaResult<R> {
        let Chunk { lua, source, name } = self;
        let func = lua.compile_closure(&source, &name)?;
        let results: Vec<Value> = lua.call(func, ())?;
        let first = results.into_iter().next().unwrap_or(Value::Nil);
        R::from_lua(first, lua)
    }

    /// チャンクを引数付きで実行し、多値の戻り値を返す。
    pub fn call<A: IntoLuaMulti, R: FromLuaMulti>(self, args: A) -> LuaResult<R> {
        let Chunk { lua, source, name } = self;
        let func = lua.compile_closure(&source, &name)?;
        lua.call(func, args)
    }
}
