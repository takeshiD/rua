//! table ライブラリ（本家 `ltablib.c` 相当）。担当: **lua-stdlib**。
//!
//! `insert`/`remove`/`concat`/`sort`/`maxn`/`getn`。`sort` の比較関数は Lua 関数を
//! VM 経由でコールバックする。

use crate::error::LuaResult;
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::convert::number_to_string;

use super::aux;

pub fn open(state: &mut LuaState) {
    let t = state.new_table();
    let tk = match t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };
    aux::register(state, tk, "insert", l_insert);
    aux::register(state, tk, "remove", l_remove);
    aux::register(state, tk, "concat", l_concat);
    aux::register(state, tk, "sort", l_sort);
    aux::register(state, tk, "maxn", l_maxn);
    aux::register(state, tk, "getn", l_getn);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "table", t);
    }
}

/// テーブルの border 長（`#t`）。
fn table_len(state: &LuaState, tk: TableKey) -> i64 {
    state
        .global
        .heap
        .get_table(tk)
        .map(|t| t.length())
        .unwrap_or(0) as i64
}

fn set_int(state: &mut LuaState, tk: TableKey, i: i64, v: Value) {
    if let Some(t) = state.global.heap.get_table_mut(tk) {
        let _ = t.set(Value::Number(i as f64), v);
    }
}

fn get_int(state: &LuaState, tk: TableKey, i: i64) -> Value {
    if i >= 1 {
        state
            .global
            .heap
            .get_table(tk)
            .map(|t| t.get_int(i as usize))
            .unwrap_or(Value::Nil)
    } else {
        state
            .global
            .heap
            .get_table(tk)
            .map(|t| t.get(&Value::Number(i as f64)))
            .unwrap_or(Value::Nil)
    }
}

// ============================================================================
// insert / remove
// ============================================================================

fn l_insert(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "insert")?;
    let n = table_len(state, tk);
    match args.len() {
        2 => {
            // table.insert(t, v): 末尾に追加。
            set_int(state, tk, n + 1, args[1]);
        }
        3 => {
            // table.insert(t, pos, v): pos に挿入し以降をシフト。
            let pos = aux::check_int(state, &args, 1, "insert")?;
            if pos < 1 || pos > n + 1 {
                return Err(aux::arg_error(state, 2, "insert", "position out of bounds"));
            }
            let mut i = n;
            while i >= pos {
                let v = get_int(state, tk, i);
                set_int(state, tk, i + 1, v);
                i -= 1;
            }
            set_int(state, tk, pos, args[2]);
        }
        _ => {
            return Err(aux::rt_error(
                state,
                "wrong number of arguments to 'insert'",
            ));
        }
    }
    aux::ret0(state)
}

fn l_remove(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "remove")?;
    let n = table_len(state, tk);
    let pos = aux::opt_int(state, &args, 1, "remove", n)?;
    if n == 0 && (args.len() < 2 || pos == 0) {
        // 空テーブル: nil を返す。
        return aux::ret(state, vec![Value::Nil]);
    }
    if n + 1 == pos {
        // 末尾の次を消す（実質 nil 返す）。
        let v = get_int(state, tk, pos);
        set_int(state, tk, pos, Value::Nil);
        return aux::ret(state, vec![v]);
    }
    if pos < 1 || pos > n + 1 {
        return Err(aux::arg_error(state, 2, "remove", "position out of bounds"));
    }
    let removed = get_int(state, tk, pos);
    let mut i = pos;
    while i < n {
        let v = get_int(state, tk, i + 1);
        set_int(state, tk, i, v);
        i += 1;
    }
    set_int(state, tk, n, Value::Nil);
    aux::ret(state, vec![removed])
}

// ============================================================================
// concat
// ============================================================================

