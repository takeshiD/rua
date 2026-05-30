#!/usr/bin/env bash
# rua vs 本家 Lua 5.1 / LuaJIT のマイクロベンチ比較（lua-conformance 所有）。
#
# 各スクリプトを各処理系で繰り返し実行し、壁時計時間の中央値(秒)を表示する。
# 本家/LuaJIT が無ければ rua 単独で計測（比較列は "-"）。CI ではゲートにしない。
#
# 使い方:
#   tests/bench/run_bench.sh
#   RUA_LUA_BIN=$(command -v lua5.1) LUAJIT_BIN=$(command -v luajit) tests/bench/run_bench.sh
#   REPS=7 tests/bench/run_bench.sh    # 反復回数（既定 5）
set -eu
cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
REPS="${REPS:-5}"

# --- rua バイナリの決定（無ければ release ビルド） ---
RUA_BIN="${RUA_BIN:-$ROOT/target/release/rua}"
if [ ! -x "$RUA_BIN" ]; then
  echo "rua release バイナリが無いのでビルドします..." >&2
  ( cd "$ROOT" && cargo build --release -p rua-cli >/dev/null )
fi

# --- 比較対象の検出 ---
LUA_BIN="${RUA_LUA_BIN:-}"
if [ -z "$LUA_BIN" ]; then
  for n in lua5.1 lua-5.1 lua51 lua; do
    if command -v "$n" >/dev/null 2>&1; then LUA_BIN="$(command -v "$n")"; break; fi
  done
fi
LUAJIT="${LUAJIT_BIN:-}"
if [ -z "$LUAJIT" ] && command -v luajit >/dev/null 2>&1; then
  LUAJIT="$(command -v luajit)"
fi

# 中央値時間(秒)を返す。$1=実行コマンドのプレフィックス, $2=スクリプト
median_time() {
  local runner="$1" script="$2"
  local times=()
  for _ in $(seq 1 "$REPS"); do
    local start end
    start=$(date +%s.%N)
    # 出力は捨てる。失敗したら "x" を返す。
    if ! $runner "$script" >/dev/null 2>&1; then echo "x"; return; fi
    end=$(date +%s.%N)
    times+=("$(awk "BEGIN{print $end-$start}")")
  done
  printf '%s\n' "${times[@]}" | sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'
}

echo "=== rua ベンチ比較 (REPS=$REPS, 中央値秒) ==="
echo "rua    : $RUA_BIN"
echo "lua5.1 : ${LUA_BIN:-(なし)}"
echo "luajit : ${LUAJIT:-(なし)}"
printf '\n%-18s %10s %10s %10s\n' "script" "rua" "lua5.1" "luajit"
printf '%-18s %10s %10s %10s\n' "------" "---" "------" "------"

for script in scripts/*.lua; do
  name="$(basename "$script")"
  t_rua=$(median_time "$RUA_BIN run" "$script")
  t_lua="-"; t_jit="-"
  [ -n "$LUA_BIN" ] && t_lua=$(median_time "$LUA_BIN" "$script")
  [ -n "$LUAJIT" ] && t_jit=$(median_time "$LUAJIT" "$script")
  printf '%-18s %10s %10s %10s\n' "$name" "$t_rua" "$t_lua" "$t_jit"
done
