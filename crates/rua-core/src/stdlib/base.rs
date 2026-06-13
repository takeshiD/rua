//! base ライブラリ（本家 `lbaselib.c` 相当）。担当: **lua-stdlib**。
//!
//! `print`/`type`/`tostring`/`tonumber`/`pairs`/`ipairs`/`next`/`select`/`error`/`assert`/
//! `pcall`/`xpcall`/`rawget`/`rawset`/`rawequal`/`setmetatable`/`getmetatable`/`unpack` と
//! グローバル `_G`/`_VERSION` を登録する。

use std::io::Write;
use std::rc::Rc;

use crate::compiler::compile;
use crate::error::{LuaError, LuaResult};
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::closure::{Closure, LuaClosure};
use crate::value::convert::str_to_number;

use super::aux;

/// base ライブラリをグローバル環境へ登録する。
pub fn open(state: &mut LuaState) {
    let g = match state.global.globals {
        GcHandle::Table(k) => k,
        _ => return,
    };
    aux::register(state, g, "print", l_print);
    aux::register(state, g, "type", l_type);
    aux::register(state, g, "tostring", l_tostring);
    aux::register(state, g, "tonumber", l_tonumber);
    aux::register(state, g, "ipairs", l_ipairs);
    aux::register(state, g, "pairs", l_pairs);
    aux::register(state, g, "next", l_next);
    aux::register(state, g, "select", l_select);
    aux::register(state, g, "error", l_error);
    aux::register(state, g, "assert", l_assert);
    aux::register(state, g, "pcall", l_pcall);
    aux::register(state, g, "xpcall", l_xpcall);
    aux::register(state, g, "rawget", l_rawget);
    aux::register(state, g, "rawset", l_rawset);
    aux::register(state, g, "rawequal", l_rawequal);
    aux::register(state, g, "rawlen", l_rawlen);
    aux::register(state, g, "setmetatable", l_setmetatable);
    aux::register(state, g, "getmetatable", l_getmetatable);
    aux::register(state, g, "unpack", l_unpack);
    aux::register(state, g, "loadstring", l_loadstring);
    aux::register(state, g, "load", l_load);
    aux::register(state, g, "loadfile", l_loadfile);
    aux::register(state, g, "dofile", l_dofile);
    aux::register(state, g, "collectgarbage", l_collectgarbage);

    // _G はグローバル環境テーブル自身。
    aux::set_field(state, g, "_G", Value::GcRef(state.global.globals));
    let ver = state.new_string(b"Lua 5.1");
    aux::set_field(state, g, "_VERSION", ver);
}

// ============================================================================
// print / type / tostring / tonumber
// ============================================================================

fn l_print(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut out: Vec<u8> = Vec::new();
    for (i, v) in args.iter().enumerate() {
        if i > 0 {
            out.push(b'\t');
        }
        let s = aux::tostring_value(state, *v)?;
        out.extend_from_slice(&s);
    }
    out.push(b'\n');
    let stdout = std::io::stdout();
    let _ = stdout.lock().write_all(&out);
    aux::ret0(state)
}

fn l_type(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "type", "value expected"));
    }
    let name = aux::type_name(args[0]);
    let v = state.new_string(name.as_bytes());
    aux::ret(state, vec![v])
}

fn l_tostring(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    let bytes = aux::tostring_value(state, v)?;
    let s = state.new_string(&bytes);
    aux::ret(state, vec![s])
}

fn l_tonumber(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    if matches!(aux::opt_value(&args, 1), Value::Nil) {
        // 基数なし: number はそのまま、数値文字列は変換、その他 nil。
        let result = match v {
            Value::Number(_) => v,
            Value::GcRef(GcHandle::Str(k)) => {
                let bytes = state.global.heap.get_str(k).unwrap().as_bytes().to_vec();
                match str_to_number(&bytes) {
                    Some(n) => Value::Number(n),
                    None => Value::Nil,
                }
            }
            _ => Value::Nil,
        };
        aux::ret(state, vec![result])
    } else {
        // 指定基数: 第1引数は文字列、第2引数は基数（2..=36）。
        let base = aux::check_int(state, &args, 1, "tonumber")?;
        if !(2..=36).contains(&base) {
            return Err(aux::arg_error(state, 2, "tonumber", "base out of range"));
        }
        let bytes = aux::check_str_bytes(state, &args, 0, "tonumber")?;
        let result = parse_in_base(&bytes, base as u32)
            .map(Value::Number)
            .unwrap_or(Value::Nil);
        aux::ret(state, vec![result])
    }
}

