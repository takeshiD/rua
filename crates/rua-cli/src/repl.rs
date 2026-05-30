//! 対話モード REPL（本家 `lua.c` の対話ループ相当）。
//!
//! `rua`（引数なし）または `rua repl` で起動する。
//! reedline を用いたリッチな対話体験を提供する：
//!   - シンタックスハイライト（キーワード/文字列/数値/コメント/演算子を色分け）
//!   - Tab 補完（グローバル変数 + Lua 予約語）
//!   - 複数行継続（`near '<eof>'` エラーで継続プロンプトを表示）
//!   - 式の評価表示（`= expr` または単純式の自動 return）
//!   - ファイル永続履歴
//!   - 非 tty（パイプ）フォールバック
//!
//! 本家 `lua.c` の対話ループ（`lua_readline`/`lua_saveline`/`loadline`/`dotty`）に相当する
//! が、UX を IPython/bpython 風に充実させる。

use std::borrow::Cow;
use std::io::{self, BufRead};
use std::process::ExitCode;
use std::rc::Rc;

use nu_ansi_term::{Color, Style};
use reedline::{
    Completer, FileBackedHistory, Highlighter, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineMenu, Signal, Span, StyledText, Suggestion,
    ColumnarMenu, MenuBuilder,
    default_emacs_keybindings, Emacs, KeyCode, KeyModifiers, ReedlineEvent,
};

use rua_core::compiler::compile;
use rua_core::error::LuaError;
use rua_core::gc::GcHandle;
use rua_core::state::{call::pcall, LuaState};
use rua_core::stdlib;
use rua_core::value::Value;
use rua_core::vm::{call as vm_call, run};

use crate::run::render_error;

// ============================================================================
// REPL バージョン文字列
// ============================================================================

const RUA_VERSION: &str = env!("CARGO_PKG_VERSION");
const LUA_VERSION: &str = "Lua 5.1";

// ============================================================================
// Lua 5.1 予約語一覧
// ============================================================================

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

// ============================================================================
// プロンプト実装
// ============================================================================

/// rua REPL のプロンプト（本家 `lua.c` の `LUA_PROMPT`/`LUA_PROMPT2` 相当）。
///
/// - 通常プロンプト: `rua> `（青色）
/// - 継続プロンプト: `   ...> `（シアン）
struct LuaPrompt {
    /// true のとき継続プロンプトを表示する。
    is_continuation: bool,
}

impl LuaPrompt {
    fn new() -> Self {
        LuaPrompt {
            is_continuation: false,
        }
    }
}

impl Prompt for LuaPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _edit_mode: PromptEditMode) -> Cow<'_, str> {
        if self.is_continuation {
            Cow::Borrowed("   ...> ")
        } else {
            Cow::Borrowed("rua> ")
        }
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("   ...> ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!("({}reverse-search: {}) ", prefix, history_search.term))
    }
}

// ============================================================================
// シンタックスハイライト実装
// ============================================================================

/// Lua 5.1 シンタックスハイライト（reedline `Highlighter` トレイト実装）。
///
/// rua_core の lexer を使わず軽量な手書きスキャナを用いる。
/// これは（1）未完入力でもクラッシュしないため、（2）lexer が内部実装に依存するため。
struct LuaHighlighter;

/// ハイライト用トークン種別。
#[derive(Clone, Copy, PartialEq, Eq)]
enum HlKind {
    Keyword,
    String,
    Number,
    Comment,
    Operator,
    Name,
    Other,
}

impl HlKind {
    fn style(self) -> Style {
        match self {
            HlKind::Keyword  => Style::new().fg(Color::Cyan).bold(),
            HlKind::String   => Style::new().fg(Color::Green),
            HlKind::Number   => Style::new().fg(Color::Yellow),
            HlKind::Comment  => Style::new().fg(Color::DarkGray).italic(),
            HlKind::Operator => Style::new().fg(Color::LightMagenta),
            HlKind::Name     => Style::new().fg(Color::White),
            HlKind::Other    => Style::new(),
        }
    }
}

