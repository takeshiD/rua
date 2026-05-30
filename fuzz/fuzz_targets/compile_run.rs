//! コンパイル→実行のファジングターゲット。任意バイト列を Lua チャンクとして
//! コンパイルし、成功したら VM で実行する。**パニック/abort はすべて互換性バグ**。
//!
//! 実行: `cargo +nightly fuzz run compile_run`
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::state::LuaState;
use rua_core::stdlib;
use rua_core::vm::run;

fuzz_target!(|data: &[u8]| {
    let mut state = LuaState::new();
    stdlib::open_libs(&mut state);

    // コンパイルエラー/実行時エラーは正常な結果（Err）。パニックのみがバグ。
    if let Ok(proto) = compile(&mut state.global.heap, data, "=fuzz") {
        let _ = run(&mut state, Rc::new(proto), &[]);
    }
});
