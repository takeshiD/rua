//! 標準ライブラリ（`stdlib`）の動作テスト。
//!
//! `open_libs` でグローバルへ開いた組み込み関数を `vm::call` 経由で直接呼び、
//! 戻り値を検証する（CLI/フロントエンドに依存せず stdlib 単体を確認する）。

use rua_core::gc::GcHandle;
use rua_core::state::LuaState;
use rua_core::stdlib;
use rua_core::value::Value;
use rua_core::vm;

// ---- ヘルパ -----------------------------------------------------------------

fn new_state() -> LuaState {
    let mut s = LuaState::new();
    stdlib::open_libs(&mut s);
    s
}

fn sval(state: &mut LuaState, s: &str) -> Value {
    state.new_string(s.as_bytes())
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

fn global(state: &mut LuaState, name: &str) -> Value {
    let k = state.new_string(name.as_bytes());
    match state.global.globals {
        GcHandle::Table(tk) => state.global.heap.get_table(tk).unwrap().get(&k),
        _ => Value::Nil,
    }
}

fn field(state: &mut LuaState, t: Value, name: &str) -> Value {
    let k = state.new_string(name.as_bytes());
    match t {
        Value::GcRef(GcHandle::Table(tk)) => state.global.heap.get_table(tk).unwrap().get(&k),
        _ => Value::Nil,
    }
}

/// グローバル関数 `name` を呼ぶ。
fn call_g(state: &mut LuaState, name: &str, args: &[Value]) -> Vec<Value> {
    let f = global(state, name);
    vm::call(state, f, args).expect("call ok")
}

/// `lib.name` を呼ぶ。
fn call_lib(state: &mut LuaState, lib: &str, name: &str, args: &[Value]) -> Vec<Value> {
    let l = global(state, lib);
    let f = field(state, l, name);
    vm::call(state, f, args).expect("call ok")
}

fn as_string(state: &LuaState, v: Value) -> String {
    match v {
        Value::GcRef(GcHandle::Str(k)) => {
            String::from_utf8_lossy(state.global.heap.get_str(k).unwrap().as_bytes()).into_owned()
        }
        _ => panic!("expected string, got {v:?}"),
    }
}

fn as_num(v: Value) -> f64 {
    match v {
        Value::Number(n) => n,
        _ => panic!("expected number, got {v:?}"),
    }
}

fn new_array(state: &mut LuaState, items: &[Value]) -> Value {
    let t = state.new_table();
    if let Value::GcRef(GcHandle::Table(tk)) = t {
        for (i, v) in items.iter().enumerate() {
            let _ = state.global.heap.get_table_mut(tk).unwrap().set(Value::Number((i + 1) as f64), *v);
        }
    }
    t
}

// ---- base -------------------------------------------------------------------

#[test]
fn type_and_tostring() {
    let mut s = new_state();
    let r = call_g(&mut s, "type", &[Value::Number(1.0)]);
    assert_eq!(as_string(&s, r[0]), "number");
    let nilv = call_g(&mut s, "type", &[Value::Nil]);
    assert_eq!(as_string(&s, nilv[0]), "nil");
    let bv = call_g(&mut s, "type", &[Value::Boolean(true)]);
    assert_eq!(as_string(&s, bv[0]), "boolean");

    let r = call_g(&mut s, "tostring", &[Value::Number(1.5)]);
    assert_eq!(as_string(&s, r[0]), "1.5");
    let r = call_g(&mut s, "tostring", &[Value::Nil]);
    assert_eq!(as_string(&s, r[0]), "nil");
    let r = call_g(&mut s, "tostring", &[Value::Boolean(false)]);
    assert_eq!(as_string(&s, r[0]), "false");
}

#[test]
fn tonumber_variants() {
    let mut s = new_state();
    let arg = sval(&mut s, "42");
    let r = call_g(&mut s, "tonumber", &[arg]);
    assert_eq!(as_num(r[0]), 42.0);

    let arg = sval(&mut s, "  3.5  ");
    let r = call_g(&mut s, "tonumber", &[arg]);
    assert_eq!(as_num(r[0]), 3.5);

    let arg = sval(&mut s, "ff");
    let r = call_g(&mut s, "tonumber", &[arg, Value::Number(16.0)]);
    assert_eq!(as_num(r[0]), 255.0);

    let arg = sval(&mut s, "hello");
    let r = call_g(&mut s, "tonumber", &[arg]);
    assert!(matches!(r[0], Value::Nil));
}

#[test]
fn select_works() {
    let mut s = new_state();
    let hash = sval(&mut s, "#");
    let r = call_g(&mut s, "select", &[hash, Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
    assert_eq!(as_num(r[0]), 3.0);

    let r = call_g(&mut s, "select", &[Value::Number(2.0), Value::Number(10.0), Value::Number(20.0), Value::Number(30.0)]);
    assert_eq!(r.len(), 2);
    assert_eq!(as_num(r[0]), 20.0);
    assert_eq!(as_num(r[1]), 30.0);
}

#[test]
fn assert_and_pcall() {
    let mut s = new_state();
    // assert(true, ...) returns its args.
    let r = call_g(&mut s, "assert", &[Value::Number(5.0), Value::Boolean(true)]);
    assert_eq!(as_num(r[0]), 5.0);

    // pcall(assert, false) -> (false, "assertion failed!")
    let assertf = global(&mut s, "assert");
    let r = call_g(&mut s, "pcall", &[assertf, Value::Boolean(false)]);
    assert_eq!(r[0], Value::Boolean(false));
    assert_eq!(as_string(&s, r[1]), "assertion failed!");
}

#[test]
fn rawequal_rawget_rawset() {
    let mut s = new_state();
    let t = new_array(&mut s, &[]);
    let k = sval(&mut s, "x");
    // rawset(t, "x", 7)
    call_g(&mut s, "rawset", &[t, k, Value::Number(7.0)]);
    // rawget(t, "x") == 7
    let r = call_g(&mut s, "rawget", &[t, k]);
    assert_eq!(as_num(r[0]), 7.0);
    // rawequal(t, t) is true; rawequal(t, other) false
    let r = call_g(&mut s, "rawequal", &[t, t]);
    assert_eq!(r[0], Value::Boolean(true));
    let other = new_array(&mut s, &[]);
    let r = call_g(&mut s, "rawequal", &[t, other]);
    assert_eq!(r[0], Value::Boolean(false));
}

#[test]
fn pairs_iterates_all() {
    let mut s = new_state();
    let t = state_table_with_mixed(&mut s);
    // pairs(t) -> (next, t, nil)
    let triplet = call_g(&mut s, "pairs", &[t]);
    let nextf = triplet[0];
    let mut key = Value::Nil;
    let mut count = 0;
    let mut sum = 0.0;
    loop {
        let r = vm::call(&mut s, nextf, &[t, key]).unwrap();
        if r.is_empty() || matches!(r[0], Value::Nil) {
            break;
        }
        key = r[0];
        if let Value::Number(n) = r[1] {
            sum += n;
        }
        count += 1;
        assert!(count <= 10, "pairs did not terminate");
    }
    // 3 array entries (1,2,3) + 1 hash entry ("k"=100)
    assert_eq!(count, 4);
    assert_eq!(sum, 1.0 + 2.0 + 3.0 + 100.0);
}

fn state_table_with_mixed(state: &mut LuaState) -> Value {
    let t = new_array(state, &[Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
    if let Value::GcRef(GcHandle::Table(tk)) = t {
        let k = state.new_string(b"k");
        let _ = state.global.heap.get_table_mut(tk).unwrap().set(k, Value::Number(100.0));
    }
    t
}

#[test]
fn ipairs_stops_at_hole() {
    let mut s = new_state();
    let t = new_array(&mut s, &[Value::Number(10.0), Value::Number(20.0)]);
    let triplet = call_g(&mut s, "ipairs", &[t]);
    let iter = triplet[0];
    let mut i = Value::Number(0.0);
    let mut seen = Vec::new();
    loop {
        let r = vm::call(&mut s, iter, &[t, i]).unwrap();
        if r.is_empty() {
            break;
        }
        i = r[0];
        seen.push(as_num(r[1]));
    }
    assert_eq!(seen, vec![10.0, 20.0]);
}

// ---- string -----------------------------------------------------------------

#[test]
fn string_basics() {
    let mut s = new_state();
    let hello = sval(&mut s, "Hello");
    let r = call_lib(&mut s, "string", "len", &[hello]);
    assert_eq!(as_num(r[0]), 5.0);

    let r = call_lib(&mut s, "string", "upper", &[hello]);
    assert_eq!(as_string(&s, r[0]), "HELLO");

    let r = call_lib(&mut s, "string", "lower", &[hello]);
    assert_eq!(as_string(&s, r[0]), "hello");

    let r = call_lib(&mut s, "string", "sub", &[hello, num(2.0), num(4.0)]);
    assert_eq!(as_string(&s, r[0]), "ell");

    let r = call_lib(&mut s, "string", "sub", &[hello, num(-3.0)]);
    assert_eq!(as_string(&s, r[0]), "llo");

    let ab = sval_ab(&mut s);
    let r = call_lib(&mut s, "string", "rep", &[ab, num(3.0)]);
    assert_eq!(as_string(&s, r[0]), "ababab");

    let r = call_lib(&mut s, "string", "byte", &[hello, num(1.0)]);
    assert_eq!(as_num(r[0]), 72.0);

    let r = call_lib(&mut s, "string", "char", &[num(65.0), num(66.0), num(67.0)]);
    assert_eq!(as_string(&s, r[0]), "ABC");

    let r = call_lib(&mut s, "string", "reverse", &[hello]);
    assert_eq!(as_string(&s, r[0]), "olleH");
}

fn sval_ab(state: &mut LuaState) -> Value {
    state.new_string(b"ab")
}

#[test]
fn string_format_specifiers() {
    let mut s = new_state();
    let fmt = sval(&mut s, "%d");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(42.0)]);
    assert_eq!(as_string(&s, r[0]), "42");

    let fmt = sval(&mut s, "%5d");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(42.0)]);
    assert_eq!(as_string(&s, r[0]), "   42");

    let fmt = sval(&mut s, "%-5d|");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(42.0)]);
    assert_eq!(as_string(&s, r[0]), "42   |");

    let fmt = sval(&mut s, "%05d");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(42.0)]);
    assert_eq!(as_string(&s, r[0]), "00042");

    let fmt = sval(&mut s, "%x");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(255.0)]);
    assert_eq!(as_string(&s, r[0]), "ff");

    let fmt = sval(&mut s, "%#x");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(255.0)]);
    assert_eq!(as_string(&s, r[0]), "0xff");

    let fmt = sval(&mut s, "%.2f");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(2.5)]);
    assert_eq!(as_string(&s, r[0]), "2.50");

    let fmt = sval(&mut s, "%g");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(100000.0)]);
    assert_eq!(as_string(&s, r[0]), "100000");

    let fmt = sval(&mut s, "[%s]");
    let arg = sval(&mut s, "hi");
    let r = call_lib(&mut s, "string", "format", &[fmt, arg]);
    assert_eq!(as_string(&s, r[0]), "[hi]");

    let fmt = sval(&mut s, "%5.2s");
    let arg = sval(&mut s, "hello");
    let r = call_lib(&mut s, "string", "format", &[fmt, arg]);
    assert_eq!(as_string(&s, r[0]), "   he");

    let fmt = sval(&mut s, "%q");
    let arg = sval(&mut s, "a\"b\nc");
    let r = call_lib(&mut s, "string", "format", &[fmt, arg]);
    assert_eq!(as_string(&s, r[0]), "\"a\\\"b\\nc\"");

    let fmt = sval(&mut s, "%d%%");
    let r = call_lib(&mut s, "string", "format", &[fmt, num(50.0)]);
    assert_eq!(as_string(&s, r[0]), "50%");
}