/// 手書きの軽量 Lua トークナイザ（ハイライト専用）。
/// ソースを `(開始バイト位置, 終了バイト位置, 種別)` のリストに分解する。
fn tokenize_for_highlight(src: &str) -> Vec<(usize, usize, HlKind)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut tokens: Vec<(usize, usize, HlKind)> = Vec::new();
    let mut i = 0;

    while i < len {
        // 空白はスキップ（Other として記録しない）。
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // --- コメント ---
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            let start = i;
            i += 2;
            // 長括弧コメント `--[[` / `--[==[`
            if i < len && bytes[i] == b'[' {
                let eq_start = i + 1;
                let mut eq_count = 0;
                while eq_start + eq_count < len && bytes[eq_start + eq_count] == b'=' {
                    eq_count += 1;
                }
                let bracket_end = eq_start + eq_count;
                if bracket_end < len && bytes[bracket_end] == b'[' {
                    // 長括弧コメント開始。
                    i = bracket_end + 1;
                    // 対応する閉じ括弧 `]===]` を探す。
                    let close_pat = {
                        let mut p = b"]".to_vec();
                        p.extend(vec![b'='; eq_count]);
                        p.push(b']');
                        p
                    };
                    while i + close_pat.len() <= len {
                        if bytes[i..i + close_pat.len()] == close_pat[..] {
                            i += close_pat.len();
                            break;
                        }
                        i += 1;
                    }
                    tokens.push((start, i, HlKind::Comment));
                    continue;
                }
            }
            // 短コメント: 行末まで。
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            tokens.push((start, i, HlKind::Comment));
            continue;
        }

        // --- 文字列リテラル `"..."` / `'...'` ---
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2; // エスケープシーケンスをスキップ。
                } else if bytes[i] == q {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push((start, i, HlKind::String));
            continue;
        }

        // --- 長括弧文字列 `[[...]]` / `[==[...]==]` ---
        if bytes[i] == b'[' {
            let start = i;
            let eq_start = i + 1;
            let mut eq_count = 0;
            while eq_start + eq_count < len && bytes[eq_start + eq_count] == b'=' {
                eq_count += 1;
            }
            let bracket_end = eq_start + eq_count;
            if bracket_end < len && bytes[bracket_end] == b'[' {
                i = bracket_end + 1;
                let close_pat = {
                    let mut p = b"]".to_vec();
                    p.extend(vec![b'='; eq_count]);
                    p.push(b']');
                    p
                };
                while i + close_pat.len() <= len {
                    if bytes[i..i + close_pat.len()] == close_pat[..] {
                        i += close_pat.len();
                        break;
                    }
                    i += 1;
                }
                tokens.push((start, i, HlKind::String));
                continue;
            }
            // 通常の `[` 演算子。
            tokens.push((i, i + 1, HlKind::Operator));
            i += 1;
            continue;
        }

        // --- 数値リテラル ---
        if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            // 16進数 `0x`/`0X`
            if bytes[i] == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
            {
                i += 2;
                while i < len && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            } else {
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                // 指数部
                if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < len && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            tokens.push((start, i, HlKind::Number));
            continue;
        }

        // --- 識別子 / 予約語 ---
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let kind = if LUA_KEYWORDS.contains(&word) {
                HlKind::Keyword
            } else {
                HlKind::Name
            };
            tokens.push((start, i, kind));
            continue;
        }

        // --- 演算子・記号 ---
        // 2文字演算子を先にチェックする。
        if i + 1 < len {
            match (bytes[i], bytes[i + 1]) {
                (b'.', b'.') => {
                    let end = if i + 2 < len && bytes[i + 2] == b'.' {
                        i + 3
                    } else {
                        i + 2
                    };
                    tokens.push((i, end, HlKind::Operator));
                    i = end;
                    continue;
                }
                (b'=', b'=') | (b'~', b'=') | (b'<', b'=') | (b'>', b'=') => {
                    tokens.push((i, i + 2, HlKind::Operator));
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }

        // 1文字演算子・記号。
        match bytes[i] {
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'#' | b'<' | b'>' | b'=' | b'('
            | b')' | b'{' | b'}' | b']' | b';' | b':' | b',' | b'.' => {
                tokens.push((i, i + 1, HlKind::Operator));
                i += 1;
            }
            _ => {
                // その他（制御文字など）。
                tokens.push((i, i + 1, HlKind::Other));
                i += 1;
            }
        }
    }
    tokens
}

