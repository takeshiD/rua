//! math ライブラリ（本家 `lmathlib.c` 相当）。担当: **lua-stdlib**。
//!
//! 三角関数・対数・指数・`floor`/`ceil`・`random`/`randomseed`・`min`/`max`・`pi`/`huge` ほか。
//! 乱数は本家が C の `rand()` を用いるが互換取得が難しいため、決定的な線形合同法で実装する
//! （値は本家と一致しないが分布特性は満たす）。NOTE: 乱数列の本家一致は非対応。

use std::cell::Cell;

use crate::error::LuaResult;
use crate::gc::GcHandle;
use crate::state::LuaState;
use crate::value::Value;

use super::aux;

thread_local! {
    /// 乱数状態（線形合同法）。`randomseed` で再設定。
    static RNG_STATE: Cell<u64> = const { Cell::new(0x2545_F491_4F6C_DD1D) };
}

pub fn open(state: &mut LuaState) {
    let m = state.new_table();
    let mk = match m {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };
    aux::register(state, mk, "abs", l_abs);
    aux::register(state, mk, "ceil", l_ceil);
    aux::register(state, mk, "floor", l_floor);
    aux::register(state, mk, "sqrt", l_sqrt);
    aux::register(state, mk, "sin", l_sin);
    aux::register(state, mk, "cos", l_cos);
    aux::register(state, mk, "tan", l_tan);
    aux::register(state, mk, "asin", l_asin);
    aux::register(state, mk, "acos", l_acos);
    aux::register(state, mk, "atan", l_atan);
    aux::register(state, mk, "sinh", l_sinh);
    aux::register(state, mk, "cosh", l_cosh);
    aux::register(state, mk, "tanh", l_tanh);
    aux::register(state, mk, "exp", l_exp);
    aux::register(state, mk, "log", l_log);
    aux::register(state, mk, "log10", l_log10);
    aux::register(state, mk, "pow", l_pow);
    aux::register(state, mk, "fmod", l_fmod);
    aux::register(state, mk, "modf", l_modf);
    aux::register(state, mk, "ldexp", l_ldexp);
    aux::register(state, mk, "frexp", l_frexp);
    aux::register(state, mk, "max", l_max);
    aux::register(state, mk, "min", l_min);
    aux::register(state, mk, "deg", l_deg);
    aux::register(state, mk, "rad", l_rad);
    aux::register(state, mk, "random", l_random);
    aux::register(state, mk, "randomseed", l_randomseed);

    aux::set_field(state, mk, "pi", Value::Number(std::f64::consts::PI));
    aux::set_field(state, mk, "huge", Value::Number(f64::INFINITY));
    // 本家: math.huge は HUGE_VAL（+inf）。最大/最小整数は無い。
    aux::set_field(state, mk, "maxinteger", Value::Number(i64::MAX as f64));

    // グローバルに math テーブルを設定。
    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "math", m);
    }
}

/// 単項 f64→f64 関数を実装するマクロ的ヘルパ。
fn unary(state: &mut LuaState, fname: &str, f: impl Fn(f64) -> f64) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, fname)?;
    aux::ret(state, vec![Value::Number(f(x))])
}

/// `math.frexp(x)`: x = m * 2^e（0.5 <= |m| < 1, または m=0/非有限はそのまま）。
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // 非正規化数: 2^64 倍して再帰し、指数を補正。
        let (m, e) = frexp(x * 2.0f64.powi(64));
        return (m, e - 64);
    }
    // 指数部を 1022 に固定して仮数を [0.5, 1) に正規化。
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, exp - 1022)
}

fn l_ldexp(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let m = aux::check_number(state, &args, 0, "ldexp")?;
    let e = aux::check_int(state, &args, 1, "ldexp")?;
    aux::ret(state, vec![Value::Number(m * 2.0f64.powi(e as i32))])
}