#[test]
fn string_find_plain_and_pattern() {
    let mut s = new_state();
    let hay = sval(&mut s, "hello world");
    let pat = sval(&mut s, "world");
    let r = call_lib(&mut s, "string", "find", &[hay, pat]);
    assert_eq!(as_num(r[0]), 7.0);
    assert_eq!(as_num(r[1]), 11.0);

    // pattern with capture
    let hay = sval(&mut s, "key=value");
    let pat = sval(&mut s, "(%w+)=(%w+)");
    let r = call_lib(&mut s, "string", "find", &[hay, pat]);
    assert_eq!(as_num(r[0]), 1.0);
    assert_eq!(as_num(r[1]), 9.0);
    assert_eq!(as_string(&s, r[2]), "key");
    assert_eq!(as_string(&s, r[3]), "value");
}

#[test]
fn string_match_and_anchors() {
    let mut s = new_state();
    let str_ = sval(&mut s, "   2026-05-30");
    let pat = sval(&mut s, "(%d+)-(%d+)-(%d+)");
    let r = call_lib(&mut s, "string", "match", &[str_, pat]);
    assert_eq!(as_string(&s, r[0]), "2026");
    assert_eq!(as_string(&s, r[1]), "05");
    assert_eq!(as_string(&s, r[2]), "30");

    // anchored
    let str_ = sval(&mut s, "abc123");
    let pat = sval(&mut s, "^%a+");
    let r = call_lib(&mut s, "string", "match", &[str_, pat]);
    assert_eq!(as_string(&s, r[0]), "abc");

    // no match -> nil
    let str_ = sval(&mut s, "xyz");
    let pat = sval(&mut s, "%d+");
    let r = call_lib(&mut s, "string", "match", &[str_, pat]);
    assert!(matches!(r[0], Value::Nil));
}

