//! `rua-cli` の共有ライブラリ。
//!
//! 2 つのバイナリ（`rua` インタプリタ / `ruac` コンパイラ）が共通で使う
//! モジュール群を公開する。本家 `lua.c` / `luac.c` のフロントエンド相当。
//!
//! 実行フロー（`rua_core` の公開 API に結線）:
//!   LuaState::new() → stdlib::open_libs() → compiler::compile() → vm::run()

pub mod bytecode;
pub mod cli;
pub mod disasm;
pub mod luac;
pub mod repl;
pub mod run;
