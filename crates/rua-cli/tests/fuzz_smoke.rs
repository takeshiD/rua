//! パニック/abort 検出スモークテスト（lua-conformance 所有, ARCHITECTURE.md §8）。
//!
//! 設計原則「パニック（abort）は互換性上ほぼ常にバグ」に基づき、構造化/ランダムな
//! Lua ソースを `rua run` に与えて **プロセスがクラッシュ（panic=exit101 / signal）しない**
//! ことを検証する。Lua レベルの構文エラー・実行時エラー（exit 1）は正常な結果として許容する。
//!
//! - 安定版 Rust の `cargo test` で動く（cargo-fuzz/nightly 不要）。本格ファジングは `fuzz/` 参照。
//! - rua が未実装（live でない）ときは自動スキップ。
//! - シードは固定（決定的）。クラッシュを起こした入力はメッセージに出力して再現可能にする。

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const RUA_BIN: &str = env!("CARGO_BIN_EXE_rua");

/// 手で選んだ「壊しにきた」エッジケース群。パーサ/VM/stdlib の境界を突く。
const ADVERSARIAL: &[&str] = &[
    // 数値の極端値・特殊形式
    "print(1e309, -1e309, 0/0, 1/0, -1/0)",
    "print(0x7fffffffffffffff, 0xffffffffffffffff)",
    "print(math.huge - math.huge)",
    "print(2^63, 2^1024, -2^1024)",
    // 文字列の境界
    "print(string.rep('x', 0), #string.rep('a', 1000))",
    "print(('a'):rep(100):len())",
    "local s = ('%q'):format('\\0\\n\\r\\\"\\\\'); print(#s)",
    "print(string.byte('hello', 1, -1))",
    // 深いネスト（パーサの再帰）
    "local t = {{{{{{{{{{1}}}}}}}}}}; print(t[1][1][1][1][1][1][1][1][1][1])",
    "print(((((((((((1))))))))))",
    "print(1+2*3-4/2^2%3 .. 'x' .. (not nil and 'y' or 'z'))",
    // テーブル長/境界
    "local t = {}; for i=1,1000 do t[i]=i end; print(#t)",
    "local t = {1,2,3,nil,5}; print(#t >= 3)",
    "local t = setmetatable({}, {__index=function() return 0 end}); print(t.anything)",
    // メタテーブル再帰・連鎖
    "local mt={}; mt.__index=mt; local t=setmetatable({},mt); print(t.x)",
    "local a=setmetatable({},{__add=function() return 42 end}); print(a+a, a+1, 1+a)",
    "local t=setmetatable({},{__tostring=function() return 'X' end}); print(tostring(t))",
    // pcall/error の嵐
    "print(pcall(error))",
    "print(pcall(function() error() end))",
    "print(pcall(function() error(nil) end))",
    "print(pcall(function() return pcall(function() return pcall(error, 'deep') end) end))",
    "print(select('#', pcall(function() return 1,2,3,4,5 end)))",
    // 可変長/select 境界
    "local function f(...) return select('#', ...) end; print(f(), f(nil), f(nil,nil))",
    "print(select(1, 'a','b','c'))",
    // 再帰（スタック）
    "local function r(n) if n<=0 then return 0 end return 1+r(n-1) end; print(r(200))",
    "local function tc(n,a) if n==0 then return a end return tc(n-1,a+1) end; print(tc(50000,0))",
    // 文字列パターンの境界
    "print(('aaa'):gsub('a*', 'x'))",
    "print(('abc'):find('()'))",
    "print(('hello world'):gmatch('%w+')())",
    "print(('a.b.c'):gsub('%.', '/'))",
    // 型混在・強制
    "print('10'+5, 10 .. 20, tostring(nil) .. tostring(true))",
    "print(tonumber('  0x1p4  '), tonumber('inf'), tonumber('nan'))",
    // 空・自明
    "",
    ";;;;",
    "do end",
    "while false do end",
    "for i=1,0 do print('never') end print('ok')",
    "repeat until true",
    // ローカル/スコープ
    "local a,b,c = 1; print(a,b,c)",
    "local a,b,c = 1,2,3,4,5; print(a,b,c)",
    "local x = 1; do local x = 2; end; print(x)",
];

