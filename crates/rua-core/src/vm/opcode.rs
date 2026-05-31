//! バイトコード命令セット（本家 `lopcodes.h` 相当）。担当: **lua-vm**（frontend と共有）。
//!
//! 本モジュールは codegen（lua-frontend）が生成し interp（lua-vm）が解釈する
//! バイトコードの **唯一の真実（single source of truth）** である。両者の共有契約のため、
//! 変更は必ず双方で合意する。
//!
//! # 命令フォーマット（本家 Lua 5.1）
//! 命令は 32bit 固定長。下位 6bit が [`OpCode`]、残りが引数。3 種のレイアウト:
//!
//! ```text
//!  31      23      14     6      0   bit 位置
//!  |   B   |   C   |  A  | Op |     iABC :  B(9) C(9) A(8) Op(6)
//!  |     Bx        |  A  | Op |     iABx :  Bx(18)     A(8) Op(6)
//!  |    sBx        |  A  | Op |     iAsBx: sBx(18,bias) A(8) Op(6)
//! ```
//!
//! - `A`:  8bit（0..=255）
//! - `B`,`C`: 各 9bit（0..=511）。算術等のオペランドは **RK エンコード**（[`is_k`]）で
//!   レジスタ番号か定数表インデックスかを表す。
//! - `Bx`: 18bit 符号なし（0..=262143）。`LOADK`/`GETGLOBAL`/`CLOSURE` 等の定数/proto 番号。
//! - `sBx`: 18bit を [`MAXARG_SBX`] だけバイアスした符号付き整数（ジャンプ用）。
//!
//! # RK エンコード
//! `B`/`C` の最上位ビット（`1<<8` = [`BITRK`]）が立っていれば定数表インデックス
//! （下位 8bit が [`index_k`]）、立っていなければレジスタ番号。よって RK で参照できる
//! 定数は 256 個まで（それを超える定数は `LOADK` で明示ロードする）。

use std::fmt;

// ---- フィールドのサイズと位置（本家 lopcodes.h のマクロに対応）-------------

/// `OpCode` のビット幅。
pub const SIZE_OP: u32 = 6;
/// `A` のビット幅。
pub const SIZE_A: u32 = 8;
/// `B` のビット幅。
pub const SIZE_B: u32 = 9;
/// `C` のビット幅。
pub const SIZE_C: u32 = 9;
/// `Bx` のビット幅。
pub const SIZE_BX: u32 = 18;

const POS_OP: u32 = 0;
const POS_A: u32 = POS_OP + SIZE_OP; // 6
const POS_C: u32 = POS_A + SIZE_A; // 14
const POS_B: u32 = POS_C + SIZE_C; // 23
const POS_BX: u32 = POS_C; // 14

/// `A` の最大値。
pub const MAXARG_A: u32 = (1 << SIZE_A) - 1;
/// `B`/`C` の最大値。
pub const MAXARG_B: u32 = (1 << SIZE_B) - 1;
/// `B`/`C` の最大値。
pub const MAXARG_C: u32 = (1 << SIZE_C) - 1;
/// `Bx` の最大値。
pub const MAXARG_BX: u32 = (1 << SIZE_BX) - 1;
/// `sBx` のバイアス値（`sBx = Bx - MAXARG_SBX`）。
pub const MAXARG_SBX: i32 = (MAXARG_BX >> 1) as i32;

// ---- RK エンコード（本家 lopcodes.h の BITRK 系）---------------------------

/// RK 値が「定数表インデックス」であることを示すビット（`B`/`C` の最上位ビット）。
pub const BITRK: u32 = 1 << (SIZE_B - 1); // 256

/// RK でアクセスできる定数インデックスの最大値。
pub const MAXINDEXRK: u32 = BITRK - 1;

/// RK 値が定数表インデックスか（true: 定数, false: レジスタ）。
#[inline]
pub fn is_k(x: u32) -> bool {
    x & BITRK != 0
}

/// RK 値から定数表インデックスを取り出す（[`is_k`] が true のとき有効）。
#[inline]
pub fn index_k(x: u32) -> u32 {
    x & !BITRK
}

/// 定数表インデックスを RK 値（定数フラグ付き）へエンコードする。
#[inline]
pub fn rk_as_k(idx: u32) -> u32 {
    idx | BITRK
}

/// `SETLIST` が 1 命令で書き込めるフィールド数（本家 `LFIELDS_PER_FLUSH`）。
pub const LFIELDS_PER_FLUSH: u32 = 50;

