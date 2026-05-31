//! 関数呼び出しプロトコル・エラー巻き戻し・コルーチン（本家 `ldo.c` 相当）。
//!
//! # エラー巻き戻し（本家 `setjmp/longjmp` → Rust `Result`）
//! 本家は保護呼び出し `lua_pcall` で `setjmp` し、`lua_error` の `longjmp` で巻き戻す。
//! rua では保護境界を [`pcall`] が表現し、内部の [`LuaResult`](crate::error::LuaResult) の
//! `Err` 伝播でスタックを巻き戻す。`Err` 検出時は呼び出し前のスタック深さへ復元する
//! （本家の `luaD_seterrorobj` + スタック復元に相当）。パニックは保護境界を越えさせない方針。
//!
//! # 本ファイルの状態
//! フェーズ0では呼び出し/巻き戻しの**骨格と契約**のみ定義する。命令ディスパッチや
//! 実際のフレーム積み下ろしは lua-vm（`vm::interp`）が実装し、ここから委譲する。
//!
//! TODO(lua-runtime/lua-vm):
//!   - `precall`/`postcall`（フレーム構築・戻り値整列, 本家 `luaD_precall`/`luaD_poscall`）。
//!   - コルーチン `resume`/`yield`（本家 `lua_resume`/`lua_yield`）。Rust では VM ループを
//!     再開可能な状態機械、もしくは別スタックで表現する（設計は VM 立ち上げ後に確定）。
//!   - スタックのオーバーフロー検査と伸長（本家 `luaD_growstack`）。

use crate::error::{LuaError, LuaResult};
use crate::state::LuaState;
use crate::value::Value;

/// 保護呼び出しの骨格（本家 `lua_pcall` 相当）。
///
/// `body` の実行中に発生した `Err` を捕捉し、呼び出し前のスタック深さへ巻き戻してから返す。
/// 実際の関数呼び出し（VM 実行）は `body` に閉じ込める想定で、命令ディスパッチは lua-vm が担う。
///
/// 戻り値: `Ok` は `body` の戻り値、`Err` は捕捉した Lua エラー（スタックは復元済み）。
pub fn pcall<F, R>(state: &mut LuaState, body: F) -> LuaResult<R>
where
    F: FnOnce(&mut LuaState) -> LuaResult<R>,
{
    let saved_depth = state.stack.len();
    let saved_ci = state.call_info.len();
    match body(state) {
        Ok(r) => Ok(r),
        Err(LuaError::Yield(vals)) => {
            // Yield は pcall を透過する（スタックを巻き戻さずそのまま上位へ伝播）。
            Err(LuaError::Yield(vals))
        }
        Err(e) => {
            state.stack.truncate(saved_depth);
            state.call_info.truncate(saved_ci);
            Err(e)
        }
    }
}

/// 値を 1 つ VM スタックに積む（呼び出し準備の最小ヘルパ）。
pub fn push(state: &mut LuaState, value: Value) {
    state.stack.push(value);
}

/// VM スタックから値を 1 つ取り出す。
pub fn pop(state: &mut LuaState) -> Option<Value> {
    state.stack.pop()
}
