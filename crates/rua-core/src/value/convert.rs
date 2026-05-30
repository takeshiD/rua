//! 数値・文字列の変換規則（本家 `lobject.c` の `luaO_str2d` / `lua_number2str` 相当）。担当: **lua-vm**。
//!
//! Lua 5.1 はすべての数値が `double`。算術での文字列→数値の暗黙変換、`tostring`/`print` での
//! 数値→文字列変換は本家の `LUAI_NUMFMT`（`"%.14g"`）と `strtod` 互換規則に従う。

/// 数値を Lua の既定書式（本家 `"%.14g"`）で文字列化する。
///
/// 例: `1.0 → "1"`, `0.5 → "0.5"`, `1e20 → "1e+20"`, `1/3 → "0.33333333333333"`。
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        // 本家は環境依存で "nan"/"-nan"。負号付き nan も "nan" に寄せる。
        return "nan".to_string();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    format_g(n, 14)
}

/// C の `printf("%.*g", prec, n)` 相当（有限値専用）。
fn format_g(n: f64, prec: usize) -> String {
    let prec = prec.max(1);
    if n == 0.0 {
        // `-0.0` も "0"（本家 %g は "-0" を出すが、Lua の慣用表示に合わせ符号を落とす）。
        return "0".to_string();
    }

    // 指数を求め、%e と %f のどちらを使うか決める（C の %g 規則）。
    let exp = n.abs().log10().floor() as i32;
    if exp < -4 || exp >= prec as i32 {
        // %e 形式: 仮数の有効桁 = prec-1。
        let mut s = format!("{:.*e}", prec - 1, n);
        s = trim_exp_mantissa(&s);
        normalize_exponent(&s)
    } else {
        // %f 形式: 小数点以下桁数 = prec-1-exp。
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        let s = format!("{:.*}", decimals, n);
        trim_fraction(&s)
    }
}

/// %f 出力から末尾の不要な 0 と小数点を落とす（"3.1400"→"3.14", "3.0"→"3"）。
fn trim_fraction(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let trimmed = s.trim_end_matches('0');
    let trimmed = trimmed.trim_end_matches('.');
    trimmed.to_string()
}

/// Rust の `{:e}` 出力（"1.2300e2" 形式）の仮数末尾 0 を落とす。
fn trim_exp_mantissa(s: &str) -> String {
    if let Some(epos) = s.find(['e', 'E']) {
        let (mant, exp) = s.split_at(epos);
        let mant = if mant.contains('.') {
            let t = mant.trim_end_matches('0');
            t.trim_end_matches('.')
        } else {
            mant
        };
        format!("{mant}{exp}")
    } else {
        s.to_string()
    }
}

/// 指数部を C 風（`e+NN` / `e-NN`, 最低 2 桁）へ整形する。
fn normalize_exponent(s: &str) -> String {
    if let Some(epos) = s.find(['e', 'E']) {
        let (mant, exp) = s.split_at(epos);
        let exp = &exp[1..]; // 'e' を除く
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp.strip_prefix('+').unwrap_or(exp)),
        };
        let digits = if digits.len() < 2 {
            format!("{digits:0>2}")
        } else {
            digits.to_string()
        };
        format!("{mant}e{sign}{digits}")
    } else {
        s.to_string()
    }
}

/// 文字列を数値へ変換する（本家 `luaO_str2d`）。前後の空白を無視し、全体が数値なら `Some`。
///
/// 10 進浮動小数点と `0x` 始まりの 16 進整数を受け付ける。`inf`/`nan` の語は受け付けない。
pub fn str_to_number(bytes: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(bytes).ok()?;
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return None;
    }

    // 16 進整数（符号付き可）。
    let (sign, body) = match t.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // 桁あふれは本家同様 wrap させず f64 へ累積する。
        let mut acc = 0.0f64;
        for b in hex.bytes() {
            let d = (b as char).to_digit(16).unwrap() as f64;
            acc = acc * 16.0 + d;
        }
        return Some(sign * acc);
    }

    // 10 進。"inf"/"nan" の語は弾く（数字を含むことを要求）。
    let lower = t.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return None;
    }
    t.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 は数値整形の検証用リテラルであり π ではない
    fn number_formatting_matches_lua() {
        assert_eq!(number_to_string(1.0), "1");
        assert_eq!(number_to_string(-2.0), "-2");
        assert_eq!(number_to_string(3.0), "3");
        assert_eq!(number_to_string(0.5), "0.5");
        assert_eq!(number_to_string(3.14), "3.14");
        assert_eq!(number_to_string(100.0), "100");
        assert_eq!(number_to_string(1e20), "1e+20");
        assert_eq!(number_to_string(1e100), "1e+100");
        assert_eq!(number_to_string(1.0 / 3.0), "0.33333333333333");
        assert_eq!(number_to_string(0.0), "0");
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 は文字列→数値変換の検証用リテラルであり π ではない
    fn string_to_number_rules() {
        assert_eq!(str_to_number(b"  42 "), Some(42.0));
        assert_eq!(str_to_number(b"3.14"), Some(3.14));
        assert_eq!(str_to_number(b"-1.5e3"), Some(-1500.0));
        assert_eq!(str_to_number(b"0xff"), Some(255.0));
        assert_eq!(str_to_number(b"0x10"), Some(16.0));
        assert_eq!(str_to_number(b".5"), Some(0.5));
        assert_eq!(str_to_number(b"hello"), None);
        assert_eq!(str_to_number(b""), None);
        assert_eq!(str_to_number(b"  "), None);
        assert_eq!(str_to_number(b"inf"), None);
        assert_eq!(str_to_number(b"nan"), None);
    }
}
