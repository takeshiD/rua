//! string ライブラリ（本家 `lstrlib.c` 相当）。担当: **lua-stdlib**。
//!
//! `len`/`sub`/`rep`/`upper`/`lower`/`byte`/`char`/`format`/`reverse` と、Lua パターンを使う
//! `find`/`match`/`gmatch`/`gsub`。パターン照合は [`super::pattern`] が担う。

use crate::error::LuaResult;
use crate::gc::{GcHandle, TableKey};
use crate::state::LuaState;
use crate::value::convert::number_to_string;
use crate::value::Value;

use super::aux;
use super::pattern::{self, Cap, MatchState};

pub fn open(state: &mut LuaState) {
    let s = state.new_table();
    let sk = match s {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };
    aux::register(state, sk, "len", l_len);
    aux::register(state, sk, "sub", l_sub);
    aux::register(state, sk, "rep", l_rep);
    aux::register(state, sk, "upper", l_upper);
    aux::register(state, sk, "lower", l_lower);
    aux::register(state, sk, "reverse", l_reverse);
    aux::register(state, sk, "byte", l_byte);
    aux::register(state, sk, "char", l_char);
    aux::register(state, sk, "format", l_format);
    aux::register(state, sk, "find", l_find);
    aux::register(state, sk, "match", l_match);
    aux::register(state, sk, "gmatch", l_gmatch);
    aux::register(state, sk, "gsub", l_gsub);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "string", s);
    }

    // 文字列型の共有メタテーブル `{ __index = string }` を登録する（本家 `lstrlib.c` の
    // `createmetatable`）。これにより `("x"):upper()` / `s:match(p)` などのメソッド構文が
    // VM の `metatable_of`（string を参照）経由で `string` テーブルへ解決される。
    let mt = state.new_table();
    if let Value::GcRef(GcHandle::Table(mtk)) = mt {
        aux::set_field(state, mtk, "__index", s);
        state.global.string_metatable = Some(GcHandle::Table(mtk));
    }
}

// ============================================================================
// 基本関数
// ============================================================================

fn l_len(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "len")?;
    aux::ret(state, vec![Value::Number(s.len() as f64)])
}

fn l_sub(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "sub")?;
    let len = s.len();
    let mut i = pattern::posrelat(aux::check_int(state, &args, 1, "sub")?, len);
    let mut j = pattern::posrelat(aux::opt_int(state, &args, 2, "sub", -1)?, len);
    if i < 1 {
        i = 1;
    }
    if j > len as i64 {
        j = len as i64;
    }
    let out = if i <= j {
        s[(i - 1) as usize..j as usize].to_vec()
    } else {
        Vec::new()
    };
    let v = state.new_string(&out);
    aux::ret(state, vec![v])
}

fn l_rep(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "rep")?;
    let n = aux::check_int(state, &args, 1, "rep")?;
    let mut out = Vec::new();
    let mut k = 0;
    while k < n {
        out.extend_from_slice(&s);
        k += 1;
    }
    let v = state.new_string(&out);
    aux::ret(state, vec![v])
}

fn l_upper(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut s = aux::check_str_bytes(state, &args, 0, "upper")?;
    s.make_ascii_uppercase();
    let v = state.new_string(&s);
    aux::ret(state, vec![v])
}

fn l_lower(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut s = aux::check_str_bytes(state, &args, 0, "lower")?;
    s.make_ascii_lowercase();
    let v = state.new_string(&s);
    aux::ret(state, vec![v])
}

fn l_reverse(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut s = aux::check_str_bytes(state, &args, 0, "reverse")?;
    s.reverse();
    let v = state.new_string(&s);
    aux::ret(state, vec![v])
}

fn l_byte(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "byte")?;
    let len = s.len();
    let mut i = pattern::posrelat(aux::opt_int(state, &args, 1, "byte", 1)?, len);
    let mut j = pattern::posrelat(aux::opt_int(state, &args, 2, "byte", i)?, len);
    if i < 1 {
        i = 1;
    }
    if j > len as i64 {
        j = len as i64;
    }
    let mut out = Vec::new();
    let mut k = i;
    while k <= j {
        out.push(Value::Number(s[(k - 1) as usize] as f64));
        k += 1;
    }
    aux::ret(state, out)
}

