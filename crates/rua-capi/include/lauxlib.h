/*
 * lauxlib.h — rua-capi: 本家 Lua 5.1 lauxlib.h と ABI 互換の手書きヘッダ。
 *
 * luaL_* ヘルパ関数群（補助ライブラリ, 本家 lauxlib.c 相当）の宣言。
 * 実装済みの関数のみを宣言する。
 */

#ifndef LAUXLIB_H
#define LAUXLIB_H

#include <stddef.h>   /* size_t */
#include <stdio.h>    /* FILE */
#include "lua.h"

#ifdef __cplusplus
extern "C" {
#endif

/* =========================================================================
 * 定数（本家 lauxlib.h と値を一致させる）
 * ========================================================================= */

/* luaL_ref の特別な戻り値 */
#define LUA_NOREF   (-2)
#define LUA_REFNIL  (-1)

/* =========================================================================
 * 状態生成 / 標準ライブラリ
 * ========================================================================= */

/* luaL_newstate: 既定アロケータで新しい Lua 状態を作る。 */
lua_State *luaL_newstate(void);

/* luaL_openlibs: 全標準ライブラリを開く。 */
void luaL_openlibs(lua_State *L);

/* =========================================================================
 * ロード
 * ========================================================================= */

int luaL_loadbuffer(lua_State *L, const char *buff, size_t sz, const char *name);
int luaL_loadstring(lua_State *L, const char *s);
int luaL_loadfile(lua_State *L, const char *filename);

/* =========================================================================
 * 型検査 / 引数取得
 * ========================================================================= */

void         luaL_checktype(lua_State *L, int narg, int t);
void         luaL_checkany(lua_State *L, int narg);
lua_Number   luaL_checknumber(lua_State *L, int narg);
lua_Integer  luaL_checkinteger(lua_State *L, int narg);
lua_Number   luaL_optnumber(lua_State *L, int narg, lua_Number def);
lua_Integer  luaL_optinteger(lua_State *L, int narg, lua_Integer def);
const char  *luaL_checklstring(lua_State *L, int narg, size_t *l);
const char  *luaL_checkstring(lua_State *L, int narg);
const char  *luaL_optlstring(lua_State *L, int narg, const char *def, size_t *l);

/* luaL_checkstring / luaL_optstring のマクロ版（本家と同じ）。 */
#define luaL_optstring(L, n, d)  luaL_optlstring(L, n, d, NULL)

/* =========================================================================
 * エラー
 * ========================================================================= */

void luaL_where(lua_State *L, int lvl);
int  luaL_error(lua_State *L, const char *fmt, ...);
int  luaL_argerror(lua_State *L, int narg, const char *extramsg);

/* =========================================================================
 * 参照（レジストリ）
 * ========================================================================= */

int  luaL_ref(lua_State *L, int t);
void luaL_unref(lua_State *L, int t, int ref);

/* =========================================================================
 * メタテーブル
 * ========================================================================= */

int luaL_newmetatable(lua_State *L, const char *tname);

/* luaL_getmetatable: レジストリからメタテーブルを取得するマクロ。 */
#define luaL_getmetatable(L, n) lua_getfield(L, LUA_REGISTRYINDEX, n)

/* =========================================================================
 * ライブラリ登録
 * ========================================================================= */

/* 関数登録テーブルのエントリ（name == NULL で終端）。 */
typedef struct luaL_Reg {
    const char *name;
    lua_CFunction func;
} luaL_Reg;

void luaL_register(lua_State *L, const char *libname, const luaL_Reg *l);

/* =========================================================================
 * 便利マクロ（本家 lauxlib.h と互換）
 * ========================================================================= */

/* luaL_dostring: load + pcall を合わせた便利マクロ（本家と同じ）。
 * 成功は 0、失敗は非 0。 */
#define luaL_dostring(L, s) \
    (luaL_loadstring(L, s) || lua_pcall(L, 0, LUA_MULTRET, 0))

/* luaL_dofile: loadfile + pcall マクロ。 */
#define luaL_dofile(L, fn) \
    (luaL_loadfile(L, fn) || lua_pcall(L, 0, LUA_MULTRET, 0))

/* luaL_typename: インデックス位置の型名を返すマクロ。 */
#define luaL_typename(L, i) lua_typename(L, lua_type(L, i))

/* luaL_argcheck: 条件が偽のとき luaL_argerror を呼ぶ。 */
#define luaL_argcheck(L, cond, n, msg) \
    ((void)((cond) || luaL_argerror(L, n, msg)))

/* luaL_checkudata: ユーザーデータの型を確認する（rua は未実装）。 */
/* void *luaL_checkudata(lua_State *L, int ud, const char *tname); */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LAUXLIB_H */