/// 指定基数で文字列を整数へ（本家 `tonumber(s, base)`）。前後空白を無視。
fn parse_in_base(bytes: &[u8], base: u32) -> Option<f64> {
    let s = std::str::from_utf8(bytes).ok()?;
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return None;
    }
    let (neg, body) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if body.is_empty() {
        return None;
    }
    let mut acc = 0.0f64;
    for ch in body.chars() {
        let d = ch.to_digit(base)?;
        acc = acc * base as f64 + d as f64;
    }
    Some(if neg { -acc } else { acc })
}

// ============================================================================
// pairs / ipairs / next
// ============================================================================

fn ipairs_aux(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "ipairs")?;
    let i = aux::check_int(state, &args, 1, "ipairs")? + 1;
    let v = if i >= 1 {
        state
            .global
            .heap
            .get_table(tk)
            .map(|t| t.get_int(i as usize))
            .unwrap_or(Value::Nil)
    } else {
        Value::Nil
    };
    if matches!(v, Value::Nil) {
        aux::ret0(state)
    } else {
        aux::ret(state, vec![Value::Number(i as f64), v])
    }
}

fn l_ipairs(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let _ = aux::check_table(state, &args, 0, "ipairs")?;
    let t = args[0];
    let iter = aux::make_native(state, ipairs_aux);
    aux::ret(state, vec![iter, t, Value::Number(0.0)])
}

fn l_pairs(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let _ = aux::check_table(state, &args, 0, "pairs")?;
    let t = args[0];
    let iter = aux::make_native(state, l_next);
    aux::ret(state, vec![iter, t, Value::Nil])
}

fn l_next(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "next")?;
    let key = aux::opt_value(&args, 1);
    let result = state.global.heap.get_table(tk).map(|t| t.next(&key));
    match result {
        Some(Ok(Some((k, v)))) => aux::ret(state, vec![k, v]),
        Some(Ok(None)) => aux::ret(state, vec![Value::Nil]),
        Some(Err(())) => Err(aux::rt_error(state, "invalid key to 'next'")),
        None => aux::ret(state, vec![Value::Nil]),
    }
}

// ============================================================================
// select
// ============================================================================

fn l_select(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let first = aux::opt_value(&args, 0);
    // select('#', ...) は引数個数。
    if let Value::GcRef(GcHandle::Str(k)) = first {
        let is_hash = state
            .global
            .heap
            .get_str(k)
            .map(|s| s.as_bytes() == b"#")
            .unwrap_or(false);
        if is_hash {
            let n = (args.len() - 1) as f64;
            return aux::ret(state, vec![Value::Number(n)]);
        }
    }
    let n = aux::check_int(state, &args, 0, "select")?;
    let total = (args.len() - 1) as i64;
    let start = if n < 0 {
        let s = total + n + 1;
        if s < 1 {
            return Err(aux::arg_error(state, 1, "select", "index out of range"));
        }
        s
    } else if n == 0 {
        return Err(aux::arg_error(state, 1, "select", "index out of range"));
    } else {
        n
    };
    let mut out = Vec::new();
    let mut i = start;
    while i <= total {
        out.push(args[i as usize]); // args[0] は select 自身の第1引数なので i がそのまま可変長添字
        i += 1;
    }
    aux::ret(state, out)
}

// ============================================================================
// error / assert
// ============================================================================

fn l_error(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let msg = aux::opt_value(&args, 0);
    let level = aux::opt_int(state, &args, 1, "error", 1)?;
    // 本家 `luaB_error`: 文字列メッセージかつ level>0 なら "source:line: " を前置する
    // （`luaL_where(level)` 相当）。非文字列や level==0 はそのまま送出。
    let errval = match msg {
        Value::GcRef(GcHandle::Str(k)) if level > 0 => {
            let body = state.global.heap.get_str(k).unwrap().as_bytes().to_vec();
            let prefix = aux::lua_where(state, level as u32);
            let mut buf = prefix.into_bytes();
            buf.extend_from_slice(&body);
            state.new_string(&buf)
        }
        other => other,
    };
    Err(LuaError::Runtime(errval))
}

