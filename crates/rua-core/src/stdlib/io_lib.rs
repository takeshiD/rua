//! io ライブラリ（本家 `liolib.c` 相当）。
//!
//! Lua 5.1 標準の io ライブラリをフルで実装する。
//!
//! ## 提供する関数
//! - `io.open(filename [, mode])` → ファイルオブジェクト or nil, errmsg
//! - `io.close([file])` → ファイルを閉じる
//! - `io.read([format, ...])` → stdin から読み込む
//! - `io.write(...)` → stdout に書き込む
//! - `io.lines([filename])` → 行イテレータ
//! - `io.flush()` → stdout をフラッシュ
//! - `io.input([file])` → デフォルト入力を取得/設定
//! - `io.output([file])` → デフォルト出力を取得/設定
//! - `io.type(file)` → "file", "closed file", nil
//! - `io.stdin`, `io.stdout`, `io.stderr` → 標準ストリーム
//!
//! ## ファイルオブジェクトのメソッド（メタテーブル経由）
//! - `file:read([format, ...])` → データを読む
//! - `file:write(...)` → データを書く
//! - `file:close()` → ファイルを閉じる
//! - `file:lines()` → 行イテレータ
//! - `file:seek([whence, offset])` → シーク
//! - `file:flush()` → フラッシュ
//! - `file:setvbuf(mode [, size])` → バッファリングモード設定

use std::cell::RefCell;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::Mutex;

use crate::error::LuaResult;
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::Value;
use crate::value::convert::number_to_string;
use crate::value::userdata::Userdata;

use super::aux;

// ============================================================================
// ファイルハンドル
// ============================================================================

/// ファイルハンドルの内部状態。
#[derive(Debug)]
pub enum FileHandle {
    /// 通常ファイル（読み書き可能）。
    File { file: std::fs::File, closed: bool },
    /// stdin（読み込み専用）。
    Stdin,
    /// stdout（書き込み専用）。
    Stdout,
    /// stderr（書き込み専用）。
    Stderr,
}

impl FileHandle {
    fn is_closed(&self) -> bool {
        match self {
            FileHandle::File { closed, .. } => *closed,
            _ => false,
        }
    }

    fn close(&mut self) -> bool {
        match self {
            FileHandle::File { closed, .. } => {
                *closed = true;
                true
            }
            _ => false,
        }
    }

