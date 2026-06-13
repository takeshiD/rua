//! `rua-capi` — 本家 Lua 5.1 の `lua.h`/`lauxlib.h`/`lualib.h` と ABI 互換の C API 層。
//!
//! 既存の C/C++ 組み込みコードが**無改変でリンク**できることを目標とする（ARCHITECTURE.md §7）。
//! `extern "C"` + `#[no_mangle]` で本家と同名・同シグネチャの関数を公開し、`lua_State` は
//! 不透明ポインタ（内部の [`CapiState`] を Box 化したもの）として扱う。
//!
//! # 実装状況（第二マイルストーン v1）
//! - **実装済**: 状態生成/破棄、スタック操作、push/to/is、type、テーブル/グローバル（raw + field）、
//!   `lua_call`/`lua_pcall`、`luaL_loadstring`/`loadbuffer`、`luaL_openlibs`、`luaL_check*`/`opt*`、
//!   `luaL_ref`/`unref`、`luaL_newmetatable`、`luaL_register`、レジストリ。
//! - **既知の制限**:
//!   - C 関数（`lua_pushcfunction`/`pushcclosure`）は値として保持・`tocfunction`/`iscfunction` は動作するが、
//!     **Lua スクリプトからの呼び出し**は VM 側の upvalue 公開フック待ち（lua-vm へ依頼済み）。
//!     C → C の直接呼び出し（`lua_pcall`/`lua_call` で push した C 関数を呼ぶ）は動作する。
//!   - `lua_getfield`/`gettable` 等のメタメソッド（`__index`/`__newindex`）解決は未経由（raw）。
//!     VM の公開 index API 追加後に対応（lua-vm へ依頼済み）。
//!   - `luaL_error` の printf 形式指定子は未展開（メッセージは fmt をそのまま使用）。
//!   - コルーチン（`lua_newthread`/`resume`/`yield`）未対応。
//!
//! # 安全性
//! すべての公開関数は `unsafe extern "C"`。事前条件（有効な `lua_State*`・有効なインデックス・
//! 有効な C 文字列ポインタ等）は各関数のコメントに明記する。Rust パニックは FFI 境界を越えさせない
//! （内部は `Result` で扱い、`lua_error` 系は thread-local のエラースロットで巻き戻す）。

#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]
// 本家 lua.h の慣例では引数名を大文字 `L`（`lua_CFunction(lua_State* L)`）とする。
// ABI 互換のため関数ポインタ型定義で同名を使うため non_snake_case を許容する。
#![allow(non_snake_case)]

use core::ffi::{c_char, c_double, c_int, c_void};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rua_core::error::LuaError;
use rua_core::gc::{GcHandle, StringKey};
use rua_core::state::{CallInfo, LuaState};
use rua_core::value::Value as CoreValue;
use rua_core::value::closure::{Closure, LuaClosure, NativeClosure};
use rua_core::value::table::Table as CoreTable;

pub mod aux;

// ============================================================================
// 型エイリアス（本家 lua.h の typedef 相当）
// ============================================================================

/// 不透明な Lua 状態（本家 `lua_State`）。実体は [`CapiState`]。
#[repr(C)]
pub struct lua_State {
    _opaque: [u8; 0],
}

/// 本家 `lua_Number`（既定で `double`）。
pub type lua_Number = c_double;
/// 本家 `lua_Integer`（既定で `ptrdiff_t`）。
pub type lua_Integer = isize;

/// 本家 `lua_CFunction`。
///
/// NOTE: 戻り値はスタックに積んだ結果数。`lua_error`/`luaL_error` で送出したエラーは
/// thread-local スロット経由で C API 境界が捕捉する（longjmp の代替, モジュールコメント参照）。
pub type lua_CFunction = Option<unsafe extern "C" fn(L: *mut lua_State) -> c_int>;

/// 本家 `lua_Reader`（`lua_load` 用）。
pub type lua_Reader = Option<
    unsafe extern "C" fn(L: *mut lua_State, ud: *mut c_void, sz: *mut usize) -> *const c_char,
>;
/// 本家 `lua_Writer`（`lua_dump` 用）。
pub type lua_Writer = Option<
    unsafe extern "C" fn(L: *mut lua_State, p: *const c_void, sz: usize, ud: *mut c_void) -> c_int,
>;
/// 本家 `lua_Alloc`（メモリ確保フック）。rua では未使用（既定アロケータを使う）。
pub type lua_Alloc = Option<
    unsafe extern "C" fn(
        ud: *mut c_void,
        ptr: *mut c_void,
        osize: usize,
        nsize: usize,
    ) -> *mut c_void,
>;

// ============================================================================
// 定数（本家 lua.h / lauxlib.h）
// ============================================================================

pub const LUA_TNONE: c_int = -1;
pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;
pub const LUA_TLIGHTUSERDATA: c_int = 2;
pub const LUA_TNUMBER: c_int = 3;
pub const LUA_TSTRING: c_int = 4;
pub const LUA_TTABLE: c_int = 5;
pub const LUA_TFUNCTION: c_int = 6;
pub const LUA_TUSERDATA: c_int = 7;
pub const LUA_TTHREAD: c_int = 8;

/// すべての戻り値を受け取る（本家 `LUA_MULTRET`）。
pub const LUA_MULTRET: c_int = -1;

/// 疑似インデックス（本家）。
pub const LUA_REGISTRYINDEX: c_int = -10000;
pub const LUA_ENVIRONINDEX: c_int = -10001;
pub const LUA_GLOBALSINDEX: c_int = -10002;

/// upvalue 疑似インデックス（本家 `lua_upvalueindex(i)` = `LUA_GLOBALSINDEX - i`）。
/// C 関数が自身の upvalue を読むための疑似スタックインデックス（1-origin）。
///
/// # 注意
/// `#[allow(non_snake_case)]` は本家 lua.h のマクロ名との一致のため必要。
#[allow(non_snake_case)]
#[inline]
pub fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_GLOBALSINDEX - i
}

/// 最小スタック保証（本家 `LUA_MINSTACK`）。
pub const LUA_MINSTACK: c_int = 20;

/// 状態コード（本家）。5.1 は `LUA_OK` 定数を持たない（成功は 0）が、利便のため定義する。
pub const LUA_OK: c_int = 0;
pub const LUA_YIELD: c_int = 1;
pub const LUA_ERRRUN: c_int = 2;
pub const LUA_ERRSYNTAX: c_int = 3;
pub const LUA_ERRMEM: c_int = 4;
pub const LUA_ERRERR: c_int = 5;

