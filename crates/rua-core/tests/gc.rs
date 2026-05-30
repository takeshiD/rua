//! GC（案A: ハンドル/アリーナ方式）の基本動作テスト。
//!
//! mark-and-sweep の到達可能性・循環参照回収・文字列インターン・解放後の安全性を検証する。

use rua_core::gc::{GcHandle, Heap};
use rua_core::value::Value;
use rua_core::value::table::Table;

#[test]
fn intern_returns_same_handle_for_equal_bytes() {
    let mut heap = Heap::new();
    let a = heap.intern_str(b"hello");
    let b = heap.intern_str(b"hello");
    let c = heap.intern_str(b"world");
    assert_eq!(a, b, "同値文字列は同一ハンドルになるべき");
    assert_ne!(a, c);
    assert_eq!(heap.live_object_count(), 2);
}

#[test]
fn sweep_collects_unreachable_objects() {
    let mut heap = Heap::new();
    let _orphan = heap.intern_str(b"garbage");
    let kept = heap.intern_str(b"kept");
    assert_eq!(heap.live_object_count(), 2);

    // ルートに kept のみを渡す → orphan は回収される。
    heap.collect([kept]);
    assert_eq!(heap.live_object_count(), 1);

    // 生存ハンドルは依然有効。
    let GcHandle::Str(k) = kept else { panic!() };
    assert_eq!(heap.get_str(k).unwrap().as_bytes(), b"kept");
}

#[test]
fn reachable_through_table_survives() {
    let mut heap = Heap::new();
    let s = heap.intern_str(b"value");
    let mut t = Table::new();
    t.array_mut().push(Value::GcRef(s));
    let table = heap.alloc_table(t);

    // ルート = table のみ。s は table 経由で到達可能なので生存すべき。
    heap.collect([table]);
    assert_eq!(heap.live_object_count(), 2);
}

#[test]
fn cycles_are_collected() {
    let mut heap = Heap::new();
    // t1 -> t2 -> t1 の循環を作り、どちらもルート外にする。
    let t1 = heap.alloc_table(Table::new());
    let t2 = heap.alloc_table(Table::new());
    let GcHandle::Table(k1) = t1 else { panic!() };
    let GcHandle::Table(k2) = t2 else { panic!() };
    heap.get_table_mut(k1).unwrap().array_mut().push(Value::GcRef(t2));
    heap.get_table_mut(k2).unwrap().array_mut().push(Value::GcRef(t1));
    assert_eq!(heap.live_object_count(), 2);

    // ルート空 → 循環していても両方回収される（参照カウントでは漏れるケース）。
    heap.collect(std::iter::empty());
    assert_eq!(heap.live_object_count(), 0);
}

#[test]
fn stale_handle_access_is_safe() {
    let mut heap = Heap::new();
    let s = heap.intern_str(b"temp");
    let GcHandle::Str(k) = s else { panic!() };
    heap.collect(std::iter::empty()); // s を回収
    // 解放済みハンドルでの get は panic せず None（世代不一致 / 空スロット）。
    assert!(heap.get_str(k).is_none());
}
