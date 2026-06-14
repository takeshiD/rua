//! debug ライブラリ（本家 `ldblib.c` 相当）。担当: **lua-stdlib**。
//!
//! `traceback`/`getinfo`/`getmetatable`/`setmetatable`/`getregistry`/
//! `getupvalue`/`setupvalue`/`getlocal`/`setlocal`/`sethook`/`gethook` を提供する。
//!
//! # 設計方針
//! - `debug.traceback` と `debug.getinfo` はテスト互換上最重要。必ず文字列/テーブルを返す。
//! - `sethook`/`gethook` はフック機構未実装のため no-op スタブ。
//! - `getlocal`/`setlocal` は最小実装（範囲外相当の nil を返すスタブ）。

use crate::error::LuaResult;
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::closure::Closure;

use super::aux;

/// debug ライブラリをグローバル環境へ開く。
pub fn open(state: &mut LuaState) {
    let t = state.new_table();
    let tk = match t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    aux::register(state, tk, "traceback", l_traceback);
    aux::register(state, tk, "getinfo", l_getinfo);
    aux::register(state, tk, "getmetatable", l_getmetatable);
    aux::register(state, tk, "setmetatable", l_setmetatable);
    aux::register(state, tk, "getregistry", l_getregistry);
    aux::register(state, tk, "getupvalue", l_getupvalue);
    aux::register(state, tk, "setupvalue", l_setupvalue);
    aux::register(state, tk, "getlocal", l_getlocal);
    aux::register(state, tk, "setlocal", l_setlocal);
    aux::register(state, tk, "sethook", l_sethook);
    aux::register(state, tk, "gethook", l_gethook);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "debug", t);
    }
}

// ============================================================================
// debug.traceback([message [, level]])
// ============================================================================

/// コールスタックを文字列として組み立てる。
///
/// call_info を末尾から辿り、source:line の情報を列挙する。
fn build_traceback(state: &LuaState) -> String {
    let mut lines: Vec<String> = Vec::new();
    // call_info は [0]=最古フレーム、末尾=現在フレーム の順。
    // 末尾フレームは traceback 自身（ネイティブ）なのでスキップする。
    let frames = state.call_info.len();
    let skip = if frames > 0 { 1 } else { 0 };

    for ci in state.call_info[..frames.saturating_sub(skip)].iter().rev() {
        match &ci.source {
            Some(src) if ci.current_line > 0 => {
                lines.push(format!("\t{}:{}", src, ci.current_line));
            }
            Some(src) => {
                lines.push(format!("\t{}: ?", src));
            }
            None => {
                lines.push("\t[C]: ?".to_string());
            }
        }
    }

    if lines.is_empty() {
        String::from("stack traceback:")
    } else {
        format!("stack traceback:\n{}", lines.join("\n"))
    }
}

fn l_traceback(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let msg = aux::opt_value(&args, 0);
    // _level = args[1] (省略可, 未使用だが受け取る)

    let result = match msg {
        // nil: スタックトレース文字列のみ。
        Value::Nil => {
            let tb = build_traceback(state);
            state.new_string(tb.as_bytes())
        }
        // 文字列: メッセージ + "\n" + スタックトレース。
        Value::GcRef(GcHandle::Str(k)) => {
            let prefix = state
                .global
                .heap
                .get_str(k)
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default();
            let prefix_str = String::from_utf8_lossy(&prefix).into_owned();
            let tb = build_traceback(state);
            let full = format!("{}\n{}", prefix_str, tb);
            state.new_string(full.as_bytes())
        }
        // 非文字列かつ非 nil: そのまま返す（本家の動作）。
        other => other,
    };

    aux::ret(state, vec![result])
}

// ============================================================================
// debug.getinfo([f] [, what])
// ============================================================================

