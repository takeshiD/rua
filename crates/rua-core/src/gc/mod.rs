//! ガベージコレクタ（本家 `lgc.c` 相当）— GC 案A: ハンドル/アリーナ方式（ARCHITECTURE.md §5）。
//!
//! # 設計
//! GC オブジェクト（string/table/closure/userdata、将来 thread）を **型別アリーナ**
//! （[`slotmap::SlotMap`]）に格納する。[`Value`](crate::value::Value) はオブジェクト本体ではなく
//! 世代付きハンドル [`GcHandle`] を保持する。ハンドルは [`Copy`] かつ型タグ（enum 判別子）を内包するため、
//! ハンドルだけで Lua の型判定ができ、本体へのデリファレンスを伴わない。
//!
//! # 安全性
//! 生ポインタを用いず、世代付きキーで dangling を排除する。`SlotMap` はスロット解放時に世代を進めるため、
//! 解放済みハンドルでの `get` は安全に `None` を返す。本モジュールに `unsafe` は無い。
//!
//! # 回収アルゴリズム（当面 stop-the-world mark-and-sweep）
//! 1. ルート集合（VM スタック + レジストリ + グローバル環境, [`crate::state`] が提供）から到達可能性を辿る。
//! 2. 各オブジェクトの [`Trace`] 実装で子ハンドルを灰色集合へ積む（tri-color の簡略版: 白/灰+黒）。
//! 3. 未マークのスロットを sweep（解放）。インターン文字列はインターナからも除去。
//!
//! インクリメンタル化・weak table・`__gc` finalizer は性能/互換フェーズで対応（TODO）。

pub mod alloc;

use slotmap::{SlotMap, new_key_type};
use std::collections::HashMap;

use crate::value::Value;
use crate::value::closure::Closure;
use crate::value::string::LuaString;
use crate::value::table::Table;
use crate::value::userdata::Userdata;

new_key_type! {
    /// 文字列アリーナ用キー（世代付き）。
    pub struct StringKey;
    /// テーブルアリーナ用キー（世代付き）。
    pub struct TableKey;
    /// クロージャ（Lua/ネイティブ関数）アリーナ用キー（世代付き）。
    pub struct ClosureKey;
    /// ユーザーデータアリーナ用キー（世代付き）。
    pub struct UserdataKey;
}

/// GC 管理オブジェクトへの参照。`Value::GcRef` が保持する。
///
/// enum 判別子が Lua の型タグを兼ねるため、本体をデリファレンスせずに型判定できる。
/// `Copy` なので VM スタック上を値として安価に移動できる。
///
/// TODO(lua-runtime): コルーチン実装時に `Thread(ThreadKey)` を追加する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GcHandle {
    Str(StringKey),
    Table(TableKey),
    Closure(ClosureKey),
    Userdata(UserdataKey),
}

/// 各 GC オブジェクトに付与するヘッダ（mark ビット）。本家 `GCObject` の `marked` に相当。
#[derive(Debug)]
struct GcBox<T> {
    /// mark フェーズで到達済みなら true。sweep 後に false へ戻す。
    marked: bool,
    value: T,
}

impl<T> GcBox<T> {
    fn new(value: T) -> Self {
        GcBox {
            marked: false,
            value,
        }
    }
}

/// オブジェクトが保持する子 GC ハンドルを列挙するためのトレイト（本家の伝搬マークに相当）。
///
/// 実装側は、自身が参照する [`Value`]／[`GcHandle`] を [`Tracer`] へ渡す。
pub trait Trace {
    fn trace(&self, tracer: &mut Tracer);
}

/// mark フェーズの灰色集合（worklist）。到達したハンドルを積む。
pub struct Tracer {
    gray: Vec<GcHandle>,
}

impl Tracer {
    /// ハンドルを灰色集合へ積む。
    pub fn mark(&mut self, handle: GcHandle) {
        self.gray.push(handle);
    }

    /// 値が GC 参照なら灰色集合へ積む。プリミティブ値は無視。
    pub fn mark_value(&mut self, value: &Value) {
        if let Value::GcRef(h) = value {
            self.gray.push(*h);
        }
    }
}

/// GC ヒープ。型別アリーナと文字列インターナを保持する。
///
/// `global_State`（[`crate::state::GlobalState`]）が 1 つ保持し、すべての GC オブジェクトを所有する。
#[derive(Default)]
pub struct Heap {
    strings: SlotMap<StringKey, GcBox<LuaString>>,
    tables: SlotMap<TableKey, GcBox<Table>>,
    closures: SlotMap<ClosureKey, GcBox<Closure>>,
    userdata: SlotMap<UserdataKey, GcBox<Userdata>>,
    /// 文字列インターナ: バイト列 → 既存キー。Lua 文字列は同値なら同一オブジェクト。
    /// （本家 `lstring.c` の文字列テーブルに相当）。
    interner: HashMap<Box<[u8]>, StringKey>,
    /// 直近の collect 以降に確保したオブジェクト数の目安（GC 起動閾値判定のたたき台）。
    alloc_count: usize,
}

impl Heap {
    pub fn new() -> Self {
        Heap::default()
    }

    // ---- 確保（allocate）-------------------------------------------------

    /// 文字列をインターンして確保する。既存の同値文字列があればそのハンドルを返す。
    ///
    /// インターンによりバイトバッファの安定性が保証されるため、将来の C API で
    /// `lua_tolstring` が返すポインタ安定性要件（ARCHITECTURE.md §5）を満たす土台になる。
    pub fn intern_str(&mut self, bytes: &[u8]) -> GcHandle {
        if let Some(&key) = self.interner.get(bytes) {
            return GcHandle::Str(key);
        }
        let s = LuaString::new(bytes);
        let key = self.strings.insert(GcBox::new(s));
        self.interner.insert(bytes.into(), key);
        self.alloc_count += 1;
        GcHandle::Str(key)
    }

