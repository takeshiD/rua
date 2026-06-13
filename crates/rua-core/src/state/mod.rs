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

use std::rc::Rc;

use crate::gc::Heap;
use crate::gc::alloc::GcConfig;
use crate::gc::{ClosureKey, GcHandle};
use crate::value::Value;
use crate::value::closure::{Closure, Upvalue};
use crate::value::table::Table;
use crate::vm::proto::Proto;

/// ネイティブ（Rust 実装）関数のシグネチャ（本家 `lua_CFunction` 相当）。
///
/// 引数・戻り値は VM スタック経由でやり取りする（本家と同じスタックプロトコル）。
/// 戻り値の `i32` は**スタックに積んだ戻り値の個数**（本家 C function の戻り値規約と同じ）。
/// エラーは `Err(LuaError)` で送出し、保護境界（`call::pcall`）まで巻き戻す。
///
/// TODO(lua-vm/lua-stdlib): 引数アクセス用のヘルパ API（`L.arg(n)` 等）を整備する。
pub type NativeFn = fn(&mut LuaState) -> crate::error::LuaResult<i32>;

/// コルーチン yield/resume 時に Lua フレームの実行状態を保存するための構造体。
///
/// `execute` ループがコルーチン yield を検出した際、次回 resume で再開するために
/// その時点のローカル変数（pc・proto・upvalue・vararg・open upvalue・スタックトップ）を
/// この構造体へ退避する。resume 時に `vm::interp::resume_execute` が読み取って復元する。
#[derive(Debug, Clone)]
pub struct LuaFrameState {
    /// yield を発生させた CALL 命令の pc（proto.code のインデックス）。
    /// resume 時にこの命令を再読みして結果レジスタを特定し、resume 引数を配置する。
    pub resume_call_pc: usize,
    /// 現在実行中のプロトタイプ。
    pub proto: Rc<Proto>,
    /// このフレームの upvalue 群。
    pub upvals: Vec<Upvalue>,
    /// このフレームの可変長引数。
    pub varargs: Vec<Value>,
    /// open upvalue リスト（絶対スタックインデックス → Upvalue セル）。
    pub open: Vec<(usize, Upvalue)>,
    /// 多値操作で動くスタックトップ（絶対インデックス）。
    pub top: usize,
    /// このフレームの関数環境テーブル（Lua 5.1 の fenv）。
    pub env: crate::gc::GcHandle,
}