/// Lua クロージャの情報をテーブルへ埋める。
fn fill_lua_closure_info(state: &mut LuaState, tk: TableKey, closure_key: crate::gc::ClosureKey) {
    let (source, short_src, line_defined, last_line_defined, nups) =
        match state.global.heap.get_closure(closure_key) {
            Some(Closure::Lua(lc)) => {
                let proto = lc.proto().clone();
                let src = proto.source.clone().unwrap_or_else(|| "=?".to_string());
                let short_src =
                    if let Some(rest) = src.strip_prefix('@').or_else(|| src.strip_prefix('=')) {
                        rest.to_string()
                    } else {
                        // チャンク文字列の短縮表示（先頭60文字）。
                        let truncated: String = src.chars().take(60).collect();
                        format!("[string \"{}\"]", truncated.replace('\n', " "))
                    };
                (
                    src,
                    short_src,
                    proto.line_defined,
                    proto.last_line_defined,
                    lc.upvalues().len(),
                )
            }
            Some(Closure::Native(_)) => ("=[C]".to_string(), "[C]".to_string(), 0u32, 0u32, 0usize),
            None => return,
        };

    let src_val = state.new_string(source.as_bytes());
    aux::set_field(state, tk, "source", src_val);

    let short_val = state.new_string(short_src.as_bytes());
    aux::set_field(state, tk, "short_src", short_val);

    aux::set_field(state, tk, "linedefined", Value::Number(line_defined as f64));
    aux::set_field(
        state,
        tk,
        "lastlinedefined",
        Value::Number(last_line_defined as f64),
    );
    aux::set_field(state, tk, "nups", Value::Number(nups as f64));

    // what: Lua / C / main
    let what = match state.global.heap.get_closure(closure_key) {
        Some(Closure::Lua(lc)) => {
            if lc.proto().line_defined == 0 {
                "main"
            } else {
                "Lua"
            }
        }
        _ => "C",
    };
    let what_val = state.new_string(what.as_bytes());
    aux::set_field(state, tk, "what", what_val);
}

fn l_getinfo(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);

    // 戻り値テーブルを確保。
    let tbl = state.new_table();
    let tk = match tbl {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return aux::ret(state, vec![Value::Nil]),
    };

    // デフォルト値を設定。
    aux::set_field(state, tk, "currentline", Value::Number(-1.0));
    aux::set_field(state, tk, "linedefined", Value::Number(-1.0));
    aux::set_field(state, tk, "lastlinedefined", Value::Number(-1.0));
    aux::set_field(state, tk, "nups", Value::Number(0.0));
    {
        let nil_v = Value::Nil;
        aux::set_field(state, tk, "name", nil_v);
    }
    {
        let empty = state.new_string(b"");
        aux::set_field(state, tk, "namewhat", empty);
    }

    match aux::opt_value(&args, 0) {
        // 引数なし、または nil: 呼び出し元（スタックレベル1）の情報。
        Value::Nil => {
            // 呼び出し元のフレームを探す。
            // call_info の末尾は getinfo 自身（ネイティブ）、その1つ前が呼び出し元。
            let frames = state.call_info.len();
            if frames >= 2 {
                let ci = &state.call_info[frames - 2];
                let current_line = ci.current_line;
                let source = ci.source.clone();
                let closure_key = if let Some(Value::GcRef(GcHandle::Closure(k))) = state
                    .stack
                    .get(state.call_info[frames - 2].func)
                    .copied()
                    .map(Some)
                    .unwrap_or(None)
                {
                    Some(k)
                } else {
                    None
                };

                aux::set_field(state, tk, "currentline", Value::Number(current_line as f64));
                if let Some(src) = source {
                    let src_v = state.new_string(src.as_bytes());
                    aux::set_field(state, tk, "source", src_v);
                    let short = src.strip_prefix('@').unwrap_or(src.as_str()).to_string();
                    let short_v = state.new_string(short.as_bytes());
                    aux::set_field(state, tk, "short_src", short_v);
                }
                if let Some(ck) = closure_key {
                    let func_v = Value::GcRef(GcHandle::Closure(ck));
                    aux::set_field(state, tk, "func", func_v);
                    fill_lua_closure_info(state, tk, ck);
                }
            }
        }

        // 数値: スタックレベル。
        Value::Number(level) => {
            let level = level as usize;
            let frames = state.call_info.len();
            // level 0 = 現在の関数（getinfo 自身）、level 1 = 呼び出し元...
            if level < frames {
                let ci_idx = frames - 1 - level;
                let ci = &state.call_info[ci_idx];
                let current_line = ci.current_line;
                let source = ci.source.clone();
                let func_stack_pos = ci.func;

                aux::set_field(state, tk, "currentline", Value::Number(current_line as f64));
                if let Some(src) = source {
                    let src_v = state.new_string(src.as_bytes());
                    aux::set_field(state, tk, "source", src_v);
                    let short = src.strip_prefix('@').unwrap_or(src.as_str()).to_string();
                    let short_v = state.new_string(short.as_bytes());
                    aux::set_field(state, tk, "short_src", short_v);
                }

                // func フィールド: スタック上の関数値。
                if let Some(&func_val) = state.stack.get(func_stack_pos) {
                    aux::set_field(state, tk, "func", func_val);
                    if let Value::GcRef(GcHandle::Closure(ck)) = func_val {
                        fill_lua_closure_info(state, tk, ck);
                    }
                }
            }
        }

        // 関数値: クロージャ情報を直接取得。
        Value::GcRef(GcHandle::Closure(ck)) => {
            let func_v = Value::GcRef(GcHandle::Closure(ck));
            aux::set_field(state, tk, "func", func_v);
            fill_lua_closure_info(state, tk, ck);
            // スタックから currentline を探す（呼び出し中なら）。
            // 関数値のみ指定時は currentline = -1 のまま（呼び出し元なし）。
        }

        // その他（テーブル等）は nil を返す（無効引数）。
        _ => {
            return aux::ret(state, vec![Value::Nil]);
        }
    }

    aux::ret(state, vec![tbl])
}

