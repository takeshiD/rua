# 実装ステータス（残作業トラッキング）

`rua`（Rust による Lua 5.1 実装）の **未実装・不完全項目** と **公式テストスイートのパス率** を
継続的に追跡するための一覧。新機能を実装したら本ファイルを更新する。

- 最終更新: 2026-06-13
- 北極星指標: **PUC-Rio 公式 Lua 5.1 テストスイートのパス率**（下記 §1）
- 参照: [ARCHITECTURE.md](ARCHITECTURE.md)（設計・フェーズ）, [CONFORMANCE.md](CONFORMANCE.md)（検証手順）

> 注意: `README.md` の "Standard Library Status" 表は実装より遅れている場合がある。
> 実装の正は本ファイルと各ソースの TODO、および公式スイートのパス率とする。

---

## 1. 公式テストスイート パス率（ゴール指標）

取得・実行手順:

```bash
tests/lua-suite/fetch.sh                                              # 本体取得（gitignore 済・非コミット）
cargo test -p rua-cli --test official_suite -- --ignored --nocapture # パス率測定
```

| 日付 | pass | fail | crash | timeout | パス率 |
|---|---|---|---|---|---|
| 2026-05-30 | 3/23 | 19 | 1 (`big.lua`) | 0 | 13.0% |
| 2026-06-13 | 3/23 | 20 | 0 | 0 | 13.0% |

- 2026-06-13 時点で **crash は 0**（`big.lua` の codegen パニックは exit 1=fail に改善済み）。
- 対象 23 本（`all.lua` 除く）。pass は `checktable.lua` / `code.lua` / `etc/` 配下と推定。

### 現在 fail している公式テスト（20本, 2026-06-13）

`attrib`, `big`, `calls`, `closure`, `constructs`, `db`, `errors`, `events`, `files`,
`gc`, `literals`, `locals`, `main`, `math`, `nextvar`, `pm`, `sort`, `strings`,
`vararg`, `verybig`

> これらを 1 本ずつ緑にしていくのが当面の作業。各テストの fail 原因が下記 §2〜§5 の
> どの未実装項目に対応するかを、着手時に切り分けて追記していく。

---

## 2. 言語仕様 / VM コア（優先度 1）

| 項目 | 状態 | 場所 |
|---|---|---|
| 文字列メタテーブルの配線（`s:upper()` 形式のメソッド呼び出し） | 未配線。`string.upper(s)` で代替。`interp::metatable_of` が文字列のメタテーブルを返す必要 | `stdlib/mod.rs`, `value/string.rs:10` |
| `error(msg, level)` の位置プレフィックス（`chunk:line:`） | 近似のみ。CallInfo に pc→line マッピングが無く正確な行番号が出ない | `error.rs:36` |

---

## 3. 標準ライブラリ（優先度 2）

実装済み: `base` / `string`（パターン含む）/ `table` / `math` / `io` / `os` / `package`(require) / `coroutine`
（`stdlib/mod.rs` の `open_libs` で登録）。

| 項目 | 状態 | 場所 |
|---|---|---|
| `debug` ライブラリ | **完全に未登録**（`open_libs` に無し）。唯一まるごと欠けているライブラリ | `stdlib/mod.rs` |
| `coroutine.running()` のメインスレッド判定 | 常に `nil` を返す簡易実装 | `stdlib/coroutine_lib.rs:332` |
| `require` の C ローダ | Pure-Lua モジュールのみ。`package.cpath` は空、C 動的ロード非対応 | `stdlib/package_lib.rs` |

---

## 4. C API（rua-capi, 優先度 3）

| 項目 | 状態 | 場所 |
|---|---|---|
| コルーチン API（`lua_newthread`/`resume`/`yield`） | 未対応 | `capi/lib.rs:18` |
| C 関数の environment（`LUA_ENVIRONINDEX`） | グローバルで近似、未実装 | `capi/lib.rs:234` |
| `lua_pcall` の `errfunc` | 未対応（0 前提） | `capi/lib.rs:980` |
| `lua_concat` の `__concat` メタメソッド | 未対応 | `capi/lib.rs:1282` |
| `lua_gc` | `LUA_GCCOLLECT` のみ実装。他操作は no-op で 0 返し | `capi/lib.rs:1304` |
| C 互換の生バイト userdata / `__gc` finalizer 起動 | 未対応 | `value/userdata.rs:10` |

---

## 5. 高レベル Rust API（優先度 4）

| 項目 | 状態 | 場所 |
|---|---|---|
| キャプチャ付きクロージャ・型付き引数/戻り値の ergonomic な登録 | 後続拡張 | `api/mod.rs:203` |
| 高レベル Userdata 型 | 後続拡張 | `api/mod.rs:127` |

---

## 6. GC（性能 / 互換フェーズ・横断）

現状は stop-the-world mark-and-sweep のみ（`gc/mod.rs:18`）。

- [ ] インクリメンタル GC
- [ ] weak table
- [ ] `__gc` finalizer の起動

---

## 7. 互換性検証インフラ（フェーズ 5・横断）

| 項目 | 状態 |
|---|---|
| 公式テストスイート ハーネス | ✅ 完備（`tests/lua-suite/fetch.sh` + `crates/rua-cli/tests/official_suite.rs`）。本体は gitignore で非コミット |
| golden テスト（本家比較・14 スクリプト） | ✅ 全通過（`crates/rua-cli/tests/golden.rs`） |
| パニック/abort スモーク | ✅（`crates/rua-cli/tests/fuzz_smoke.rs`） |
| C ABI リンクテスト | ✅ 結線確認済み（`crates/rua-cli/tests/capi_abi.rs`）。ケース拡充は今後 |
| ファジング（cargo-fuzz） | target あり（`fuzz/`）。コーパス蓄積・運用は今後 |
| ベンチマーク（rua vs 本家/LuaJIT） | スクリプトあり（`tests/bench/`）。CI ゲート外 |

---

## メンテナンス方針

- 公式スイートのパス率が変わったら §1 の表に 1 行追記する（日付・内訳）。
- 項目を実装し終えたら該当行を削除（または「✅ 完了」に更新）し、対応する公式テストが
  pass に転じたかを §1 で確認する。
- 新たに判明した未実装・非互換は、根拠となるソース位置（`file:line`）付きで追記する。
