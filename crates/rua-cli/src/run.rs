//! チャンクの読込 → コンパイル → 実行（本家 `lua.c` の `dofile`/`dostring` 相当）。
//!
//! `rua run <file>` / `rua <file>` / `rua -`（stdin）の実体。エラー表示・終了コードは
//! 本家 `lua5.1` に寄せる。

use std::process::ExitCode;
use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::error::LuaError;
use rua_core::gc::GcHandle;
use rua_core::state::LuaState;
use rua_core::stdlib;
use rua_core::value::Value;
use rua_core::vm::run;

/// Lua スクリプトファイルを読み込んで実行する。
///
/// `script_args` はスクリプトへ渡す引数（`arg` テーブルおよびメインチャンクの `...` に束ねる）。
pub fn run_file(path: &str, script_args: &[String]) -> ExitCode {
    let source = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rua: cannot open {path}: {e}");
            return ExitCode::from(1);
        }
    };
    // 本家同様、ファイル由来のチャンク名は `@` プレフィックス（エラー表示時に除去される）。
    execute(&source, &format!("@{path}"), Some(path), script_args)
}

/// 標準入力から読み込んで実行する。
pub fn run_stdin(script_args: &[String]) -> ExitCode {
    use std::io::Read;
    let mut source = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut source) {
        eprintln!("rua: cannot read stdin: {e}");
        return ExitCode::from(1);
    }
    execute(&source, "=stdin", None, script_args)
}

/// ソースをコンパイル→実行し、本家に寄せた終了コードを返す。
///
/// `script_name` は `arg[0]` に入れるスクリプト名（stdin の場合は `None`）。
fn execute(
    source: &[u8],
    chunkname: &str,
    script_name: Option<&str>,
    script_args: &[String],
) -> ExitCode {
    let mut state = LuaState::new();
    stdlib::open_libs(&mut state);
    setup_arg_table(&mut state, script_name, script_args);

    // rua バイナリチャンク（`luac -o` の出力）ならコンパイルせず逆シリアライズする。
    let proto = if crate::bytecode::is_rua_chunk(source) {
        match crate::bytecode::undump(&mut state.global.heap, source) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("rua: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        match compile(&mut state.global.heap, source, chunkname) {
            Ok(p) => p,
            Err(e) => {
                // 構文エラー: 本家は `lua: <chunk>:<line>: <msg>` 形式。
                eprintln!("rua: {e}");
                return ExitCode::from(1);
            }
        }
    };

    // メインチャンクは vararg。スクリプト引数を `...` として渡す。
    let argv: Vec<Value> = script_args
        .iter()
        .map(|a| state.global.heap.intern_str(a.as_bytes()))
        .map(Value::GcRef)
        .collect();

    match run(&mut state, Rc::new(proto), &argv) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            // 未捕捉の実行時エラー: 本家標準インタプリタは終了コード 1 で stderr に出力。
            eprintln!("rua: {}", render_error(&state, &e));
            ExitCode::from(1)
        }
    }
}

/// 本家 `lua.c` 同様、グローバル `arg` テーブルを構築する。
///
/// `arg[0]` = スクリプト名、`arg[1..]` = スクリプト引数。
fn setup_arg_table(state: &mut LuaState, script_name: Option<&str>, script_args: &[String]) {
    use rua_core::value::table::Table;

    let mut t = Table::new();
    if let Some(name) = script_name {
        let v = Value::GcRef(state.global.heap.intern_str(name.as_bytes()));
        let _ = t.set(Value::Number(0.0), v);
    }
    let handles: Vec<Value> = script_args
        .iter()
        .map(|a| Value::GcRef(state.global.heap.intern_str(a.as_bytes())))
        .collect();
    for (i, v) in handles.into_iter().enumerate() {
        let _ = t.set(Value::Number((i + 1) as f64), v);
    }
    let arg_handle = state.global.heap.alloc_table(t);
    if let GcHandle::Table(g) = state.global.globals {
        let key = state.global.heap.intern_str(b"arg");
        if let Some(globals) = state.global.heap.get_table_mut(g) {
            let _ = globals.set(Value::GcRef(key), Value::GcRef(arg_handle));
        }
    }
}

/// 未捕捉エラーを本家 `lua.c` に寄せて整形する。
///
/// Lua のエラーオブジェクトは任意の値を取りうる。文字列ならそのまま、数値なら数値表現、
/// それ以外で `__tostring` も無い場合は本家同様 `(error object is a <type> value)` とする。
pub fn render_error(state: &LuaState, e: &LuaError) -> String {
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