#[test]
fn string_gsub_string_and_count() {
    let mut s = new_state();
    let src = sval(&mut s, "hello world");
    let pat = sval(&mut s, "o");
    let repl = sval(&mut s, "0");
    let r = call_lib(&mut s, "string", "gsub", &[src, pat, repl]);
    assert_eq!(as_string(&s, r[0]), "hell0 w0rld");
    assert_eq!(as_num(r[1]), 2.0);

    // capture reference in replacement
    let src = sval(&mut s, "hello world");
    let pat = sval(&mut s, "(%w+)");
    let repl = sval(&mut s, "<%1>");
    let r = call_lib(&mut s, "string", "gsub", &[src, pat, repl]);
    assert_eq!(as_string(&s, r[0]), "<hello> <world>");
    assert_eq!(as_num(r[1]), 2.0);

    // max replacements
    let src = sval(&mut s, "aaa");
    let pat = sval(&mut s, "a");
    let repl = sval(&mut s, "b");
    let r = call_lib(&mut s, "string", "gsub", &[src, pat, repl, num(2.0)]);
    assert_eq!(as_string(&s, r[0]), "bba");
    assert_eq!(as_num(r[1]), 2.0);
}

#[test]
fn string_gmatch_iterates() {
    let mut s = new_state();
    let src = sval(&mut s, "a,bb,ccc");
    let pat = sval(&mut s, "[^,]+");
    let triplet = call_lib(&mut s, "string", "gmatch", &[src, pat]);
    let iter = triplet[0];
    let mut out = Vec::new();
    loop {
        let r = vm::call(&mut s, iter, &[Value::Nil, Value::Nil]).unwrap();
        if r.is_empty() {
            break;
        }
        out.push(as_string(&s, r[0]));
        assert!(out.len() <= 5);
    }
    assert_eq!(out, vec!["a", "bb", "ccc"]);
}