/// `luaL_ref` の特別値（本家 lauxlib.h）。
pub const LUA_REFNIL: c_int = -1;
pub const LUA_NOREF: c_int = -2;

/// GC 操作コード（本家 `lua_gc` 用, 値だけ定義）。
pub const LUA_GCSTOP: c_int = 0;
pub const LUA_GCRESTART: c_int = 1;
pub const LUA_GCCOLLECT: c_int = 2;
pub const LUA_GCCOUNT: c_int = 3;
pub const LUA_GCCOUNTB: c_int = 4;
pub const LUA_GCSTEP: c_int = 5;
pub const LUA_GCSETPAUSE: c_int = 6;
pub const LUA_GCSETSTEPMUL: c_int = 7;

// ============================================================================
// 内部状態
// ============================================================================

/// C API が保持する状態。`lua_State*` はこの構造体への生ポインタ。
///
/// `#[repr(C)]` により `lua` フィールド（先頭）のオフセットが 0 であることを保証する。
/// これにより `c_trampoline` 内で `&mut LuaState` のポインタから `CapiState*` を逆引きできる。
/// （`*mut lua_State` ↔ `*mut CapiState` の相互キャストと同じ原理）
#[repr(C)]
pub struct CapiState {
    /// コアの実行状態。先頭フィールドであること必須（`c_trampoline` の container_of 逆引き）。
    lua: LuaState,
    /// パニックハンドラ（本家 `lua_atpanic`）。
    panic: lua_CFunction,
    /// `lua_tolstring` が返す NUL 終端バッファのキャッシュ（ポインタ安定性のため）。
    /// 文字列キーごとに 1 度だけ確保し、状態が生きている間ポインタを安定に保つ
    /// （本家の文字列ポインタ寿命要件, ARCHITECTURE.md §5）。
    cstr_cache: HashMap<StringKey, Box<[u8]>>,
    /// `lua_pushcclosure` で登録した C 関数（クロージャキー → C 関数 + upvalue）。
    c_functions: HashMap<rua_core::gc::ClosureKey, CFunc>,
}

/// 登録済み C 関数の情報。
struct CFunc {
    f: unsafe extern "C" fn(*mut lua_State) -> c_int,
    upvalues: Vec<CoreValue>,
}

thread_local! {
    /// `lua_error`/`luaL_error` が送出したエラーを保持する（longjmp の代替）。
    /// C 関数の直接呼び出し境界がこれを検査して `Err` へ変換する。
    static CAPI_ERROR: RefCell<Option<LuaError>> = const { RefCell::new(None) };
}

impl CapiState {
    /// `lua_State*` から内部状態への可変参照を得る。
    ///
    /// # Safety
    /// `s` は [`luaL_newstate`]/[`lua_newstate`] が返した有効なポインタで、未だ
    /// [`lua_close`] されていないこと。
    pub(crate) unsafe fn from_ptr<'a>(s: *mut lua_State) -> &'a mut CapiState {
        debug_assert!(!s.is_null());
        unsafe { &mut *(s as *mut CapiState) }
    }

    fn as_ptr(&mut self) -> *mut lua_State {
        self as *mut CapiState as *mut lua_State
    }

    /// 現在のフレームのスタックベース（C 関数フレームがあればその base, なければ 0）。
    fn base(&self) -> usize {
        self.lua.call_info.last().map(|c| c.base).unwrap_or(0)
    }

    /// スタック末尾へ値を積む。
    fn push(&mut self, v: CoreValue) {
        self.lua.stack.push(v);
    }

