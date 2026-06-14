//! 命令ディスパッチループ（本家 `lvm.c` の `luaV_execute` 相当）。担当: **lua-vm**。
//!
//! [`Proto`] の命令列を [`LuaState`] のスタック（レジスタ領域）上で解釈実行する。
//! 算術・比較・連結のメタメソッド解決、`__index`/`__newindex`、`CALL`/`RETURN` の
//! フレーム操作、数値/汎用 for、upvalue の open/close を実装する。
//!
//! # 呼び出しモデル
//! `CALL` は Lua 関数に対して Rust 再帰（[`call`] → [`execute`]）でフレームを積む。
//! 各 [`execute`] 呼び出しが 1 フレームを管理し、`base` を起点にレジスタ `R(i)` を
//! `state.stack[base + i]` に対応づける。再帰深度は [`MAX_CALL_DEPTH`] で保護する
//! （Lua の "stack overflow" 相当）。エラーは [`LuaResult`] の `Err` で伝播し、
//! 最寄りの [`crate::state::call::pcall`] 境界がスタックを巻き戻す。

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{LuaError, LuaResult};
use crate::gc::GcHandle;
use crate::state::{CallInfo, LuaFrameState, LuaState};
use crate::value::closure::{Closure, LuaClosure, Upvalue, UpvalueState};
use crate::value::convert::{number_to_string, str_to_number};
use crate::value::table::Table;
use crate::value::{LuaType, Value};

use super::opcode::{self, LFIELDS_PER_FLUSH, OpCode};
use super::proto::Proto;

/// Lua 関数/ネイティブ呼び出しの最大ネスト深度。本家 Lua 5.1 の `LUAI_MAXCALLS`
/// （`luaconf.h` の既定値 200）に合わせる。
///
/// rua の VM はネストした呼び出しを **Rust の関数再帰**として実行するため、無限/過大な
/// Lua 再帰を放置すると OS/Rust スタックを溢れさせ、**プロセスがクラッシュ**
/// （abort/segfault = 互換性方針上の最悪結果）してしまう。この上限に達した時点で
/// `call()` 冒頭が捕捉可能な `"stack overflow"` エラーへ変換し、クラッシュを防ぐ。
///
/// 値のトレードオフ:
/// - 小さすぎる → 正当だが深い再帰で誤って `stack overflow`（偽陽性）。
/// - 大きすぎる → 上限到達前に実 Rust スタックが先に溢れ、捕捉不能なクラッシュになる。
///
/// PUC-Rio が検証済みの既定値 **200** を採用する（本家との挙動一致のため。実測でも
/// 深さ 200 の非末尾再帰は `pcall` で `stack overflow` を捕捉でき、クラッシュしないことを
/// 確認済み, 2026-06-14）。値を変更する場合は本コメントと根拠を必ず更新すること。
pub const MAX_CALL_DEPTH: usize = 200;

/// `__index`/`__newindex` チェーンを辿る最大回数（本家 `MAXTAGLOOP`）。
const MAXTAGLOOP: usize = 100;

// ============================================================================
// 公開エントリ
// ============================================================================

/// メインチャンク（プロトタイプ）を実行する。
///
/// `proto` を upvalue 無しのクロージャとして実体化し、`args` を可変長引数として渡す。
/// 返り値はチャンクの戻り値列。
pub fn run(state: &mut LuaState, proto: Rc<Proto>, args: &[Value]) -> LuaResult<Vec<Value>> {
    let env = state.global.globals;
    let closure = LuaClosure::new_with_env(proto, env);
    let h = state.global.heap.alloc_closure(Closure::Lua(closure));
    call(state, Value::GcRef(h), args)
}

/// 中断済みコルーチンフレームを `resume_args` で再開する。
///
/// `state.stack[stack_base..]` および `state.call_info[ci_base..]` には
/// コルーチンの保存済みフレームが積まれていること。各フレームの `CallInfo.lua_frame`
/// が保存された実行状態を持つ（tail call パススルーの場合は `None`）。
pub fn resume_execute(
    state: &mut LuaState,
    _stack_base: usize,
    ci_base: usize,
    resume_args: Vec<Value>,
) -> LuaResult<Vec<Value>> {
    let mut current_vals = resume_args;

    loop {
        if state.call_info.len() <= ci_base {
            return Ok(current_vals);
        }
        let ci_idx = state.call_info.len() - 1;

        // lua_frame が None のフレームはネイティブへの TAILCALL が yield したもの。
        // このフレームには再開点がないのでポップして外側フレームへ値を渡す。
        if state.call_info[ci_idx].lua_frame.is_none() {
            let base = state.call_info[ci_idx].base;
            state.call_info.pop();
            state.stack.truncate(base);
            continue;
        }

        let frame = state.call_info[ci_idx].lua_frame.take().unwrap();
        let LuaFrameState {
            resume_call_pc,
            proto,
            upvals,
            varargs,
            open,
            top,
            env,
        } = *frame;
        let base = state.call_info[ci_idx].base;

        // yield を発生させた CALL 命令を再読みして結果レジスタへ current_vals を配置。
        let call_instr = proto.code[resume_call_pc];
        let call_a = call_instr.a() as usize;
        let want = if call_instr.c() == 0 {
            current_vals.len()
        } else {
            call_instr.c() as usize - 1
        };
        let mut restored_top = top;
        for i in 0..want {
            set_reg(
                state,
                base + call_a + i,
                current_vals.get(i).copied().unwrap_or(Value::Nil),
            );
        }
        if call_instr.c() == 0 {
            restored_top = base + call_a + current_vals.len();
        }

        match execute_inner(
            state,
            base,
            proto,
            upvals,
            varargs,
            open,
            restored_top,
            resume_call_pc + 1,
            env,
        ) {
            Ok(vals) => {
                // このフレームが正常終了。CI とスタックをポップして外側フレームへ。
                state.call_info.pop();
                state.stack.truncate(base);
                current_vals = vals;
            }
            Err(LuaError::Yield(vals)) => return Err(LuaError::Yield(vals)),
            Err(e) => return Err(e),
        }
    }
}