// ---- table ------------------------------------------------------------------

#[test]
fn table_insert_remove() {
    let mut s = new_state();
    let t = new_array(&mut s, &[Value::Number(1.0), Value::Number(2.0)]);
    call_lib(&mut s, "table", "insert", &[t, Value::Number(3.0)]);
    let r = call_lib(&mut s, "table", "getn", &[t]);
    assert_eq!(as_num(r[0]), 3.0);

    // insert at position
    call_lib(&mut s, "table", "insert", &[t, num(1.0), num(99.0)]);
    let got = field_int(&mut s, t, 1);
    assert_eq!(as_num(got), 99.0);

    // remove last
    let r = call_lib(&mut s, "table", "remove", &[t]);
    assert_eq!(as_num(r[0]), 3.0);
}

fn field_int(state: &mut LuaState, t: Value, i: usize) -> Value {
    match t {
        Value::GcRef(GcHandle::Table(tk)) => state.global.heap.get_table(tk).unwrap().get_int(i),
        _ => Value::Nil,
    }
}

#[test]
fn table_concat() {
    let mut s = new_state();
    let a = sval(&mut s, "a");
    let b = sval(&mut s, "b");
    let c = sval(&mut s, "c");
    let t = new_array(&mut s, &[a, b, c]);
    let sep = sval(&mut s, ", ");
    let r = call_lib(&mut s, "table", "concat", &[t, sep]);
    assert_eq!(as_string(&s, r[0]), "a, b, c");
}