// ---- OpCode ----------------------------------------------------------------

/// レジスタ型 VM の命令種別（本家 `OpCode`, `lopcodes.h`）。
///
/// 値（判別子）は本家の列挙順に一致させてあり、バイトコードの 6bit フィールドへ
/// そのままエンコードされる。順序の変更はバイトコード互換性を壊すため不可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    /// `R(A) := R(B)`
    Move = 0,
    /// `R(A) := Kst(Bx)`
    LoadK,
    /// `R(A) := (Bool)B; if (C) pc++`
    LoadBool,
    /// `R(A) := ... := R(B) := nil`
    LoadNil,
    /// `R(A) := UpValue[B]`
    GetUpval,
    /// `R(A) := Gbl[Kst(Bx)]`
    GetGlobal,
    /// `R(A) := R(B)[RK(C)]`
    GetTable,
    /// `Gbl[Kst(Bx)] := R(A)`
    SetGlobal,
    /// `UpValue[B] := R(A)`
    SetUpval,
    /// `R(A)[RK(B)] := RK(C)`
    SetTable,
    /// `R(A) := {} (size = B,C)`
    NewTable,
    /// `R(A+1) := R(B); R(A) := R(B)[RK(C)]`
    SelfOp,
    /// `R(A) := RK(B) + RK(C)`
    Add,
    /// `R(A) := RK(B) - RK(C)`
    Sub,
    /// `R(A) := RK(B) * RK(C)`
    Mul,
    /// `R(A) := RK(B) / RK(C)`
    Div,
    /// `R(A) := RK(B) % RK(C)`
    Mod,
    /// `R(A) := RK(B) ^ RK(C)`
    Pow,
    /// `R(A) := -R(B)`
    Unm,
    /// `R(A) := not R(B)`
    Not,
    /// `R(A) := length of R(B)`
    Len,
    /// `R(A) := R(B).. ... ..R(C)`
    Concat,
    /// `pc += sBx`
    Jmp,
    /// `if ((RK(B) == RK(C)) ~= A) then pc++`
    Eq,
    /// `if ((RK(B) <  RK(C)) ~= A) then pc++`
    Lt,
    /// `if ((RK(B) <= RK(C)) ~= A) then pc++`
    Le,
    /// `if not (R(A) <=> C) then pc++`
    Test,
    /// `if (R(B) <=> C) then R(A) := R(B) else pc++`
    TestSet,
    /// `R(A), ... ,R(A+C-2) := R(A)(R(A+1), ... ,R(A+B-1))`
    Call,
    /// `return R(A)(R(A+1), ... ,R(A+B-1))`
    TailCall,
    /// `return R(A), ... ,R(A+B-2)`
    Return,
    /// `R(A) += R(A+2); if R(A) <?= R(A+1) then { pc += sBx; R(A+3) = R(A) }`
    ForLoop,
    /// `R(A) -= R(A+2); pc += sBx`
    ForPrep,
    /// `R(A+3), ... ,R(A+2+C) := R(A)(R(A+1), R(A+2)); if R(A+3) ~= nil then R(A+2) = R(A+3) else pc++`
    TForLoop,
    /// `R(A)[(C-1)*FPF+i] := R(A+i), 1 <= i <= B`
    SetList,
    /// close all variables in the stack up to (>=) R(A)
    Close,
    /// `R(A) := closure(KPROTO[Bx], R(A), ... ,R(A+n))`
    Closure,
    /// `R(A), R(A+1), ..., R(A+B-2) = vararg`
    Vararg,
}

impl OpCode {
    /// 命令の総数（0..[`NUM_OPCODES`)）。
    pub const NUM_OPCODES: u8 = OpCode::Vararg as u8 + 1;

    /// 6bit のオペコードフィールドから [`OpCode`] を復元する。範囲外は `None`。
    #[inline]
    pub fn from_u8(v: u8) -> Option<OpCode> {
        if v < OpCode::NUM_OPCODES {
            // 判別子が連続（0..NUM）で repr(u8) のため transmute 可能だが、
            // unsafe を避けるため明示 match で復元する。
            Some(ALL_OPCODES[v as usize])
        } else {
            None
        }
    }

    /// この命令のオペランド形式。
    #[inline]
    pub fn mode(self) -> OpMode {
        OP_MODES[self as usize]
    }

    /// 命令のニーモニック名（デバッグ/ダンプ用, `luac -l` 風）。
    pub fn name(self) -> &'static str {
        OP_NAMES[self as usize]
    }
}

