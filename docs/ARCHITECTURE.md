# rua アーキテクチャ設計書

`rua` は **Lua 5.1** の Rust 実装です。本書は全エージェント共通の設計の拠り所であり、
作業開始前に必ず参照してください。設計変更は team-lead との合意の上で本書を更新します。

## 1. 目標

1. **Lua 5.1 完全互換** — 言語仕様・標準ライブラリ・エラー文言まで本家 PUC-Rio Lua 5.1 に準拠。
2. **レジスタ型バイトコード VM** — 本家同様、ソースを register-machine 用バイトコードへコンパイルして実行。
3. **本家 `lua.h` ABI 互換の C API** — 既存の C/C++ 組み込みコードが無改変でリンクできる。
4. **ergonomic な Rust 組み込み API** — `mlua`/`rlua` 風の安全な高レベル API も併せて提供。

非目標（現時点）: LuaJIT 拡張、JIT コンパイル、Lua 5.2+ 機能。

### 現在の優先順位（ユーザー指示 2026-05-30）

**第一マイルストーン = `rua` コマンドでLuaスクリプトを実行できること**（組み込みより先）。
ユーザーが `rua run script.lua` で動作確認し、OKが出てから C API / Rust API（目標3・4）に着手する。

この方針により、当面 C 側へポインタを渡す要件が無くなるため、**GC方式は §5 案A（ハンドル/アリーナ）で確定**する（後述）。
網羅的なテスト用Luaスクリプトを用意し `cargo test` に統合することも第一マイルストーンに含む。

## 2. クレート構成（ワークスペース）

```
rua/                  # Cargo workspace
├── crates/
│   ├── rua-core/     # lib: lexer/parser/compiler/vm/gc/value/stdlib + Rust高レベルAPI
│   ├── rua-capi/     # cdylib + staticlib: 本家lua.h ABI互換のextern "C"層。cbindgenでヘッダ生成
│   └── rua-cli/      # bin: スタンドアロンの `lua` / `luac` 相当インタプリタ
├── tests/            # 横断的な互換性テスト・ゴールデン
├── docs/
└── fuzz/             # cargo-fuzz ターゲット
```

> 現状は単一パッケージ（`src/main.rs`）。フェーズ0でワークスペース化する。

## 3. 本家ソース → ruaモジュール 対応表

| 本家Lua 5.1 | rua モジュール | 担当エージェント |
|---|---|---|
| llex.c | `compiler::lexer` | lua-frontend |
| lparser.c | `compiler::parser` | lua-frontend |
| lcode.c | `compiler::codegen` | lua-frontend |
| lopcodes.h | `vm::opcode`（共有） | lua-frontend ↔ lua-vm |
| lvm.c | `vm::interp` | lua-vm |
| lobject.c | `value` | lua-vm |
| ltable.c | `value::table` | lua-vm |
| lstring.c | `value::string`（インターン） | lua-vm |
| lfunc.c | `value::closure` | lua-vm |
| lgc.c | `gc` | lua-runtime |
| lstate.c | `state` (`lua_State`) | lua-runtime |
| ldo.c | `state::call`（呼出・エラー・コルーチン） | lua-runtime |
| lmem.c | `gc::alloc` | lua-runtime |
| lapi.c | `capi` (`rua-capi`) | lua-capi |
| lauxlib.c | `capi::aux` | lua-capi |
| lbaselib.c 他 lib*.c | `stdlib::*` | lua-stdlib |

## 4. 値モデル（TValue 相当）

Lua 5.1 の値型: `nil`, `boolean`, `number`(double のみ・整数型なし), `string`, `table`,
`function`, `userdata`, `lightuserdata`, `thread`。

Rust 表現（暫定）:
```rust
enum Value {
    Nil,
    Boolean(bool),
    Number(f64),          // Lua 5.1 は全数値がdouble
    LightUserData(*mut c_void),
    GcRef(GcHandle),      // string/table/function/userdata/thread はGC管理
}
```
`#` 演算子の border 規則、`tostring`/`tonumber` 変換規則、メタテーブル解決順は本家準拠（lua-vm 管理）。

