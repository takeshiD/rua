//! フロントエンド（本家 `llex.c` / `lparser.c` / `lcode.c` 相当）。担当: **lua-frontend**。
//!
//! ソース文字列 → トークン列（[`lexer`]）→ AST/直接コード生成（[`parser`]）→
//! バイトコード `Proto`（[`codegen`]）の流れで、本家 `luac` 相当のバイトコードを生成する。
//!
//! フェーズ0（基盤）では空の骨格のみ。実装は lua-frontend が担当（ARCHITECTURE.md §9 フェーズ1）。

pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod parser;

use crate::error::LuaResult;
use crate::gc::Heap;
use crate::vm::proto::Proto;

use codegen::CodeGen;
use parser::Parser;

/// ソース文字列とチャンク名から Lua チャンクをコンパイルし、main 関数の [`Proto`] を返す。
///
/// これがフロントエンドの公開エントリ（CLI/VM から呼ぶ）。文字列定数のインターンに
/// [`Heap`] を要する（[`Proto::constants`] が [`crate::value::Value`] 型で、Lua 文字列は
/// GC 管理のため）。構文エラーは `<chunk>:<line>: <msg>` 形式（本家準拠）で返す。
///
/// `chunkname` は本家 `luaL_loadbuffer`/`lua_load` 同様、`@file`（ファイル）・`=name`
/// （表示名そのまま）・その他（`[string "..."]` 形式）の規約に従う。エラー/デバッグ表示には
/// [`chunk_id`] で短縮した名前を用いる。
pub fn compile(heap: &mut Heap, src: &[u8], chunkname: &str) -> LuaResult<Proto> {
    let id = chunk_id(chunkname);
    let block = Parser::parse(src, id.clone())?;
    // NOTE(lua-stdlib→lua-frontend): `Proto::source` には **生のチャンク名**（`@file` 等）を
    // 渡す。VM 側（`interp::short_src` / `CallInfo.source`）が表示時に短縮するため、ここで
    // 短縮済み `id` を渡すとネストした関数の source が二重短縮され `[string "..."]` になる
    // （`error()`/`assert()` の位置前置や tracebackが不正になる）。main も nested も生名で統一。
    let proto = CodeGen::compile(heap, &block, chunkname)?;
    Ok(proto)
}

/// 本家 `luaO_chunkid` 相当のチャンク名短縮（エラー/トレースバック表示用）。
pub fn chunk_id(source: &str) -> String {
    const MAX_LEN: usize = 60;
    if let Some(rest) = source.strip_prefix('=') {
        // `=name`: そのまま（長ければ切り詰め）。
        rest.chars().take(MAX_LEN).collect()
    } else if let Some(rest) = source.strip_prefix('@') {
        // `@file`: ファイル名。長ければ先頭を `...` で省略。
        if rest.len() > MAX_LEN - 3 {
            format!("...{}", &rest[rest.len() - (MAX_LEN - 3)..])
        } else {
            rest.to_string()
        }
    } else {
        // その他: `[string "first line..."]`。
        let first_line = source.split(['\n', '\r']).next().unwrap_or("");
        let budget = MAX_LEN - "[string \"...\"]".len();
        if first_line.len() > budget || source.contains(['\n', '\r']) {
            let take: String = first_line.chars().take(budget).collect();
            format!("[string \"{take}...\"]")
        } else {
            format!("[string \"{first_line}\"]")
        }
    }
}
