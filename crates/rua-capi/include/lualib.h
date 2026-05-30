/*
 * lualib.h — rua-capi: 本家 Lua 5.1 lualib.h と ABI 互換の手書きヘッダ。
 *
 * 標準ライブラリの開き関数（luaopen_* / luaL_openlibs）の宣言。
 * rua では luaL_openlibs で全ライブラリを一括開放する。
 * 個別の luaopen_* 関数は未エクスポートだが宣言のみ本家と互換させる。
 */

#ifndef LUALIB_H
#define LUALIB_H

#include "lua.h"

#ifdef __cplusplus
extern "C" {
#endif

/* =========================================================================
 * 標準ライブラリ名（テーブル名, 本家 lualib.h と同じ値）
 * ========================================================================= */

#define LUA_COLIBNAME    "coroutine"
#define LUA_TABLIBNAME   "table"
#define LUA_IOLIBNAME    "io"
#define LUA_OSLIBNAME    "os"
#define LUA_STRLIBNAME   "string"
#define LUA_MATHLIBNAME  "math"
#define LUA_DBLIBNAME    "debug"
#define LUA_LOADLIBNAME  "package"

/* =========================================================================
 * 全標準ライブラリ一括オープン（実装済み）
 * ========================================================================= */

/* luaL_openlibs は lauxlib.h でも宣言しているが、
 * lualib.h 単独でも include 可能にするため再宣言する（extern "C" 内）。 */
void luaL_openlibs(lua_State *L);

/* =========================================================================
 * 個別ライブラリオープン関数
 * （rua は luaL_openlibs 経由でのみ開く。個別呼び出しは未実装）
 * 本家 5.1 ヘッダとの互換性のために宣言は残す。
 * ========================================================================= */

/* NOTE: 以下の関数は rua-capi にはエクスポートされていない。
 * 個別に呼び出すと未定義シンボルエラーになる。luaL_openlibs を使うこと。 */

/* int luaopen_base(lua_State *L); */
/* int luaopen_table(lua_State *L); */
/* int luaopen_io(lua_State *L); */
/* int luaopen_os(lua_State *L); */
/* int luaopen_string(lua_State *L); */
/* int luaopen_math(lua_State *L); */
/* int luaopen_debug(lua_State *L); */
/* int luaopen_package(lua_State *L); */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LUALIB_H */