// ============================================================================
// debug.getmetatable(v) — __metatable を無視して生のメタテーブルを返す。
// ============================================================================

fn l_getmetatable(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    // aux::metatable_handle は __metatable を考慮しないので直接取得する。
    let mt = match v {
        Value::GcRef(GcHandle::Table(k)) => {
            state.global.heap.get_table(k).and_then(|t| t.metatable())
        }
        Value::GcRef(GcHandle::Userdata(k)) => state
            .global
            .heap
            .get_userdata(k)
            .and_then(|u| u.metatable()),
        Value::GcRef(GcHandle::Str(_)) => state.global.string_metatable,
        _ => None,
    };
    let result = match mt {
        Some(h) => Value::GcRef(h),
        None => Value::Nil,
    };
    aux::ret(state, vec![result])
}

// ============================================================================
// debug.setmetatable(v, mt) — __metatable チェックなしに設定し v を返す。
// ============================================================================

fn l_setmetatable(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    let mt = aux::opt_value(&args, 1);
    let mt_handle = match mt {
        Value::Nil => None,
        Value::GcRef(h @ GcHandle::Table(_)) => Some(h),
        _ => {
            return Err(aux::arg_error(
                state,
                2,
                "debug.setmetatable",
                "nil or table expected",
            ));
        }
    };
    match v {
        Value::GcRef(GcHandle::Table(tk)) => {
            if let Some(t) = state.global.heap.get_table_mut(tk) {
                t.set_metatable(mt_handle);
            }
        }
        Value::GcRef(GcHandle::Userdata(uk)) => {
            if let Some(u) = state.global.heap.get_userdata_mut(uk) {
                u.set_metatable(mt_handle);
            }
        }
        // 文字列型には string_metatable をセット。
        Value::GcRef(GcHandle::Str(_)) => {
            state.global.string_metatable = mt_handle;
        }
        // 数値型の型共有メタテーブル。
        Value::Number(_) => {
            state.global.number_metatable = mt_handle;
        }
        // boolean型の型共有メタテーブル。
        Value::Boolean(_) => {
            state.global.boolean_metatable = mt_handle;
        }
        // nil型の型共有メタテーブル。
        Value::Nil => {
            state.global.nil_metatable = mt_handle;
        }
        _ => {}
    }
    aux::ret(state, vec![v])
}

// ============================================================================
// debug.getregistry()
// ============================================================================

fn l_getregistry(state: &mut LuaState) -> LuaResult<i32> {
    let reg = Value::GcRef(state.global.registry);
    aux::ret(state, vec![reg])
}

// ============================================================================
// debug.getupvalue(f, n) — 1 始まり n 番目の upvalue（名前, 値）を返す。
// ============================================================================

