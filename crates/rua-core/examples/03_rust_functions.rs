//! Example 3: Exposing Rust functions to Lua
//!
//! Demonstrates how to register Rust functions so that Lua code can call
//! them directly.  Three functions are shown:
//!
//!   - `vec_len(x, y, z)` — 3-D vector length (pure math, returns f64)
//!   - `repeat_str(s, n)` — repeats a string n times (string ↔ Rust)
//!   - `dump_table(t)`    — iterates a Lua table from Rust (table traversal)
//!
//! Run with:
//!   cargo run -p rua-core --example 03_rust_functions

use rua_core::api::Lua;
use rua_core::error::LuaResult;
use rua_core::state::LuaState;
use rua_core::stdlib::aux;
use rua_core::value::Value;

// ── Native function 1: 3-D vector length ─────────────────────────────────────
//
// NativeFn signature: fn(&mut LuaState) -> LuaResult<i32>
// Arguments arrive on the VM stack; use aux helpers to extract them.
// Return value: push results onto the stack and return the count.
fn vec_len(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, "vec_len")?;
    let y = aux::check_number(state, &args, 1, "vec_len")?;
    let z = aux::check_number(state, &args, 2, "vec_len")?;
    let len = (x * x + y * y + z * z).sqrt();
    aux::ret(state, vec![Value::Number(len)])
}

// ── Native function 2: repeat a string ───────────────────────────────────────
fn repeat_str(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "repeat_str")?;
    let n = aux::check_int(state, &args, 1, "repeat_str")?;
    let repeated: Vec<u8> = s.repeat(n.max(0) as usize);
    let sv = state.new_string(&repeated);
    aux::ret(state, vec![sv])
}

// ── Native function 3: dump a Lua table ──────────────────────────────────────
//
// Iterates key-value pairs of the table passed as argument 1 and prints them
// to stdout.  This demonstrates how to traverse a Lua table from Rust.
fn dump_table(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "dump_table")?;

    // Collect all pairs first (can't borrow heap and print at the same time).
    let pairs: Vec<(Value, Value)> = state
        .global
        .heap
        .get_table(tk)
        .map(|t| {
            let mut result = Vec::new();
            let mut cur = Value::Nil;
            while let Ok(Some((k, v))) = t.next(&cur) {
                cur = k;
                result.push((cur, v));
            }
            result
        })
        .unwrap_or_default();

    for (k, v) in &pairs {
        let ks = aux::raw_tostring(state, *k);
        let vs = aux::raw_tostring(state, *v);
        println!(
            "  [{}] = {}",
            String::from_utf8_lossy(&ks),
            String::from_utf8_lossy(&vs)
        );
    }

    aux::ret0(state)
}

fn main() {
    let mut lua = Lua::new();

    // ── Register the Rust functions as Lua globals ────────────────────────
    lua.register_fn("vec_len", vec_len).unwrap();
    lua.register_fn("repeat_str", repeat_str).unwrap();
    lua.register_fn("dump_table", dump_table).unwrap();

    // ── Call from Lua ─────────────────────────────────────────────────────

    // 1. Vector length
    let len: f64 = lua.load("return vec_len(1, 2, 2)").eval().unwrap();
    println!("vec_len(1, 2, 2) = {len:.4}"); // sqrt(1+4+4) = 3.0000

    // 2. String repetition
    let repeated: String = lua.load(r#"return repeat_str("ab", 4)"#).eval().unwrap();
    println!("repeat_str('ab', 4) = {repeated}"); // abababab

    // 3. Table dump (printed inside the Rust function)
    println!("dump_table({{x=10, y=20, z=30}}):");
    lua.load(r#"dump_table({x = 10, y = 20, z = 30})"#)
        .exec()
        .unwrap();

    // ── Round-trip: Lua defines a function, Rust calls it ─────────────────
    lua.load(
        r#"
        function greet(name)
            return "Hello, " .. name .. "! vec_len(3,4,0) = " .. vec_len(3, 4, 0)
        end
    "#,
    )
    .exec()
    .unwrap();

    let greet_fn: rua_core::api::Function = lua.get_global("greet").unwrap();
    let (msg,): (String,) = lua.call(greet_fn, ("Rust",)).unwrap();
    println!("{msg}"); // Hello, Rust! vec_len(3,4,0) = 5.0
}
