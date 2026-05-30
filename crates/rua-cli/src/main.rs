//! `rua` スタンドアロンインタプリタ（本家 `lua.c` / `luac.c` 相当）。
//!
//! clap によるサブコマンド構成:
//!   - `rua run <file>` / `rua <file>` / `rua -`  … スクリプト実行（後方互換）
//!   - `rua luac ...`                              … コンパイラ（#17）
//!   - `rua`（引数なし） / `rua repl`              … 対話モード（#18）
//!   - `rua completions <shell>`                   … シェル補完生成
//!
//! 実行フロー（`rua_core` の公開 API に結線）:
//!   LuaState::new() → stdlib::open_libs() → compiler::compile() → vm::run()

mod bytecode;
mod cli;
mod disasm;
mod luac;
mod repl;
mod run;

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::generate;

use cli::{Cli, Command, CompletionsArgs};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run(args)) => dispatch_script(&args.script, &args.args),
        Some(Command::Luac(args)) => luac::main(args),
        Some(Command::Repl) => repl::main(),
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
