# rua

PUC-Rio Lua 5.1 との完全互換を目標とした、Rust 製 Lua 5.1 インタープリタです。

> [English README](README.md)

## 特徴

- **Lua 5.1 言語仕様の完全サポート** — 全構文・演算子・メタテーブル・クロージャ・可変長引数・多値返却・末尾呼び出し最適化
- **レジスタ型バイトコード VM** — 本家 PUC-Rio と同じアーキテクチャ
- **標準ライブラリ** — `base`, `string`, `table`, `math`, `io`, `os`（[実装状況](#標準ライブラリ実装状況)を参照）
- **リッチな対話 REPL** — シンタックスハイライト・Tab 補完・永続履歴・複数行継続
- **`luac` 相当のコンパイラ** — バイトコード列挙・構文チェック・チャンク出力
- **シェル補完** — bash / zsh / fish / elvish / powershell
- **C API レイヤー** — `lua.h` ABI 互換の `extern "C"` 関数群（cdylib / staticlib）
- **Rust 組み込み API** — `mlua` / `rlua` 風の安全な高レベル API
- **ガベージコレクタ** — アリーナ + マーク & スイープ（`unsafe` 不使用）

## インストール

### ソースからビルド

Rust **1.96** 以降（stable）が必要です。

```bash
git clone https://github.com/takeshiD/rua
cd rua
cargo build --release
```

バイナリは `target/release/rua` に生成されます。

`~/.cargo/bin` にインストールする場合:

```bash
cargo install --path crates/rua-cli
```

## クイックスタート

```bash
# スクリプトを実行
rua script.lua

# 引数つきで実行（arg[1], arg[2], ... および ... でアクセス可能）
rua script.lua foo bar

# 標準入力から読み込む
echo 'print("hello, world")' | rua -

# 対話モード（REPL）を起動
rua
```

## CLI リファレンス

### `rua run` — スクリプト実行

```bash
rua run script.lua [引数...]
rua run -               # 標準入力から実行
```

スクリプト引数は `arg[0]`, `arg[1]`, ... およびメインチャンクの `...` からアクセスできます。これは本家 `lua5.1` バイナリと同じ規約です。

### `rua`（サブコマンド省略）— 短縮形

```bash
rua script.lua [引数...]   # rua run と同じ
rua                        # REPL を起動
```

### `rua repl` — 対話インタープリタ

```bash
rua repl
```

| キー | 動作 |
|------|------|
| `Tab` | 補完候補を表示 |
| `Enter` | 実行（ブロックが未完なら継続） |
| `Ctrl-C` | 現在の入力を破棄 |
| `Ctrl-D` | REPL を終了 |

式を入力すると自動的に評価して値を表示します（例: `1+2` → `3`）。  
履歴は `~/.local/share/rua/history.txt` に保存されます。

### `rua luac` — コンパイラ

```bash
rua luac -p script.lua              # 構文チェックのみ（成功時は無出力）
rua luac -l script.lua              # バイトコード命令を列挙
rua luac -ll script.lua             # バイトコード＋定数表・ローカル変数・upvalue も表示
rua luac -o out.rbc script.lua      # コンパイル済みチャンクをファイルへ出力
rua luac -s -o out.rbc script.lua   # デバッグ情報を除去して出力
rua run out.rbc                     # コンパイル済みチャンクを実行
```

### `rua completions` — シェル補完

```bash
# bash
rua completions bash >> ~/.bashrc

# zsh
rua completions zsh > ~/.zfunc/_rua
# ~/.zshrc に fpath=(~/.zfunc $fpath) と autoload -U compinit が必要

# fish
rua completions fish > ~/.config/fish/completions/rua.fish
```

## 標準ライブラリ実装状況

| ライブラリ | 状況 | 実装内容 |
|------------|------|----------|
| `base` | ✅ 完了 | `print`, `type`, `tostring`, `tonumber`, `pairs`, `ipairs`, `next`, `select`, `error`, `assert`, `pcall`, `xpcall`, `rawget`, `rawset`, `rawequal`, `setmetatable`, `getmetatable`, `unpack`, `_G`, `_VERSION` |
| `string` | ✅ 完了 | パターンエンジン完全実装（`find`, `match`, `gmatch`, `gsub` を含む全関数） |
| `table` | ✅ 完了 | `insert`, `remove`, `concat`, `sort`, `maxn` |
| `math` | ✅ 完了 | 三角関数・指数・丸め・乱数など全関数 |
| `io` | ✅ 完了 | `io.open`, `io.close`, `io.read`, `io.write`, `io.lines`, `io.flush`, `io.input`, `io.output`, `io.type`, `io.stdin/stdout/stderr`、全 `file:*` メソッド |
| `os` | 🔶 一部 | `os.time`, `os.date`, `os.clock`, `os.exit` 実装済み。`os.execute`, `os.getenv`, `os.remove`, `os.rename` は未実装 |
| `debug` | ❌ 未実装 | 予定 |
| `package` / `require` | ❌ 未実装 | 予定 |
| `coroutine` | ❌ 未実装 | 予定 |

### 既知の制限

- `s:upper()` などの**文字列メソッド構文**は共有文字列メタテーブルが未接続のため動作しません。代わりに `string.upper(s)` を使ってください。
- `error(msg, level)` のエラー位置情報（行番号プレフィックス）は近似値です（CallInfo に pc/行番号マッピングが未実装）。

## アーキテクチャ

```
rua/
├── crates/
│   ├── rua-core/        # レキサー → パーサー → コードジェネレータ → VM → GC · stdlib · Rust API
│   ├── rua-cli/         # スタンドアロンインタープリタ（rua run, repl, luac）
│   └── rua-capi/        # C API レイヤー（lua.h ABI 互換 cdylib + staticlib）
├── tests/
│   ├── lua/             # ゴールデンテスト 15 本（lua5.1 との出力比較）
│   └── lua-suite/       # PUC-Rio 公式テストスイート統合
├── fuzz/                # cargo-fuzz ターゲット（compile_only, compile_run）
└── docs/
    ├── ARCHITECTURE.md  # 設計方針・GC 戦略・開発フェーズ
    └── CONFORMANCE.md   # テスト戦略・ゴールデンハーネス・リファレンス管理
```

### クレートの責務

| クレート | 役割 | Lua 5.1 本家対応 |
|---------|------|-----------------|
| `rua-core` | CLI フロント以外の全実装 | `llex.c`, `lparser.c`, `lcode.c`, `lvm.c`, `lgc.c`, `lstate.c`, `ldo.c`, `lib*.c` |
| `rua-cli` | `rua` バイナリ・REPL・luac | `lua.c`, `luac.c` |
| `rua-capi` | `extern "C"` ABI レイヤー | `lapi.c`, `lauxlib.c` |

### 値モデル

```
Lua 型               Rust 表現
────────────────────────────────────────────────────
nil                  Value::Nil
boolean              Value::Boolean(bool)
number               Value::Number(f64)         ← Lua 5.1 は全数値が double
string               Value::GcRef(GcHandle::Str(_))      ← 文字列インターン済み
table                Value::GcRef(GcHandle::Table(_))
function             Value::GcRef(GcHandle::Closure(_))
userdata             Value::GcRef(GcHandle::Userdata(_))
lightuserdata        Value::LightUserData(*mut c_void)
```

GC オブジェクトは型別アリーナ（[slotmap](https://docs.rs/slotmap)）に格納し、`Value` は世代付きインデックスを保持します。GC 走査に `unsafe` は不要です。

## テストの実行

```bash
# 単体テスト + 統合テスト
cargo test --workspace

# ゴールデン .expected の本家 lua5.1 との検証
# （要 lua5.1: apt install lua5.1）
cargo test -p rua-cli -- --ignored validate_expected_against_reference

# PUC-Rio 公式テストスイート（lua.org から取得）
tests/lua-suite/fetch.sh
cargo test -p rua-cli --test official_suite -- --ignored --nocapture

# ファジング（nightly + cargo-fuzz が必要）
cargo +nightly fuzz run compile_only -- -max_total_time=60
cargo +nightly fuzz run compile_run  -- -max_total_time=60
```

## Rust からの組み込み

```rust
use rua_core::{LuaState, stdlib};

let mut state = LuaState::new();
stdlib::open_libs(&mut state);

// ネイティブ関数を登録
state.register("add", |state| {
    let a = state.check_number(1)?;
    let b = state.check_number(2)?;
    state.push_number(a + b);
    Ok(1)
});

// Lua コードを実行
state.do_string("print(add(1, 2))")?;
```

## C / C++ からの組み込み

同梱ヘッダをインクルードし `librua_capi` にリンクするだけで使えます:

```c
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"

int main(void) {
    lua_State *L = luaL_newstate();
    luaL_openlibs(L);
    luaL_dostring(L, "print('hello from C')");
    lua_close(L);
    return 0;
}
```

```bash
# 静的リンク
gcc main.c -Icrates/rua-capi/include \
    target/release/librua_capi.a -lpthread -ldl -lm -o demo
```

## 互換性

`rua` は **Lua 5.1** をターゲットとしています（Neovim・Redis・World of Warcraft アドオンなどに組み込まれているバージョンと同じです）。

意図的な非目標:
- LuaJIT 拡張（`bit`, `ffi`, `jit`）
- Lua 5.2 以降の機能（`goto`、整数サブタイプ、ビット演算子など）
- JIT コンパイル

## コントリビューション

```bash
# CI と同じ検査を手元で実行
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI は **Rust stable**（`dtolnay/rust-toolchain@stable`）で動きます。バージョン差による CI 失敗を防ぐため、手元のツールチェーンも最新に保ってください（`rustup update stable`）。

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