fn l_char(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut out = Vec::with_capacity(args.len());
    for i in 0..args.len() {
        let c = aux::check_int(state, &args, i, "char")?;
        if !(0..=255).contains(&c) {
            return Err(aux::arg_error(state, i + 1, "char", "value out of range"));
        }
        out.push(c as u8);
    }
    let v = state.new_string(&out);
    aux::ret(state, vec![v])
}

// ============================================================================
// format
// ============================================================================

fn l_format(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let fmt = aux::check_str_bytes(state, &args, 0, "format")?;
    let mut out: Vec<u8> = Vec::new();
    let mut argi = 1usize; // 次に消費する引数（0 はフォーマット文字列）
    let mut i = 0usize;
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1; // '%' を消費
        if i >= fmt.len() {
            return Err(aux::rt_error(state, "invalid option '%' to 'format'"));
        }
        if fmt[i] == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        // フラグ・幅・精度・変換指定子を読む。
        let spec_start = i;
        while i < fmt.len() && matches!(fmt[i], b'-' | b'+' | b' ' | b'#' | b'0') {
            i += 1;
        }
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            i += 1;
        }
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i >= fmt.len() {
            return Err(aux::rt_error(state, "invalid conversion to 'format'"));
        }
        let conv = fmt[i];
        let spec = std::str::from_utf8(&fmt[spec_start..i]).unwrap_or("");
        i += 1;

        let parsed = parse_spec(spec);
        match conv {
            b'd' | b'i' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_int(&parsed, n, 10, false, true).as_bytes());
            }
            b'u' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_int(&parsed, n, 10, false, false).as_bytes());
            }
            b'o' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_int(&parsed, n, 8, false, false).as_bytes());
            }
            b'x' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_int(&parsed, n, 16, false, false).as_bytes());
            }
            b'X' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_int(&parsed, n, 16, true, false).as_bytes());
            }
            b'c' => {
                let n = aux::check_int(state, &args, argi, "format")?;
                argi += 1;
                out.push(n as u8);
            }
            b'e' | b'E' | b'f' | b'g' | b'G' => {
                let x = aux::check_number(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(format_float(&parsed, x, conv).as_bytes());
            }
            b's' => {
                let v = aux::opt_value(&args, argi);
                argi += 1;
                let bytes = aux::tostring_value(state, v)?;
                out.extend_from_slice(&format_str(&parsed, &bytes));
            }
            b'q' => {
                let bytes = aux::check_str_bytes(state, &args, argi, "format")?;
                argi += 1;
                out.extend_from_slice(&format_q(&bytes));
            }
            other => {
                return Err(aux::rt_error(
                    state,
                    format!("invalid option '%{}' to 'format'", other as char),
                ));
            }
        }
    }
    let v = state.new_string(&out);
    aux::ret(state, vec![v])
}

/// 解析済みフォーマット指定子。
struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    hash: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
}

fn parse_spec(spec: &str) -> Spec {
    let b = spec.as_bytes();
    let mut i = 0;
    let mut s = Spec {
        minus: false,
        plus: false,
        space: false,
        hash: false,
        zero: false,
        width: 0,
        prec: None,
    };
    while i < b.len() {
        match b[i] {
            b'-' => s.minus = true,
            b'+' => s.plus = true,
            b' ' => s.space = true,
            b'#' => s.hash = true,
            b'0' => s.zero = true,
            _ => break,
        }
        i += 1;
    }
    let mut w = 0usize;
    let mut has_w = false;
    while i < b.len() && b[i].is_ascii_digit() {
        w = w * 10 + (b[i] - b'0') as usize;
        has_w = true;
        i += 1;
    }
    if has_w {
        s.width = w;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let mut p = 0usize;
        while i < b.len() && b[i].is_ascii_digit() {
            p = p * 10 + (b[i] - b'0') as usize;
            i += 1;
        }
        s.prec = Some(p);
    }
    s
}

/// 幅・寄せ（左/右, ゼロ埋め）を適用する。`prefix` は符号や `0x` など、ゼロ埋めより前に置く部分。
fn pad(s: &Spec, prefix: &str, body: &str, allow_zero: bool) -> String {
    let total = prefix.len() + body.len();
    if total >= s.width {
        return format!("{prefix}{body}");
    }
    let padn = s.width - total;
    if s.minus {
        format!("{prefix}{body}{}", " ".repeat(padn))
    } else if s.zero && allow_zero {
        format!("{prefix}{}{body}", "0".repeat(padn))
    } else {
        format!("{}{prefix}{body}", " ".repeat(padn))
    }
}