fn l_assert(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "assert", "value expected"));
    }
    let v = args[0];
    if v.is_truthy() {
        // 全引数をそのまま返す。
        aux::ret(state, args)
    } else {
        // 本家 `luaB_assert`: `luaL_error(L, "%s", msg)`。msg は文字列化され、
        // level 1（assert の呼び出し元）の位置が前置される。
        let msg_bytes = match aux::opt_value(&args, 1) {
            Value::Nil => b"assertion failed!".to_vec(),
            _ => aux::check_str_bytes(state, &args, 1, "assert")?,
        };
        let prefix = aux::lua_where(state, 1);
        let mut buf = prefix.into_bytes();
        buf.extend_from_slice(&msg_bytes);
        let ev = state.new_string(&buf);
        Err(LuaError::Runtime(ev))
    }
}

// ============================================================================
// pcall / xpcall
// ============================================================================

/// `LuaError` を pcall が返すエラーオブジェクト（Lua 値）へ変換する。
fn error_to_value(state: &mut LuaState, e: LuaError) -> Value {
    match e {
        LuaError::Runtime(v) => v,
        LuaError::Syntax(s) | LuaError::Internal(s) => state.new_string(s.as_bytes()),
        LuaError::Memory => state.new_string(b"not enough memory"),
        LuaError::ErrorInError => state.new_string(b"error in error handling"),
        LuaError::Yield(_) => state.new_string(b"attempt to yield across a C-call boundary"),
    }
}

fn l_pcall(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "pcall", "value expected"));
    }
    let func = args[0];
    let call_args = args[1..].to_vec();
    let result = crate::state::call::pcall(state, |s| crate::vm::call(s, func, &call_args));
    match result {
        Ok(rets) => {
            let mut out = Vec::with_capacity(rets.len() + 1);
            out.push(Value::Boolean(true));
            out.extend(rets);
            aux::ret(state, out)
        }
        Err(e) => {
            let ev = error_to_value(state, e);
            aux::ret(state, vec![Value::Boolean(false), ev])
        }
    }
}

fn l_xpcall(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let func = aux::opt_value(&args, 0);
    let handler = aux::opt_value(&args, 1);
    if matches!(func, Value::Nil) {
        return Err(aux::arg_error(state, 1, "xpcall", "value expected"));
    }
    let result = crate::state::call::pcall(state, |s| crate::vm::call(s, func, &[]));
    match result {
        Ok(rets) => {
            let mut out = Vec::with_capacity(rets.len() + 1);
            out.push(Value::Boolean(true));
            out.extend(rets);
            aux::ret(state, out)
        }
        Err(e) => {
            let ev = error_to_value(state, e);
            // ハンドラを errobj で呼ぶ。
            let hres = crate::vm::call(state, handler, &[ev])?;
            let hval = hres.into_iter().next().unwrap_or(Value::Nil);
            aux::ret(state, vec![Value::Boolean(false), hval])
        }
    }
}

// ============================================================================
// raw アクセス
// ============================================================================

fn l_rawget(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "rawget")?;
    let key = aux::opt_value(&args, 1);
    let v = state
        .global
        .heap
        .get_table(tk)
        .map(|t| t.get(&key))
        .unwrap_or(Value::Nil);
    aux::ret(state, vec![v])
}

fn l_rawset(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "rawset")?;
    let key = aux::opt_value(&args, 1);
    let val = aux::opt_value(&args, 2);
    let res = state.global.heap.get_table_mut(tk).map(|t| t.set(key, val));
    match res {
        Some(Err(crate::value::table::TableKeyError::NilKey)) => {
            Err(aux::rt_error(state, "table index is nil"))
        }
        Some(Err(crate::value::table::TableKeyError::NanKey)) => {
            Err(aux::rt_error(state, "table index is NaN"))
        }
        _ => aux::ret(state, vec![args[0]]),
    }
}

fn l_rawequal(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let a = aux::opt_value(&args, 0);
    let b = aux::opt_value(&args, 1);
    aux::ret(state, vec![Value::Boolean(a == b)])
}

fn l_rawlen(state: &mut LuaState) -> LuaResult<i32> {
    // Lua 5.1 には rawlen は無いが、table/string の生の長さ取得として提供（簡便）。
    let args = aux::args_vec(state);
    let len = match aux::opt_value(&args, 0) {
        Value::GcRef(GcHandle::Table(k)) => state
            .global
            .heap
            .get_table(k)
            .map(|t| t.length())
            .unwrap_or(0),
        Value::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(k).map(|s| s.len()).unwrap_or(0)
        }
        _ => {
            return Err(aux::arg_error(
                state,
                1,
                "rawlen",
                "table or string expected",
            ));
        }
    };
    aux::ret(state, vec![Value::Number(len as f64)])
}

