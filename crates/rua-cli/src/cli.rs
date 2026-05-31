//! コマンドライン定義（clap derive）。本家 `lua.c` / `luac.c` のフロントエンド相当。
//!
//! 後方互換のため、サブコマンドを省略した `rua <file>` / `rua -` / 引数なし `rua`（REPL）を
//! 受け付ける（`args_conflicts_with_subcommands` + フラット化したデフォルト引数で表現）。
//!
//! # カラースタイル
//! clap 4 の `styles` で色付きヘルプを提供する。

use clap::builder::styling::{AnsiColor, Effects};
use clap::{Args, Parser, Subcommand, builder::Styles};
use clap_complete::Shell;

/// clap のヘルプ出力のカラースタイル（clap 4 の `Styles` API）。
///
/// タイトル行：太字・下線、各項目：明るいシアン/黄色で見やすく。
fn clap_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::BrightGreen.on_default() | Effects::BOLD)
        .usage(AnsiColor::BrightCyan.on_default() | Effects::BOLD)
        .literal(AnsiColor::BrightCyan.on_default())
        .placeholder(AnsiColor::BrightYellow.on_default())
        .error(AnsiColor::BrightRed.on_default() | Effects::BOLD)
        .valid(AnsiColor::BrightGreen.on_default())
        .invalid(AnsiColor::BrightRed.on_default())
}

const LONG_ABOUT: &str = "\
rua は Lua 5.1 の Rust 実装インタプリタです。
本家 PUC-Rio lua5.1 のコマンドライン仕様に概ね準拠しています。

使用例:
  rua script.lua [args...]   スクリプトを実行（`rua run` と同じ）
  rua -                      標準入力から読み込んで実行
  rua                        対話モード（REPL）を起動

サブコマンド:
  rua run    スクリプトを実行
  rua luac   コンパイル（構文チェック / バイトコード列挙 / チャンク出力）
  rua repl   対話モードを明示的に起動
  rua completions <shell>    シェル補完スクリプトを生成";

const RUN_LONG_ABOUT: &str = "\
Lua スクリプトを実行します。本家 `lua5.1 script.lua` 相当。

スクリプトに渡した追加引数は `arg` テーブルおよびメインチャンクの `...` から
アクセスできます（本家 lua5.1 と同じ規約）:
  arg[0]  = スクリプト名
  arg[1]  = 第1引数（`...` の第1値）
  arg[2]  = 第2引数（`...` の第2値）
  ...

例:
  rua run script.lua
  rua run script.lua arg1 arg2
  echo 'print(...)' | rua run -  # 標準入力から実行";

const LUAC_LONG_ABOUT: &str = "\
Lua ソースをコンパイルします。本家 `luac` 相当。

用途:
  構文チェックのみ: -p
  バイトコード列挙: -l（詳細: -ll）
  コンパイル済みチャンクを出力: -o outfile infile
  デバッグ情報除去: -s

本家 `luac` との相違点:
  チャンクの出力フォーマットは rua 独自形式（\\x1bRua マジック）です。
  `rua run` はこのマジックを自動検出して実行します。

例:
  rua luac -p script.lua            # 構文チェックのみ（エラーがなければ無出力）
  rua luac -l script.lua            # バイトコード列挙
  rua luac -ll script.lua           # バイトコード + 定数表・ローカル・upvalue
  rua luac -o out.rbc script.lua    # コンパイル済みチャンクを out.rbc へ出力
  rua luac -s -o out.rbc script.lua # デバッグ情報を除去して出力
  rua run out.rbc                   # 出力チャンクを実行";

const REPL_LONG_ABOUT: &str = "\
リッチな対話インタプリタ（REPL）を起動します。

機能:
  - シンタックスハイライト（キーワード/文字列/数値/コメントを色分け）
  - Tab 補完（Lua 予約語 + グローバル変数）
  - 複数行継続（未完ブロックを自動検出して継続プロンプト表示）
  - 式の自動 return（`1+2` → `3` を表示）
  - 永続履歴（~/.local/share/rua/history.txt）

操作:
  式を入力して Enter  式を評価し結果を表示
  Tab                 補完候補を表示
  Ctrl-C              現在の入力を破棄
  Ctrl-D              REPL を終了
  exit                REPL を終了

例:
  rua          # 引数なしで REPL 起動
  rua repl     # 明示的に REPL 起動";

const COMPLETIONS_LONG_ABOUT: &str = "\
シェル補完スクリプトを標準出力に生成します。

生成後は各シェルの補完設定に応じて読み込んでください:

  bash:
    rua completions bash >> ~/.bashrc

  zsh:
    rua completions zsh > ~/.zfunc/_rua
    # ~/.zshrc に `fpath=(~/.zfunc $fpath)` と `autoload -U compinit` が必要

  fish:
    rua completions fish > ~/.config/fish/completions/rua.fish

  elvish:
    rua completions elvish >> ~/.config/elvish/rc.elv";

/// `rua` のトップレベル CLI。
#[derive(Debug, Parser)]
#[command(
    name = "rua",
    version,
    about = "Lua 5.1 interpreter written in Rust",
    long_about = LONG_ABOUT,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    disable_help_subcommand = false,
    styles = clap_styles(),
    after_help = "詳細は `rua <SUBCOMMAND> --help` で確認できます。",
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// サブコマンド省略時のデフォルト（`rua <file>` / `rua -` / 引数なし）。
    #[command(flatten)]
    pub default: DefaultArgs,
}

