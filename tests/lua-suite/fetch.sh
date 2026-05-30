#!/usr/bin/env bash
# 公式 Lua 5.1 テストスイート（PUC-Rio）を取得・展開する。
# テスト本体はリポジトリにコミットしない（.gitignore 済み）。
#
# 使い方: tests/lua-suite/fetch.sh
set -eu
cd "$(dirname "$0")"

URL="https://www.lua.org/tests/lua5.1-tests.tar.gz"
TARBALL="lua5.1-tests.tar.gz"

echo "fetching $URL ..."
if command -v curl >/dev/null 2>&1; then
    curl -R -L -o "$TARBALL" "$URL"
elif command -v wget >/dev/null 2>&1; then
    wget -O "$TARBALL" "$URL"
else
    echo "error: curl も wget も見つかりません。" >&2
    exit 1
fi

echo "extracting ..."
tar zxf "$TARBALL"
rm -f "$TARBALL"

echo "done. 展開されたディレクトリ:"
ls -d */ 2>/dev/null || true
echo
echo "実行: cargo test -p rua-cli --test official_suite -- --ignored --nocapture"
