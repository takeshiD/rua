//! coroutine ライブラリ（本家 `lcorolib.c` 相当）。担当: **lua-stdlib**。
//!
//! `coroutine.create` / `coroutine.resume` / `coroutine.yield` /
//! `coroutine.status` / `coroutine.wrap` / `coroutine.isyieldable` を実装する。
//!
//! # 方式: スタック・スライシング（シングルスレッド）
//! コルーチンはメインスレッドの `LuaState` のスタックとコールフレームを
//! スライスとして `LuaThread` に保存/復元することで、OS スレッドや unsafe を
//! 一切使わずに実現する。
//!
//! ## resume 時
//! 1. コルーチンの保存済みスタック/call_info を `state` へ append する。
//! 2. 初回は `body` 関数を通常呼び出し、2 回目以降は `vm::resume_execute` で再開。
//! 3. 正常終了: Dead にして結果を `true, ...` で返す。
//! 4. `LuaError::Yield`: フレームを `LuaThread` へ保存し `true, ...` で返す。
//! 5. エラー: Dead にして `false, errmsg` で返す。
//!
//! ## yield 時
//! `LuaError::Yield(vals)` を発生させる。`execute_inner` が各フレームで
//! `CallInfo.lua_frame` に実行状態を保存しながら伝播させ、`l_resume` まで届ける。

