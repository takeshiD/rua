# ベンチマーク（rua vs 本家 Lua / LuaJIT）

`rua` の実行速度を本家 PUC-Rio Lua 5.1（あれば LuaJIT も）と比較する軽量ベンチ雛形。
lua-conformance 所有（ARCHITECTURE.md §8 / フェーズ5・任意項目）。

## 構成

```
tests/bench/
├── README.md
├── run_bench.sh         比較ランナー（rua + 見つかった本家を計測）
└── scripts/             マイクロベンチ（決定的・副作用なしの計算中心）
    ├── fib.lua          再帰フィボナッチ（関数呼び出し/再帰）
    ├── nbody.lua        浮動小数演算ループ
    ├── string_build.lua 文字列連結・table.concat
    └── table_ops.lua    テーブル挿入/参照/ソート
```

## 実行

```bash
# rua はビルド済みを使う（無ければ自動で cargo build --release）。
tests/bench/run_bench.sh

# 比較対象の本家を明示する場合:
RUA_LUA_BIN=$(command -v lua5.1) LUAJIT_BIN=$(command -v luajit) tests/bench/run_bench.sh
```

各スクリプトを各処理系で複数回実行し、壁時計時間（`time`）の中央値を表示する。
本家/LuaJIT が無ければ rua 単独で計測する（比較列は `-`）。

## 注意
- 計測値は環境依存。CI ではゲートにせず、傾向把握・回帰検知の参考に使う。
- スクリプトは本家 5.1 と LuaJIT 双方で動く構文・API のみを使う（決定的な計算のみ）。
