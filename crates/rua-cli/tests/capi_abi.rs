//! C ABI リンク互換テスト（lua-conformance 所有, ARCHITECTURE.md §7）。
//!
//! 本家 `lua.h` を include する C プログラム（`tests/capi/smoke.c`）を
//! `rua-capi`（staticlib）にリンクしてビルド・実行し、標準出力を期待値
//! （`tests/capi/smoke.expected`）と厳密比較する。これが通れば
//! 「本家ヘッダを使う C コードが無改変でリンク・動作する」= ABI 互換を満たす。
//!
//! # 依存と自動スキップ
//! 本テストは **lua-capi の rua-capi クレート（タスク #6）に依存**する。
//! 以下のいずれかが欠ける場合は自動スキップ（パス扱い）し CI を壊さない:
//!   - C コンパイラ（`cc` / `$CC`）が無い
//!   - 生成済みヘッダ `lua.h` が見つからない（include ディレクトリ）
//!   - rua-capi の staticlib（`librua_capi.a`）が見つからない
//!
//! rua-capi が用意されたら、ヘッダ/ライブラリの場所を以下で指定するか、
//! 既定の探索パス（`target/<profile>/`、`crates/rua-capi/include/`）に置けば自動で結線される。
//!
//! # 環境変数
//! - `RUA_CAPI_INCLUDE=<dir>` : 生成 `lua.h`（lauxlib.h/lualib.h）のあるディレクトリ。
//! - `RUA_CAPI_LIB=<dir>`     : `librua_capi.a`（staticlib）のあるディレクトリ。
//! - `CC=<compiler>`          : 使用する C コンパイラ（既定 `cc`）。
//! - `RUA_CAPI_REQUIRE=1`     : スキップを許さず、欠ければテスト失敗（結線確認用）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <workspace>/crates/rua-cli
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn capi_dir() -> PathBuf {
    workspace_root().join("tests").join("capi")
}

/// C コンパイラを決定する（`$CC` 優先、無ければ `cc`）。存在しなければ None。
fn find_cc() -> Option<String> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    // `<cc> --version` が起動できるかで存在確認。
    match Command::new(&cc).arg("--version").output() {
        Ok(_) => Some(cc),
        Err(_) => None,
    }
}

/// `lua.h` を含む include ディレクトリを探す。
fn find_include_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("RUA_CAPI_INCLUDE") {
        let p = PathBuf::from(d);
        if p.join("lua.h").exists() {
            return Some(p);
        }
    }
    // 既定: rua-capi が cbindgen で生成するヘッダの想定置き場。
    [
        workspace_root()
            .join("crates")
            .join("rua-capi")
            .join("include"),
        workspace_root().join("target").join("include"),
    ]
    .into_iter()
    .find(|cand| cand.join("lua.h").exists())
}

/// `librua_capi.a`（staticlib）のあるディレクトリを探す。
fn find_lib_dir() -> Option<PathBuf> {
    let lib_name = "librua_capi.a";
    if let Ok(d) = std::env::var("RUA_CAPI_LIB") {
        let p = PathBuf::from(d);
        if p.join(lib_name).exists() {
            return Some(p);
        }
    }
    for profile in ["debug", "release"] {
        let cand = workspace_root().join("target").join(profile);
        if cand.join(lib_name).exists() {
            return Some(cand);
        }
    }
    None
}

/// スキップ要因を文字列で返す。None なら全て揃っている。
fn skip_reason(
    cc: &Option<String>,
    inc: &Option<PathBuf>,
    lib: &Option<PathBuf>,
) -> Option<String> {
    if cc.is_none() {
        return Some("C コンパイラ（cc/$CC）が見つかりません".into());
    }
    if inc.is_none() {
        return Some(
            "rua-capi の生成ヘッダ lua.h が見つかりません（タスク #6 lua-capi 待ち）".into(),
        );
    }
    if lib.is_none() {
        return Some(
            "rua-capi の staticlib（librua_capi.a）が見つかりません（タスク #6 待ち）".into(),
        );
    }
    None
}

#[test]
fn capi_link_smoke() {
    let cc = find_cc();
    let inc = find_include_dir();
    let lib = find_lib_dir();

    if let Some(reason) = skip_reason(&cc, &inc, &lib) {
        if std::env::var("RUA_CAPI_REQUIRE").as_deref() == Ok("1") {
            panic!("[capi-abi] RUA_CAPI_REQUIRE=1 だが結線できない: {reason}");
        }
        eprintln!("[capi-abi] SKIP: {reason}");
        eprintln!(
            "[capi-abi] rua-capi 完成後、ヘッダ/ライブラリを既定パスに置くか \
             RUA_CAPI_INCLUDE / RUA_CAPI_LIB を設定すると自動で結線されます。"
        );
        return;
    }
    let (cc, inc, lib) = (cc.unwrap(), inc.unwrap(), lib.unwrap());

    let src = capi_dir().join("smoke.c");
    assert!(src.exists(), "C テストソースが無い: {}", src.display());
    let exe = std::env::temp_dir().join(format!("rua_capi_smoke_{}", std::process::id()));

    // コンパイル + リンク。
    // 静的リンク優先（-Wl,-Bstatic -lrua_capi -Wl,-Bdynamic）を試みる。
    // Linux では -Wl,-Bstatic が使えるため staticlib を優先する。
    // macOS 等で -Wl,-Bstatic が利用不可な場合はフォールバックとして通常リンクを試みる。
    // staticlib はシステムライブラリ（-lm/-ldl/-lpthread）が必要。
    let try_static_link = || {
        Command::new(&cc)
            .arg(&src)
            .arg("-I")
            .arg(&inc)
            .arg("-L")
            .arg(&lib)
            .arg("-Wl,-Bstatic")
            .arg("-lrua_capi")
            .arg("-Wl,-Bdynamic")
            .arg("-lm")
            .arg("-ldl")
            .arg("-lpthread")
            .arg("-o")
            .arg(&exe)
            .status()
    };
    let try_dynamic_link = || {
        Command::new(&cc)
            .arg(&src)
            .arg("-I")
            .arg(&inc)
            .arg("-L")
            .arg(&lib)
            .arg("-lrua_capi")
            .arg("-lm")
            .arg("-ldl")
            .arg("-lpthread")
            .arg("-o")
            .arg(&exe)
            .status()
    };
    // まず静的リンクを試みる。失敗した場合は動的リンクにフォールバック。
    let status = match try_static_link() {
        Ok(s) if s.success() => Ok(s),
        _ => try_dynamic_link(),
    };

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            let _ = std::fs::remove_file(&exe);
            panic!(
                "[capi-abi] C テストのコンパイル/リンク失敗 (status {s}). ABI ヘッダ不整合の可能性。"
            );
        }
        Err(e) => {
            let _ = std::fs::remove_file(&exe);
            panic!("[capi-abi] cc 起動失敗: {e}");
        }
    }

    let output = Command::new(&exe).output().expect("capi smoke 実行失敗");
    let _ = std::fs::remove_file(&exe);

    let expected = std::fs::read(capi_dir().join("smoke.expected")).unwrap_or_default();
    assert!(
        output.status.success(),
        "[capi-abi] C テスト異常終了: status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        output.stdout,
        expected,
        "[capi-abi] stdout 不一致\n--- expected ---\n{}\n--- got ---\n{}",
        String::from_utf8_lossy(&expected),
        String::from_utf8_lossy(&output.stdout),
    );
    eprintln!("[capi-abi] OK: rua-capi に C プログラムをリンクして実行成功（ABI 互換）");
}
