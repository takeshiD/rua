//! 仮想マシン（本家 `lvm.c` / `lopcodes.h` 相当）。担当: **lua-vm**（opcode は frontend と共有）。
//!
//! - [`opcode`][]: レジスタ型バイトコードの命令定義。lua-frontend（codegen）と lua-vm（interp）が共有。
//! - [`proto`][]: 関数プロトタイプ（命令列・定数表・デバッグ情報）。frontend と共有。
//! - [`interp`][]: 命令ディスパッチループ本体。

pub mod interp;
pub mod opcode;
pub mod proto;

pub use interp::{call, resume_execute, run, set_string_metatable, string_metatable, where_string};
pub use proto::Proto;