/// 任意の呼び出し可能値 `func` を `args` で呼ぶ（本家 `lua_call`/`luaD_call` 相当）。
///
/// Lua クロージャ・ネイティブ関数・`__call` メタメソッドを持つ値に対応する。
/// 返り値は呼び出しの全戻り値列。
pub fn call(state: &mut LuaState, func: Value, args: &[Value]) -> LuaResult<Vec<Value>> {
    if state.call_info.len() >= MAX_CALL_DEPTH {
        return Err(rt_err(state, "stack overflow".to_string()));
    }
    match func {
        Value::GcRef(GcHandle::Closure(k)) => {
            let is_lua = matches!(state.global.heap.get_closure(k), Some(Closure::Lua(_)));
            if is_lua {
                call_lua(state, k, args)
            } else {
                call_native(state, k, args)
            }
        }
        _ => {
            // __call メタメソッド: handler(func, args...)
            let mm = get_metamethod(state, func, b"__call");
            if matches!(mm, Value::Nil) {
                Err(type_err(state, "call", func))
            } else {
                let mut newargs = Vec::with_capacity(args.len() + 1);
                newargs.push(func);
                newargs.extend_from_slice(args);
                call(state, mm, &newargs)
            }
        }
    }
}

// ============================================================================
// 公開ヘルパ（lua-stdlib 連携）
// ============================================================================

/// 文字列型の共有メタテーブルを設定する（本家 `luaL_*` で `string` ライブラリが行う登録に相当）。
///
/// `mt` は通常 `{ __index = string }` を表すテーブルハンドル。設定後、文字列値への
/// インデックス/メソッド呼び出し（`("s"):upper()` 等）でこのメタテーブルの `__index` が辿られる。
/// **lua-stdlib が `string` ライブラリ初期化時に呼ぶことを想定。**
pub fn set_string_metatable(state: &mut LuaState, mt: GcHandle) {
    state.global.string_metatable = Some(mt);
}

/// 現在の文字列型メタテーブルを取得する（`getmetatable("s")` の実装補助）。
pub fn string_metatable(state: &LuaState) -> Option<GcHandle> {
    state.global.string_metatable
}

/// 本家 `luaL_where(level)` 相当。指定 level のコールフレームの位置を `"chunk:line: "` で返す。
///
/// `level == 0` は現在実行中の関数自身、`level == 1` はその呼び出し元（＝ネイティブ関数から見て
/// 自分を呼んだ Lua 関数）。情報が無ければ空文字列を返す。`error(msg, level)` の位置付与に用いる。
///
/// 例（lua-stdlib の `error` 実装）:
/// ```ignore
/// let prefix = vm::where_string(state, level); // level 既定 1
/// let full = format!("{prefix}{msg}");
/// return Err(LuaError::Runtime(state.new_string(full.as_bytes())));
/// ```
pub fn where_string(state: &LuaState, level: usize) -> String {
    let n = state.call_info.len();
    // 実行中フレーム(level 0)は call_info の末尾。level 段だけ上（呼び出し元方向）を見る。
    let Some(idx) = n.checked_sub(1 + level) else {
        return String::new();
    };
    let ci = &state.call_info[idx];
    match &ci.source {
        Some(src) if ci.current_line > 0 => format!("{src}:{}: ", ci.current_line),
        _ => String::new(),
    }
}

// ============================================================================
// 呼び出し（Lua / ネイティブ）
// ============================================================================

fn call_native(
    state: &mut LuaState,
    key: crate::gc::ClosureKey,
    args: &[Value],
) -> LuaResult<Vec<Value>> {
    let func = match state.global.heap.get_closure(key) {
        Some(Closure::Native(nc)) => nc.func(),
        _ => return Err(rt_err(state, "internal: not a native closure".to_string())),
    };
    let base = state.stack.len();
    for a in args {
        state.stack.push(*a);
    }
    // native_closure にキーを記録する。NativeFn 本体が
    // `state.current_native_closure()` でキーを取得し、upvalue へアクセスできる
    // （本家 lua_upvalueindex 相当, 第二マイルストーン C API 対応）。
    state.call_info.push(CallInfo {
        base,
        func: base,
        expected_results: 0,
        source: None,
        current_line: 0,
        native_closure: Some(key),
        lua_frame: None,
        env: None,
    });
    let r = func(state);
    match r {
        Ok(nres) => {
            let nres = nres.max(0) as usize;
            let total = state.stack.len();
            let start = total.saturating_sub(nres);
            let results = state.stack[start..total].to_vec();
            state.call_info.pop();
            state.stack.truncate(base);
            Ok(results)
        }
        Err(LuaError::Yield(vals)) => {
            // Yield: ネイティブフレームをクリーンアップしてから伝播する。
            // これにより上位の execute_inner の CALL ハンドラが正しい CallInfo を参照できる。
            state.call_info.pop();
            state.stack.truncate(base);
            Err(LuaError::Yield(vals))
        }
        Err(e) => Err(e),
    }
}

fn call_lua(
    state: &mut LuaState,
    key: crate::gc::ClosureKey,
    args: &[Value],
) -> LuaResult<Vec<Value>> {
    let (proto, upvals, env) = match state.global.heap.get_closure(key) {
        Some(Closure::Lua(lc)) => (lc.proto().clone(), lc.upvalues().to_vec(), lc.env()),
        _ => return Err(rt_err(state, "internal: not a Lua closure".to_string())),
    };

    let nparams = proto.num_params as usize;
    let maxstack = proto.max_stack_size as usize;
    let base = state.stack.len();

    // 固定引数を R(0..nparams) に配置。
    for i in 0..nparams {
        state.stack.push(args.get(i).copied().unwrap_or(Value::Nil));
    }
    // 残りのレジスタを nil で確保。
    state.stack.resize(base + maxstack.max(nparams), Value::Nil);

    // 可変長引数（`...`）。
    let varargs: Vec<Value> = if proto.is_vararg && args.len() > nparams {
        args[nparams..].to_vec()
    } else {
        Vec::new()
    };

    state.call_info.push(CallInfo {
        base,
        func: base,
        expected_results: 0,
        source: Some(short_src(proto.source.as_deref())),
        current_line: proto.line_defined,
        native_closure: None,
        lua_frame: None,
        env: Some(env),
    });

    let result = execute(state, base, proto, upvals, varargs, env);

    match result {
        Ok(v) => {
            state.call_info.pop();
            state.stack.truncate(base);
            Ok(v)
        }
        Err(LuaError::Yield(vals)) => {
            // コルーチン yield: CI とスタックフレームを保持したまま伝播する。
            // l_resume がこれらを saved_call_info / saved_stack に移す。
            Err(LuaError::Yield(vals))
        }
        Err(e) => {
            state.call_info.pop();
            state.stack.truncate(base);
            Err(e)
        }
    }
}