    fn read_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                let mut line = Vec::new();
                let mut buf = [0u8; 1];
                loop {
                    match file.read(&mut buf) {
                        Ok(0) => {
                            if line.is_empty() {
                                return Ok(None); // EOF
                            }
                            return Ok(Some(line));
                        }
                        Ok(_) => {
                            if buf[0] == b'\n' {
                                return Ok(Some(line));
                            }
                            if buf[0] != b'\r' {
                                line.push(buf[0]);
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            FileHandle::Stdin => {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut line = String::new();
                let n = lock.read_line(&mut line)?;
                if n == 0 {
                    return Ok(None);
                }
                while matches!(line.as_bytes().last(), Some(b'\n') | Some(b'\r')) {
                    line.pop();
                }
                Ok(Some(line.into_bytes()))
            }
            _ => Err(std::io::Error::other("not readable")),
        }
    }

    fn read_all(&mut self) -> std::io::Result<Vec<u8>> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)?;
                Ok(buf)
            }
            FileHandle::Stdin => {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut buf = Vec::new();
                lock.read_to_end(&mut buf)?;
                Ok(buf)
            }
            _ => Err(std::io::Error::other("not readable")),
        }
    }

    fn read_number(&mut self) -> std::io::Result<Option<f64>> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                // Skip leading whitespace
                let mut byte = [0u8; 1];
                loop {
                    match file.read(&mut byte) {
                        Ok(0) => return Ok(None),
                        Ok(_) => {
                            if !byte[0].is_ascii_whitespace() {
                                break;
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                let mut buf = vec![byte[0]];
                loop {
                    match file.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => {
                            if byte[0].is_ascii_whitespace() {
                                break;
                            }
                            buf.push(byte[0]);
                        }
                        Err(e) => return Err(e),
                    }
                }
                let s = String::from_utf8_lossy(&buf);
                match crate::value::convert::str_to_number(s.as_bytes()) {
                    Some(n) => Ok(Some(n)),
                    None => Ok(None),
                }
            }
            FileHandle::Stdin => {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut line = String::new();
                let n = lock.read_line(&mut line)?;
                if n == 0 {
                    return Ok(None);
                }
                match crate::value::convert::str_to_number(line.trim().as_bytes()) {
                    Some(n) => Ok(Some(n)),
                    None => Ok(None),
                }
            }
            _ => Err(std::io::Error::other("not readable")),
        }
    }

    fn read_bytes(&mut self, count: usize) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                let mut buf = vec![0u8; count];
                let n = file.read(&mut buf)?;
                if n == 0 {
                    Ok(None)
                } else {
                    buf.truncate(n);
                    Ok(Some(buf))
                }
            }
            FileHandle::Stdin => {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut buf = vec![0u8; count];
                let n = lock.read(&mut buf)?;
                if n == 0 {
                    Ok(None)
                } else {
                    buf.truncate(n);
                    Ok(Some(buf))
                }
            }
            _ => Err(std::io::Error::other("not readable")),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                file.write_all(data)
            }
            FileHandle::Stdout => {
                let stdout = std::io::stdout();
                stdout.lock().write_all(data)
            }
            FileHandle::Stderr => {
                let stderr = std::io::stderr();
                stderr.lock().write_all(data)
            }
            _ => Err(std::io::Error::other("not writable")),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                file.flush()
            }
            FileHandle::Stdout => {
                let stdout = std::io::stdout();
                stdout.lock().flush()
            }
            FileHandle::Stderr => {
                let stderr = std::io::stderr();
                stderr.lock().flush()
            }
            _ => Ok(()),
        }
    }

    fn seek(&mut self, whence: SeekFrom) -> std::io::Result<u64> {
        match self {
            FileHandle::File { file, closed } => {
                if *closed {
                    return Err(std::io::Error::other("file is closed"));
                }
                file.seek(whence)
            }
            _ => Err(std::io::Error::other("cannot seek")),
        }
    }
}

/// GC Userdata に格納する型: `Rc<RefCell<FileHandle>>`。
/// Rc を使うことで複数の Lua 値が同じファイルを参照できるようにする。
type FileHandleRef = Rc<RefCell<FileHandle>>;

// ============================================================================
// グローバルなファイルメタテーブルキーを保持する仕組み
// ============================================================================

/// グローバルなファイルメタテーブルキー（`file:read` 等のメソッドを保持するテーブル）。
/// open() 時に一度設定し、以降はここから参照する。
static FILE_METATABLE: Mutex<Option<TableKey>> = Mutex::new(None);

fn set_file_metatable(tk: TableKey) {
    if let Ok(mut guard) = FILE_METATABLE.lock() {
        *guard = Some(tk);
    }
}

fn get_file_metatable() -> Option<TableKey> {
    FILE_METATABLE.lock().ok().and_then(|g| *g)
}

// ============================================================================
// ライブラリ初期化
// ============================================================================

