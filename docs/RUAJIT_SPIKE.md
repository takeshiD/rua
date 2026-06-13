# ruajit スパイク（LuaJIT 互換実装の事前調査）

- ステータス: **調査（spike）** — 実装着手前の意思決定材料。結論が固まったら ADR / `ARCHITECTURE.md` に反映する。
- 最終更新: 2026-06-13
- 関連: [ARCHITECTURE.md](ARCHITECTURE.md), [CONFORMANCE.md](CONFORMANCE.md), [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md)
- 出典: [luajit.org/extensions.html](https://luajit.org/extensions.html), [openresty/luajit2-test-suite](https://github.com/openresty/luajit2-test-suite)

## 0. 目的とスコープ

`ruajit`（`rua` とは**別バイナリ**の CLI）で LuaJIT 互換の処理系を提供したい。
本スパイクは「何が必要か / どこに `unsafe` が要るか / どうテストするか」を洗い出し、
段階計画と未決事項（＝後続 ADR の論点）を提示する。
**本ドキュメント時点では実装しない。** 調査結果に基づき次の意思決定を行う。

## 1. 前提: LuaJIT は PUC Lua 5.1 の「別実装」

- **言語**は Lua 5.1 互換（+ 5.2/5.3 から選択的に機能を採用）。
- **ランタイム**（バイトコード・VM・インタプリタ・GC）は完全に別物。
- バイトコードは PUC Lua と**非互換**（現行 rua の bytecode / `ruac` とは別系統）。
- つまり「rua を速くしたもの」ではなく、**並走する第2処理系**を作るということ。

## 2. 言語レベルの差分（lexer / parser への影響）

LuaJIT が Lua 5.1 に追加する構文（出典: 公式 Extensions ページ）:

| 由来 | 機能 |
|---|---|
| 5.2（無条件採用） | `goto` 文 / `::label::`、`\x` 16進エスケープ、`\z` エスケープ、`load()` の mode/env 引数 |
| 5.3 | `\u{XX...}` Unicode（UTF-8）エスケープ |

→ rua-core の lexer/parser は Lua 5.1 準拠。上記拡張の追加が必要。
AST / parser は**大部分を共有可能**（モードフラグ、または薄いフロントエンド層で分岐）。

## 3. ライブラリ / ランタイムの差分と難度

| 機能 | 内容 | `unsafe` | 難度 |
|---|---|---|---|
| `bit.*` | 32/64bit ビット演算（`band`/`bor`/`bxor`/`lshift`/`rol`/`bswap` 等） | 不要 | 低 |
| `jit.*` | JIT 制御 API | 不要（インタプリタならスタブで可） | 低 |
| `table.new` / `table.clear` | 事前サイズ確保 / サイズ保持クリア | 不要 | 低 |
| 5.2/5.3 ライブラリ拡張 | `math.log(x,base)`, `string.rep(s,n,sep)`, `string.format` 改良, `io.read("*L")`, `package.searchpath`, `table.move`, `coroutine.isyieldable` 等 | 不要 | 中 |
| 強化 PRNG | `math.random` が Tausworthe（周期 2^223）。系列が PUC/標準と異なる | 不要 | 中（系列一致が要る） |
| Fully resumable VM | `pcall`/`xpcall`・メタメソッド・イテレータ越しに yield 可能 | 不要だが**VM 設計に影響** | 高 |
| `string.buffer` | 高速文字列バッファ（LuaJIT 2.1） | 不要 | 中 |
| **`ffi.*`** | C 関数 / C データ構造を直接利用 | **必須** | 非常に高 |
| **実トレーシング JIT** | ホット経路の記録→IR→機械語生成 | **必須**（実行可能メモリ） | 研究級 |

## 4. `unsafe` 方針への影響（最重要論点）

- rua の設計原則は「**`unsafe` 不要**（アリーナ GC）」。
- だが **FFI と 実 JIT は本質的に `unsafe` を要求する**:
  - FFI: 生ポインタ、`dlopen`、任意 C ABI 呼び出し。
  - JIT: `mmap`+exec な実行可能メモリへの機械語書き込みと飛び込み。
- 選択肢:
  - **(a) 隔離方式**: FFI/JIT を `unsafe` 許容の専用 crate（例 `ruajit-ffi` / `ruajit-jit`）に閉じ込め、core は no-unsafe を維持。
  - **(b) 限定方式**: FFI/JIT を当面対象外とし「LuaJIT 言語/ライブラリ互換インタプリタ」に限定（`unsafe` ゼロを維持）。ただし FFI 不在は LuaJIT の主要価値を欠く。
- → **後続 ADR で明文化すべき最初の論点。**

## 5. テスト資産

既存の公式 Lua スイート統合（`tests/lua-suite/` + `crates/rua-cli/tests/official_suite.rs`）と
**同じ枠組みを複製**するのが筋。

- 一次候補: **openresty/luajit2-test-suite**（Mike Pall のテストを基にした LuaJIT 2.1 用。`test/ffi/`・`test/misc/` 等）。
  ライセンスはテスト/ベンチが概ね public domain、一部 BSD/MIT。
- 補助: lua-Harness（Lua/LuaJIT 両対応スイート）、PUC Lua 5.1 公式スイート（言語互換部分の流用）。
- 取り込み方: `tests/luajit-suite/`（README + `fetch.sh` + `.gitignore`、本体は非コミット）と
  `crates/ruajit-cli/tests/luajit_suite.rs`（パス率分類・経時追跡）を、既存実装のミラーで作る。

## 6. crate 構成案

```
crates/
├── rua-core        # 既存。lexer/parser/AST は ruajit と共有
├── ruajit-cli      # 新規。`ruajit` バイナリ（rua-cli のミラー）
├── ruajit-core     # 新規。LuaJIT 独自の bytecode / VM（rua-core 内 feature でも可）
├── ruajit-ffi      # 将来。unsafe 許容境界（FFI）
└── ruajit-jit      # 将来。unsafe 許容境界（トレーシング JIT）
```

- フロントエンド（lexer/parser/AST）は `rua-core` を共有し、LuaJIT 拡張をモード/フィーチャで分岐。
- codegen/bytecode/VM は LuaJIT 独自系統なので分離。

## 7. 段階計画

- **Phase A（推奨先行・有界・`unsafe` ゼロ）**: LuaJIT 言語互換インタプリタ
  - parser 拡張（`goto`/`\x`/`\z`/`\u`/`load`）、`bit`、`jit.*` スタブ、`table.new/clear`、
    5.2/5.3 ライブラリ拡張、PRNG 系列一致。
  - 検証: luajit2-test-suite のうち FFI/JIT 非依存のテスト + 言語テスト。
- **Phase B**: `ffi` ライブラリ（`unsafe` 隔離 crate）。価値大・労力大。
- **Phase C**: 実トレーシング JIT。**Cranelift 等の Rust ネイティブ codegen が現実的**
  （手書き asm / DynASM 移植は非現実的）。研究級・長期。
- Resumable VM の完全互換は、現行 rua のスタックスライス方式との整合をどこかで判断する必要。

## 8. 未決事項（次の意思決定 = ADR 化する論点）

1. **`unsafe` 方針**: FFI/JIT を隔離 crate で許容するか、当面非対応か（§4）。
2. **ターゲット LuaJIT バージョン**: 2.1 を基準とするか。
3. **`ffi` を MVP に含めるか**: 含めないと「LuaJIT 互換」を名乗る価値が薄い、という指摘。
4. **resumable VM 互換**をどこまでやるか。
5. **テストスイートの選定**: luajit2-test-suite を正にするか。

## 9. スパイク所見（結論）

- **Phase A（言語/ライブラリ互換インタプリタ）は、現行 rua 資産（front-end 共有）を活かして
  `unsafe` ゼロで有界に達成可能**。まずここを MVP の足場とするのが妥当。
- ただし **LuaJIT の主要価値は FFI と JIT にあり、これらは `unsafe` 必須**。
  `ruajit` を「LuaJIT 互換」と名乗るなら、Phase B(FFI) までを早期スコープに含めるか、
  明確に「言語/ライブラリ互換インタプリタのみ」と位置づけるかを**先に決めるべき**。
- 推奨アクション: Phase A を足場に着手しつつ、論点1（`unsafe` 方針）と論点3（`ffi` スコープ）を
  ADR で先に確定する。

---

> 進捗・タスクの追跡は今後 GitHub Issues / Projects へ移行する方針（[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) 参照）。
> 本スパイクは「設計・調査」ドキュメントとしてリポジトリ（`docs/`）に残す。
