//! Lua テーブル（本家 `ltable.c` / `Table` 相当）。担当: **lua-vm**。
//!
//! 配列部（連番整数キー）とハッシュ部（その他のキー）のハイブリッド構造。
//! 数値キーは Lua 5.1 ではすべて `double` だが、整数値かつ 1 始まり連番のものは配列部に格納し、
//! `#`（長さ）演算子の border 規則・`ipairs`・`table.insert` を効率化する。
//!
//! # キーの正規化（本家規則）
//! - `nil` キーでの代入はエラー（呼び出し側 VM が検出）。get では常に `nil`。
//! - `NaN` キーは代入エラー。
//! - 整数値の `number`（`floor(n)==n` かつ有限）は整数キーとして扱う。`2` と `2.0` は同一キー。
//! - `-0.0` は `0.0` に正規化する。
//! - 文字列キーはインターン済みハンドルで比較（同値 ⇔ 同一ハンドル）。

use std::collections::HashMap;

use crate::gc::{GcHandle, Trace, Tracer};
use crate::value::Value;

/// ハッシュ部のキー（[`Value`] を `Hash`/`Eq` 可能な形に正規化したもの）。
///
/// 配列部に入らない number は [`HKey::Number`] にビットパターンで格納する
/// （`-0.0`→`0.0` 正規化済み、`NaN` は格納しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HKey {
    Boolean(bool),
    /// 整数値でない / 範囲外の number。正規化済み f64 のビットパターン。
    Number(u64),
    LightUserData(usize),
    Gc(GcHandle),
}

/// 正規化済みハッシュキー [`HKey`] を [`Value`] へ戻す（`next` 反復用）。
fn hkey_to_value(hk: &HKey) -> Value {
    match hk {
        HKey::Boolean(b) => Value::Boolean(*b),
        HKey::Number(bits) => Value::Number(f64::from_bits(*bits)),
        HKey::LightUserData(p) => Value::LightUserData(*p as *mut std::os::raw::c_void),
        HKey::Gc(h) => Value::GcRef(*h),
    }
}

/// 代入・取得に使うキーの分類。
enum KeyClass {
    /// 1 始まりの正整数（配列部候補）。
    ArrayIndex(usize),
    /// ハッシュ部キー。
    Hash(HKey),
    /// 無効キー（`nil` / `NaN`）。
    Invalid,
}

/// number を配列インデックス（1 始まり usize）へ変換できれば返す。
fn num_to_array_index(n: f64) -> Option<usize> {
    if n.is_finite() && n.floor() == n && n >= 1.0 && n <= usize::MAX as f64 {
        Some(n as usize)
    } else {
        None
    }
}

/// number を正規化したビットパターンへ（`-0.0`→`0.0`）。`NaN` は `None`。
fn norm_num_bits(n: f64) -> Option<u64> {
    if n.is_nan() {
        None
    } else if n == 0.0 {
        Some(0.0f64.to_bits())
    } else {
        Some(n.to_bits())
    }
}

fn classify_key(key: &Value) -> KeyClass {
    match key {
        Value::Nil => KeyClass::Invalid,
        Value::Boolean(b) => KeyClass::Hash(HKey::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = num_to_array_index(*n) {
                KeyClass::ArrayIndex(i)
            } else {
                match norm_num_bits(*n) {
                    Some(bits) => KeyClass::Hash(HKey::Number(bits)),
                    None => KeyClass::Invalid,
                }
            }
        }
        Value::LightUserData(p) => KeyClass::Hash(HKey::LightUserData(*p as usize)),
        Value::GcRef(h) => KeyClass::Hash(HKey::Gc(*h)),
    }
}

/// Lua テーブル。
#[derive(Debug, Default)]
pub struct Table {
    /// 配列部（キー `1..=array.len()` を `array[i-1]` に格納）。末尾の `nil` は border 計算で扱う。
    array: Vec<Value>,
    /// ハッシュ部。配列部に入らないキー。
    hash: HashMap<HKey, Value>,
    /// メタテーブル（無ければ `None`）。
    metatable: Option<GcHandle>,
}

impl Table {
    pub fn new() -> Self {
        Table::default()
    }

    /// 配列部/ハッシュ部の初期容量を予約して作る（`NEWTABLE` のサイズヒント用）。
    pub fn with_capacity(narray: usize, nhash: usize) -> Self {
        Table {
            array: Vec::with_capacity(narray),
            hash: HashMap::with_capacity(nhash),
            metatable: None,
        }
    }