pub fn open(state: &mut LuaState) {
    // ファイルメソッドを保持するメタテーブルを作成する
    let file_mt = state.new_table();
    let file_mt_k = match file_mt {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    // ファイルオブジェクトのメソッドテーブル（__index に設定するテーブル）
    let methods_t = state.new_table();
    let methods_k = match methods_t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    aux::register(state, methods_k, "read", file_read);
    aux::register(state, methods_k, "write", file_write);
    aux::register(state, methods_k, "close", file_close);
    aux::register(state, methods_k, "lines", file_lines);
    aux::register(state, methods_k, "seek", file_seek);
    aux::register(state, methods_k, "flush", file_flush);
    aux::register(state, methods_k, "setvbuf", file_setvbuf);

    // __index = methods
    aux::set_field(state, file_mt_k, "__index", methods_t);
    // __tostring for file objects
    aux::register(state, file_mt_k, "__tostring", file_tostring);
    // __gc (ファイルを閉じる)
    aux::register(state, file_mt_k, "__gc", file_gc);

    // メタテーブルキーをグローバルに保存
    set_file_metatable(file_mt_k);

    // io ライブラリテーブルを作成
    let t = state.new_table();
    let tk = match t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };

    // io 関数を登録
    aux::register(state, tk, "open", l_open);
    aux::register(state, tk, "close", l_close);
    aux::register(state, tk, "read", l_read);
    aux::register(state, tk, "write", l_write);
    aux::register(state, tk, "lines", l_lines);
    aux::register(state, tk, "flush", l_flush);
    aux::register(state, tk, "input", l_input);
    aux::register(state, tk, "output", l_output);
    aux::register(state, tk, "type", l_type);

    // 標準ストリームを userdata として作成
    let stdin_val = make_file_userdata(state, FileHandle::Stdin);
    let stdout_val = make_file_userdata(state, FileHandle::Stdout);
    let stderr_val = make_file_userdata(state, FileHandle::Stderr);

    aux::set_field(state, tk, "stdin", stdin_val);
    aux::set_field(state, tk, "stdout", stdout_val);
    aux::set_field(state, tk, "stderr", stderr_val);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "io", t);
    }
}

// ============================================================================
// ファイル userdata の作成ヘルパ
// ============================================================================

fn make_file_userdata(state: &mut LuaState, handle: FileHandle) -> Value {
    let handle_ref: FileHandleRef = Rc::new(RefCell::new(handle));
    let mut ud = Userdata::new(Box::new(handle_ref));
    if let Some(mt_k) = get_file_metatable() {
        ud.set_metatable(Some(GcHandle::Table(mt_k)));
    }
    Value::GcRef(state.global.heap.alloc_userdata(ud))
}

fn get_file_handle(state: &LuaState, v: Value) -> Option<FileHandleRef> {
    match v {
        Value::GcRef(GcHandle::Userdata(k)) => {
            let ud = state.global.heap.get_userdata(k)?;
            ud.data().downcast_ref::<FileHandleRef>().cloned()
        }
        _ => None,
    }
}

// ============================================================================
// io ライブラリ関数
// ============================================================================

