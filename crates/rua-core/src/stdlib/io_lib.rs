//! io ライブラリ（本家 `liolib.c` 相当・最小実装）。担当: **lua-stdlib**。
//!
//! 第一マイルストーンでは標準出力への `io.write`（改行なし・引数連結・数値は文字列化）と
//! `io.read`（行読み込み）の基本のみを提供する。ファイルハンドル（userdata）や `io.open`、
//! `io.lines` 等は後続フェーズで実装する。

use std::io::{BufRead, Write};

use crate::error::LuaResult;
use crate::gc::GcHandle;
use crate::state::LuaState;
use crate::value::Value;
use crate::value::convert::number_to_string;

use super::aux;

pub fn open(state: &mut LuaState) {
    let t = state.new_table();
    let tk = match t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };
    aux::register(state, tk, "write", l_write);
    aux::register(state, tk, "read", l_read);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "io", t);
    }
}

/// `io.write(...)`: 各引数を改行なしで標準出力へ連結出力する。
///
/// 文字列はそのまま、数値は `tostring` 規則で文字列化。それ以外は型エラー。
fn l_write(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut buf: Vec<u8> = Vec::new();
    for (i, v) in args.iter().enumerate() {
        match v {
            Value::GcRef(GcHandle::Str(k)) => {
                buf.extend_from_slice(state.global.heap.get_str(*k).unwrap().as_bytes());
            }
            Value::Number(n) => buf.extend_from_slice(number_to_string(*n).as_bytes()),
            other => {
                return Err(aux::arg_error(
                    state,
                    i + 1,
                    "write",
                    &format!("string expected, got {}", other.type_of().name()),
                ));
            }
        }
    }
    let stdout = std::io::stdout();
    let _ = stdout.lock().write_all(&buf);
    aux::ret0(state)
}

/// `io.read([fmt])`: 標準入力から読む（最小: 1 行 / `"*l"` / `"*n"` / `"*a"`）。
fn l_read(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let fmt = match aux::opt_value(&args, 0) {
        Value::Nil => b"*l".to_vec(),
        _ => aux::check_str_bytes(state, &args, 0, "read")?,
    };
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let f = fmt.strip_prefix(b"*").unwrap_or(&fmt);
    match f.first().copied() {
        Some(b'a') => {
            // 全入力。
            let mut s = Vec::new();
            let _ = std::io::Read::read_to_end(&mut lock, &mut s);
            let v = state.new_string(&s);
            aux::ret(state, vec![v])
        }
        Some(b'n') => {
            // 数値 1 つ。
            let mut line = String::new();
            let _ = lock.read_line(&mut line);
            match crate::value::convert::str_to_number(line.trim().as_bytes()) {
                Some(n) => aux::ret(state, vec![Value::Number(n)]),
                None => aux::ret(state, vec![Value::Nil]),
            }
        }
        _ => {
            // 1 行（改行を除く）。EOF は nil。
            let mut line = String::new();
            let read = lock.read_line(&mut line).unwrap_or(0);
            if read == 0 {
                return aux::ret(state, vec![Value::Nil]);
            }
            while matches!(line.as_bytes().last(), Some(b'\n') | Some(b'\r')) {
                line.pop();
            }
            let v = state.new_string(line.as_bytes());
            aux::ret(state, vec![v])
        }
    }
}