    /// メタテーブルを取得。
    pub fn metatable(&self) -> Option<GcHandle> {
        self.metatable
    }

    /// メタテーブルを設定。
    pub fn set_metatable(&mut self, mt: Option<GcHandle>) {
        self.metatable = mt;
    }

    // ---- raw get/set（メタメソッド非経由, 本家 `luaH_get`/`luaH_set`）-------

    /// キーに対応する値を返す（無ければ `nil`）。raw アクセス（`__index` 非経由）。
    pub fn get(&self, key: &Value) -> Value {
        match classify_key(key) {
            KeyClass::ArrayIndex(i) if i <= self.array.len() => self.array[i - 1],
            KeyClass::ArrayIndex(i) => {
                // 配列範囲外の整数キーはハッシュ部にあるかもしれない。
                self.hash
                    .get(&HKey::Number((i as f64).to_bits()))
                    .copied()
                    .unwrap_or(Value::Nil)
            }
            KeyClass::Hash(hk) => self.hash.get(&hk).copied().unwrap_or(Value::Nil),
            KeyClass::Invalid => Value::Nil,
        }
    }

    /// 整数キーでの取得（高速パス, `ipairs`/`#` 内部用）。
    pub fn get_int(&self, i: usize) -> Value {
        if i >= 1 && i <= self.array.len() {
            self.array[i - 1]
        } else if i >= 1 {
            self.hash
                .get(&HKey::Number((i as f64).to_bits()))
                .copied()
                .unwrap_or(Value::Nil)
        } else {
            Value::Nil
        }
    }

    /// キーに値を代入する（raw, `__newindex` 非経由）。`nil`/`NaN` キーは `Err` を返す。
    ///
    /// `value` が `nil` の場合は削除に相当（配列部は穴になりうる）。
    pub fn set(&mut self, key: Value, value: Value) -> Result<(), TableKeyError> {
        match classify_key(&key) {
            KeyClass::ArrayIndex(i) => {
                self.set_array_index(i, value);
                Ok(())
            }
            KeyClass::Hash(hk) => {
                if matches!(value, Value::Nil) {
                    self.hash.remove(&hk);
                } else {
                    self.hash.insert(hk, value);
                }
                Ok(())
            }
            KeyClass::Invalid => Err(if matches!(key, Value::Nil) {
                TableKeyError::NilKey
            } else {
                TableKeyError::NanKey
            }),
        }
    }

    /// 整数キーでの代入。配列部の伸長とハッシュ部からの巻き取りを行う。
    fn set_array_index(&mut self, i: usize, value: Value) {
        let len = self.array.len();
        if i <= len {
            self.array[i - 1] = value;
            return;
        }
        if i == len + 1 {
            if matches!(value, Value::Nil) {
                // 末尾の次に nil を書く: ハッシュ部にあれば消す、無ければ何もしない。
                self.hash.remove(&HKey::Number((i as f64).to_bits()));
                return;
            }
            // 配列末尾へ追加し、後続の整数キーをハッシュ部から巻き取る。
            self.array.push(value);
            let mut next = self.array.len() + 1;
            while let Some(v) = self.hash.remove(&HKey::Number((next as f64).to_bits())) {
                self.array.push(v);
                next += 1;
            }
            return;
        }
        // 連番でない整数キーはハッシュ部へ。
        let hk = HKey::Number((i as f64).to_bits());
        if matches!(value, Value::Nil) {
            self.hash.remove(&hk);
        } else {
            self.hash.insert(hk, value);
        }
    }

    /// `#`（長さ）演算子の border（本家 `luaH_getn`）。
    ///
    /// border `n` は `t[n] ~= nil` かつ `t[n+1] == nil` を満たす整数。配列部末尾が `nil` なら
    /// 配列部を二分探索、そうでなければハッシュ部を非有界探索する。
    pub fn length(&self) -> usize {
        let mut j = self.array.len();
        if j > 0 && matches!(self.array[j - 1], Value::Nil) {
            // 配列部に穴がある: array[i] != nil かつ array[j] == nil となる境界を二分探索。
            let mut i = 0;
            while j - i > 1 {
                let m = (i + j) / 2;
                if matches!(self.array[m - 1], Value::Nil) {
                    j = m;
                } else {
                    i = m;
                }
            }
            return i;
        }
        // 配列部は穴なし。ハッシュ部に続きがあるか非有界探索する。
        if self.hash.is_empty() {
            return j;
        }
        self.unbound_search(j)
    }

