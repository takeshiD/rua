//! package ライブラリ + `require`（本家 `loadlib.c` 相当・Lua サブセット）。担当: **lua-stdlib**。
//!
//! 本家 Lua 5.1 のモジュールシステムのうち、**純 Lua モジュール**の読み込みを実装する。
//! C ライブラリ（`.so`/`.dll`）の動的ロード（`package.loadlib` / C ローダ）は rua の
//! 安全 Rust 方針により非対応で、`package.cpath` は空・C ローダは未登録とする。
//!
//! ## 提供するもの
//! - グローバル `require(modname)` — モジュールを読み込み、戻り値（または `true`）を返す。
//! - `package` テーブル:
//!   - `package.loaded` — 読み込み済みモジュールのキャッシュ（`modname → 値`）。
//!   - `package.preload` — 事前登録ローダ表（`modname → function`）。
//!   - `package.loaders` — searcher（ローダ探索関数）の配列。`require` が順に呼ぶ。
//!   - `package.path` — Lua モジュールの検索パステンプレート（`?` をモジュール名で置換）。
//!   - `package.cpath` — C モジュール用（rua では空）。
//!   - `package.config` — パス構成文字列（dirsep/pathsep/置換文字など）。
//!
//! ## `require(modname)` の流れ（本家 `ll_require`）
//! 1. `package.loaded[modname]` が真値ならそれを返す（既読）。
//! 2. `package.loaders[1], [2], …` を順に `loader(modname)` で呼ぶ。
//!    - 関数を返したら、それがモジュールローダ。探索終了。
//!    - 文字列を返したらエラーメッセージとして蓄積し、次の searcher へ。
//! 3. どの searcher も関数を返さなければ `module 'x' not found:` エラー。
//! 4. ローダを `loader(modname)` で実行。非 nil を返したら `package.loaded[modname]` に格納。
//!    モジュールが値を設定しなかった場合は `true` を格納する。
//! 5. `package.loaded[modname]` を返す。

use std::rc::Rc;

use crate::compiler::compile;
use crate::error::LuaResult;
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::closure::{Closure, LuaClosure};

use super::aux;

/// `package.path` の既定値（カレントディレクトリ基準の純 Lua モジュール検索）。
const DEFAULT_PATH: &str = "./?.lua;./?/init.lua";

/// `package.config` の既定値（本家 5.1 と同様の 5 行）:
/// dirsep `/`、pathsep `;`、置換マーク `?`、実行マーク `!`、ignore マーク `-`。
const DEFAULT_CONFIG: &str = "/\n;\n?\n!\n-\n";

/// package ライブラリと `require` をグローバル環境へ登録する。
pub fn open(state: &mut LuaState) {
    let pkg = state.new_table();
    let pk = match pkg {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    let loaded = state.new_table();
    aux::set_field(state, pk, "loaded", loaded);

    let preload = state.new_table();
    aux::set_field(state, pk, "preload", preload);

    let path = state.new_string(DEFAULT_PATH.as_bytes());
    aux::set_field(state, pk, "path", path);

    // C モジュール非対応のため cpath は空。
    let cpath = state.new_string(b"");
    aux::set_field(state, pk, "cpath", cpath);

    let config = state.new_string(DEFAULT_CONFIG.as_bytes());
    aux::set_field(state, pk, "config", config);

    // package.loaders 配列（searcher を順に登録）。
    let loaders = state.new_table();
    if let Value::GcRef(GcHandle::Table(lk)) = loaders {
        let preload_searcher = aux::make_native(state, searcher_preload);
        let lua_searcher = aux::make_native(state, searcher_lua);
        if let Some(t) = state.global.heap.get_table_mut(lk) {
            let _ = t.set(Value::Number(1.0), preload_searcher);
            let _ = t.set(Value::Number(2.0), lua_searcher);
        }
    }
    aux::set_field(state, pk, "loaders", loaders);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "package", pkg);
        aux::register(state, g, "require", l_require);
    }
}

