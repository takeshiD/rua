//! Lua 文字列（本家 `lstring.c` / `TString` 相当）。
//!
//! Lua 文字列は **不変なバイト列**（任意の `\0` を含みうる）であり、インターンされる。
//! インターンは [`crate::gc::Heap::intern_str`] が担い、同値文字列は同一ハンドルになる。
//!
//! # バッファ安定性
//! 内容を `Box<[u8]>` で確保し、生成後は再確保しない。これにより将来の C API で
//! `lua_tolstring` が返す `const char*` のポインタ安定性（ARCHITECTURE.md §5）を満たす土台となる。
//!
//! TODO(lua-vm): メタテーブル（文字列共通メタテーブル）参照、`..` 連結や `string.*` 用の補助。

use std::hash::{Hash, Hasher};

/// インターン済み Lua 文字列の本体。
#[derive(Debug, Clone)]
pub struct LuaString {
    /// 不変バイト列。生成後に変更しない（安定バッファ）。
    bytes: Box<[u8]>,
    /// 事前計算ハッシュ（本家 `TString` も hash を保持）。テーブルキー等で再利用する。
    hash: u64,
}

impl LuaString {
    /// バイト列から生成する（通常は [`crate::gc::Heap::intern_str`] 経由で呼ばれる）。
    pub fn new(bytes: &[u8]) -> Self {
        LuaString {
            bytes: bytes.into(),
            hash: hash_bytes(bytes),
        }
    }

    /// 生のバイト列。
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 事前計算済みハッシュ。
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// バイト長（`#` 演算子や `string.len` の基礎）。
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// UTF-8 として妥当ならば `&str` を返す（表示・診断用途）。
    /// Lua 文字列は本来バイト列なので、不正 UTF-8 では `None`。
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }
}

/// 文字列ハッシュ。インターナのキー安定性のため決定的なハッシュを使う。
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
