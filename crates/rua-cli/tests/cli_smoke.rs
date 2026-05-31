//! `rua` CLI のスモークテスト（lua-cli 所有）。
//!
//! `rua <file>` / `rua -` でのスクリプト実行、補完生成・終了コードが
//! 期待通りであることを最小限で確認する。

use std::io::Write;
use std::process::{Command, Stdio};

const RUA_BIN: &str = env!("CARGO_BIN_EXE_rua");

/// stdin を与えて `rua <args>` を起動し、(stdout, stderr, exit_code) を返す。
fn run_with_stdin(args: &[&str], stdin: &[u8]) -> (String, String, i32) {
    let mut child = Command::new(RUA_BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rua バイナリを起動できない");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin)
        .expect("stdin 書き込み失敗");
    let out = child.wait_with_output().expect("rua の終了待ち失敗");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn run(args: &[&str]) -> (String, String, i32) {
    run_with_stdin(args, b"")
}

#[test]
fn version_flag() {
    let (stdout, _stderr, code) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("rua"), "version 出力: {stdout}");
}

#[test]
fn help_flag_succeeds() {
    let (stdout, _stderr, code) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Usage"), "help 出力: {stdout}");
}

#[test]
fn stdin_dash_runs() {
    let (stdout, _stderr, code) = run_with_stdin(&["-"], b"print(1+2)\n");
    assert_eq!(code, 0);
    assert_eq!(stdout, "3\n");
}

#[test]
fn missing_file_exits_one() {
    let (_stdout, stderr, code) = run(&["definitely_missing_file.lua"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("cannot open"), "stderr: {stderr}");
}

#[test]
fn runtime_error_exits_one() {
    let (_stdout, stderr, code) = run_with_stdin(&["-"], b"error('boom', 0)\n");
    assert_eq!(code, 1);
    assert!(stderr.contains("boom"), "stderr: {stderr}");
}

#[test]
fn script_args_exposed_as_arg_and_vararg() {
    let src = b"print(arg[0], arg[1], arg[2])\nprint(...)\n";
    // 一時ファイル経由（arg[0] はスクリプト名になる）。
    let dir = std::env::temp_dir();
    let path = dir.join(format!("rua_cli_argtest_{}.lua", std::process::id()));
    std::fs::write(&path, src).unwrap();
    let path_str = path.to_str().unwrap();
    let (stdout, _stderr, code) = run(&[path_str, "a", "b"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0);
    assert_eq!(stdout, format!("{path_str}\ta\tb\na\tb\n"));
}

#[test]
fn completions_bash_generates_script() {
    let (stdout, _stderr, code) = run(&["completions", "bash"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("_rua"),
        "bash 補完: {}",
        &stdout[..stdout.len().min(80)]
    );
}

#[test]
fn syntax_error_exits_one() {
    let (_stdout, stderr, code) = run_with_stdin(&["-"], b"x = \n");
    assert_eq!(code, 1);
    assert!(stderr.contains("rua:"), "stderr: {stderr}");
}
