//! 公式 Lua 5.1 テストスイート（PUC-Rio）のパス率トラッキング（lua-conformance 所有）。
//!
//! `tests/lua-suite/` 配下に取得済みの公式テスト `*.lua` を `rua run` で実行し、
//! 終了コードで分類・集計する。テスト本体は別ライセンス配布のためコミットしない
//! （`tests/lua-suite/fetch.sh` で取得）。
//!
//! # 分類（ARCHITECTURE.md §8 / 役割定義）
//! - **pass**    : 終了コード 0（本家スイートは末尾で `print"OK"` 等しつつ成功時 0 で終わる）。
//! - **fail**    : それ以外の通常終了（Lua エラー＝機能未実装/非互換）。パス率追跡対象。
//! - **crash**   : Rust パニック（exit 101）/ シグナル異常終了（abort/segfault）。
//!   設計原則上ほぼ常にバグ。検出したら最小再現付きで該当ロールへ報告する。
//! - **timeout** : 暴走（無限ループ等）。やはり非互換の兆候。
//!
//! 重く既定では実行しないため `#[ignore]`。明示実行:
//! ```text
//! cargo test -p rua-cli --test official_suite -- --ignored --nocapture
//! ```
//! スイート未取得なら自動スキップ（CI を壊さない）。
//! `RUA_SUITE_STRICT=1` を設定すると crash 検出時にテストを失敗させる（回帰検出用）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const RUA_BIN: &str = env!("CARGO_BIN_EXE_rua");

/// 1 本のテストが暴走しても全体を止めないためのタイムアウト（秒）。
const PER_TEST_TIMEOUT: Duration = Duration::from_secs(30);

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("lua-suite")
}

fn collect_suite_scripts(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_suite_scripts(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("lua") {
            out.push(path);
        }
    }
}

/// プロセス終了の分類。
enum Outcome {
    /// 通常終了（終了コード付き）。
    Exited(i32),
    /// シグナルによる異常終了（abort/segfault 等）。code() == None。
    Signaled,
    /// タイムアウト（こちらから kill）。
    Timeout,
    /// 起動失敗等。
    SpawnError,
}

/// タイムアウト付きでプロセスを待つ簡易ヘルパ（外部 crate 非依存）。
fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Outcome {
    use std::process::Stdio;
    let mut child = match cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Outcome::SpawnError,
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(code) => Outcome::Exited(code),
                    // code() == None はシグナル終了（Unix）。abort/segfault を含む。
                    None => Outcome::Signaled,
                };
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Outcome::Timeout;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Outcome::SpawnError,
        }
    }
}

#[test]
#[ignore = "公式スイートの取得が必要。tests/lua-suite/fetch.sh 後に --ignored で実行"]
fn official_suite_pass_rate() {
    let dir = suite_dir();
    let mut scripts = Vec::new();
    collect_suite_scripts(&dir, &mut scripts);
    scripts.retain(|p| {
        // ドライバ all.lua は他ファイルを require/dofile するため個別集計から除外。
        p.file_name().and_then(|s| s.to_str()) != Some("all.lua")
    });
    scripts.sort();

    if scripts.is_empty() {
        eprintln!(
            "[official-suite] SKIP: {} に *.lua が見つかりません。\n\
             tests/lua-suite/fetch.sh で公式スイートを取得してください。",
            dir.display()
        );
        return;
    }

    let mut passed = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut crashed: Vec<String> = Vec::new();
    let mut timed_out: Vec<String> = Vec::new();

    for script in &scripts {
        let rel = script.strip_prefix(&dir).unwrap_or(script);
        let mut cmd = Command::new(RUA_BIN);
        cmd.arg(rel).current_dir(&dir);
        match run_with_timeout(&mut cmd, PER_TEST_TIMEOUT) {
            Outcome::Exited(0) => passed += 1,
            // 101 = Rust パニック。互換性上ほぼ常にバグ。
            Outcome::Exited(101) => crashed.push(format!("{} (panic / exit 101)", rel.display())),
            Outcome::Exited(code) => failed.push(format!("{} (exit {code})", rel.display())),
            Outcome::Signaled => {
                crashed.push(format!("{} (abort/segfault: signal)", rel.display()))
            }
            Outcome::Timeout => timed_out.push(format!("{} (timeout)", rel.display())),
            Outcome::SpawnError => failed.push(format!("{} (spawn error)", rel.display())),
        }
    }

    let total = scripts.len();
    let rate = 100.0 * passed as f64 / total as f64;
    eprintln!("\n===== 公式 Lua 5.1 テストスイート パス率 =====");
    eprintln!("pass:    {passed}/{total} ({rate:.1}%)");
    eprintln!("fail:    {}", failed.len());
    eprintln!(
        "crash:   {}  (パニック/abort = バグ。要報告)",
        crashed.len()
    );
    eprintln!("timeout: {}", timed_out.len());
    if !failed.is_empty() {
        eprintln!("-- fail (Lua エラー = 機能未実装/非互換) --");
        for f in &failed {
            eprintln!("  - {f}");
        }
    }
    if !crashed.is_empty() {
        eprintln!("-- crash (パニック/abort = 互換性バグ。最小再現付きで該当ロールへ報告) --");
        for c in &crashed {
            eprintln!("  - {c}");
        }
    }
    if !timed_out.is_empty() {
        eprintln!("-- timeout --");
        for t in &timed_out {
            eprintln!("  - {t}");
        }
    }
    eprintln!("=============================================\n");

    // 通常 fail はパス率の経時追跡が目的なのでテストを落とさない。
    // crash（パニック/abort）は設計原則上バグなので、STRICT 指定時はテストを失敗させる。
    if std::env::var("RUA_SUITE_STRICT").as_deref() == Ok("1") {
        assert!(
            crashed.is_empty(),
            "公式スイートで {} 件のクラッシュ（パニック/abort）を検出。\
             最小再現を作成し該当ロールへ報告すること:\n{}",
            crashed.len(),
            crashed.join("\n")
        );
    }
}