/// サブコマンドを省略したときの引数（本家 `lua [script [args]]` 相当）。
#[derive(Debug, Args)]
pub struct DefaultArgs {
    /// 実行する Lua スクリプト。`-` で標準入力。省略時は REPL を起動。
    #[arg(value_name = "SCRIPT")]
    pub script: Option<String>,

    /// スクリプトへ渡す引数（`arg` テーブル / メインチャンクの `...`）。
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// サブコマンド。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Lua スクリプトを実行する（本家 `lua` 相当）。
    #[command(long_about = RUN_LONG_ABOUT, after_help = "スクリプト引数は `arg[0]`, `arg[1]`, ... および `...` で参照できます。")]
    Run(RunArgs),

    /// Lua ソースをコンパイルする（本家 `luac` 相当）。
    #[command(name = "luac", long_about = LUAC_LONG_ABOUT)]
    Luac(LuacArgs),

    /// リッチな対話インタプリタ（REPL）を起動する。
    #[command(long_about = REPL_LONG_ABOUT)]
    Repl,

    /// シェル補完スクリプトを生成して標準出力へ書き出す。
    #[command(long_about = COMPLETIONS_LONG_ABOUT)]
    Completions(CompletionsArgs),
}

/// `rua run` の引数。
#[derive(Debug, Args)]
#[command(styles = clap_styles())]
pub struct RunArgs {
    /// 実行する Lua スクリプト。`-` で標準入力。
    #[arg(
        value_name = "SCRIPT",
        help = "実行する Lua スクリプト（`-` で標準入力）"
    )]
    pub script: String,

    /// スクリプトへ渡す引数（`arg[1]` 以降 / `...` に対応）。
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "スクリプトへ渡す引数（`arg[1]` 以降および `...` に対応）"
    )]
    pub args: Vec<String>,
}

/// `rua luac` の引数（本家 `luac` のオプションに寄せる）。
#[derive(Debug, Args)]
#[command(styles = clap_styles())]
pub struct LuacArgs {
    /// 構文チェックのみ行い、実行・出力はしない（本家 `luac -p`）。
    ///
    /// エラーがない場合は何も出力せずに終了します（exit code 0）。
    #[arg(
        short = 'p',
        long = "parse-only",
        help = "構文チェックのみ行う（出力なし、本家 `luac -p` 相当）",
        long_help = "構文チェックのみ行い、実行・ファイル出力はしません。\nエラーがなければ無出力で終了します（exit code 0）。\nエラーがあれば stderr に出力し exit code 1 で終了します。"
    )]
    pub parse_only: bool,

    /// バイトコードを一覧表示する（本家 `luac -l`）。
    ///
    /// 2 回以上指定すると定数表・ローカル変数・upvalue も出力します（`-ll` = `-l -l`）。
    #[arg(
        short = 'l',
        long = "list",
        action = clap::ArgAction::Count,
        help = "バイトコードを列挙する（`-ll` で詳細表示、本家 `luac -l` 相当）",
        long_help = "コンパイル済みバイトコードを本家 `luac -l` 形式で列挙します。\n  -l   命令一覧（アドレス・行番号・ニーモニック・オペランド・コメント）\n  -ll  上記 + 定数表・ローカル変数・upvalue 名の列挙"
    )]
    pub list: u8,

    /// コンパイル済みチャンクの出力先ファイル（本家 `luac -o`、既定 `luac.out`）。
    ///
    /// 出力形式は rua 独自（`\x1bRua` マジック）です。`rua run` で実行できます。
    #[arg(
        short = 'o',
        long = "output",
        value_name = "FILE",
        help = "コンパイル済みチャンクの出力先ファイル（既定: luac.out）",
        long_help = "コンパイル済みチャンクを指定のファイルへ出力します。\n出力形式は rua 独自形式（マジック \\x1bRua）で、`rua run <file>` で実行できます。\n`-p` を指定した場合は出力しません。"
    )]
    pub output: Option<String>,

    /// デバッグ情報（行番号・ローカル名）を取り除く（本家 `luac -s`）。
    ///
    /// 出力ファイルのサイズを削減できます。
    #[arg(
        short = 's',
        long = "strip",
        help = "デバッグ情報（行番号・ローカル名・upvalue 名）を除去する（本家 `luac -s` 相当）"
    )]
    pub strip: bool,

    /// 入力ファイル。`-` で標準入力。複数指定可能。
    #[arg(
        value_name = "FILES",
        required = true,
        help = "入力ファイル（`-` で標準入力）",
        long_help = "コンパイルする Lua ソースファイル。\n複数指定できます（`-l` と `-p` のみ。チャンク出力 `-o` は単一ファイルのみ）。"
    )]
    pub files: Vec<String>,
}

/// `rua completions` の引数。
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// 補完を生成するシェル（bash/zsh/fish/elvish/powershell）。
    #[arg(
        value_name = "SHELL",
        help = "補完を生成するシェル（bash / zsh / fish / elvish / powershell）"
    )]
    pub shell: Shell,
}
