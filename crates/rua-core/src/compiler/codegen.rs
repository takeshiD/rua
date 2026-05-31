//! コード生成（本家 `lcode.c` + `lparser.c` のレジスタ割付部 相当）。担当: **lua-frontend**。
//!
//! [`ast::Block`](crate::compiler::ast::Block) を走査し、本家 Lua 5.1 のレジスタ機械向け
//! バイトコード（[`vm::opcode`](crate::vm::opcode)）を生成して [`Proto`] を構築する。
//!
//! 本家 `lcode.c` の式記述子（`expdesc`）・ジャンプリスト・定数畳み込み・RK エンコード、
//! および `lparser.c` のレジスタ割付（`freereg`/`nactvar`）・スコープ（block/upvalue）・
//! 制御構造（if/while/repeat/for）の生成規則を忠実に再現する。最終的な byte-exact 検証は
//! lua-conformance のゴールデン比較（本家 `luac -l`）で行う。

use std::collections::HashMap;
use std::rc::Rc;

use crate::compiler::ast::*;
use crate::error::{LuaError, LuaResult};
use crate::gc::Heap;
use crate::value::Value;
use crate::vm::opcode::*;
use crate::vm::proto::{LocalVar, Proto};

/// ジャンプリストの終端を表す番兵（本家 `NO_JUMP`）。
const NO_JUMP: i32 = -1;
/// 「レジスタなし」を表す値（本家 `NO_REG` = `MAXARG_A`）。
const NO_REG: u32 = MAXARG_A;
/// 可変長結果（本家 `LUA_MULTRET`）。
const MULTRET: i32 = -1;
/// レジスタ数の上限（本家 `MAXSTACK`）。
const MAXSTACK: u32 = 250;
/// ローカル変数の上限（本家 `LUAI_MAXVARS`）。
const MAX_VARS: usize = 200;
/// upvalue の上限（本家 `LUAI_MAXUPVALUES`）。
const MAX_UPVALUES: usize = 60;

// ============================================================================
// Instruction のフィールド書き換えヘルパ
// （opcode.rs は read-only 契約のため、ここで再エンコードして更新する）
// ============================================================================

fn set_arg_a(i: &mut Instruction, a: u32) {
    let op = i.opcode().expect("valid opcode");
    *i = match op.mode() {
        OpMode::ABC => Instruction::abc(op, a, i.b(), i.c()),
        OpMode::ABx => Instruction::abx(op, a, i.bx()),
        OpMode::AsBx => Instruction::asbx(op, a, i.sbx()),
    };
}

fn set_arg_b(i: &mut Instruction, b: u32) {
    let op = i.opcode().expect("valid opcode");
    debug_assert_eq!(op.mode(), OpMode::ABC);
    *i = Instruction::abc(op, i.a(), b, i.c());
}

fn set_arg_c(i: &mut Instruction, c: u32) {
    let op = i.opcode().expect("valid opcode");
    debug_assert_eq!(op.mode(), OpMode::ABC);
    *i = Instruction::abc(op, i.a(), i.b(), c);
}

fn set_arg_sbx(i: &mut Instruction, sbx: i32) {
    let op = i.opcode().expect("valid opcode");
    debug_assert_eq!(op.mode(), OpMode::AsBx);
    *i = Instruction::asbx(op, i.a(), sbx);
}

fn set_opcode_keep_args(i: &mut Instruction, op: OpCode) {
    // ABC 同士の付け替え（TESTSET→TEST, CALL→TAILCALL）専用。
    debug_assert_eq!(op.mode(), OpMode::ABC);
    *i = Instruction::abc(op, i.a(), i.b(), i.c());
}

/// テスト系命令か（本家 `testTMode`）。直後の `JMP` を条件付きにする。
fn is_test_mode(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Eq | OpCode::Lt | OpCode::Le | OpCode::Test | OpCode::TestSet
    )
}

/// 本家 `luaO_int2fb`: テーブルサイズを「浮動小数バイト」へ符号化。
fn int2fb(mut x: u32) -> u32 {
    let mut e = 0;
    while x >= 16 {
        x = (x + 1) >> 1;
        e += 1;
    }
    if x < 8 { x } else { ((e + 1) << 3) | (x - 8) }
}

// ============================================================================
// 式記述子（本家 expdesc）
// ============================================================================

#[derive(Clone)]
enum EK {
    /// 値なし。
    Void,
    Nil,
    True,
    False,
    /// 数値定数（まだ定数表に入れていない）。
    KNum(f64),
    /// 定数表インデックス（`VK`）。
    K(u32),
    /// ローカル変数（レジスタ番号）。
    Local(u32),
    /// upvalue（upvalue インデックス）。
    Upval(u32),
    /// グローバル変数（名前の定数表インデックス）。
    Global(u32),
    /// テーブル添字 `t[k]`（テーブルのレジスタ + キーの RK）。
    Indexed {
        table: u32,
        key: u32,
    },
    /// 再配置可能命令（A 未設定。`info` = pc）。
    Reloc(u32),
    /// 非再配置（既にレジスタ `info` にある）。
    NonReloc(u32),
    /// 関数呼び出し（`info` = CALL の pc）。
    Call(u32),
    /// 可変長 `...`（`info` = VARARG の pc）。
    Vararg(u32),
    /// 比較等のジャンプ（`info` = JMP の pc）。
    Jmp(u32),
}

#[derive(Clone)]
struct ExpDesc {
    k: EK,
    /// 真のとき脱出するジャンプのリスト。
    t: i32,
    /// 偽のとき脱出するジャンプのリスト。
    f: i32,
}

impl ExpDesc {
    fn new(k: EK) -> Self {
        ExpDesc {
            k,
            t: NO_JUMP,
            f: NO_JUMP,
        }
    }

    fn has_jumps(&self) -> bool {
        self.t != NO_JUMP || self.f != NO_JUMP
    }

    fn is_numeral(&self) -> bool {
        matches!(self.k, EK::KNum(_)) && self.t == NO_JUMP && self.f == NO_JUMP
    }

    fn num(&self) -> f64 {
        match self.k {
            EK::KNum(n) => n,
            _ => panic!("not a numeral"),
        }
    }
}

/// 定数表の重複排除キー。
#[derive(Clone, PartialEq, Eq, Hash)]
enum ConstKey {
    Nil,
    Bool(bool),
    /// f64 のビット列（`-0.0` は `0.0` に正規化）。
    Num(u64),
    Str(Vec<u8>),
}

/// upvalue の捕捉元（本家 `upvaldesc`）。
#[derive(Clone)]
struct UpvalDesc {
    /// 親のローカル（true: `MOVE` で捕捉）か親の upvalue（false: `GETUPVAL`）か。
    from_local: bool,
    /// 親レジスタ番号 または 親 upvalue インデックス。
    index: u32,
}

/// ブロック制御（本家 `BlockCnt`）。
struct BlockCnt {
    /// break 可能なループブロックか。
    is_loop: bool,
    /// このブロック開始時の有効ローカル数。
    nactvar: usize,
    /// ブロック内のローカルが upvalue として捕捉されたか（true なら `CLOSE` を出す）。
    has_upval: bool,
    /// break のジャンプリスト。
    break_list: i32,
}

/// 有効なローカル変数（index = レジスタ番号）。
struct ActiveLocal {
    /// 変数名。
    name: String,
    /// `Proto::local_vars` 内の対応インデックス（end_pc 設定用）。
    locvar_idx: usize,
}

/// 1 関数のコンパイル状態（本家 `FuncState`）。
struct FuncState {
    proto: Proto,
    const_map: HashMap<ConstKey, u32>,
    /// 有効なローカル変数。
    actives: Vec<ActiveLocal>,
    /// `new_localvar` で予約され `adjustlocalvars` 待ちのローカル（名前, locvar_idx）。
    pending: Vec<(String, usize)>,
    /// 次の空きレジスタ。
    freereg: u32,
    blocks: Vec<BlockCnt>,
    upvalues: Vec<UpvalDesc>,
    /// 次に出す命令へ繋ぐ保留ジャンプ（本家 `jpc`）。
    jpc: i32,
    /// 直近のジャンプ先 pc（本家 `lasttarget`）。
    lasttarget: i32,
}

impl FuncState {
    fn new() -> Self {
        let mut proto = Proto::new();
        proto.max_stack_size = 2; // レジスタ 0,1 は常に有効（本家準拠）
        FuncState {
            proto,
            const_map: HashMap::new(),
            actives: Vec::new(),
            pending: Vec::new(),
            freereg: 0,
            blocks: Vec::new(),
            upvalues: Vec::new(),
            jpc: NO_JUMP,
            lasttarget: -1,
        }
    }

    fn nactvar(&self) -> u32 {
        self.actives.len() as u32
    }

    fn pc(&self) -> i32 {
        self.proto.code.len() as i32
    }

    fn code_at(&self, pc: i32) -> Instruction {
        self.proto.code[pc as usize]
    }

    fn with_code<R>(&mut self, pc: i32, f: impl FnOnce(&mut Instruction) -> R) -> R {
        f(&mut self.proto.code[pc as usize])
    }

    // ---- 命令出力（本家 luaK_code 系）-------------------------------------

    fn emit(&mut self, i: Instruction, line: u32) -> i32 {
        self.discharge_jpc();
        self.proto.code.push(i);
        self.proto.line_info.push(line);
        self.pc() - 1
    }

    fn emit_abc(&mut self, op: OpCode, a: u32, b: u32, c: u32, line: u32) -> i32 {
        self.emit(Instruction::abc(op, a, b, c), line)
    }

    fn emit_abx(&mut self, op: OpCode, a: u32, bx: u32, line: u32) -> i32 {
        self.emit(Instruction::abx(op, a, bx), line)
    }

    fn emit_asbx(&mut self, op: OpCode, a: u32, sbx: i32, line: u32) -> i32 {
        self.emit(Instruction::asbx(op, a, sbx), line)
    }

    /// 直近に出した命令の行番号を上書きする（本家 `luaK_fixline`）。
    fn fixline(&mut self, line: u32) {
        let last = self.proto.line_info.len() - 1;
        self.proto.line_info[last] = line;
    }

    // ---- レジスタ管理 ------------------------------------------------------

