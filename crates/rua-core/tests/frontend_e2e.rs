//! フロントエンド（lexer→parser→codegen）と VM のエンドツーエンド結合テスト。担当: lua-frontend。
//!
//! Lua ソースを [`rua_core::compiler::compile`] でコンパイルし、生成した `Proto` を
//! VM（lua-vm）で実行して、戻り値が期待どおりかを確認する。これは codegen が
//! 実行可能なバイトコードを生成していることの実地検証（byte-exact 検証は lua-conformance）。

use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::state::LuaState;
use rua_core::value::Value;
use rua_core::vm::run;

/// ソースをコンパイル→実行し、戻り値列を得る。
fn run_src(state: &mut LuaState, src: &str) -> Vec<Value> {
    let proto = compile(&mut state.global.heap, src.as_bytes(), "=test").expect("compile");
    run(state, Rc::new(proto), &[]).expect("run")
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        other => panic!("expected number, got {other:?}"),
    }
}

#[test]
fn arithmetic_precedence() {
    let mut state = LuaState::new();
    let r = run_src(&mut state, "return 1 + 2 * 3 - 4 / 2");
    assert_eq!(num(&r[0]), 5.0);
}

#[test]
fn locals_and_assignment() {
    let mut state = LuaState::new();
    let r = run_src(&mut state, "local a = 10 local b = 20 a = a + b return a");
    assert_eq!(num(&r[0]), 30.0);
}

#[test]
fn multiple_return_values() {
    let mut state = LuaState::new();
    let r = run_src(&mut state, "return 1, 2, 3");
    assert_eq!(r.len(), 3);
    assert_eq!(num(&r[0]), 1.0);
    assert_eq!(num(&r[2]), 3.0);
}

#[test]
fn if_else_branch() {
    let mut state = LuaState::new();
    assert_eq!(num(&run_src(&mut state, "if 1 < 2 then return 10 else return 20 end")[0]), 10.0);
    assert_eq!(num(&run_src(&mut state, "if 1 > 2 then return 10 else return 20 end")[0]), 20.0);
}

#[test]
fn numeric_for_sum() {
    let mut state = LuaState::new();
    let r = run_src(&mut state, "local s = 0 for i = 1, 10 do s = s + i end return s");
    assert_eq!(num(&r[0]), 55.0);
}

#[test]
fn while_loop_countdown() {
    let mut state = LuaState::new();
    let r = run_src(
        &mut state,
        "local n = 5 local acc = 1 while n > 0 do acc = acc * n n = n - 1 end return acc",
    );
    assert_eq!(num(&r[0]), 120.0); // 5!
}

#[test]
fn repeat_until() {
    let mut state = LuaState::new();
    let r = run_src(
        &mut state,
        "local i = 0 repeat i = i + 1 until i >= 3 return i",
    );
    assert_eq!(num(&r[0]), 3.0);
}

#[test]
fn function_call_and_recursion() {
    let mut state = LuaState::new();
    let r = run_src(
        &mut state,
        "local function fib(n) if n < 2 then return n end return fib(n-1) + fib(n-2) end return fib(10)",
    );
    assert_eq!(num(&r[0]), 55.0);
}

#[test]
fn closure_upvalue_counter() {
    let mut state = LuaState::new();
    let r = run_src(
        &mut state,
        "local function mk() local c = 0 return function() c = c + 1 return c end end \
         local f = mk() f() f() return f()",
    );
    assert_eq!(num(&r[0]), 3.0);
}

#[test]
fn break_in_numeric_for() {
    let mut state = LuaState::new();
    // 05_control_flow.lua の break 部分相当: k*k>50 で初めて成立するのは k=8。
    let r = run_src(
        &mut state,
        "local found for k = 1, 100 do if k * k > 50 then found = k break end end return found",
    );
    assert_eq!(num(&r[0]), 8.0);
}

#[test]
fn nested_for_break_inner_only() {
    let mut state = LuaState::new();
    // 内側ループは b==2 で break。各 a で b=1 の 1 回だけ加算 → 3。
    let r = run_src(
        &mut state,
        "local c = 0 for a = 1, 3 do for b = 1, 3 do if b == 2 then break end c = c + 1 end end return c",
    );
    assert_eq!(num(&r[0]), 3.0);
}

#[test]
fn and_or_logic() {
    let mut state = LuaState::new();
    assert_eq!(num(&run_src(&mut state, "return 1 and 2")[0]), 2.0);
    assert_eq!(num(&run_src(&mut state, "return false or 5")[0]), 5.0);
    assert_eq!(num(&run_src(&mut state, "return (nil and 1) or 7")[0]), 7.0);
}

#[test]
fn table_array_and_length() {
    let mut state = LuaState::new();
    let r = run_src(&mut state, "local t = {10, 20, 30} return t[1] + t[2] + t[3], #t");
    assert_eq!(num(&r[0]), 60.0);
    assert_eq!(num(&r[1]), 3.0);
}

#[test]
fn varargs_passthrough() {
    let mut state = LuaState::new();
    // ipairs はまだ stdlib 未実装のため、テーブル長と数値 for で集計する。
    let r = run_src(
        &mut state,
        "local function sum(...) local t = {...} local s = 0 \
         for i = 1, #t do s = s + t[i] end return s end \
         return sum(1, 2, 3, 4)",
    );
    assert_eq!(num(&r[0]), 10.0);
}
