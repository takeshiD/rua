//! コンパイル済みチャンクのシリアライズ（本家 `luac` の `dump`/`undump` 相当）。
//!
//! 本家 Lua 5.1 のバイナリチャンク形式（`\x1bLua`）はプラットフォーム依存のヘッダ
//! （サイズ・エンディアン）を持ち、互換ローダ実装は rua-core 側（VM/frontend）の合意が必要。
//! そこで当面は **rua 独自の可搬フォーマット**（マジック `\x1bRua`・常にリトルエンディアン）を
//! 採用する。`rua run <file>` はこのマジックを検出すると逆シリアライズして実行する（[`crate::run`]）。
//!
//! 将来 本家 dump 互換が必要になった時点で lua-vm/lua-frontend と形式を確定し差し替える。

use std::rc::Rc;

use rua_core::gc::{GcHandle, Heap};
use rua_core::value::Value;
use rua_core::vm::Proto;
use rua_core::vm::opcode::Instruction;
use rua_core::vm::proto::LocalVar;

/// rua バイナリチャンクのマジック（本家 `\x1bLua` と区別する）。
pub const RUA_SIGNATURE: &[u8; 4] = b"\x1bRua";
/// フォーマットバージョン。
const VERSION: u8 = 1;

// 定数タグ。
const TAG_NIL: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_NUMBER: u8 = 2;
const TAG_STRING: u8 = 3;

/// バイト列が rua バイナリチャンクか判定する。
pub fn is_rua_chunk(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..4] == RUA_SIGNATURE
}

// ---- dump ------------------------------------------------------------------

/// `Proto` を rua バイナリチャンクへシリアライズする。
///
/// `strip` が真ならデバッグ情報（行番号・ローカル名・upvalue 名・ソース名）を除く。
pub fn dump(heap: &Heap, proto: &Proto, strip: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(RUA_SIGNATURE);
    buf.push(VERSION);
    buf.push(strip as u8);
    dump_proto(&mut buf, heap, proto, strip);
    buf
}

fn dump_proto(buf: &mut Vec<u8>, heap: &Heap, p: &Proto, strip: bool) {
    buf.push(p.num_params);
    buf.push(p.is_vararg as u8);
    buf.push(p.max_stack_size);
    buf.push(p.num_upvalues);
    write_u32(buf, p.line_defined);
    write_u32(buf, p.last_line_defined);

    // source（strip 時は省略）。
    match (strip, p.source.as_deref()) {
        (false, Some(s)) => {
            buf.push(1);
            write_bytes(buf, s.as_bytes());
        }
        _ => buf.push(0),
    }

    // code
    write_u32(buf, p.code.len() as u32);
    for ins in &p.code {
        write_u32(buf, ins.raw());
    }

    // line_info（strip 時は空）。
    if strip {
        write_u32(buf, 0);
    } else {
        write_u32(buf, p.line_info.len() as u32);
        for &l in &p.line_info {
            write_u32(buf, l);
        }
    }

    // constants
    write_u32(buf, p.constants.len() as u32);
    for v in &p.constants {
        dump_constant(buf, heap, v);
    }

    // local_vars（strip 時は空）。
    if strip {
        write_u32(buf, 0);
    } else {
        write_u32(buf, p.local_vars.len() as u32);
        for lv in &p.local_vars {
            write_bytes(buf, lv.name.as_bytes());
            write_u32(buf, lv.start_pc);
            write_u32(buf, lv.end_pc);
        }
    }

    // upvalue_names（strip 時は空）。
    if strip {
        write_u32(buf, 0);
    } else {
        write_u32(buf, p.upvalue_names.len() as u32);
        for name in &p.upvalue_names {
            write_bytes(buf, name.as_bytes());
        }
    }

    // nested protos
    write_u32(buf, p.protos.len() as u32);
    for child in &p.protos {
        dump_proto(buf, heap, child, strip);
    }
}

