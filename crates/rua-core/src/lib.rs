//! `rua-core` — Lua 5.1 実装のコア。
//!
//! モジュール構成は `docs/ARCHITECTURE.md §3`（本家ソース → rua モジュール対応表）に対応する。
//! フェーズ0（基盤）時点では各モジュールはインタフェース骨格＋最小スタブであり、
//! 実装の本体は担当エージェントが順次埋める：
//!   - `compiler`（lexer/parser/codegen） … lua-frontend
//!   - `vm`（opcode/interp）              … lua-vm（opcode は frontend と共有）
//!   - `value`（table/string/closure）    … lua-vm（型の骨格は lua-runtime が用意）
//!   - `gc`, `state`                      … lua-runtime（本モジュール群）
//!   - `stdlib`                           … lua-stdlib
//!
//! # 値モデルと GC（確定事項, ARCHITECTURE.md §5 案A）
//! GC オブジェクト（string/table/closure/userdata/thread）は型別アリーナに格納し、
//! [`value::Value`] は世代付きハンドル [`gc::GcHandle`] でそれらを参照する。
//! mark-and-sweep はアリーナ走査で実装するため `unsafe` を用いない。

pub mod compiler;
pub mod error;
pub mod gc;
pub mod state;
pub mod stdlib;
pub mod value;
pub mod vm;

pub use error::{LuaError, LuaResult};
pub use value::{LuaType, Value};