impl Highlighter for LuaHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        let tokens = tokenize_for_highlight(line);
        if tokens.is_empty() {
            // 空行・空白のみ: そのまま出力。
            styled.push((Style::new(), line.to_string()));
            return styled;
        }

        let mut prev_end = 0;
        for (start, end, kind) in tokens {
            // トークン前の空白。
            if start > prev_end {
                styled.push((Style::new(), line[prev_end..start].to_string()));
            }
            styled.push((kind.style(), line[start..end].to_string()));
            prev_end = end;
        }
        // 末尾の残り。
        if prev_end < line.len() {
            styled.push((Style::new(), line[prev_end..].to_string()));
        }
        styled
    }
}

// ============================================================================
// Tab 補完実装
// ============================================================================

/// Lua REPL の Tab 補完（reedline `Completer` トレイト実装）。
///
/// 補完候補: Lua 予約語 + グローバル環境のキー。
/// カーソル直前の識別子（`[a-zA-Z0-9_.]` パターン）を span として置換する。
///
/// `LuaState` の生存期間の問題があるため、補完時点でのグローバルキーのスナップショットを
/// 保持する。REPL ループが補完器を再構築するか `refresh_globals` を呼ぶことで最新化する。
pub(crate) struct LuaCompleter {
    /// グローバル変数名のスナップショット。
    globals: Vec<String>,
}

impl LuaCompleter {
    fn new() -> Self {
        LuaCompleter {
            globals: Vec::new(),
        }
    }

    /// グローバル環境テーブルのキーを列挙してスナップショットを更新する。
    pub(crate) fn refresh_globals(&mut self, state: &LuaState) {
        let mut names: Vec<String> = LUA_KEYWORDS.iter().map(|s| s.to_string()).collect();
        if let GcHandle::Table(gk) = state.global.globals
            && let Some(t) = state.global.heap.get_table(gk)
        {
            let mut key = Value::Nil;
            loop {
                match t.next(&key) {
                    Ok(Some((k, _v))) => {
                        // 文字列キーのみ補完対象にする。
                        if let Value::GcRef(GcHandle::Str(sk)) = k
                            && let Some(s) = state.global.heap.get_str(sk)
                        {
                            let name = String::from_utf8_lossy(s.as_bytes()).into_owned();
                            if !names.contains(&name) {
                                names.push(name);
                            }
                        }
                        key = k;
                    }
                    Ok(None) => break,
                    Err(()) => break,
                }
            }
        }
        names.sort();
        names.dedup();
        self.globals = names;
    }
}

impl Completer for LuaCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        // カーソル直前の識別子を抽出する（`.` を含む: `string.` 等のフィールド補完対応）。
        let before_cursor = &line[..pos];
        let word_start = before_cursor
            .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &before_cursor[word_start..];

        if prefix.is_empty() {
            return vec![];
        }

        let span = Span::new(word_start, pos);
        let kw_style = Style::new().fg(Color::Cyan).bold();
        let global_style = Style::new().fg(Color::LightBlue);

        self.globals
            .iter()
            .filter(|name| name.starts_with(prefix))
            .map(|name| {
                let is_kw = LUA_KEYWORDS.contains(&name.as_str());
                Suggestion {
                    value: name.clone(),
                    display_override: None,
                    description: if is_kw {
                        Some("keyword".to_string())
                    } else {
                        None
                    },
                    style: Some(if is_kw { kw_style } else { global_style }),
                    extra: None,
                    span,
                    append_whitespace: false,
                    match_indices: None,
                }
            })
            .collect()
    }
}