/// 命令のオペランド形式（本家 `enum OpMode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    /// A, B, C
    ABC,
    /// A, Bx
    ABx,
    /// A, sBx
    AsBx,
}

/// 命令の総数。
pub const NUM_OPCODES: usize = OpCode::NUM_OPCODES as usize;

const ALL_OPCODES: [OpCode; NUM_OPCODES] = [
    OpCode::Move,
    OpCode::LoadK,
    OpCode::LoadBool,
    OpCode::LoadNil,
    OpCode::GetUpval,
    OpCode::GetGlobal,
    OpCode::GetTable,
    OpCode::SetGlobal,
    OpCode::SetUpval,
    OpCode::SetTable,
    OpCode::NewTable,
    OpCode::SelfOp,
    OpCode::Add,
    OpCode::Sub,
    OpCode::Mul,
    OpCode::Div,
    OpCode::Mod,
    OpCode::Pow,
    OpCode::Unm,
    OpCode::Not,
    OpCode::Len,
    OpCode::Concat,
    OpCode::Jmp,
    OpCode::Eq,
    OpCode::Lt,
    OpCode::Le,
    OpCode::Test,
    OpCode::TestSet,
    OpCode::Call,
    OpCode::TailCall,
    OpCode::Return,
    OpCode::ForLoop,
    OpCode::ForPrep,
    OpCode::TForLoop,
    OpCode::SetList,
    OpCode::Close,
    OpCode::Closure,
    OpCode::Vararg,
];

const OP_NAMES: [&str; NUM_OPCODES] = [
    "MOVE",
    "LOADK",
    "LOADBOOL",
    "LOADNIL",
    "GETUPVAL",
    "GETGLOBAL",
    "GETTABLE",
    "SETGLOBAL",
    "SETUPVAL",
    "SETTABLE",
    "NEWTABLE",
    "SELF",
    "ADD",
    "SUB",
    "MUL",
    "DIV",
    "MOD",
    "POW",
    "UNM",
    "NOT",
    "LEN",
    "CONCAT",
    "JMP",
    "EQ",
    "LT",
    "LE",
    "TEST",
    "TESTSET",
    "CALL",
    "TAILCALL",
    "RETURN",
    "FORLOOP",
    "FORPREP",
    "TFORLOOP",
    "SETLIST",
    "CLOSE",
    "CLOSURE",
    "VARARG",
];

const OP_MODES: [OpMode; NUM_OPCODES] = {
    use OpMode::*;
    [
        ABC,  // MOVE
        ABx,  // LOADK
        ABC,  // LOADBOOL
        ABC,  // LOADNIL
        ABC,  // GETUPVAL
        ABx,  // GETGLOBAL
        ABC,  // GETTABLE
        ABx,  // SETGLOBAL
        ABC,  // SETUPVAL
        ABC,  // SETTABLE
        ABC,  // NEWTABLE
        ABC,  // SELF
        ABC,  // ADD
        ABC,  // SUB
        ABC,  // MUL
        ABC,  // DIV
        ABC,  // MOD
        ABC,  // POW
        ABC,  // UNM
        ABC,  // NOT
        ABC,  // LEN
        ABC,  // CONCAT
        AsBx, // JMP
        ABC,  // EQ
        ABC,  // LT
        ABC,  // LE
        ABC,  // TEST
        ABC,  // TESTSET
        ABC,  // CALL
        ABC,  // TAILCALL
        ABC,  // RETURN
        AsBx, // FORLOOP
        AsBx, // FORPREP
        ABC,  // TFORLOOP
        ABC,  // SETLIST
        ABC,  // CLOSE
        ABx,  // CLOSURE
        ABC,  // VARARG
    ]
};

// ---- Instruction -----------------------------------------------------------

/// 1 命令（32bit にパックされたオペコード + 引数）。
///
/// 内部表現は本家と同じ生の `u32`。デコードはアクセサ（[`Instruction::opcode`] 等）、
/// エンコードはコンストラクタ（[`Instruction::abc`] 等）で行う。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Instruction(pub u32);

impl Instruction {
    /// 生の 32bit から構築する（バイナリチャンク読込用）。
    #[inline]
    pub fn from_raw(raw: u32) -> Self {
        Instruction(raw)
    }

    /// 生の 32bit を取り出す。
    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    /// iABC 形式の命令を作る。
    #[inline]
    pub fn abc(op: OpCode, a: u32, b: u32, c: u32) -> Self {
        debug_assert!(a <= MAXARG_A && b <= MAXARG_B && c <= MAXARG_C);
        Instruction(((op as u32) << POS_OP) | (a << POS_A) | (b << POS_B) | (c << POS_C))
    }

