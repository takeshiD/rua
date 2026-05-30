//! VM コア（`vm::interp`）の動作テスト。
//!
//! codegen（lua-frontend）がまだ揃わないため、**手書きの Proto/バイトコード** で
//! 命令ディスパッチ・テーブル・算術・クロージャ/upvalue・for ループ・メタメソッドを検証する。
//! frontend が生成すべき Proto 形式の参照例も兼ねる。

use std::rc::Rc;

use rua_core::gc::GcHandle;
use rua_core::state::LuaState;
use rua_core::value::closure::{Closure, NativeClosure};
use rua_core::value::table::Table;
use rua_core::value::Value;
use rua_core::vm::opcode::{rk_as_k, Instruction, OpCode};
use rua_core::vm::proto::Proto;
use rua_core::vm::{call, run};

/// Proto を組み立てる小さなビルダ。
fn proto(code: Vec<Instruction>, consts: Vec<Value>, max_stack: u8) -> Rc<Proto> {
    Rc::new(Proto {
        code,
        constants: consts,
        max_stack_size: max_stack,
        source: Some("test".to_string()),
        ..Proto::default()
    })
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        other => panic!("expected number, got {other:?}"),
    }
}

#[test]
fn arithmetic_and_return() {
    // local a = 10 + 20 * 2; return a   (定数で表現)
    // R0 = K0(10) + (K1(20) * K2(2))  を 2 命令で。
    let code = vec![
        Instruction::abc(OpCode::Mul, 1, rk_as_k(1), rk_as_k(2)), // R1 = 20*2 = 40
        Instruction::abc(OpCode::Add, 0, rk_as_k(0), 1),          // R0 = 10 + R1 = 50
        Instruction::abc(OpCode::Return, 0, 2, 0),                // return R0
    ];
    let consts = vec![Value::Number(10.0), Value::Number(20.0), Value::Number(2.0)];
    let p = proto(code, consts, 2);

    let mut state = LuaState::new();
    let res = run(&mut state, p, &[]).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(num(&res[0]), 50.0);
}

#[test]
fn table_set_get_and_length() {
    // local t = {}; t[1]=10; t[2]=20; t[3]=30; return #t, t[2]
    let code = vec![
        Instruction::abc(OpCode::NewTable, 0, 0, 0),               // R0 = {}
        Instruction::abc(OpCode::SetTable, 0, rk_as_k(0), rk_as_k(1)), // t[1]=10
        Instruction::abc(OpCode::SetTable, 0, rk_as_k(2), rk_as_k(3)), // t[2]=20
        Instruction::abc(OpCode::SetTable, 0, rk_as_k(4), rk_as_k(5)), // t[3]=30
        Instruction::abc(OpCode::Len, 1, 0, 0),                    // R1 = #t
        Instruction::abc(OpCode::GetTable, 2, 0, rk_as_k(2)),      // R2 = t[2]
        Instruction::abc(OpCode::Return, 1, 3, 0),                 // return R1, R2
    ];
    let consts = vec![
        Value::Number(1.0),
        Value::Number(10.0),
        Value::Number(2.0),
        Value::Number(20.0),
        Value::Number(3.0),
        Value::Number(30.0),
    ];
    let p = proto(code, consts, 3);

    let mut state = LuaState::new();
    let res = run(&mut state, p, &[]).unwrap();
    assert_eq!(num(&res[0]), 3.0, "#t");
    assert_eq!(num(&res[1]), 20.0, "t[2]");
}