// ============================================================================
// 補完チェッカー（未完入力判定）
// ============================================================================

/// 構文エラーメッセージが「入力が未完」を示しているか判定する。
///
/// 本家 `lua.c` の `incomplete` 関数（`LUA_ERRSYNTAX` + `near '<eof>'` の確認）に相当する。
/// rua では `LuaError::Syntax(msg)` の `msg` が `<eof>` を含むかで判定する。
fn is_incomplete(e: &LuaError) -> bool {
    match e {
        LuaError::Syntax(msg) => msg.contains("<eof>"),
        _ => false,
    }
}

// ============================================================================
// 式評価ヘルパ
// ============================================================================

/// 入力を `return <input>` としてコンパイル/実行し、戻り値を print する試みを行う。
///
/// 本家 `lua.c` の `lua_loadline` 相当: まず `return <input>` でコンパイルし、
/// 失敗したら `<input>` そのままで再コンパイル/実行する。
///
/// 返り値:
///   - `Ok(true)`  : 実行成功（戻り値を print した）。
///   - `Ok(false)` : 実行時エラー（エラーを表示した）。
///   - `Err(msg)`  : 構文エラー（継続判定に使う）。
fn eval_line(state: &mut LuaState, src: &str) -> Result<bool, LuaError> {
    // まず `return <src>` を試みる（式評価モード）。
    let return_src = format!("return {src}");
    let expr_proto = compile(
        &mut state.global.heap,
        return_src.as_bytes(),
        "=stdin",
    );

    let proto = match expr_proto {
        Ok(p) => p,
        Err(_) => {
            // 式として解釈できない: 文としてコンパイルする。
            let stmt_proto = compile(&mut state.global.heap, src.as_bytes(), "=stdin")?;
            // 文として実行。
            let rc = Rc::new(stmt_proto);
            let exec_result = pcall(state, |s| run(s, rc, &[]));
            match exec_result {
                Ok(_) => return Ok(true),
                Err(e) => {
                    let msg = render_error(state, &e);
                    eprintln!("{}", Style::new().fg(Color::Red).paint(format!("rua: {msg}")));
                    return Ok(false);
                }
            }
        }
    };

    // 式として実行し、戻り値を print する。
    let rc = Rc::new(proto);
    let exec_result = pcall(state, |s| run(s, rc, &[]));
    match exec_result {
        Ok(results) if !results.is_empty() => {
            // グローバル `print` で戻り値を表示する（本家同様）。
            print_results(state, results);
            Ok(true)
        }
        Ok(_) => Ok(true),
        Err(e) => {
            let msg = render_error(state, &e);
            eprintln!("{}", Style::new().fg(Color::Red).paint(format!("rua: {msg}")));
            Ok(false)
        }
    }
}

/// `print(...)` 相当: グローバル `print` 関数を呼んで結果を表示する。
///
/// `print` が無い場合は `tostring` 相当の素朴な表示にフォールバックする。
fn print_results(state: &mut LuaState, values: Vec<Value>) {
    // グローバル `print` を取得する。
    // intern_str（可変借用）を先に呼んでから get_table（不変借用）を呼ぶ。
    let print_key = state.global.heap.intern_str(b"print");
    let print_fn = if let GcHandle::Table(gk) = state.global.globals {
        if let Some(t) = state.global.heap.get_table(gk) {
            let v = t.get(&Value::GcRef(print_key));
            if !matches!(v, Value::Nil) {
                Some(v)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some(print_func) = print_fn {
        // pcall で保護実行。
        let _ = pcall(state, |s| vm_call(s, print_func, &values));
    } else {
        // フォールバック: 素朴な表示。
        let parts: Vec<String> = values.iter().map(|v| value_to_display(state, v)).collect();
        println!("{}", parts.join("\t"));
    }
}

/// 値を簡易表示用文字列に変換する（print のフォールバック用）。
fn value_to_display(state: &LuaState, v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => format_number(*n),
        Value::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(*k)
                .map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned())
                .unwrap_or_else(|| "?".to_string())
        }
        Value::GcRef(GcHandle::Table(_)) => "table".to_string(),
        Value::GcRef(GcHandle::Closure(_)) => "function".to_string(),
        Value::GcRef(GcHandle::Userdata(_)) => "userdata".to_string(),
        Value::LightUserData(_) => "userdata".to_string(),
    }
}