fn l_frexp(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, "frexp")?;
    let (m, e) = frexp(x);
    aux::ret(state, vec![Value::Number(m), Value::Number(e as f64)])
}

fn l_abs(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "abs", f64::abs)
}
fn l_ceil(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "ceil", f64::ceil)
}
fn l_floor(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "floor", f64::floor)
}
fn l_sqrt(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "sqrt", f64::sqrt)
}
fn l_sin(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "sin", f64::sin)
}
fn l_cos(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "cos", f64::cos)
}
fn l_tan(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "tan", f64::tan)
}
fn l_asin(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "asin", f64::asin)
}
fn l_acos(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "acos", f64::acos)
}
fn l_atan(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "atan", f64::atan)
}
fn l_sinh(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "sinh", f64::sinh)
}
fn l_cosh(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "cosh", f64::cosh)
}
fn l_tanh(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "tanh", f64::tanh)
}
fn l_exp(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "exp", f64::exp)
}
fn l_log(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "log", f64::ln)
}
fn l_log10(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "log10", f64::log10)
}
fn l_deg(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "deg", f64::to_degrees)
}
fn l_rad(state: &mut LuaState) -> LuaResult<i32> {
    unary(state, "rad", f64::to_radians)
}

fn l_pow(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, "pow")?;
    let y = aux::check_number(state, &args, 1, "pow")?;
    aux::ret(state, vec![Value::Number(x.powf(y))])
}

fn l_fmod(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, "fmod")?;
    let y = aux::check_number(state, &args, 1, "fmod")?;
    // C fmod: 符号は被除数に従う（Rust の % と同じ）。
    aux::ret(state, vec![Value::Number(x % y)])
}

fn l_modf(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let x = aux::check_number(state, &args, 0, "modf")?;
    let int_part = x.trunc();
    let frac = x - int_part;
    aux::ret(state, vec![Value::Number(int_part), Value::Number(frac)])
}

fn l_max(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut best = aux::check_number(state, &args, 0, "max")?;
    for i in 1..args.len() {
        let v = aux::check_number(state, &args, i, "max")?;
        if v > best {
            best = v;
        }
    }
    aux::ret(state, vec![Value::Number(best)])
}

fn l_min(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let mut best = aux::check_number(state, &args, 0, "min")?;
    for i in 1..args.len() {
        let v = aux::check_number(state, &args, i, "min")?;
        if v < best {
            best = v;
        }
    }
    aux::ret(state, vec![Value::Number(best)])
}

// ---- 乱数 -------------------------------------------------------------------

fn next_rand() -> f64 {
    // xorshift64 で [0,1) の値を作る。
    RNG_STATE.with(|st| {
        let mut x = st.get();
        if x == 0 {
            x = 0x2545_F491_4F6C_DD1D;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        st.set(x);
        // 上位 53bit を [0,1) へ。
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn l_random(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let r = next_rand();
    match args.len() {
        0 => aux::ret(state, vec![Value::Number(r)]),
        1 => {
            let m = aux::check_int(state, &args, 0, "random")?;
            if m < 1 {
                return Err(aux::arg_error(state, 1, "random", "interval is empty"));
            }
            let v = (r * m as f64).floor() as i64 + 1;
            aux::ret(state, vec![Value::Number(v as f64)])
        }
        _ => {
            let lo = aux::check_int(state, &args, 0, "random")?;
            let hi = aux::check_int(state, &args, 1, "random")?;
            if lo > hi {
                return Err(aux::arg_error(state, 2, "random", "interval is empty"));
            }
            let span = (hi - lo + 1) as f64;
            let v = lo + (r * span).floor() as i64;
            aux::ret(state, vec![Value::Number(v as f64)])
        }
    }
}

fn l_randomseed(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let seed = aux::opt_number(state, &args, 0, "randomseed", 0.0)?;
    RNG_STATE.with(|st| st.set((seed.to_bits()) ^ 0x2545_F491_4F6C_DD1D));
    aux::ret0(state)
}
