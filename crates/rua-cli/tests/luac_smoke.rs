//! `rua luac`（luac 相当）のスモークテスト（lua-cli 所有）。
//!
//! 構文チェック(`-p`)・列挙(`-l`)・チャンク出力(`-o`)とラウンドトリップ実行を確認する。

use std::io::Write;
use std::process::{Command, Stdio};

const RUA_BIN: &str = env!("CARGO_BIN_EXE_rua");

fn run_with_stdin(args: &[&str], stdin: &[u8]) -> (Vec<u8>, String, i32) {
    let mut child = Command::new(RUA_BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rua バイナリを起動できない");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().expect("rua 終了待ち失敗");
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn stdout_str(args: &[&str], stdin: &[u8]) -> (String, String, i32) {
    let (o, e, c) = run_with_stdin(args, stdin);
    (String::from_utf8_lossy(&o).into_owned(), e, c)
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("rua_luac_{}_{}", std::process::id(), name))
}

#[test]
fn parse_only_ok() {
    let (stdout, _e, code) = stdout_str(&["luac", "-p", "-"], b"print(1+2)\n");
    assert_eq!(code, 0);
    assert!(stdout.is_empty(), "出力があってはならない: {stdout}");
}

#[test]
fn parse_only_reports_syntax_error() {
    let (_o, stderr, code) = stdout_str(&["luac", "-p", "-"], b"local = =\n");
    assert_eq!(code, 1);
    assert!(stderr.contains("rua:"), "stderr: {stderr}");
}

#[test]
fn list_outputs_bytecode() {
    let (stdout, _e, code) = stdout_str(&["luac", "-l", "-p", "-"], b"print(40 + 2)\n");
    assert_eq!(code, 0);
    // ヘッダと主要命令が含まれること（luac -l 風）。
    assert!(stdout.contains("main <"), "header: {stdout}");
    assert!(stdout.contains("GETGLOBAL"), "GETGLOBAL: {stdout}");
    assert!(stdout.contains("RETURN"), "RETURN: {stdout}");
    assert!(stdout.contains("; \"print\""), "comment: {stdout}");
}

#[test]
fn list_verbose_dumps_constants_and_locals() {
    let (stdout, _e, code) = stdout_str(&["luac", "-ll", "-p", "-"], b"local x = 7\nreturn x\n");
    assert_eq!(code, 0);
    assert!(stdout.contains("constants ("), "constants: {stdout}");
    assert!(stdout.contains("locals ("), "locals: {stdout}");
}

#[test]
fn dump_and_run_roundtrip() {
    let src = b"local function add(a,b) return a+b end\nprint(add(40, 2))\n";
    let lua = tmp_path("rt.lua");
    let out = tmp_path("rt.rbc");
    std::fs::write(&lua, src).unwrap();

    // コンパイル → チャンク出力。
    let (_o, stderr, code) = stdout_str(
        &["luac", "-o", out.to_str().unwrap(), lua.to_str().unwrap()],
        b"",
    );
    assert_eq!(code, 0, "luac 失敗: {stderr}");
    assert!(out.exists(), "出力チャンクが無い");
    // マジックを確認。
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[..4], b"\x1bRua");

    // 出力チャンクを実行。
    let (stdout, stderr, code) = stdout_str(&["run", out.to_str().unwrap()], b"");
    assert_eq!(code, 0, "チャンク実行失敗: {stderr}");
    assert_eq!(stdout, "42\n");

    let _ = std::fs::remove_file(&lua);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn strip_roundtrip_runs() {
    let src = b"print(('ok'):upper())\n";
    let lua = tmp_path("strip.lua");
    let out = tmp_path("strip.rbc");
    std::fs::write(&lua, src).unwrap();

    let (_o, _e, code) = stdout_str(
        &[
            "luac",
            "-s",
            "-o",
            out.to_str().unwrap(),
            lua.to_str().unwrap(),
        ],
        b"",
    );
    assert_eq!(code, 0);
    let (stdout, _e, code) = stdout_str(&["run", out.to_str().unwrap()], b"");
    assert_eq!(code, 0);
    assert_eq!(stdout, "OK\n");

    let _ = std::fs::remove_file(&lua);
    let _ = std::fs::remove_file(&out);
}