    /// インデックスを実スタックの絶対位置へ解決する（疑似インデックスは `None`）。
    fn abs_stack(&self, idx: c_int) -> Option<usize> {
        let base = self.base();
        let len = self.lua.stack.len();
        if idx > 0 {
            let i = base + (idx as usize) - 1;
            if i < len { Some(i) } else { None }
        } else if idx > LUA_REGISTRYINDEX {
            // 負の相対インデックス（-1 = 最上位）。
            let neg = (-idx) as usize;
            if neg >= 1 && neg <= len - base {
                Some(len - neg)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// インデックス位置の値を返す（無効なら `None`）。疑似インデックスにも対応。
    ///
    /// upvalue 疑似インデックス (`LUA_GLOBALSINDEX - i`, i >= 1) にも対応する。
    /// 対応する upvalue は現在実行中の C 関数クロージャの upvalue（`c_functions` に登録済みのもの）。
    fn value_at_opt(&self, idx: c_int) -> Option<CoreValue> {
        if idx > LUA_REGISTRYINDEX || idx > 0 {
            self.abs_stack(idx).map(|i| self.lua.stack[i])
        } else {
            match idx {
                LUA_REGISTRYINDEX => Some(CoreValue::GcRef(self.lua.global.registry)),
                LUA_GLOBALSINDEX => Some(CoreValue::GcRef(self.lua.global.globals)),
                // ENVIRONINDEX は当面グローバルで近似（C 関数の env 未実装）。
                LUA_ENVIRONINDEX => Some(CoreValue::GcRef(self.lua.global.globals)),
                // upvalue 疑似インデックス: LUA_GLOBALSINDEX - 1, -2, ... (本家 lua_upvalueindex)。
                // 対応する 1-origin インデックスは (LUA_GLOBALSINDEX - idx) 。
                _ if idx < LUA_GLOBALSINDEX => {
                    let upv_idx = (LUA_GLOBALSINDEX - idx) as usize; // 1-origin
                    // コールスタック末尾フレームの native_closure キーを取得。
                    let key = self.lua.current_native_closure()?;
                    let upv = self.c_functions.get(&key)?.upvalues.get(upv_idx - 1)?;
                    Some(*upv)
                }
                _ => None,
            }
        }
    }

    /// インデックス位置の値（無効なら `nil`）。
    fn value_at(&self, idx: c_int) -> CoreValue {
        self.value_at_opt(idx).unwrap_or(CoreValue::Nil)
    }

    /// 文字列キーに対応する NUL 終端バッファのポインタと長さを返す（安定ポインタ）。
    fn cstr_ptr(&mut self, key: StringKey) -> (*const c_char, usize) {
        let bytes = match self.lua.global.heap.get_str(key) {
            Some(s) => s.as_bytes().to_vec(),
            None => return (c"".as_ptr(), 0),
        };
        let len = bytes.len();
        let entry = self.cstr_cache.entry(key).or_insert_with(|| {
            let mut buf = bytes;
            buf.push(0); // NUL 終端（本家 C 文字列契約）
            buf.into_boxed_slice()
        });
        (entry.as_ptr() as *const c_char, len)
    }
}

// ============================================================================
// エラー処理ヘルパ
// ============================================================================

/// `LuaError` を本家のステータスコードへ写像する。
fn error_code(e: &LuaError) -> c_int {
    match e {
        LuaError::Runtime(_) | LuaError::Internal(_) => LUA_ERRRUN,
        LuaError::Syntax(_) => LUA_ERRSYNTAX,
        LuaError::Memory => LUA_ERRMEM,
        LuaError::ErrorInError => LUA_ERRERR,
        LuaError::Yield(_) => LUA_ERRRUN,
    }
}

/// `LuaError` のエラーオブジェクト（Lua 値）を得る。Rust 由来メッセージは Lua 文字列へ昇格。
fn error_value(cs: &mut CapiState, e: LuaError) -> CoreValue {
    match e {
        LuaError::Runtime(v) => v,
        LuaError::Syntax(s) | LuaError::Internal(s) => cs.lua.new_string(s.as_bytes()),
        LuaError::Memory => cs.lua.new_string(b"not enough memory"),
        LuaError::ErrorInError => cs.lua.new_string(b"error in error handling"),
        LuaError::Yield(_) => cs
            .lua
            .new_string(b"attempt to yield across a C-call boundary"),
    }
}

/// thread-local エラースロットへ保存（`lua_error`/`luaL_error` 用）。
pub(crate) fn set_pending_error(e: LuaError) {
    CAPI_ERROR.with(|slot| *slot.borrow_mut() = Some(e));
}

/// thread-local エラースロットを取り出す。
fn take_pending_error() -> Option<LuaError> {
    CAPI_ERROR.with(|slot| slot.borrow_mut().take())
}

// ============================================================================
// 状態の生成・破棄（本家 lstate / lapi）
// ============================================================================

/// 新しい Lua 状態を作る（本家 `lua_newstate`）。`alloc`/`ud` は無視し既定アロケータを使う。
///
/// # Safety
/// 返ったポインタは [`lua_close`] で解放すること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_newstate(_alloc: lua_Alloc, _ud: *mut c_void) -> *mut lua_State {
    let boxed = Box::new(CapiState {
        lua: LuaState::new(),
        panic: None,
        cstr_cache: HashMap::new(),
        c_functions: HashMap::new(),
    });
    Box::into_raw(boxed) as *mut lua_State
}

/// Lua 状態を破棄する（本家 `lua_close`）。
///
/// # Safety
/// `s` は本クレートが返した有効なポインタで、二重 close しないこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_close(s: *mut lua_State) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(s as *mut CapiState));
    }
}

/// パニックハンドラを設定し、旧ハンドラを返す（本家 `lua_atpanic`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_atpanic(s: *mut lua_State, panicf: lua_CFunction) -> lua_CFunction {
    let cs = unsafe { CapiState::from_ptr(s) };
    let old = cs.panic;
    cs.panic = panicf;
    old
}

// ============================================================================
// スタック操作（本家 lapi: lua_gettop/settop/pushvalue/remove/insert/replace）
// ============================================================================

/// スタックの要素数（現フレーム基準, 本家 `lua_gettop`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_gettop(s: *mut lua_State) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    (cs.lua.stack.len() - cs.base()) as c_int
}

/// スタックトップを設定する（本家 `lua_settop`）。負値は相対（`lua_pop` の実体）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_settop(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let base = cs.base();
    let len = cs.lua.stack.len();
    let newlen = if idx >= 0 {
        base + idx as usize
    } else {
        // idx 負: 新トップ = 現トップ + idx + 1（lua_pop(n) = settop(-n-1)）。
        (len as i64 + idx as i64 + 1).max(base as i64) as usize
    };
    if newlen > len {
        cs.lua.stack.resize(newlen, CoreValue::Nil);
    } else {
        cs.lua.stack.truncate(newlen);
    }
}

/// 指定インデックスの値をコピーしてトップへ積む（本家 `lua_pushvalue`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushvalue(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let v = cs.value_at(idx);
    cs.push(v);
}

/// 指定インデックスの要素を取り除き、上の要素を詰める（本家 `lua_remove`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_remove(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    if let Some(i) = cs.abs_stack(idx) {
        cs.lua.stack.remove(i);
    }
}

/// トップ要素を指定位置へ挿入し、以降を上へずらす（本家 `lua_insert`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_insert(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    if let Some(i) = cs.abs_stack(idx)
        && let Some(top) = cs.lua.stack.pop()
    {
        cs.lua.stack.insert(i, top);
    }
}

/// トップ要素を指定位置へ移動（上書き）し、トップを 1 つ減らす（本家 `lua_replace`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_replace(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    if let Some(top) = cs.lua.stack.pop()
        && let Some(i) = cs.abs_stack(idx)
    {
        cs.lua.stack[i] = top;
    }
}

/// スタックに `extra` 個の空きを確保できるか（本家 `lua_checkstack`）。rua は動的伸長なので常に成功。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_checkstack(_s: *mut lua_State, _extra: c_int) -> c_int {
    1
}

// ============================================================================
// push 系（本家 lapi）
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushnil(s: *mut lua_State) {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.push(CoreValue::Nil);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushboolean(s: *mut lua_State, b: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.push(CoreValue::Boolean(b != 0));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushnumber(s: *mut lua_State, n: lua_Number) {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.push(CoreValue::Number(n));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushinteger(s: *mut lua_State, n: lua_Integer) {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.push(CoreValue::Number(n as f64));
}

/// 長さ指定の文字列を積む（本家 `lua_pushlstring`）。`\0` を含みうる。
///
/// # Safety
/// `sp` は少なくとも `len` バイト読み取り可能なポインタであること（`len==0` なら NULL 可）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushlstring(s: *mut lua_State, sp: *const c_char, len: usize) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let bytes: &[u8] = if len == 0 || sp.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(sp as *const u8, len) }
    };
    let v = cs.lua.new_string(bytes);
    cs.push(v);
}

/// NUL 終端文字列を積む（本家 `lua_pushstring`）。`sp` が NULL なら nil を積む。
///
/// # Safety
/// `sp` は NULL か、NUL 終端された有効な C 文字列であること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushstring(s: *mut lua_State, sp: *const c_char) {
    let cs = unsafe { CapiState::from_ptr(s) };
    if sp.is_null() {
        cs.push(CoreValue::Nil);
        return;
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(sp) }.to_bytes();
    let v = cs.lua.new_string(bytes);
    cs.push(v);
}

