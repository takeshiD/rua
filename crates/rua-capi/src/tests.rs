//! Rust 内 FFI 往復テスト（本格的な C リンクテストは lua-conformance の P5 で実施）。
//!
//! `extern "C"` 関数を Rust から直接呼び、最小の C API ワークフローを検証する。

use super::*;
use crate::aux::luaL_newstate;
use core::ffi::c_int;

/// NUL 終端 C 文字列リテラルのヘルパ。
fn cstr(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

#[test]
fn newstate_loadstring_pcall_result() {
    unsafe {
        let l = luaL_newstate();
        assert!(!l.is_null());
        aux::luaL_openlibs(l);

        // 最小の往復: loadstring → pcall → 結果取得。
        let src = cstr("return 1 + 2");
        let status = aux::luaL_loadstring(l, src.as_ptr());
        assert_eq!(status, LUA_OK);

        let status = lua_pcall(l, 0, 1, 0);
        assert_eq!(status, LUA_OK, "pcall should succeed");

        assert_eq!(lua_type(l, -1), LUA_TNUMBER);
        assert_eq!(lua_tonumber(l, -1), 3.0);
        lua_settop(l, 0);
        assert_eq!(lua_gettop(l), 0);

        lua_close(l);
    }
}

#[test]
fn push_and_type_checks() {
    unsafe {
        let l = luaL_newstate();
        lua_pushnil(l);
        lua_pushboolean(l, 1);
        lua_pushnumber(l, 42.5);
        lua_pushinteger(l, 7);
        let hello = cstr("hello");
        lua_pushstring(l, hello.as_ptr());

        assert_eq!(lua_gettop(l), 5);
        assert_eq!(lua_type(l, 1), LUA_TNIL);
        assert_eq!(lua_type(l, 2), LUA_TBOOLEAN);
        assert_eq!(lua_type(l, 3), LUA_TNUMBER);
        assert_eq!(lua_type(l, 4), LUA_TNUMBER);
        assert_eq!(lua_type(l, 5), LUA_TSTRING);

        assert_eq!(lua_toboolean(l, 2), 1);
        assert_eq!(lua_tonumber(l, 3), 42.5);
        assert_eq!(lua_tointeger(l, 4), 7);

        // tolstring: NUL 終端ポインタ + 長さ。
        let mut len: usize = 0;
        let p = lua_tolstring(l, 5, &mut len);
        assert_eq!(len, 5);
        let bytes = std::slice::from_raw_parts(p as *const u8, len);
        assert_eq!(bytes, b"hello");
        // NUL 終端されている。
        assert_eq!(*p.add(len) as u8, 0);

        // 負のインデックス。
        assert_eq!(lua_type(l, -1), LUA_TSTRING);
        assert_eq!(lua_type(l, -3), LUA_TNUMBER);

        lua_close(l);
    }
}

#[test]
fn globals_and_fields() {
    unsafe {
        let l = luaL_newstate();
        // _G.x = 10 を C API で設定し、Lua から読む。
        lua_pushnumber(l, 10.0);
        let name = cstr("x");
        lua_setfield(l, LUA_GLOBALSINDEX, name.as_ptr());

        aux::luaL_openlibs(l);
        let src = cstr("return x * 2");
        assert_eq!(aux::luaL_loadstring(l, src.as_ptr()), LUA_OK);
        assert_eq!(lua_pcall(l, 0, 1, 0), LUA_OK);
        assert_eq!(lua_tonumber(l, -1), 20.0);

        // getfield でグローバルを読む。
        lua_settop(l, 0);
        lua_getfield(l, LUA_GLOBALSINDEX, name.as_ptr());
        assert_eq!(lua_tonumber(l, -1), 10.0);

        lua_close(l);
    }
}

#[test]
fn table_create_rawset_rawget() {
    unsafe {
        let l = luaL_newstate();
        lua_createtable(l, 0, 0);
        // t[1] = 100
        lua_pushnumber(l, 100.0);
        lua_rawseti(l, -2, 1);
        // t["k"] = true
        let k = cstr("k");
        lua_pushboolean(l, 1);
        lua_setfield(l, -2, k.as_ptr());

        assert_eq!(lua_objlen(l, -1), 1);
        lua_rawgeti(l, -1, 1);
        assert_eq!(lua_tonumber(l, -1), 100.0);
        lua_settop(l, -2);

        lua_getfield(l, -1, k.as_ptr());
        assert_eq!(lua_toboolean(l, -1), 1);

        lua_close(l);
    }
}

#[test]
fn runtime_error_pcall() {
    unsafe {
        let l = luaL_newstate();
        aux::luaL_openlibs(l);
        let src = cstr("error('boom')");
        assert_eq!(aux::luaL_loadstring(l, src.as_ptr()), LUA_OK);
        let status = lua_pcall(l, 0, 0, 0);
        assert_ne!(status, LUA_OK);
        // エラーオブジェクト（文字列）がトップ。
        assert_eq!(lua_type(l, -1), LUA_TSTRING);
        let mut len = 0;
        let p = lua_tolstring(l, -1, &mut len);
        let msg = std::slice::from_raw_parts(p as *const u8, len);
        assert!(msg.ends_with(b"boom"), "got: {:?}", String::from_utf8_lossy(msg));
        lua_close(l);
    }
}

#[test]
fn syntax_error_load() {
    unsafe {
        let l = luaL_newstate();
        let src = cstr("this is not ( lua");
        let status = aux::luaL_loadstring(l, src.as_ptr());
        assert_eq!(status, LUA_ERRSYNTAX);
        assert_eq!(lua_type(l, -1), LUA_TSTRING);
        lua_close(l);
    }
}

#[test]
fn c_function_direct_call() {
    // C → C 直接呼び出し（lua_pcall 経路）。Lua からの呼び出しは別途 VM フック待ち。
    unsafe extern "C" fn add(l: *mut lua_State) -> c_int {
        // Safety: `l` は lua_pcall から渡される有効なポインタ。
        unsafe {
            let a = lua_tonumber(l, 1);
            let b = lua_tonumber(l, 2);
            lua_pushnumber(l, a + b);
        }
        1
    }
    unsafe {
        let l = luaL_newstate();
        lua_pushcfunction(l, Some(add));
        assert_eq!(lua_type(l, -1), LUA_TFUNCTION);
        assert_eq!(lua_iscfunction(l, -1), 1);
        lua_pushnumber(l, 3.0);
        lua_pushnumber(l, 4.0);
        let status = lua_pcall(l, 2, 1, 0);
        assert_eq!(status, LUA_OK);
        assert_eq!(lua_tonumber(l, -1), 7.0);
        lua_close(l);
    }
}

#[test]
fn c_function_error_via_pending() {
    // lua_error を使う C 関数（`return lua_error(L)` 慣用形）。
    unsafe extern "C" fn fail(l: *mut lua_State) -> c_int {
        // Safety: `l` は lua_pcall から渡される有効なポインタ。
        unsafe {
            let msg = cstr("c side failure");
            lua_pushstring(l, msg.as_ptr());
            lua_error(l)
        }
    }
    unsafe {
        let l = luaL_newstate();
        lua_pushcfunction(l, Some(fail));
        let status = lua_pcall(l, 0, 0, 0);
        assert_eq!(status, LUA_ERRRUN);
        assert_eq!(lua_type(l, -1), LUA_TSTRING);
        let mut len = 0;
        let p = lua_tolstring(l, -1, &mut len);
        let msg = std::slice::from_raw_parts(p as *const u8, len);
        assert_eq!(msg, b"c side failure");
        lua_close(l);
    }
}

/// Lua スクリプトから push した C 関数を呼ぶ往復テスト（VM → c_trampoline → C 経路）。
///
/// `lua_pushcfunction` でグローバルに登録した C 関数を Lua の `my_add(3, 4)` で呼び出し、
/// 戻り値が正しく Lua 側へ伝わることを確認する。
#[test]
fn lua_calls_c_function_roundtrip() {
    // Lua から呼ばれる C 関数: 2 つの数値引数を足して返す。
    unsafe extern "C" fn my_add(l: *mut lua_State) -> c_int {
        // Safety: `l` は VM から渡された有効なポインタ。
        unsafe {
            let a = lua_tonumber(l, 1);
            let b = lua_tonumber(l, 2);
            lua_pushnumber(l, a + b);
        }
        1
    }

    unsafe {
        let l = luaL_newstate();
        aux::luaL_openlibs(l);

        // グローバル `my_add` に C 関数を登録する。
        lua_pushcfunction(l, Some(my_add));
        let name = cstr("my_add");
        lua_setfield(l, LUA_GLOBALSINDEX, name.as_ptr());

        // Lua スクリプトから C 関数を呼び出す。
        let src = cstr("return my_add(10, 32)");
        let status = aux::luaL_loadstring(l, src.as_ptr());
        assert_eq!(status, LUA_OK, "load should succeed");

        let status = lua_pcall(l, 0, 1, 0);
        assert_eq!(
            status,
            LUA_OK,
            "pcall should succeed; top type={}",
            lua_type(l, -1)
        );

        assert_eq!(lua_type(l, -1), LUA_TNUMBER);
        assert_eq!(lua_tonumber(l, -1), 42.0);

        lua_close(l);
    }
}

/// Lua から C 関数を呼び、その C 関数が lua_error でエラーを送出するテスト。
#[test]
fn lua_calls_c_function_error() {
    unsafe extern "C" fn bad_fn(l: *mut lua_State) -> c_int {
        // Safety: `l` は VM から渡された有効なポインタ。
        unsafe {
            let msg = cstr("error from C via Lua");
            lua_pushstring(l, msg.as_ptr());
            lua_error(l)
        }
    }

    unsafe {
        let l = luaL_newstate();
        aux::luaL_openlibs(l);

        lua_pushcfunction(l, Some(bad_fn));
        let name = cstr("bad_fn");
        lua_setfield(l, LUA_GLOBALSINDEX, name.as_ptr());

        let src = cstr("bad_fn()");
        let status = aux::luaL_loadstring(l, src.as_ptr());
        assert_eq!(status, LUA_OK);

        let status = lua_pcall(l, 0, 0, 0);
        assert_ne!(status, LUA_OK, "should fail");
        assert_eq!(lua_type(l, -1), LUA_TSTRING);

        let mut len = 0;
        let p = lua_tolstring(l, -1, &mut len);
        let msg = std::slice::from_raw_parts(p as *const u8, len);
        assert!(
            msg.windows(b"error from C via Lua".len()).any(|w| w == b"error from C via Lua"),
            "got: {:?}",
            String::from_utf8_lossy(msg)
        );

        lua_close(l);
    }
}

/// `lua_upvalueindex` を使って upvalue 付き C クロージャを登録し、
/// Lua から呼んで upvalue の値が読めることを確認するテスト。
#[test]
fn lua_calls_cclosure_with_upvalue() {
    // upvalue[1] に格納した数値を返す C 関数。
    unsafe extern "C" fn get_upvalue(l: *mut lua_State) -> c_int {
        // Safety: `l` は VM から渡された有効なポインタ。
        unsafe {
            let idx = lua_upvalueindex(1);
            let v = lua_tonumber(l, idx);
            lua_pushnumber(l, v);
        }
        1
    }

    unsafe {
        let l = luaL_newstate();
        aux::luaL_openlibs(l);

        // upvalue として 99.0 を積んでクロージャを作る。
        lua_pushnumber(l, 99.0);
        lua_pushcclosure(l, Some(get_upvalue), 1);
        let name = cstr("get_upvalue");
        lua_setfield(l, LUA_GLOBALSINDEX, name.as_ptr());

        let src = cstr("return get_upvalue()");
        let status = aux::luaL_loadstring(l, src.as_ptr());
        assert_eq!(status, LUA_OK);

        let status = lua_pcall(l, 0, 1, 0);
        assert_eq!(
            status,
            LUA_OK,
            "pcall should succeed; top type={}",
            lua_type(l, -1)
        );

        assert_eq!(lua_type(l, -1), LUA_TNUMBER);
        assert_eq!(lua_tonumber(l, -1), 99.0);

        lua_close(l);
    }
}

#[test]
fn ref_unref_registry() {
    unsafe {
        let l = luaL_newstate();
        let hello = cstr("anchored");
        lua_pushstring(l, hello.as_ptr());
        let r = aux::luaL_ref(l, LUA_REGISTRYINDEX);
        assert!(r >= 1);
        // 取り出して確認。
        lua_rawgeti(l, LUA_REGISTRYINDEX, r);
        assert_eq!(lua_type(l, -1), LUA_TSTRING);
        lua_settop(l, 0);
        aux::luaL_unref(l, LUA_REGISTRYINDEX, r);
        lua_close(l);
    }
}