fn format_int(s: &Spec, n: i64, base: u32, upper: bool, signed: bool) -> String {
    let (neg, mag) = if signed && n < 0 {
        (true, (n as i128).unsigned_abs())
    } else {
        (false, n as u64 as u128)
    };
    let mut digits = match base {
        8 => format!("{mag:o}"),
        16 => {
            if upper {
                format!("{mag:X}")
            } else {
                format!("{mag:x}")
            }
        }
        _ => format!("{mag}"),
    };
    // 精度: 最小桁数（ゼロ埋め）。精度指定時はフラグ '0' を無効化（C 準拠）。
    let zero_pad_allowed = s.prec.is_none();
    if let Some(p) = s.prec {
        if digits.len() < p {
            digits = format!("{}{digits}", "0".repeat(p - digits.len()));
        }
        if p == 0 && mag == 0 {
            digits.clear();
        }
    }
    let mut prefix = String::new();
    if neg {
        prefix.push('-');
    } else if signed && s.plus {
        prefix.push('+');
    } else if signed && s.space {
        prefix.push(' ');
    }
    if s.hash && mag != 0 {
        if base == 16 {
            prefix.push_str(if upper { "0X" } else { "0x" });
        } else if base == 8 && !digits.starts_with('0') {
            prefix.push('0');
        }
    }
    pad(s, &prefix, &digits, zero_pad_allowed)
}

fn format_float(s: &Spec, x: f64, conv: u8) -> String {
    let prec = s.prec.unwrap_or(6);
    let upper = conv.is_ascii_uppercase();
    let mut prefix = String::new();
    let mag = x.abs();
    if x.is_sign_negative() && !x.is_nan() {
        prefix.push('-');
    } else if s.plus {
        prefix.push('+');
    } else if s.space {
        prefix.push(' ');
    }

    let body = if x.is_nan() {
        prefix.clear();
        if x.is_sign_negative() { format!("-{}", if upper {"NAN"} else {"nan"}) }
        else { (if upper {"NAN"} else {"nan"}).to_string() }
    } else if x.is_infinite() {
        (if upper { "INF" } else { "inf" }).to_string()
    } else {
        match conv.to_ascii_lowercase() {
            b'f' => format!("{mag:.prec$}"),
            b'e' => fmt_exp(mag, prec, upper),
            b'g' => fmt_g(mag, prec.max(1), upper, s.hash),
            _ => format!("{mag}"),
        }
    };
    let allow_zero = !x.is_nan() && !x.is_infinite();
    pad(s, &prefix, &body, allow_zero)
}

/// `%e`: 指数表記（指数部 2 桁以上）。
fn fmt_exp(mag: f64, prec: usize, upper: bool) -> String {
    let s = format!("{mag:.prec$e}");
    let e = if upper { 'E' } else { 'e' };
    normalize_exp(&s, e)
}

/// Rust の `{:e}` 出力（"1.5e2"）を C 風（"1.500000e+02"）へ整える。
fn normalize_exp(s: &str, e: char) -> String {
    if let Some(epos) = s.find(['e', 'E']) {
        let (mant, exp) = s.split_at(epos);
        let exp = &exp[1..];
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp.strip_prefix('+').unwrap_or(exp)),
        };
        let digits = if digits.len() < 2 {
            format!("{digits:0>2}")
        } else {
            digits.to_string()
        };
        format!("{mant}{e}{sign}{digits}")
    } else {
        s.to_string()
    }
}

/// `%g`: 仮数・指数を C の `%g` 規則で選択し、末尾 0 を落とす（`#` 指定時は残す）。
fn fmt_g(mag: f64, prec: usize, upper: bool, hash: bool) -> String {
    if mag == 0.0 {
        return "0".to_string();
    }
    let exp = mag.abs().log10().floor() as i32;
    let (mut body, used_exp) = if exp < -4 || exp >= prec as i32 {
        let s = format!("{mag:.*e}", prec - 1);
        (normalize_exp(&s, if upper { 'E' } else { 'e' }), true)
    } else {
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        (format!("{mag:.decimals$}"), false)
    };
    if !hash {
        body = trim_g(&body, used_exp);
    }
    body
}

