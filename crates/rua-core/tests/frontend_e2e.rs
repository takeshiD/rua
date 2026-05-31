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
    assert_eq!(
        num(&run_src(&mut state, "if 1 < 2 then return 10 else return 20 end")[0]),
        10.0
    );
    assert_eq!(
        num(&run_src(&mut state, "if 1 > 2 then return 10 else return 20 end")[0]),
        20.0
    );
}

#[test]
fn numeric_for_sum() {
    let mut state = LuaState::new();
    let r = run_src(
        &mut state,
        "local s = 0 for i = 1, 10 do s = s + i end return s",
    );
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
    let r = run_src(
        &mut state,
        "local t = {10, 20, 30} return t[1] + t[2] + t[3], #t",
    );
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

// ============================================================================
// 回帰テスト: RK オーバーフロー (big.lua クラッシュ #issue)
// ハッシュフィールドが 256 個以上のテーブルコンストラクタで、定数インデックスが
// MAXINDEXRK(=255) を超えた場合に codegen がパニックしないこと確認。
// 根本原因: Field::Named のキーが exp2rk を経由せず rk_as_k を直接呼んでいたため
// MAXARG_C(=511) の debug_assert に B/C フィールドの BITRK|idx が引っかかっていた。
// 修正: キー/値ともに exp2rk を経由させ、インデックス > MAXINDEXRK なら
// LOADK+レジスタへ spill させる（本家 luaK_exp2RK の挙動と同様）。
// ============================================================================

/// 257 個のハッシュフィールドを持つコンストラクタがパニックせず正しく値を格納する。
/// 本家 big.lua クラッシュの最小再現ケース。
#[test]
fn hash_table_constructor_over_256_fields() {
    let mut state = LuaState::new();
    // 257 個のフィールドを持つテーブルを構築し、先頭・末尾・中間の値を確認する。
    // フィールド名 a1..a257 と値 1..257 がすべて別の定数なので、
    // 定数テーブルは 514 エントリ以上になり MAXINDEXRK(255) をはるかに超える。
    let src = {
        let fields: String = (1u32..=257)
            .map(|i| format!("a{}={}", i, i))
            .collect::<Vec<_>>()
            .join(",");
        format!("local t = {{{}}} return t.a1, t.a256, t.a257", fields)
    };
    let r = run_src(&mut state, &src);
    assert_eq!(num(&r[0]), 1.0);
    assert_eq!(num(&r[1]), 256.0);
    assert_eq!(num(&r[2]), 257.0);
}

/// 境界値: ちょうど 256 個のハッシュフィールド（最後の定数インデックスが
/// MAXINDEXRK = 255 に到達するケース）。
#[test]
fn hash_table_constructor_exactly_256_fields() {
    let mut state = LuaState::new();
    let src = {
        let fields: String = (1u32..=256)
            .map(|i| format!("a{}={}", i, i))
            .collect::<Vec<_>>()
            .join(",");
        format!("local t = {{{}}} return t.a1, t.a256", fields)
    };
    let r = run_src(&mut state, &src);
    assert_eq!(num(&r[0]), 1.0);
    assert_eq!(num(&r[1]), 256.0);
}

/// キーと値が両方とも異なる文字列定数の場合、定数テーブルは 2n エントリになり
/// n > 128 で MAXINDEXRK を超える。spill が正しく機能することを確認する。
#[test]
fn hash_table_constructor_string_key_string_val_overflow() {
    let mut state = LuaState::new();
    // 143 フィールド: キー143個 + 値143個 = 286 定数 > MAXINDEXRK
    let src = {
        let fields: String = (1u32..=143)
            .map(|i| format!("k{}=\"v{}\"", i, i))
            .collect::<Vec<_>>()
            .join(",");
        format!("local t = {{{}}} return t.k1, t.k128, t.k143", fields)
    };
    let r = run_src(&mut state, &src);
    // 戻り値は文字列なので GcRef になる。nil でないことだけ確認する。
    assert!(!matches!(r[0], Value::Nil));
    assert!(!matches!(r[1], Value::Nil));
    assert!(!matches!(r[2], Value::Nil));
}

/// 大量のハッシュフィールド（500 個）でもパニックしないことを確認する。
#[test]
fn hash_table_constructor_500_fields_no_panic() {
    let mut state = LuaState::new();
    let src = {
        let fields: String = (1u32..=500)
            .map(|i| format!("a{}={}", i, i))
            .collect::<Vec<_>>()
            .join(",");
        format!("local t = {{{}}} return t.a500", fields)
    };
    let r = run_src(&mut state, &src);
    assert_eq!(num(&r[0]), 500.0);
}