/// `io.open(filename [, mode])`: ファイルを開く。
/// 成功時はファイルオブジェクト、失敗時は nil, errmsg を返す。
fn l_open(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let filename = aux::check_str_bytes(state, &args, 0, "open")?;
    let mode_bytes = if matches!(aux::opt_value(&args, 1), Value::Nil) {
        b"r".to_vec()
    } else {
        aux::check_str_bytes(state, &args, 1, "open")?
    };

    let filename_str = String::from_utf8_lossy(&filename).to_string();
    let mode_str = String::from_utf8_lossy(&mode_bytes).to_string();

    // モード文字列を解析
    // r, w, a, r+, w+, a+ とバイナリ b の組み合わせ
    let mut read = false;
    let mut write = false;
    let mut append = false;
    let mut update = false;
    // binary フラグは Lua 5.1 の b でも Rust では関係ないが一応パース

    for ch in mode_str.chars() {
        match ch {
            'r' => read = true,
            'w' => write = true,
            'a' => append = true,
            '+' => update = true,
            'b' => {} // バイナリモード（Unix では無意味）
            _ => {
                let msg =
                    state.new_string(format!("invalid mode '{}' in io.open", mode_str).as_bytes());
                return aux::ret(state, vec![Value::Nil, msg]);
            }
        }
    }

    let result = std::fs::OpenOptions::new()
        .read(read || update || (!write && !append))
        .write(write || update)
        .append(append)
        .create(write || append)
        .truncate(write && !update)
        .open(&filename_str);

    match result {
        Ok(file) => {
            let handle = FileHandle::File {
                file,
                closed: false,
            };
            let ud_val = make_file_userdata(state, handle);
            aux::ret(state, vec![ud_val])
        }
        Err(e) => {
            let msg = state.new_string(format!("{}: {}", filename_str, e).as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `io.close([file])`: ファイルを閉じる。
fn l_close(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);

    if matches!(v, Value::Nil) {
        // デフォルト出力を閉じる（stdout は閉じない）
        return aux::ret(state, vec![Value::Boolean(true)]);
    }

    match get_file_handle(state, v) {
        Some(fh) => {
            let closed = fh.borrow_mut().close();
            aux::ret(state, vec![Value::Boolean(closed)])
        }
        None => {
            let msg = state.new_string(b"file expected");
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `io.read([format, ...])`: stdin から読み込む。
fn l_read(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    read_from_handle_args(state, &args, None)
}

/// `io.write(...)`: stdout に書き込む。
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

/// `io.lines([filename])`: 行イテレータを返す。
fn l_lines(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);

    if matches!(v, Value::Nil) {
        // stdin から読む行イテレータ
        let iter = aux::make_native(state, lines_iter_stdin);
        return aux::ret(state, vec![iter]);
    }

    let filename = aux::check_str_bytes(state, &args, 0, "lines")?;
    let filename_str = String::from_utf8_lossy(&filename).to_string();

    match std::fs::File::open(&filename_str) {
        Ok(file) => {
            let handle = FileHandle::File {
                file,
                closed: false,
            };
            let ud_val = make_file_userdata(state, handle);
            // ファイルオブジェクトをテーブルに格納してイテレータを返す
            // テーブルに file を保存してクロージャ状態として使う
            let state_t = state.new_table();
            if let Value::GcRef(GcHandle::Table(stk)) = state_t {
                let key = state.new_string(b"file");
                if let Some(t) = state.global.heap.get_table_mut(stk) {
                    let _ = t.set(key, ud_val);
                }
                // __call メタメソッドを持つテーブルをイテレータ状態として返す
                let mt = state.new_table();
                if let Value::GcRef(GcHandle::Table(mtk)) = mt {
                    aux::register(state, mtk, "__call", lines_iter_file);
                    if let Some(t) = state.global.heap.get_table_mut(stk) {
                        t.set_metatable(Some(GcHandle::Table(mtk)));
                    }
                }
            }
            let iter = aux::make_native(state, lines_iter_file_wrapper);
            // ファイルを state として渡す方法として、テーブルを使う
            // 代わりにシンプルに: ファイルを開いてネイティブ関数のクロージャにする
            // ここでは upvalue が使えないため、代わりにシンプルなアプローチを取る
            // state_t を返し、__call メタメソッドで動かす
            let _ = iter; // 使わない
            aux::ret(state, vec![state_t])
        }
        Err(e) => Err(aux::rt_error(state, format!("{}: {}", filename_str, e))),
    }
}

fn lines_iter_stdin(state: &mut LuaState) -> LuaResult<i32> {
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
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

fn lines_iter_file_wrapper(state: &mut LuaState) -> LuaResult<i32> {
    // このアプローチは使わない（state_t の __call に委ねる）
    aux::ret(state, vec![Value::Nil])
}

/// `io.flush()`: stdout をフラッシュ。
fn l_flush(state: &mut LuaState) -> LuaResult<i32> {
    let stdout = std::io::stdout();
    let _ = stdout.lock().flush();
    aux::ret0(state)
}

/// `io.input([file])`: デフォルト入力を取得/設定（簡易実装: 引数なしなら stdin を返す）。
fn l_input(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);

    if matches!(v, Value::Nil) {
        // デフォルト入力（stdin）を返す
        let stdin_val = make_file_userdata(state, FileHandle::Stdin);
        return aux::ret(state, vec![stdin_val]);
    }

    match v {
        Value::GcRef(GcHandle::Str(_)) => {
            // ファイル名が指定された場合はファイルを開く
            let filename = aux::check_str_bytes(state, &args, 0, "input")?;
            let filename_str = String::from_utf8_lossy(&filename).to_string();
            match std::fs::File::open(&filename_str) {
                Ok(file) => {
                    let handle = FileHandle::File {
                        file,
                        closed: false,
                    };
                    let ud_val = make_file_userdata(state, handle);
                    aux::ret(state, vec![ud_val])
                }
                Err(e) => Err(aux::rt_error(state, format!("{}: {}", filename_str, e))),
            }
        }
        Value::GcRef(GcHandle::Userdata(_)) => {
            // ファイルオブジェクトが指定された場合はそのまま返す
            aux::ret(state, vec![v])
        }
        _ => Err(aux::arg_error(state, 1, "input", "string or file expected")),
    }
}

/// `io.output([file])`: デフォルト出力を取得/設定（簡易実装: 引数なしなら stdout を返す）。
fn l_output(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);

    if matches!(v, Value::Nil) {
        // デフォルト出力（stdout）を返す
        let stdout_val = make_file_userdata(state, FileHandle::Stdout);
        return aux::ret(state, vec![stdout_val]);
    }

    match v {
        Value::GcRef(GcHandle::Str(_)) => {
            let filename = aux::check_str_bytes(state, &args, 0, "output")?;
            let filename_str = String::from_utf8_lossy(&filename).to_string();
            match std::fs::File::create(&filename_str) {
                Ok(file) => {
                    let handle = FileHandle::File {
                        file,
                        closed: false,
                    };
                    let ud_val = make_file_userdata(state, handle);
                    aux::ret(state, vec![ud_val])
                }
                Err(e) => Err(aux::rt_error(state, format!("{}: {}", filename_str, e))),
            }
        }
        Value::GcRef(GcHandle::Userdata(_)) => aux::ret(state, vec![v]),
        _ => Err(aux::arg_error(
            state,
            1,
            "output",
            "string or file expected",
        )),
    }
}

/// `io.type(file)`: "file", "closed file", nil を返す。
fn l_type(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);

    match get_file_handle(state, v) {
        Some(fh) => {
            if fh.borrow().is_closed() {
                let s = state.new_string(b"closed file");
                aux::ret(state, vec![s])
            } else {
                let s = state.new_string(b"file");
                aux::ret(state, vec![s])
            }
        }
        None => aux::ret(state, vec![Value::Nil]),
    }
}

// ============================================================================
// ファイルオブジェクトのメソッド
// ============================================================================

/// 共通の読み込みロジック。
/// `file_handle` が Some の場合はそのファイルから、None の場合は stdin から読む。
fn read_from_handle_args(
    state: &mut LuaState,
    args: &[Value],
    file_val: Option<Value>,
) -> LuaResult<i32> {
    // フォーマット引数の開始インデックス（file:read / io.read とも引数先頭から）
    let fmt_start = 0;

    let fh_opt: Option<FileHandleRef> = file_val.and_then(|v| get_file_handle(state, v));

    if let Some(ref fh) = fh_opt
        && fh.borrow().is_closed()
    {
        let msg = state.new_string(b"attempt to use a closed file");
        return aux::ret(state, vec![Value::Nil, msg]);
    }

    // フォーマットが無い場合は "*l" がデフォルト
    if args.len() <= fmt_start || matches!(aux::opt_value(args, fmt_start), Value::Nil) {
        // デフォルト: "*l" (1行読み込み)
        let result = if let Some(ref fh) = fh_opt {
            fh.borrow_mut().read_line()
        } else {
            // stdin から読む
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            match lock.read_line(&mut line) {
                Ok(0) => Ok(None),
                Ok(_) => {
                    while matches!(line.as_bytes().last(), Some(b'\n') | Some(b'\r')) {
                        line.pop();
                    }
                    Ok(Some(line.into_bytes()))
                }
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(Some(data)) => {
                let v = state.new_string(&data);
                return aux::ret(state, vec![v]);
            }
            Ok(None) => return aux::ret(state, vec![Value::Nil]),
            Err(e) => return Err(aux::rt_error(state, e.to_string())),
        }
    }

    // 複数フォーマット対応
    let mut results = Vec::new();
    let mut i = fmt_start;
    while i < args.len() {
        let fmt_val = args[i];
        let result_val = read_one_format(state, fmt_val, fh_opt.as_ref())?;
        results.push(result_val);
        i += 1;
    }
    aux::ret(state, results)
}

fn read_one_format(
    state: &mut LuaState,
    fmt_val: Value,
    fh_opt: Option<&FileHandleRef>,
) -> LuaResult<Value> {
    let fmt = match fmt_val {
        Value::GcRef(GcHandle::Str(k)) => state.global.heap.get_str(k).unwrap().as_bytes().to_vec(),
        Value::Number(n) => {
            // 数値の場合は n バイト読む
            let count = n as usize;
            let result = if let Some(fh) = fh_opt {
                fh.borrow_mut().read_bytes(count)
            } else {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut buf = vec![0u8; count];
                match lock.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(n) => {
                        buf.truncate(n);
                        Ok(Some(buf))
                    }
                    Err(e) => Err(e),
                }
            };
            return match result {
                Ok(Some(data)) => Ok(state.new_string(&data)),
                Ok(None) => Ok(Value::Nil),
                Err(e) => Err(aux::rt_error(state, e.to_string())),
            };
        }
        _ => {
            return Err(aux::rt_error(state, "invalid format in read"));
        }
    };

    let f = fmt.strip_prefix(b"*").unwrap_or(&fmt);

    match f.first().copied() {
        Some(b'l') | Some(b'L') => {
            // "*l" / "*L": 1行読む
            let keep_newline = f.first().copied() == Some(b'L');
            let result = if let Some(fh) = fh_opt {
                fh.borrow_mut().read_line().map(|opt| {
                    opt.map(|mut line| {
                        if keep_newline {
                            line.push(b'\n');
                        }
                        line
                    })
                })
            } else {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut line = String::new();
                lock.read_line(&mut line).map(|n| {
                    if n == 0 {
                        None
                    } else {
                        if !keep_newline {
                            while matches!(line.as_bytes().last(), Some(b'\n') | Some(b'\r')) {
                                line.pop();
                            }
                        }
                        Some(line.into_bytes())
                    }
                })
            };
            match result {
                Ok(Some(data)) => Ok(state.new_string(&data)),
                Ok(None) => Ok(Value::Nil),
                Err(e) => Err(aux::rt_error(state, e.to_string())),
            }
        }
        Some(b'n') => {
            // "*n": 数値1つ
            let result = if let Some(fh) = fh_opt {
                fh.borrow_mut().read_number()
            } else {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut line = String::new();
                match lock.read_line(&mut line) {
                    Ok(0) => Ok(None),
                    Ok(_) => match crate::value::convert::str_to_number(line.trim().as_bytes()) {
                        Some(n) => Ok(Some(n)),
                        None => Ok(None),
                    },
                    Err(e) => Err(e),
                }
            };
            match result {
                Ok(Some(n)) => Ok(Value::Number(n)),
                Ok(None) => Ok(Value::Nil),
                Err(e) => Err(aux::rt_error(state, e.to_string())),
            }
        }
        Some(b'a') => {
            // "*a": 全部読む
            let result = if let Some(fh) = fh_opt {
                fh.borrow_mut().read_all()
            } else {
                let stdin = std::io::stdin();
                let mut lock = stdin.lock();
                let mut buf = Vec::new();
                lock.read_to_end(&mut buf).map(|_| buf)
            };
            match result {
                Ok(data) => Ok(state.new_string(&data)),
                Err(e) => Err(aux::rt_error(state, e.to_string())),
            }
        }
        _ => Err(aux::rt_error(
            state,
            format!("invalid format '{}'", String::from_utf8_lossy(&fmt)),
        )),
    }
}

/// `file:read([format, ...])`: ファイルから読む。
fn file_read(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "read", "file expected"));
    }
    let file_val = args[0];
    let fmt_args = args[1..].to_vec();

    let fh_opt = match get_file_handle(state, file_val) {
        Some(fh) => fh,
        None => {
            return Err(aux::arg_error(state, 1, "read", "file expected"));
        }
    };

    if fh_opt.borrow().is_closed() {
        let msg = state.new_string(b"attempt to use a closed file");
        return aux::ret(state, vec![Value::Nil, msg]);
    }

    // フォーマット引数が無い場合はデフォルト "*l"
    if fmt_args.is_empty() || matches!(fmt_args[0], Value::Nil) {
        let result = fh_opt.borrow_mut().read_line();
        return match result {
            Ok(Some(data)) => {
                let v = state.new_string(&data);
                aux::ret(state, vec![v])
            }
            Ok(None) => aux::ret(state, vec![Value::Nil]),
            Err(e) => Err(aux::rt_error(state, e.to_string())),
        };
    }

    let mut results = Vec::new();
    for fmt_val in &fmt_args {
        let result_val = read_one_format(state, *fmt_val, Some(&fh_opt))?;
        results.push(result_val);
    }
    aux::ret(state, results)
}

