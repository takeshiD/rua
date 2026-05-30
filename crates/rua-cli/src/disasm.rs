//! バイトコード逆アセンブル（本家 `luac -l` の `PrintFunction` 相当）。
//!
//! [`Proto`] を本家 `luac -l` の表記に寄せて整形する。命令一覧（既定）に加え、
//! `-l` を 2 回以上指定すると定数表・ローカル変数・upvalue も出力する（本家 `luac -l -l`）。
//!
//! 本家 `luac` が出力する関数アドレス（`at 0x...`）は非決定的なので省略し、
//! それ以外の構造（ヘッダ・命令・オペランド・コメント）を忠実に再現する。

use std::fmt::Write as _;

use rua_core::compiler::chunk_id;
use rua_core::gc::{GcHandle, Heap};
use rua_core::value::Value;
use rua_core::value::convert::number_to_string;
use rua_core::vm::Proto;
use rua_core::vm::opcode::{Instruction, OpCode, OpMode, index_k, is_k};

/// 命令オペランド `B`/`C` の用途（本家 `OpArgMask`）。
///
/// `luac -l` がオペランドを表示するか、定数（`K`）として `-1-idx` 表記にするかは
/// この用途で決まる。本家 `lopcodes.c` の `luaP_opmodes` 表に一致させる。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgMode {
    /// 未使用（表示しない）。
    N,
    /// 即値（そのまま表示）。
    U,
    /// レジスタ（そのまま表示）。
    R,
    /// レジスタ or 定数（RK エンコード。定数なら `-1-idx`）。
    K,
}

/// オペコードごとの `(Bmode, Cmode)`（本家 `luaP_opmodes`）。
fn bc_modes(op: OpCode) -> (ArgMode, ArgMode) {
    use ArgMode::*;
    use OpCode::*;
    match op {
        Move => (R, N),
        LoadK => (K, N),
        LoadBool => (U, U),
        LoadNil => (R, N),
        GetUpval => (U, N),
        GetGlobal => (K, N),
        GetTable => (R, K),
        SetGlobal => (K, N),
        SetUpval => (U, N),
        SetTable => (K, K),
        NewTable => (U, U),
        SelfOp => (R, K),
        Add | Sub | Mul | Div | Mod | Pow => (K, K),
        Unm | Not | Len => (R, N),
        Concat => (R, R),
        Jmp => (R, N),
        Eq | Lt | Le => (K, K),
        Test => (R, U),
        TestSet => (R, U),
        Call => (U, U),
        TailCall => (U, U),
        Return => (U, N),
        ForLoop => (R, N),
        ForPrep => (R, N),
        TForLoop => (N, U),
        SetList => (U, U),
        Close => (N, N),
        Closure => (U, N),
        Vararg => (U, N),
    }
}

/// `Proto` を `luac -l` 風に整形して文字列で返す。
///
/// `verbose`（`-l` の指定回数）が 2 以上なら定数表・ローカル・upvalue も出力する。
pub fn disassemble(heap: &Heap, proto: &Proto, verbose: u8) -> String {
    let mut out = String::new();
    print_function(&mut out, heap, proto, verbose);
    out
}

fn print_function(out: &mut String, heap: &Heap, p: &Proto, verbose: u8) {
    print_header(out, p);
    print_code(out, heap, p);
    if verbose >= 2 {
        print_constants(out, heap, p);
        print_locals(out, p);
        print_upvalues(out, p);
    }
    // 子プロトタイプを再帰的に出力する（本家同様、深さ優先）。
    for child in &p.protos {
        out.push('\n');
        print_function(out, heap, child, verbose);
    }
}

fn print_header(out: &mut String, p: &Proto) {
    let kind = if p.line_defined == 0 { "main" } else { "function" };
    let source = p.source.as_deref().map(chunk_id).unwrap_or_default();
    let n = p.code.len();
    let _ = writeln!(
        out,
        "\n{kind} <{source}:{},{}> ({n} instruction{}, {} bytes)",
        p.line_defined,
        p.last_line_defined,
        if n == 1 { "" } else { "s" },
        n * 4,
    );
    let _ = writeln!(
        out,
        "{}{} params, {} slots, {} upvalues, {} locals, {} constants, {} functions",
        p.num_params,
        if p.is_vararg { "+" } else { "" },
        p.max_stack_size,
        p.num_upvalues,
        p.local_vars.len(),
        p.constants.len(),
        p.protos.len(),
    );
}

fn print_code(out: &mut String, heap: &Heap, p: &Proto) {
    for (pc, ins) in p.code.iter().enumerate() {
        let line = p.line_at(pc);
        let _ = write!(out, "\t{}\t", pc + 1);
        if line > 0 {
            let _ = write!(out, "[{line}]\t");
        } else {
            let _ = write!(out, "[-]\t");
        }
        let Some(op) = ins.opcode() else {
            let _ = writeln!(out, "<bad opcode {:#x}>", ins.raw());
            continue;
        };
        let _ = write!(out, "{:<9}\t", op.name());
        print_operands(out, *ins, op);
        print_comment(out, heap, p, pc, *ins, op);
        out.push('\n');
    }
}

