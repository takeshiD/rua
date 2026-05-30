//! `rua` スタンドアロンインタプリタ（本家 `lua.c` 相当）。
//!
//! 第一マイルストーンの目標コマンド `rua run <file>`（および `rua <file>`）。
//! チャンクの読込 → コンパイル → 実行 → エラー表示/終了コードを本家 `lua5.1` に寄せる。
//!
//! 実行フロー（`rua_core` の公開 API に結線）:
//!   LuaState::new() → stdlib::open_libs() → compiler::compile() → vm::run()

use std::process::ExitCode;
use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::error::LuaError;
use rua_core::gc::GcHandle;
use rua_core::state::LuaState;
use rua_core::stdlib;
use rua_core::value::Value;
use rua_core::vm::run;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // `rua run <script.lua>`
        Some("run") => match args.get(2) {
            Some(path) => run_file(path),
            None => usage(),
        },
        // `rua -` で標準入力から読む（本家互換の簡易版）。
        Some("-") => run_stdin(),
        // `rua <script.lua>`（run 省略形）。
        Some(path) => run_file(path),
        None => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: rua run <script.lua>  |  rua <script.lua>  |  rua -");
    ExitCode::from(1)
}

/// Lua スクリプトファイルを読み込んで実行する。
fn run_file(path: &str) -> ExitCode {
    let source = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rua: cannot open {path}: {e}");
            return ExitCode::from(1);
        }
    };
    // 本家同様、ファイル由来のチャンク名は `@` プレフィックス（エラー表示時に除去される）。
    execute(&source, &format!("@{path}"))
}

/// 標準入力から読み込んで実行する。
fn run_stdin() -> ExitCode {
    use std::io::Read;
    let mut source = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut source) {
        eprintln!("rua: cannot read stdin: {e}");
        return ExitCode::from(1);
    }
    execute(&source, "=stdin")
}

/// ソースをコンパイル→実行し、本家に寄せた終了コードを返す。
fn execute(source: &[u8], chunkname: &str) -> ExitCode {
    let mut state = LuaState::new();
    stdlib::open_libs(&mut state);

    let proto = match compile(&mut state.global.heap, source, chunkname) {
        Ok(p) => p,
        Err(e) => {
            // 構文エラー: 本家は `lua: <chunk>:<line>: <msg>` 形式。詳細整形は今後 lua-conformance と調整。
            eprintln!("rua: {e}");
            return ExitCode::from(1);
        }
    };

    match run(&mut state, Rc::new(proto), &[]) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            // 未捕捉の実行時エラー: 本家標準インタプリタは終了コード 1 で stderr に出力。
            eprintln!("rua: {}", render_error(&state, &e));
            ExitCode::from(1)
        }
    }
}

/// 未捕捉エラーを本家 `lua.c` に寄せて整形する。
///
/// Lua のエラーオブジェクトは任意の値を取りうる。文字列ならそのまま、数値なら数値表現、
/// それ以外で `__tostring` も無い場合は本家同様 `(error object is a <type> value)` とする。
/// 文字列値はヒープを引かないと内容が取れないため、ここ（state を持つ CLI）で解決する。
fn render_error(state: &LuaState, e: &LuaError) -> String {
    match e {
        LuaError::Runtime(Value::GcRef(GcHandle::Str(key))) => {
            match state.global.heap.get_str(*key) {
                Some(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
                None => "(error object is a dangling string)".to_string(),
            }
        }
        LuaError::Runtime(Value::Number(n)) => format!("{n}"),
        LuaError::Runtime(Value::Nil) => "nil".to_string(),
        LuaError::Runtime(Value::Boolean(b)) => b.to_string(),
        LuaError::Runtime(v) => {
            format!("(error object is a {} value)", v.type_of().name())
        }
        other => other.to_string(),
    }
}
