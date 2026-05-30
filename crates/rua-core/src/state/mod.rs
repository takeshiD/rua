//! 実行状態（本家 `lstate.c` の `lua_State` / `global_State` 相当）。
//!
//! - [`GlobalState`]: 全スレッド（コルーチン）で共有する状態。GC ヒープ・レジストリ・
//!   グローバル環境（`_G`）・文字列インターナ・GC 設定を所有する（本家 `global_State`）。
//! - [`LuaState`]: 1 つの実行スレッド。VM スタックとコールスタック（[`CallInfo`]）を持つ
//!   （本家 `lua_State`）。コルーチンは複数の [`LuaState`] が 1 つの [`GlobalState`] を共有する。
//!
//! # GC ルート（ARCHITECTURE.md §5）
//! ルート集合 = 各スレッドの VM スタック上の生存値 + レジストリ + グローバル環境。
//! [`LuaState::collect_garbage`] がこれらを集めて [`Heap::collect`](crate::gc::Heap::collect) を呼ぶ。
//!
//! # 所有モデル（第一マイルストーン時点の決定）
//! 本マイルストーンでは C へ `lua_State*` を渡さないため、`lua_State` のポインタ安定性制約は無い。
//! よって `LuaState` が `GlobalState` を**直接所有**する単純構成を採る（Rust の借用検査で安全）。
//! 第二マイルストーン（C API）で `lua_State*` を不透明安定ポインタとして公開する際、
//! `Box`/ピン留め + 共有 `global_State`（`Rc`/`Arc`）へ再構成する。TODO(lua-runtime/lua-capi)。

pub mod call;

use crate::gc::Heap;
use crate::gc::GcHandle;
use crate::gc::alloc::GcConfig;
use crate::value::Value;
use crate::value::table::Table;

/// ネイティブ（Rust 実装）関数のシグネチャ（本家 `lua_CFunction` 相当）。
///
/// 引数・戻り値は VM スタック経由でやり取りする（本家と同じスタックプロトコル）。
/// 戻り値の `i32` は**スタックに積んだ戻り値の個数**（本家 C function の戻り値規約と同じ）。
/// エラーは `Err(LuaError)` で送出し、保護境界（`call::pcall`）まで巻き戻す。
///
/// TODO(lua-vm/lua-stdlib): 引数アクセス用のヘルパ API（`L.arg(n)` 等）を整備する。
pub type NativeFn = fn(&mut LuaState) -> crate::error::LuaResult<i32>;

/// コールスタックのフレーム（本家 `CallInfo` 相当）。
///
/// 関数呼び出しごとに 1 つ積まれ、スタック上の関数位置・ベース・戻り先などを記録する。
/// 中身は VM 実装に合わせて拡張する。TODO(lua-vm/lua-runtime): pc・期待戻り値数・可変長引数情報。
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// このフレームのスタックベース（最初のローカル/引数のインデックス）。
    pub base: usize,
    /// 呼び出された関数値のスタック位置。
    pub func: usize,
    /// 期待される戻り値の数（`LUA_MULTRET` 相当は将来表現）。
    pub expected_results: usize,
    /// 実行中プロトタイプの整形済みソース名（本家 `short_src`）。ネイティブ関数フレームは `None`。
    /// `error()` の level 指定によるエラー位置付与（`luaL_where` 相当）に用いる。
    pub source: Option<String>,
    /// このフレームで現在（または直近のサブ呼び出し時点で）実行中の命令のソース行。
    /// Lua フレームでは VM が逐次更新する。ネイティブ関数フレームは 0。
    pub current_line: u32,
}

/// 全スレッド共有の状態（本家 `global_State`）。
pub struct GlobalState {
    /// GC ヒープ（全 GC オブジェクトの所有者, 文字列インターナ含む）。
    pub heap: Heap,
    /// レジストリ（C/内部用の隠しテーブル, 本家 `LUA_REGISTRYINDEX`）。常時 GC ルート。
    pub registry: GcHandle,
    /// グローバル環境テーブル `_G`（本家 `l_gt` / グローバルテーブル）。常時 GC ルート。
    pub globals: GcHandle,
    /// 文字列型の共有メタテーブル（本家 `global_State.mt[LUA_TSTRING]`）。
    ///
    /// `("s"):upper()` のような文字列値へのインデックス/メソッド呼び出しで VM が参照する。
    /// 本体（通常 `{ __index = string }`）の登録は lua-stdlib が `string` ライブラリを開く際に行う。
    /// `Some` の間は常時 GC ルート。
    pub string_metatable: Option<GcHandle>,
    /// GC 起動設定。
    pub gc_config: GcConfig,
}

impl GlobalState {
    /// レジストリとグローバル環境を確保した初期 `global_State` を作る。
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let registry = heap.alloc_table(Table::new());
        let globals = heap.alloc_table(Table::new());
        GlobalState {
            heap,
            registry,
            globals,
            string_metatable: None,
            gc_config: GcConfig::default(),
        }
    }
}

impl Default for GlobalState {
    fn default() -> Self {
        GlobalState::new()
    }
}

/// 1 実行スレッド（本家 `lua_State`）。VM スタックとコールスタックを持つ。
pub struct LuaState {
    /// 共有グローバル状態。第一マイルストーンでは直接所有（上記「所有モデル」参照）。
    pub global: GlobalState,
    /// VM 値スタック（レジスタ機械のレジスタ領域）。
    pub stack: Vec<Value>,
    /// コールスタック（本家の CallInfo 配列）。
    pub call_info: Vec<CallInfo>,
}

impl LuaState {
    /// 新しいメインスレッドを生成する。
    pub fn new() -> Self {
        LuaState {
            global: GlobalState::new(),
            stack: Vec::new(),
            call_info: Vec::new(),
        }
    }

    /// バイト列をインターンして Lua 文字列値を得る（よく使うため state 経由のショートカット）。
    pub fn new_string(&mut self, bytes: &[u8]) -> Value {
        Value::GcRef(self.global.heap.intern_str(bytes))
    }

    /// 新しいテーブルを確保して値を返す。
    pub fn new_table(&mut self) -> Value {
        Value::GcRef(self.global.heap.alloc_table(Table::new()))
    }

    /// このスレッドの現在の GC ルート集合を列挙する。
    ///
    /// ルート = レジストリ + グローバル環境 + VM スタック上の全 GC 値。
    /// TODO(lua-runtime): コルーチン対応時は全生存スレッドのスタックを合算する。
    pub fn roots(&self) -> Vec<GcHandle> {
        let mut roots = Vec::with_capacity(self.stack.len() + 3);
        roots.push(self.global.registry);
        roots.push(self.global.globals);
        if let Some(mt) = self.global.string_metatable {
            roots.push(mt);
        }
        for v in &self.stack {
            if let Value::GcRef(h) = v {
                roots.push(*h);
            }
        }
        roots
    }

    /// ルート集合から到達不能なオブジェクトを回収する（stop-the-world）。
    pub fn collect_garbage(&mut self) {
        let roots = self.roots();
        self.global.heap.collect(roots);
    }
}

impl Default for LuaState {
    fn default() -> Self {
        LuaState::new()
    }
}