/// `%g` の末尾不要 0 と小数点を除去する。
fn trim_g(s: &str, used_exp: bool) -> String {
    if used_exp {
        // 指数表記: 仮数部のみトリム。
        if let Some(epos) = s.find(['e', 'E']) {
            let (mant, exp) = s.split_at(epos);
            let mant = if mant.contains('.') {
                mant.trim_end_matches('0').trim_end_matches('.')
            } else {
                mant
            };
            return format!("{mant}{exp}");
        }
        s.to_string()
    } else if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

fn format_str(s: &Spec, bytes: &[u8]) -> Vec<u8> {
    let mut body: Vec<u8> = match s.prec {
        Some(p) if p < bytes.len() => bytes[..p].to_vec(),
        _ => bytes.to_vec(),
    };
    if body.len() < s.width {
        let padn = s.width - body.len();
        if s.minus {
            body.extend(std::iter::repeat_n(b' ', padn));
        } else {
            let mut padded = vec![b' '; padn];
            padded.extend_from_slice(&body);
            body = padded;
        }
    }
    body
}

/// `%q`: Lua ソースとして読み戻せる形式へエスケープする。
fn format_q(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'"');
    for &c in bytes {
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0 => out.extend_from_slice(b"\\0"),
            _ => out.push(c),
        }
    }
    out.push(b'"');
    out
}

// ============================================================================
// パターン: find / match / gmatch / gsub
// ============================================================================

/// `Cap` 列を `Value` 列へ（`src` 断片はインターン）。
fn caps_to_values(state: &mut LuaState, src: &[u8], caps: &[Cap]) -> Vec<Value> {
    caps.iter()
        .map(|c| match c {
            Cap::Str(start, len) => state.new_string(&src[*start..*start + *len]),
            Cap::Pos(init) => Value::Number((*init as f64) + 1.0),
        })
        .collect()
}

fn find_or_match(state: &mut LuaState, find: bool) -> LuaResult<i32> {
    let fname = if find { "find" } else { "match" };
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, fname)?;
    let p = aux::check_str_bytes(state, &args, 1, fname)?;
    let len = s.len();
    let mut init = pattern::posrelat(aux::opt_int(state, &args, 2, fname, 1)?, len) - 1;
    if init < 0 {
        init = 0;
    } else if init as usize > len {
        // 本家: 初期位置が末尾超なら len へクランプ（空パターンが末尾でマッチしうる）。
        init = len as i64;
    }
    let init = init as usize;

    // find のプレーン検索（明示要求 or 特殊文字なし）。
    let plain = aux::opt_value(&args, 3).is_truthy();
    if find && (plain || !pattern::has_specials(&p)) {
        if let Some(pos) = mem_find(&s[init..], &p) {
            let start = init + pos;
            return aux::ret(
                state,
                vec![
                    Value::Number((start + 1) as f64),
                    Value::Number((start + p.len()) as f64),
                ],
            );
        }
        return aux::ret(state, vec![Value::Nil]);
    }

    // パターン照合。
    let anchor = p.first() == Some(&b'^');
    let pat_start = if anchor { 1 } else { 0 };
    let mut s1 = init;
    loop {
        let mut ms = MatchState::new(&s, &p);
        ms.reset();
        if let Some(e) = ms.do_match(s1, pat_start).map_err(|e| aux::rt_error(state, e))? {
            if find {
                let caps = ms.captures(s1, e, false).map_err(|er| aux::rt_error(state, er))?;
                let mut out = vec![Value::Number((s1 + 1) as f64), Value::Number(e as f64)];
                out.extend(caps_to_values(state, &s, &caps));
                return aux::ret(state, out);
            } else {
                let caps = ms.captures(s1, e, true).map_err(|er| aux::rt_error(state, er))?;
                let out = caps_to_values(state, &s, &caps);
                return aux::ret(state, out);
            }
        }
        if anchor || s1 >= s.len() {
            break;
        }
        s1 += 1;
    }
    aux::ret(state, vec![Value::Nil])
}

fn l_find(state: &mut LuaState) -> LuaResult<i32> {
    find_or_match(state, true)
}

fn l_match(state: &mut LuaState) -> LuaResult<i32> {
    find_or_match(state, false)
}

/// `haystack` 内の `needle` の先頭位置（プレーン検索）。
fn mem_find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