fn dump_constant(buf: &mut Vec<u8>, heap: &Heap, v: &Value) {
    match v {
        Value::Nil => buf.push(TAG_NIL),
        Value::Boolean(b) => {
            buf.push(TAG_BOOL);
            buf.push(*b as u8);
        }
        Value::Number(n) => {
            buf.push(TAG_NUMBER);
            write_u64(buf, n.to_bits());
        }
        Value::GcRef(GcHandle::Str(key)) => {
            buf.push(TAG_STRING);
            let bytes = heap.get_str(*key).map(|s| s.as_bytes()).unwrap_or(b"");
            write_bytes(buf, bytes);
        }
        // 定数表にはこれら以外の値は現れない（codegen 契約）。
        other => panic!("dump: 想定外の定数型 {:?}", other.type_of()),
    }
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    write_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
}

// ---- undump ----------------------------------------------------------------

/// 逆シリアライズ時のエラー。
#[derive(Debug)]
pub struct UndumpError(pub String);

impl std::fmt::Display for UndumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// バイト列カーソル。
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], UndumpError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| UndumpError("不正な rua チャンク: 途中で終端".into()))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, UndumpError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, UndumpError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, UndumpError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn bytes(&mut self) -> Result<&'a [u8], UndumpError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    fn string(&mut self) -> Result<String, UndumpError> {
        let b = self.bytes()?;
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}

/// rua バイナリチャンクを逆シリアライズし、メイン [`Proto`] を返す。
///
/// 文字列定数・ソース名は `heap` にインターンする。
pub fn undump(heap: &mut Heap, data: &[u8]) -> Result<Proto, UndumpError> {
    let mut r = Reader::new(data);
    if r.take(4)? != RUA_SIGNATURE {
        return Err(UndumpError("rua チャンクのマジックが不正".into()));
    }
    let version = r.u8()?;
    if version != VERSION {
        return Err(UndumpError(format!(
            "未対応の rua チャンクバージョン: {version}（対応 {VERSION}）"
        )));
    }
    let _strip = r.u8()?;
    undump_proto(heap, &mut r)
}

fn undump_proto(heap: &mut Heap, r: &mut Reader) -> Result<Proto, UndumpError> {
    let mut p = Proto::new();
    p.num_params = r.u8()?;
    p.is_vararg = r.u8()? != 0;
    p.max_stack_size = r.u8()?;
    p.num_upvalues = r.u8()?;
    p.line_defined = r.u32()?;
    p.last_line_defined = r.u32()?;

    p.source = if r.u8()? != 0 {
        Some(r.string()?)
    } else {
        None
    };

    let ncode = r.u32()? as usize;
    p.code = Vec::with_capacity(ncode);
    for _ in 0..ncode {
        p.code.push(Instruction::from_raw(r.u32()?));
    }

    let nlines = r.u32()? as usize;
    p.line_info = Vec::with_capacity(nlines);
    for _ in 0..nlines {
        p.line_info.push(r.u32()?);
    }

    let nconst = r.u32()? as usize;
    p.constants = Vec::with_capacity(nconst);
    for _ in 0..nconst {
        p.constants.push(undump_constant(heap, r)?);
    }

    let nlocals = r.u32()? as usize;
    p.local_vars = Vec::with_capacity(nlocals);
    for _ in 0..nlocals {
        let name = r.string()?;
        let start_pc = r.u32()?;
        let end_pc = r.u32()?;
        p.local_vars.push(LocalVar {
            name,
            start_pc,
            end_pc,
        });
    }

    let nupval = r.u32()? as usize;
    p.upvalue_names = Vec::with_capacity(nupval);
    for _ in 0..nupval {
        p.upvalue_names.push(r.string()?);
    }

    let nprotos = r.u32()? as usize;
    p.protos = Vec::with_capacity(nprotos);
    for _ in 0..nprotos {
        p.protos.push(Rc::new(undump_proto(heap, r)?));
    }

    Ok(p)
}

fn undump_constant(heap: &mut Heap, r: &mut Reader) -> Result<Value, UndumpError> {
    match r.u8()? {
        TAG_NIL => Ok(Value::Nil),
        TAG_BOOL => Ok(Value::Boolean(r.u8()? != 0)),
        TAG_NUMBER => Ok(Value::Number(f64::from_bits(r.u64()?))),
        TAG_STRING => {
            let bytes = r.bytes()?;
            Ok(Value::GcRef(heap.intern_str(bytes)))
        }
        other => Err(UndumpError(format!("不正な定数タグ {other}"))),
    }
}
