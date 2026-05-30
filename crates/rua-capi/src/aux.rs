//! 補助ライブラリ（本家 `lauxlib.c` の `luaL_*` 相当）。
//!
//! コア C API（[`crate`] 直下の `lua_*`）の上に構築する高水準ヘルパ。状態生成・ロード・
//! 型検査・参照（`luaL_ref`）・メタテーブル登録・ライブラリ登録などを提供する。

use core::ffi::{c_char, c_int};

use rua_core::gc::{GcHandle, TableKey};
use rua_core::value::Value as CoreValue;

use crate::{
    CapiState, LUA_NOREF, LUA_REFNIL, LUA_REGISTRYINDEX, LUA_TNIL, LUA_TNONE, lua_CFunction,
    lua_Integer, lua_Number, lua_State,
};

// ============================================================================
// 状態生成 / 標準ライブラリ
// ============================================================================

/// 既定アロケータで新しい状態を作る（本家 `luaL_newstate`）。
///
/// # Safety
/// 返ったポインタは [`crate::lua_close`] で解放すること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_newstate() -> *mut lua_State {
    unsafe { crate::lua_newstate(None, std::ptr::null_mut()) }
}

/// 全標準ライブラリを開く（本家 `luaL_openlibs`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_openlibs(s: *mut lua_State) {
    let cs = unsafe { CapiState::from_ptr(s) };
    rua_core::stdlib::open_libs(&mut cs.lua);
}

// ============================================================================
// ロード（本家 luaL_loadstring / loadbuffer / loadfile）
// ============================================================================

/// バイト列をチャンクとして読み込み、関数値をトップへ積む（本家 `luaL_loadbuffer`）。
///
/// # Safety
/// `buff` は `sz` バイト読める有効なポインタ。`name` は NULL か NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_loadbuffer(
    s: *mut lua_State,
    buff: *const c_char,
    sz: usize,
    name: *const c_char,
) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let src: &[u8] = if sz == 0 || buff.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(buff as *const u8, sz) }
    };
    let chunkname = unsafe { opt_cstr(name) }.unwrap_or_else(|| "?".to_string());
    crate::load_buffer(cs, src, &chunkname)
}

/// NUL 終端文字列をチャンクとして読み込む（本家 `luaL_loadstring`）。
///
/// チャンク名は文字列自身（本家同様 `[string "..."]` 形式へ整形される）。
///
/// # Safety
/// `sp` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_loadstring(s: *mut lua_State, sp: *const c_char) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    if sp.is_null() {
        return crate::load_buffer(cs, &[], "?");
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(sp) }.to_bytes().to_vec();
    let name = String::from_utf8_lossy(&bytes).into_owned();
    crate::load_buffer(cs, &bytes, &name)
}

/// ファイルからチャンクを読み込む（本家 `luaL_loadfile`）。チャンク名は `@filename`。
///
/// # Safety
/// `filename` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_loadfile(s: *mut lua_State, filename: *const c_char) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let Some(path) = (unsafe { opt_cstr(filename) }) else {
        let v = cs.lua.new_string(b"bad filename");
        cs.lua.stack.push(v);
        return crate::LUA_ERRRUN;
    };
    match std::fs::read(&path) {
        Ok(bytes) => crate::load_buffer(cs, &bytes, &format!("@{path}")),
        Err(e) => {
            let v = cs.lua.new_string(format!("cannot open {path}: {e}").as_bytes());
            cs.lua.stack.push(v);
            crate::LUA_ERRRUN
        }
    }
}

// ============================================================================
// 型検査 / 引数取得（本家 luaL_check* / luaL_opt*）
// ============================================================================

/// 指定インデックスが型 `t` であることを要求する（本家 `luaL_checktype`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checktype(s: *mut lua_State, idx: c_int, t: c_int) {
    let actual = unsafe { crate::lua_type(s, idx) };
    if actual != t {
        let cs = unsafe { CapiState::from_ptr(s) };
        let exp = type_name_of(t);
        let got = type_name_of(actual);
        raise_arg_error(cs, idx, &format!("{exp} expected, got {got}"));
    }
}

