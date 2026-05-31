//! `rua` スタンドアロンインタプリタ（本家 `lua.c` 相当）。
//!
//! コマンド構成:
//!   - `rua <file> [args...]` / `rua -`  … スクリプト実行（`-` は標準入力）
//!   - `rua`（引数なし）                 … 対話モード（REPL）
//!   - `rua completions <shell>`         … シェル補完生成
//!
//! コンパイラは別バイナリ `ruac`（本家 `luac` 相当）として提供する。
//!
//! 実行フロー（`rua_core` の公開 API に結線）:
//!   LuaState::new() → stdlib::open_libs() → compiler::compile() → vm::run()

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::generate;

use rua_cli::cli::{Cli, Command, CompletionsArgs};
use rua_cli::{repl, run};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Completions(args)) => completions(args),
        None => match cli.default.script {
            Some(script) => dispatch_script(&script, &cli.default.args),
            // 引数なし → REPL。
            None => repl::main(),
        },
    }
}

/// スクリプト指定を実行する。`-` は標準入力。
fn dispatch_script(script: &str, args: &[String]) -> ExitCode {
    if script == "-" {
        run::run_stdin(args)
    } else {
        run::run_file(script, args)
    }
}

/// シェル補完スクリプトを標準出力へ生成する。
fn completions(args: CompletionsArgs) -> ExitCode {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, name, &mut io::stdout());
    ExitCode::SUCCESS
}
