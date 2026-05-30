# 互換性検証（conformance）ガイド

`rua` が「本家 PUC-Rio Lua 5.1 と同じように動く」ことを保証するためのテスト資産・手順をまとめる。
運用は lua-conformance が担当する（役割定義 `.claude/agents/lua-conformance.md`、戦略 `ARCHITECTURE.md §8`）。

## 1. テスト資産の構成

```
tests/lua/
├── 01_arithmetic.lua        算術・優先順位・比較・論理・連結
├── 02_strings.lua           文字列リテラル/エスケープ/long string/string ライブラリ基本
├── 03_string_patterns.lua   find/match/gmatch/gsub とパターン
├── 04_tables.lua            コンストラクタ/配列/ハッシュ/#/table ライブラリ/sort
├── 05_control_flow.lua      if/while/repeat/数値for/ジェネリックfor/break
├── 06_functions.lua         関数/クロージャ/可変長引数/多値返却/再帰/末尾呼び出し
├── 07_metatables.lua        __index/__newindex/算術/__eq/__lt/__le/__call/__tostring/__concat
├── 08_errors.lua            pcall/error/assert/xpcall
├── 09_iterators.lua         ipairs/pairs/next/select
├── 10_conversions.lua       type/tostring/tonumber と数値強制
├── 11_math.lua              math ライブラリ主要関数
├── 12_recursive_data.lua    連結リスト/二分木/メモ化など再帰的データ構造
├── 13_io_os.lua             io.write と os の決定的部分
├── 14_uncaught_error.lua    捕捉されないエラーの終了コード(1)検証
├── *.expected               各スクリプトの期待 stdout（本家 5.1 準拠）
├── *.exitcode               期待する終了コード（省略時 0）
└── regenerate_expected.sh   本家 lua5.1 から .expected を再生成するスクリプト
```

### 決定性の方針
ゴールデン比較は stdout を**バイト単位で厳密比較**するため、各スクリプトの出力は決定的でなければならない。

- 数値整形は本家の `"%.14g"`（`print`/`tostring`/連結すべて共通）を前提にした値のみ使用。
- `pairs` のハッシュ部の列挙順は未規定 → キーをソートしてから出力。
- `math.random` 等の非決定的値は範囲・型のみ検証。
- エラーメッセージは既定 level だと `チャンク名:行: ` が前置されパス・行依存になる →
  位置情報が不要な箇所は `error(msg, 0)` を使い、必要な箇所は `string.match` で
  サフィックスのみ検証して真偽値を出力する。
- stderr はチャンク名・行番号を含むため**比較対象外**。終了コードは `.exitcode` で比較する。

## 2. ハーネス（cargo test 統合）

`crates/rua-cli/tests/golden.rs` に Rust 統合テストとして実装。`cargo test` に自動で含まれる。

| テスト | 役割 | 実行条件 |
|---|---|---|
| `every_script_has_expected` | 全 `*.lua` に `*.expected` が揃っているか | 常時 |
| `golden_compare` | `rua run <script>` の stdout/終了コードを期待値と比較 | rua が実行可能なときのみ（自動判定） |
| `validate_expected_against_reference` | コミット済み `.expected` が本家 `lua5.1` 出力と一致するか | `--ignored` 指定 + 本家あり |

### 自動スキップの仕組み
`golden_compare` は実行前に `print("RUA_PROBE_OK")` を `rua run` で試し、
正しく実行できた場合のみ本比較を行う。コンパイラ/VM/CLI が未完成の段階では自動的にスキップ
（テストはパス扱い）し、CI を壊さない。CLI が動くようになると自動的に比較へ切り替わる。

### 環境変数
- `RUA_CONFORMANCE=run` : スキップ判定を無視して強制実行（未実装だと失敗する）。
- `RUA_CONFORMANCE=skip`: 強制スキップ。
- `RUA_LUA_BIN=<path>`  : リファレンス Lua 5.1 のパス（検証/再生成用）。

## 3. リファレンス Lua 5.1 の導入

期待値は「本家を正」とする。手元で本家出力を生成・検証するには Lua 5.1 が必要。

```bash
# Debian/Ubuntu
sudo apt-get install lua5.1

# macOS (Homebrew) — 5.1 系
brew install lua@5.1

# ソースから
curl -R -O https://www.lua.org/ftp/lua-5.1.5.tar.gz
tar zxf lua-5.1.5.tar.gz && cd lua-5.1.5
make linux test        # or: make macosx
```

導入後の検証・再生成:

```bash
# コミット済み .expected が本家と一致するか検証
cargo test -p rua-cli -- --ignored validate_expected_against_reference

# .expected を本家出力で再生成
RUA_LUA_BIN=$(command -v lua5.1) tests/lua/regenerate_expected.sh
```

本家が無い環境では上記はスキップされ、コミット済みの `.expected`（本家 5.1 準拠で作成）を使う。

## 4. 今後の拡張（第二マイルストーン以降）
- 公式 Lua 5.1 テストスイート（PUC-Rio 配布の `*.lua`）の取り込みとパス率追跡。
- `luac -l` ダンプとのフロントエンド・ゴールデン比較。
- C ABI リンクテスト（`rua-capi`）。
- `cargo-fuzz` によるパーサ/VM ファジング。
- 本家 Lua / LuaJIT とのベンチマーク比較。