// ============================================================================
// 命令ディスパッチループ
// ============================================================================

fn execute(
    state: &mut LuaState,
    base: usize,
    proto: Rc<Proto>,
    upvals: Vec<Upvalue>,
    varargs: Vec<Value>,
    env: GcHandle,
) -> LuaResult<Vec<Value>> {
    let initial_top = base + proto.max_stack_size as usize;
    execute_inner(
        state,
        base,
        proto,
        upvals,
        varargs,
        Vec::new(),
        initial_top,
        0,
        env,
    )
}

/// コルーチン再開用エントリ。保存済みの open upvalue リスト・top・pc から実行を続ける。
#[allow(clippy::too_many_arguments)]
fn execute_inner(
    state: &mut LuaState,
    base: usize,
    mut proto: Rc<Proto>,
    mut upvals: Vec<Upvalue>,
    mut varargs: Vec<Value>,
    saved_open: Vec<(usize, Upvalue)>,
    saved_top: usize,
    saved_pc: usize,
    mut env: GcHandle,
) -> LuaResult<Vec<Value>> {
    // このフレームの CallInfo インデックスを記録する。ネストした呼び出しが
    // Yield したとき、last_mut() ではなくこのインデックスで自フレームを参照する。
    let my_ci_index = state.call_info.len().saturating_sub(1);
    let mut first_entry = true;
    'reenter: loop {
        let (mut open, mut top, mut pc) = if first_entry {
            first_entry = false;
            (saved_open.clone(), saved_top, saved_pc)
        } else {
            (Vec::new(), base + proto.max_stack_size as usize, 0)
        };

        // このフレームの CallInfo に整形済みソース名を反映（TCO 再入時の差し替えにも対応）。
        if let Some(ci) = state.call_info.last_mut() {
            ci.source = Some(short_src(proto.source.as_deref()));
        }

        loop {
            let instr = proto.code[pc];
            let cur_pc = pc;
            pc += 1;

            // `error()` の level 指定（luaL_where 相当）に備え、現在行を CallInfo に記録する。
            if let Some(ci) = state.call_info.last_mut() {
                ci.current_line = proto.line_at(cur_pc);
            }

            let op = match instr.opcode() {
                Some(op) => op,
                None => return Err(err_at(state, &proto, cur_pc, "bad opcode".to_string())),
            };
            let a = instr.a() as usize;

            match op {
                OpCode::Move => {
                    let v = reg(state, base, instr.b() as usize);
                    set_reg(state, base + a, v);
                }
                OpCode::LoadK => {
                    let v = proto.constants[instr.bx() as usize];
                    set_reg(state, base + a, v);
                }
                OpCode::LoadBool => {
                    set_reg(state, base + a, Value::Boolean(instr.b() != 0));
                    if instr.c() != 0 {
                        pc += 1;
                    }
                }
                OpCode::LoadNil => {
                    let b = instr.b() as usize;
                    for r in a..=b {
                        set_reg(state, base + r, Value::Nil);
                    }
                }
                OpCode::GetUpval => {
                    let v = upval_get(state, &upvals[instr.b() as usize]);
                    set_reg(state, base + a, v);
                }
                OpCode::SetUpval => {
                    let v = reg(state, base, a);
                    upval_set(state, &upvals[instr.b() as usize], v);
                }
                OpCode::GetGlobal => {
                    let key = proto.constants[instr.bx() as usize];
                    // setfenv による env 変更を反映するため CallInfo から都度読む。
                    let cur_env = state.call_info.get(my_ci_index)
                        .and_then(|ci| ci.env)
                        .unwrap_or(env);
                    let g = Value::GcRef(cur_env);
                    let v = index_get(state, g, key, &proto, cur_pc)?;
                    set_reg(state, base + a, v);
                }
                OpCode::SetGlobal => {
                    let key = proto.constants[instr.bx() as usize];
                    let v = reg(state, base, a);
                    // setfenv による env 変更を反映するため CallInfo から都度読む。
                    let cur_env = state.call_info.get(my_ci_index)
                        .and_then(|ci| ci.env)
                        .unwrap_or(env);
                    let g = Value::GcRef(cur_env);
                    index_set(state, g, key, v, &proto, cur_pc)?;
                }
                OpCode::GetTable => {
                    let t = reg(state, base, instr.b() as usize);
                    let k = rk(state, &proto, base, instr.c());
                    let v = index_get(state, t, k, &proto, cur_pc)?;
                    set_reg(state, base + a, v);
                }
                OpCode::SetTable => {
                    let t = reg(state, base, a);
                    let k = rk(state, &proto, base, instr.b());
                    let v = rk(state, &proto, base, instr.c());
                    index_set(state, t, k, v, &proto, cur_pc)?;
                }
                OpCode::NewTable => {
                    let narray = fb2int(instr.b());
                    let nhash = fb2int(instr.c());
                    let h = state
                        .global
                        .heap
                        .alloc_table(Table::with_capacity(narray, nhash));
                    set_reg(state, base + a, Value::GcRef(h));
                }
                OpCode::SelfOp => {
                    let t = reg(state, base, instr.b() as usize);
                    set_reg(state, base + a + 1, t);
                    let k = rk(state, &proto, base, instr.c());
                    let v = index_get(state, t, k, &proto, cur_pc)?;
                    set_reg(state, base + a, v);
                }
                OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Div
                | OpCode::Mod
                | OpCode::Pow => {
                    let b = rk(state, &proto, base, instr.b());
                    let c = rk(state, &proto, base, instr.c());
                    let v = arith(state, op, b, c, &proto, cur_pc)?;
                    set_reg(state, base + a, v);
                }
                OpCode::Unm => {
                    let b = reg(state, base, instr.b() as usize);
                    let v = match tonum(state, b) {
                        Some(n) => Value::Number(-n),
                        None => {
                            let mm = get_metamethod(state, b, b"__unm");
                            if matches!(mm, Value::Nil) {
                                return Err(arith_err(state, &proto, cur_pc, b));
                            }
                            first(call(state, mm, &[b, b])?)
                        }
                    };
                    set_reg(state, base + a, v);
                }
                OpCode::Not => {
                    let b = reg(state, base, instr.b() as usize);
                    set_reg(state, base + a, Value::Boolean(!b.is_truthy()));
                }
                OpCode::Len => {
                    let b = reg(state, base, instr.b() as usize);
                    let v = len_op(state, b, &proto, cur_pc)?;
                    set_reg(state, base + a, v);
                }
                OpCode::Concat => {
                    let bb = instr.b() as usize;
                    let cc = instr.c() as usize;
                    // 右結合で R(B)..R(B+1)..R(C) を連結。
                    let mut acc = reg(state, base, cc);
                    let mut i = cc;
                    while i > bb {
                        i -= 1;
                        let left = reg(state, base, i);
                        acc = concat_two(state, left, acc, &proto, cur_pc)?;
                    }
                    set_reg(state, base + a, acc);
                }
                OpCode::Jmp => {
                    pc = (pc as i32 + instr.sbx()) as usize;
                }
                OpCode::Eq => {
                    let b = rk(state, &proto, base, instr.b());
                    let c = rk(state, &proto, base, instr.c());
                    let eq = values_equal(state, b, c)?;
                    if eq != (a != 0) {
                        pc += 1;
                    }
                }
                OpCode::Lt => {
                    let b = rk(state, &proto, base, instr.b());
                    let c = rk(state, &proto, base, instr.c());
                    let lt = less_than(state, b, c, &proto, cur_pc)?;
                    if lt != (a != 0) {
                        pc += 1;
                    }
                }
                OpCode::Le => {
                    let b = rk(state, &proto, base, instr.b());
                    let c = rk(state, &proto, base, instr.c());
                    let le = less_equal(state, b, c, &proto, cur_pc)?;
                    if le != (a != 0) {
                        pc += 1;
                    }
                }
                OpCode::Test => {
                    let ra = reg(state, base, a);
                    if ra.is_truthy() != (instr.c() != 0) {
                        pc += 1;
                    }
                }
                OpCode::TestSet => {
                    let rb = reg(state, base, instr.b() as usize);
                    if rb.is_truthy() == (instr.c() != 0) {
                        set_reg(state, base + a, rb);
                    } else {
                        pc += 1;
                    }
                }
                OpCode::Call => {
                    let nargs = if instr.b() == 0 {
                        top - (base + a + 1)
                    } else {
                        instr.b() as usize - 1
                    };
                    let func = reg(state, base, a);
                    let mut callargs = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        callargs.push(reg(state, base, a + 1 + i));
                    }
                    let r = call(state, func, &callargs);
                    let results = match r {
                        Ok(v) => v,
                        Err(LuaError::Yield(vals)) => {
                            // コルーチン yield: 自フレームの状態を CallInfo に保存して上位へ伝播。
                            // my_ci_index を使うことでネストした Lua 呼び出しが CI を保持したまま
                            // 伝播してきた場合でも正しい自フレームに保存できる。
                            if let Some(ci) = state.call_info.get_mut(my_ci_index) {
                                ci.lua_frame = Some(Box::new(LuaFrameState {
                                    resume_call_pc: cur_pc,
                                    proto: proto.clone(),
                                    upvals: upvals.clone(),
                                    varargs: varargs.clone(),
                                    open: open.clone(),
                                    top,
                                    env,
                                }));
                            }
                            return Err(LuaError::Yield(vals));
                        }
                        Err(e) => return Err(e),
                    };
                    let want = if instr.c() == 0 {
                        results.len()
                    } else {
                        instr.c() as usize - 1
                    };
                    for i in 0..want {
                        set_reg(
                            state,
                            base + a + i,
                            results.get(i).copied().unwrap_or(Value::Nil),
                        );
                    }
                    if instr.c() == 0 {
                        top = base + a + results.len();
                    } else {
                        top = base + proto.max_stack_size as usize;
                    }
                }
                OpCode::TailCall => {
                    let nargs = if instr.b() == 0 {
                        top - (base + a + 1)
                    } else {
                        instr.b() as usize - 1
                    };
                    let func = reg(state, base, a);
                    let mut callargs = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        callargs.push(reg(state, base, a + 1 + i));
                    }
                    // 現フレームの open upvalue を閉じてからフレームを明け渡す。
                    close_upvals(state, &mut open, base);

                    // 呼び先が Lua クロージャなら **フレームを再利用** して 'reenter（真の TCO）。
                    if let Value::GcRef(GcHandle::Closure(k)) = func
                        && let Some(Closure::Lua(lc)) = state.global.heap.get_closure(k)
                    {
                        let new_proto = lc.proto().clone();
                        let new_upvals = lc.upvalues().to_vec();
                        let new_env = lc.env();
                        let nparams = new_proto.num_params as usize;
                        let maxstack = new_proto.max_stack_size as usize;
                        let need = base + maxstack.max(nparams);

                        // フレーム領域を新関数のレジスタ数に合わせて調整。
                        if state.stack.len() < need {
                            state.stack.resize(need, Value::Nil);
                        } else {
                            state.stack.truncate(need);
                        }
                        // 固定引数を配置し、余りのレジスタを nil で初期化。
                        for i in 0..nparams {
                            state.stack[base + i] = callargs.get(i).copied().unwrap_or(Value::Nil);
                        }
                        for i in nparams..(need - base) {
                            state.stack[base + i] = Value::Nil;
                        }
                        varargs = if new_proto.is_vararg && callargs.len() > nparams {
                            callargs[nparams..].to_vec()
                        } else {
                            Vec::new()
                        };
                        proto = new_proto;
                        upvals = new_upvals;
                        env = new_env;
                        continue 'reenter;
                    }

                    // ネイティブ関数 / `__call`: 通常呼び出しで結果をそのまま返す。
                    let results = call(state, func, &callargs)?;
                    return Ok(results);
                }
                OpCode::Return => {
                    let n = if instr.b() == 0 {
                        top - (base + a)
                    } else {
                        instr.b() as usize - 1
                    };
                    let mut rets = Vec::with_capacity(n);
                    for i in 0..n {
                        rets.push(reg(state, base, a + i));
                    }
                    close_upvals(state, &mut open, base);
                    return Ok(rets);
                }
                OpCode::ForPrep => {
                    let init = num_for(state, base, a, "initial", &proto, cur_pc)?;
                    let _limit = num_for(state, base, a + 1, "limit", &proto, cur_pc)?;
                    let step = num_for(state, base, a + 2, "step", &proto, cur_pc)?;
                    set_reg(state, base + a, Value::Number(init - step));
                    pc = (pc as i32 + instr.sbx()) as usize;
                }
                OpCode::ForLoop => {
                    let idx = num_at(state, base, a) + num_at(state, base, a + 2);
                    let limit = num_at(state, base, a + 1);
                    let step = num_at(state, base, a + 2);
                    let cont = if step >= 0.0 {
                        idx <= limit
                    } else {
                        idx >= limit
                    };
                    if cont {
                        set_reg(state, base + a, Value::Number(idx));
                        set_reg(state, base + a + 3, Value::Number(idx));
                        pc = (pc as i32 + instr.sbx()) as usize;
                    }
                }
                OpCode::TForLoop => {
                    let func = reg(state, base, a);
                    let s = reg(state, base, a + 1);
                    let ctrl = reg(state, base, a + 2);
                    let nresults = instr.c() as usize;
                    let results = call(state, func, &[s, ctrl])?;
                    for i in 0..nresults {
                        set_reg(
                            state,
                            base + a + 3 + i,
                            results.get(i).copied().unwrap_or(Value::Nil),
                        );
                    }
                    let first_res = reg(state, base, a + 3);
                    if !matches!(first_res, Value::Nil) {
                        set_reg(state, base + a + 2, first_res);
                    } else {
                        pc += 1;
                    }
                }
                OpCode::SetList => {
                    let n = if instr.b() == 0 {
                        top - (base + a + 1)
                    } else {
                        instr.b() as usize
                    };
                    let mut block = instr.c() as usize;
                    if block == 0 {
                        // 大きな C は次の命令ワードに格納される。
                        block = proto.code[pc].raw() as usize;
                        pc += 1;
                    }
                    let tval = reg(state, base, a);
                    let tk = match tval {
                        Value::GcRef(GcHandle::Table(k)) => k,
                        _ => {
                            return Err(err_at(
                                state,
                                &proto,
                                cur_pc,
                                "SETLIST on non-table".to_string(),
                            ));
                        }
                    };
                    for i in 1..=n {
                        let idx = (block - 1) * LFIELDS_PER_FLUSH as usize + i;
                        let v = reg(state, base, a + i);
                        if let Some(t) = state.global.heap.get_table_mut(tk) {
                            let _ = t.set(Value::Number(idx as f64), v);
                        }
                    }
                    top = base + proto.max_stack_size as usize;
                }
                OpCode::Close => {
                    close_upvals(state, &mut open, base + a);
                }
                OpCode::Closure => {
                    let child = proto.protos[instr.bx() as usize].clone();
                    let nup = child.num_upvalues as usize;
                    // 子クロージャは親の env を継承する（Lua 5.1 の規則）。
                    // setfenv 後の env を反映するため CallInfo から読む。
                    let child_env = state.call_info.get(my_ci_index)
                        .and_then(|ci| ci.env)
                        .unwrap_or(env);
                    let mut newc = LuaClosure::new_with_env(child, child_env);
                    for _ in 0..nup {
                        let pseudo = proto.code[pc];
                        pc += 1;
                        match pseudo.opcode() {
                            Some(OpCode::Move) => {
                                let abs = base + pseudo.b() as usize;
                                let uv = find_or_create_upval(&mut open, abs);
                                newc.push_upvalue(uv);
                            }
                            Some(OpCode::GetUpval) => {
                                let uv = upvals[pseudo.b() as usize].clone();
                                newc.push_upvalue(uv);
                            }
                            _ => {
                                return Err(err_at(
                                    state,
                                    &proto,
                                    cur_pc,
                                    "malformed CLOSURE upvalue capture".to_string(),
                                ));
                            }
                        }
                    }
                    let h = state.global.heap.alloc_closure(Closure::Lua(newc));
                    set_reg(state, base + a, Value::GcRef(h));
                }
                OpCode::Vararg => {
                    let want = if instr.b() == 0 {
                        varargs.len()
                    } else {
                        instr.b() as usize - 1
                    };
                    for i in 0..want {
                        set_reg(
                            state,
                            base + a + i,
                            varargs.get(i).copied().unwrap_or(Value::Nil),
                        );
                    }
                    if instr.b() == 0 {
                        top = base + a + varargs.len();
                    }
                }
            }
        }
    }
}