// ---- gmatch（__call 付きテーブルで状態を保持） ------------------------------

fn l_gmatch(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let s = aux::check_str_bytes(state, &args, 0, "gmatch")?;
    let p = aux::check_str_bytes(state, &args, 1, "gmatch")?;
    let sval = state.new_string(&s);
    let pval = state.new_string(&p);
    // 状態テーブル {s, p, pos=0} を作り、__call = gmatch_aux のメタテーブルを付ける。
    let tbl = state.new_table();
    let tk = match tbl {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => unreachable!(),
    };
    aux::set_field(state, tk, "s", sval);
    aux::set_field(state, tk, "p", pval);
    aux::set_field(state, tk, "pos", Value::Number(0.0));

    let mt = state.new_table();
    let mtk = match mt {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => unreachable!(),
    };
    aux::register(state, mtk, "__call", gmatch_aux);
    if let Some(t) = state.global.heap.get_table_mut(tk) {
        t.set_metatable(Some(GcHandle::Table(mtk)));
    }
    aux::ret(state, vec![tbl])
}

/// gmatch のイテレータ本体。`__call` 経由で `self`（状態テーブル）が第1引数に来る。
fn gmatch_aux(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let selfk = aux::check_table(state, &args, 0, "gmatch")?;
    let s = field_bytes(state, selfk, "s");
    let p = field_bytes(state, selfk, "p");
    let pos = field_num(state, selfk, "pos") as usize;

    let mut src = pos;
    loop {
        if src > s.len() {
            return aux::ret0(state);
        }
        let mut ms = MatchState::new(&s, &p);
        ms.reset();
        match ms.do_match(src, 0).map_err(|e| aux::rt_error(state, e))? {
            Some(e) => {
                let newstart = if e == src { e + 1 } else { e };
                aux::set_field(state, selfk, "pos", Value::Number(newstart as f64));
                let caps = ms.captures(src, e, true).map_err(|er| aux::rt_error(state, er))?;
                let out = caps_to_values(state, &s, &caps);
                return aux::ret(state, out);
            }
            None => src += 1,
        }
    }
}

