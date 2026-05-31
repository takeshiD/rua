//! 高レベル Rust API（`rua_core::api::Lua`）の動作テスト。
//!
//! `Lua::new/load/eval/call/set_global/get_global/create_function/register_fn` の
//! 基本動作と、Rust 関数を Lua から呼ぶ往復を検証する。

use rua_core::api::{Lua, Value};
use rua_core::error::LuaResult;
use rua_core::state::LuaState;

// ============================================================================
// 基本操作（load / eval / exec）
// ============================================================================

#[test]
fn eval_arithmetic() {
    let mut lua = Lua::new();
    let n: f64 = lua.load("return 1 + 2").eval().unwrap();
    assert_eq!(n, 3.0);
}

#[test]
fn eval_string_concat() {
    let mut lua = Lua::new();
    let s: String = lua.load("return 'hello' .. ' world'").eval().unwrap();
    assert_eq!(s, "hello world");
}

#[test]
fn eval_nil_returns_nil() {
    let mut lua = Lua::new();
    let v: Value = lua.load("return nil").eval().unwrap();
    assert!(v.is_nil());
}

#[test]
fn exec_no_return() {
    let mut lua = Lua::new();
    lua.load("x = 42").exec().unwrap();
    let n: f64 = lua.get_global("x").unwrap();
    assert_eq!(n, 42.0);
}

// ============================================================================
// グローバル変数の設定・取得
// ============================================================================

#[test]
fn set_and_get_global_number() {
    let mut lua = Lua::new();
    lua.set_global("answer", 42.0f64).unwrap();
    let v: f64 = lua.get_global("answer").unwrap();
    assert_eq!(v, 42.0);
}

#[test]
fn set_global_read_from_lua() {
    let mut lua = Lua::new();
    lua.set_global("base", 10i64).unwrap();
    let n: f64 = lua.load("return base * 3").eval().unwrap();
    assert_eq!(n, 30.0);
}

#[test]
fn get_global_bool() {
    let mut lua = Lua::new();
    lua.set_global("flag", true).unwrap();
    let b: bool = lua.get_global("flag").unwrap();
    assert!(b);
}

#[test]
fn get_global_string() {
    let mut lua = Lua::new();
    lua.set_global("greeting", "hello").unwrap();
    let s: String = lua.get_global("greeting").unwrap();
    assert_eq!(s, "hello");
}

// ============================================================================
// create_function / register_fn: Rust 関数を Lua から呼ぶ
// ============================================================================

/// 2 引数の加算関数（低レベル NativeFn）。
///
/// スタックプロトコル: コールフレームのベース以降に引数が並ぶ。
/// `stdlib::aux::args_vec` でスナップショットを取り、`check_number` で型検査する。
fn native_add(state: &mut LuaState) -> LuaResult<i32> {
    use rua_core::stdlib::aux::{args_vec, check_number, ret};
    let args = args_vec(state);
    let a = check_number(state, &args, 0, "native_add")?;
    let b = check_number(state, &args, 1, "native_add")?;
    ret(state, vec![rua_core::value::Value::Number(a + b)])
}

#[test]
fn create_function_and_call() {
    let mut lua = Lua::new();
    let func = lua.create_function(native_add);
    let result: Vec<Value> = lua.call(func, (3.0f64, 4.0f64)).unwrap();
    assert_eq!(result.len(), 1);
    match result[0] {
        Value::Number(n) => assert_eq!(n, 7.0),
        _ => panic!("expected number"),
    }
}

#[test]
fn register_fn_callable_from_lua() {
    let mut lua = Lua::new();
    lua.register_fn("my_add", native_add).unwrap();

    let n: f64 = lua.load("return my_add(10, 32)").eval().unwrap();
    assert_eq!(n, 42.0);
}

#[test]
fn register_fn_multiple_calls() {
    let mut lua = Lua::new();
    lua.register_fn("my_add", native_add).unwrap();

    // 複数回呼べることを確認する。
    let n1: f64 = lua.load("return my_add(1, 2)").eval().unwrap();
    let n2: f64 = lua.load("return my_add(10, 20)").eval().unwrap();
    assert_eq!(n1, 3.0);
    assert_eq!(n2, 30.0);
}

// ============================================================================
// テーブル操作
// ============================================================================

#[test]
fn create_table_set_get() {
    let mut lua = Lua::new();
    let t = lua.create_table();
    lua.set(t, "key", "value").unwrap();
    let v: String = lua.get(t, "key").unwrap();
    assert_eq!(v, "value");
}

#[test]
fn table_accessible_from_lua() {
    let mut lua = Lua::new();
    let t = lua.create_table();
    lua.set(t, "x", 99.0f64).unwrap();
    lua.set_global("mytable", t).unwrap();

    let n: f64 = lua.load("return mytable.x").eval().unwrap();
    assert_eq!(n, 99.0);
}

// ============================================================================
// エラー処理
// ============================================================================

#[test]
fn syntax_error_returns_err() {
    let mut lua = Lua::new();
    let result = lua.load("this is not ( lua").into_function();
    assert!(result.is_err());
}

#[test]
fn runtime_error_returns_err() {
    let mut lua = Lua::new();
    let result = lua.load("error('boom')").exec();
    assert!(result.is_err());
}

#[test]
fn pcall_boundary_restores_stack() {
    let mut lua = Lua::new();
    // エラー後もスタックが汚染されず次の呼び出しが正常動作することを確認する。
    let _ = lua.load("error('boom')").exec();
    let n: f64 = lua.load("return 1 + 1").eval().unwrap();
    assert_eq!(n, 2.0);
}

// ============================================================================
// 型変換（IntoLua / FromLua）
// ============================================================================

#[test]
fn from_lua_option_nil() {
    let mut lua = Lua::new();
    let v: Option<f64> = lua.load("return nil").eval().unwrap();
    assert!(v.is_none());
}

#[test]
fn from_lua_option_number() {
    let mut lua = Lua::new();
    // 周知の数学定数と近似しない値（clippy::approx_constant 回避）。
    let v: Option<f64> = lua.load("return 1.234").eval().unwrap();
    assert_eq!(v, Some(1.234));
}

// ============================================================================
// Chunk API（set_name, into_function）
// ============================================================================

#[test]
fn chunk_into_function_and_call() {
    let mut lua = Lua::new();
    let func = lua
        .load("return ...")
        .set_name("test_chunk")
        .into_function()
        .unwrap();
    // 引数付き呼び出し（可変長引数）。
    let result: Vec<Value> = lua.call(func, (1.0f64, 2.0f64)).unwrap();
    // チャンクに渡した引数が `...` として返る。
    assert!(!result.is_empty());
}