/// C 関数を upvalue 付きで積む（本家 `lua_pushcclosure`）。
///
/// `n` 個の upvalue をスタックトップから取り、C 関数値を積む。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushcclosure(s: *mut lua_State, f: lua_CFunction, n: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let n = n.max(0) as usize;
    let len = cs.lua.stack.len();
    let upvalues: Vec<CoreValue> = if n > 0 && len >= n {
        cs.lua.stack.split_off(len - n)
    } else {
        Vec::new()
    };
    let key = match cs
        .lua
        .global
        .heap
        .alloc_closure(Closure::Native(NativeClosure::new(c_trampoline)))
    {
        GcHandle::Closure(k) => k,
        _ => unreachable!(),
    };
    if let Some(func) = f {
        cs.c_functions.insert(key, CFunc { f: func, upvalues });
    }
    cs.push(CoreValue::GcRef(GcHandle::Closure(key)));
}

/// C 関数を upvalue 無しで積む（本家マクロ `lua_pushcfunction` 相当）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushcfunction(s: *mut lua_State, f: lua_CFunction) {
    unsafe { lua_pushcclosure(s, f, 0) }
}

/// 軽量ユーザーデータ（生ポインタ）を積む（本家 `lua_pushlightuserdata`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pushlightuserdata(s: *mut lua_State, p: *mut c_void) {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.push(CoreValue::LightUserData(p));
}

/// Lua スクリプト（VM）から push 済みの C 関数を呼ぶトランポリン（VM → C 経路）。
///
/// `lua_pushcfunction`/`lua_pushcclosure` で登録した C 関数を Lua スクリプトから呼び出す際、
/// VM は `NativeClosure::func` としてこの関数を呼ぶ。トランポリンは以下の手順で C 関数を代理呼出しする:
///
/// 1. `state.current_native_closure()` で実行中クロージャの [`ClosureKey`] を取得する。
/// 2. `CapiState` へポインタを逆引きする（`LuaState` は `CapiState` の先頭フィールドかつ
///    `#[repr(C)]` で保証されるため、`*mut LuaState as *mut CapiState` が正当）。
/// 3. `c_functions` マップから C 関数ポインタを取得し、`lua_State*`（= `CapiState*`）を渡して呼ぶ。
/// 4. C 関数が `lua_error`/`luaL_error` で送出したエラーを thread-local スロットから回収し、`Err` へ変換する。
///
/// # Safety 事前条件
/// - `state` はヒープ上に確保された [`CapiState`] の先頭フィールドであること。
/// - [`CapiState`] は `#[repr(C)]` を付与してあり、`lua` フィールドのオフセットが 0 であること。
fn c_trampoline(state: &mut LuaState) -> Result<i32, LuaError> {
    // 実行中クロージャのキーを取得。VM の call_native が native_closure を設定済みのはず。
    let key = state
        .current_native_closure()
        .ok_or_else(|| LuaError::Internal("c_trampoline: no native_closure in CallInfo".into()))?;

    // LuaState ポインタから CapiState ポインタを逆引きする。
    // 安全性根拠: CapiState は #[repr(C)] で lua フィールドが先頭（オフセット 0）。
    // よって &lua と &CapiState は同一アドレスを指す。
    let cs: &mut CapiState = unsafe { &mut *(state as *mut LuaState as *mut CapiState) };

    // c_functions マップから C 関数ポインタを取得。
    let f =
        cs.c_functions.get(&key).map(|cf| cf.f).ok_or_else(|| {
            LuaError::Internal("c_trampoline: no CFunc registered for key".into())
        })?;

    // thread-local エラースロットをクリアしてから C 関数を呼ぶ。
    let _ = take_pending_error();
    let lua_state_ptr = cs.as_ptr();
    let nret = unsafe { f(lua_state_ptr) };

    // C 関数が lua_error/luaL_error でエラーを送出していれば Err へ変換する。
    if let Some(e) = take_pending_error() {
        return Err(e);
    }
    Ok(nret)
}

// ============================================================================
// 型・取得・変換（本家 lapi: lua_type/tonumber/toboolean/tolstring ...）
// ============================================================================

/// 値型を返す（本家 `lua_type`）。無効インデックスは `LUA_TNONE`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_type(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at_opt(idx) {
        None => LUA_TNONE,
        Some(v) => core_type_code(&v),
    }
}

fn core_type_code(v: &CoreValue) -> c_int {
    use rua_core::value::LuaType::*;
    match v.type_of() {
        Nil => LUA_TNIL,
        Boolean => LUA_TBOOLEAN,
        Number => LUA_TNUMBER,
        String => LUA_TSTRING,
        Table => LUA_TTABLE,
        Function => LUA_TFUNCTION,
        Userdata => LUA_TUSERDATA,
        Thread => LUA_TTHREAD,
        LightUserData => LUA_TLIGHTUSERDATA,
    }
}