/// パーサを狙う「トークンスープ」。構文的に壊れていてもパニックしてはならない（クリーンな構文エラー終了は可）。
fn token_soup(seed: u64, len: usize) -> String {
    const TOKENS: &[&str] = &[
        "function", "end", "if", "then", "else", "elseif", "while", "do", "for", "in", "repeat",
        "until", "return", "break", "local", "nil", "true", "false", "and", "or", "not", "{", "}",
        "(", ")", "[", "]", "=", "==", "~=", "<", ">", "..", "...", "+", "-", "*", "/", "%", "^",
        "#", ",", ";", ":", ".", "1", "0xFF", "1e10", "'s'", "\"d\"", "x", "foo", "print", "[[",
        "]]",
    ];
    let mut state = seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);
    let mut next = || {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545F4914F6CDD1D)
    };
    let mut s = String::new();
    for _ in 0..len {
        let t = TOKENS[(next() as usize) % TOKENS.len()];
        s.push_str(t);
        s.push(' ');
    }
    s
}

fn rua_is_live() -> bool {
    match std::env::var("RUA_CONFORMANCE").as_deref() {
        Ok("run") => return true,
        Ok("skip") => return false,
        _ => {}
    }
    let tmp = std::env::temp_dir().join(format!("rua_fzprobe_{}.lua", std::process::id()));
    if std::fs::write(&tmp, b"print(1)\n").is_err() {
        return false;
    }
    let out = Command::new(RUA_BIN).arg("run").arg(&tmp).output();
    let _ = std::fs::remove_file(&tmp);
    matches!(out, Ok(o) if o.status.success())
}

/// 1 本のソースを `rua run` で実行し、クラッシュ（panic/abort/signal）を検出する。
/// 戻り値: `Some(reason)` ならクラッシュ。`None` なら正常（成功 or クリーンなエラー終了）。
fn run_and_detect_crash(idx: usize, source: &str) -> Option<String> {
    let path: PathBuf =
        std::env::temp_dir().join(format!("rua_fz_{}_{}.lua", std::process::id(), idx));
    {
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => return Some(format!("一時ファイル作成失敗: {e}")),
        };
        if f.write_all(source.as_bytes()).is_err() {
            return Some("一時ファイル書込失敗".into());
        }
    }
    let result = Command::new(RUA_BIN).arg("run").arg(&path).output();
    let _ = std::fs::remove_file(&path);

    match result {
        Ok(out) => match out.status.code() {
            // Rust パニックは 101。abort/SIGSEGV 等はシグナル終了で code() == None。
            Some(101) => Some(format!(
                "rua がパニック (exit 101)\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )),
            None => Some(format!(
                "rua がシグナルで異常終了 (abort/segfault 等)\nstatus: {:?}\nstderr:\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )),
            // 0(成功) や 1(Lua エラー) などは正常。クラッシュではない。
            Some(_) => None,
        },
        Err(e) => Some(format!("rua 起動失敗: {e}")),
    }
}

#[test]
fn smoke_no_panic_on_adversarial_sources() {
    if !rua_is_live() {
        eprintln!("[fuzz-smoke] SKIP: rua がまだ実行できません（CLI/VM 未完）。");
        return;
    }

    let mut crashes = Vec::new();
    let mut total = 0usize;

    // 1) 手選びの敵対的入力
    for (i, src) in ADVERSARIAL.iter().enumerate() {
        total += 1;
        if let Some(reason) = run_and_detect_crash(i, src) {
            crashes.push(format!("[adversarial #{i}] source = {src:?}\n{reason}"));
        }
    }

    // 2) 構造化ランダム（パーサ狙い）— 決定的シードで再現可能
    for seed in 0..200u64 {
        let len = 5 + (seed as usize % 40);
        let src = token_soup(seed, len);
        total += 1;
        if let Some(reason) = run_and_detect_crash(1000 + seed as usize, &src) {
            crashes.push(format!(
                "[token-soup seed={seed} len={len}] source = {src:?}\n{reason}"
            ));
        }
    }

    eprintln!(
        "[fuzz-smoke] {total} 本実行, クラッシュ {} 件",
        crashes.len()
    );

    assert!(
        crashes.is_empty(),
        "rua がクラッシュした入力を検出 ({} 件):\n\n{}",
        crashes.len(),
        crashes.join("\n\n---\n\n")
    );
}