    fn check_stack(&mut self, n: u32) -> LuaResult<()> {
        let newstack = self.freereg + n;
        if newstack > self.proto.max_stack_size as u32 {
            if newstack >= MAXSTACK {
                return Err(LuaError::Syntax(
                    "function or expression too complex".into(),
                ));
            }
            self.proto.max_stack_size = newstack as u8;
        }
        Ok(())
    }

    fn reserve_regs(&mut self, n: u32) -> LuaResult<()> {
        self.check_stack(n)?;
        self.freereg += n;
        Ok(())
    }

    fn free_reg(&mut self, reg: u32) {
        if !is_k(reg) && reg >= self.nactvar() {
            self.freereg -= 1;
            debug_assert_eq!(reg, self.freereg);
        }
    }

    fn free_exp(&mut self, e: &ExpDesc) {
        if let EK::NonReloc(r) = e.k {
            self.free_reg(r);
        }
    }

    // ---- 定数表 ------------------------------------------------------------

    fn add_constant(&mut self, key: ConstKey, v: Value) -> u32 {
        if let Some(&idx) = self.const_map.get(&key) {
            return idx;
        }
        let idx = self.proto.constants.len() as u32;
        self.proto.constants.push(v);
        self.const_map.insert(key, idx);
        idx
    }

    fn number_k(&mut self, n: f64) -> u32 {
        let bits = if n == 0.0 {
            0.0f64.to_bits()
        } else {
            n.to_bits()
        };
        self.add_constant(ConstKey::Num(bits), Value::Number(n))
    }

    fn bool_k(&mut self, b: bool) -> u32 {
        self.add_constant(ConstKey::Bool(b), Value::Boolean(b))
    }

    fn nil_k(&mut self) -> u32 {
        self.add_constant(ConstKey::Nil, Value::Nil)
    }

    // ---- ジャンプ（本家 lcode.c のジャンプリスト操作）----------------------

    fn get_jump(&self, pc: i32) -> i32 {
        let offset = self.code_at(pc).sbx();
        if offset == NO_JUMP {
            NO_JUMP
        } else {
            (pc + 1) + offset
        }
    }

    fn fix_jump(&mut self, pc: i32, dest: i32) -> LuaResult<()> {
        let offset = dest - (pc + 1);
        debug_assert!(dest != NO_JUMP);
        if offset.unsigned_abs() > MAXARG_SBX as u32 {
            return Err(LuaError::Syntax("control structure too long".into()));
        }
        self.with_code(pc, |i| set_arg_sbx(i, offset));
        Ok(())
    }

    /// 制御命令（テスト命令）の位置を返す（本家 `getjumpcontrol`）。
    fn jump_control(&self, pc: i32) -> i32 {
        if pc >= 1 && self.code_at(pc - 1).opcode().is_some_and(is_test_mode) {
            return pc - 1;
        }
        pc
    }

    /// 本家 `patchtestreg`: TESTSET のターゲットレジスタを差し替え（または TEST 化）。
    fn patch_test_reg(&mut self, node: i32, reg: u32) -> bool {
        let ctrl = self.jump_control(node);
        let instr = self.code_at(ctrl);
        if instr.opcode() != Some(OpCode::TestSet) {
            return false;
        }
        if reg != NO_REG && reg != instr.b() {
            self.with_code(ctrl, |i| set_arg_a(i, reg));
        } else {
            // 値を置く先が無い → TEST へ置換。
            let b = instr.b();
            let c = instr.c();
            self.with_code(ctrl, |i| *i = Instruction::abc(OpCode::Test, b, 0, c));
        }
        true
    }

    fn remove_values(&mut self, mut list: i32) {
        while list != NO_JUMP {
            self.patch_test_reg(list, NO_REG);
            list = self.get_jump(list);
        }
    }

    fn patch_list_aux(
        &mut self,
        mut list: i32,
        vtarget: i32,
        reg: u32,
        dtarget: i32,
    ) -> LuaResult<()> {
        while list != NO_JUMP {
            let next = self.get_jump(list);
            if self.patch_test_reg(list, reg) {
                self.fix_jump(list, vtarget)?;
            } else {
                self.fix_jump(list, dtarget)?;
            }
            list = next;
        }
        Ok(())
    }

    fn discharge_jpc(&mut self) {
        let jpc = self.jpc;
        let pc = self.pc();
        // discharge_jpc は emit 直前に呼ばれ、保留ジャンプを次命令へ向ける。
        // patch_list_aux は emit しないので再帰は起きない。
        let _ = self.patch_list_aux(jpc, pc, NO_REG, pc);
        self.jpc = NO_JUMP;
    }

    fn patch_list(&mut self, list: i32, target: i32) -> LuaResult<()> {
        if target == self.pc() {
            self.patch_to_here(list)
        } else {
            self.patch_list_aux(list, target, NO_REG, target)
        }
    }

    fn patch_to_here(&mut self, list: i32) -> LuaResult<()> {
        self.lasttarget = self.pc();
        let mut jpc = self.jpc;
        self.concat_jump(&mut jpc, list)?;
        self.jpc = jpc;
        Ok(())
    }

    fn get_label(&mut self) -> i32 {
        self.lasttarget = self.pc();
        self.pc()
    }

    /// 本家 `luaK_concat`: ジャンプリスト `l1` の末尾に `l2` を連結。
    fn concat_jump(&mut self, l1: &mut i32, l2: i32) -> LuaResult<()> {
        if l2 == NO_JUMP {
            return Ok(());
        }
        if *l1 == NO_JUMP {
            *l1 = l2;
            return Ok(());
        }
        let mut list = *l1;
        loop {
            let next = self.get_jump(list);
            if next == NO_JUMP {
                break;
            }
            list = next;
        }
        self.fix_jump(list, l2)
    }

    /// 本家 `luaK_jump`: 保留 jpc を取り込んで JMP を出す。
    fn emit_jump(&mut self, line: u32) -> LuaResult<i32> {
        let saved = self.jpc;
        self.jpc = NO_JUMP;
        let mut j = self.emit_asbx(OpCode::Jmp, 0, NO_JUMP, line);
        self.concat_jump(&mut j, saved)?;
        Ok(j)
    }

    // ---- nil ロード最適化（本家 luaK_nil）---------------------------------

    fn code_nil(&mut self, from: u32, n: u32, line: u32) {
        if self.pc() > self.lasttarget {
            if self.pc() == 0 {
                if from >= self.nactvar() {
                    return; // 関数先頭は既に nil
                }
            } else {
                let prev_pc = self.pc() - 1;
                let prev = self.code_at(prev_pc);
                if prev.opcode() == Some(OpCode::LoadNil) {
                    let pfrom = prev.a();
                    let pto = prev.b();
                    if pfrom <= from && from <= pto + 1 {
                        if from + n - 1 > pto {
                            self.with_code(prev_pc, |i| set_arg_b(i, from + n - 1));
                        }
                        return;
                    }
                }
            }
        }
        self.emit_abc(OpCode::LoadNil, from, from + n - 1, 0, line);
    }

    // ---- expdesc の評価（本家 lcode.c）------------------------------------

    fn discharge_vars(&mut self, e: &mut ExpDesc, line: u32) {
        match e.k {
            EK::Local(reg) => {
                e.k = EK::NonReloc(reg);
            }
            EK::Upval(idx) => {
                let pc = self.emit_abc(OpCode::GetUpval, 0, idx, 0, line);
                e.k = EK::Reloc(pc as u32);
            }
            EK::Global(name_k) => {
                let pc = self.emit_abx(OpCode::GetGlobal, 0, name_k, line);
                e.k = EK::Reloc(pc as u32);
            }
            EK::Indexed { table, key } => {
                self.free_reg(key);
                self.free_reg(table);
                let pc = self.emit_abc(OpCode::GetTable, 0, table, key, line);
                e.k = EK::Reloc(pc as u32);
            }
            EK::Vararg(_) | EK::Call(_) => {
                self.set_one_ret(e);
            }
            _ => {}
        }
    }

    fn discharge2reg(&mut self, e: &mut ExpDesc, reg: u32, line: u32) {
        self.discharge_vars(e, line);
        match e.k {
            EK::Nil => self.code_nil(reg, 1, line),
            EK::False => {
                self.emit_abc(OpCode::LoadBool, reg, 0, 0, line);
            }
            EK::True => {
                self.emit_abc(OpCode::LoadBool, reg, 1, 0, line);
            }
            EK::K(idx) => {
                self.emit_abx(OpCode::LoadK, reg, idx, line);
            }
            EK::KNum(n) => {
                let idx = self.number_k(n);
                self.emit_abx(OpCode::LoadK, reg, idx, line);
            }
            EK::Reloc(pc) => {
                self.with_code(pc as i32, |i| set_arg_a(i, reg));
            }
            EK::NonReloc(r) => {
                if reg != r {
                    self.emit_abc(OpCode::Move, reg, r, 0, line);
                }
            }
            EK::Void | EK::Jmp(_) => return,
            _ => unreachable!("discharge_vars should have handled this kind"),
        }
        e.k = EK::NonReloc(reg);
    }

    fn discharge2anyreg(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        if !matches!(e.k, EK::NonReloc(_)) {
            self.reserve_regs(1)?;
            let r = self.freereg - 1;
            self.discharge2reg(e, r, line);
        }
        Ok(())
    }

    fn code_label(&mut self, a: u32, b: u32, jump: u32, line: u32) -> i32 {
        self.get_label();
        self.emit_abc(OpCode::LoadBool, a, b, jump, line)
    }

    fn need_value(&self, mut list: i32) -> bool {
        while list != NO_JUMP {
            let ctrl = self.jump_control(list);
            if self.code_at(ctrl).opcode() != Some(OpCode::TestSet) {
                return true;
            }
            list = self.get_jump(list);
        }
        false
    }

    fn exp2reg(&mut self, e: &mut ExpDesc, reg: u32, line: u32) -> LuaResult<()> {
        self.discharge2reg(e, reg, line);
        if let EK::Jmp(pc) = e.k {
            let mut t = e.t;
            self.concat_jump(&mut t, pc as i32)?;
            e.t = t;
        }
        if e.has_jumps() {
            let mut p_f = NO_JUMP;
            let mut p_t = NO_JUMP;
            if self.need_value(e.t) || self.need_value(e.f) {
                let fj = if matches!(e.k, EK::Jmp(_)) {
                    NO_JUMP
                } else {
                    self.emit_jump(line)?
                };
                p_f = self.code_label(reg, 0, 1, line);
                p_t = self.code_label(reg, 1, 0, line);
                self.patch_to_here(fj)?;
            }
            let fin = self.get_label();
            self.patch_list_aux(e.f, fin, reg, p_f)?;
            self.patch_list_aux(e.t, fin, reg, p_t)?;
        }
        e.f = NO_JUMP;
        e.t = NO_JUMP;
        e.k = EK::NonReloc(reg);
        Ok(())
    }

