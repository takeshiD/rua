/*
 * lua.h — rua-capi: 本家 Lua 5.1 lua.h と ABI 互換の手書きヘッダ。
 *
 * 本家 PUC-Rio Lua 5.1 の lua.h とシグネチャ・定数・マクロを一致させ、
 * 既存の C/C++ 組み込みコードが無改変でリンクできることを目標とする
 * (ARCHITECTURE.md §7)。
 *
 * 実装済みの関数・定数のみを宣言する。未実装の関数（コルーチン等）は
 * コメントアウトして存在を明示する。
 *
 * ABI ノート:
 *   - lua_State は不透明型（内部は rua-capi::CapiState）。
 *   - lua_Number = double (LUAI_NUMTYPE)
 *   - lua_Integer = ptrdiff_t
 *   - lua_CFunction = int (*)(lua_State *)
 */

#ifndef LUA_H
#define LUA_H

#include <stddef.h>   /* size_t, ptrdiff_t */
#include <stdarg.h>   /* va_list */

#ifdef __cplusplus
extern "C" {
#endif

/* =========================================================================
 * バージョン情報
 * ========================================================================= */

#define LUA_VERSION      "Lua 5.1"
#define LUA_RELEASE      "Lua 5.1.5"
#define LUA_VERSION_NUM  501
#define LUA_COPYRIGHT    "Copyright (C) 1994-2012 Lua.org, PUC-Rio"
#define LUA_AUTHORS      "R. Ierusalimschy, L. H. de Figueiredo, W. Celes"

/* =========================================================================
 * 型定義
 * ========================================================================= */

/* 不透明な Lua 状態。実体は rua-capi::CapiState。 */
typedef struct lua_State lua_State;

/* 基本数値型。Lua 5.1 はすべての数値を double で扱う。 */
typedef double lua_Number;

/* 整数型（lua_tointeger / luaL_checkinteger 用）。 */
typedef ptrdiff_t lua_Integer;

/* C 関数ポインタ型。戻り値はスタックに積んだ結果数。 */
typedef int (*lua_CFunction)(lua_State *L);

/* lua_load 用リーダ関数型。 */
typedef const char *(*lua_Reader)(lua_State *L, void *ud, size_t *sz);

/* lua_dump 用ライタ関数型。 */
typedef int (*lua_Writer)(lua_State *L, const void *p, size_t sz, void *ud);

/* アロケータ関数型（rua では無視し既定アロケータを使う）。 */
typedef void *(*lua_Alloc)(void *ud, void *ptr, size_t osize, size_t nsize);

/* =========================================================================
 * 定数（本家 lua.h と値を一致させる）
 * ========================================================================= */

/* 型タグ */
#define LUA_TNONE          (-1)
#define LUA_TNIL            0
#define LUA_TBOOLEAN        1
#define LUA_TLIGHTUSERDATA  2
#define LUA_TNUMBER         3
#define LUA_TSTRING         4
#define LUA_TTABLE          5
#define LUA_TFUNCTION       6
#define LUA_TUSERDATA       7
#define LUA_TTHREAD         8

/* 複数戻り値 */
#define LUA_MULTRET         (-1)

/* 疑似インデックス */
#define LUA_REGISTRYINDEX   (-10000)
#define LUA_ENVIRONINDEX    (-10001)
#define LUA_GLOBALSINDEX    (-10002)

/* upvalue 疑似インデックス（C クロージャの upvalue を参照）。 */
#define lua_upvalueindex(i) (LUA_GLOBALSINDEX - (i))

/* スタック保証最小値 */
#define LUA_MINSTACK        20

/* 実行状態コード（本家 5.1 では LUA_OK は無いが便宜上定義する） */
#define LUA_OK              0
#define LUA_YIELD           1
#define LUA_ERRRUN          2
#define LUA_ERRSYNTAX       3
#define LUA_ERRMEM          4
#define LUA_ERRERR          5

/* GC 操作コード（lua_gc の第2引数） */
#define LUA_GCSTOP          0
#define LUA_GCRESTART       1
#define LUA_GCCOLLECT       2
#define LUA_GCCOUNT         3
#define LUA_GCCOUNTB        4
#define LUA_GCSTEP          5
#define LUA_GCSETPAUSE      6
#define LUA_GCSETSTEPMUL    7

/* =========================================================================
 * 状態生成 / 破棄
 * ========================================================================= */

/* lua_newstate: 新しい Lua 状態を作る（alloc/ud は無視）。 */
lua_State *lua_newstate(lua_Alloc f, void *ud);

/* lua_close: Lua 状態を破棄する。 */
void lua_close(lua_State *L);

/* lua_atpanic: パニックハンドラを設定し、旧ハンドラを返す。 */
lua_CFunction lua_atpanic(lua_State *L, lua_CFunction panicf);

/* =========================================================================
 * スタック操作
 * ========================================================================= */

int  lua_gettop(lua_State *L);
void lua_settop(lua_State *L, int idx);
void lua_pushvalue(lua_State *L, int idx);
void lua_remove(lua_State *L, int idx);
void lua_insert(lua_State *L, int idx);
void lua_replace(lua_State *L, int idx);
int  lua_checkstack(lua_State *L, int extra);

/* 便利マクロ（本家 lua.h と一致）。 */
#define lua_pop(L, n)       lua_settop(L, -(n) - 1)

/* =========================================================================
 * push 系
 * ========================================================================= */

void lua_pushnil(lua_State *L);
void lua_pushnumber(lua_State *L, lua_Number n);
void lua_pushinteger(lua_State *L, lua_Integer n);
void lua_pushlstring(lua_State *L, const char *s, size_t len);
void lua_pushstring(lua_State *L, const char *s);
void lua_pushcclosure(lua_State *L, lua_CFunction fn, int n);
void lua_pushboolean(lua_State *L, int b);
void lua_pushlightuserdata(lua_State *L, void *p);

/* lua_pushcfunction: upvalue なし C 関数を積む（lua_pushcclosure(L,f,0) のマクロ）。 */
#define lua_pushcfunction(L, f)  lua_pushcclosure(L, f, 0)

/* =========================================================================
 * 型取得 / 変換
 * ========================================================================= */

int         lua_type(lua_State *L, int idx);
const char *lua_typename(lua_State *L, int tp);

int          lua_isnumber(lua_State *L, int idx);
int          lua_isstring(lua_State *L, int idx);
int          lua_iscfunction(lua_State *L, int idx);
int          lua_isuserdata(lua_State *L, int idx);

/* lua_isfunction / lua_istable / lua_isnil / lua_isboolean / lua_isnone /
 * lua_isnoneornil は型タグ比較マクロで実装（本家と同じ）。 */
#define lua_isfunction(L, n)    (lua_type(L, n) == LUA_TFUNCTION)
#define lua_istable(L, n)       (lua_type(L, n) == LUA_TTABLE)
#define lua_islightuserdata(L, n) (lua_type(L, n) == LUA_TLIGHTUSERDATA)
#define lua_isnil(L, n)         (lua_type(L, n) == LUA_TNIL)
#define lua_isboolean(L, n)     (lua_type(L, n) == LUA_TBOOLEAN)
#define lua_isthread(L, n)      (lua_type(L, n) == LUA_TTHREAD)
#define lua_isnone(L, n)        (lua_type(L, n) == LUA_TNONE)
#define lua_isnoneornil(L, n)   (lua_type(L, n) <= 0)

int           lua_toboolean(lua_State *L, int idx);
lua_Number    lua_tonumber(lua_State *L, int idx);
lua_Integer   lua_tointeger(lua_State *L, int idx);
const char   *lua_tolstring(lua_State *L, int idx, size_t *len);
size_t        lua_objlen(lua_State *L, int idx);
lua_CFunction lua_tocfunction(lua_State *L, int idx);
void         *lua_touserdata(lua_State *L, int idx);

/* lua_tostring: lua_tolstring の len 省略版マクロ（本家と同じ）。 */
#define lua_tostring(L, i)  lua_tolstring(L, i, NULL)

int  lua_equal(lua_State *L, int idx1, int idx2);
int  lua_rawequal(lua_State *L, int idx1, int idx2);

/* =========================================================================
 * テーブル / グローバル
 * ========================================================================= */

void lua_createtable(lua_State *L, int narr, int nrec);
void lua_rawget(lua_State *L, int idx);
void lua_rawset(lua_State *L, int idx);
void lua_rawgeti(lua_State *L, int idx, int n);
void lua_rawseti(lua_State *L, int idx, int n);
void lua_gettable(lua_State *L, int idx);
void lua_settable(lua_State *L, int idx);
void lua_getfield(lua_State *L, int idx, const char *k);
void lua_setfield(lua_State *L, int idx, const char *k);
int  lua_getmetatable(lua_State *L, int idx);
int  lua_setmetatable(lua_State *L, int idx);
int  lua_next(lua_State *L, int idx);

/* グローバル変数アクセスのマクロ（本家 5.1 は getfield/setfield で実装）。 */
void lua_getglobal(lua_State *L, const char *name);
void lua_setglobal(lua_State *L, const char *name);

/* lua_newtable: createtable(L,0,0) のマクロ（本家と同じ）。 */
#define lua_newtable(L)  lua_createtable(L, 0, 0)

/* =========================================================================
 * 文字列操作
 * ========================================================================= */

/* lua_concat: スタックトップ n 個の値を連結して 1 つにする。 */
void lua_concat(lua_State *L, int n);

/* =========================================================================
 * 呼び出し / エラー
 * ========================================================================= */

void lua_call(lua_State *L, int nargs, int nresults);
int  lua_pcall(lua_State *L, int nargs, int nresults, int errfunc);
int  lua_cpcall(lua_State *L, lua_CFunction func, void *ud);
int  lua_error(lua_State *L);

/* =========================================================================
 * ロード
 * ========================================================================= */

/* lua_load: Reader ベースのロード（rua は未実装。luaL_loadbuffer を使うこと）。 */
/* int lua_load(lua_State *L, lua_Reader reader, void *dt, const char *chunkname); */

/* =========================================================================
 * GC
 * ========================================================================= */

int lua_gc(lua_State *L, int what, int data);

/* =========================================================================
 * 未実装の関数（コルーチン等）— 宣言のみ。リンクしても定義がないためリンクエラーになる。
 * 本家 5.1 との ABI 互換性確認用に記載する。
 * ========================================================================= */

/* コルーチン: 未実装 */
/* lua_State *lua_newthread(lua_State *L); */
/* int lua_resume(lua_State *L, int narg); */
/* int lua_yield(lua_State *L, int nresults); */
/* int lua_status(lua_State *L); */

/* デバッグ/フック: 未実装 */
/* int lua_getinfo(lua_State *L, const char *what, lua_Debug *ar); */
/* int lua_getstack(lua_State *L, int level, lua_Debug *ar); */

/* =========================================================================
 * 互換性マクロ（本家 lua.h と共通）
 * ========================================================================= */

/* lua_register: グローバルに C 関数を登録する。 */
#define lua_register(L, n, f) \
    (lua_pushcfunction(L, f), lua_setglobal(L, n))

/* スタックサイズ確認系 */
#define lua_getregistry(L)  lua_pushvalue(L, LUA_REGISTRYINDEX)
#define lua_getgccount(L)   lua_gc(L, LUA_GCCOUNT, 0)

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LUA_H */