    /// iABx 形式の命令を作る。
    #[inline]
    pub fn abx(op: OpCode, a: u32, bx: u32) -> Self {
        debug_assert!(a <= MAXARG_A && bx <= MAXARG_BX);
        Instruction(((op as u32) << POS_OP) | (a << POS_A) | (bx << POS_BX))
    }

    /// iAsBx 形式の命令を作る（`sbx` はバイアス前の符号付き値）。
    #[inline]
    pub fn asbx(op: OpCode, a: u32, sbx: i32) -> Self {
        debug_assert!(a <= MAXARG_A);
        debug_assert!((-MAXARG_SBX..=MAXARG_SBX).contains(&sbx));
        let bx = (sbx + MAXARG_SBX) as u32;
        Instruction::abx(op, a, bx)
    }

    /// オペコードフィールドの生値（0..[`NUM_OPCODES`)）。
    #[inline]
    pub fn opcode_raw(self) -> u8 {
        (self.0 & ((1 << SIZE_OP) - 1)) as u8
    }

    /// デコード済みオペコード。未知のオペコードなら `None`。
    #[inline]
    pub fn opcode(self) -> Option<OpCode> {
        OpCode::from_u8(self.opcode_raw())
    }

    /// 引数 `A`。
    #[inline]
    pub fn a(self) -> u32 {
        (self.0 >> POS_A) & MAXARG_A
    }

    /// 引数 `B`。
    #[inline]
    pub fn b(self) -> u32 {
        (self.0 >> POS_B) & MAXARG_B
    }

    /// 引数 `C`。
    #[inline]
    pub fn c(self) -> u32 {
        (self.0 >> POS_C) & MAXARG_C
    }

    /// 引数 `Bx`（符号なし 18bit）。
    #[inline]
    pub fn bx(self) -> u32 {
        (self.0 >> POS_BX) & MAXARG_BX
    }

    /// 引数 `sBx`（バイアス済み符号付き）。
    #[inline]
    pub fn sbx(self) -> i32 {
        self.bx() as i32 - MAXARG_SBX
    }
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.opcode() {
            Some(op) => match op.mode() {
                OpMode::ABC => write!(f, "{} {} {} {}", op.name(), self.a(), self.b(), self.c()),
                OpMode::ABx => write!(f, "{} {} {}", op.name(), self.a(), self.bx()),
                OpMode::AsBx => write!(f, "{} {} {}", op.name(), self.a(), self.sbx()),
            },
            None => write!(f, "<bad opcode {:#x}>", self.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_roundtrip() {
        for i in 0..OpCode::NUM_OPCODES {
            let op = OpCode::from_u8(i).unwrap();
            assert_eq!(op as u8, i);
        }
        assert!(OpCode::from_u8(OpCode::NUM_OPCODES).is_none());
    }

    #[test]
    fn abc_encode_decode() {
        let i = Instruction::abc(OpCode::Add, 1, rk_as_k(2), 100);
        assert_eq!(i.opcode(), Some(OpCode::Add));
        assert_eq!(i.a(), 1);
        assert_eq!(i.b(), rk_as_k(2));
        assert_eq!(i.c(), 100);
        assert!(is_k(i.b()));
        assert_eq!(index_k(i.b()), 2);
        assert!(!is_k(i.c()));
    }

    #[test]
    fn abx_encode_decode() {
        let i = Instruction::abx(OpCode::LoadK, 7, 131072);
        assert_eq!(i.opcode(), Some(OpCode::LoadK));
        assert_eq!(i.a(), 7);
        assert_eq!(i.bx(), 131072);
    }

    #[test]
    fn asbx_encode_decode() {
        for sbx in [-MAXARG_SBX, -1, 0, 1, MAXARG_SBX] {
            let i = Instruction::asbx(OpCode::Jmp, 0, sbx);
            assert_eq!(i.opcode(), Some(OpCode::Jmp));
            assert_eq!(i.sbx(), sbx);
        }
    }

    #[test]
    fn field_positions_match_lua51() {
        // A は 6bit 目から、C は 14bit 目、B は 23bit 目（本家レイアウト）。
        assert_eq!(POS_A, 6);
        assert_eq!(POS_C, 14);
        assert_eq!(POS_B, 23);
        assert_eq!(BITRK, 256);
        assert_eq!(MAXARG_SBX, 131071);
    }
}