#[test]
fn numeric_for_loop_sum() {
    // local s=0; for i=1,5 do s=s+i end; return s   => 15
    // レジスタ: R0=s, ループ制御 R1(idx/init) R2(limit) R3(step) R4(i)
    let code = vec![
        Instruction::abc(OpCode::LoadK, 0, 0, 0),    // R0 = 0   (K0=0)  ※abx だが a/bx で表現
        // 上は本当は LOADK(abx) だが bx=0 なので abc とビットが一致する。明示的に abx で作る:
        Instruction::abx(OpCode::LoadK, 1, 1),       // R1 = 1   (init, K1=1)
        Instruction::abx(OpCode::LoadK, 2, 2),       // R2 = 5   (limit, K2=5)
        Instruction::abx(OpCode::LoadK, 3, 1),       // R3 = 1   (step, K1=1)
        Instruction::asbx(OpCode::ForPrep, 1, 1),    // prep, jump to FORLOOP
        Instruction::abc(OpCode::Add, 0, 0, 4),      // body: R0 = R0 + R4(i)
        Instruction::asbx(OpCode::ForLoop, 1, -2),   // loop back to body
        Instruction::abc(OpCode::Return, 0, 2, 0),   // return R0
    ];
    let consts = vec![Value::Number(0.0), Value::Number(1.0), Value::Number(5.0)];
    // 注意: 先頭 LOADK は abx で作り直す。
    let mut code = code;
    code[0] = Instruction::abx(OpCode::LoadK, 0, 0);
    let p = proto(code, consts, 5);

    let mut state = LuaState::new();
    let res = run(&mut state, p, &[]).unwrap();
    assert_eq!(num(&res[0]), 15.0);
}

#[test]
fn closure_counter_shares_upvalue() {
    // 親: local x = 0; 子クロージャ inc が x を捕捉して x=x+1; return x。
    // 親は inc を 3 回呼び、最後の戻り値を返す（=3）。upvalue 共有を検証。
    //
    // 子 proto: 1 upvalue(x)。命令:
    //   GETUPVAL R0, U0
    //   ADD      R0, R0, K0(1)
    //   SETUPVAL R0, U0
    //   RETURN   R0 (1 value)
    let child = Rc::new(Proto {
        code: vec![
            Instruction::abc(OpCode::GetUpval, 0, 0, 0),
            Instruction::abc(OpCode::Add, 0, 0, rk_as_k(0)),
            Instruction::abc(OpCode::SetUpval, 0, 0, 0),
            Instruction::abc(OpCode::Return, 0, 2, 0),
        ],
        constants: vec![Value::Number(1.0)],
        num_upvalues: 1,
        max_stack_size: 1,
        source: Some("test".to_string()),
        ..Proto::default()
    });

    // 親 proto:
    //   LOADK    R0, K0(0)      ; x = 0
    //   CLOSURE  R1, proto0     ; inc = closure(child)
    //     MOVE   _, R0          ; 捕捉疑似命令: 親 R0 を upvalue へ
    //   MOVE     R2, R1         ; （呼び出し用にコピー）
    //   CALL     R2, 1, 1       ; inc()  (結果0個)
    //   MOVE     R2, R1
    //   CALL     R2, 1, 1
    //   MOVE     R2, R1
    //   CALL     R2, 1, 2       ; inc() -> R2 に1個
    //   RETURN   R2, 2
    let parent = Rc::new(Proto {
        code: vec![
            Instruction::abx(OpCode::LoadK, 0, 0),
            Instruction::abx(OpCode::Closure, 1, 0),
            Instruction::abc(OpCode::Move, 0, 0, 0), // CLOSURE 捕捉疑似命令: MOVE B=0 -> 親R0
            Instruction::abc(OpCode::Move, 2, 1, 0),
            Instruction::abc(OpCode::Call, 2, 1, 1),
            Instruction::abc(OpCode::Move, 2, 1, 0),
            Instruction::abc(OpCode::Call, 2, 1, 1),
            Instruction::abc(OpCode::Move, 2, 1, 0),
            Instruction::abc(OpCode::Call, 2, 1, 2),
            Instruction::abc(OpCode::Return, 2, 2, 0),
        ],
        constants: vec![Value::Number(0.0)],
        protos: vec![child],
        max_stack_size: 3,
        source: Some("test".to_string()),
        ..Proto::default()
    });

    let mut state = LuaState::new();
    let res = run(&mut state, parent, &[]).unwrap();
    assert_eq!(num(&res[0]), 3.0, "共有 upvalue が 3 回インクリメントされるべき");
}

#[test]
fn call_native_function() {
    // ネイティブ関数 double(x) = x*2 をグローバルに置き、呼び出す。
    fn double(state: &mut LuaState) -> rua_core::error::LuaResult<i32> {
        // 引数は呼び出しフレームの base から積まれている。
        let base = state.call_info.last().unwrap().base;
        let x = match state.stack.get(base) {
            Some(Value::Number(n)) => *n,
            _ => 0.0,
        };
        state.stack.push(Value::Number(x * 2.0));
        Ok(1)
    }

    let mut state = LuaState::new();
    let nat = state
        .global
        .heap
        .alloc_closure(Closure::Native(NativeClosure::new(double)));
    let res = call(&mut state, Value::GcRef(nat), &[Value::Number(21.0)]).unwrap();
    assert_eq!(num(&res[0]), 42.0);
}

