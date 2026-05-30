//! アロケータ抽象（本家 `lmem.c` / `lua_Alloc` フック相当）。
//!
//! 本家 Lua は全メモリ確保を 1 つの `lua_Alloc` コールバックに集約し、組み込み側が
//! 差し替えられる。第一マイルストーンでは Rust の標準アロケータ（`SlotMap` 経由）に委ね、
//! 本モジュールは将来 C API（rua-capi）で `lua_Alloc` を受ける際の差し込み口を予約する。
//!
//! TODO(lua-runtime/lua-capi, 第二マイルストーン):
//!   - `lua_Alloc` 互換シグネチャ `fn(ud, ptr, osize, nsize) -> ptr` の橋渡し。
//!   - GC 起動閾値（本家 `g->GCthreshold`）に基づく自動コレクションのトリガ。

/// GC 起動方針の設定（本家 `global_State` の GC パラメータに相当）。
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// 自動 GC を行うか。false の間は明示 collect のみ。
    pub enabled: bool,
    /// 前回 collect 後、この確保回数を超えたら collect を検討する（暫定の単純閾値）。
    pub step_threshold: usize,
}

impl Default for GcConfig {
    fn default() -> Self {
        GcConfig {
            enabled: true,
            // 本家の GCpause/GCstepmul 相当は性能フェーズで再設計する。暫定値。
            step_threshold: 1024,
        }
    }
}
