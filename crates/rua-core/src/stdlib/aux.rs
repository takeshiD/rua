//! 標準ライブラリ共通の補助関数（本家 `lauxlib.c` の一部に相当）。担当: **lua-stdlib**。
//!
//! ネイティブ関数（[`NativeFn`](crate::state::NativeFn)）の呼び出し規約は
//! `vm::interp::call_native` が定める:
//! - 引数はスタックの `call_info.last().base` 以降に積まれている。
//! - 戻り値は**スタック最上位に積んだ個数**を `i32` で返す。
//! - エラーは `Err(LuaError)` で送出し、保護境界（`state::call::pcall`）まで巻き戻す。
//!
//! 本モジュールはこの規約を扱いやすくするヘルパ（引数取得・戻り値設定・型検査・
//! エラー生成・メタフィールド取得・`tostring`/比較）をまとめる。

use slotmap::Key;

use crate::error::{LuaError, LuaResult};
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::state::NativeFn;
use crate::value::closure::{Closure, NativeClosure};
use crate::value::convert::{number_to_string, str_to_number};
use crate::value::{LuaType, Value};

// ============================================================================
// 引数アクセス / 戻り値設定
// ============================================================================

/// 現在のネイティブフレームの引数ベース（スタックインデックス）。
pub fn arg_base(state: &LuaState) -> usize {
    state.call_info.last().map(|ci| ci.base).unwrap_or(0)
}

/// 現在のネイティブ関数に渡された引数列を取り出す（コピー）。
pub fn args_vec(state: &LuaState) -> Vec<Value> {
    let base = arg_base(state);
    state.stack[base..].to_vec()
}

/// 戻り値列をスタックへ積んで個数を返す（ネイティブ関数の終端で使う）。
///
/// 現フレームの引数領域を破棄してから結果を積む（最上位 = 戻り値となる）。
pub fn ret(state: &mut LuaState, results: Vec<Value>) -> LuaResult<i32> {
    let base = arg_base(state);
    state.stack.truncate(base);
    let n = results.len();
    state.stack.extend(results);
    Ok(n as i32)
}

/// 戻り値 0 個。
pub fn ret0(state: &mut LuaState) -> LuaResult<i32> {
    let base = arg_base(state);
    state.stack.truncate(base);
    Ok(0)
}

/// `args` の `i` 番目（0 始まり）。範囲外は `nil`。
pub fn opt_value(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Nil)
}

// ============================================================================
// エラー生成
// ============================================================================

/// 任意メッセージの実行時エラー（Lua 文字列値を保持）を作る。
pub fn rt_error(state: &mut LuaState, msg: impl Into<String>) -> LuaError {
    let v = state.new_string(msg.into().as_bytes());
    LuaError::Runtime(v)
}

/// `bad argument #n to 'fname' (extra)` 形式の引数エラー（本家 `luaL_argerror`）。
pub fn arg_error(state: &mut LuaState, n: usize, fname: &str, extra: &str) -> LuaError {
    rt_error(state, format!("bad argument #{n} to '{fname}' ({extra})"))
}

/// 型不一致の引数エラー（`TYPE expected, got TYPE`）。
fn type_arg_error(state: &mut LuaState, n: usize, fname: &str, expected: &str, got: Value) -> LuaError {
    let got_name = if matches!(got, Value::Nil) {
        "no value"
    } else {
        got.type_of().name()
    };
    arg_error(state, n, fname, &format!("{expected} expected, got {got_name}"))
}

// ============================================================================
// 型検査 / 変換（本家 luaL_check* / luaL_opt*）
// ============================================================================

/// 文字列バイト列を取得する（数値は本家同様に文字列へ強制変換）。
pub fn check_str_bytes(
    state: &mut LuaState,
    args: &[Value],
    i: usize,
    fname: &str,
) -> LuaResult<Vec<u8>> {
    match opt_value(args, i) {
        Value::GcRef(GcHandle::Str(k)) => Ok(state.global.heap.get_str(k).unwrap().as_bytes().to_vec()),
        Value::Number(n) => Ok(number_to_string(n).into_bytes()),
        other => Err(type_arg_error(state, i + 1, fname, "string", other)),
    }
}

/// 数値を取得する（数値そのもの、または数値に見える文字列を変換）。
pub fn check_number(state: &mut LuaState, args: &[Value], i: usize, fname: &str) -> LuaResult<f64> {
    match opt_value(args, i) {
        Value::Number(n) => Ok(n),
        Value::GcRef(GcHandle::Str(k)) => {
            let bytes = state.global.heap.get_str(k).unwrap().as_bytes().to_vec();
            match str_to_number(&bytes) {
                Some(n) => Ok(n),
                None => Err(type_arg_error(state, i + 1, fname, "number", opt_value(args, i))),
            }
        }
        other => Err(type_arg_error(state, i + 1, fname, "number", other)),
    }
}