// ============================================================================
// レジスタ / upvalue アクセス
// ============================================================================

#[inline]
fn reg(state: &LuaState, base: usize, i: usize) -> Value {
    state.stack.get(base + i).copied().unwrap_or(Value::Nil)
}

#[inline]
fn set_reg(state: &mut LuaState, idx: usize, v: Value) {
    if idx >= state.stack.len() {
        state.stack.resize(idx + 1, Value::Nil);
    }
    state.stack[idx] = v;
}

#[inline]
fn rk(state: &LuaState, proto: &Proto, base: usize, x: u32) -> Value {
    if opcode::is_k(x) {
        proto.constants[opcode::index_k(x) as usize]
    } else {
        reg(state, base, x as usize)
    }
}

#[inline]
fn num_at(state: &LuaState, base: usize, i: usize) -> f64 {
    match reg(state, base, i) {
        Value::Number(n) => n,
        _ => f64::NAN,
    }
}

fn upval_get(state: &LuaState, uv: &Upvalue) -> Value {
    match &*uv.borrow() {
        UpvalueState::Open(idx) => state.stack.get(*idx).copied().unwrap_or(Value::Nil),
        UpvalueState::Closed(v) => *v,
    }
}

fn upval_set(state: &mut LuaState, uv: &Upvalue, v: Value) {
    let open_idx = match &*uv.borrow() {
        UpvalueState::Open(idx) => Some(*idx),
        UpvalueState::Closed(_) => None,
    };
    match open_idx {
        Some(idx) => set_reg(state, idx, v),
        None => *uv.borrow_mut() = UpvalueState::Closed(v),
    }
}

