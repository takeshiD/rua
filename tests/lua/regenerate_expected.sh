#!/usr/bin/env bash
# tests/lua/*.lua の期待値(.expected)を、リファレンス本家 Lua 5.1 から再生成する。
#
# 設計原則（docs/ARCHITECTURE.md §8）: 期待値はハードコードせず本家実行から生成するのが原則。
# 本リポジトリには手作業で本家 5.1 準拠に作成した .expected を同梱しているが、
# 本家 lua5.1 が手元にある場合は本スクリプトで正規の出力に上書きできる。
#
# 使い方:
#   tests/lua/regenerate_expected.sh            # PATH の lua5.1 を使用
#   RUA_LUA_BIN=/path/to/lua5.1 tests/lua/regenerate_expected.sh
#
# 終了コードが 0 以外になるスクリプト（例: 14_uncaught_error.lua）は
# stdout のみを .expected に保存し、終了コードを .exitcode に保存する。

set -u
cd "$(dirname "$0")"

# リファレンス Lua を解決
LUA="${RUA_LUA_BIN:-}"
if [ -z "$LUA" ]; then
    for cand in lua5.1 lua-5.1 lua51 lua; do
        if command -v "$cand" >/dev/null 2>&1; then
            if "$cand" -v 2>&1 | grep -q "Lua 5.1"; then
                LUA="$cand"
                break
            fi
        fi
    done
fi

if [ -z "$LUA" ]; then
    echo "error: リファレンス Lua 5.1 が見つかりません。" >&2
    echo "       docs/CONFORMANCE.md を参照して導入するか RUA_LUA_BIN を設定してください。" >&2
    exit 1
fi

echo "using reference: $LUA ($("$LUA" -v 2>&1 | head -n1))"

for script in *.lua; do
    base="${script%.lua}"
    out="$("$LUA" "$script" 2>/dev/null)"
    code=$?
    printf '%s' "$out" > "$base.expected"
    # 末尾改行を保つ（print は各行に \n を付与する）
    if [ -n "$out" ]; then printf '\n' >> "$base.expected"; fi
    if [ "$code" -ne 0 ]; then
        printf '%s\n' "$code" > "$base.exitcode"
        echo "  $script -> .expected (+ .exitcode=$code)"
    else
        rm -f "$base.exitcode"
        echo "  $script -> .expected"
    fi
done

echo "done."