/// 省略可能な数値（無ければ `default`）。
pub fn opt_number(
    state: &mut LuaState,
    args: &[Value],
    i: usize,
    fname: &str,
    default: f64,
) -> LuaResult<f64> {
    if matches!(opt_value(args, i), Value::Nil) {
        Ok(default)
    } else {
        check_number(state, args, i, fname)
    }
}

/// 整数を取得する（本家 `luaL_checkinteger`: 数値を 0 方向へ切り捨て）。
pub fn check_int(state: &mut LuaState, args: &[Value], i: usize, fname: &str) -> LuaResult<i64> {
    Ok(check_number(state, args, i, fname)?.trunc() as i64)
}

/// 省略可能な整数（無ければ `default`）。
pub fn opt_int(
    state: &mut LuaState,
    args: &[Value],
    i: usize,
    fname: &str,
    default: i64,
) -> LuaResult<i64> {
    if matches!(opt_value(args, i), Value::Nil) {
        Ok(default)
    } else {
        check_int(state, args, i, fname)
    }
}

/// テーブルキーを取得する（テーブルでなければ引数エラー）。
pub fn check_table(state: &mut LuaState, args: &[Value], i: usize, fname: &str) -> LuaResult<TableKey> {
    match opt_value(args, i) {
        Value::GcRef(GcHandle::Table(k)) => Ok(k),
        other => Err(type_arg_error(state, i + 1, fname, "table", other)),
    }
}

/// 関数値を取得する（関数でなければ引数エラー）。
pub fn check_function(state: &mut LuaState, args: &[Value], i: usize, fname: &str) -> LuaResult<Value> {
    match opt_value(args, i) {
        v @ Value::GcRef(GcHandle::Closure(_)) => Ok(v),
        other => Err(type_arg_error(state, i + 1, fname, "function", other)),
    }
}

// ============================================================================
// 文字列バイト列アクセス
// ============================================================================

/// 文字列値ならバイト列を返す。
pub fn str_bytes(state: &LuaState, v: Value) -> Option<Vec<u8>> {
    match v {
        Value::GcRef(GcHandle::Str(k)) => state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec()),
        _ => None,
    }
}

// ============================================================================
// ネイティブ関数の登録
// ============================================================================

/// `NativeFn` を関数値（クロージャ）として確保する。
pub fn make_native(state: &mut LuaState, f: NativeFn) -> Value {
    let h = state.global.heap.alloc_closure(Closure::Native(NativeClosure::new(f)));
    Value::GcRef(h)
}

/// グローバル/ライブラリテーブル `tk` に `name = f`（ネイティブ関数）を登録する。
pub fn register(state: &mut LuaState, tk: TableKey, name: &str, f: NativeFn) {
    let fval = make_native(state, f);
    let key = state.new_string(name.as_bytes());
    if let Some(t) = state.global.heap.get_table_mut(tk) {
        let _ = t.set(key, fval);
    }
}

/// テーブル `tk` に `name = v` を登録する。
pub fn set_field(state: &mut LuaState, tk: TableKey, name: &str, v: Value) {
    let key = state.new_string(name.as_bytes());
    if let Some(t) = state.global.heap.get_table_mut(tk) {
        let _ = t.set(key, v);
    }
}

// ============================================================================
// メタテーブル / メタフィールド
// ============================================================================

/// 値のメタテーブル（テーブルキー）を返す。
pub fn metatable_handle(state: &LuaState, v: Value) -> Option<TableKey> {
    let mt = match v {
        Value::GcRef(GcHandle::Table(k)) => state.global.heap.get_table(k).and_then(|t| t.metatable()),
        Value::GcRef(GcHandle::Userdata(k)) => {
            state.global.heap.get_userdata(k).and_then(|u| u.metatable())
        }
        // 文字列は型共有メタテーブルを参照（VM の `metatable_of` と整合）。
        Value::GcRef(GcHandle::Str(_)) => state.global.string_metatable,
        _ => None,
    };
    match mt {
        Some(GcHandle::Table(k)) => Some(k),
        _ => None,
    }
}

/// 値のメタフィールド（イベント名のハンドラ）を取得（無ければ `nil`）。
pub fn get_metafield(state: &mut LuaState, v: Value, event: &str) -> Value {
    let Some(mtk) = metatable_handle(state, v) else {
        return Value::Nil;
    };
    let key = state.new_string(event.as_bytes());
    state.global.heap.get_table(mtk).map(|t| t.get(&key)).unwrap_or(Value::Nil)
}