fn field_bytes(state: &mut LuaState, tk: TableKey, name: &str) -> Vec<u8> {
    let kv = state.new_string(name.as_bytes());
    match state.global.heap.get_table(tk).map(|t| t.get(&kv)) {
        Some(Value::GcRef(GcHandle::Str(sk))) => {
            state.global.heap.get_str(sk).map(|s| s.as_bytes().to_vec()).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn field_num(state: &mut LuaState, tk: TableKey, name: &str) -> f64 {
    let key = state.new_string(name.as_bytes());
    match state.global.heap.get_table(tk).map(|t| t.get(&key)) {
        Some(Value::Number(n)) => n,
        _ => 0.0,
    }
}

// ---- gsub -------------------------------------------------------------------

fn l_gsub(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let src = aux::check_str_bytes(state, &args, 0, "gsub")?;
    let pat = aux::check_str_bytes(state, &args, 1, "gsub")?;
    let repl = aux::opt_value(&args, 2);
    let max_s = if matches!(aux::opt_value(&args, 3), Value::Nil) {
        (src.len() + 1) as i64
    } else {
        aux::check_int(state, &args, 3, "gsub")?
    };
    // 置換種別の検査。
    match repl {
        Value::GcRef(GcHandle::Str(_)) | Value::Number(_) | Value::GcRef(GcHandle::Table(_))
        | Value::GcRef(GcHandle::Closure(_)) => {}
        _ => return Err(aux::arg_error(state, 3, "gsub", "string/function/table expected")),
    }

    let anchor = pat.first() == Some(&b'^');
    let pat_start = if anchor { 1 } else { 0 };
    let mut out: Vec<u8> = Vec::new();
    let mut n = 0i64;
    let mut s = 0usize;
    loop {
        if n >= max_s {
            break;
        }
        let mut ms = MatchState::new(&src, &pat);
        ms.reset();
        let e = ms.do_match(s, pat_start).map_err(|er| aux::rt_error(state, er))?;
        match e {
            Some(e) => {
                n += 1;
                add_value(state, &mut out, &ms, &src, s, e, repl)?;
                if e > s {
                    s = e; // 非空マッチ: 進める
                } else if s < src.len() {
                    out.push(src[s]);
                    s += 1;
                } else {
                    break;
                }
            }
            None => {
                if s < src.len() {
                    out.push(src[s]);
                    s += 1;
                } else {
                    break;
                }
            }
        }
        if anchor {
            break;
        }
    }
    // 残りを追加。
    out.extend_from_slice(&src[s.min(src.len())..]);
    let res = state.new_string(&out);
    aux::ret(state, vec![res, Value::Number(n as f64)])
}

/// gsub の 1 マッチ分の置換値を `out` へ追加する（本家 `add_value`）。
fn add_value(
    state: &mut LuaState,
    out: &mut Vec<u8>,
    ms: &MatchState,
    src: &[u8],
    s: usize,
    e: usize,
    repl: Value,
) -> LuaResult<()> {
    let caps = ms.captures(s, e, true).map_err(|er| aux::rt_error(state, er))?;
    match repl {
        Value::GcRef(GcHandle::Str(_)) | Value::Number(_) => {
            // 文字列置換: %0=全体, %1..=キャプチャ。
            let news = match repl {
                Value::GcRef(GcHandle::Str(k)) => state.global.heap.get_str(k).unwrap().as_bytes().to_vec(),
                Value::Number(num) => number_to_string(num).into_bytes(),
                _ => unreachable!(),
            };
            add_s(state, out, &news, src, s, e, &caps)?;
            Ok(())
        }
        Value::GcRef(GcHandle::Table(tk)) => {
            // 第1キャプチャでテーブルを引く。
            let key = caps_to_values(state, src, &caps[..1.min(caps.len())])
                .into_iter()
                .next()
                .unwrap_or(Value::Nil);
            let v = state.global.heap.get_table(tk).map(|t| t.get(&key)).unwrap_or(Value::Nil);
            append_repl_result(state, out, src, s, e, v)
        }
        Value::GcRef(GcHandle::Closure(_)) => {
            let cap_vals = caps_to_values(state, src, &caps);
            let res = crate::vm::call(state, repl, &cap_vals)?;
            let v = res.into_iter().next().unwrap_or(Value::Nil);
            append_repl_result(state, out, src, s, e, v)
        }
        _ => unreachable!(),
    }
}

/// 関数/テーブル置換の戻り値を `out` へ追加（false/nil は元テキストを残す）。
fn append_repl_result(
    state: &mut LuaState,
    out: &mut Vec<u8>,
    src: &[u8],
    s: usize,
    e: usize,
    v: Value,
) -> LuaResult<()> {
    match v {
        Value::Nil | Value::Boolean(false) => {
            out.extend_from_slice(&src[s..e]); // 元のテキストを保持
            Ok(())
        }
        Value::GcRef(GcHandle::Str(k)) => {
            let bytes = state.global.heap.get_str(k).unwrap().as_bytes().to_vec();
            out.extend_from_slice(&bytes);
            Ok(())
        }
        Value::Number(num) => {
            out.extend_from_slice(number_to_string(num).as_bytes());
            Ok(())
        }
        other => Err(aux::rt_error(
            state,
            format!("invalid replacement value (a {})", other.type_of().name()),
        )),
    }
}

/// 文字列置換 `news` 中の `%n` を展開して `out` へ（本家 `add_s`）。
fn add_s(
    state: &mut LuaState,
    out: &mut Vec<u8>,
    news: &[u8],
    src: &[u8],
    s: usize,
    e: usize,
    caps: &[Cap],
) -> LuaResult<()> {
    let mut i = 0;
    while i < news.len() {
        let c = news[i];
        if c != b'%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= news.len() {
            return Err(aux::rt_error(state, "invalid use of '%' in replacement string"));
        }
        let d = news[i];
        i += 1;
        if !d.is_ascii_digit() {
            out.push(d); // %% や %エスケープ
        } else if d == b'0' {
            out.extend_from_slice(&src[s..e]); // マッチ全体
        } else {
            let idx = (d - b'1') as usize;
            if idx >= caps.len() {
                return Err(aux::rt_error(state, "invalid capture index in replacement string"));
            }
            match &caps[idx] {
                Cap::Str(start, len) => out.extend_from_slice(&src[*start..*start + *len]),
                Cap::Pos(init) => out.extend_from_slice(number_to_string((*init as f64) + 1.0).as_bytes()),
            }
        }
    }
    Ok(())
}