/// 値が存在することを要求する（本家 `luaL_checkany`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checkany(s: *mut lua_State, idx: c_int) {
    if unsafe { crate::lua_type(s, idx) } == LUA_TNONE {
        let cs = unsafe { CapiState::from_ptr(s) };
        raise_arg_error(cs, idx, "value expected");
    }
}

/// 数値引数を取得（本家 `luaL_checknumber`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checknumber(s: *mut lua_State, idx: c_int) -> lua_Number {
    if unsafe { crate::lua_isnumber(s, idx) } == 0 {
        let cs = unsafe { CapiState::from_ptr(s) };
        let got = type_name_of(unsafe { crate::lua_type(s, idx) });
        raise_arg_error(cs, idx, &format!("number expected, got {got}"));
    }
    unsafe { crate::lua_tonumber(s, idx) }
}

/// 整数引数を取得（本家 `luaL_checkinteger`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checkinteger(s: *mut lua_State, idx: c_int) -> lua_Integer {
    unsafe { luaL_checknumber(s, idx) as lua_Integer }
}

/// 省略可能な数値引数（本家 `luaL_optnumber`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_optnumber(s: *mut lua_State, idx: c_int, def: lua_Number) -> lua_Number {
    if unsafe { crate::lua_type(s, idx) } <= LUA_TNIL {
        def
    } else {
        unsafe { luaL_checknumber(s, idx) }
    }
}

/// 省略可能な整数引数（本家 `luaL_optinteger`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_optinteger(
    s: *mut lua_State,
    idx: c_int,
    def: lua_Integer,
) -> lua_Integer {
    if unsafe { crate::lua_type(s, idx) } <= LUA_TNIL {
        def
    } else {
        unsafe { luaL_checkinteger(s, idx) }
    }
}

/// 文字列引数を取得し長さを返す（本家 `luaL_checklstring`）。
///
/// # Safety
/// `len` は NULL か書き込み可能な `*mut usize`。返るポインタは [`crate::lua_tolstring`] と同じ安定性。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checklstring(
    s: *mut lua_State,
    idx: c_int,
    len: *mut usize,
) -> *const c_char {
    let p = unsafe { crate::lua_tolstring(s, idx, len) };
    if p.is_null() {
        let cs = unsafe { CapiState::from_ptr(s) };
        let got = type_name_of(unsafe { crate::lua_type(s, idx) });
        raise_arg_error(cs, idx, &format!("string expected, got {got}"));
    }
    p
}

/// 文字列引数を取得（本家 `luaL_checkstring`, 長さ不要版）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_checkstring(s: *mut lua_State, idx: c_int) -> *const c_char {
    unsafe { luaL_checklstring(s, idx, std::ptr::null_mut()) }
}

/// 省略可能な文字列引数（本家 `luaL_optlstring`）。
///
/// # Safety
/// `def` は NULL か NUL 終端 C 文字列。`len` は NULL か書込可能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_optlstring(
    s: *mut lua_State,
    idx: c_int,
    def: *const c_char,
    len: *mut usize,
) -> *const c_char {
    if unsafe { crate::lua_type(s, idx) } <= LUA_TNIL {
        if !len.is_null() && !def.is_null() {
            unsafe { *len = std::ffi::CStr::from_ptr(def).to_bytes().len() };
        } else if !len.is_null() {
            unsafe { *len = 0 };
        }
        def
    } else {
        unsafe { luaL_checklstring(s, idx, len) }
    }
}

// ============================================================================
// エラー（本家 luaL_error / argerror / where）
// ============================================================================

/// `where(level)` の位置プレフィックスを積む（本家 `luaL_where`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_where(s: *mut lua_State, level: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let w = rua_core::stdlib::aux::lua_where(&cs.lua, level.max(0) as u32);
    let v = cs.lua.new_string(w.as_bytes());
    cs.lua.stack.push(v);
}