// ============================================================================
// 内部ヘルパ
// ============================================================================

/// グローバルから `package` テーブルのキーを取得する。
fn package_table(state: &mut LuaState) -> Option<TableKey> {
    let g = match state.global.globals {
        GcHandle::Table(k) => k,
        _ => return None,
    };
    let key = state.new_string(b"package");
    match state.global.heap.get_table(g).map(|t| t.get(&key)) {
        Some(Value::GcRef(GcHandle::Table(pk))) => Some(pk),
        _ => None,
    }
}

/// `package` テーブルのフィールド（テーブル型）のキーを取得する。
fn package_subtable(state: &mut LuaState, field: &str) -> Option<TableKey> {
    let pk = package_table(state)?;
    let key = state.new_string(field.as_bytes());
    match state.global.heap.get_table(pk).map(|t| t.get(&key)) {
        Some(Value::GcRef(GcHandle::Table(k))) => Some(k),
        _ => None,
    }
}

/// `package` テーブルのフィールド（文字列型）のバイト列を取得する。
fn package_str_field(state: &mut LuaState, field: &str) -> Option<Vec<u8>> {
    let pk = package_table(state)?;
    let key = state.new_string(field.as_bytes());
    match state.global.heap.get_table(pk).map(|t| t.get(&key)) {
        Some(Value::GcRef(GcHandle::Str(k))) => {
            state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    }
}

/// ソースをコンパイルしてメインチャンクのクロージャ値を作る（本家 `luaL_loadfile`/`loadbuffer` 相当）。
///
/// メインチャンクは upvalue を持たない。成功で関数値、失敗で構文エラーメッセージを返す。
fn load_chunk(state: &mut LuaState, src: &[u8], chunkname: &str) -> Result<Value, String> {
    match compile(&mut state.global.heap, src, chunkname) {
        Ok(proto) => {
            let closure = LuaClosure::new(Rc::new(proto));
            let h = state.global.heap.alloc_closure(Closure::Lua(closure));
            Ok(Value::GcRef(h))
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// モジュール名をファイル名素片へ変換する（`.` → dirsep）。
fn module_to_path(name: &[u8], dirsep: u8) -> Vec<u8> {
    name.iter()
        .map(|&b| if b == b'.' { dirsep } else { b })
        .collect()
}

// ============================================================================
// searcher（package.loaders の要素）
// ============================================================================

/// preload searcher（本家 `loader_preload`）。`package.preload[modname]` を返す。
fn searcher_preload(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let name = aux::check_str_bytes(state, &args, 0, "require")?;
    let Some(preload) = package_subtable(state, "preload") else {
        let msg = state.new_string(b"\n\tno field package.preload");
        return aux::ret(state, vec![msg]);
    };
    let key = state.new_string(&name);
    let loader = state
        .global
        .heap
        .get_table(preload)
        .map(|t| t.get(&key))
        .unwrap_or(Value::Nil);
    if matches!(loader, Value::GcRef(GcHandle::Closure(_))) {
        aux::ret(state, vec![loader])
    } else {
        let mut buf = b"\n\tno field package.preload['".to_vec();
        buf.extend_from_slice(&name);
        buf.extend_from_slice(b"']");
        let msg = state.new_string(&buf);
        aux::ret(state, vec![msg])
    }
}

/// Lua ファイル searcher（本家 `loader_Lua`）。`package.path` を辿りファイルを探す。
///
/// 見つかればコンパイルした関数を返す。見つからなければ試したパスを列挙したエラー文字列を返す。
/// ファイルは存在するが構文エラーの場合は本家同様ハードエラー（`Err`）を送出する。
fn searcher_lua(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let name = aux::check_str_bytes(state, &args, 0, "require")?;

    let path = package_str_field(state, "path").unwrap_or_else(|| DEFAULT_PATH.as_bytes().to_vec());
    let fname = module_to_path(&name, b'/');

    let mut errbuf: Vec<u8> = Vec::new();
    for template in path.split(|&b| b == b';') {
        if template.is_empty() {
            continue;
        }
        // テンプレート中の `?` をモジュールパスで置換。
        let mut filename: Vec<u8> = Vec::with_capacity(template.len() + fname.len());
        for &b in template {
            if b == b'?' {
                filename.extend_from_slice(&fname);
            } else {
                filename.push(b);
            }
        }
        let path_str = String::from_utf8_lossy(&filename).into_owned();
        match std::fs::read(&path_str) {
            Ok(src) => {
                let chunkname = format!("@{path_str}");
                return match load_chunk(state, &src, &chunkname) {
                    Ok(func) => aux::ret(state, vec![func]),
                    Err(e) => {
                        let modname = String::from_utf8_lossy(&name).into_owned();
                        Err(aux::rt_error(
                            state,
                            format!(
                                "error loading module '{modname}' from file '{path_str}':\n\t{e}"
                            ),
                        ))
                    }
                };
            }
            Err(_) => {
                errbuf.extend_from_slice(b"\n\tno file '");
                errbuf.extend_from_slice(&filename);
                errbuf.push(b'\'');
            }
        }
    }
    let msg = state.new_string(&errbuf);
    aux::ret(state, vec![msg])
}

// ============================================================================
// require
// ============================================================================

fn l_require(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let name = aux::check_str_bytes(state, &args, 0, "require")?;
    let name_val = state.new_string(&name);

    let Some(loaded_tk) = package_subtable(state, "loaded") else {
        return Err(aux::rt_error(state, "'package.loaded' is not a table"));
    };

    // 1. 既読チェック。真値ならそれを返す。
    let cached = state
        .global
        .heap
        .get_table(loaded_tk)
        .map(|t| t.get(&name_val))
        .unwrap_or(Value::Nil);
    if cached.is_truthy() {
        return aux::ret(state, vec![cached]);
    }

    // 2. package.loaders を順に呼んでローダを探す。
    let Some(loaders_tk) = package_subtable(state, "loaders") else {
        return Err(aux::rt_error(state, "'package.loaders' must be a table"));
    };

    let modname = String::from_utf8_lossy(&name).into_owned();
    let mut errmsg = format!("module '{modname}' not found:");
    let mut idx = 1usize;
    let loader = loop {
        let searcher = state
            .global
            .heap
            .get_table(loaders_tk)
            .map(|t| t.get_int(idx))
            .unwrap_or(Value::Nil);
        if matches!(searcher, Value::Nil) {
            return Err(aux::rt_error(state, errmsg));
        }
        let res = crate::vm::call(state, searcher, &[name_val])?;
        match res.into_iter().next().unwrap_or(Value::Nil) {
            f @ Value::GcRef(GcHandle::Closure(_)) => break f,
            Value::GcRef(GcHandle::Str(k)) => {
                if let Some(s) = state.global.heap.get_str(k) {
                    errmsg.push_str(&String::from_utf8_lossy(s.as_bytes()));
                }
            }
            _ => {}
        }
        idx += 1;
    };

    // 3. ローダを modname 引数で実行。
    let rets = crate::vm::call(state, loader, &[name_val])?;
    let modval = rets.into_iter().next().unwrap_or(Value::Nil);

    // 4. 非 nil の戻り値は package.loaded[modname] に格納。
    if !matches!(modval, Value::Nil)
        && let Some(t) = state.global.heap.get_table_mut(loaded_tk)
    {
        let _ = t.set(name_val, modval);
    }

    // 5. モジュールが値を設定しなかった場合は true を格納し、それを返す。
    let final_val = state
        .global
        .heap
        .get_table(loaded_tk)
        .map(|t| t.get(&name_val))
        .unwrap_or(Value::Nil);
    let result = if matches!(final_val, Value::Nil) {
        if let Some(t) = state.global.heap.get_table_mut(loaded_tk) {
            let _ = t.set(name_val, Value::Boolean(true));
        }
        Value::Boolean(true)
    } else {
        final_val
    };
    aux::ret(state, vec![result])
}
