//! ゴールデン比較ハーネス（lua-conformance 所有）。
//!
//! `tests/lua/**/*.lua` の各スクリプトを `rua run <file>` で実行し、
//! 標準出力（stdout）と終了コードをコミット済みの期待値（`*.expected` / `*.exitcode`）と比較する。
//!
//! # 設計（ARCHITECTURE.md §8 / 役割定義）
//! - 「本家を正」とする差分ベース。期待値はハードコードせず本家 `lua5.1` から生成するのが原則。
//!   本リポジトリにはコミット済みの `*.expected`（手作業で本家 5.1 準拠に作成）を同梱し、
//!   本家が利用可能な環境では [`validate_expected_against_reference`] で検証・再生成できる。
//! - rua の実行系（コンパイラ/VM/CLI）がまだスクリプトを実行できない段階では、
//!   [`golden_compare`] は自動的に **スキップ**（パス扱い）し、CI を壊さない。
//!   CLI が動くようになると自動的に実行へ切り替わる（`RUA_PROBE_OK` プローブで判定）。
//!
//! # 環境変数
//! - `RUA_CONFORMANCE=run`  : スキップ判定を無視して強制的に実行（rua が未実装だと失敗する）。
//! - `RUA_CONFORMANCE=skip` : 強制スキップ。
//! - `RUA_LUA_BIN=<path>`   : リファレンス Lua 5.1 インタプリタのパス（検証/再生成用）。
//!
//! stderr はチャンク名（パス）・行番号を含み環境依存なので**厳密比較しない**。
//! 位置情報に依存するテストは Lua 側で `error(msg, 0)` 等を使い決定的にしてある。

use std::path::{Path, PathBuf};
use std::process::Command;

/// ビルド済み `rua` バイナリの絶対パス（cargo が test 前にビルドを保証する）。
const RUA_BIN: &str = env!("CARGO_BIN_EXE_rua");

/// `tests/lua/` ディレクトリ（ワークスペースルート基準）。
fn lua_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <workspace>/crates/rua-cli
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("lua")
}

/// `tests/lua/**` 配下の `*.lua` を列挙（ファイル名でソート、決定的順序）。
fn collect_lua_scripts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_into(dir, &mut out);
    out.sort();
    out
}

fn collect_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_into(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("lua") {
            out.push(path);
        }
    }
}

/// rua CLI が実際に Lua スクリプトを実行できる状態かをプローブする。
///
/// 一時ファイルに `print("RUA_PROBE_OK")` を書き出して `rua <file>` し、
/// stdout がそれと一致し終了コード 0 なら「実行可能（live）」とみなす。
/// まだ未実装（フェーズ0 スケルトン）なら false。
fn rua_is_live() -> bool {
    match std::env::var("RUA_CONFORMANCE").as_deref() {
        Ok("run") => return true,
        Ok("skip") => return false,
        _ => {}
    }
    let probe_path = std::env::temp_dir().join(format!("rua_probe_{}.lua", std::process::id()));
    if std::fs::write(&probe_path, b"print(\"RUA_PROBE_OK\")\n").is_err() {
        return false;
    }
    let result = Command::new(RUA_BIN).arg(&probe_path).output();
    let _ = std::fs::remove_file(&probe_path);
    match result {
        Ok(out) => {
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "RUA_PROBE_OK"
        }
        Err(_) => false,
    }
}

/// スクリプトを `dir` からの相対パス（通常は basename）に直す。
///
/// エラーメッセージのチャンク名は「インタプリタに渡したパス文字列そのもの」になる
/// （本家 5.1: `luaL_error` が `短いソース名:行:` を前置）。`regenerate_expected.sh` は
/// `cd tests/lua` してから basename で本家を起動するため、`.expected` 内の位置情報は
/// `08_errors.lua:27:` のような basename 形式になっている。rua をゴールデン比較する際も
/// 同一の cwd・相対パスで起動し、チャンク名を一致させる（= エラー整形の互換性も検証する）。
fn rel_arg<'a>(dir: &Path, script: &'a Path) -> &'a Path {
    script.strip_prefix(dir).unwrap_or(script)
}

/// 期待される終了コードを `<script>.exitcode` から読む（無ければ 0）。
fn expected_exit_code(script: &Path) -> i32 {
    let p = script.with_extension("exitcode");
    match std::fs::read_to_string(&p) {
        Ok(s) => s.trim().parse().unwrap_or(0),
        Err(_) => 0,
    }
}