// ============================================================================
// setmetatable / getmetatable
// ============================================================================

fn l_setmetatable(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "setmetatable")?;
    let mt = aux::opt_value(&args, 1);
    let mt_handle = match mt {
        Value::Nil => None,
        Value::GcRef(h @ GcHandle::Table(_)) => Some(h),
        _ => {
            return Err(aux::arg_error(
                state,
                2,
                "setmetatable",
                "nil or table expected",
            ));
        }
    };
    // 既存メタテーブルが __metatable で保護されていれば変更不可。
    if has_protected_metatable(state, tk) {
        return Err(aux::rt_error(state, "cannot change a protected metatable"));
    }
    if let Some(t) = state.global.heap.get_table_mut(tk) {
        t.set_metatable(mt_handle);
    }
    aux::ret(state, vec![args[0]])
}

fn has_protected_metatable(state: &mut LuaState, tk: TableKey) -> bool {
    !matches!(
        aux::get_metafield(state, Value::GcRef(GcHandle::Table(tk)), "__metatable"),
        Value::Nil
    )
}

fn l_getmetatable(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    let Some(mtk) = aux::metatable_handle(state, v) else {
        return aux::ret(state, vec![Value::Nil]);
    };
    // __metatable があればそれを返す（保護）。
    let protected = aux::get_metafield(state, v, "__metatable");
    if !matches!(protected, Value::Nil) {
        return aux::ret(state, vec![protected]);
    }
    aux::ret(state, vec![Value::GcRef(GcHandle::Table(mtk))])
}

// ============================================================================
// unpack
// ============================================================================

fn l_unpack(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "unpack")?;
    let i = aux::opt_int(state, &args, 1, "unpack", 1)?;
    let j = if matches!(aux::opt_value(&args, 2), Value::Nil) {
        state
            .global
            .heap
            .get_table(tk)
            .map(|t| t.length())
            .unwrap_or(0) as i64
    } else {
        aux::check_int(state, &args, 2, "unpack")?
    };
    let mut out = Vec::new();
    let mut idx = i;
    while idx <= j {
        let v = if idx >= 1 {
            state
                .global
                .heap
                .get_table(tk)
                .map(|t| t.get_int(idx as usize))
                .unwrap_or(Value::Nil)
        } else {
            state
                .global
                .heap
                .get_table(tk)
                .map(|t| t.get(&Value::Number(idx as f64)))
                .unwrap_or(Value::Nil)
        };
        out.push(v);
        idx += 1;
    }
    aux::ret(state, out)
}

// ============================================================================
// loadstring / load / loadfile / dofile（本家 lbaselib.c の load 系）
// ============================================================================

