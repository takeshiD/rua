//! ユーザーデータ（本家 `Udata` 相当）。
//!
//! Lua の full userdata は GC 管理されるメモリブロックで、メタテーブルと環境（env）を持つ。
//! rua（Rust 側）では任意の Rust 値を `Box<dyn Any>` として保持する。
//!
//! # 将来の C API（ARCHITECTURE.md §5）
//! `lua_newuserdata` が返す生ポインタの安定性は、本体を個別 box 化し、スタック生存値で
//! ルート保持することで満たす。第一マイルストーンでは C へポインタを渡さないため Rust 値で保持する。
//!
//! TODO(lua-vm/lua-capi): C 互換の生バイト userdata 表現、`__gc` finalizer の起動。

use std::any::Any;

use crate::gc::{GcHandle, Trace, Tracer};

/// full userdata。
pub struct Userdata {
    /// 保持する Rust 値（型消去）。C 互換 userdata は将来別表現を追加する。
    data: Box<dyn Any>,
    /// メタテーブル。
    metatable: Option<GcHandle>,
    /// 環境テーブル（本家 userdata の env）。
    env: Option<GcHandle>,
}

impl Userdata {
    pub fn new(data: Box<dyn Any>) -> Self {
        Userdata {
            data,
            metatable: None,
            env: None,
        }
    }

    pub fn data(&self) -> &dyn Any {
        self.data.as_ref()
    }

    pub fn data_mut(&mut self) -> &mut dyn Any {
        self.data.as_mut()
    }

    pub fn metatable(&self) -> Option<GcHandle> {
        self.metatable
    }

    pub fn set_metatable(&mut self, mt: Option<GcHandle>) {
        self.metatable = mt;
    }

    pub fn env(&self) -> Option<GcHandle> {
        self.env
    }

    pub fn set_env(&mut self, env: Option<GcHandle>) {
        self.env = env;
    }
}

impl std::fmt::Debug for Userdata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Userdata")
            .field("metatable", &self.metatable)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

impl Trace for Userdata {
    fn trace(&self, tracer: &mut Tracer) {
        if let Some(mt) = self.metatable {
            tracer.mark(mt);
        }
        if let Some(env) = self.env {
            tracer.mark(env);
        }
    }
}