fn find_or_create_upval(open: &mut Vec<(usize, Upvalue)>, abs: usize) -> Upvalue {
    for (idx, uv) in open.iter() {
        if *idx == abs {
            return uv.clone();
        }
    }
    let uv = Rc::new(RefCell::new(UpvalueState::Open(abs)));
    open.push((abs, uv.clone()));
    uv
}

/// `from_abs` 以上の絶対インデックスを指す open upvalue を閉じる。
fn close_upvals(state: &LuaState, open: &mut Vec<(usize, Upvalue)>, from_abs: usize) {
    open.retain(|(idx, uv)| {
        if *idx >= from_abs {
            let v = state.stack.get(*idx).copied().unwrap_or(Value::Nil);
            *uv.borrow_mut() = UpvalueState::Closed(v);
            false
        } else {
            true
        }
    });
}

// ============================================================================
// 算術・比較・連結・長さ
// ============================================================================

/// 値を数値へ（number はそのまま、数値に見える文字列は変換）。
fn tonum(state: &LuaState, v: Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(n),
        Value::GcRef(GcHandle::Str(k)) => state
            .global
            .heap
            .get_str(k)
            .and_then(|s| str_to_number(s.as_bytes())),
        _ => None,
    }
}

fn do_arith(op: OpCode, x: f64, y: f64) -> f64 {
    match op {
        OpCode::Add => x + y,
        OpCode::Sub => x - y,
        OpCode::Mul => x * y,
        OpCode::Div => x / y,
        OpCode::Mod => x - (x / y).floor() * y,
        OpCode::Pow => x.powf(y),
        _ => unreachable!("do_arith: not an arithmetic opcode"),
    }
}