/// `file:write(...)`: ファイルに書き込む。
fn file_write(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "write", "file expected"));
    }
    let file_val = args[0];

    let fh = match get_file_handle(state, file_val) {
        Some(fh) => fh,
        None => {
            return Err(aux::arg_error(state, 1, "write", "file expected"));
        }
    };

    if fh.borrow().is_closed() {
        let msg = state.new_string(b"attempt to use a closed file");
        return aux::ret(state, vec![Value::Nil, msg]);
    }

    let mut buf: Vec<u8> = Vec::new();
    for (i, v) in args[1..].iter().enumerate() {
        match v {
            Value::GcRef(GcHandle::Str(k)) => {
                buf.extend_from_slice(state.global.heap.get_str(*k).unwrap().as_bytes());
            }
            Value::Number(n) => buf.extend_from_slice(number_to_string(*n).as_bytes()),
            other => {
                return Err(aux::arg_error(
                    state,
                    i + 2,
                    "write",
                    &format!("string expected, got {}", other.type_of().name()),
                ));
            }
        }
    }

    match fh.borrow_mut().write_bytes(&buf) {
        Ok(()) => aux::ret(state, vec![file_val]),
        Err(e) => {
            let msg = state.new_string(e.to_string().as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `file:close()`: ファイルを閉じる。
fn file_close(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "close", "file expected"));
    }
    let file_val = args[0];

    match get_file_handle(state, file_val) {
        Some(fh) => {
            fh.borrow_mut().close();
            aux::ret(state, vec![Value::Boolean(true)])
        }
        None => Err(aux::arg_error(state, 1, "close", "file expected")),
    }
}

