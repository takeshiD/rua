//! Lua 5.1 パターンマッチング（本家 `lstrlib.c` の `MatchState` / `match` 移植）。担当: **lua-stdlib**。
//!
//! これは**正規表現ではなく** Lua 独自のパターン仕様である。`find`/`match`/`gmatch`/`gsub`
//! が共有する照合エンジンを提供する。本家のポインタ演算をバイト列のインデックス演算へ
//! 1:1 で移植し、挙動（キャプチャ・`%b`・`%f`・量指定子・アンカー）を厳密に再現する。
//!
//! エラー（不正パターン等）は [`String`] で返し、呼び出し側がランタイムエラーへ昇格する
//! （照合エンジンは `LuaState` を借用しないため、借用衝突を避けられる）。

/// キャプチャ最大数（本家 `LUA_MAXCAPTURES`）。
const MAXCAPTURES: usize = 32;
/// 未確定キャプチャ（本家 `CAP_UNFINISHED`）。
const CAP_UNFINISHED: isize = -1;
/// 位置キャプチャ（本家 `CAP_POSITION`）。
const CAP_POSITION: isize = -2;
/// パターンのエスケープ文字 `%`（本家 `L_ESC`）。
const L_ESC: u8 = b'%';
/// パターン中の特殊文字（プレーン検索可否の判定に使う, 本家 `SPECIALS`）。
pub const SPECIALS: &[u8] = b"^$*+?.([%-";

/// キャプチャ 1 件（開始位置と長さ／特殊マーカ）。
#[derive(Clone, Copy)]
struct Capture {
    init: usize,
    len: isize,
}

/// 抽出済みキャプチャ（文字列断片 or 位置）。
pub enum Cap {
    /// 部分文字列（`src` 内の開始位置・長さ）。
    Str(usize, usize),
    /// 位置キャプチャ（0 始まり開始位置。Lua へは +1 して返す）。
    Pos(usize),
}

/// 照合状態（本家 `MatchState`）。`src`/`pat` をバイト列で借用する。
pub struct MatchState<'a> {
    src: &'a [u8],
    pat: &'a [u8],
    level: usize,
    capture: [Capture; MAXCAPTURES],
}

impl<'a> MatchState<'a> {
    pub fn new(src: &'a [u8], pat: &'a [u8]) -> Self {
        MatchState {
            src,
            pat,
            level: 0,
            capture: [Capture { init: 0, len: 0 }; MAXCAPTURES],
        }
    }

    pub fn src_len(&self) -> usize {
        self.src.len()
    }

    /// 照合をリセット（`gmatch`/`gsub`/`find` の各試行前に呼ぶ）。
    pub fn reset(&mut self) {
        self.level = 0;
    }

    // ---- パターン要素の終端 -------------------------------------------------