fn arith_event(op: OpCode) -> &'static [u8] {
    match op {
        OpCode::Add => b"__add",
        OpCode::Sub => b"__sub",
        OpCode::Mul => b"__mul",
        OpCode::Div => b"__div",
        OpCode::Mod => b"__mod",
        OpCode::Pow => b"__pow",
        _ => unreachable!(),
    }
}

fn arith(
    state: &mut LuaState,
    op: OpCode,
    b: Value,
    c: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<Value> {
    if let (Some(x), Some(y)) = (tonum(state, b), tonum(state, c)) {
        return Ok(Value::Number(do_arith(op, x, y)));
    }
    let event = arith_event(op);
    let mut mm = get_metamethod(state, b, event);
    if matches!(mm, Value::Nil) {
        mm = get_metamethod(state, c, event);
    }
    if matches!(mm, Value::Nil) {
        // 数値化できなかった側を報告する。
        let culprit = if tonum(state, b).is_none() { b } else { c };
        return Err(arith_err(state, proto, pc, culprit));
    }
    Ok(first(call(state, mm, &[b, c])?))
}

/// `..` の 2 値連結（string/number 同士は直結、それ以外は `__concat`）。
fn concat_two(
    state: &mut LuaState,
    a: Value,
    b: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<Value> {
    if let (Some(mut x), Some(y)) = (stringable(state, a), stringable(state, b)) {
        x.extend_from_slice(&y);
        return Ok(Value::GcRef(state.global.heap.intern_str(&x)));
    }
    let mut mm = get_metamethod(state, a, b"__concat");
    if matches!(mm, Value::Nil) {
        mm = get_metamethod(state, b, b"__concat");
    }
    if matches!(mm, Value::Nil) {
        let culprit = if stringable(state, a).is_none() { a } else { b };
        return Err(concat_err(state, proto, pc, culprit));
    }
    Ok(first(call(state, mm, &[a, b])?))
}

/// 連結に使えるなら byte 列を返す（number は `%.14g` 文字列化）。
fn stringable(state: &LuaState, v: Value) -> Option<Vec<u8>> {
    match v {
        Value::Number(n) => Some(number_to_string(n).into_bytes()),
        Value::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    }
}

fn len_op(state: &mut LuaState, v: Value, proto: &Proto, pc: usize) -> LuaResult<Value> {
    match v {
        Value::GcRef(GcHandle::Str(k)) => {
            let len = state.global.heap.get_str(k).map(|s| s.len()).unwrap_or(0);
            Ok(Value::Number(len as f64))
        }
        Value::GcRef(GcHandle::Table(k)) => {
            // Lua 5.1: テーブルの `#` はメタメソッドを参照せず border を返す。
            let len = state
                .global
                .heap
                .get_table(k)
                .map(|t| t.length())
                .unwrap_or(0);
            Ok(Value::Number(len as f64))
        }
        _ => {
            let mm = get_metamethod(state, v, b"__len");
            if matches!(mm, Value::Nil) {
                Err(err_at(
                    state,
                    proto,
                    pc,
                    format!("attempt to get length of a {} value", v.type_of().name()),
                ))
            } else {
                Ok(first(call(state, mm, &[v])?))
            }
        }
    }
}

/// `==`（`__eq` 込み）。
///
/// Lua 5.1 の規則: `__eq` が呼ばれるのは両オブジェクトが同じメタテーブル上の `__eq` ハンドラを
/// 共有している場合のみ（本家 lvm.c `equalobj` 参照）。
/// 片方のみにメタテーブルがある場合は `__eq` を呼ばない（raw 比較で false）。
fn values_equal(state: &mut LuaState, a: Value, b: Value) -> LuaResult<bool> {
    if a == b {
        return Ok(true);
    }
    // 型が違えば false（number と string も等しくない）。
    let (ta, tb) = (a.type_of(), b.type_of());
    if ta != tb {
        return Ok(false);
    }
    // table/userdata のみ __eq を参照。
    let eligible = matches!(ta, LuaType::Table | LuaType::Userdata);
    if !eligible {
        return Ok(false);
    }
    let mm_a = get_metamethod(state, a, b"__eq");
    let mm_b = get_metamethod(state, b, b"__eq");
    // Lua 5.1: 両者の __eq が同じ関数であるときのみ呼ぶ。
    // 片方のみが __eq を持つ場合は false（raw 比較）。
    let mm = if !matches!(mm_a, Value::Nil) && mm_a == mm_b {
        mm_a
    } else if !matches!(mm_a, Value::Nil) && matches!(mm_b, Value::Nil) {
        // 片方のみ: Lua 5.1 は __eq を呼ばない。
        return Ok(false);
    } else if matches!(mm_a, Value::Nil) && !matches!(mm_b, Value::Nil) {
        // 片方のみ: Lua 5.1 は __eq を呼ばない。
        return Ok(false);
    } else {
        return Ok(false);
    };
    Ok(first(call(state, mm, &[a, b])?).is_truthy())
}

/// 比較メタメソッドを取得する。Lua 5.1 の規則:
/// - 両オペランドが同じメタテーブルを持つか同じハンドラ関数を持つ場合にのみ使用する。
/// - 片方のみがメタメソッドを持つ場合は使用しない（None を返す）。
fn get_cmp_metamethod(state: &mut LuaState, a: Value, b: Value, event: &[u8]) -> Value {
    let mm_a = get_metamethod(state, a, event);
    let mm_b = get_metamethod(state, b, event);
    match (matches!(mm_a, Value::Nil), matches!(mm_b, Value::Nil)) {
        (false, false) => {
            // 両方にある場合: 同じハンドラなら使う、違えば a 側を使う（本家準拠）。
            mm_a
        }
        (false, true) => {
            // a のみ: 片方だけなのでメタメソッドを使わない。
            Value::Nil
        }
        (true, false) => {
            // b のみ: 片方だけなのでメタメソッドを使わない。
            Value::Nil
        }
        (true, true) => Value::Nil,
    }
}

fn less_than(
    state: &mut LuaState,
    a: Value,
    b: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<bool> {
    if let (Value::Number(x), Value::Number(y)) = (a, b) {
        return Ok(x < y);
    }
    if let (Value::GcRef(GcHandle::Str(ka)), Value::GcRef(GcHandle::Str(kb))) = (a, b) {
        let sa = state.global.heap.get_str(ka).unwrap().as_bytes();
        let sb = state.global.heap.get_str(kb).unwrap().as_bytes();
        return Ok(sa < sb);
    }
    let mm = get_cmp_metamethod(state, a, b, b"__lt");
    if matches!(mm, Value::Nil) {
        return Err(cmp_err(state, proto, pc, a, b));
    }
    Ok(first(call(state, mm, &[a, b])?).is_truthy())
}

fn less_equal(
    state: &mut LuaState,
    a: Value,
    b: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<bool> {
    if let (Value::Number(x), Value::Number(y)) = (a, b) {
        return Ok(x <= y);
    }
    if let (Value::GcRef(GcHandle::Str(ka)), Value::GcRef(GcHandle::Str(kb))) = (a, b) {
        let sa = state.global.heap.get_str(ka).unwrap().as_bytes();
        let sb = state.global.heap.get_str(kb).unwrap().as_bytes();
        return Ok(sa <= sb);
    }
    let mm = get_cmp_metamethod(state, a, b, b"__le");
    if !matches!(mm, Value::Nil) {
        return Ok(first(call(state, mm, &[a, b])?).is_truthy());
    }
    // 本家 5.1: __le が無ければ `not (b < a)` を __lt で試す。
    let lt = get_cmp_metamethod(state, a, b, b"__lt");
    if matches!(lt, Value::Nil) {
        return Err(cmp_err(state, proto, pc, a, b));
    }
    Ok(!first(call(state, lt, &[b, a])?).is_truthy())
}

// ============================================================================
// テーブルインデックス（メタメソッド込み）
// ============================================================================

fn index_get(
    state: &mut LuaState,
    mut t: Value,
    key: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<Value> {
    for _ in 0..MAXTAGLOOP {
        if let Value::GcRef(GcHandle::Table(k)) = t {
            let raw = state
                .global
                .heap
                .get_table(k)
                .map(|tb| tb.get(&key))
                .unwrap_or(Value::Nil);
            if !matches!(raw, Value::Nil) {
                return Ok(raw);
            }
            let mm = get_metamethod(state, t, b"__index");
            if matches!(mm, Value::Nil) {
                return Ok(Value::Nil);
            }
            if mm.type_of() == LuaType::Function {
                return Ok(first(call(state, mm, &[t, key])?));
            }
            t = mm; // テーブル等: チェーンを継続
        } else {
            let mm = get_metamethod(state, t, b"__index");
            if matches!(mm, Value::Nil) {
                return Err(index_err(state, proto, pc, t));
            }
            if mm.type_of() == LuaType::Function {
                return Ok(first(call(state, mm, &[t, key])?));
            }
            t = mm;
        }
    }
    Err(err_at(
        state,
        proto,
        pc,
        "'__index' chain too long; possible loop".to_string(),
    ))
}

fn index_set(
    state: &mut LuaState,
    mut t: Value,
    key: Value,
    val: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<()> {
    for _ in 0..MAXTAGLOOP {
        if let Value::GcRef(GcHandle::Table(k)) = t {
            let exists = state
                .global
                .heap
                .get_table(k)
                .map(|tb| !matches!(tb.get(&key), Value::Nil))
                .unwrap_or(false);
            if exists {
                return raw_set(state, k, key, val, proto, pc);
            }
            let mm = get_metamethod(state, t, b"__newindex");
            if matches!(mm, Value::Nil) {
                return raw_set(state, k, key, val, proto, pc);
            }
            if mm.type_of() == LuaType::Function {
                call(state, mm, &[t, key, val])?;
                return Ok(());
            }
            t = mm;
        } else {
            let mm = get_metamethod(state, t, b"__newindex");
            if matches!(mm, Value::Nil) {
                return Err(index_err(state, proto, pc, t));
            }
            if mm.type_of() == LuaType::Function {
                call(state, mm, &[t, key, val])?;
                return Ok(());
            }
            t = mm;
        }
    }
    Err(err_at(
        state,
        proto,
        pc,
        "'__newindex' chain too long; possible loop".to_string(),
    ))
}

fn raw_set(
    state: &mut LuaState,
    k: crate::gc::TableKey,
    key: Value,
    val: Value,
    proto: &Proto,
    pc: usize,
) -> LuaResult<()> {
    let res = state
        .global
        .heap
        .get_table_mut(k)
        .map(|tb| tb.set(key, val))
        .unwrap_or(Ok(()));
    match res {
        Ok(()) => Ok(()),
        Err(crate::value::table::TableKeyError::NilKey) => {
            Err(err_at(state, proto, pc, "table index is nil".to_string()))
        }
        Err(crate::value::table::TableKeyError::NanKey) => {
            Err(err_at(state, proto, pc, "table index is NaN".to_string()))
        }
    }
}

// ============================================================================
// メタテーブル
// ============================================================================

fn metatable_of(state: &LuaState, v: Value) -> Option<crate::gc::TableKey> {
    let mt = match v {
        Value::GcRef(GcHandle::Table(k)) => {
            state.global.heap.get_table(k).and_then(|t| t.metatable())
        }
        Value::GcRef(GcHandle::Userdata(k)) => state
            .global
            .heap
            .get_userdata(k)
            .and_then(|u| u.metatable()),
        // 文字列は型ごとの共有メタテーブル（`global_State.string_metatable`）を参照する。
        // 本体の登録は lua-stdlib が string ライブラリ初期化時に行う。
        Value::GcRef(GcHandle::Str(_)) => state.global.string_metatable,
        // 数値・boolean・nil の型共有メタテーブル（debug.setmetatable で設定）。
        Value::Number(_) => state.global.number_metatable,
        Value::Boolean(_) => state.global.boolean_metatable,
        Value::Nil => state.global.nil_metatable,
        _ => None,
    };
    match mt {
        Some(GcHandle::Table(k)) => Some(k),
        _ => None,
    }
}

/// `v` のメタテーブルからイベント名のハンドラを取得（無ければ `Nil`）。
fn get_metamethod(state: &mut LuaState, v: Value, event: &[u8]) -> Value {
    let Some(mtk) = metatable_of(state, v) else {
        return Value::Nil;
    };
    let key = Value::GcRef(state.global.heap.intern_str(event));
    state
        .global
        .heap
        .get_table(mtk)
        .map(|t| t.get(&key))
        .unwrap_or(Value::Nil)
}

// ============================================================================
// 数値 for のオペランド検証
// ============================================================================

fn num_for(
    state: &mut LuaState,
    base: usize,
    i: usize,
    what: &str,
    proto: &Proto,
    pc: usize,
) -> LuaResult<f64> {
    let v = reg(state, base, i);
    match tonum(state, v) {
        Some(n) => {
            set_reg(state, base + i, Value::Number(n));
            Ok(n)
        }
        None => Err(err_at(
            state,
            proto,
            pc,
            format!("'for' {what} value must be a number"),
        )),
    }
}

// ============================================================================
// ヘルパ / エラー構築
// ============================================================================

#[inline]
fn first(mut results: Vec<Value>) -> Value {
    if results.is_empty() {
        Value::Nil
    } else {
        results.swap_remove(0)
    }
}

fn fb2int(x: u32) -> usize {
    // 本家 "floating point byte": (eeeeexxx) -> (1xxx) * 2^(eeeee-1)。
    let e = (x >> 3) & 0x1f;
    if e == 0 {
        x as usize
    } else {
        (((x & 7) | 8) as usize) << (e - 1)
    }
}

/// Lua 文字列値としての実行時エラーを作る（本家のエラーオブジェクトに一致）。
fn rt_err(state: &mut LuaState, msg: String) -> LuaError {
    let v = state.new_string(msg.as_bytes());
    LuaError::Runtime(v)
}

/// "chunk:line: msg" を付したエラーを作る。チャンク名は本家 `luaO_chunkid` 相当に整形する。
fn err_at(state: &mut LuaState, proto: &Proto, pc: usize, msg: String) -> LuaError {
    let line = proto.line_at(pc);
    let src = short_src(proto.source.as_deref());
    let full = if line > 0 {
        format!("{src}:{line}: {msg}")
    } else {
        format!("{src}: {msg}")
    };
    rt_err(state, full)
}

/// 本家 `luaO_chunkid` 相当のソース名整形。
///
/// - `@file`   → `file`（ファイル由来チャンク）
/// - `=name`   → `name`（特殊な名前。コマンドライン等）
/// - その他    → `[string "先頭行..."]`（`load`/`loadstring` の文字列チャンク）
fn short_src(source: Option<&str>) -> String {
    let Some(src) = source else {
        return "?".to_string();
    };
    match src.as_bytes().first() {
        Some(b'@') | Some(b'=') => src[1..].to_string(),
        _ => {
            // 改行までの先頭行を取り、長ければ省略記号を付す（本家 LUA_IDSIZE 近似）。
            let first_line = src.split(['\n', '\r']).next().unwrap_or(src);
            const MAX: usize = 60;
            if src.contains(['\n', '\r']) || first_line.len() > MAX {
                let truncated: String = first_line.chars().take(MAX).collect();
                format!("[string \"{truncated}...\"]")
            } else {
                format!("[string \"{first_line}\"]")
            }
        }
    }
}

fn type_err(state: &mut LuaState, action: &str, v: Value) -> LuaError {
    rt_err(
        state,
        format!("attempt to {action} a {} value", v.type_of().name()),
    )
}

fn arith_err(state: &mut LuaState, proto: &Proto, pc: usize, v: Value) -> LuaError {
    err_at(
        state,
        proto,
        pc,
        format!(
            "attempt to perform arithmetic on a {} value",
            v.type_of().name()
        ),
    )
}

fn concat_err(state: &mut LuaState, proto: &Proto, pc: usize, v: Value) -> LuaError {
    err_at(
        state,
        proto,
        pc,
        format!("attempt to concatenate a {} value", v.type_of().name()),
    )
}

fn index_err(state: &mut LuaState, proto: &Proto, pc: usize, v: Value) -> LuaError {
    err_at(
        state,
        proto,
        pc,
        format!("attempt to index a {} value", v.type_of().name()),
    )
}

fn cmp_err(state: &mut LuaState, proto: &Proto, pc: usize, a: Value, b: Value) -> LuaError {
    let (ta, tb) = (a.type_of().name(), b.type_of().name());
    let msg = if ta == tb {
        format!("attempt to compare two {ta} values")
    } else {
        format!("attempt to compare {ta} with {tb}")
    };
    err_at(state, proto, pc, msg)
}