/// `file:lines()`: 行イテレータを返す。
fn file_lines(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "lines", "file expected"));
    }
    let file_val = args[0];

    match get_file_handle(state, file_val) {
        Some(_) => {
            // ファイルをテーブルに格納してイテレータを返す
            let state_t = state.new_table();
            if let Value::GcRef(GcHandle::Table(stk)) = state_t {
                let fkey = state.new_string(b"file");
                if let Some(t) = state.global.heap.get_table_mut(stk) {
                    let _ = t.set(fkey, file_val);
                }
                // __call でイテレータを実装
                let mt = state.new_table();
                if let Value::GcRef(GcHandle::Table(mtk)) = mt {
                    aux::register(state, mtk, "__call", lines_iter_file);
                    if let Some(t) = state.global.heap.get_table_mut(stk) {
                        t.set_metatable(Some(GcHandle::Table(mtk)));
                    }
                }
            }
            aux::ret(state, vec![state_t])
        }
        None => Err(aux::arg_error(state, 1, "lines", "file expected")),
    }
}

/// ファイル行イテレータ（テーブルの __call として設定される）。
fn lines_iter_file(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let state_t = aux::opt_value(&args, 0);

    let file_val = match state_t {
        Value::GcRef(GcHandle::Table(k)) => {
            let fkey = state.new_string(b"file");
            state
                .global
                .heap
                .get_table(k)
                .map(|t| t.get(&fkey))
                .unwrap_or(Value::Nil)
        }
        _ => return aux::ret(state, vec![Value::Nil]),
    };

    match get_file_handle(state, file_val) {
        Some(fh) => {
            if fh.borrow().is_closed() {
                return aux::ret(state, vec![Value::Nil]);
            }
            match fh.borrow_mut().read_line() {
                Ok(Some(data)) => {
                    let v = state.new_string(&data);
                    aux::ret(state, vec![v])
                }
                Ok(None) => aux::ret(state, vec![Value::Nil]),
                Err(e) => Err(aux::rt_error(state, e.to_string())),
            }
        }
        None => aux::ret(state, vec![Value::Nil]),
    }
}