/// 数値を Lua 的に表示する（整数値は整数形式、その他は通常の float 形式）。
fn format_number(n: f64) -> String {
    if n.is_finite() && n.floor() == n && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

// ============================================================================
// 起動バナー
// ============================================================================

fn print_banner() {
    let version_str = format!("rua {RUA_VERSION} ({LUA_VERSION})");
    let banner = format!(
        "{}\n{}",
        Style::new().fg(Color::Green).bold().paint(&version_str),
        Style::new().fg(Color::DarkGray).paint("Type 'exit' to quit. Type Ctrl-D or Ctrl-C to abort."),
    );
    println!("{banner}");
}

// ============================================================================
// reedline エンジン構築
// ============================================================================

/// reedline エンジンを構築する。Tab 補完・ハイライトを登録する。
fn build_editor(completer: Box<LuaCompleter>) -> Reedline {
    // ファイル永続履歴の設定。失敗しても REPL は継続する（インメモリ履歴にフォールバック）。
    let history: Option<Box<dyn reedline::History>> = {
        let history_path = dirs_history_path();
        if let Some(path) = history_path {
            FileBackedHistory::with_file(1000, path)
                .ok()
                .map(|h| -> Box<dyn reedline::History> { Box::new(h) })
        } else {
            None
        }
    };

    // Tab で補完メニューを開くキーバインドを設定する。
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let edit_mode = Box::new(Emacs::new(keybindings));

    // 補完メニューの設定。
    let columnar_menu = ColumnarMenu::default()
        .with_name("completion_menu")
        .with_columns(4)
        .with_column_padding(2);
    let completion_menu = Box::new(columnar_menu);

    let mut editor = Reedline::create()
        .with_highlighter(Box::new(LuaHighlighter))
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode);

    if let Some(h) = history {
        editor = editor.with_history(h);
    }

    editor
}

/// 履歴ファイルのパスを返す。
/// `~/.local/share/rua/history.txt` を使う（XDG に準拠）。
fn dirs_history_path() -> Option<std::path::PathBuf> {
    let mut path = dirs_data_dir()?;
    path.push("rua");
    // ディレクトリがなければ作成する。
    let _ = std::fs::create_dir_all(&path);
    path.push("history.txt");
    Some(path)
}

/// XDG_DATA_HOME / `~/.local/share` を返す簡易実装。
/// `dirs` クレートへの依存を避けるため手動実装する。
fn dirs_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Some(std::path::PathBuf::from(xdg));
    }
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".local").join("share"))
}

// ============================================================================
// 非 tty（パイプ入力）フォールバック
// ============================================================================

/// 標準入力が tty でない場合のパイプモード実行。
///
/// 行ごとに読み込み、`eval_line` で評価する。複数行継続は行バッファで対応する。
fn run_pipe_mode(state: &mut LuaState) -> ExitCode {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut buf = String::new();
    let mut exit_code = ExitCode::SUCCESS;

    loop {
        let line = match lines.next() {
            Some(Ok(l)) => l,
            Some(Err(e)) => {
                eprintln!("rua: stdin read error: {e}");
                return ExitCode::from(1);
            }
            None => break, // EOF
        };

        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&line);

        if buf.trim().is_empty() {
            continue;
        }

        match eval_line(state, &buf) {
            Ok(true) => {
                // 実行成功。
                buf.clear();
            }
            Ok(false) => {
                // 実行時エラー（eval_line 内でエラー表示済み）。
                exit_code = ExitCode::from(1);
                buf.clear();
            }
            Err(e) if is_incomplete(&e) => {
                // 未完: 次の行を待つ。
            }
            Err(e) => {
                let msg = render_error(state, &e);
                eprintln!("rua: {msg}");
                exit_code = ExitCode::from(1);
                buf.clear();
            }
        }
    }

    // バッファに残りがある場合は実行を試みる。
    if !buf.trim().is_empty() && let Err(e) = eval_line(state, &buf) {
        let msg = render_error(state, &e);
        eprintln!("rua: {msg}");
        exit_code = ExitCode::from(1);
    }

    exit_code
}

