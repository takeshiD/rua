# rua ファジング（cargo-fuzz）

パーサ・VM へランダム/構造化入力を与え、**パニック・abort・無限ループ**を検出する。
「パニック（abort）は互換性上ほぼ常にバグ」（役割定義）なので、クラッシュ＝報告対象。

`fuzz/` はワークスペースから `exclude` 済み（通常の `cargo build`/`cargo test` には影響しない）。

## 前提

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
```

## ターゲット

| ターゲット | 対象 | 担当への報告先 |
|---|---|---|
| `compile_only` | lexer→parser→codegen | lua-frontend |
| `compile_run`  | 上記 + VM 実行 + stdlib | lua-vm / lua-stdlib |

## 実行

```bash
cargo +nightly fuzz run compile_only
cargo +nightly fuzz run compile_run

# 時間制限・並列・コーパス指定の例
cargo +nightly fuzz run compile_run -- -max_total_time=60 -jobs=4
```

クラッシュを検出すると `fuzz/artifacts/<target>/` に再現入力が保存される。再現:

```bash
cargo +nightly fuzz run compile_run fuzz/artifacts/compile_run/crash-XXXX
```

## CI

nightly 依存のため通常 CI ジョブからは外している。短時間スモークを
`continue-on-error` の別ジョブで回す運用が可能（`.github/workflows/ci.yml` 参照）。

## 安定版での簡易パニック検出

nightly 無しでも、`crates/rua-cli/tests/fuzz_smoke.rs`（`cargo test` に同梱）が
敵対的入力＋構造化ランダム入力でクラッシュを検出する。日常の回帰検出はこちらで足りる。
