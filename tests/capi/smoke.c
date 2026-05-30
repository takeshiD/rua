/*
 * C ABI 互換スモークテスト（lua-conformance 所有, ARCHITECTURE.md §7）。
 *
 * 本家 PUC-Rio Lua 5.1 の `lua.h` / `lauxlib.h` / `lualib.h` を include し、
 * rua-capi（staticlib/cdylib）にリンクして動かす。これがビルド・実行・期待出力一致
 * できれば「本家ヘッダを使う C プログラムが無改変でリンクできる」= ABI 互換を満たす。
 *
 * ビルド/実行は crates/rua-cli/tests/capi_abi.rs が駆動する（rua-capi 完成後に結線）。
 * rua-capi が未提供の段階では capi_abi.rs が自動スキップするため本ファイルは未使用。
 *
 * 期待標準出力（厳密一致でゴールデン比較する）:
 *   2 + 3 = 5
 *   concat = rua-capi
 *   CAPI_OK
 */
#include <stdio.h>

#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"

int main(void) {
    lua_State *L = luaL_newstate();
    if (L == NULL) {
        fprintf(stderr, "luaL_newstate failed\n");
        return 2;
    }
    luaL_openlibs(L);

    /* 1) スタック操作 + 算術: 2 + 3 を Lua 数値として計算して取り出す。 */
    lua_pushnumber(L, 2);
    lua_pushnumber(L, 3);
    double a = lua_tonumber(L, -2);
    double b = lua_tonumber(L, -1);
    lua_pop(L, 2);
    printf("2 + 3 = %d\n", (int)(a + b));

    /* 2) 文字列の push と連結（C 文字列 ABI）。 */
    lua_pushstring(L, "rua");
    lua_pushstring(L, "-capi");
    lua_concat(L, 2);
    printf("concat = %s\n", lua_tostring(L, -1));
    lua_pop(L, 1);

    /* 3) ソース文字列をロード→実行して大域へ反映（luaL_dostring 相当）。 */
    if (luaL_dostring(L, "result = 6 * 7")) {
        fprintf(stderr, "dostring error: %s\n", lua_tostring(L, -1));
        lua_close(L);
        return 3;
    }
    lua_getglobal(L, "result");
    if ((int)lua_tonumber(L, -1) != 42) {
        fprintf(stderr, "unexpected result: %d\n", (int)lua_tonumber(L, -1));
        lua_close(L);
        return 4;
    }
    lua_pop(L, 1);

    printf("CAPI_OK\n");
    lua_close(L);
    return 0;
}