## 5. GC とポインタ安定性 — ✅ 案A（ハンドル/アリーナ）で確定

> **決定（2026-05-30）**: 第一マイルストーンでは C API を実装しないため、C 側へ渡すポインタの安定性制約が無い。
> よって **案A（ハンドル/アリーナ方式）を採用**する。安全で循環回収も容易、テストしやすい。
> 将来 C API を実装する際、`lua_tolstring`/`lua_newuserdata` のポインタ安定性は、
> 文字列インターンバッファと個別 box 化 userdata をスタック生存値でルート保持することで満たす（§4 で既述の方針を踏襲）。
> 当面 GC は単純な stop-the-world mark-and-sweep で可。インクリメンタル化は性能フェーズで検討。

Lua 5.1 のGCは本来 **インクリメンタル tri-color mark-and-sweep**（weak table・`__gc` finalizer・循環回収あり）。
Rust の所有権モデルは循環参照を素直に扱えないため、当初は2案を比較した（案Aを採用）:

- **案A: ハンドル/アリーナ方式（推奨）** — GCオブジェクトを型別アリーナ（slotmap）に格納し、`Value` は世代付きインデックスを持つ。mark-sweep はアリーナ走査で実装。`unsafe` をほぼ排除でき、循環も自然に回収。
  - 課題: C APIの ABI 互換。`lua_tolstring` の返す `const char*`、`lua_newuserdata` の返すポインタは GC 回収まで**安定**でなければならない。→ 文字列はインターンバッファ、userdata は個別 box 化し、スタック上の生存値をルートに保持することで満たす。
- **案B: 生ポインタ方式（unsafe・本家忠実）** — `GCObject` を生ポインタで連結し本家を踏襲。ABI 互換と性能で最有利だが `unsafe` が広範。

> 制約（両案共通）: C コードへ渡したポインタは、対応する値がLuaスタックやレジストリでGCルートに繋がっている限り安定でなければならない。lua-capi はこの不変条件を前提に設計する。

### 5.1 実装確定事項（フェーズ0, lua-runtime, 2026-05-30）

案A を `rua-core` に実装した。`lua-vm`/`lua-frontend`/`lua-stdlib` は以下のインタフェースに従う。

**ハンドル `GcHandle`（`gc` モジュール）** — `Copy` な enum。判別子が Lua 型タグを兼ね、
本体をデリファレンスせず型判定できる。`slotmap` の世代付きキーで解放済み参照を安全に検出。
```rust
pub enum GcHandle { Str(StringKey), Table(TableKey), Closure(ClosureKey), Userdata(UserdataKey) }
// TODO: コルーチン実装時に Thread(ThreadKey) を追加。
```

**値 `Value`（`value` モジュール）** — `#[derive(Clone, Copy)]`。VM スタック上を安価に運搬可能。
```rust
pub enum Value { Nil, Boolean(bool), Number(f64), LightUserData(*mut c_void), GcRef(GcHandle) }
```
`Value::type_of() -> LuaType` / `is_truthy()` / `as_gc()` を提供。`PartialEq` は raw 等価
（文字列はインターンによりハンドル一致 ⇔ 内容一致）。`__eq` 込みの等価判定は lua-vm 担当。

**ヒープ `Heap`（`gc` モジュール）** — 型別アリーナ + 文字列インターナを所有。`global_State` が 1 つ保持。
- 確保: `intern_str(&[u8]) -> GcHandle` / `alloc_table(Table)` / `alloc_closure(Closure)` / `alloc_userdata(Userdata)`
- 参照: `get_table(key)` / `get_table_mut(key)` 等（型不一致・解放済みは `None`）
- 回収: `collect(roots: impl IntoIterator<Item=GcHandle>)` で stop-the-world mark-and-sweep。

**トレース** — GC 子参照を持つ型は `Trace` を実装し、`Tracer::mark(handle)` / `mark_value(&Value)` で子を申告する。
新たな GC 内包型を追加する担当は `Trace` 実装を必ず用意すること（漏れると誤回収する）。

