//! Example 2: Load and execute an external Lua file
//!
//! Reads a Lua source file from disk, executes it inside a `Lua` environment,
//! and exchanges values between Rust and the script via globals.
//!
//! Run with:
//!   cargo run -p rua-core --example 02_load_file

use std::fs;
use std::io::Write;

use rua_core::api::Lua;

fn main() {
    // ── Write a temporary Lua script to disk ──────────────────────────────
    // In real usage you would point this at an existing file.
    let script_path = std::env::temp_dir().join("rua_example_02.lua");
    {
        let mut f = fs::File::create(&script_path).unwrap();
        writeln!(
            f,
            r#"
-- rua_example_02.lua
-- Receives `input_value` and `multiplier` from Rust (set as globals),
-- performs some computation, and stores results back as globals.

local function factorial(n)
    if n <= 1 then return 1 end
    return n * factorial(n - 1)
end

result      = input_value * multiplier
fact_result = factorial(input_value)
message     = string.format(
    "%.0f * %.0f = %.0f  |  %.0f! = %.0f",
    input_value, multiplier, result, input_value, fact_result
)
"#
        )
        .unwrap();
    }

    let mut lua = Lua::new();

    // ── Pass values into the script via globals ───────────────────────────
    lua.set_global("input_value", 7i64).unwrap();
    lua.set_global("multiplier", 6i64).unwrap();

    // ── Load and execute ──────────────────────────────────────────────────
    let source = fs::read(&script_path).expect("failed to read script");
    lua.load(source)
        .set_name(format!("@{}", script_path.display()))
        .exec()
        .expect("script error");

    // ── Read results back into Rust ───────────────────────────────────────
    let result: f64 = lua.get_global("result").unwrap();
    let fact: f64 = lua.get_global("fact_result").unwrap();
    let msg: String = lua.get_global("message").unwrap();

    println!("{msg}");
    println!("  result      = {result}");
    println!("  7! in Rust  = {}", (1..=7u64).product::<u64>());
    println!("  7! from Lua = {fact}");

    // Sanity check
    assert_eq!(result as u64, 42);
    assert_eq!(fact as u64, 5040);
    println!("All assertions passed.");

    // Cleanup
    let _ = fs::remove_file(&script_path);
}
