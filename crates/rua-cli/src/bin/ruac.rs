//! `ruac` スタンドアロンコンパイラ（本家 `luac.c` 相当）。
//!
//! コマンド構成:
//!   - `ruac -p <file>`            … 構文チェックのみ
//!   - `ruac -l <file>`            … バイトコード列挙（`-ll` で詳細）
//!   - `ruac -o out.rbc <file>`    … コンパイル済みチャンクを出力
//!   - `ruac -s -o out.rbc <file>` … デバッグ情報を除去して出力
//!
//! 出力チャンクは rua 独自形式（`\x1bRua` マジック）で、`rua <file>` が実行できる。

use std::process::ExitCode;

use clap::Parser;

use rua_cli::cli::RuacCli;
use rua_cli::luac;

fn main() -> ExitCode {
    let args = RuacCli::parse();
    luac::main(args)
}