/// 型名（本家 `lua_typename`）。`tp` は `lua_type` の戻り値。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_typename(_s: *mut lua_State, tp: c_int) -> *const c_char {
    let name: &'static [u8] = match tp {
        LUA_TNIL => b"nil\0",
        LUA_TBOOLEAN => b"boolean\0",
        LUA_TLIGHTUSERDATA => b"userdata\0",
        LUA_TNUMBER => b"number\0",
        LUA_TSTRING => b"string\0",
        LUA_TTABLE => b"table\0",
        LUA_TFUNCTION => b"function\0",
        LUA_TUSERDATA => b"userdata\0",
        LUA_TTHREAD => b"thread\0",
        _ => b"no value\0",
    };
    name.as_ptr() as *const c_char
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_isnumber(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::Number(_) => 1,
        CoreValue::GcRef(GcHandle::Str(k)) => {
            let ok = cs
                .lua
                .global
                .heap
                .get_str(k)
                .map(|s| rua_core::value::convert::str_to_number(s.as_bytes()).is_some())
                .unwrap_or(false);
            ok as c_int
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_isstring(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    matches!(
        cs.value_at(idx),
        CoreValue::GcRef(GcHandle::Str(_)) | CoreValue::Number(_)
    ) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_iscfunction(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Closure(k)) => cs.c_functions.contains_key(&k) as c_int,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_isuserdata(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    matches!(
        cs.value_at(idx),
        CoreValue::GcRef(GcHandle::Userdata(_)) | CoreValue::LightUserData(_)
    ) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_toboolean(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    cs.value_at(idx).is_truthy() as c_int
}

/// 数値へ変換（本家 `lua_tonumber`）。変換不能は 0.0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_tonumber(s: *mut lua_State, idx: c_int) -> lua_Number {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::Number(n) => n,
        CoreValue::GcRef(GcHandle::Str(k)) => cs
            .lua
            .global
            .heap
            .get_str(k)
            .and_then(|s| rua_core::value::convert::str_to_number(s.as_bytes()))
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 整数へ変換（本家 `lua_tointeger`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_tointeger(s: *mut lua_State, idx: c_int) -> lua_Integer {
    unsafe { lua_tonumber(s, idx) as lua_Integer }
}

/// 文字列バイト列へのポインタを返し、長さを `len` に書く（本家 `lua_tolstring`）。
///
/// 返るポインタは NUL 終端され、対応文字列がスタック/レジストリで生存する間安定（§5）。
/// 値が数値なら本家同様その場で文字列へ変換する。文字列でも数値でもなければ NULL を返す。
///
/// # Safety
/// `len` は NULL か、書き込み可能な `*mut usize` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_tolstring(
    s: *mut lua_State,
    idx: c_int,
    len: *mut usize,
) -> *const c_char {
    let cs = unsafe { CapiState::from_ptr(s) };
    // 数値はその場で文字列へ変換し、スタック上の値も差し替える（本家挙動）。
    let key = match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Str(k)) => k,
        CoreValue::Number(n) => {
            let strv = cs
                .lua
                .new_string(rua_core::value::convert::number_to_string(n).as_bytes());
            if let Some(i) = cs.abs_stack(idx) {
                cs.lua.stack[i] = strv;
            }
            match strv {
                CoreValue::GcRef(GcHandle::Str(k)) => k,
                _ => unreachable!(),
            }
        }
        _ => {
            if !len.is_null() {
                unsafe { *len = 0 };
            }
            return std::ptr::null();
        }
    };
    let (ptr, n) = cs.cstr_ptr(key);
    if !len.is_null() {
        unsafe { *len = n };
    }
    ptr
}

/// 文字列長／テーブル長／その他のサイズ（本家 `lua_objlen`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_objlen(s: *mut lua_State, idx: c_int) -> usize {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Str(k)) => {
            cs.lua.global.heap.get_str(k).map(|s| s.len()).unwrap_or(0)
        }
        CoreValue::GcRef(GcHandle::Table(k)) => cs
            .lua
            .global
            .heap
            .get_table(k)
            .map(|t| t.length())
            .unwrap_or(0),
        CoreValue::Number(n) => rua_core::value::convert::number_to_string(n).len(),
        _ => 0,
    }
}

/// C 関数ポインタを返す（C 関数でなければ NULL）（本家 `lua_tocfunction`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_tocfunction(s: *mut lua_State, idx: c_int) -> lua_CFunction {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Closure(k)) => cs.c_functions.get(&k).map(|c| c.f),
        _ => None,
    }
}

/// 軽量/フルユーザーデータのポインタ（本家 `lua_touserdata`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_touserdata(s: *mut lua_State, idx: c_int) -> *mut c_void {
    let cs = unsafe { CapiState::from_ptr(s) };
    match cs.value_at(idx) {
        CoreValue::LightUserData(p) => p,
        _ => std::ptr::null_mut(),
    }
}

/// 値の同一性比較（本家 `lua_equal`, ここでは raw 等価で近似。`__eq` は未経由）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_equal(s: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    (cs.value_at(idx1) == cs.value_at(idx2)) as c_int
}

/// raw 等価（本家 `lua_rawequal`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_rawequal(s: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    (cs.value_at(idx1) == cs.value_at(idx2)) as c_int
}

// ============================================================================
// テーブル / グローバル（本家 lapi）
// ============================================================================

/// 新しいテーブルを作りトップへ積む（本家 `lua_createtable`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_createtable(s: *mut lua_State, narr: c_int, nrec: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let t = CoreTable::with_capacity(narr.max(0) as usize, nrec.max(0) as usize);
    let h = cs.lua.global.heap.alloc_table(t);
    cs.push(CoreValue::GcRef(h));
}

/// テーブル `t[k]` を取得（raw, 本家 `lua_rawget`）。トップのキーを結果で置き換える。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_rawget(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let key = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    let v = table_raw_get(cs, cs.value_at(idx), key);
    cs.push(v);
}

/// `t[k] = v`（raw, 本家 `lua_rawset`）。トップ 2 つ（v, k）を消費する。
///
/// NOTE: テーブルのインデックス `idx` は pop **前**のスタック基準で解決する（本家の動作）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_rawset(s: *mut lua_State, idx: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    // pop 前にテーブルを解決してから v, k を取り出す。
    let t = cs.value_at(idx);
    let v = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    let k = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    table_raw_set(cs, t, k, v);
}

/// `t[n]` を取得（本家 `lua_rawgeti`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_rawgeti(s: *mut lua_State, idx: c_int, n: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let v = table_raw_get(cs, cs.value_at(idx), CoreValue::Number(n as f64));
    cs.push(v);
}

/// `t[n] = v`（本家 `lua_rawseti`）。トップの値を消費する。
///
/// NOTE: テーブルのインデックス `idx` は pop **前**のスタック基準で解決する（本家の動作）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_rawseti(s: *mut lua_State, idx: c_int, n: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    // pop 前にテーブルを解決してから値を取り出す（pop後にインデックスがずれるのを防ぐ）。
    let t = cs.value_at(idx);
    let v = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    table_raw_set(cs, t, CoreValue::Number(n as f64), v);
}