#[test]
fn index_metamethod_fallback() {
    // t = setmetatable({}, { __index = fallback }) で t.missing が fallback を引く。
    // fallback テーブルに answer=42 を置き、t.answer == 42 を確認。
    let mut state = LuaState::new();

    // fallback テーブル: { answer = 42 }
    let fallback = state.global.heap.alloc_table(Table::new());
    let answer_key = state.new_string(b"answer");
    let GcHandle::Table(fk) = fallback else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(fk)
        .unwrap()
        .set(answer_key, Value::Number(42.0))
        .unwrap();

    // metatable: { __index = fallback }
    let mt = state.global.heap.alloc_table(Table::new());
    let index_key = state.new_string(b"__index");
    let GcHandle::Table(mtk) = mt else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(mtk)
        .unwrap()
        .set(index_key, Value::GcRef(fallback))
        .unwrap();

    // t = {} with metatable mt
    let t = state.global.heap.alloc_table(Table::new());
    let GcHandle::Table(tk) = t else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(tk)
        .unwrap()
        .set_metatable(Some(mt));

    // proto: R0 = arg t ; R1 = t["answer"] ; return R1  （num_params=1）
    let p = Rc::new(Proto {
        code: vec![
            Instruction::abc(OpCode::GetTable, 1, 0, rk_as_k(0)),
            Instruction::abc(OpCode::Return, 1, 2, 0),
        ],
        constants: vec![state.new_string(b"answer")],
        num_params: 1,
        max_stack_size: 2,
        source: Some("test".to_string()),
        ..Proto::default()
    });

    // t を引数に渡して呼ぶ。
    let closure = Closure::Lua(rua_core::value::closure::LuaClosure::new(p));
    let ch = state.global.heap.alloc_closure(closure);
    let res = call(&mut state, Value::GcRef(ch), &[Value::GcRef(t)]).unwrap();
    assert_eq!(num(&res[0]), 42.0);
}