// ============================================================================
// メイン REPL ループ
// ============================================================================

/// REPL のエントリポイント（`main.rs` から呼ばれる）。
///
/// tty 接続時は reedline でリッチな対話インタプリタを提供する。
/// パイプ（非 tty）入力時は行読み込みモードにフォールバックする。
pub fn main() -> ExitCode {
    let mut state = LuaState::new();
    stdlib::open_libs(&mut state);

    // 非 tty（パイプ）チェック。
    if !is_tty() {
        return run_pipe_mode(&mut state);
    }

    print_banner();

    // 補完器を構築してグローバルをロードする。
    let mut completer = Box::new(LuaCompleter::new());
    completer.refresh_globals(&state);

    let mut editor = build_editor(completer);
    let mut prompt = LuaPrompt::new();
    let mut input_buf = String::new(); // 複数行バッファ

    loop {
        // 補完器のグローバルスナップショットは評価後に更新したいが、
        // reedline は `with_completer` でムーブしてしまうため、
        // ループ毎に再構築せず起動時のスナップショットを使う設計とする。
        // グローバルが増えた場合は次回 REPL 起動時に反映される。
        // （改善余地あり: Mutex/RefCell を使ってスナップショットを共有）

        let sig = match editor.read_line(&prompt) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("rua: readline error: {e}");
                return ExitCode::from(1);
            }
        };

        match sig {
            Signal::Success(line) => {
                // 空行は継続バッファが空のときはスキップ。
                if line.trim().is_empty() && input_buf.is_empty() {
                    continue;
                }

                // バッファに追加する。
                if !input_buf.is_empty() {
                    input_buf.push('\n');
                }
                input_buf.push_str(&line);

                // `exit` コマンドのチェック（本家は `os.exit()` を使うが利便性のため対応）。
                if input_buf.trim() == "exit" {
                    break;
                }

                // eval_line で評価を試みる。
                match eval_line(&mut state, &input_buf) {
                    Ok(_) => {
                        // 成功: バッファをクリアして通常プロンプトに戻る。
                        input_buf.clear();
                        prompt.is_continuation = false;
                    }
                    Err(e) if is_incomplete(&e) => {
                        // 未完: 継続プロンプトに切り替えて次行を待つ。
                        prompt.is_continuation = true;
                    }
                    Err(e) => {
                        // 構文エラー（不完全ではない）: エラーを表示してクリア。
                        let msg = render_error(&state, &e);
                        eprintln!("{}", Style::new().fg(Color::Red).paint(format!("rua: {msg}")));
                        input_buf.clear();
                        prompt.is_continuation = false;
                    }
                }
            }
            Signal::CtrlC => {
                // 現在の入力を破棄して継続する（本家同様）。
                if !input_buf.is_empty() {
                    input_buf.clear();
                    prompt.is_continuation = false;
                    eprintln!("(input interrupted)");
                }
                // 空のときは何もしない（通常プロンプトに戻る）。
            }
            Signal::CtrlD => {
                // EOF: ループを抜けて終了する。
                println!();
                break;
            }
        }
    }

    ExitCode::SUCCESS
}

// ============================================================================
// tty チェック
// ============================================================================

/// 標準入力が端末（tty）かどうかを返す。
///
/// UNIX の `isatty(0)` 相当。libc に依存しないため `/proc/self/fd/0` のシンボリックリンク先で
/// 簡易判定するか、`TERM` 環境変数を参考にする。
/// より確実な実装は `libc::isatty` を使うが、クレート依存を増やさないため
/// 標準入力の `is_terminal()` メソッドを `std::io::IsTerminal` で利用する。
fn is_tty() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal()
}