    fn exp2nextreg(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        self.discharge_vars(e, line);
        self.free_exp(e);
        self.reserve_regs(1)?;
        let r = self.freereg - 1;
        self.exp2reg(e, r, line)
    }

    fn exp2anyreg(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<u32> {
        self.discharge_vars(e, line);
        if let EK::NonReloc(r) = e.k {
            if !e.has_jumps() {
                return Ok(r);
            }
            if r >= self.nactvar() {
                self.exp2reg(e, r, line)?;
                return Ok(r);
            }
        }
        self.exp2nextreg(e, line)?;
        match e.k {
            EK::NonReloc(r) => Ok(r),
            _ => unreachable!(),
        }
    }

    fn exp2val(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        if e.has_jumps() {
            self.exp2anyreg(e, line)?;
        } else {
            self.discharge_vars(e, line);
        }
        Ok(())
    }

    /// 本家 `luaK_exp2RK`: 値をレジスタか定数（RK）として返す。
    ///
    /// 定数インデックスが MAXINDEXRK (= BITRK - 1 = 255) を超える場合は RK として
    /// 埋め込めない（B/C フィールドは 9bit だが最上位ビットが BITRK フラグのため
    /// 実質 8bit = 0..=255 しか定数インデックスに使えない）。本家 `luaK_exp2RK` と
    /// 同様、超過時は exp2anyreg でレジスタへ spill してレジスタ番号を返す。
    fn exp2rk(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<u32> {
        self.exp2val(e, line)?;
        match e.k {
            EK::Nil | EK::True | EK::False | EK::KNum(_) => {
                // 重複排除後の定数インデックスを先に求め、RK 範囲内かを確認する。
                let idx = match e.k {
                    EK::Nil => self.nil_k(),
                    EK::True => self.bool_k(true),
                    EK::False => self.bool_k(false),
                    EK::KNum(n) => self.number_k(n),
                    _ => unreachable!(),
                };
                if idx <= MAXINDEXRK {
                    e.k = EK::K(idx);
                    return Ok(rk_as_k(idx));
                }
                // インデックスが RK 範囲外 → 定数は既に登録済みなので
                // EK::K として LOADK 経由でレジスタへ落とす。
                e.k = EK::K(idx);
            }
            EK::K(idx) => {
                if idx <= MAXINDEXRK {
                    return Ok(rk_as_k(idx));
                }
                // idx > MAXINDEXRK: LOADK でレジスタへ spill させる（fall-through）。
            }
            _ => {}
        }
        self.exp2anyreg(e, line)
    }

    fn store_var(&mut self, var: &ExpDesc, ex: &mut ExpDesc, line: u32) -> LuaResult<()> {
        match var.k {
            EK::Local(reg) => {
                self.free_exp(ex);
                self.exp2reg(ex, reg, line)?;
                return Ok(());
            }
            EK::Upval(idx) => {
                let e = self.exp2anyreg(ex, line)?;
                self.emit_abc(OpCode::SetUpval, e, idx, 0, line);
            }
            EK::Global(name_k) => {
                let e = self.exp2anyreg(ex, line)?;
                self.emit_abx(OpCode::SetGlobal, e, name_k, line);
            }
            EK::Indexed { table, key } => {
                let e = self.exp2rk(ex, line)?;
                self.emit_abc(OpCode::SetTable, table, key, e, line);
            }
            _ => unreachable!("cannot store into this expression"),
        }
        self.free_exp(ex);
        Ok(())
    }

    /// 本家 `luaK_self`: メソッド呼び出しの SELF を出す。
    fn code_self(&mut self, e: &mut ExpDesc, key: &mut ExpDesc, line: u32) -> LuaResult<()> {
        self.exp2anyreg(e, line)?;
        let ereg = match e.k {
            EK::NonReloc(r) => r,
            _ => unreachable!(),
        };
        self.free_exp(e);
        let func = self.freereg;
        self.reserve_regs(2)?;
        let krk = self.exp2rk(key, line)?;
        self.emit_abc(OpCode::SelfOp, func, ereg, krk, line);
        self.free_exp(key);
        e.k = EK::NonReloc(func);
        Ok(())
    }

    /// 本家 `luaK_indexed`: `t[k]` を VINDEXED にする（t は既にレジスタ）。
    fn code_indexed(&mut self, t: &mut ExpDesc, k: &mut ExpDesc, line: u32) -> LuaResult<()> {
        let table = match t.k {
            EK::NonReloc(r) => r,
            _ => unreachable!("table must be in a register before indexing"),
        };
        let key = self.exp2rk(k, line)?;
        t.k = EK::Indexed { table, key };
        Ok(())
    }

    // ---- 条件分岐（本家 luaK_goif*）---------------------------------------

    fn invert_jump(&mut self, e: &ExpDesc) {
        let pc = match e.k {
            EK::Jmp(pc) => pc as i32,
            _ => unreachable!(),
        };
        let ctrl = self.jump_control(pc);
        let a = self.code_at(ctrl).a();
        self.with_code(ctrl, |i| set_arg_a(i, if a == 0 { 1 } else { 0 }));
    }

    fn cond_jump(&mut self, op: OpCode, a: u32, b: u32, c: u32, line: u32) -> LuaResult<i32> {
        self.emit_abc(op, a, b, c, line);
        self.emit_jump(line)
    }

    fn jump_on_cond(&mut self, e: &mut ExpDesc, cond: bool, line: u32) -> LuaResult<i32> {
        if let EK::Reloc(pc) = e.k {
            let ie = self.code_at(pc as i32);
            if ie.opcode() == Some(OpCode::Not) {
                // 直前の NOT を取り除き、条件を反転した TEST にする。
                self.proto.code.pop();
                self.proto.line_info.pop();
                let b = ie.b();
                return self.cond_jump(OpCode::Test, b, 0, (!cond) as u32, line);
            }
        }
        self.discharge2anyreg(e, line)?;
        self.free_exp(e);
        let einfo = match e.k {
            EK::NonReloc(r) => r,
            _ => unreachable!(),
        };
        self.cond_jump(OpCode::TestSet, NO_REG, einfo, cond as u32, line)
    }

    fn go_if_true(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        self.discharge_vars(e, line);
        let pc = match e.k {
            EK::K(_) | EK::KNum(_) | EK::True => NO_JUMP,
            EK::False => self.emit_jump(line)?,
            EK::Jmp(jpc) => {
                self.invert_jump(e);
                jpc as i32
            }
            _ => self.jump_on_cond(e, false, line)?,
        };
        let mut f = e.f;
        self.concat_jump(&mut f, pc)?;
        e.f = f;
        let t = e.t;
        self.patch_to_here(t)?;
        e.t = NO_JUMP;
        Ok(())
    }

    fn go_if_false(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        self.discharge_vars(e, line);
        let pc = match e.k {
            EK::Nil | EK::False => NO_JUMP,
            EK::True => self.emit_jump(line)?,
            EK::Jmp(jpc) => jpc as i32,
            _ => self.jump_on_cond(e, true, line)?,
        };
        let mut t = e.t;
        self.concat_jump(&mut t, pc)?;
        e.t = t;
        let f = e.f;
        self.patch_to_here(f)?;
        e.f = NO_JUMP;
        Ok(())
    }

    fn code_not(&mut self, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        self.discharge_vars(e, line);
        match e.k {
            EK::Nil | EK::False => e.k = EK::True,
            EK::K(_) | EK::KNum(_) | EK::True => e.k = EK::False,
            EK::Jmp(_) => self.invert_jump(e),
            EK::Reloc(_) | EK::NonReloc(_) => {
                self.discharge2anyreg(e, line)?;
                self.free_exp(e);
                let r = match e.k {
                    EK::NonReloc(r) => r,
                    _ => unreachable!(),
                };
                let pc = self.emit_abc(OpCode::Not, 0, r, 0, line);
                e.k = EK::Reloc(pc as u32);
            }
            _ => unreachable!(),
        }
        std::mem::swap(&mut e.f, &mut e.t);
        self.remove_values(e.f);
        self.remove_values(e.t);
        Ok(())
    }

    // ---- 算術・比較（本家 codearith / codecomp / constfolding）-------------

    fn const_folding(op: OpCode, e1: &mut ExpDesc, e2: &ExpDesc) -> bool {
        if !e1.is_numeral() || !e2.is_numeral() {
            return false;
        }
        let v1 = e1.num();
        let v2 = e2.num();
        let r = match op {
            OpCode::Add => v1 + v2,
            OpCode::Sub => v1 - v2,
            OpCode::Mul => v1 * v2,
            OpCode::Div => {
                if v2 == 0.0 {
                    return false;
                }
                v1 / v2
            }
            OpCode::Mod => {
                if v2 == 0.0 {
                    return false;
                }
                v1 - (v1 / v2).floor() * v2
            }
            OpCode::Pow => v1.powf(v2),
            OpCode::Unm => -v1,
            OpCode::Len => return false,
            _ => unreachable!(),
        };
        if r.is_nan() {
            return false;
        }
        e1.k = EK::KNum(r);
        true
    }

    fn code_arith(
        &mut self,
        op: OpCode,
        e1: &mut ExpDesc,
        e2: &mut ExpDesc,
        line: u32,
    ) -> LuaResult<()> {
        if Self::const_folding(op, e1, e2) {
            return Ok(());
        }
        let o2 = if op != OpCode::Unm && op != OpCode::Len {
            self.exp2rk(e2, line)?
        } else {
            0
        };
        let o1 = self.exp2rk(e1, line)?;
        if o1 > o2 {
            self.free_exp(e1);
            self.free_exp(e2);
        } else {
            self.free_exp(e2);
            self.free_exp(e1);
        }
        let pc = self.emit_abc(op, 0, o1, o2, line);
        e1.k = EK::Reloc(pc as u32);
        Ok(())
    }

    fn code_comp(
        &mut self,
        op: OpCode,
        cond: bool,
        e1: &mut ExpDesc,
        e2: &mut ExpDesc,
        line: u32,
    ) -> LuaResult<()> {
        let mut o1 = self.exp2rk(e1, line)?;
        let mut o2 = self.exp2rk(e2, line)?;
        self.free_exp(e2);
        self.free_exp(e1);
        let mut a = cond as u32;
        if !cond && op != OpCode::Eq {
            // `>`/`>=` は引数を入れ替えて `<`/`<=` に変換（A=1）。
            std::mem::swap(&mut o1, &mut o2);
            a = 1;
        }
        let pc = self.cond_jump(op, a, o1, o2, line)?;
        e1.k = EK::Jmp(pc as u32);
        Ok(())
    }

    // ---- 戻り値の調整（本家 luaK_setreturns / setoneret）-------------------

    fn set_returns(&mut self, e: &mut ExpDesc, nresults: i32) -> LuaResult<()> {
        match e.k {
            EK::Call(pc) => {
                self.with_code(pc as i32, |i| set_arg_c(i, (nresults + 1) as u32));
            }
            EK::Vararg(pc) => {
                let fr = self.freereg;
                self.with_code(pc as i32, |i| {
                    set_arg_b(i, (nresults + 1) as u32);
                    set_arg_a(i, fr);
                });
                self.reserve_regs(1)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn set_one_ret(&mut self, e: &mut ExpDesc) {
        match e.k {
            EK::Call(pc) => {
                let a = self.code_at(pc as i32).a();
                e.k = EK::NonReloc(a);
            }
            EK::Vararg(pc) => {
                self.with_code(pc as i32, |i| set_arg_b(i, 2));
                e.k = EK::Reloc(pc);
            }
            _ => {}
        }
    }

    fn set_multret(&mut self, e: &mut ExpDesc) -> LuaResult<()> {
        self.set_returns(e, MULTRET)
    }

    fn code_ret(&mut self, first: u32, nret: i32, line: u32) {
        self.emit_abc(OpCode::Return, first, (nret + 1) as u32, 0, line);
    }

    /// 本家 `luaK_setlist`: テーブルコンストラクタの配列部を書き込む。
    fn set_list(&mut self, base: u32, nelems: u32, tostore: i32, line: u32) {
        let c = (nelems - 1) / LFIELDS_PER_FLUSH + 1;
        let b = if tostore == MULTRET {
            0
        } else {
            tostore as u32
        };
        if c <= MAXARG_C {
            self.emit_abc(OpCode::SetList, base, b, c, line);
        } else {
            self.emit_abc(OpCode::SetList, base, b, 0, line);
            self.emit(Instruction::from_raw(c), line);
        }
        self.freereg = base + 1;
    }
}

// ============================================================================
// CodeGen ドライバ（本家 lparser.c の文・式生成に対応）
// ============================================================================

/// AST → `Proto` のコード生成器。関数ネストを `states` のスタックで表現する。
pub struct CodeGen<'h> {
    heap: &'h mut Heap,
    /// FuncState スタック（末尾が現在の関数）。
    states: Vec<FuncState>,
    /// チャンク名（`Proto::source`／エラー表示）。
    chunk: String,
}

impl<'h> CodeGen<'h> {
    /// チャンク（main 関数）をコンパイルする。
    pub fn compile(heap: &'h mut Heap, block: &Block, chunk: &str) -> LuaResult<Proto> {
        let mut cg = CodeGen {
            heap,
            states: Vec::new(),
            chunk: chunk.to_string(),
        };
        cg.open_func();
        // main 関数は常に vararg（本家 luaY_parser）。
        cg.cur().proto.is_vararg = true;
        cg.cur().proto.line_defined = 0;
        cg.cur().proto.last_line_defined = 0;
        cg.cur().proto.source = Some(chunk.to_string());
        cg.statements(block)?;
        let (proto, _upvals) = cg.close_func(0)?;
        Ok(proto)
    }

    fn cur(&mut self) -> &mut FuncState {
        self.states.last_mut().expect("a current function")
    }

    fn open_func(&mut self) {
        self.states.push(FuncState::new());
    }

    /// 関数を閉じ、最終 RETURN を出して `Proto` と upvalue 記述子を取り出す（本家 `close_func`）。
    fn close_func(&mut self, line: u32) -> LuaResult<(Proto, Vec<UpvalDesc>)> {
        self.remove_vars(0);
        let nactvar = self.cur().nactvar();
        self.cur().code_ret(nactvar, 0, line);
        let mut fs = self.states.pop().expect("a function to close");
        fs.proto.num_upvalues = fs.upvalues.len() as u8;
        fs.proto.upvalue_names = vec![String::new(); fs.upvalues.len()];
        Ok((fs.proto, fs.upvalues))
    }

    fn err(&self, msg: impl Into<String>) -> LuaError {
        LuaError::Syntax(msg.into())
    }

    // ---- 文字列定数（heap でインターン）-----------------------------------

    fn string_k(&mut self, bytes: &[u8]) -> u32 {
        let h = self.heap.intern_str(bytes);
        let fs = self.states.last_mut().unwrap();
        fs.add_constant(ConstKey::Str(bytes.to_vec()), Value::GcRef(h))
    }

    // ---- スコープ・ローカル変数（本家 lparser.c）--------------------------

    /// 新しいローカルを登録（まだ有効化しない）。本家 `new_localvar`。
    fn new_localvar(&mut self, name: &str) -> LuaResult<()> {
        let fs = self.cur();
        if fs.actives.len() + fs.pending.len() + 1 > MAX_VARS {
            return Err(LuaError::Syntax("too many local variables".into()));
        }
        let locvar_idx = fs.proto.local_vars.len();
        fs.proto.local_vars.push(LocalVar {
            name: name.to_string(),
            start_pc: 0,
            end_pc: 0,
        });
        fs.pending.push((name.to_string(), locvar_idx));
        Ok(())
    }

    /// 先頭 `n` 個の保留ローカルを有効化（本家 `adjustlocalvars`）。
    fn adjust_localvars(&mut self, n: usize) {
        let fs = self.cur();
        let pc = fs.proto.code.len() as u32;
        for _ in 0..n {
            let (name, locvar_idx) = fs.pending.remove(0);
            fs.proto.local_vars[locvar_idx].start_pc = pc;
            fs.actives.push(ActiveLocal { name, locvar_idx });
        }
    }

    /// 有効ローカルを `tolevel` 個まで減らす（本家 `removevars`）。
    fn remove_vars(&mut self, tolevel: usize) {
        let fs = self.cur();
        let pc = fs.proto.code.len() as u32;
        while fs.actives.len() > tolevel {
            let al = fs.actives.pop().unwrap();
            fs.proto.local_vars[al.locvar_idx].end_pc = pc;
        }
    }

    fn enter_block(&mut self, is_loop: bool) {
        let nactvar = self.cur().actives.len();
        self.cur().blocks.push(BlockCnt {
            is_loop,
            nactvar,
            has_upval: false,
            break_list: NO_JUMP,
        });
    }

    fn leave_block(&mut self, line: u32) -> LuaResult<()> {
        let bl = self.cur().blocks.pop().expect("a block to leave");
        self.remove_vars(bl.nactvar);
        if bl.has_upval {
            self.cur()
                .emit_abc(OpCode::Close, bl.nactvar as u32, 0, 0, line);
        }
        self.cur().freereg = bl.nactvar as u32;
        self.cur().patch_to_here(bl.break_list)?;
        Ok(())
    }

    // ---- 変数解決（本家 singlevaraux / indexupvalue / markupval）----------

    /// `level`（1 始まり, `states[level-1]`）の関数で名前を解決する。本家 `singlevaraux`。
    /// 戻り値が `EK::Global` のとき呼び出し側が名前定数を設定する。
    fn single_var_aux(&mut self, level: usize, name: &str, base: bool) -> EK {
        if level == 0 {
            return EK::Global(0); // これ以上外側が無い → グローバル
        }
        // searchvar: 現在レベルの有効ローカルを後ろから探す。
        let found = self.states[level - 1]
            .actives
            .iter()
            .rposition(|a| a.name == name)
            .map(|i| i as u32);
        if let Some(reg) = found {
            if !base {
                self.mark_upval(level - 1, reg);
            }
            return EK::Local(reg);
        }
        // 上位レベルを探す。見つかれば現在レベルに upvalue を作る。
        match self.single_var_aux(level - 1, name, false) {
            EK::Global(_) => EK::Global(0),
            EK::Local(reg) => EK::Upval(self.index_upvalue(level - 1, name, true, reg)),
            EK::Upval(uidx) => EK::Upval(self.index_upvalue(level - 1, name, false, uidx)),
            _ => unreachable!(),
        }
    }

    /// 名前を変数式として解決（本家 `singlevar`）。
    fn single_var(&mut self, name: &str) -> EK {
        let level = self.states.len();
        let k = self.single_var_aux(level, name, true);
        if matches!(k, EK::Global(_)) {
            let name_k = self.string_k(name.as_bytes());
            EK::Global(name_k)
        } else {
            k
        }
    }

    /// `fs_level` 番目の関数に upvalue を登録（重複排除）。本家 `indexupvalue`。
    fn index_upvalue(&mut self, fs_level: usize, _name: &str, from_local: bool, index: u32) -> u32 {
        let fs = &mut self.states[fs_level];
        for (i, uv) in fs.upvalues.iter().enumerate() {
            if uv.from_local == from_local && uv.index == index {
                return i as u32;
            }
        }
        let idx = fs.upvalues.len() as u32;
        // 上限チェックは呼び出し側方針に委ねる（MAX_UPVALUES）。
        debug_assert!(fs.upvalues.len() < MAX_UPVALUES);
        fs.upvalues.push(UpvalDesc { from_local, index });
        idx
    }

    /// レジスタ `reg` のローカルを含むブロックを upvalue 持ちにマーク。本家 `markupval`。
    fn mark_upval(&mut self, fs_level: usize, reg: u32) {
        let fs = &mut self.states[fs_level];
        for bl in fs.blocks.iter_mut().rev() {
            if (bl.nactvar as u32) > reg {
                continue;
            }
            bl.has_upval = true;
            break;
        }
    }

    // ---- 文の生成 ----------------------------------------------------------

    fn statements(&mut self, block: &Block) -> LuaResult<()> {
        for stmt in &block.stmts {
            self.statement(stmt)?;
            // 本家 statement() 末尾と同様、文ごとに一時レジスタを解放する。
            let fs = self.cur();
            debug_assert!(fs.freereg >= fs.nactvar());
            fs.freereg = fs.nactvar();
        }
        Ok(())
    }

    /// ネストしたブロック（本家 `block`）。
    fn block(&mut self, block: &Block) -> LuaResult<()> {
        self.enter_block(false);
        self.statements(block)?;
        self.leave_block(0)?;
        Ok(())
    }

    fn statement(&mut self, stmt: &Stmt) -> LuaResult<()> {
        let line = stmt.line;
        match &stmt.kind {
            StmtKind::Local { names, exprs } => self.local_stat(names, exprs, line),
            StmtKind::LocalFunction { name, body } => self.local_function(name, body, line),
            StmtKind::Assign { targets, exprs } => self.assign_stat(targets, exprs, line),
            StmtKind::ExprStat(e) => self.expr_stat(e, line),
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => self.while_stat(cond, body, line),
            StmtKind::Repeat { body, cond } => self.repeat_stat(body, cond, line),
            StmtKind::If { arms, else_block } => self.if_stat(arms, else_block.as_ref(), line),
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => self.numeric_for(var, start, limit, step.as_ref(), body, line),
            StmtKind::GenericFor { names, exprs, body } => {
                self.generic_for(names, exprs, body, line)
            }
            StmtKind::Function { name, body } => self.func_stat(name, body, line),
            StmtKind::Return(exprs) => self.return_stat(exprs, line),
            StmtKind::Break => self.break_stat(line),
        }
    }

    /// `local a, b = e1, e2`。
    fn local_stat(&mut self, names: &[String], exprs: &[Expr], line: u32) -> LuaResult<()> {
        for n in names {
            self.new_localvar(n)?;
        }
        let nvars = names.len() as i32;
        if exprs.is_empty() {
            let mut e = ExpDesc::new(EK::Void);
            self.adjust_assign(nvars, 0, &mut e, line)?;
        } else {
            let (nexps, mut e) = self.explist(exprs, line)?;
            self.adjust_assign(nvars, nexps, &mut e, line)?;
        }
        self.adjust_localvars(names.len());
        Ok(())
    }

    /// `local function f() ... end`。
    fn local_function(&mut self, name: &str, body: &FuncBody, line: u32) -> LuaResult<()> {
        self.new_localvar(name)?;
        let reg = self.cur().freereg;
        let var = ExpDesc::new(EK::Local(reg));
        self.cur().reserve_regs(1)?;
        self.adjust_localvars(1);
        let mut b = self.func_body(body)?;
        self.cur().store_var(&var, &mut b, line)?;
        // この時点以降でデバッグ情報に現れる（startpc 更新）。
        let fs = self.cur();
        let pc = fs.proto.code.len() as u32;
        if let Some(al) = fs.actives.last() {
            let idx = al.locvar_idx;
            fs.proto.local_vars[idx].start_pc = pc;
        }
        Ok(())
    }

    /// `function a.b.c:m() ... end`。
    fn func_stat(&mut self, name: &FuncName, body: &FuncBody, line: u32) -> LuaResult<()> {
        let mut v = self.func_name(name, line)?;
        let mut b = self.func_body(body)?;
        self.cur().store_var(&v, &mut b, line)?;
        self.cur().fixline(line);
        // v は使い終わり。借用警告回避。
        let _ = &mut v;
        Ok(())
    }

    fn func_name(&mut self, name: &FuncName, line: u32) -> LuaResult<ExpDesc> {
        let mut v = self.resolve_name(&name.base);
        for field in &name.fields {
            v = self.index_with_name(v, field, line)?;
        }
        if let Some(m) = &name.method {
            v = self.index_with_name(v, m, line)?;
        }
        Ok(v)
    }

    /// `lhs1, lhs2 = e1, e2`。
    fn assign_stat(&mut self, targets: &[Expr], exprs: &[Expr], line: u32) -> LuaResult<()> {
        // 左辺を順に評価して expdesc 化。VLOCAL のとき競合検査。
        let mut lhs: Vec<ExpDesc> = Vec::with_capacity(targets.len());
        for (i, t) in targets.iter().enumerate() {
            let v = self.compile_lvalue(t)?;
            if i > 0
                && let EK::Local(reg) = v.k
            {
                self.check_conflict(&mut lhs, reg, line)?;
            }
            lhs.push(v);
        }
        let nvars = targets.len() as i32;
        let (nexps, mut e) = self.explist(exprs, line)?;

        if nexps != nvars {
            self.adjust_assign(nvars, nexps, &mut e, line)?;
            if nexps > nvars {
                self.cur().freereg -= (nexps - nvars) as u32;
            }
            // 全ターゲットを後ろから既定レジスタで代入。
            for v in lhs.iter().rev() {
                let fr = self.cur().freereg;
                let mut def = ExpDesc::new(EK::NonReloc(fr - 1));
                self.cur().store_var(v, &mut def, line)?;
            }
        } else {
            // 個数一致: 最後のターゲットに最後の式を直接代入。
            self.cur().set_one_ret(&mut e);
            let last = lhs.last().unwrap();
            self.cur().store_var(last, &mut e, line)?;
            // 残りを後ろから既定レジスタで代入。
            for v in lhs[..lhs.len() - 1].iter().rev() {
                let fr = self.cur().freereg;
                let mut def = ExpDesc::new(EK::NonReloc(fr - 1));
                self.cur().store_var(v, &mut def, line)?;
            }
        }
        Ok(())
    }

    /// 本家 `check_conflict`: 既存ターゲットの table/key レジスタが新ローカルと衝突したら退避。
    fn check_conflict(&mut self, lhs: &mut [ExpDesc], reg: u32, line: u32) -> LuaResult<()> {
        let extra = self.cur().freereg;
        let mut conflict = false;
        for v in lhs.iter_mut() {
            if let EK::Indexed { table, key } = &mut v.k {
                if *table == reg {
                    conflict = true;
                    *table = extra;
                }
                if *key == reg {
                    conflict = true;
                    *key = extra;
                }
            }
        }
        if conflict {
            self.cur().emit_abc(OpCode::Move, extra, reg, 0, line);
            self.cur().reserve_regs(1)?;
        }
        Ok(())
    }

    /// 式文（関数呼び出し）。
    fn expr_stat(&mut self, e: &Expr, _line: u32) -> LuaResult<()> {
        let v = self.expr_node(e)?;
        match v.k {
            EK::Call(pc) => {
                // 結果を使わない呼び出し（C=1）。
                self.cur().with_code(pc as i32, |i| set_arg_c(i, 1));
            }
            _ => return Err(self.err("syntax error")),
        }
        Ok(())
    }

    fn return_stat(&mut self, exprs: &[Expr], line: u32) -> LuaResult<()> {
        if exprs.is_empty() {
            self.cur().code_ret(0, 0, line);
            return Ok(());
        }
        let (nret, mut e) = self.explist(exprs, line)?;
        let (first, count) = if Self::has_multret(&e) {
            self.cur().set_multret(&mut e)?;
            if matches!(e.k, EK::Call(_)) && nret == 1 {
                // 末尾呼び出し化。
                if let EK::Call(pc) = e.k {
                    self.cur()
                        .with_code(pc as i32, |i| set_opcode_keep_args(i, OpCode::TailCall));
                }
            }
            (self.cur().nactvar(), MULTRET)
        } else if nret == 1 {
            let r = self.cur().exp2anyreg(&mut e, line)?;
            (r, 1)
        } else {
            self.cur().exp2nextreg(&mut e, line)?;
            (self.cur().nactvar(), nret)
        };
        self.cur().code_ret(first, count, line);
        Ok(())
    }

    fn break_stat(&mut self, line: u32) -> LuaResult<()> {
        // 最も内側の break 可能ブロックを探す。途中の upval を集計。
        let fs = self.cur();
        let mut idx = None;
        let mut upval = false;
        for (i, bl) in fs.blocks.iter().enumerate().rev() {
            if bl.is_loop {
                idx = Some(i);
                break;
            }
            upval |= bl.has_upval;
        }
        let idx = idx.ok_or_else(|| self.err("no loop to break"))?;
        if upval {
            let nactvar = self.cur().blocks[idx].nactvar as u32;
            self.cur().emit_abc(OpCode::Close, nactvar, 0, 0, line);
        }
        let j = self.cur().emit_jump(line)?;
        let mut bl_break = self.cur().blocks[idx].break_list;
        self.cur().concat_jump(&mut bl_break, j)?;
        self.cur().blocks[idx].break_list = bl_break;
        Ok(())
    }

    // ---- 制御構造 ----------------------------------------------------------

    /// 条件式をコンパイルし、偽脱出リストを返す（本家 `cond`）。
    fn cond(&mut self, e: &Expr, line: u32) -> LuaResult<i32> {
        let mut v = self.expr_node(e)?;
        if matches!(v.k, EK::Nil) {
            v.k = EK::False; // nil も false 扱い
        }
        self.cur().go_if_true(&mut v, line)?;
        Ok(v.f)
    }

    fn if_stat(
        &mut self,
        arms: &[(Expr, Block)],
        else_block: Option<&Block>,
        line: u32,
    ) -> LuaResult<()> {
        let mut escape = NO_JUMP;
        let mut flist;
        // 最初の if アーム。
        let (first_cond, first_body) = &arms[0];
        flist = self.cond(
            first_cond,
            first_body.stmts.first().map(|s| s.line).unwrap_or(line),
        )?;
        self.block(first_body)?;

        for (cond_e, body) in &arms[1..] {
            let j = self.cur().emit_jump(line)?;
            self.cur().concat_jump(&mut escape, j)?;
            self.cur().patch_to_here(flist)?;
            flist = self.cond(cond_e, line)?;
            self.block(body)?;
        }

        if let Some(eb) = else_block {
            let j = self.cur().emit_jump(line)?;
            self.cur().concat_jump(&mut escape, j)?;
            self.cur().patch_to_here(flist)?;
            self.block(eb)?;
        } else {
            self.cur().concat_jump(&mut escape, flist)?;
        }
        self.cur().patch_to_here(escape)?;
        Ok(())
    }

    fn while_stat(&mut self, cond: &Expr, body: &Block, line: u32) -> LuaResult<()> {
        let while_init = self.cur().get_label();
        let cond_exit = self.cond(cond, line)?;
        self.enter_block(true);
        self.block(body)?;
        let j = self.cur().emit_jump(line)?;
        self.cur().patch_list(j, while_init)?;
        self.leave_block(line)?;
        self.cur().patch_to_here(cond_exit)?;
        Ok(())
    }

    fn repeat_stat(&mut self, body: &Block, cond: &Expr, line: u32) -> LuaResult<()> {
        let repeat_init = self.cur().get_label();
        self.enter_block(true); // ループブロック
        self.enter_block(false); // スコープブロック
        self.statements(body)?;
        // 条件はスコープブロック内で評価。
        let cond_exit = self.cond(cond, line)?;
        let inner_upval = self.cur().blocks.last().unwrap().has_upval;
        if !inner_upval {
            self.leave_block(line)?; // スコープ終了
            self.cur().patch_list(cond_exit, repeat_init)?;
        } else {
            // upvalue があるとき: 条件成立で break、不成立でループ先頭へ。
            self.break_stat(line)?;
            self.cur().patch_to_here(cond_exit)?;
            self.leave_block(line)?;
            let j = self.cur().emit_jump(line)?;
            self.cur().patch_list(j, repeat_init)?;
        }
        self.leave_block(line)?; // ループ終了
        Ok(())
    }

    fn numeric_for(
        &mut self,
        var: &str,
        start: &Expr,
        limit: &Expr,
        step: Option<&Expr>,
        body: &Block,
        line: u32,
    ) -> LuaResult<()> {
        // 本家 forstat: ループ全体（制御変数を含む）を break 可能ブロックで囲む。
        // break はこのブロックの末尾（ループ脱出点）へジャンプする。
        self.enter_block(true);
        let base = self.cur().freereg;
        self.new_localvar("(for index)")?;
        self.new_localvar("(for limit)")?;
        self.new_localvar("(for step)")?;
        self.new_localvar(var)?;
        // 初期値・上限・刻み。
        let mut e = self.expr_node(start)?;
        self.cur().exp2nextreg(&mut e, line)?;
        let mut e = self.expr_node(limit)?;
        self.cur().exp2nextreg(&mut e, line)?;
        if let Some(s) = step {
            let mut e = self.expr_node(s)?;
            self.cur().exp2nextreg(&mut e, line)?;
        } else {
            let fr = self.cur().freereg;
            let one = self.cur().number_k(1.0);
            self.cur().emit_abx(OpCode::LoadK, fr, one, line);
            self.cur().reserve_regs(1)?;
        }
        self.for_body(base, 1, true, body, line)?;
        self.leave_block(line) // break はここに着地
    }

    fn generic_for(
        &mut self,
        names: &[String],
        exprs: &[Expr],
        body: &Block,
        line: u32,
    ) -> LuaResult<()> {
        // 本家 forstat: ループ全体を break 可能ブロックで囲む。
        self.enter_block(true);
        let base = self.cur().freereg;
        self.new_localvar("(for generator)")?;
        self.new_localvar("(for state)")?;
        self.new_localvar("(for control)")?;
        for n in names {
            self.new_localvar(n)?;
        }
        let (nexps, mut e) = self.explist(exprs, line)?;
        self.adjust_assign(3, nexps, &mut e, line)?;
        self.cur().check_stack(3)?; // ジェネレータ呼び出し用の余分。
        self.for_body(base, names.len() as i32, false, body, line)?;
        self.leave_block(line) // break はここに着地
    }

    fn for_body(
        &mut self,
        base: u32,
        nvars: i32,
        is_num: bool,
        body: &Block,
        line: u32,
    ) -> LuaResult<()> {
        self.adjust_localvars(3); // 制御変数。
        let prep = if is_num {
            self.cur().emit_asbx(OpCode::ForPrep, base, NO_JUMP, line)
        } else {
            self.cur().emit_jump(line)?
        };
        self.enter_block(false); // 宣言変数のスコープ。
        self.adjust_localvars(nvars as usize);
        self.cur().reserve_regs(nvars as u32)?;
        self.block(body)?;
        self.leave_block(line)?;
        self.cur().patch_to_here(prep)?;
        let end_for = if is_num {
            self.cur().emit_asbx(OpCode::ForLoop, base, NO_JUMP, line)
        } else {
            self.cur()
                .emit_abc(OpCode::TForLoop, base, 0, nvars as u32, line)
        };
        self.cur().fixline(line);
        if is_num {
            self.cur().patch_list(end_for, prep + 1)?;
        } else {
            let j = self.cur().emit_jump(line)?;
            self.cur().patch_list(j, prep + 1)?;
        }
        Ok(())
    }

    // ---- 式リスト・代入調整 ------------------------------------------------

    /// 本家 `explist1`: 最後以外を順に nextreg へ、最後は ExpDesc のまま返す。
    fn explist(&mut self, exprs: &[Expr], line: u32) -> LuaResult<(i32, ExpDesc)> {
        debug_assert!(!exprs.is_empty());
        let mut e = self.expr_node(&exprs[0])?;
        for ex in &exprs[1..] {
            self.cur().exp2nextreg(&mut e, line)?;
            e = self.expr_node(ex)?;
        }
        Ok((exprs.len() as i32, e))
    }

    fn has_multret(e: &ExpDesc) -> bool {
        matches!(e.k, EK::Call(_) | EK::Vararg(_))
    }

    /// 本家 `adjust_assign`: 変数の数と式の数を揃える。
    fn adjust_assign(
        &mut self,
        nvars: i32,
        nexps: i32,
        e: &mut ExpDesc,
        line: u32,
    ) -> LuaResult<()> {
        let mut extra = nvars - nexps;
        if Self::has_multret(e) {
            extra += 1;
            if extra < 0 {
                extra = 0;
            }
            self.cur().set_returns(e, extra)?;
            if extra > 1 {
                self.cur().reserve_regs((extra - 1) as u32)?;
            }
        } else {
            if !matches!(e.k, EK::Void) {
                self.cur().exp2nextreg(e, line)?;
            }
            if extra > 0 {
                let reg = self.cur().freereg;
                self.cur().reserve_regs(extra as u32)?;
                self.cur().code_nil(reg, extra as u32, line);
            }
        }
        Ok(())
    }

    // ---- 関数本体 ----------------------------------------------------------

    fn func_body(&mut self, body: &FuncBody) -> LuaResult<ExpDesc> {
        self.open_func();
        self.cur().proto.line_defined = body.line;
        self.cur().proto.source = Some(self.chunk.clone());
        // 仮引数。
        for p in &body.params {
            self.new_localvar(p)?;
        }
        self.adjust_localvars(body.params.len());
        let nparams = self.cur().nactvar();
        self.cur().proto.num_params = nparams as u8;
        self.cur().reserve_regs(nparams)?;
        self.cur().proto.is_vararg = body.is_vararg;
        self.statements(&body.body)?;
        let (proto, upvals) = self.close_func(body.last_line)?;
        // 親へ proto を登録し CLOSURE を出す。
        self.push_closure(proto, upvals, body.line)
    }

    /// 本家 `pushclosure`: CLOSURE + upvalue 捕捉疑似命令を出す。
    fn push_closure(
        &mut self,
        proto: Proto,
        upvals: Vec<UpvalDesc>,
        line: u32,
    ) -> LuaResult<ExpDesc> {
        let idx = {
            let parent = self.cur();
            let i = parent.proto.protos.len() as u32;
            parent.proto.protos.push(Rc::new(proto));
            i
        };
        let pc = self.cur().emit_abx(OpCode::Closure, 0, idx, line);
        for uv in &upvals {
            let op = if uv.from_local {
                OpCode::Move
            } else {
                OpCode::GetUpval
            };
            self.cur().emit_abc(op, 0, uv.index, 0, line);
        }
        Ok(ExpDesc::new(EK::Reloc(pc as u32)))
    }
}

// ============================================================================
// 式のコード生成（本家 lparser.c の expr / primaryexp / lcode.c の prefix/infix/posfix）
// ============================================================================

impl<'h> CodeGen<'h> {
    /// 名前を変数参照式として解決する。
    fn resolve_name(&mut self, name: &str) -> ExpDesc {
        ExpDesc::new(self.single_var(name))
    }

    /// `obj.name` をテーブル添字式にする（obj は任意の式）。
    fn index_with_name(&mut self, mut obj: ExpDesc, name: &str, line: u32) -> LuaResult<ExpDesc> {
        self.cur().exp2anyreg(&mut obj, line)?;
        let mut key = ExpDesc::new(EK::K(self.string_k(name.as_bytes())));
        self.cur().code_indexed(&mut obj, &mut key, line)?;
        Ok(obj)
    }

    /// 代入の左辺（Name か Index）を ExpDesc 化（discharge はしない）。
    fn compile_lvalue(&mut self, e: &Expr) -> LuaResult<ExpDesc> {
        match &e.kind {
            ExprKind::Name(_) | ExprKind::Index { .. } => self.expr_node(e),
            _ => Err(self.err("cannot be assigned to")),
        }
    }

    /// 式を ExpDesc にコンパイルする（本家 expr → subexpr → simpleexp/primaryexp）。
    fn expr_node(&mut self, e: &Expr) -> LuaResult<ExpDesc> {
        let line = e.line;
        match &e.kind {
            ExprKind::Nil => Ok(ExpDesc::new(EK::Nil)),
            ExprKind::True => Ok(ExpDesc::new(EK::True)),
            ExprKind::False => Ok(ExpDesc::new(EK::False)),
            ExprKind::Number(n) => Ok(ExpDesc::new(EK::KNum(*n))),
            ExprKind::Str(s) => {
                let idx = self.string_k(s);
                Ok(ExpDesc::new(EK::K(idx)))
            }
            ExprKind::Vararg => {
                if !self.cur().proto.is_vararg {
                    return Err(self.err("cannot use '...' outside a vararg function"));
                }
                let pc = self.cur().emit_abc(OpCode::Vararg, 0, 1, 0, line);
                Ok(ExpDesc::new(EK::Vararg(pc as u32)))
            }
            ExprKind::Name(n) => Ok(self.resolve_name(n)),
            ExprKind::Index { obj, key } => {
                let mut o = self.expr_node(obj)?;
                self.cur().exp2anyreg(&mut o, line)?;
                let mut k = self.expr_node(key)?;
                self.cur().code_indexed(&mut o, &mut k, line)?;
                Ok(o)
            }
            ExprKind::Call { func, args } => self.code_call(func, args, line),
            ExprKind::MethodCall { obj, method, args } => self.code_method(obj, method, args, line),
            ExprKind::Function(body) => self.func_body(body),
            ExprKind::Table(fields) => self.constructor(fields, line),
            ExprKind::BinOp { op, lhs, rhs } => self.code_binop(*op, lhs, rhs, line),
            ExprKind::UnOp { op, expr } => self.code_unop(*op, expr, line),
            ExprKind::Paren(inner) => {
                let mut v = self.expr_node(inner)?;
                self.cur().discharge_vars(&mut v, line);
                Ok(v)
            }
        }
    }

    fn code_binop(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, line: u32) -> LuaResult<ExpDesc> {
        let mut e1 = self.expr_node(lhs)?;
        self.cur().infix(op, &mut e1, line)?;
        let mut e2 = self.expr_node(rhs)?;
        self.cur().posfix(op, &mut e1, &mut e2, line)?;
        Ok(e1)
    }

    fn code_unop(&mut self, op: UnOp, operand: &Expr, line: u32) -> LuaResult<ExpDesc> {
        let mut e = self.expr_node(operand)?;
        self.cur().prefix(op, &mut e, line)?;
        Ok(e)
    }

    /// `func(args)`。
    fn code_call(&mut self, func: &Expr, args: &[Expr], line: u32) -> LuaResult<ExpDesc> {
        let mut f = self.expr_node(func)?;
        self.cur().exp2nextreg(&mut f, line)?;
        let base = match f.k {
            EK::NonReloc(r) => r,
            _ => unreachable!(),
        };
        let nparams = self.code_args(args, base, line)?;
        let pc = self
            .cur()
            .emit_abc(OpCode::Call, base, (nparams + 1) as u32, 2, line);
        self.cur().fixline(line);
        self.cur().freereg = base + 1;
        Ok(ExpDesc::new(EK::Call(pc as u32)))
    }

    /// `obj:method(args)`。
    fn code_method(
        &mut self,
        obj: &Expr,
        method: &str,
        args: &[Expr],
        line: u32,
    ) -> LuaResult<ExpDesc> {
        let mut e = self.expr_node(obj)?;
        let mut key = ExpDesc::new(EK::K(self.string_k(method.as_bytes())));
        self.cur().code_self(&mut e, &mut key, line)?;
        let base = match e.k {
            EK::NonReloc(r) => r,
            _ => unreachable!(),
        };
        let nparams = self.code_args(args, base, line)?;
        let pc = self
            .cur()
            .emit_abc(OpCode::Call, base, (nparams + 1) as u32, 2, line);
        self.cur().fixline(line);
        self.cur().freereg = base + 1;
        Ok(ExpDesc::new(EK::Call(pc as u32)))
    }

    /// 呼び出し引数を評価し、引数個数（multret なら [`MULTRET`]）を返す。本家 `funcargs`。
    fn code_args(&mut self, args: &[Expr], base: u32, line: u32) -> LuaResult<i32> {
        if args.is_empty() {
            // NOTE(lua-stdlib→lua-frontend): 引数なしでも param 数は freereg から算出する
            // （本家 `funcargs` の VVOID 経路）。メソッド呼び出し `o:m()` では SELF が
            // self を base+1 に積むため、ここを 0 固定にすると self が渡らない
            // （`o:m()` 系が全滅する）。プレーン呼び出しでは freereg==base+1 で 0 になる。
            return Ok((self.cur().freereg - (base + 1)) as i32);
        }
        let (_, mut last) = self.explist(args, line)?;
        if Self::has_multret(&last) {
            self.cur().set_multret(&mut last)?;
            Ok(MULTRET)
        } else {
            self.cur().exp2nextreg(&mut last, line)?;
            Ok((self.cur().freereg - (base + 1)) as i32)
        }
    }

    /// テーブルコンストラクタ `{ ... }`（本家 `constructor`）。
    fn constructor(&mut self, fields: &[Field], line: u32) -> LuaResult<ExpDesc> {
        let pc = self.cur().emit_abc(OpCode::NewTable, 0, 0, 0, line);
        let mut t = ExpDesc::new(EK::Reloc(pc as u32));
        self.cur().exp2nextreg(&mut t, line)?; // テーブルをレジスタに固定（GC のため）
        let treg = match t.k {
            EK::NonReloc(r) => r,
            _ => unreachable!(),
        };
        let mut na: u32 = 0; // 配列要素数
        let mut nh: u32 = 0; // ハッシュ要素数
        let mut tostore: i32 = 0; // 未フラッシュの配列要素数
        let mut pending: Option<ExpDesc> = None; // 直近の配列要素（本家 cc.v）

        for field in fields {
            // closelistfield: 直前の配列要素をレジスタへ確定し、必要ならフラッシュ。
            if let Some(mut v) = pending.take() {
                self.cur().exp2nextreg(&mut v, line)?;
                if tostore == LFIELDS_PER_FLUSH as i32 {
                    self.cur().set_list(treg, na, tostore, line);
                    tostore = 0;
                }
            }
            match field {
                Field::Positional(e) => {
                    let v = self.expr_node(e)?;
                    na += 1;
                    tostore += 1;
                    pending = Some(v);
                }
                Field::Named(name, e) => {
                    nh += 1;
                    let reg_save = self.cur().freereg;
                    // キーは文字列定数。定数インデックスが MAXINDEXRK を超える場合は
                    // rk_as_k を直接使えない（B/C は 9bit, BITRK = 256 がフラグ bit
                    // のため定数インデックスは 0..=255 のみ）。本家 luaK_exp2RK と
                    // 同様に exp2rk 経由で spill させる。
                    let key_idx = self.string_k(name.as_bytes());
                    let mut key_e = ExpDesc::new(EK::K(key_idx));
                    let key_rk = self.cur().exp2rk(&mut key_e, line)?;
                    let mut val = self.expr_node(e)?;
                    let val_rk = self.cur().exp2rk(&mut val, line)?;
                    self.cur()
                        .emit_abc(OpCode::SetTable, treg, key_rk, val_rk, line);
                    self.cur().freereg = reg_save;
                }
                Field::Keyed(k, v) => {
                    nh += 1;
                    let reg_save = self.cur().freereg;
                    let mut key = self.expr_node(k)?;
                    let key_rk = self.cur().exp2rk(&mut key, line)?;
                    let mut val = self.expr_node(v)?;
                    let val_rk = self.cur().exp2rk(&mut val, line)?;
                    self.cur()
                        .emit_abc(OpCode::SetTable, treg, key_rk, val_rk, line);
                    self.cur().freereg = reg_save;
                }
            }
        }
        // lastlistfield: 残った配列要素をフラッシュ。
        if let Some(mut v) = pending {
            if Self::has_multret(&v) {
                self.cur().set_multret(&mut v)?;
                self.cur().set_list(treg, na, MULTRET, line);
                na -= 1; // 末尾の可変長分は数えない
            } else {
                self.cur().exp2nextreg(&mut v, line)?;
                self.cur().set_list(treg, na, tostore, line);
            }
        } else if tostore != 0 {
            self.cur().set_list(treg, na, tostore, line);
        }
        // NEWTABLE のサイズヒントを設定。
        self.cur().with_code(pc, |i| {
            set_arg_b(i, int2fb(na));
            set_arg_c(i, int2fb(nh));
        });
        Ok(t)
    }
}

impl FuncState {
    /// 本家 `luaK_infix`: 二項演算の左オペランドを準備（右オペランド評価の前に呼ぶ）。
    fn infix(&mut self, op: BinOp, v: &mut ExpDesc, line: u32) -> LuaResult<()> {
        match op {
            BinOp::And => self.go_if_true(v, line),
            BinOp::Or => self.go_if_false(v, line),
            BinOp::Concat => self.exp2nextreg(v, line),
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                if !v.is_numeral() {
                    self.exp2rk(v, line)?;
                }
                Ok(())
            }
            // 比較演算。
            _ => {
                self.exp2rk(v, line)?;
                Ok(())
            }
        }
    }

    /// 本家 `luaK_posfix`: 二項演算の最終コード生成。
    fn posfix(
        &mut self,
        op: BinOp,
        e1: &mut ExpDesc,
        e2: &mut ExpDesc,
        line: u32,
    ) -> LuaResult<()> {
        match op {
            BinOp::And => {
                debug_assert_eq!(e1.t, NO_JUMP);
                self.discharge_vars(e2, line);
                let mut f = e2.f;
                self.concat_jump(&mut f, e1.f)?;
                e2.f = f;
                *e1 = e2.clone();
            }
            BinOp::Or => {
                debug_assert_eq!(e1.f, NO_JUMP);
                self.discharge_vars(e2, line);
                let mut t = e2.t;
                self.concat_jump(&mut t, e1.t)?;
                e2.t = t;
                *e1 = e2.clone();
            }
            BinOp::Concat => {
                self.exp2val(e2, line)?;
                // 右が CONCAT 命令なら B を伸ばして連結（右結合の畳み込み）。
                if let EK::Reloc(pc) = e2.k
                    && self.code_at(pc as i32).opcode() == Some(OpCode::Concat)
                {
                    let e1info = match e1.k {
                        EK::NonReloc(r) => r,
                        _ => unreachable!(),
                    };
                    self.free_exp(e1);
                    self.with_code(pc as i32, |i| set_arg_b(i, e1info));
                    e1.k = EK::Reloc(pc);
                    return Ok(());
                }
                self.exp2nextreg(e2, line)?;
                self.code_arith(OpCode::Concat, e1, e2, line)?;
            }
            BinOp::Add => self.code_arith(OpCode::Add, e1, e2, line)?,
            BinOp::Sub => self.code_arith(OpCode::Sub, e1, e2, line)?,
            BinOp::Mul => self.code_arith(OpCode::Mul, e1, e2, line)?,
            BinOp::Div => self.code_arith(OpCode::Div, e1, e2, line)?,
            BinOp::Mod => self.code_arith(OpCode::Mod, e1, e2, line)?,
            BinOp::Pow => self.code_arith(OpCode::Pow, e1, e2, line)?,
            BinOp::Eq => self.code_comp(OpCode::Eq, true, e1, e2, line)?,
            BinOp::Ne => self.code_comp(OpCode::Eq, false, e1, e2, line)?,
            BinOp::Lt => self.code_comp(OpCode::Lt, true, e1, e2, line)?,
            BinOp::Le => self.code_comp(OpCode::Le, true, e1, e2, line)?,
            BinOp::Gt => self.code_comp(OpCode::Lt, false, e1, e2, line)?,
            BinOp::Ge => self.code_comp(OpCode::Le, false, e1, e2, line)?,
        }
        Ok(())
    }

    /// 本家 `luaK_prefix`: 単項演算のコード生成。
    fn prefix(&mut self, op: UnOp, e: &mut ExpDesc, line: u32) -> LuaResult<()> {
        match op {
            UnOp::Neg => {
                if !e.is_numeral() {
                    self.exp2anyreg(e, line)?;
                }
                let mut dummy = ExpDesc::new(EK::KNum(0.0));
                self.code_arith(OpCode::Unm, e, &mut dummy, line)?;
            }
            UnOp::Not => self.code_not(e, line)?,
            UnOp::Len => {
                self.exp2anyreg(e, line)?;
                let mut dummy = ExpDesc::new(EK::KNum(0.0));
                self.code_arith(OpCode::Len, e, &mut dummy, line)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::parser::Parser;

    /// ソースをコンパイルして main `Proto` を得る。
    fn compile(src: &str) -> Proto {
        let mut heap = Heap::new();
        let block = Parser::parse(src.as_bytes(), "test").expect("parse");
        CodeGen::compile(&mut heap, &block, "test").expect("codegen")
    }

    /// 命令列をオペコード列に変換（検査用）。
    fn ops(proto: &Proto) -> Vec<OpCode> {
        proto.code.iter().map(|i| i.opcode().unwrap()).collect()
    }

    #[test]
    fn empty_chunk_returns() {
        let p = compile("");
        // 本家: 空チャンクは RETURN 0 1 のみ。
        assert_eq!(ops(&p), vec![OpCode::Return]);
        assert!(p.is_vararg);
    }

    #[test]
    fn constant_folding() {
        // 1 + 2 は定数畳み込みされ LOADK 3 になる。末尾に暗黙の RETURN が付く（本家準拠）。
        let p = compile("return 1 + 2");
        assert_eq!(ops(&p), vec![OpCode::LoadK, OpCode::Return, OpCode::Return]);
        assert_eq!(p.constants.len(), 1);
        match p.constants[0] {
            Value::Number(n) => assert_eq!(n, 3.0),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn no_fold_div_by_zero() {
        // 1/0 は畳み込まれない（本家準拠）。
        let p = compile("return 1 / 0");
        assert!(ops(&p).contains(&OpCode::Div));
    }

    #[test]
    fn local_arith() {
        // local a = 10; local b = 20; return a + b
        let p = compile("local a = 10 local b = 20 return a + b");
        let o = ops(&p);
        assert_eq!(o[0], OpCode::LoadK); // a = 10
        assert_eq!(o[1], OpCode::LoadK); // b = 20
        assert_eq!(o[2], OpCode::Add); // a + b
        assert_eq!(o[3], OpCode::Return);
    }

    #[test]
    fn global_access() {
        // print("hi")
        let p = compile("print('hi')");
        let o = ops(&p);
        assert_eq!(o[0], OpCode::GetGlobal); // print
        assert_eq!(o[1], OpCode::LoadK); // "hi" -> reg (引数)
        assert_eq!(o[2], OpCode::Call);
        assert_eq!(o[3], OpCode::Return);
        // 定数表に print と hi。
        assert_eq!(p.constants.len(), 2);
    }

    #[test]
    fn assignment_to_global() {
        let p = compile("x = 1");
        let o = ops(&p);
        assert!(o.contains(&OpCode::SetGlobal));
    }

    #[test]
    fn table_access() {
        // t.x = t.y
        let p = compile("local t = {} t.x = t.y");
        let o = ops(&p);
        assert!(o.contains(&OpCode::NewTable));
        assert!(o.contains(&OpCode::GetTable));
        assert!(o.contains(&OpCode::SetTable));
    }

    #[test]
    fn method_call() {
        let p = compile("local o = {} o:m(1)");
        let o = ops(&p);
        assert!(o.contains(&OpCode::SelfOp));
        assert!(o.contains(&OpCode::Call));
    }

    #[test]
    fn if_then_else() {
        let p = compile("if x then return 1 else return 2 end");
        let o = ops(&p);
        // 条件で GETGLOBAL + TEST/JMP、各分岐で LOADK + RETURN。
        assert!(o.contains(&OpCode::Test) || o.contains(&OpCode::Eq));
        assert!(o.contains(&OpCode::Jmp));
        assert!(o.iter().filter(|&&x| x == OpCode::Return).count() >= 2);
    }

    #[test]
    fn while_loop() {
        let p = compile("while x do x = x end");
        let o = ops(&p);
        assert!(o.contains(&OpCode::Jmp));
    }

    #[test]
    fn numeric_for_loop() {
        let p = compile("for i = 1, 10 do end");
        let o = ops(&p);
        assert!(o.contains(&OpCode::ForPrep));
        assert!(o.contains(&OpCode::ForLoop));
    }

    #[test]
    fn generic_for_loop() {
        let p = compile("for k, v in pairs(t) do end");
        let o = ops(&p);
        assert!(o.contains(&OpCode::TForLoop));
    }

    #[test]
    fn break_inside_numeric_for() {
        // for 内（さらに if 内）の break が囲みループを解決できること（#12 回帰）。
        let p = compile("for k = 1, 100 do if k > 5 then break end end");
        let o = ops(&p);
        assert!(o.contains(&OpCode::ForLoop));
        assert!(o.contains(&OpCode::Jmp)); // break のジャンプ
    }

    #[test]
    fn break_inside_generic_for() {
        let p = compile("for k, v in pairs(t) do break end");
        assert!(ops(&p).contains(&OpCode::TForLoop));
    }

    #[test]
    fn nested_for_break_inner_only() {
        // 内側ループの break が内側のみ抜ける（外側に影響しない）。
        let p = compile("for a = 1, 3 do for b = 1, 3 do if b == 2 then break end end end");
        assert_eq!(ops(&p).iter().filter(|&&x| x == OpCode::ForLoop).count(), 2);
    }

    #[test]
    fn comparison() {
        let p = compile("return 1 < 2");
        let o = ops(&p);
        assert!(o.contains(&OpCode::Lt));
        assert!(o.contains(&OpCode::LoadBool));
    }

    #[test]
    fn concat() {
        // 右結合の連結が 1 つの CONCAT に畳まれる。
        let p = compile("return 'a' .. 'b' .. 'c'");
        let o = ops(&p);
        assert_eq!(o.iter().filter(|&&x| x == OpCode::Concat).count(), 1);
    }

    #[test]
    fn closure_with_upvalue() {
        // 外側ローカルを捕捉するクロージャ。
        let p = compile("local x = 1 local function f() return x end return f");
        let o = ops(&p);
        assert!(o.contains(&OpCode::Closure));
        // CLOSURE の直後に upvalue 捕捉の MOVE 疑似命令。
        let cpos = o.iter().position(|&x| x == OpCode::Closure).unwrap();
        assert_eq!(o[cpos + 1], OpCode::Move);
        // 子 proto が 1 つ、upvalue 1 個。
        assert_eq!(p.protos.len(), 1);
        assert_eq!(p.protos[0].num_upvalues, 1);
        assert!(
            p.protos[0]
                .code
                .iter()
                .any(|i| i.opcode() == Some(OpCode::GetUpval))
        );
    }

    #[test]
    fn vararg_function() {
        let p = compile("local function f(...) return ... end");
        assert_eq!(p.protos.len(), 1);
        assert!(p.protos[0].is_vararg);
        assert!(
            p.protos[0]
                .code
                .iter()
                .any(|i| i.opcode() == Some(OpCode::Vararg))
        );
    }

    #[test]
    fn vararg_outside_errors() {
        let mut heap = Heap::new();
        let block = Parser::parse(b"return ...", "test").unwrap();
        // main は vararg なので OK、関数内の非可変長で `...` はエラー。
        assert!(CodeGen::compile(&mut heap, &block, "test").is_ok());

        let block2 = Parser::parse(b"local function f() return ... end", "test").unwrap();
        assert!(CodeGen::compile(&mut Heap::new(), &block2, "test").is_err());
    }

    #[test]
    fn table_constructor_mixed() {
        let p = compile("return { 1, 2, x = 3, [10] = 4, 5 }");
        let o = ops(&p);
        assert!(o.contains(&OpCode::NewTable));
        assert!(o.contains(&OpCode::SetList)); // 配列部 1,2,5
        assert!(o.contains(&OpCode::SetTable)); // x=3, [10]=4
    }

    #[test]
    fn multiple_assignment() {
        let p = compile("local a, b = 1, 2 a, b = b, a");
        // パニックせずコンパイルできること（レジスタ割付・順序）。
        assert!(ops(&p).contains(&OpCode::Return));
    }

    #[test]
    fn and_or_shortcircuit() {
        let p = compile("return a and b or c");
        let o = ops(&p);
        assert!(o.contains(&OpCode::TestSet) || o.contains(&OpCode::Test));
    }

    #[test]
    fn unary_neg_folds_constant() {
        let p = compile("return -5");
        assert_eq!(ops(&p), vec![OpCode::LoadK, OpCode::Return, OpCode::Return]);
        match p.constants[0] {
            Value::Number(n) => assert_eq!(n, -5.0),
            _ => panic!(),
        }
    }

    #[test]
    fn unary_not_and_len() {
        let p = compile("return not x, #t");
        let o = ops(&p);
        assert!(o.contains(&OpCode::Not));
        assert!(o.contains(&OpCode::Len));
    }

    #[test]
    fn line_info_recorded() {
        let p = compile("local a = 1\nlocal b = 2\nreturn a");
        // 命令ごとに行番号が記録されている。
        assert_eq!(p.line_info.len(), p.code.len());
        assert!(p.line_info.contains(&2));
    }
}