    /// テーブルを確保する。
    pub fn alloc_table(&mut self, table: Table) -> GcHandle {
        let key = self.tables.insert(GcBox::new(table));
        self.alloc_count += 1;
        GcHandle::Table(key)
    }

    /// クロージャ（Lua/ネイティブ）を確保する。
    pub fn alloc_closure(&mut self, closure: Closure) -> GcHandle {
        let key = self.closures.insert(GcBox::new(closure));
        self.alloc_count += 1;
        GcHandle::Closure(key)
    }

    /// ユーザーデータを確保する。
    pub fn alloc_userdata(&mut self, ud: Userdata) -> GcHandle {
        let key = self.userdata.insert(GcBox::new(ud));
        self.alloc_count += 1;
        GcHandle::Userdata(key)
    }

    // ---- 参照（access）---------------------------------------------------
    //
    // ハンドルの型と一致しない get は `None`。世代不一致（解放済み）も `None`。

    pub fn get_str(&self, key: StringKey) -> Option<&LuaString> {
        self.strings.get(key).map(|b| &b.value)
    }

    pub fn get_table(&self, key: TableKey) -> Option<&Table> {
        self.tables.get(key).map(|b| &b.value)
    }

    pub fn get_table_mut(&mut self, key: TableKey) -> Option<&mut Table> {
        self.tables.get_mut(key).map(|b| &mut b.value)
    }

    pub fn get_closure(&self, key: ClosureKey) -> Option<&Closure> {
        self.closures.get(key).map(|b| &b.value)
    }

    pub fn get_closure_mut(&mut self, key: ClosureKey) -> Option<&mut Closure> {
        self.closures.get_mut(key).map(|b| &mut b.value)
    }

    pub fn get_userdata(&self, key: UserdataKey) -> Option<&Userdata> {
        self.userdata.get(key).map(|b| &b.value)
    }

    pub fn get_userdata_mut(&mut self, key: UserdataKey) -> Option<&mut Userdata> {
        self.userdata.get_mut(key).map(|b| &mut b.value)
    }

    /// 現在の生存オブジェクト総数（テスト/デバッグ用）。
    pub fn live_object_count(&self) -> usize {
        self.strings.len() + self.tables.len() + self.closures.len() + self.userdata.len()
    }

    // ---- 回収（mark-and-sweep）------------------------------------------

    /// ルート集合から到達不能なオブジェクトを回収する（stop-the-world）。
    ///
    /// `roots` は VM スタック・レジストリ・グローバル環境など、生存が保証された
    /// 全ハンドルの列。重複や無効ハンドルが混じっても安全（get が None を返す）。
    pub fn collect<I>(&mut self, roots: I)
    where
        I: IntoIterator<Item = GcHandle>,
    {
        // --- mark ---
        let mut tracer = Tracer {
            gray: roots.into_iter().collect(),
        };
        while let Some(handle) = tracer.gray.pop() {
            // 1) 当該スロットを黒化（marked=true）。既に黒なら無視（循環で停止）。
            // 2) 黒化後に改めて不変参照で trace し、子を灰色集合へ積む。
            //    set と trace を分離するのは、可変借用と不変借用の競合を避けるため。
            match handle {
                GcHandle::Str(k) => {
                    // 文字列は子を持たないので黒化のみ。
                    if let Some(b) = self.strings.get_mut(k) {
                        b.marked = true;
                    }
                }
                GcHandle::Table(k) => {
                    if mark_box(self.tables.get_mut(k))
                        && let Some(b) = self.tables.get(k)
                    {
                        b.value.trace(&mut tracer);
                    }
                }
                GcHandle::Closure(k) => {
                    if mark_box(self.closures.get_mut(k))
                        && let Some(b) = self.closures.get(k)
                    {
                        b.value.trace(&mut tracer);
                    }
                }
                GcHandle::Userdata(k) => {
                    if mark_box(self.userdata.get_mut(k))
                        && let Some(b) = self.userdata.get(k)
                    {
                        b.value.trace(&mut tracer);
                    }
                }
            }
        }

        // --- sweep ---
        // 白（!marked）のスロットを解放し、生存スロットの mark を白へ戻す。
        // 文字列の解放時はインターナからも対応エントリを除去する。
        let interner = &mut self.interner;
        self.strings.retain(|_k, b| {
            let keep = b.marked;
            b.marked = false;
            if !keep {
                interner.remove(b.value.as_bytes());
            }
            keep
        });
        self.tables.retain(|_k, b| sweep_box(b));
        self.closures.retain(|_k, b| sweep_box(b));
        self.userdata.retain(|_k, b| sweep_box(b));

        self.alloc_count = 0;
    }
}

/// `get_mut` の結果を黒化し、「今回新たに黒化した（= trace すべき）」なら true を返す。
fn mark_box<T>(slot: Option<&mut GcBox<T>>) -> bool {
    match slot {
        Some(b) if !b.marked => {
            b.marked = true;
            true
        }
        _ => false,
    }
}

/// sweep 用 retain 述語: 生存なら mark を白へ戻して保持、白なら破棄。
fn sweep_box<T>(b: &mut GcBox<T>) -> bool {
    let keep = b.marked;
    b.marked = false;
    keep
}
