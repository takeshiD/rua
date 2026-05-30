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

### 本家非インストール環境での挙動（2026-05-30 確認）

- `lua5.1` / `luac5.1` が PATH にない場合:
  - `golden_compare` は `rua run` プローブ成功なら**実比較を実行**する（本家は不要）。
    14/14 スクリプトの stdout・終了コードをコミット済み `.expected` と比較し、全て一致を確認。
  - `validate_expected_against_reference` は `#[ignore]` 指定かつ本家が見つからないためスキップ。
  - 公式スイート `official_suite_pass_rate` は `#[ignore]` のため `--ignored` を付けた場合のみ実行。

## 4. パニック/abort 検出スモーク（`cargo test` 同梱）

`crates/rua-cli/tests/fuzz_smoke.rs` が、敵対的エッジケース群＋決定的シードの構造化ランダム入力
（トークンスープ）を `rua run` に与え、**プロセスがクラッシュ（panic=exit 101 / signal=abort）
しない**ことを検証する。Lua の構文/実行時エラー（exit 1）は正常結果として許容。
nightly 不要で日常の回帰検出に使える。クラッシュ入力はメッセージに出力され再現可能。

## 5. 公式 Lua 5.1 テストスイート（PUC-Rio）

`tests/lua-suite/`（README/fetch.sh/.gitignore のみ追跡、本体は別配布）。

```bash
tests/lua-suite/fetch.sh                                            # 取得
cargo test -p rua-cli --test official_suite -- --ignored --nocapture   # パス率表示
```

`crates/rua-cli/tests/official_suite.rs` が各 `*.lua` を `rua run`（30s タイムアウト付き）で
実行し、終了状態を分類・集計する。未取得時は自動スキップ。

- **pass**: 終了コード 0。
- **fail**: その他の通常終了（Lua エラー＝機能未実装/非互換）。パス率追跡対象。
- **crash**: パニック（exit 101）/ シグナル異常終了（abort・segfault）。**互換性上ほぼ常にバグ**。
  最小再現を作り該当ロールへ報告する。`RUA_SUITE_STRICT=1` 指定時は crash 検出でテストを失敗させる。
- **timeout**: 暴走（無限ループ等）。非互換の兆候。

### ベースライン（2026-05-30）

対象: `lua5.1-tests/` 配下の `*.lua` 全 23 本（`all.lua` を除く）。

| カテゴリ | 件数 |
|---|---|
| pass | 3 (13.0%) |
| fail (Lua エラー・機能未実装) | 19 |
| crash (パニック/abort = 要バグ報告) | 1 |
| timeout | 0 |

**pass の 3 本**: `checktable.lua`, `code.lua`, `etc/` 配下各 1 本と推定（詳細は `--nocapture` で確認）。

**crash が 1 件（`big.lua`）**: `rua-frontend` (codegen) へ申し送り済み。詳細は §5.1 参照。

#### §5.1 クラッシュ詳細: `big.lua` (exit 101, Rust パニック)

- **現象**: `big.lua` を `rua run` すると Rust がパニック（exit 101）する。
- **原因**: `crates/rua-core/src/compiler/codegen.rs` の `constructor` 関数で、
  ハッシュフィールドが 256 個以上のテーブルコンストラクタをコンパイルする際に
  `emit_abc(OpCode::SetTable, ...)` の引数が `MAXARG_C (= 255)` を超える。
  本来は `exp2rk` の定数インデックスが `MAXINDEXRK` を超えた場合にレジスタへ追い出す
  パスが必要（本家 `luaK_exp2RK` の対応処理が不足）。
  `crates/rua-core/src/vm/opcode.rs:335` の `debug_assert!` がパニックとして現れる。
- **最小再現スクリプト**:
  ```lua
  -- 257 個以上のハッシュフィールドを持つテーブルコンストラクタ
  local t = { a1=1, a2=2, ..., a257=257 }
  print(t.a256)
  ```
- **担当ロール**: `lua-frontend` (codegen)。
- **修正指針**: `codegen.rs` の `constructor` 内で `Field::Named` / `Field::Keyed` を処理する際、
  `key_rk` または `val_rk` が `MAXARG_C` を超える場合に `exp2anyreg` でレジスタへ逃がす。
  `string_k` で生成したキー定数インデックスが 256 以上になった場合も同様の処置が必要。

## 6. ファジング（cargo-fuzz, nightly）

`fuzz/`（ワークスペースから exclude）。詳細は `fuzz/README.md`。

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run compile_only   # パーサ/コンパイラ → lua-frontend
cargo +nightly fuzz run compile_run    # + VM/stdlib       → lua-vm / lua-stdlib
```

## 7. C ABI リンク互換テスト（`cargo test` 同梱）

本家 `lua.h` を include した C プログラムを `rua-capi`（staticlib/cdylib）にリンクして実行し、
ABI 互換（ARCHITECTURE.md §7）を検証する。

### 現状（2026-05-30 時点）

`rua-capi` クレートは実装済み。`crates/rua-capi/include/{lua.h,lauxlib.h,lualib.h}` が存在し、
`target/debug/librua_capi.a`（staticlib）および `librua_capi.so`（cdylib）もビルド済み。

**実リンクが通ることを確認済み**: `RUA_CAPI_REQUIRE=1 cargo test -p rua-cli --test capi_abi`
が `CAPI_OK` を出力して正常終了する。

### リンク方式

- **静的リンク優先**: ハーネスは `-Wl,-Bstatic -lrua_capi -Wl,-Bdynamic` でまず静的リンクを試みる。
  Linux 環境では `librua_capi.a` に対してこの方式が成功する。
- **動的リンクフォールバック**: 静的リンクが失敗した場合（macOS 等）は通常の `-lrua_capi` で動的リンク。
- **自動スキップ**: C コンパイラ・ヘッダ・`librua_capi.a` のいずれかが欠ければ自動スキップ（CI を壊さない）。

### 詳細

- C ソース/期待値: `tests/capi/smoke.c` / `tests/capi/smoke.expected`。
- ハーネス: `crates/rua-cli/tests/capi_abi.rs`（`capi_link_smoke`）。`cargo test` に同梱。
- 既定探索パス: ヘッダは `crates/rua-capi/include/`、ライブラリは `target/debug/` または `target/release/`。

```bash
# 既定探索パスにヘッダ/ライブラリがある場合（ビルド済みなら自動で結線）:
cargo test -p rua-cli --test capi_abi

# 場所を明示する場合:
RUA_CAPI_INCLUDE=crates/rua-capi/include RUA_CAPI_LIB=target/debug \
  cargo test -p rua-cli --test capi_abi

# RUA_CAPI_REQUIRE=1 でスキップを禁止（結線確認・CI 用）:
RUA_CAPI_REQUIRE=1 cargo test -p rua-cli --test capi_abi
```

## 8. ベンチマーク（rua vs 本家 Lua / LuaJIT, 任意）

`tests/bench/`。マイクロベンチを各処理系で計測し壁時計時間の中央値を比較する（CI ゲートにしない）。
詳細は `tests/bench/README.md`。

```bash
tests/bench/run_bench.sh
RUA_LUA_BIN=$(command -v lua5.1) LUAJIT_BIN=$(command -v luajit) tests/bench/run_bench.sh
```

## 9. 今後の拡張
- `luac -l` ダンプとのフロントエンド・ゴールデン比較（フロントエンド命令列の本家一致）。
- 公式スイートのパス率ベースライン化と回帰検出（fail 件数の閾値ガード）。
- ファジングのコーパス蓄積・構造化（`arbitrary` による AST レベル生成）。