fn l_concat(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "concat")?;
    let sep = if matches!(aux::opt_value(&args, 1), Value::Nil) {
        Vec::new()
    } else {
        aux::check_str_bytes(state, &args, 1, "concat")?
    };
    let i = aux::opt_int(state, &args, 2, "concat", 1)?;
    let j = if matches!(aux::opt_value(&args, 3), Value::Nil) {
        table_len(state, tk)
    } else {
        aux::check_int(state, &args, 3, "concat")?
    };
    let mut out: Vec<u8> = Vec::new();
    let mut idx = i;
    while idx <= j {
        let v = get_int(state, tk, idx);
        let piece = match v {
            Value::GcRef(GcHandle::Str(k)) => {
                state.global.heap.get_str(k).unwrap().as_bytes().to_vec()
            }
            Value::Number(num) => number_to_string(num).into_bytes(),
            other => {
                return Err(aux::rt_error(
                    state,
                    format!(
                        "invalid value (at index {idx}) in table for 'concat' ({})",
                        other.type_of().name()
                    ),
                ));
            }
        };
        out.extend_from_slice(&piece);
        if idx < j {
            out.extend_from_slice(&sep);
        }
        idx += 1;
    }
    let s = state.new_string(&out);
    aux::ret(state, vec![s])
}

// ============================================================================
// sort
// ============================================================================

fn l_sort(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "sort")?;
    let comp = aux::opt_value(&args, 1);
    let n = table_len(state, tk);
    // 要素を取り出す。
    let mut elems: Vec<Value> = Vec::with_capacity(n as usize);
    for i in 1..=n {
        elems.push(get_int(state, tk, i));
    }
    // マージソート（比較関数は失敗しうるので手書き）。
    let sorted = merge_sort(state, elems, comp)?;
    // 書き戻し。
    for (i, v) in sorted.into_iter().enumerate() {
        set_int(state, tk, i as i64 + 1, v);
    }
    aux::ret0(state)
}

/// `a < b` を Lua 比較関数 or 既定（lua_lt）で判定する。
fn less(state: &mut LuaState, comp: Value, a: Value, b: Value) -> LuaResult<bool> {
    if matches!(comp, Value::Nil) {
        aux::lua_lt(state, a, b)
    } else {
        let res = crate::vm::call(state, comp, &[a, b])?;
        Ok(res.into_iter().next().unwrap_or(Value::Nil).is_truthy())
    }
}

fn merge_sort(state: &mut LuaState, mut v: Vec<Value>, comp: Value) -> LuaResult<Vec<Value>> {
    let len = v.len();
    if len <= 1 {
        return Ok(v);
    }
    let mid = len / 2;
    let right = v.split_off(mid);
    let left = merge_sort(state, v, comp)?;
    let right = merge_sort(state, right, comp)?;
    // マージ。
    let mut out = Vec::with_capacity(len);
    let mut li = 0;
    let mut ri = 0;
    while li < left.len() && ri < right.len() {
        // 安定性: right[ri] < left[li] のときのみ right を先に出す。
        if less(state, comp, right[ri], left[li])? {
            out.push(right[ri]);
            ri += 1;
        } else {
            out.push(left[li]);
            li += 1;
        }
    }
    out.extend_from_slice(&left[li..]);
    out.extend_from_slice(&right[ri..]);
    Ok(out)
}

// ============================================================================
// maxn / getn
// ============================================================================

fn l_maxn(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "maxn")?;
    // 全キーを next で走査し、最大の正の数値キーを探す。
    let mut max = 0.0f64;
    let mut key = Value::Nil;
    loop {
        let nxt = state
            .global
            .heap
            .get_table(tk)
            .and_then(|t| t.next(&key).ok().flatten());
        match nxt {
            Some((k, _)) => {
                if let Value::Number(n) = k
                    && n > max
                {
                    max = n;
                }
                key = k;
            }
            None => break,
        }
    }
    aux::ret(state, vec![Value::Number(max)])
}

fn l_getn(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let tk = aux::check_table(state, &args, 0, "getn")?;
    let n = table_len(state, tk);
    aux::ret(state, vec![Value::Number(n as f64)])
}