// ============================================================================
// tostring（__tostring 込み）
// ============================================================================

/// GC オブジェクトの疑似アドレス（`tostring` 表示用）。
fn gc_addr(h: GcHandle) -> u64 {
    match h {
        GcHandle::Str(k) => k.data().as_ffi(),
        GcHandle::Table(k) => k.data().as_ffi(),
        GcHandle::Closure(k) => k.data().as_ffi(),
        GcHandle::Userdata(k) => k.data().as_ffi(),
    }
}

/// 値を `tostring` のバイト列へ変換する（`__tostring` メタメソッドを尊重）。
pub fn tostring_value(state: &mut LuaState, v: Value) -> LuaResult<Vec<u8>> {
    let mm = get_metafield(state, v, "__tostring");
    if !matches!(mm, Value::Nil) {
        let res = crate::vm::call(state, mm, &[v])?;
        let first = res.into_iter().next().unwrap_or(Value::Nil);
        return match first {
            Value::GcRef(GcHandle::Str(k)) => Ok(state.global.heap.get_str(k).unwrap().as_bytes().to_vec()),
            _ => Err(rt_error(state, "'__tostring' must return a string")),
        };
    }
    Ok(raw_tostring(state, v))
}

/// メタメソッド非経由の既定 `tostring`。
pub fn raw_tostring(state: &LuaState, v: Value) -> Vec<u8> {
    match v {
        Value::Nil => b"nil".to_vec(),
        Value::Boolean(true) => b"true".to_vec(),
        Value::Boolean(false) => b"false".to_vec(),
        Value::Number(n) => number_to_string(n).into_bytes(),
        Value::LightUserData(p) => format!("userdata: {p:p}").into_bytes(),
        Value::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec()).unwrap_or_default()
        }
        Value::GcRef(h) => {
            let kind = match h {
                GcHandle::Table(_) => "table",
                GcHandle::Closure(_) => "function",
                GcHandle::Userdata(_) => "userdata",
                GcHandle::Str(_) => unreachable!(),
            };
            format!("{kind}: 0x{:012x}", gc_addr(h)).into_bytes()
        }
    }
}

// ============================================================================
// 比較（table.sort の既定比較・< セマンティクス）
// ============================================================================

/// Lua の `a < b`（数値・文字列・`__lt` メタメソッド）。
pub fn lua_lt(state: &mut LuaState, a: Value, b: Value) -> LuaResult<bool> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(x < y),
        (Value::GcRef(GcHandle::Str(ka)), Value::GcRef(GcHandle::Str(kb))) => {
            let sa = state.global.heap.get_str(ka).unwrap().as_bytes().to_vec();
            let sb = state.global.heap.get_str(kb).unwrap().as_bytes().to_vec();
            Ok(sa < sb)
        }
        _ => {
            let mut mm = get_metafield(state, a, "__lt");
            if matches!(mm, Value::Nil) {
                mm = get_metafield(state, b, "__lt");
            }
            if matches!(mm, Value::Nil) {
                let (ta, tb) = (a.type_of().name(), b.type_of().name());
                let msg = if ta == tb {
                    format!("attempt to compare two {ta} values")
                } else {
                    format!("attempt to compare {ta} with {tb}")
                };
                return Err(rt_error(state, msg));
            }
            let res = crate::vm::call(state, mm, &[a, b])?;
            Ok(res.into_iter().next().unwrap_or(Value::Nil).is_truthy())
        }
    }
}

/// `luaL_where(level)` 相当。`level` 段上の呼び出し元の `"source:line: "` を返す。
///
/// 呼び出し規約: 本関数はネイティブ関数（`error`/`assert` 等）から呼ばれる前提で、
/// `call_info` 最上段が当該ネイティブフレーム。`level==1` はその呼び出し元（直近の
/// Lua フレーム）を指す。位置が取れない（C 関数フレーム等）場合は空文字列。
pub fn lua_where(state: &LuaState, level: u32) -> String {
    if level == 0 {
        return String::new();
    }
    let n = state.call_info.len();
    let Some(idx) = n.checked_sub(1 + level as usize) else {
        return String::new();
    };
    let ci = &state.call_info[idx];
    match &ci.source {
        Some(src) if ci.current_line > 0 => format!("{}:{}: ", src, ci.current_line),
        _ => String::new(),
    }
}

/// 値の Lua 型名（`type` 関数用）。
pub fn type_name(v: Value) -> &'static str {
    // lightuserdata も "userdata"。
    match v.type_of() {
        LuaType::LightUserData => "userdata",
        other => other.name(),
    }
}
