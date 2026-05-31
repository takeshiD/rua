//! 標準ライブラリ（本家 `lbaselib.c` ほか `lib*.c` 相当）。担当: **lua-stdlib**。
//!
//! base（`print`/`type`/`pairs`/`ipairs`/`pcall`/`tostring` 等）、`string`/`table`/`math` の
//! 主要関数を [`NativeFn`](crate::state::NativeFn) として実装し、グローバル環境に登録する。
//! C API 非依存で内部 Rust API（[`crate::state`]）を用いる（ARCHITECTURE.md §9 フェーズ4）。
//!
//! # エントリ
//! CLI（`rua run`）は [`open_libs`] を呼んで全ライブラリを開く。個別に開く場合は
//! 各サブモジュールの `open` を使う。
//!
//! # 呼び出し規約（重要）
//! ネイティブ関数は `vm::interp::call_native` から呼ばれる。引数はスタックの
//! `call_info.last().base` 以降、戻り値は最上位に積んだ個数を返す。これらの操作は
//! [`aux`] のヘルパに集約している。
//!
//! # 既知の制限 / 他担当への依頼事項
//! - **文字列メソッド構文 `s:upper()`**: VM 側の文字列共有メタテーブル対応が必要
//!   （`interp::metatable_of` が文字列のメタテーブルを返すこと）。未対応のため当面は
//!   `string.upper(s)` 形式のみ動作する。→ lua-vm へ依頼。
//! - **`error(msg, level)` の位置前置**: ネイティブから呼び出し元の `source:line` を取れない
//!   （`CallInfo` に pc/line が無い）。当面メッセージは前置なしで送出。→ lua-runtime/lua-vm へ依頼。
//! - **`table.next` / ハッシュ部反復**: `pairs`/`next`/`table.maxn` のため
//!   `value::table::Table::next` を lua-stdlib が追加（owner: lua-vm のレビュー希望）。
//! - **`gmatch`**: ネイティブクロージャの upvalue を読む手段が無いため、状態テーブルに
//!   `__call` メタメソッドを付けて反復状態を保持する方式で実装した（upvalue API 不要）。

pub mod aux;
pub mod base;
pub mod coroutine_lib;
pub mod io_lib;
pub mod math_lib;
pub mod os_lib;
pub mod package_lib;
pub mod pattern;
pub mod string_lib;
pub mod table_lib;

use crate::state::LuaState;

/// 全標準ライブラリをグローバル環境へ開く（本家 `luaL_openlibs` 相当）。
///
/// CLI（`rua run`）が [`LuaState`] 初期化後に 1 回呼ぶ。
pub fn open_libs(state: &mut LuaState) {
    base::open(state);
    string_lib::open(state);
    table_lib::open(state);
    math_lib::open(state);
    io_lib::open(state);
    os_lib::open(state);
    package_lib::open(state);
    coroutine_lib::open(state);
}