/// ソースをコンパイルしてメインチャンクの関数値を作る。
/// 成功で関数値、失敗で構文エラーメッセージ（文字列）を返す。
fn compile_to_function(state: &mut LuaState, src: &[u8], chunkname: &str) -> Result<Value, String> {
    match compile(&mut state.global.heap, src, chunkname) {
        Ok(proto) => {
            let closure = LuaClosure::new(Rc::new(proto));
            let h = state.global.heap.alloc_closure(Closure::Lua(closure));
            Ok(Value::GcRef(h))
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// コンパイル結果を `function` または `nil, errmsg` として返す（load 系の共通処理）。
fn ret_loaded(state: &mut LuaState, compiled: Result<Value, String>) -> LuaResult<i32> {
    match compiled {
        Ok(f) => aux::ret(state, vec![f]),
        Err(e) => {
            let msg = state.new_string(e.as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `loadstring(s [, chunkname])` — 文字列をコンパイルして関数を返す（失敗で nil, errmsg）。
fn l_loadstring(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let src = aux::check_str_bytes(state, &args, 0, "loadstring")?;
    // chunkname 省略時は本家同様ソース文字列自体（compile 側で `[string "..."]` に短縮）。
    let chunkname = match aux::opt_value(&args, 1) {
        Value::Nil => String::from_utf8_lossy(&src).into_owned(),
        _ => String::from_utf8_lossy(&aux::check_str_bytes(state, &args, 1, "loadstring")?)
            .into_owned(),
    };
    let compiled = compile_to_function(state, &src, &chunkname);
    ret_loaded(state, compiled)
}

/// `load(func [, chunkname])` — reader 関数を繰り返し呼んでチャンクを組み立て、コンパイルする。
fn l_load(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let func = aux::opt_value(&args, 0);
    let chunkname = match aux::opt_value(&args, 1) {
        Value::Nil => "=(load)".to_string(),
        _ => String::from_utf8_lossy(&aux::check_str_bytes(state, &args, 1, "load")?).into_owned(),
    };
    let mut src: Vec<u8> = Vec::new();
    loop {
        let piece = crate::vm::call(state, func, &[])?;
        match piece.into_iter().next() {
            Some(Value::GcRef(GcHandle::Str(k))) => {
                let bytes = state
                    .global
                    .heap
                    .get_str(k)
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                if bytes.is_empty() {
                    break; // 空文字列で終端（本家準拠）。
                }
                src.extend_from_slice(&bytes);
            }
            // nil / 値なし / その他 → 終端。
            _ => break,
        }
    }
    let compiled = compile_to_function(state, &src, &chunkname);
    ret_loaded(state, compiled)
}

/// ファイル（省略時は stdin）を読み、先頭の `#` 行（shebang）は読み飛ばす。
fn read_file_source(state: &mut LuaState, args: &[Value]) -> Result<(Vec<u8>, String), String> {
    let (mut src, name) = match aux::opt_value(args, 0) {
        Value::Nil => {
            use std::io::Read;
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            (buf, "=stdin".to_string())
        }
        Value::GcRef(GcHandle::Str(k)) => {
            let path = state
                .global
                .heap
                .get_str(k)
                .map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned())
                .unwrap_or_default();
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot open {path}: {e}"))?;
            (bytes, format!("@{path}"))
        }
        _ => return Err("bad argument (string expected)".to_string()),
    };
    // shebang（先頭が '#'）の行を読み飛ばす。行番号維持のため改行は残す。
    if src.first() == Some(&b'#') {
        if let Some(pos) = src.iter().position(|&b| b == b'\n') {
            src.drain(..pos);
        } else {
            src.clear();
        }
    }
    Ok((src, name))
}

/// `loadfile([filename])` — ファイル/stdin をコンパイルして関数を返す（失敗で nil, errmsg）。
fn l_loadfile(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    match read_file_source(state, &args) {
        Ok((src, name)) => {
            let compiled = compile_to_function(state, &src, &name);
            ret_loaded(state, compiled)
        }
        Err(e) => {
            let msg = state.new_string(e.as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `dofile([filename])` — ファイル/stdin をロードして即実行する。エラーは送出（pcall 可能）。
fn l_dofile(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let (src, name) = read_file_source(state, &args).map_err(|e| aux::rt_error(state, e))?;
    let f = compile_to_function(state, &src, &name).map_err(|e| aux::rt_error(state, e))?;
    let rets = crate::vm::call(state, f, &[])?;
    aux::ret(state, rets)
}

// ============================================================================
// collectgarbage（本家 lbaselib.c `luaB_collectgarbage`）
// ============================================================================

/// `collectgarbage([opt [, arg]])` — GC 制御。
///
/// 現状の rua は実行中に GC を起動していない（ルート集合の収集が未配線, #17）。
/// 不完全なルートで `Heap::collect` を呼ぶと生存オブジェクトを誤回収して壊れるため、
/// `collect`/`step` は **安全な no-op**（実回収しない）とし、各オプションは本家準拠の
/// 形（型・既定値）で値を返す。`count` はメモリ会計未実装のため生存オブジェクト数を
/// KB の近似として返す（Lua 5.1 は単一の数値を返す）。
fn l_collectgarbage(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let opt = match aux::opt_value(&args, 0) {
        Value::Nil => b"collect".to_vec(),
        Value::GcRef(GcHandle::Str(k)) => state
            .global
            .heap
            .get_str(k)
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default(),
        _ => {
            return Err(aux::arg_error(
                state,
                1,
                "collectgarbage",
                "string expected",
            ));
        }
    };
    let result = match opt.as_slice() {
        // 実回収は #17（ルート収集）待ち。ここでは安全に何もしない。
        b"collect" | b"" => Value::Number(0.0),
        // KB 近似（生存オブジェクト数）。
        b"count" => Value::Number(state.global.heap.live_object_count() as f64),
        b"step" => Value::Boolean(false),
        b"setpause" => Value::Number(200.0),
        b"setstepmul" => Value::Number(100.0),
        b"stop" | b"restart" => Value::Number(0.0),
        _ => Value::Number(0.0),
    };
    aux::ret(state, vec![result])
}