    /// ハッシュ部における border の非有界探索（本家 `unbound_search`）。
    fn unbound_search(&self, mut i: usize) -> usize {
        let mut j = i + 1;
        // t[j] != nil の間、区間を倍々で広げる。
        while !matches!(self.get_int(j), Value::Nil) {
            i = j;
            if j > usize::MAX / 2 {
                // 異常に大きい: 線形探索にフォールバック。
                let mut k = 1;
                while !matches!(self.get_int(k), Value::Nil) {
                    k += 1;
                }
                return k - 1;
            }
            j *= 2;
        }
        // [i, j) を二分探索。
        while j - i > 1 {
            let m = (i + j) / 2;
            if matches!(self.get_int(m), Value::Nil) {
                j = m;
            } else {
                i = m;
            }
        }
        i
    }

    // ---- 反復（pairs/next 用の補助）----------------------------------------

    /// `next(t, key)`（本家 `luaH_next` 相当）。`key` の次のキー/値ペアを返す。
    ///
    /// - `key == nil`: 最初のペアを返す。
    /// - `Ok(Some((k, v)))`: `key` の次のペア。
    /// - `Ok(None)`: もう要素が無い（反復終了）。
    /// - `Err(())`: `key` がテーブルに存在しない（本家 "invalid key to 'next'"）。
    ///
    /// 反復順は「配列部（昇順, `nil` を飛ばす）→ ハッシュ部（`HashMap` の反復順）」。
    /// 反復中にテーブルを変更しなければ順序は安定する（本家と同じ契約）。
    ///
    /// NOTE(lua-stdlib): `pairs`/`next`/`table.maxn` 実装のため lua-stdlib が追加した
    /// 補助メソッド。ハッシュ部反復の公開口が他に無いため。owner（lua-vm）レビュー希望。
    ///
    /// `Err(())` は「キー不在」のみを表す単純なシグナルのため、専用エラー型は設けない。
    #[allow(clippy::result_unit_err)]
    pub fn next(&self, key: &Value) -> Result<Option<(Value, Value)>, ()> {
        // 配列部の `idx`（0 始まり）以降で最初の非 nil を返すヘルパ。
        let first_array_from = |start: usize| -> Option<(Value, Value)> {
            for idx in start..self.array.len() {
                if !matches!(self.array[idx], Value::Nil) {
                    return Some((Value::Number((idx + 1) as f64), self.array[idx]));
                }
            }
            None
        };
        let first_hash = || -> Option<(Value, Value)> {
            self.hash.iter().next().map(|(k, v)| (hkey_to_value(k), *v))
        };

        match key {
            Value::Nil => Ok(first_array_from(0).or_else(first_hash)),
            _ => match classify_key(key) {
                KeyClass::ArrayIndex(i) if i <= self.array.len() => {
                    Ok(first_array_from(i).or_else(first_hash))
                }
                KeyClass::ArrayIndex(i) => {
                    // 配列範囲外の整数キー: ハッシュ部にある想定。
                    self.hash_next(HKey::Number((i as f64).to_bits()))
                }
                KeyClass::Hash(hk) => self.hash_next(hk),
                KeyClass::Invalid => Err(()),
            },
        }
    }

    /// ハッシュ部で `hk` の次のエントリを返す（`hk` 不在なら `Err`）。
    fn hash_next(&self, hk: HKey) -> Result<Option<(Value, Value)>, ()> {
        let mut found = false;
        for (k, v) in self.hash.iter() {
            if found {
                return Ok(Some((hkey_to_value(k), *v)));
            }
            if *k == hk {
                found = true;
            }
        }
        if found { Ok(None) } else { Err(()) }
    }

    /// 配列部への参照。
    pub fn array(&self) -> &[Value] {
        &self.array
    }

    /// 配列部への可変参照（GC テスト/低レベル構築用）。
    pub fn array_mut(&mut self) -> &mut Vec<Value> {
        &mut self.array
    }
}

/// テーブルキーの無効値エラー（VM がランタイムエラーへ昇格する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKeyError {
    /// `nil` をキーにしようとした。
    NilKey,
    /// `NaN` をキーにしようとした。
    NanKey,
}

impl Trace for Table {
    fn trace(&self, tracer: &mut Tracer) {
        for v in &self.array {
            tracer.mark_value(v);
        }
        for (k, v) in &self.hash {
            if let HKey::Gc(h) = k {
                tracer.mark(*h);
            }
            tracer.mark_value(v);
        }
        if let Some(mt) = self.metatable {
            tracer.mark(mt);
        }
    }
}