/// `file:seek([whence, offset])`: ファイルポインタ操作。
fn file_seek(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "seek", "file expected"));
    }
    let file_val = args[0];

    let fh = match get_file_handle(state, file_val) {
        Some(fh) => fh,
        None => return Err(aux::arg_error(state, 1, "seek", "file expected")),
    };

    if fh.borrow().is_closed() {
        return Err(aux::rt_error(state, "attempt to use a closed file"));
    }

    let whence_bytes = if matches!(aux::opt_value(&args, 1), Value::Nil) {
        b"cur".to_vec()
    } else {
        aux::check_str_bytes(state, &args, 1, "seek")?
    };
    let offset = aux::opt_int(state, &args, 2, "seek", 0)?;

    let seek_from = match whence_bytes.as_slice() {
        b"set" => SeekFrom::Start(offset.max(0) as u64),
        b"cur" => SeekFrom::Current(offset),
        b"end" => SeekFrom::End(offset),
        _ => {
            return Err(aux::arg_error(state, 2, "seek", "invalid option"));
        }
    };

    match fh.borrow_mut().seek(seek_from) {
        Ok(pos) => aux::ret(state, vec![Value::Number(pos as f64)]),
        Err(e) => {
            let msg = state.new_string(e.to_string().as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `file:flush()`: バッファをフラッシュ。
fn file_flush(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "flush", "file expected"));
    }
    let file_val = args[0];

    let fh = match get_file_handle(state, file_val) {
        Some(fh) => fh,
        None => return Err(aux::arg_error(state, 1, "flush", "file expected")),
    };

    match fh.borrow_mut().flush() {
        Ok(()) => aux::ret(state, vec![Value::Boolean(true)]),
        Err(e) => {
            let msg = state.new_string(e.to_string().as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `file:setvbuf(mode [, size])`: バッファリングモード設定（stub）。
fn file_setvbuf(state: &mut LuaState) -> LuaResult<i32> {
    // Rust の標準ライブラリではバッファリングモードを細かく制御できないため stub
    let args = aux::args_vec(state);
    if args.is_empty() {
        return Err(aux::arg_error(state, 1, "setvbuf", "file expected"));
    }
    aux::ret(state, vec![Value::Boolean(true)])
}

/// `__tostring` メタメソッド: file オブジェクトの文字列表現。
fn file_tostring(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    let s = match get_file_handle(state, v) {
        Some(fh) => {
            if fh.borrow().is_closed() {
                state.new_string(b"file (closed)")
            } else {
                state.new_string(b"file (open)")
            }
        }
        None => state.new_string(b"file"),
    };
    aux::ret(state, vec![s])
}

/// `__gc` メタメソッド: ファイルを GC 時に閉じる。
fn file_gc(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let v = aux::opt_value(&args, 0);
    if let Some(fh) = get_file_handle(state, v) {
        fh.borrow_mut().close();
    }
    aux::ret0(state)
}