/// `t[k]`（本家 `lua_gettable`。メタメソッド未経由＝raw で近似）。トップのキーを結果で置換。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_gettable(s: *mut lua_State, idx: c_int) {
    unsafe { lua_rawget(s, idx) }
}

/// `t[k] = v`（本家 `lua_settable`。メタメソッド未経由）。トップ 2 つを消費。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_settable(s: *mut lua_State, idx: c_int) {
    unsafe { lua_rawset(s, idx) }
}

/// `t[k]`（`k` は C 文字列, 本家 `lua_getfield`）。結果をトップへ積む。
///
/// # Safety
/// `k` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_getfield(s: *mut lua_State, idx: c_int, k: *const c_char) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let t = cs.value_at(idx);
    let key = unsafe { cstr_to_value(cs, k) };
    let v = table_raw_get(cs, t, key);
    cs.push(v);
}

/// `t[k] = v`（`k` は C 文字列, 本家 `lua_setfield`）。トップの値を消費する。
///
/// NOTE: テーブルのインデックス `idx` は pop **前**のスタック基準で解決する（本家の動作）。
///
/// # Safety
/// `k` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_setfield(s: *mut lua_State, idx: c_int, k: *const c_char) {
    let cs = unsafe { CapiState::from_ptr(s) };
    // pop 前にテーブルを解決してから値を取り出す。
    let t = cs.value_at(idx);
    let v = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    let key = unsafe { cstr_to_value(cs, k) };
    table_raw_set(cs, t, key, v);
}

/// メタテーブルを取得（本家 `lua_getmetatable`）。あれば 1 を返しトップへ積む。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_getmetatable(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let mt = match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Table(k)) => {
            cs.lua.global.heap.get_table(k).and_then(|t| t.metatable())
        }
        CoreValue::GcRef(GcHandle::Userdata(k)) => cs
            .lua
            .global
            .heap
            .get_userdata(k)
            .and_then(|u| u.metatable()),
        CoreValue::GcRef(GcHandle::Str(_)) => cs.lua.global.string_metatable,
        _ => None,
    };
    match mt {
        Some(h) => {
            cs.push(CoreValue::GcRef(h));
            1
        }
        None => 0,
    }
}

/// メタテーブルを設定（本家 `lua_setmetatable`）。トップのテーブル（or nil）を消費する。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_setmetatable(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let mtv = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    let mt = match mtv {
        CoreValue::GcRef(h @ GcHandle::Table(_)) => Some(h),
        _ => None,
    };
    match cs.value_at(idx) {
        CoreValue::GcRef(GcHandle::Table(k)) => {
            if let Some(t) = cs.lua.global.heap.get_table_mut(k) {
                t.set_metatable(mt);
            }
        }
        CoreValue::GcRef(GcHandle::Userdata(k)) => {
            if let Some(u) = cs.lua.global.heap.get_userdata_mut(k) {
                u.set_metatable(mt);
            }
        }
        _ => {}
    }
    1
}

// ---- テーブル raw ヘルパ -----------------------------------------------

fn table_raw_get(cs: &CapiState, t: CoreValue, key: CoreValue) -> CoreValue {
    match t {
        CoreValue::GcRef(GcHandle::Table(k)) => cs
            .lua
            .global
            .heap
            .get_table(k)
            .map(|tbl| tbl.get(&key))
            .unwrap_or(CoreValue::Nil),
        _ => CoreValue::Nil,
    }
}

fn table_raw_set(cs: &mut CapiState, t: CoreValue, key: CoreValue, value: CoreValue) {
    if let CoreValue::GcRef(GcHandle::Table(k)) = t
        && let Some(tbl) = cs.lua.global.heap.get_table_mut(k)
    {
        let _ = tbl.set(key, value);
    }
}

/// C 文字列をインターン済み Lua 文字列値へ。
///
/// # Safety
/// `k` は NULL か NUL 終端 C 文字列。
unsafe fn cstr_to_value(cs: &mut CapiState, k: *const c_char) -> CoreValue {
    if k.is_null() {
        return CoreValue::Nil;
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(k) }.to_bytes();
    cs.lua.new_string(bytes)
}

// ============================================================================
// 呼び出し / エラー（本家 lapi / ldo）
// ============================================================================

/// 関数呼び出し（保護付き, 本家 `lua_pcall`）。
///
/// スタックは下から `func, arg1, .., argN` の並び。呼び出し後、それらを結果で置き換える。
/// `nresults == LUA_MULTRET` なら全戻り値、そうでなければ `nresults` 個に調整する。
/// 戻り値は 0（成功）または `LUA_ERR*`。失敗時はエラーオブジェクトをトップへ積む。
/// `errfunc` は当面未対応（0 を想定）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_pcall(
    s: *mut lua_State,
    nargs: c_int,
    nresults: c_int,
    _errfunc: c_int,
) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    match do_call(cs, nargs, nresults) {
        Ok(()) => LUA_OK,
        Err(e) => {
            let code = error_code(&e);
            let ev = error_value(cs, e);
            cs.push(ev);
            code
        }
    }
}

/// 関数呼び出し（非保護, 本家 `lua_call`）。エラー時はパニックハンドラを呼び `abort` する。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_call(s: *mut lua_State, nargs: c_int, nresults: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    if let Err(e) = do_call(cs, nargs, nresults) {
        let ev = error_value(cs, e);
        cs.push(ev);
        if let Some(panicf) = cs.panic {
            let p = cs.as_ptr();
            unsafe {
                panicf(p);
            }
        }
        // 本家の既定パニックは abort（longjmp 先が無い場合）。
        std::process::abort();
    }
}

/// 保護付きで C 関数を呼ぶ（本家 `lua_cpcall`）。`func(L)` を `ud` 引数 1 つで保護実行する。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_cpcall(
    s: *mut lua_State,
    func: lua_CFunction,
    ud: *mut c_void,
) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let Some(f) = func else { return LUA_OK };
    // ud を軽量ユーザーデータとして 1 引数で積み、直接 C 関数を保護呼び出しする。
    cs.push(CoreValue::LightUserData(ud));
    let base = cs.lua.stack.len() - 1;
    match call_c_function(cs, f, &[], base) {
        Ok(_) => {
            cs.lua.stack.truncate(base);
            LUA_OK
        }
        Err(e) => {
            cs.lua.stack.truncate(base);
            let code = error_code(&e);
            let ev = error_value(cs, e);
            cs.push(ev);
            code
        }
    }
}