    /// `p` の指す 1 パターン要素の直後インデックスを返す（本家 `classend`）。
    fn class_end(&self, mut p: usize) -> Result<usize, String> {
        let c = self.pat[p];
        p += 1;
        if c == L_ESC {
            if p >= self.pat.len() {
                return Err("malformed pattern (ends with '%')".into());
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if p < self.pat.len() && self.pat[p] == b'^' {
                p += 1;
            }
            // do-while: 最低 1 文字を消費してから ']' を探す。
            loop {
                if p >= self.pat.len() {
                    return Err("malformed pattern (missing ']')".into());
                }
                let prev = self.pat[p];
                p += 1;
                if prev == L_ESC && p < self.pat.len() {
                    p += 1; // エスケープ（例: `%]`）をスキップ
                }
                if p < self.pat.len() && self.pat[p] == b']' {
                    break;
                }
            }
            return Ok(p + 1);
        }
        Ok(p)
    }

    // ---- 主照合ループ -------------------------------------------------------

    /// `s`（src インデックス）から `p`（pat インデックス）を照合する（本家 `match`）。
    ///
    /// 成功時 `Ok(Some(end))`（マッチ末尾の src インデックス）、失敗時 `Ok(None)`、
    /// 不正パターン時 `Err`。本家の `goto init` 末尾再帰はループで表現する。
    pub fn do_match(&mut self, mut s: usize, mut p: usize) -> Result<Option<usize>, String> {
        loop {
            if p >= self.pat.len() {
                return Ok(Some(s)); // パターン終端 = マッチ成功
            }
            match self.pat[p] {
                b'(' => {
                    return if self.pat.get(p + 1) == Some(&b')') {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                L_ESC => match self.pat.get(p + 1).copied() {
                    Some(b'b') => match self.match_balance(s, p + 2)? {
                        None => return Ok(None),
                        Some(ns) => {
                            s = ns;
                            p += 4;
                            continue;
                        }
                    },
                    Some(b'f') => {
                        p += 2;
                        if self.pat.get(p) != Some(&b'[') {
                            return Err("missing '[' after '%f' in pattern".into());
                        }
                        let ep = self.class_end(p)?;
                        let prev = if s == 0 { 0u8 } else { self.src[s - 1] };
                        let cur = if s < self.src.len() { self.src[s] } else { 0u8 };
                        if !single_match(prev, p, ep, self.pat)
                            && single_match(cur, p, ep, self.pat)
                        {
                            p = ep;
                            continue;
                        }
                        return Ok(None);
                    }
                    Some(d) if d.is_ascii_digit() => match self.match_capture(s, d)? {
                        None => return Ok(None),
                        Some(ns) => {
                            s = ns;
                            p += 2;
                            continue;
                        }
                    },
                    _ => { /* 既定処理へ */ }
                },
                b'$' if p + 1 == self.pat.len() => {
                    // 末尾アンカー: src 終端でのみ成功。
                    return Ok(if s == self.src.len() { Some(s) } else { None });
                }
                _ => { /* 既定処理へ */ }
            }

            // ---- 既定: 1 パターン要素 + 量指定子 ----
            let ep = self.class_end(p)?;
            let m = s < self.src.len() && single_match(self.src[s], p, ep, self.pat);
            match self.pat.get(ep).copied() {
                Some(b'?') => {
                    if m && let Some(res) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(res));
                    }
                    p = ep + 1;
                    continue;
                }
                Some(b'*') => return self.max_expand(s, p, ep),
                Some(b'+') => return if m { self.max_expand(s + 1, p, ep) } else { Ok(None) },
                Some(b'-') => return self.min_expand(s, p, ep),
                _ => {
                    if !m {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                    continue;
                }
            }
        }
    }

    /// `*`/`+` の最大展開（本家 `max_expand`）。
    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> Result<Option<usize>, String> {
        let mut i = 0usize;
        while s + i < self.src.len() && single_match(self.src[s + i], p, ep, self.pat) {
            i += 1;
        }
        loop {
            if let Some(res) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(res));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    /// `-` の最小展開（本家 `min_expand`）。
    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> Result<Option<usize>, String> {
        loop {
            if let Some(res) = self.do_match(s, ep + 1)? {
                return Ok(Some(res));
            }
            if s < self.src.len() && single_match(self.src[s], p, ep, self.pat) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    // ---- キャプチャ ---------------------------------------------------------

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> Result<Option<usize>, String> {
        let level = self.level;
        if level >= MAXCAPTURES {
            return Err("too many captures".into());
        }
        self.capture[level].len = what;
        self.capture[level].init = s;
        self.level = level + 1;
        let res = self.do_match(s, p)?;
        if res.is_none() {
            self.level -= 1; // キャプチャを巻き戻す
        }
        Ok(res)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> Result<Option<usize>, String> {
        let l = self.capture_to_close()?;
        self.capture[l].len = (s - self.capture[l].init) as isize;
        let res = self.do_match(s, p)?;
        if res.is_none() {
            self.capture[l].len = CAP_UNFINISHED;
        }
        Ok(res)
    }

    fn capture_to_close(&self) -> Result<usize, String> {
        let mut level = self.level as isize - 1;
        while level >= 0 {
            if self.capture[level as usize].len == CAP_UNFINISHED {
                return Ok(level as usize);
            }
            level -= 1;
        }
        Err("invalid pattern capture".into())
    }

    fn check_capture(&self, l: u8) -> Result<usize, String> {
        let idx = l as isize - b'1' as isize;
        if idx < 0 || idx as usize >= self.level || self.capture[idx as usize].len == CAP_UNFINISHED {
            return Err("invalid capture index".into());
        }
        Ok(idx as usize)
    }

    fn match_capture(&mut self, s: usize, l: u8) -> Result<Option<usize>, String> {
        let l = self.check_capture(l)?;
        let len = self.capture[l].len as usize;
        let init = self.capture[l].init;
        if self.src.len() - s >= len && self.src[s..s + len] == self.src[init..init + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    fn match_balance(&mut self, mut s: usize, p: usize) -> Result<Option<usize>, String> {
        if p + 1 >= self.pat.len() {
            return Err("unbalanced pattern".into());
        }
        if s >= self.src.len() || self.src[s] != self.pat[p] {
            return Ok(None);
        }
        let b = self.pat[p];
        let e = self.pat[p + 1];
        let mut cont = 1i32;
        loop {
            s += 1;
            if s >= self.src.len() {
                break;
            }
            if self.src[s] == e {
                cont -= 1;
                if cont == 0 {
                    return Ok(Some(s + 1));
                }
            } else if self.src[s] == b {
                cont += 1;
            }
        }
        Ok(None)
    }

    // ---- キャプチャ抽出 -----------------------------------------------------

    /// `i` 番目のキャプチャを抽出する（本家 `push_onecapture`）。
    fn one_capture(&self, i: usize, s: usize, e: usize) -> Result<Cap, String> {
        if i >= self.level {
            if i == 0 {
                Ok(Cap::Str(s, e - s)) // キャプチャ無し: マッチ全体
            } else {
                Err("invalid capture index".into())
            }
        } else {
            let l = self.capture[i].len;
            if l == CAP_UNFINISHED {
                return Err("unfinished capture".into());
            }
            if l == CAP_POSITION {
                Ok(Cap::Pos(self.capture[i].init))
            } else {
                Ok(Cap::Str(self.capture[i].init, l as usize))
            }
        }
    }

    /// マッチ `[s, e)` のキャプチャ列を返す（本家 `push_captures`）。
    ///
    /// キャプチャが無い場合、`whole_if_empty` が真ならマッチ全体を 1 件返す。
    pub fn captures(&self, s: usize, e: usize, whole_if_empty: bool) -> Result<Vec<Cap>, String> {
        let nlevels = if self.level == 0 && whole_if_empty { 1 } else { self.level };
        let mut v = Vec::with_capacity(nlevels);
        for i in 0..nlevels {
            v.push(self.one_capture(i, s, e)?);
        }
        Ok(v)
    }
}

// ============================================================================
// 文字クラス照合（自由関数）
// ============================================================================

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// 単一文字クラス `%a` 等の照合（本家 `match_class`）。
fn match_class(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => is_space(c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return cl == c, // 非英字クラス: そのまま比較
    };
    if cl.is_ascii_uppercase() { !res } else { res }
}

/// `[...]` 集合クラスの照合（本家 `matchbracketclass`）。`p`=`[` 位置, `ec`=`]` 位置。
fn match_bracket_class(c: u8, p: usize, ec: usize, pat: &[u8]) -> bool {
    let mut sig = true;
    let mut p = p;
    if pat.get(p + 1) == Some(&b'^') {
        sig = false;
        p += 1;
    }
    loop {
        p += 1;
        if p >= ec {
            break;
        }
        if pat[p] == L_ESC {
            p += 1;
            if match_class(c, pat[p]) {
                return sig;
            }
        } else if pat.get(p + 1) == Some(&b'-') && p + 2 < ec {
            // 範囲 a-z
            if pat[p] <= c && c <= pat[p + 2] {
                return sig;
            }
            p += 2;
        } else if pat[p] == c {
            return sig;
        }
    }
    !sig
}

/// 1 パターン要素 `[p, ep)` と 1 文字 `c` の照合（本家 `singlematch`）。
fn single_match(c: u8, p: usize, ep: usize, pat: &[u8]) -> bool {
    match pat[p] {
        b'.' => true,
        L_ESC => match_class(c, pat[p + 1]),
        b'[' => match_bracket_class(c, p, ep - 1, pat),
        other => other == c,
    }
}

/// パターンに特殊文字が含まれるか（`find` のプレーン検索判定, 本家 `strpbrk` 相当）。
pub fn has_specials(pat: &[u8]) -> bool {
    pat.iter().any(|b| SPECIALS.contains(b))
}

/// 相対位置の正規化（本家 `posrelat`）。負値は末尾からの逆算。
pub fn posrelat(pos: i64, len: usize) -> i64 {
    if pos >= 0 {
        pos
    } else if (-pos) as usize > len {
        0
    } else {
        len as i64 + pos + 1
    }
}
