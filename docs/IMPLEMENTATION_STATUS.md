# 実装ステータス（進捗の在り処）

> **進捗・残作業のトラッキングは GitHub へ移行しました。** 本ファイルは案内（ポインタ）です。
> ドキュメント方針はハイブリッド（設計＝リポジトリ / 進捗＝GitHub / パス率＝CI）。

- 最終更新: 2026-06-13

## 進捗・タスク（GitHub）

- **Project（ロードマップ board）**: https://github.com/users/takeshiD/projects/4
- **Milestones**: [公式スイート 50%](https://github.com/takeshiD/rua/milestones) / 公式スイート 100% / ruajit Phase A (MVP)
- **Issues（area ラベルで分類）**:
  [`area:vm`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Avm) ・
  [`area:stdlib`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Astdlib) ・
  [`area:capi`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Acapi) ・
  [`area:gc`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Agc) ・
  [`area:api`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Aapi) ・
  [`luajit`](https://github.com/takeshiD/rua/issues?q=is%3Aissue+is%3Aopen+label%3Aluajit)

## 公式スイート パス率（CI で自動追跡）

- 手書き表は廃止。**最新値は CI の `official-suite` ジョブのサマリ**（Actions 画面）を参照。
- ベースライン 2026-06-13: **3/23 (13.0%)**, crash 0。
- 手元測定: `tests/lua-suite/fetch.sh` 後に
  `cargo test -p rua-cli --test official_suite -- --ignored --nocapture`。
- 追跡トラッキング issue: [公式 20本を緑にする](https://github.com/takeshiD/rua/issues/19)

## 設計ドキュメント（リポジトリに残す）

- [ARCHITECTURE.md](ARCHITECTURE.md) — 設計・GC 戦略・フェーズ
- [CONFORMANCE.md](CONFORMANCE.md) — 互換性検証の手順・ハーネス
- [RUAJIT_SPIKE.md](RUAJIT_SPIKE.md) — ruajit（LuaJIT 互換）の調査スパイク

## メンテ方針

- 残作業は **markdown に書かず Issue/Project で管理**する。
- 設計判断・調査は本 `docs/`（必要なら ADR）に残す。
- パス率は **CI が自動で出す**（このファイルに数値表を作らない）。