/// エラーを送出する（本家 `luaL_error`）。
///
/// **制限**: printf 形式指定子は展開しない。`fmt` をそのままメッセージとして用い、
/// `luaL_where(1)` の位置を前置する（thread-local スロット経由で巻き戻す）。
///
/// # Safety
/// `fmt` は有効な NUL 終端 C 文字列。可変長引数は無視する（ABI 上は受理されるが未使用）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_error(s: *mut lua_State, fmt: *const c_char) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let msg = unsafe { opt_cstr(fmt) }.unwrap_or_default();
    let where_ = rua_core::stdlib::aux::lua_where(&cs.lua, 1);
    let full = format!("{where_}{msg}");
    crate::set_pending_error(rua_core::error::LuaError::Runtime(
        cs.lua.new_string(full.as_bytes()),
    ));
    0
}

/// 引数エラーを送出する（本家 `luaL_argerror`）。
///
/// # Safety
/// `extramsg` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_argerror(
    s: *mut lua_State,
    narg: c_int,
    extramsg: *const c_char,
) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let extra = unsafe { opt_cstr(extramsg) }.unwrap_or_default();
    raise_arg_error(cs, narg, &extra);
    0
}

// ============================================================================
// 参照（本家 luaL_ref / unref）
// ============================================================================

/// トップの値をテーブル `t` に格納し、整数参照を返す（本家 `luaL_ref`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_ref(s: *mut lua_State, t: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let v = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    if matches!(v, CoreValue::Nil) {
        return LUA_REFNIL;
    }
    let Some(tk) = table_key_at(cs, t) else {
        return LUA_NOREF;
    };
    // フリーリストの先頭は t[0]。
    let free = num_field(cs, tk, 0);
    let r = if free != 0 {
        let next = num_field(cs, tk, free);
        set_num_key(cs, tk, 0, next);
        free
    } else {
        let len = cs
            .lua
            .global
            .heap
            .get_table(tk)
            .map(|t| t.length())
            .unwrap_or(0);
        (len + 1) as i64
    };
    if let Some(tbl) = cs.lua.global.heap.get_table_mut(tk) {
        let _ = tbl.set(CoreValue::Number(r as f64), v);
    }
    r as c_int
}

/// 参照を解放する（本家 `luaL_unref`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_unref(s: *mut lua_State, t: c_int, r: c_int) {
    if r < 0 {
        return;
    }
    let cs = unsafe { CapiState::from_ptr(s) };
    let Some(tk) = table_key_at(cs, t) else {
        return;
    };
    let free_head = num_field(cs, tk, 0);
    // t[r] = t[0]（旧フリーヘッドを退避）、 t[0] = r。
    if let Some(tbl) = cs.lua.global.heap.get_table_mut(tk) {
        let _ = tbl.set(CoreValue::Number(r as f64), CoreValue::Number(free_head as f64));
        let _ = tbl.set(CoreValue::Number(0.0), CoreValue::Number(r as f64));
    }
}

// ============================================================================
// メタテーブル（本家 luaL_newmetatable）
// ============================================================================

/// レジストリにメタテーブル `tname` を作る/取得する（本家 `luaL_newmetatable`）。
///
/// 既に存在すれば 0 を返し（テーブルをトップへ積む）、新規作成なら 1 を返す。
///
/// # Safety
/// `tname` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_newmetatable(s: *mut lua_State, tname: *const c_char) -> c_int {
    // registry[tname] が既にテーブルなら積んで 0。
    unsafe { crate::lua_getfield(s, LUA_REGISTRYINDEX, tname) };
    if unsafe { crate::lua_type(s, -1) } == crate::LUA_TTABLE {
        return 0;
    }
    unsafe { crate::lua_settop(s, -2) }; // nil を捨てる
    unsafe { crate::lua_createtable(s, 0, 0) };
    unsafe { crate::lua_pushvalue(s, -1) }; // 複製を registry へ
    unsafe { crate::lua_setfield(s, LUA_REGISTRYINDEX, tname) };
    1
}

// ============================================================================
// ライブラリ登録（本家 luaL_register + luaL_Reg）
// ============================================================================