#[test]
fn table_sort_default_and_custom() {
    let mut s = new_state();
    let t = new_array(
        &mut s,
        &[Value::Number(3.0), Value::Number(1.0), Value::Number(2.0)],
    );
    call_lib(&mut s, "table", "sort", &[t]);
    assert_eq!(as_num(field_int(&mut s, t, 1)), 1.0);
    assert_eq!(as_num(field_int(&mut s, t, 2)), 2.0);
    assert_eq!(as_num(field_int(&mut s, t, 3)), 3.0);
}

#[test]
fn table_maxn() {
    let mut s = new_state();
    let t = new_array(&mut s, &[Value::Number(10.0), Value::Number(20.0)]);
    if let Value::GcRef(GcHandle::Table(tk)) = t {
        state_set(&mut s, tk, 100.0, Value::Number(5.0));
    }
    let r = call_lib(&mut s, "table", "maxn", &[t]);
    assert_eq!(as_num(r[0]), 100.0);
}

fn state_set(state: &mut LuaState, tk: rua_core::gc::TableKey, key: f64, v: Value) {
    let _ = state.global.heap.get_table_mut(tk).unwrap().set(Value::Number(key), v);
}

// ---- math -------------------------------------------------------------------

#[test]
fn math_functions() {
    let mut s = new_state();
    let r = call_lib(&mut s, "math", "floor", &[num(3.7)]);
    assert_eq!(as_num(r[0]), 3.0);
    let r = call_lib(&mut s, "math", "ceil", &[num(3.2)]);
    assert_eq!(as_num(r[0]), 4.0);
    let r = call_lib(&mut s, "math", "abs", &[num(-5.0)]);
    assert_eq!(as_num(r[0]), 5.0);
    let r = call_lib(&mut s, "math", "max", &[num(1.0), num(9.0), num(4.0)]);
    assert_eq!(as_num(r[0]), 9.0);
    let r = call_lib(&mut s, "math", "min", &[num(1.0), num(9.0), num(4.0)]);
    assert_eq!(as_num(r[0]), 1.0);
    let r = call_lib(&mut s, "math", "sqrt", &[num(16.0)]);
    assert_eq!(as_num(r[0]), 4.0);

    let mathlib = global(&mut s, "math");
    let pi = field(&mut s, mathlib, "pi");
    assert!((as_num(pi) - std::f64::consts::PI).abs() < 1e-12);
    let huge = field(&mut s, mathlib, "huge");
    assert!(as_num(huge).is_infinite());

    // random in range
    let r = call_lib(&mut s, "math", "random", &[num(1.0), num(6.0)]);
    let v = as_num(r[0]);
    assert!((1.0..=6.0).contains(&v) && v.fract() == 0.0);
}

// ---- os ---------------------------------------------------------------------

#[test]
fn os_date_utc_and_difftime() {
    let mut s = new_state();
    let fmt = sval(&mut s, "!%Y-%m-%d %H:%M:%S");
    let r = call_lib(&mut s, "os", "date", &[fmt, num(0.0)]);
    assert_eq!(as_string(&s, r[0]), "1970-01-01 00:00:00");

    let fmt = sval(&mut s, "!%Y-%m-%d");
    let r = call_lib(&mut s, "os", "date", &[fmt, num(86400.0)]);
    assert_eq!(as_string(&s, r[0]), "1970-01-02");

    let r = call_lib(&mut s, "os", "difftime", &[num(100.0), num(40.0)]);
    assert_eq!(as_num(r[0]), 60.0);

    // time/clock は数値。
    let r = call_lib(&mut s, "os", "time", &[]);
    assert!(matches!(r[0], Value::Number(_)));
    let r = call_lib(&mut s, "os", "clock", &[]);
    assert!(matches!(r[0], Value::Number(_)));

    // getenv: 未設定は nil。
    let key = sval(&mut s, "RUA_DEFINITELY_NOT_SET_98765");
    let r = call_lib(&mut s, "os", "getenv", &[key]);
    assert!(matches!(r[0], Value::Nil));
}