/// トップの値をエラーオブジェクトとして送出する（本家 `lua_error`）。
///
/// longjmp の代替として thread-local スロットへ保存する。本関数は通常戻らない契約だが、
/// rua では 0 を返す（呼び出し側は `return lua_error(L)` の慣用形を取ることを想定）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_error(s: *mut lua_State) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    let v = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);
    set_pending_error(LuaError::Runtime(v));
    0
}

/// 内部: 呼び出し本体。スタックから func+args を取り、結果を積む。
fn do_call(cs: &mut CapiState, nargs: c_int, nresults: c_int) -> Result<(), LuaError> {
    let nargs = nargs.max(0) as usize;
    let len = cs.lua.stack.len();
    if len < nargs + 1 {
        return Err(LuaError::Runtime(
            cs.lua.new_string(b"not enough elements in the stack"),
        ));
    }
    let func_pos = len - nargs - 1;
    let func = cs.lua.stack[func_pos];
    let args: Vec<CoreValue> = cs.lua.stack[func_pos + 1..].to_vec();
    // func+args を取り除く（結果で置き換えるため）。
    cs.lua.stack.truncate(func_pos);

    let results = call_value(cs, func, &args)?;

    // 結果数を nresults に合わせる。
    let results = if nresults == LUA_MULTRET {
        results
    } else {
        let want = nresults.max(0) as usize;
        let mut r = results;
        r.resize(want, CoreValue::Nil);
        r
    };
    cs.lua.stack.extend(results);
    Ok(())
}

/// 値を呼び出す。登録 C 関数なら直接（C→C 経路）、それ以外は VM へ委譲（保護付き）。
fn call_value(
    cs: &mut CapiState,
    func: CoreValue,
    args: &[CoreValue],
) -> Result<Vec<CoreValue>, LuaError> {
    if let CoreValue::GcRef(GcHandle::Closure(k)) = func
        && let Some(cf) = cs.c_functions.get(&k)
    {
        let f = cf.f;
        let base = cs.lua.stack.len();
        return call_c_function(cs, f, args, base);
    }
    // Lua クロージャ / ネイティブ（stdlib）は VM の保護呼び出しへ。
    rua_core::state::call::pcall(&mut cs.lua, |st| rua_core::vm::call(st, func, args))
}

/// 登録 C 関数を直接呼ぶ（引数を base から積み、結果を回収）。
fn call_c_function(
    cs: &mut CapiState,
    f: unsafe extern "C" fn(*mut lua_State) -> c_int,
    args: &[CoreValue],
    base: usize,
) -> Result<Vec<CoreValue>, LuaError> {
    // 引数を base 以降へ配置し、C 関数フレームを積む。
    cs.lua.stack.truncate(base);
    cs.lua.stack.extend_from_slice(args);
    cs.lua.call_info.push(CallInfo {
        base,
        func: base,
        expected_results: 0,
        source: None,
        current_line: 0,
        native_closure: None,
        lua_frame: None,
        env: None,
    });
    let _ = take_pending_error(); // 念のためクリア
    let p = cs.as_ptr();
    let nret = unsafe { f(p) };
    cs.lua.call_info.pop();

    if let Some(e) = take_pending_error() {
        cs.lua.stack.truncate(base);
        return Err(e);
    }
    let nret = nret.max(0) as usize;
    let total = cs.lua.stack.len();
    let start = total.saturating_sub(nret);
    let results = cs.lua.stack[start..total].to_vec();
    cs.lua.stack.truncate(base);
    Ok(results)
}

// ============================================================================
// グローバル変数アクセス（本家 lua_getglobal / lua_setglobal）
// ============================================================================

/// グローバル変数を取得してトップへ積む（本家 `lua_getglobal`）。
///
/// `lua_getfield(L, LUA_GLOBALSINDEX, name)` 相当。
///
/// # Safety
/// `name` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_getglobal(s: *mut lua_State, name: *const c_char) {
    unsafe { lua_getfield(s, LUA_GLOBALSINDEX, name) }
}

/// グローバル変数を設定する（本家 `lua_setglobal`）。トップの値を消費する。
///
/// `lua_setfield(L, LUA_GLOBALSINDEX, name)` 相当。
///
/// # Safety
/// `name` は有効な NUL 終端 C 文字列。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_setglobal(s: *mut lua_State, name: *const c_char) {
    unsafe { lua_setfield(s, LUA_GLOBALSINDEX, name) }
}

// ============================================================================
// 文字列連結（本家 lua_concat）
// ============================================================================

/// スタックトップの `n` 個の値を文字列として連結し、結果をトップへ積む（本家 `lua_concat`）。
///
/// `n == 0` の場合は空文字列を積む。`n == 1` の場合はトップをそのまま残す。
/// 文字列/数値以外を含む場合は `__concat` メタメソッドを試み、なければエラー（panic）。
/// エラーは pcall 境界が無い場合 abort になる（本家 `lua_call` と同じ挙動）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_concat(s: *mut lua_State, n: c_int) {
    let cs = unsafe { CapiState::from_ptr(s) };
    let n = n.max(0) as usize;
    if n == 0 {
        // 空文字列を積む（本家挙動）。
        let v = cs.lua.new_string(b"");
        cs.push(v);
        return;
    }
    if n == 1 {
        // トップはそのまま（何もしない）。
        return;
    }
    let len = cs.lua.stack.len();
    if len < n {
        // スタックに足りない: abort（本家と同様、非保護呼び出し）。
        std::process::abort();
    }
    let start = len - n;
    let vals: Vec<CoreValue> = cs.lua.stack[start..].to_vec();
    cs.lua.stack.truncate(start);

    // 文字列/数値の連結: 全て stringable ならインプレースで結合。
    let mut result_bytes: Vec<u8> = Vec::new();
    let mut all_stringable = true;
    for v in &vals {
        match concat_to_bytes(cs, *v) {
            Some(b) => result_bytes.extend_from_slice(&b),
            None => {
                all_stringable = false;
                break;
            }
        }
    }

    if all_stringable {
        let v = cs.lua.new_string(&result_bytes);
        cs.push(v);
    } else {
        // __concat メタメソッド経由（保護付き pcall 使用）。
        // 失敗時は abort（非保護呼び出しと同等）。
        let result = rua_core::state::call::pcall(&mut cs.lua, |st| {
            // 左から右へ 2 値ずつ畳み込み。
            let mut acc = vals[0];
            for &next in &vals[1..] {
                acc = concat_via_vm(st, acc, next)?;
            }
            Ok(vec![acc])
        });
        match result {
            Ok(r) => {
                let v = r.into_iter().next().unwrap_or(CoreValue::Nil);
                cs.push(v);
            }
            Err(e) => {
                let ev = error_value(cs, e);
                cs.push(ev);
                if let Some(panicf) = cs.panic {
                    let p = cs.as_ptr();
                    unsafe { panicf(p) };
                }
                std::process::abort();
            }
        }
    }
}