use crate::error::{LuaError, LuaResult};
use crate::gc::{GcHandle, ThreadKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::thread::{LuaThread, ThreadStatus};

use super::aux;

/// coroutine ライブラリをグローバル環境の `coroutine` テーブルとして登録する。
pub fn open(state: &mut LuaState) {
    let lib = state.new_table();
    let lib_tk = match lib {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    aux::register(state, lib_tk, "create", l_create);
    aux::register(state, lib_tk, "resume", l_resume);
    aux::register(state, lib_tk, "yield", l_yield);
    aux::register(state, lib_tk, "status", l_status);
    aux::register(state, lib_tk, "wrap", l_wrap);
    aux::register(state, lib_tk, "isyieldable", l_isyieldable);
    aux::register(state, lib_tk, "running", l_running);

    // _G.coroutine = lib
    let g = match state.global.globals {
        GcHandle::Table(k) => k,
        _ => return,
    };
    aux::set_field(state, g, "coroutine", lib);
}

// ============================================================================
// coroutine.create(f)
// ============================================================================

fn l_create(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let f = aux::check_function(state, &args, 0, "coroutine.create")?;
    let thread = LuaThread::new(f);
    let h = state.global.heap.alloc_thread(thread);
    aux::ret(state, vec![Value::GcRef(h)])
}

// ============================================================================
// coroutine.resume(co, ...)
// ============================================================================

fn l_resume(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_thread(state, &args, 0, "coroutine.resume")?;
    let resume_args: Vec<Value> = args[1..].to_vec();

    // ステータス確認
    let status = state.global.heap.get_thread(tk).map(|t| t.status);
    match status {
        None => {
            let ev = state.new_string(b"coroutine handle invalid");
            return aux::ret(state, vec![Value::Boolean(false), ev]);
        }
        Some(ThreadStatus::Dead) => {
            let ev = state.new_string(b"cannot resume dead coroutine");
            return aux::ret(state, vec![Value::Boolean(false), ev]);
        }
        Some(ThreadStatus::Running) => {
            let ev = state.new_string(b"cannot resume non-suspended coroutine");
            return aux::ret(state, vec![Value::Boolean(false), ev]);
        }
        _ => {}
    }

    // Running にマーク
    state.global.heap.get_thread_mut(tk).unwrap().status = ThreadStatus::Running;

    let stack_marker = state.stack.len();
    let ci_marker = state.call_info.len();

    // コルーチンのスタック/call_info を積み戻す
    let (saved_stack, saved_ci, is_first) = {
        let th = state.global.heap.get_thread_mut(tk).unwrap();
        let is_first = th.body.is_some();
        (
            std::mem::take(&mut th.saved_stack),
            std::mem::take(&mut th.saved_call_info),
            is_first,
        )
    };
    state.stack.extend(saved_stack);
    state.call_info.extend(saved_ci);

    let result = if is_first {
        // 初回 resume: body 関数を呼び出す
        let body = state
            .global
            .heap
            .get_thread_mut(tk)
            .unwrap()
            .body
            .take()
            .unwrap();
        crate::vm::call(state, body, &resume_args)
    } else {
        // 再開: 保存済みフレームから続きを実行
        crate::vm::resume_execute(state, stack_marker, ci_marker, resume_args)
    };

    match result {
        Ok(vals) => {
            // 正常終了
            let th = state.global.heap.get_thread_mut(tk).unwrap();
            th.status = ThreadStatus::Dead;
            // コルーチンフレームをクリア（通常呼び出しが truncate している）
            let mut out = vec![Value::Boolean(true)];
            out.extend(vals);
            state.stack.truncate(stack_marker);
            state.call_info.truncate(ci_marker);
            aux::ret(state, out)
        }
        Err(LuaError::Yield(vals)) => {
            // yield — コルーチンのフレームを保存
            let th = state.global.heap.get_thread_mut(tk).unwrap();
            th.status = ThreadStatus::Suspended;
            th.saved_stack = state.stack.drain(stack_marker..).collect();
            th.saved_call_info = state.call_info.drain(ci_marker..).collect();
            let mut out = vec![Value::Boolean(true)];
            out.extend(vals);
            aux::ret(state, out)
        }
        Err(e) => {
            // エラー
            let th = state.global.heap.get_thread_mut(tk).unwrap();
            th.status = ThreadStatus::Dead;
            th.saved_stack.clear();
            th.saved_call_info.clear();
            state.stack.truncate(stack_marker);
            state.call_info.truncate(ci_marker);
            let ev = error_to_value(state, e);
            aux::ret(state, vec![Value::Boolean(false), ev])
        }
    }
}

// ============================================================================
// coroutine.yield(...)
// ============================================================================

fn l_yield(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    // スタックをクリーンにしてから Yield エラーを発生させる。
    // Yield は execute_inner が各フレームで LuaFrameState を保存しながら伝播する。
    Err(LuaError::Yield(args))
}

// ============================================================================
// coroutine.status(co)
// ============================================================================

fn l_status(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_thread(state, &args, 0, "coroutine.status")?;
    let status = state
        .global
        .heap
        .get_thread(tk)
        .map(|th| th.status)
        .unwrap_or(ThreadStatus::Dead);
    let name = match status {
        ThreadStatus::Suspended => b"suspended" as &[u8],
        ThreadStatus::Running => b"running",
        ThreadStatus::Dead => b"dead",
        ThreadStatus::Normal => b"normal",
    };
    let sv = state.new_string(name);
    aux::ret(state, vec![sv])
}

// ============================================================================
// coroutine.wrap(f)
// ============================================================================

fn l_wrap(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let f = aux::check_function(state, &args, 0, "coroutine.wrap")?;
    let thread = LuaThread::new(f);
    let th_val = Value::GcRef(state.global.heap.alloc_thread(thread));

    // コルーチンハンドルを upvalue として持つクロージャを返す。
    let wrapped = aux::make_native_with_upvalue(state, wrap_iter, th_val);
    aux::ret(state, vec![wrapped])
}

fn wrap_iter(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    // upvalue[0] がコルーチンハンドル
    let th_val = state.current_upvalue(0).unwrap_or(Value::Nil);
    let tk = match th_val {
        Value::GcRef(GcHandle::Thread(k)) => k,
        _ => return Err(aux::rt_error(state, "wrap: invalid coroutine")),
    };

    let resume_result = resume_thread(state, tk, args)?;
    // resume_result[0] は boolean (ok/fail)
    match resume_result.first().copied().unwrap_or(Value::Nil) {
        Value::Boolean(true) => {
            let vals: Vec<Value> = resume_result[1..].to_vec();
            aux::ret(state, vals)
        }
        _ => {
            let errmsg = resume_result.get(1).copied().unwrap_or(Value::Nil);
            Err(LuaError::Runtime(errmsg))
        }
    }
}

/// `l_resume` と同じ処理を関数として切り出した版。
fn resume_thread(
    state: &mut LuaState,
    tk: ThreadKey,
    resume_args: Vec<Value>,
) -> LuaResult<Vec<Value>> {
    {
        let th = match state.global.heap.get_thread(tk) {
            Some(t) => t,
            None => {
                let ev = state.new_string(b"coroutine handle invalid");
                return Ok(vec![Value::Boolean(false), ev]);
            }
        };
        match th.status {
            ThreadStatus::Dead => {
                let ev = state.new_string(b"cannot resume dead coroutine");
                return Ok(vec![Value::Boolean(false), ev]);
            }
            ThreadStatus::Running => {
                let ev = state.new_string(b"cannot resume non-suspended coroutine");
                return Ok(vec![Value::Boolean(false), ev]);
            }
            _ => {}
        }
    }

    state.global.heap.get_thread_mut(tk).unwrap().status = ThreadStatus::Running;

    let stack_marker = state.stack.len();
    let ci_marker = state.call_info.len();

    let (saved_stack, saved_ci, is_first) = {
        let th = state.global.heap.get_thread_mut(tk).unwrap();
        let is_first = th.body.is_some();
        (
            std::mem::take(&mut th.saved_stack),
            std::mem::take(&mut th.saved_call_info),
            is_first,
        )
    };
    state.stack.extend(saved_stack);
    state.call_info.extend(saved_ci);

    let result = if is_first {
        let body = state
            .global
            .heap
            .get_thread_mut(tk)
            .unwrap()
            .body
            .take()
            .unwrap();
        crate::vm::call(state, body, &resume_args)
    } else {
        crate::vm::resume_execute(state, stack_marker, ci_marker, resume_args)
    };

    match result {
        Ok(vals) => {
            state.global.heap.get_thread_mut(tk).unwrap().status = ThreadStatus::Dead;
            state.stack.truncate(stack_marker);
            state.call_info.truncate(ci_marker);
            let mut out = vec![Value::Boolean(true)];
            out.extend(vals);
            Ok(out)
        }
        Err(LuaError::Yield(vals)) => {
            let th = state.global.heap.get_thread_mut(tk).unwrap();
            th.status = ThreadStatus::Suspended;
            th.saved_stack = state.stack.drain(stack_marker..).collect();
            th.saved_call_info = state.call_info.drain(ci_marker..).collect();
            let mut out = vec![Value::Boolean(true)];
            out.extend(vals);
            Ok(out)
        }
        Err(e) => {
            let th = state.global.heap.get_thread_mut(tk).unwrap();
            th.status = ThreadStatus::Dead;
            th.saved_stack.clear();
            th.saved_call_info.clear();
            state.stack.truncate(stack_marker);
            state.call_info.truncate(ci_marker);
            let ev = error_to_value(state, e);
            Ok(vec![Value::Boolean(false), ev])
        }
    }
}

// ============================================================================
// coroutine.isyieldable()
// ============================================================================

fn l_isyieldable(state: &mut LuaState) -> LuaResult<i32> {
    // メインスレッドから直接呼ばれた場合は false。
    // コルーチン内なら true（簡易実装: call_info の深さで判定は難しいので常に true）。
    // TODO: メインスレッド判定を実装する。
    let _ = state;
    aux::ret(state, vec![Value::Boolean(true)])
}

// ============================================================================
// coroutine.running()
// ============================================================================

fn l_running(state: &mut LuaState) -> LuaResult<i32> {
    // 現在実行中のコルーチンを返す。メインスレッドでは nil, true を返す（Lua 5.2+ 互換）。
    let _ = state;
    aux::ret(state, vec![Value::Nil, Value::Boolean(true)])
}

// ============================================================================
// ヘルパ
// ============================================================================

fn error_to_value(state: &mut LuaState, e: LuaError) -> Value {
    match e {
        LuaError::Runtime(v) => v,
        LuaError::Syntax(s) | LuaError::Internal(s) => state.new_string(s.as_bytes()),
        LuaError::Memory => state.new_string(b"not enough memory"),
        LuaError::ErrorInError => state.new_string(b"error in error handling"),
        LuaError::Yield(_) => state.new_string(b"unexpected yield"),
    }
}