/// オペランド部（本家 `PrintCode` の switch(getOpMode) 相当）。
fn print_operands(out: &mut String, ins: Instruction, op: OpCode) {
    let a = ins.a();
    match op.mode() {
        OpMode::ABC => {
            let _ = write!(out, "{a}");
            let (bmode, cmode) = bc_modes(op);
            if bmode != ArgMode::N {
                let _ = write!(out, " {}", rk_operand(ins.b(), bmode));
            }
            if cmode != ArgMode::N {
                let _ = write!(out, " {}", rk_operand(ins.c(), cmode));
            }
        }
        OpMode::ABx => {
            let (bmode, _) = bc_modes(op);
            if bmode == ArgMode::K {
                // 定数番号は本家同様 `-1-Bx` で表す。
                let _ = write!(out, "{a} {}", -1 - (ins.bx() as i64));
            } else {
                let _ = write!(out, "{a} {}", ins.bx());
            }
        }
        OpMode::AsBx => {
            if op == OpCode::Jmp {
                let _ = write!(out, "{}", ins.sbx());
            } else {
                let _ = write!(out, "{a} {}", ins.sbx());
            }
        }
    }
}

/// RK オペランドを本家表記で表す（定数なら `-1-idx`、レジスタ/即値はそのまま）。
fn rk_operand(x: u32, mode: ArgMode) -> i64 {
    if mode == ArgMode::K && is_k(x) {
        -1 - (index_k(x) as i64)
    } else {
        x as i64
    }
}

/// 末尾コメント（本家 `PrintCode` の switch(o) 相当）。
fn print_comment(out: &mut String, heap: &Heap, p: &Proto, pc: usize, ins: Instruction, op: OpCode) {
    use OpCode::*;
    match op {
        LoadK => {
            let _ = write!(out, "\t; {}", const_str(heap, p, ins.bx() as usize));
        }
        GetUpval | SetUpval => {
            if let Some(name) = p.upvalue_names.get(ins.b() as usize) {
                let _ = write!(out, "\t; {name}");
            }
        }
        GetGlobal | SetGlobal => {
            let _ = write!(out, "\t; {}", const_str(heap, p, ins.bx() as usize));
        }
        GetTable | SelfOp => {
            if is_k(ins.c()) {
                let _ = write!(out, "\t; {}", const_str(heap, p, index_k(ins.c()) as usize));
            }
        }
        SetTable | Add | Sub | Mul | Div | Mod | Pow | Eq | Lt | Le => {
            let bk = is_k(ins.b());
            let ck = is_k(ins.c());
            if bk || ck {
                let _ = write!(out, "\t; ");
                if bk {
                    let _ = write!(out, "{}", const_str(heap, p, index_k(ins.b()) as usize));
                } else {
                    let _ = write!(out, "-");
                }
                let _ = write!(out, " ");
                if ck {
                    let _ = write!(out, "{}", const_str(heap, p, index_k(ins.c()) as usize));
                } else {
                    let _ = write!(out, "-");
                }
            }
        }
        Jmp | ForLoop | ForPrep => {
            // 飛び先（1 始まりの命令番号）。本家: sbx + pc + 2。
            let _ = write!(out, "\t; to {}", ins.sbx() + pc as i32 + 2);
        }
        Closure => {
            // 本家は関数アドレスを出すが非決定的なので proto 番号で代替する。
            let _ = write!(out, "\t; function at index {}", ins.bx());
        }
        SetList => {
            if ins.c() == 0 {
                // C==0 は次命令にバッチ番号が埋め込まれている（本家同様）。
                if let Some(next) = p.code.get(pc + 1) {
                    let _ = write!(out, "\t; {}", next.raw());
                }
            } else {
                let _ = write!(out, "\t; {}", ins.c());
            }
        }
        _ => {}
    }
}

/// 定数表 `idx` の表示用文字列（数値・真偽・nil・文字列）。
fn const_str(heap: &Heap, p: &Proto, idx: usize) -> String {
    match p.constants.get(idx) {
        Some(v) => value_repr(heap, v),
        None => "?".to_string(),
    }
}

/// 定数値の `luac` 風表現（文字列はクォート・エスケープ）。
fn value_repr(heap: &Heap, v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => number_to_string(*n),
        Value::GcRef(GcHandle::Str(key)) => match heap.get_str(*key) {
            Some(s) => quote_string(s.as_bytes()),
            None => "\"?\"".to_string(),
        },
        other => format!("({} value)", other.type_of().name()),
    }
}

/// 本家 `PrintString` 相当: ダブルクォートで囲み制御文字をエスケープする。
fn quote_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() + 2);
    s.push('"');
    for &b in bytes {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0 => s.push_str("\\0"),
            0x20..=0x7e => s.push(b as char),
            other => {
                let _ = write!(s, "\\{other}");
            }
        }
    }
    s.push('"');
    s
}

fn print_constants(out: &mut String, heap: &Heap, p: &Proto) {
    let _ = writeln!(out, "constants ({}):", p.constants.len());
    for (i, v) in p.constants.iter().enumerate() {
        let _ = writeln!(out, "\t{}\t{}", i + 1, value_repr(heap, v));
    }
}

fn print_locals(out: &mut String, p: &Proto) {
    let _ = writeln!(out, "locals ({}):", p.local_vars.len());
    for (i, lv) in p.local_vars.iter().enumerate() {
        let _ = writeln!(out, "\t{}\t{}\t{}\t{}", i, lv.name, lv.start_pc + 1, lv.end_pc + 1);
    }
}

fn print_upvalues(out: &mut String, p: &Proto) {
    let _ = writeln!(out, "upvalues ({}):", p.num_upvalues);
    for (i, name) in p.upvalue_names.iter().enumerate() {
        let _ = writeln!(out, "\t{i}\t{name}");
    }
}