fn l_getupvalue(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let f = aux::opt_value(&args, 0);
    let n = match aux::opt_value(&args, 1) {
        Value::Number(v) => v as usize,
        _ => return aux::ret(state, vec![Value::Nil]),
    };
    if n == 0 {
        return aux::ret(state, vec![Value::Nil]);
    }
    let idx = n - 1; // 0-origin へ変換。

    match f {
        Value::GcRef(GcHandle::Closure(ck)) => {
            match state.global.heap.get_closure(ck) {
                Some(Closure::Lua(lc)) => {
                    let name = lc
                        .proto()
                        .upvalue_names
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| format!("(upvalue {})", n));
                    let val = match lc.upvalue(idx) {
                        Some(uv) => match &*uv.borrow() {
                            crate::value::closure::UpvalueState::Closed(v) => *v,
                            crate::value::closure::UpvalueState::Open(stack_idx) => {
                                state.stack.get(*stack_idx).copied().unwrap_or(Value::Nil)
                            }
                        },
                        None => return aux::ret(state, vec![Value::Nil]),
                    };
                    let name_v = state.new_string(name.as_bytes());
                    aux::ret(state, vec![name_v, val])
                }
                Some(Closure::Native(_)) => {
                    // ネイティブクロージャの upvalue は名前情報なし。
                    aux::ret(state, vec![Value::Nil])
                }
                None => aux::ret(state, vec![Value::Nil]),
            }
        }
        _ => aux::ret(state, vec![Value::Nil]),
    }
}

// ============================================================================
// debug.setupvalue(f, n, v) — 1 始まり n 番目の upvalue を v に設定し名前を返す。
// ============================================================================

fn l_setupvalue(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let f = aux::opt_value(&args, 0);
    let n = match aux::opt_value(&args, 1) {
        Value::Number(v) => v as usize,
        _ => return aux::ret(state, vec![Value::Nil]),
    };
    let new_val = aux::opt_value(&args, 2);
    if n == 0 {
        return aux::ret(state, vec![Value::Nil]);
    }
    let idx = n - 1;

    match f {
        Value::GcRef(GcHandle::Closure(ck)) => {
            // upvalue名を先に取得（不変参照が必要なため）。
            let name = match state.global.heap.get_closure(ck) {
                Some(Closure::Lua(lc)) => {
                    if lc.upvalue(idx).is_none() {
                        return aux::ret(state, vec![Value::Nil]);
                    }
                    lc.proto()
                        .upvalue_names
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| format!("(upvalue {})", n))
                }
                _ => return aux::ret(state, vec![Value::Nil]),
            };

            // upvalue を書き換える。
            let uv_ref = match state.global.heap.get_closure(ck) {
                Some(Closure::Lua(lc)) => lc.upvalue(idx).cloned(),
                _ => None,
            };

            if let Some(uv) = uv_ref {
                let mut borrow = uv.borrow_mut();
                match &*borrow {
                    crate::value::closure::UpvalueState::Closed(_) => {
                        *borrow = crate::value::closure::UpvalueState::Closed(new_val);
                    }
                    crate::value::closure::UpvalueState::Open(stack_idx) => {
                        let si = *stack_idx;
                        drop(borrow);
                        if let Some(slot) = state.stack.get_mut(si) {
                            *slot = new_val;
                        }
                        let name_v = state.new_string(name.as_bytes());
                        return aux::ret(state, vec![name_v]);
                    }
                }
            } else {
                return aux::ret(state, vec![Value::Nil]);
            }

            let name_v = state.new_string(name.as_bytes());
            aux::ret(state, vec![name_v])
        }
        _ => aux::ret(state, vec![Value::Nil]),
    }
}

// ============================================================================
// debug.getlocal / debug.setlocal — スタブ（最小実装）。
// ============================================================================

fn l_getlocal(state: &mut LuaState) -> LuaResult<i32> {
    // 最小実装: 常に nil を返す。
    aux::ret(state, vec![Value::Nil])
}

fn l_setlocal(state: &mut LuaState) -> LuaResult<i32> {
    // 最小実装: 常に nil を返す。
    aux::ret(state, vec![Value::Nil])
}

// ============================================================================
// debug.sethook / debug.gethook — no-op スタブ（フック機構未実装）。
// ============================================================================

fn l_sethook(state: &mut LuaState) -> LuaResult<i32> {
    // フック機構未実装: 何もしない。
    aux::ret0(state)
}

fn l_gethook(state: &mut LuaState) -> LuaResult<i32> {
    // フック機構未実装: nil, "", 0 を返す（本家の返り値に倣う）。
    let empty = state.new_string(b"");
    aux::ret(state, vec![Value::Nil, empty, Value::Number(0.0)])
}
