//! コマンドライン定義（clap derive）。本家 `lua.c` / `luac.c` のフロントエンド相当。
//!
//! - `rua` バイナリ … `rua <file>` でスクリプト実行、引数なし `rua` で REPL を起動。
//! - `ruac` バイナリ … 本家 `luac` 相当のコンパイラ（[`RuacCli`]）。
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

const RUA_LONG_ABOUT: &str = "\
rua is a Lua 5.1 interpreter written in Rust.
It broadly follows the command-line behavior of the reference PUC-Rio lua5.1.

Usage:
  rua script.lua [args...]   Run a script
  rua -                      Read from standard input and run
  rua                        Start the interactive interpreter (REPL)

Script arguments are available through the `arg` table and the `...` of the
main chunk (same convention as the reference lua5.1):
  arg[0]  = script name
  arg[1]  = first argument (first value of `...`)
  arg[2]  = second argument (second value of `...`)
  ...

The compiler is provided as a separate `ruac` command (reference `luac`).

With no arguments, rua starts a rich interactive interpreter (REPL) with
syntax highlighting, Tab completion, multi-line continuation, automatic
`return` for expressions, and persistent history.";

const COMPLETIONS_LONG_ABOUT: &str = "\
Generate a shell completion script and write it to standard output.

Load the generated script according to your shell's completion setup:

  bash:
    rua completions bash >> ~/.bashrc

  zsh:
    rua completions zsh > ~/.zfunc/_rua
    # ~/.zshrc must contain `fpath=(~/.zfunc $fpath)` and `autoload -U compinit`

  fish:
    rua completions fish > ~/.config/fish/completions/rua.fish

  elvish:
    rua completions elvish >> ~/.config/elvish/rc.elv";

const RUAC_LONG_ABOUT: &str = "\
ruac compiles Lua source. Equivalent to the reference `luac`.

Modes:
  syntax check only:    -p
  list bytecode:        -l (verbose: -ll)
  emit compiled chunk:  -o outfile infile
  strip debug info:     -s

Differences from the reference `luac`:
  The compiled chunk format is rua-specific (\\x1bRua magic).
  `rua` automatically detects this magic and runs it.

Examples:
  ruac -p script.lua            # syntax check only (no output if OK)
  ruac -l script.lua            # list bytecode
  ruac -ll script.lua           # bytecode + constants / locals / upvalues
  ruac -o out.rbc script.lua    # write the compiled chunk to out.rbc
  ruac -s -o out.rbc script.lua # strip debug info and write
  rua out.rbc                   # run the emitted chunk";

/// `rua` インタプリタの CLI。
///
/// `rua <file> [args...]` でスクリプト実行、引数なしで REPL を起動する。
/// 補完生成のみサブコマンド（`rua completions <shell>`）として提供する。
#[derive(Debug, Parser)]
#[command(
    name = "rua",
    version,
    about = "Lua 5.1 interpreter written in Rust",
    long_about = RUA_LONG_ABOUT,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    disable_help_subcommand = true,
    styles = clap_styles(),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// スクリプト実行 / REPL のデフォルト引数（`rua <file>` / `rua -` / 引数なし）。
    #[command(flatten)]
    pub default: DefaultArgs,
}

/// `rua <file> [args...]` のデフォルト引数（本家 `lua [script [args]]` 相当）。
#[derive(Debug, Args)]
pub struct DefaultArgs {
    /// Lua script to run (`-` for standard input). Starts the REPL when omitted.
    #[arg(value_name = "SCRIPT")]
    pub script: Option<String>,

    /// Arguments passed to the script (`arg` table / `...` of the main chunk).
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// `rua` のサブコマンド（補完生成のみ）。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate a shell completion script on standard output.
    #[command(long_about = COMPLETIONS_LONG_ABOUT)]
    Completions(CompletionsArgs),
}

/// `rua completions` の引数。
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for (bash / zsh / fish / elvish / powershell).
    #[arg(value_name = "SHELL")]
    pub shell: Shell,
}

/// `ruac` コンパイラの CLI（トップレベル引数 = 本家 `luac` のオプション）。
#[derive(Debug, Parser)]
#[command(
    name = "ruac",
    version,
    about = "Lua 5.1 compiler written in Rust (reference `luac`)",
    long_about = RUAC_LONG_ABOUT,
    disable_help_subcommand = true,
    styles = clap_styles(),
)]
pub struct RuacCli {
    /// Check syntax only; do not run or write output (reference `luac -p`).
    ///
    /// Exits with code 0 and no output when there are no errors.
    #[arg(
        short = 'p',
        long = "parse-only",
        help = "Check syntax only (no output, reference `luac -p`)",
        long_help = "Check syntax only; do not run or write any output file.\nExits with code 0 and no output when there are no errors.\nOn error, writes to stderr and exits with code 1."
    )]
    pub parse_only: bool,

    /// List bytecode (reference `luac -l`).
    ///
    /// Specify twice or more to also list constants, locals and upvalues (`-ll`).
    #[arg(
        short = 'l',
        long = "list",
        action = clap::ArgAction::Count,
        help = "List bytecode (`-ll` for details, reference `luac -l`)",
        long_help = "List compiled bytecode in the reference `luac -l` format.\n  -l   instruction list (address, line, mnemonic, operands, comment)\n  -ll  the above + constant table, local variables, upvalue names"
    )]
    pub list: u8,

    /// Output file for the compiled chunk (reference `luac -o`, default `luac.out`).
    ///
    /// The output format is rua-specific (`\x1bRua` magic) and runs with `rua`.
    #[arg(
        short = 'o',
        long = "output",
        value_name = "FILE",
        help = "Output file for the compiled chunk (default: luac.out)",
        long_help = "Write the compiled chunk to the given file.\nThe output format is rua-specific (magic \\x1bRua) and runs with `rua <file>`.\nNothing is written when `-p` is given."
    )]
    pub output: Option<String>,

    /// Strip debug info (line numbers, local names) (reference `luac -s`).
    ///
    /// Reduces the size of the output file.
    #[arg(
        short = 's',
        long = "strip",
        help = "Strip debug info (line numbers, local / upvalue names) (reference `luac -s`)"
    )]
    pub strip: bool,

    /// Input files (`-` for standard input). Multiple files allowed.
    #[arg(
        value_name = "FILES",
        required = true,
        help = "Input files (`-` for standard input)",
        long_help = "Lua source files to compile.\nMultiple files are allowed with `-l` and `-p` only; chunk output `-o` takes a single file."
    )]
    pub files: Vec<String>,
}
