# 公式 Lua 5.1 テストスイート（PUC-Rio）取り込み

本ディレクトリは、PUC-Rio が配布する**公式 Lua 5.1 テストスイート**（`lua5.1-tests.tar.gz`）を
置いて rua で実行し、パス率を追跡するための場所。テスト本体はライセンス上別配布のため
**リポジトリにはコミットしない**（`.gitignore` 済み）。各自が `fetch.sh` で取得する。

## 取得

```bash
tests/lua-suite/fetch.sh
# → tests/lua-suite/lua-5.1-tests/ に *.lua が展開される
```

## 実行（パス率の確認）

ハーネス `crates/rua-cli/tests/official_suite.rs` が本ディレクトリ配下の `*.lua` を
`rua run` で実行し、終了コード 0 を pass として集計する。重く現状はほぼ fail するため
既定では `#[ignore]`。明示実行する:

```bash
# rua で実行してパス率を表示
cargo test -p rua-cli --test official_suite -- --ignored --nocapture
```

## 注意・現状

公式スイートは `debug`・`coroutine`・`os`・`io`・`package`/`require`・`collectgarbage` 等の
広範な機能と本家固有のエラー文言に依存する。第一マイルストーン（基本実行）の段階では
大半が fail する想定。**パス率を経時で追跡**し、機能実装の進捗指標とする。

代表的な構成ファイル（取得後）:
`all.lua`（ドライバ）, `api.lua`, `attrib.lua`, `big.lua`, `calls.lua`, `checktable.lua`,
`closure.lua`, `code.lua`, `constructs.lua`, `db.lua`, `errors.lua`, `events.lua`,
`files.lua`, `gc.lua`, `literals.lua`, `locals.lua`, `math.lua`, `nextvar.lua`,
`pm.lua`, `sort.lua`, `strings.lua`, `vararg.lua`, `verybig.lua` など。