/// コールスタックのフレーム（本家 `CallInfo` 相当）。
///
/// 関数呼び出しごとに 1 つ積まれ、スタック上の関数位置・ベース・戻り先などを記録する。
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// このフレームのスタックベース（最初のローカル/引数のインデックス）。
    pub base: usize,
    /// 呼び出された関数値のスタック位置。
    pub func: usize,
    /// 期待される戻り値の数（`LUA_MULTRET` 相当は将来表現）。
    pub expected_results: usize,
    /// 実行中プロトタイプの整形済みソース名（本家 `short_src`）。ネイティブ関数フレームは `None`。
    pub source: Option<String>,
    /// このフレームで現在（または直近のサブ呼び出し時点で）実行中の命令のソース行。
    /// Lua フレームでは VM が逐次更新する。ネイティブ関数フレームは 0。
    pub current_line: u32,
    /// ネイティブクロージャフレームの場合、実行中のクロージャのヒープキーを保持する。
    pub native_closure: Option<crate::gc::ClosureKey>,
    /// コルーチン yield 時に保存する Lua フレームの実行状態。
    /// Lua クロージャフレームが yield で中断されたときのみ `Some`。
    pub lua_frame: Option<Box<LuaFrameState>>,
    /// このフレームが Lua クロージャである場合の関数環境テーブルハンドル。
    /// `setfenv`/`getfenv` がレベル指定でフレームを辿るために使う。
    /// ネイティブ関数フレームは `None`。
    pub env: Option<crate::gc::GcHandle>,
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

    // -------------------------------------------------------------------------
    // ネイティブクロージャ / upvalue アクセス（第二マイルストーン C API 対応）
    // -------------------------------------------------------------------------

    /// 現在実行中のネイティブクロージャのヒープキーを返す。
    ///
    /// コールスタック末尾のフレームが [`CallInfo::native_closure`] を持っていれば返す。
    /// Lua クロージャフレームや `__call` 経由の場合は `None`。
    ///
    /// # 主な用途（lua-capi）
    /// `c_trampoline` の中から `state.current_native_closure()` でキーを得て
    /// `c_functions: HashMap<ClosureKey, _>` を引き、登録済み C 関数と upvalue を取り出す。
    ///
    /// ```ignore
    /// // lua-capi 内のイメージ（rua-capi/src/lib.rs）
    /// fn c_trampoline(state: &mut LuaState) -> LuaResult<i32> {
    ///     let key = state.current_native_closure()
    ///         .ok_or_else(|| LuaError::Internal("no native closure".into()))?;
    ///     // CapiState は LuaState をラップしているため、外から c_functions を参照する。
    ///     // 実際の取り出しは CapiState 側で行う。
    ///     ...
    /// }
    /// ```
    pub fn current_native_closure(&self) -> Option<ClosureKey> {
        self.call_info.last()?.native_closure
    }

    /// 現在実行中のネイティブクロージャの `i` 番目の upvalue を返す（0-origin）。
    ///
    /// 本家 `lua_upvalueindex(i)` の内部実装補助。
    /// コールスタック末尾のフレームがネイティブクロージャであり、
    /// そのクロージャがインデックス `i` の upvalue を持つ場合に値を返す。
    /// それ以外（Lua フレーム、upvalue 範囲外）は `None`。
    ///
    /// ```ignore
    /// // stdlib 関数内での使用例
    /// fn my_native(state: &mut LuaState) -> LuaResult<i32> {
    ///     let upv0 = state.current_upvalue(0).unwrap_or(Value::Nil);
    ///     // upv0 を使って処理...
    ///     Ok(0)
    /// }
    /// ```
    pub fn current_upvalue(&self, i: usize) -> Option<Value> {
        let key = self.current_native_closure()?;
        match self.global.heap.get_closure(key)? {
            Closure::Native(nc) => nc.upvalue(i).copied(),
            Closure::Lua(_) => None,
        }
    }

    /// 現在実行中のネイティブクロージャが持つ upvalue の個数を返す。
    ///
    /// ネイティブクロージャフレームでなければ `0` を返す。
    pub fn current_upvalue_count(&self) -> usize {
        let Some(key) = self.current_native_closure() else {
            return 0;
        };
        match self.global.heap.get_closure(key) {
            Some(Closure::Native(nc)) => nc.upvalue_count(),
            _ => 0,
        }
    }

    /// `level` 段上のコールフレーム（Lua クロージャフレーム）の環境テーブルを返す。
    ///
    /// level 0 = スレッドのグローバル環境（`state.global.globals`）。
    /// level 1 = 現在の関数（`setfenv`/`getfenv` からの呼び出し元が Lua 関数のとき）。
    /// call_info の末尾から数えて `level` 段目の Lua フレームを探す。
    ///
    /// 本家の動作に合わせ、ネイティブフレームはスキップする。
    pub fn fenv_at_level(&self, level: usize) -> Option<GcHandle> {
        if level == 0 {
            return Some(self.global.globals);
        }
        let n = self.call_info.len();
        // 末尾（= 現在のネイティブフレーム）からスキップしてレベルを数える。
        let mut lua_count = 0usize;
        for i in (0..n).rev() {
            let ci = &self.call_info[i];
            if ci.env.is_some() {
                // Lua フレーム
                lua_count += 1;
                if lua_count == level {
                    return ci.env;
                }
            }
        }
        None
    }

    /// `level` 段上の Lua クロージャフレームの環境テーブルを `new_env` に差し替える。
    /// level 0 はスレッドのグローバル環境（`state.global.globals`）を差し替える。
    /// 対象フレームが見つからなければ false を返す。
    pub fn set_fenv_at_level(&mut self, level: usize, new_env: GcHandle) -> bool {
        if level == 0 {
            self.global.globals = new_env;
            return true;
        }
        let n = self.call_info.len();
        let mut lua_count = 0usize;
        for i in (0..n).rev() {
            let ci = &self.call_info[i];
            if ci.env.is_some() {
                lua_count += 1;
                if lua_count == level {
                    self.call_info[i].env = Some(new_env);
                    return true;
                }
            }
        }
        false
    }
}

impl Default for LuaState {
    fn default() -> Self {
        LuaState::new()
    }
}