**ネイティブ関数シグネチャ（`state` モジュール）**
```rust
pub type NativeFn = fn(&mut LuaState) -> LuaResult<i32>;  // 戻り値 i32 = スタックに積んだ結果数
```

**状態（`state` モジュール）** — `GlobalState{ heap, registry, globals, gc_config }`、
`LuaState{ global, stack, call_info }`。`LuaState::roots()` がルート集合（レジストリ+グローバル+スタック）を返し、
`collect_garbage()` が GC を起動。**第一マイルストーンでは `LuaState` が `GlobalState` を直接所有**する
（C へ `lua_State*` を渡さないため安定ポインタ制約が無い）。第二マイルストーンで `Box`+共有 `global_State` へ再構成する。

**エラー巻き戻し（`state::call`）** — `pcall(state, body)` が保護境界。`Err` 伝播時にスタック/コールフレームを
呼び出し前の深さへ復元（本家 longjmp 相当）。命令ディスパッチは lua-vm が `body` 内に実装し委譲する。

## 6. エラー処理と制御フロー

本家は `setjmp/longjmp` でエラーを巻き戻す。rua では `lua_pcall` 境界を `Result` + 制御された
巻き戻しで表現する（パニックを FFI 境界に漏らさない）。コルーチンの yield/resume も含め lua-runtime が `state::call` で設計。

## 7. C API ABI 互換の判定基準

「本家 `lua.h` を include する C プログラムを `rua-capi`(staticlib/cdylib) にリンクして動かせるか」を正とする。
`extern "C"`・不透明な `lua_State`・本家と一致する定数/マクロ。ヘッダは cbindgen で生成しつつ本家ヘッダと差分検証（lua-capi ↔ lua-conformance）。

## 8. 互換性検証戦略

- フロントエンド: 本家 `luac -l` のダンプと比較。
- VM/stdlib: 同一スクリプトを本家 `lua5.1` と rua で実行し stdout/エラー/終了コードを比較。
- C ABI: 本家ヘッダを使う C テストをリンクして実行。
- 公式 Lua 5.1 テストスイートのパス率を追跡。ファジングでパニックを検出。

（lua-conformance が運用。期待値はハードコードせず本家実行から生成するのが原則。）

## 9. 開発フェーズ（依存関係）

### 第一マイルストーン: `rua` コマンドでLuaスクリプトを実行（最優先）

- **フェーズ0: 基盤** — ワークスペース化、値モデル+GC（§5 案Aで確定済み）、`lua_State` 骨格、命令セット定義。←全ての前提（直列点）
- **フェーズ1: フロントエンド** — lexer→parser→codegen で本家 `luac` 相当のバイトコードを生成。
- **フェーズ2: VMコア** — 命令ディスパッチ、テーブル、文字列、クロージャでスクリプトを実行。
- **フェーズ4: 標準ライブラリ（基本分）** — print/type/pairs/ipairs/pcall/tostring 等の base、string/table/math の主要関数。CLI実行に必要な範囲を先行。内部Rust APIで登録（C API非依存）。
- **CLI: `rua run script.lua`** — ファイル/標準入力からチャンクを読みコンパイル→実行。エラー表示・終了コードを本家 `lua5.1` に合わせる。
- **テスト: 網羅的なLuaスクリプト群** — `tests/lua/` に機能別スクリプトを用意し `cargo test` で本家 `lua5.1` とのゴールデン比較。

→ ここでユーザーが動作確認。**OK後に第二マイルストーンへ。**

### 第二マイルストーン: 組み込みAPI（ユーザー確認後）

- **フェーズ3: C API（lua.h ABI互換）+ Rust高レベルAPI**。
- **フェーズ5: 互換性の作り込み** — 公式テストスイート全体、C ABIリンクテスト、ファジング、性能（GCインクリメンタル化等）。

フェーズ1・2 はフェーズ0確定後に並行可能。フェーズ4・CLI は VM の最小実行が立ち上がり次第。
