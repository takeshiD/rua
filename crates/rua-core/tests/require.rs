//! `require` / `package` ライブラリの動作テスト。
//!
//! `open_libs` 後の `require` を、コンパイル→`vm::run` でメインチャンクを実行する形で
//! エンドツーエンドに検証する（preload ローダ・Lua ファイル探索・キャッシュ・未検出エラー）。

use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::gc::GcHandle;
use rua_core::state::LuaState;
use rua_core::stdlib;
use rua_core::value::Value;
use rua_core::vm;

fn new_state() -> LuaState {
    let mut s = LuaState::new();
    stdlib::open_libs(&mut s);
    s
}

/// ソース文字列をコンパイルしてメインチャンクとして実行し、戻り値列を返す。
fn run_src(state: &mut LuaState, src: &str) -> Vec<Value> {
    let proto = compile(&mut state.global.heap, src.as_bytes(), "=test").expect("compile ok");
    vm::run(state, Rc::new(proto), &[]).expect("run ok")
}

fn as_num(v: Value) -> f64 {
    match v {
        Value::Number(n) => n,
        _ => panic!("expected number, got {v:?}"),
    }
}

fn as_string(state: &LuaState, v: Value) -> String {
    match v {
        Value::GcRef(GcHandle::Str(k)) => {
            String::from_utf8_lossy(state.global.heap.get_str(k).unwrap().as_bytes()).into_owned()
        }
        _ => panic!("expected string, got {v:?}"),
    }
}

/// テスト専用のユニークな一時ディレクトリ。
fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "rua_require_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ))
}

#[test]
fn package_table_has_expected_fields() {
    let mut s = new_state();
    let r = run_src(
        &mut s,
        r#"
        return type(package), type(package.loaded), type(package.preload),
               type(package.loaders), type(package.path), type(package.config),
               type(require)
    "#,
    );
    assert_eq!(as_string(&s, r[0]), "table"); // package
    assert_eq!(as_string(&s, r[1]), "table"); // loaded
    assert_eq!(as_string(&s, r[2]), "table"); // preload
    assert_eq!(as_string(&s, r[3]), "table"); // loaders
    assert_eq!(as_string(&s, r[4]), "string"); // path
    assert_eq!(as_string(&s, r[5]), "string"); // config
    assert_eq!(as_string(&s, r[6]), "function"); // require
}

#[test]
fn require_via_preload_and_cache() {
    let mut s = new_state();
    let r = run_src(
        &mut s,
        r#"
        local calls = 0
        package.preload["mymod"] = function(name)
            calls = calls + 1
            return { id = name, value = 42 }
        end
        local m = require("mymod")
        local m2 = require("mymod")
        return m.value, m.id, (m == m2), calls
    "#,
    );
    assert_eq!(as_num(r[0]), 42.0);
    assert_eq!(as_string(&s, r[1]), "mymod");
    assert!(
        matches!(r[2], Value::Boolean(true)),
        "cached module identity"
    );
    // ローダは一度だけ呼ばれる（2 回目はキャッシュ）。
    assert_eq!(as_num(r[3]), 1.0);
}

#[test]
fn require_preload_without_return_uses_true() {
    let mut s = new_state();
    let r = run_src(
        &mut s,
        r#"
        package.preload["sideeffect"] = function() _G.loaded_flag = true end
        local m = require("sideeffect")
        return m, package.loaded["sideeffect"], _G.loaded_flag
    "#,
    );
    // 戻り値なしのモジュールは true がキャッシュ・返却される。
    assert!(matches!(r[0], Value::Boolean(true)));
    assert!(matches!(r[1], Value::Boolean(true)));
    assert!(matches!(r[2], Value::Boolean(true)));
}

#[test]
fn require_lua_file_from_path() {
    let dir = unique_temp_dir("file");
    std::fs::create_dir_all(&dir).unwrap();
    let modfile = dir.join("mathx.lua");
    std::fs::write(
        &modfile,
        b"local M = {}\nfunction M.double(x) return x * 2 end\nM.name = ...\nreturn M\n",
    )
    .unwrap();

    let mut s = new_state();
    let src = format!(
        r#"
        package.path = "{}/?.lua"
        local m = require("mathx")
        return m.double(21), m.name
    "#,
        dir.display()
    );
    let r = run_src(&mut s, &src);
    assert_eq!(as_num(r[0]), 42.0);
    assert_eq!(as_string(&s, r[1]), "mathx");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn require_dotted_name_maps_to_subdir() {
    let dir = unique_temp_dir("dotted");
    std::fs::create_dir_all(dir.join("foo")).unwrap();
    std::fs::write(dir.join("foo").join("bar.lua"), b"return { ok = true }\n").unwrap();

    let mut s = new_state();
    let src = format!(
        r#"
        package.path = "{}/?.lua"
        local m = require("foo.bar")
        return m.ok
    "#,
        dir.display()
    );
    let r = run_src(&mut s, &src);
    assert!(matches!(r[0], Value::Boolean(true)));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn require_not_found_errors() {
    let mut s = new_state();
    // 検索パスを空にして確実に見つからないようにする。
    let proto = compile(
        &mut s.global.heap,
        b"package.path = ''\nreturn require('no_such_module_xyz')",
        "=test",
    )
    .expect("compile ok");
    let res = vm::run(&mut s, Rc::new(proto), &[]);
    assert!(res.is_err(), "require of missing module must error");
}