/// 関数登録テーブルのエントリ（本家 `luaL_Reg`）。配列の終端は `name == NULL`。
#[repr(C)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

/// 関数群をテーブル/グローバルに登録する（本家 `luaL_register`）。
///
/// `libname` が非 NULL なら同名のグローバルテーブルを作り（無ければ）対象にする。
/// NULL なら直前にスタックトップへ積まれているテーブルへ登録する。登録後、対象テーブルを
/// トップへ残す。
///
/// # Safety
/// `l` は `name == NULL` で終端する有効な [`luaL_Reg`] 配列。`libname` は NULL か C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaL_register(
    s: *mut lua_State,
    libname: *const c_char,
    l: *const luaL_Reg,
) {
    if !libname.is_null() {
        // グローバルに libname テーブルを用意してトップへ。
        unsafe { crate::lua_getfield(s, crate::LUA_GLOBALSINDEX, libname) };
        if unsafe { crate::lua_type(s, -1) } != crate::LUA_TTABLE {
            unsafe { crate::lua_settop(s, -2) };
            unsafe { crate::lua_createtable(s, 0, 0) };
            unsafe { crate::lua_pushvalue(s, -1) };
            unsafe { crate::lua_setfield(s, crate::LUA_GLOBALSINDEX, libname) };
        }
    }
    // 対象テーブルはトップ。その絶対インデックスを固定。
    let tidx = unsafe { crate::lua_gettop(s) };
    if l.is_null() {
        return;
    }
    let mut i = 0isize;
    loop {
        let entry = unsafe { &*l.offset(i) };
        if entry.name.is_null() {
            break;
        }
        unsafe { crate::lua_pushcclosure(s, entry.func, 0) };
        unsafe { crate::lua_setfield(s, tidx, entry.name) };
        i += 1;
    }
}

// ============================================================================
// 内部ヘルパ
// ============================================================================

/// `lua_type` の戻り値 → 型名。
fn type_name_of(t: c_int) -> &'static str {
    match t {
        crate::LUA_TNIL => "nil",
        crate::LUA_TBOOLEAN => "boolean",
        crate::LUA_TLIGHTUSERDATA | crate::LUA_TUSERDATA => "userdata",
        crate::LUA_TNUMBER => "number",
        crate::LUA_TSTRING => "string",
        crate::LUA_TTABLE => "table",
        crate::LUA_TFUNCTION => "function",
        crate::LUA_TTHREAD => "thread",
        _ => "no value",
    }
}

/// `bad argument #n to '?' (msg)` を thread-local エラースロットへ。
fn raise_arg_error(cs: &mut CapiState, narg: c_int, msg: &str) {
    let where_ = rua_core::stdlib::aux::lua_where(&cs.lua, 1);
    let full = format!("{where_}bad argument #{narg} ({msg})");
    crate::set_pending_error(rua_core::error::LuaError::Runtime(
        cs.lua.new_string(full.as_bytes()),
    ));
}

/// インデックス位置のテーブルキーを得る。
fn table_key_at(cs: &CapiState, idx: c_int) -> Option<TableKey> {
    match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Table(k)) => Some(k),
        _ => None,
    }
}

/// テーブルの整数キーの値を i64 として読む（無ければ 0）。
fn num_field(cs: &CapiState, tk: TableKey, key: i64) -> i64 {
    match cs
        .lua
        .global
        .heap
        .get_table(tk)
        .map(|t| t.get(&CoreValue::Number(key as f64)))
    {
        Some(CoreValue::Number(n)) => n as i64,
        _ => 0,
    }
}

/// テーブルの整数キーへ整数値を書く。
fn set_num_key(cs: &mut CapiState, tk: TableKey, key: i64, val: i64) {
    if let Some(t) = cs.lua.global.heap.get_table_mut(tk) {
        let _ = t.set(CoreValue::Number(key as f64), CoreValue::Number(val as f64));
    }
}

/// C 文字列を `Option<String>` へ（NULL → None）。
///
/// # Safety
/// `p` は NULL か有効な NUL 終端 C 文字列。
unsafe fn opt_cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(
            unsafe { std::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}