/// 値を連結可能なバイト列へ変換する（`string` または `number`）。それ以外は `None`。
fn concat_to_bytes(cs: &CapiState, v: CoreValue) -> Option<Vec<u8>> {
    match v {
        CoreValue::Number(n) => Some(rua_core::value::convert::number_to_string(n).into_bytes()),
        CoreValue::GcRef(GcHandle::Str(k)) => {
            cs.lua.global.heap.get_str(k).map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    }
}

/// VM 経由で 2 値を連結する（文字列/数値のインプレース結合 + 型エラー）。
///
/// NOTE: `__concat` メタメソッドの呼び出しは vm::interp の内部関数に依存するため、
/// 現状は文字列/数値同士のみ対応。それ以外は型エラーを返す。
/// `__concat` 対応は lua-vm へ依頼済み（公開 API 追加後に拡張）。
fn concat_via_vm(
    state: &mut LuaState,
    a: CoreValue,
    b: CoreValue,
) -> rua_core::error::LuaResult<CoreValue> {
    let a_bytes = match a {
        CoreValue::Number(n) => Some(rua_core::value::convert::number_to_string(n).into_bytes()),
        CoreValue::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    };
    let b_bytes = match b {
        CoreValue::Number(n) => Some(rua_core::value::convert::number_to_string(n).into_bytes()),
        CoreValue::GcRef(GcHandle::Str(k)) => {
            state.global.heap.get_str(k).map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    };
    if let (Some(mut ab), Some(bb)) = (a_bytes, b_bytes) {
        ab.extend_from_slice(&bb);
        return Ok(CoreValue::GcRef(state.global.heap.intern_str(&ab)));
    }
    // 型エラー（`__concat` メタメソッド対応は TODO）。
    let culprit = match (a, b) {
        (CoreValue::GcRef(GcHandle::Str(_)), _) | (CoreValue::Number(_), _) => b,
        _ => a,
    };
    Err(rua_core::error::LuaError::Runtime(
        state.new_string(
            format!(
                "attempt to concatenate a {} value",
                culprit.type_of().name()
            )
            .as_bytes(),
        ),
    ))
}

// ============================================================================
// GC 操作（本家 lua_gc）
// ============================================================================

/// GC 操作を行う（本家 `lua_gc`）。
///
/// `LUA_GCCOLLECT` は実際に GC を起動する。その他の操作は未実装（0 を返す）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_gc(s: *mut lua_State, what: c_int, _data: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    match what {
        LUA_GCCOLLECT => {
            cs.lua.collect_garbage();
            0
        }
        LUA_GCSTOP | LUA_GCRESTART => 0,
        LUA_GCCOUNT => {
            // 概算: ヒープの GC オブジェクト数（バイト数を返すのが本来だが近似で可）。
            0
        }
        _ => 0,
    }
}

// ============================================================================
// テーブル反復（本家 lua_next）
// ============================================================================

/// テーブルの次のキー/値ペアを取得する（本家 `lua_next`）。
///
/// スタックトップのキーの「次」のキー/値をそれぞれ積んで 1 を返す（本家 `lua_next`）。
/// テーブルに次のエントリが無ければ 0 を返し、スタックはキーを消費した状態のまま。
///
/// NOTE: rua は現状 `next` 反復の順序保証を本家と完全一致させていない（ハッシュ部順序は
/// 実装依存）。将来の互換性強化は lua-vm へ依頼済み。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lua_next(s: *mut lua_State, idx: c_int) -> c_int {
    let cs = unsafe { CapiState::from_ptr(s) };
    // テーブルは pop 前に解決する（pop 後にインデックスがずれる可能性があるため）。
    let tbl_val = cs.value_at(idx);
    let key = cs.lua.stack.pop().unwrap_or(CoreValue::Nil);

    let CoreValue::GcRef(GcHandle::Table(tk)) = tbl_val else {
        return 0;
    };

    // Table::next を使って key の「次」のエントリを取得する。
    let result = cs.lua.global.heap.get_table(tk).map(|t| t.next(&key));

    match result {
        Some(Ok(Some((k, v)))) => {
            cs.push(k);
            cs.push(v);
            1
        }
        Some(Ok(None)) => {
            // テーブルの末尾（次のエントリなし）。
            0
        }
        Some(Err(())) | None => {
            // キー不在 or テーブル無効。本家では無効キーはエラーだが、ここでは 0 で安全に終了。
            0
        }
    }
}

// ============================================================================
// ロード（本家 lua_load の最小版。luaL_loadstring 等は aux.rs）
// ============================================================================

/// 内部: ソースをコンパイルし、関数値をトップへ積む。成功で `LUA_OK`、失敗でエラーコード。
/// 失敗時はエラーメッセージ文字列をトップへ積む（本家 `lua_load` の契約）。
pub(crate) fn load_buffer(cs: &mut CapiState, src: &[u8], chunkname: &str) -> c_int {
    match rua_core::compiler::compile(&mut cs.lua.global.heap, src, chunkname) {
        Ok(proto) => {
            let env = cs.lua.global.globals;
            let h = cs
                .lua
                .global
                .heap
                .alloc_closure(Closure::Lua(LuaClosure::new_with_env(Rc::new(proto), env)));
            cs.push(CoreValue::GcRef(h));
            LUA_OK
        }
        Err(e) => {
            let code = error_code(&e);
            let ev = error_value(cs, e);
            cs.push(ev);
            code
        }
    }
}

// ============================================================================
// テスト（Rust 内 FFI 往復）
// ============================================================================

#[cfg(test)]
mod tests;
