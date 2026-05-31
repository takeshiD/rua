//! Example 1: Basic evaluation
//!
//! Create a Lua environment, run inline Lua code, and retrieve results
//! as Rust values — no files, no external state.
//!
//! Run with:
//!   cargo run -p rua-core --example 01_basic_eval

use rua_core::api::Lua;

fn main() {
    let mut lua = Lua::new();

    // ── Arithmetic ────────────────────────────────────────────────────────
    let sum: f64 = lua.load("return 1 + 2").eval().unwrap();
    println!("1 + 2 = {sum}"); // 3

    let result: f64 = lua.load("return (10 + 5) * 2 - 3").eval().unwrap();
    println!("(10 + 5) * 2 - 3 = {result}"); // 27

    // ── String operations ─────────────────────────────────────────────────
    let greeting: String = lua
        .load(r#"return "Hello" .. ", " .. "world" .. "!""#)
        .eval()
        .unwrap();
    println!("{greeting}"); // Hello, world!

    // ── Global variables ──────────────────────────────────────────────────
    // Set a global from Rust, then read it from Lua.
    lua.set_global("base", 100i64).unwrap();
    let doubled: f64 = lua.load("return base * 2").eval().unwrap();
    println!("base * 2 = {doubled}"); // 200

    // Run a Lua snippet that modifies a global, then read it back in Rust.
    lua.load("counter = 0; for i = 1, 5 do counter = counter + i end")
        .exec()
        .unwrap();
    let counter: f64 = lua.get_global("counter").unwrap();
    println!("sum 1..5 = {counter}"); // 15

    // ── Tables ────────────────────────────────────────────────────────────
    let max: f64 = lua
        .load(
            r#"
            local nums = {3, 1, 4, 1, 5, 9, 2, 6}
            table.sort(nums)
            return nums[#nums]
        "#,
        )
        .eval()
        .unwrap();
    println!("max of {{3,1,4,1,5,9,2,6}} = {max}"); // 9

    // ── Error handling ────────────────────────────────────────────────────
    // Lua runtime errors propagate as Rust `Err`.
    let bad: Result<f64, _> = lua.load("error('oops')").eval();
    println!("error caught: {}", bad.is_err()); // true
}