/// 全 `*.lua` に対応する `*.expected` が存在することを保証する（rua の実装状況に依存しない）。
///
/// テスト資産の整合性を常に担保するため、live でなくても実行される。
#[test]
fn every_script_has_expected() {
    let dir = lua_dir();
    let scripts = collect_lua_scripts(&dir);
    assert!(
        !scripts.is_empty(),
        "tests/lua にスクリプトが見つからない: {}",
        dir.display()
    );

    let mut missing = Vec::new();
    for script in &scripts {
        let expected = script.with_extension("expected");
        if !expected.exists() {
            missing.push(script.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "期待値ファイル(.expected)が無いスクリプト:\n{}",
        missing.join("\n")
    );

    eprintln!("[conformance] {} 本のテストスクリプトを確認", scripts.len());
}

/// ゴールデン比較本体。rua が live なら全スクリプトを実行して stdout/終了コードを比較。
/// live でなければスキップ（パス扱い）し、その旨を表示する。
#[test]
fn golden_compare() {
    let dir = lua_dir();
    let scripts = collect_lua_scripts(&dir);

    if !rua_is_live() {
        eprintln!(
            "[conformance] SKIP: rua はまだ Lua スクリプトを実行できません \
             (フェーズ0 スケルトン)。CLI/VM が動くと自動で実行に切り替わります。\n\
             強制実行するには RUA_CONFORMANCE=run を設定してください。\n\
             ({} 本のスクリプトが実行待ち)",
            scripts.len()
        );
        return;
    }

    let mut failures = Vec::new();
    let mut passed = 0usize;

    for script in &scripts {
        let expected_path = script.with_extension("expected");
        let expected_stdout = match std::fs::read(&expected_path) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{}: .expected 読込失敗: {e}", script.display()));
                continue;
            }
        };
        let want_code = expected_exit_code(script);

        // 本家と同じ cwd・相対パスで起動（チャンク名一致 → エラー整形も比較対象になる）。
        let output = match Command::new(RUA_BIN)
            .arg(rel_arg(&dir, script))
            .current_dir(&dir)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                failures.push(format!("{}: rua 起動失敗: {e}", script.display()));
                continue;
            }
        };

        let got_code = output.status.code().unwrap_or(-1);
        let mut local_fail = Vec::new();
        if output.stdout != expected_stdout {
            local_fail.push(format!(
                "  stdout 不一致:\n--- expected ---\n{}\n--- got ---\n{}\n--- end ---",
                String::from_utf8_lossy(&expected_stdout),
                String::from_utf8_lossy(&output.stdout),
            ));
        }
        if got_code != want_code {
            local_fail.push(format!(
                "  終了コード不一致: expected {want_code}, got {got_code}"
            ));
        }

        if local_fail.is_empty() {
            passed += 1;
        } else {
            failures.push(format!("{}:\n{}", script.display(), local_fail.join("\n")));
        }
    }

    let total = scripts.len();
    eprintln!("[conformance] golden_compare: {passed}/{total} パス");

    assert!(
        failures.is_empty(),
        "{} 本のスクリプトで非互換を検出:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// リファレンス `lua5.1` が利用可能な場合に、コミット済み `*.expected` が
/// 本家出力と一致するか検証する（手動: `cargo test -- --ignored`）。
///
/// 本家が見つからなければスキップ。差分があれば「期待値が誤り or 本家との非互換」を示す。
#[test]
#[ignore = "リファレンス lua5.1 が必要。手動で `cargo test -- --ignored` 実行"]
fn validate_expected_against_reference() {
    let reference = match find_reference_lua() {
        Some(p) => p,
        None => {
            eprintln!(
                "[conformance] SKIP: リファレンス Lua が見つかりません。\n\
                 docs/CONFORMANCE.md の手順で lua5.1 を導入するか RUA_LUA_BIN を設定してください。"
            );
            return;
        }
    };
    eprintln!("[conformance] reference = {}", reference.display());

    let dir = lua_dir();
    let scripts = collect_lua_scripts(&dir);
    let mut mismatches = Vec::new();

    for script in &scripts {
        let expected_path = script.with_extension("expected");
        let committed = std::fs::read(&expected_path).unwrap_or_default();
        // regenerate_expected.sh と同じく cwd=tests/lua・相対パスで起動する。
        let output = match Command::new(&reference)
            .arg(rel_arg(&dir, script))
            .current_dir(&dir)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                mismatches.push(format!("{}: 本家起動失敗: {e}", script.display()));
                continue;
            }
        };
        if output.stdout != committed {
            mismatches.push(format!(
                "{}:\n--- committed .expected ---\n{}\n--- reference lua5.1 ---\n{}\n",
                script.display(),
                String::from_utf8_lossy(&committed),
                String::from_utf8_lossy(&output.stdout),
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "コミット済み .expected が本家出力と不一致 ({} 本):\n\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!("[conformance] 全 .expected が本家 lua5.1 と一致");
}

/// リファレンス Lua 5.1 インタプリタを探す。`RUA_LUA_BIN` 優先、次に既知の名前を PATH から。
fn find_reference_lua() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUA_LUA_BIN") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    for name in ["lua5.1", "lua-5.1", "lua51", "lua"] {
        if let Ok(out) = Command::new(name).arg("-v").output() {
            // Lua 5.1 系のみ受理（"Lua 5.1" を含む）。lua5.1 はバナーを stderr に出す。
            let banner = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            );
            if banner.contains("Lua 5.1") {
                return Some(PathBuf::from(name));
            }
        }
    }
    None
}
