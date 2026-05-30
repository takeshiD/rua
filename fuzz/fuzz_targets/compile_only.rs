//! パーサ/コンパイラ単体のファジングターゲット（lexer→parser→codegen）。
//! 実行はせず、フロントエンドのパニック/abort のみを狙う。
//!
//! 実行: `cargo +nightly fuzz run compile_only`
#![no_main]

use libfuzzer_sys::fuzz_target;

use rua_core::compiler::compile;
use rua_core::state::LuaState;

fuzz_target!(|data: &[u8]| {
    let mut state = LuaState::new();
    // コンパイルエラーは正常（Err）。パニックのみがバグ。
    let _ = compile(&mut state.global.heap, data, "=fuzz");
});