#[test]
fn string_concat() {
    // return "foo" .. "bar" .. 42  => "foobar42"
    let mut state = LuaState::new();
    let foo = state.new_string(b"foo");
    let bar = state.new_string(b"bar");
    let p = proto(
        vec![
            Instruction::abc(OpCode::LoadK, 0, 0, 0), // R0 = "foo"  (使うのは LOADK abx)
            Instruction::abc(OpCode::Return, 0, 2, 0),
        ],
        vec![foo],
        4,
    );
    // 正式には CONCAT を使う。code を組み直す。
    let code = vec![
        Instruction::abx(OpCode::LoadK, 0, 0), // "foo"
        Instruction::abx(OpCode::LoadK, 1, 1), // "bar"
        Instruction::abx(OpCode::LoadK, 2, 2), // 42
        Instruction::abc(OpCode::Concat, 0, 0, 2),
        Instruction::abc(OpCode::Return, 0, 2, 0),
    ];
    let p2 = proto(code, vec![foo, bar, Value::Number(42.0)], 3);
    let _ = p;

    let res = run(&mut state, p2, &[]).unwrap();
    match res[0] {
        Value::GcRef(GcHandle::Str(k)) => {
            assert_eq!(state.global.heap.get_str(k).unwrap().as_bytes(), b"foobar42");
        }
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn deep_tail_recursion_does_not_overflow() {
    // f(n, acc): if n == 0 then return acc else return f(n-1, acc+n) end  （末尾呼び出し）
    // f を _G["f"] に置き、f(100000, 0) を呼ぶ。TCO が無ければ stack overflow になる深さ。
    let mut state = LuaState::new();

    // f の定数: K0=0, K1=1, K2="f"
    let fname = state.new_string(b"f");
    let f_proto = Rc::new(Proto {
        code: vec![
            Instruction::abc(OpCode::Eq, 1, 0, rk_as_k(0)), // if (n==0) ~= true -> pc++（n!=0なら次をスキップ）
            Instruction::asbx(OpCode::Jmp, 0, 7),           // n==0: 末尾の RETURN acc へ
            Instruction::abc(OpCode::Sub, 2, 0, rk_as_k(1)), // R2 = n - 1
            Instruction::abc(OpCode::Add, 3, 1, 0),          // R3 = acc + n
            Instruction::abx(OpCode::GetGlobal, 4, 2),       // R4 = _G["f"]
            Instruction::abc(OpCode::Move, 5, 2, 0),         // R5 = R2
            Instruction::abc(OpCode::Move, 6, 3, 0),         // R6 = R3
            Instruction::abc(OpCode::TailCall, 4, 3, 0),     // return f(R5, R6)
            Instruction::abc(OpCode::Return, 4, 0, 0),       // （TAILCALL 後の定型 RETURN, 実行されない）
            Instruction::abc(OpCode::Return, 1, 2, 0),       // return acc
        ],
        constants: vec![Value::Number(0.0), Value::Number(1.0), fname],
        num_params: 2,
        max_stack_size: 7,
        source: Some("@tco".to_string()),
        ..Proto::default()
    });

    // main: f = closure(f_proto); _G["f"] = f; return f(100000, 0)
    let main = Rc::new(Proto {
        code: vec![
            Instruction::abx(OpCode::Closure, 0, 0),   // R0 = f
            Instruction::abx(OpCode::SetGlobal, 0, 0),  // _G["f"] = R0
            Instruction::abx(OpCode::GetGlobal, 0, 0),  // R0 = _G["f"]
            Instruction::abx(OpCode::LoadK, 1, 1),      // R1 = 100000
            Instruction::abx(OpCode::LoadK, 2, 2),      // R2 = 0
            Instruction::abc(OpCode::Call, 0, 3, 2),    // R0 = f(R1, R2)
            Instruction::abc(OpCode::Return, 0, 2, 0),  // return R0
        ],
        constants: vec![state.new_string(b"f"), Value::Number(100000.0), Value::Number(0.0)],
        protos: vec![f_proto],
        max_stack_size: 3,
        source: Some("@main".to_string()),
        ..Proto::default()
    });

    let res = run(&mut state, main, &[]).unwrap();
    // sum 1..100000 = 5000050000
    assert_eq!(num(&res[0]), 5_000_050_000.0);
}

#[test]
fn string_indexing_uses_string_metatable() {
    // string メタテーブルに __index = { upper = <native> } を設定し、("hi"):upper() 相当の
    // GETTABLE / SELF が文字列メタテーブル経由で解決されることを検証。
    fn upper(state: &mut LuaState) -> rua_core::error::LuaResult<i32> {
        let base = state.call_info.last().unwrap().base;
        let s = match state.stack.get(base) {
            Some(Value::GcRef(GcHandle::Str(k))) => {
                state.global.heap.get_str(*k).unwrap().as_bytes().to_ascii_uppercase()
            }
            _ => Vec::new(),
        };
        let v = state.new_string(&s);
        state.stack.push(v);
        Ok(1)
    }

    let mut state = LuaState::new();

    // メソッドテーブル string_lib = { upper = upper }
    let string_lib = state.global.heap.alloc_table(Table::new());
    let upper_fn = state
        .global
        .heap
        .alloc_closure(Closure::Native(NativeClosure::new(upper)));
    let upper_key = state.new_string(b"upper");
    let GcHandle::Table(slk) = string_lib else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(slk)
        .unwrap()
        .set(upper_key, Value::GcRef(upper_fn))
        .unwrap();

    // string メタテーブル = { __index = string_lib }
    let str_mt = state.global.heap.alloc_table(Table::new());
    let index_key = state.new_string(b"__index");
    let GcHandle::Table(mtk) = str_mt else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(mtk)
        .unwrap()
        .set(index_key, Value::GcRef(string_lib))
        .unwrap();
    state.global.string_metatable = Some(str_mt);

    // proto(s): SELF R1,R0,"upper"  ; CALL R1,1,2 ; RETURN R1
    //   SELF: R2 := R0; R1 := R0["upper"]
    let p = Rc::new(Proto {
        code: vec![
            Instruction::abc(OpCode::SelfOp, 1, 0, rk_as_k(0)), // R1 = R0["upper"], R2 = R0
            Instruction::abc(OpCode::Call, 1, 2, 2),            // R1 = R1(R2)
            Instruction::abc(OpCode::Return, 1, 2, 0),
        ],
        constants: vec![state.new_string(b"upper")],
        num_params: 1,
        max_stack_size: 3,
        source: Some("@s".to_string()),
        ..Proto::default()
    });

    let closure = Closure::Lua(rua_core::value::closure::LuaClosure::new(p));
    let ch = state.global.heap.alloc_closure(closure);
    let arg = state.new_string(b"hi");
    let res = call(&mut state, Value::GcRef(ch), &[arg]).unwrap();
    match res[0] {
        Value::GcRef(GcHandle::Str(k)) => {
            assert_eq!(state.global.heap.get_str(k).unwrap().as_bytes(), b"HI");
        }
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn error_message_strips_chunk_prefix() {
    // @file 由来チャンクのエラーは "file:line: ..." と表示される（先頭 '@' を除去）。
    let mut state = LuaState::new();
    let p = Rc::new(Proto {
        code: vec![
            // R0 = nil; R1 = R0 + 1 -> arithmetic on nil でエラー。
            Instruction::abc(OpCode::LoadNil, 0, 0, 0),
            Instruction::abc(OpCode::Add, 1, 0, rk_as_k(0)),
            Instruction::abc(OpCode::Return, 1, 2, 0),
        ],
        constants: vec![Value::Number(1.0)],
        max_stack_size: 2,
        source: Some("@myscript.lua".to_string()),
        line_info: vec![1, 2, 2],
        ..Proto::default()
    });

    let err = run(&mut state, p, &[]).unwrap_err();
    let rua_core::error::LuaError::Runtime(Value::GcRef(GcHandle::Str(k))) = err else {
        panic!("expected runtime string error, got {err:?}");
    };
    let msg = String::from_utf8_lossy(state.global.heap.get_str(k).unwrap().as_bytes()).into_owned();
    assert!(msg.starts_with("myscript.lua:2:"), "got: {msg}");
    assert!(msg.contains("attempt to perform arithmetic on a nil value"), "got: {msg}");
}

#[test]
fn where_string_reports_caller_line() {
    // ネイティブ関数が where_string(1) で「自分を呼んだ Lua 関数の現在行」を取得できること。
    // error(msg, 1) の位置付与（luaL_where 相当）の検証。
    use std::sync::Mutex;
    static CAPTURED: Mutex<Option<String>> = Mutex::new(None);

    fn probe(state: &mut LuaState) -> rua_core::error::LuaResult<i32> {
        let w = rua_core::vm::where_string(state, 1);
        *CAPTURED.lock().unwrap() = Some(w);
        Ok(0)
    }

    let mut state = LuaState::new();
    let probe_fn = state
        .global
        .heap
        .alloc_closure(Closure::Native(NativeClosure::new(probe)));
    // _G["probe"] = probe
    let gname = state.new_string(b"probe");
    let GcHandle::Table(gk) = state.global.globals else { unreachable!() };
    state
        .global
        .heap
        .get_table_mut(gk)
        .unwrap()
        .set(gname, Value::GcRef(probe_fn))
        .unwrap();

    // proto: 命令0 は GETGLOBAL(行10), 命令1 は CALL(行20), 命令2 RETURN
    let p = Rc::new(Proto {
        code: vec![
            Instruction::abx(OpCode::GetGlobal, 0, 0), // R0 = _G["probe"]
            Instruction::abc(OpCode::Call, 0, 1, 1),   // probe()
            Instruction::abc(OpCode::Return, 0, 1, 0),
        ],
        constants: vec![state.new_string(b"probe")],
        max_stack_size: 1,
        source: Some("@caller.lua".to_string()),
        line_info: vec![10, 20, 21],
        ..Proto::default()
    });

    run(&mut state, p, &[]).unwrap();
    let captured = CAPTURED.lock().unwrap().clone().unwrap();
    // CALL 命令（行20）が probe を呼んだので "caller.lua:20: " が得られる。
    assert_eq!(captured, "caller.lua:20: ");
}
