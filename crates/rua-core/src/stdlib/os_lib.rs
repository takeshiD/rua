//! os ライブラリ（本家 `loslib.c` 相当）。担当: **lua-stdlib**。
//!
//! `time`/`clock`/`date`/`getenv`/`difftime`/`tmpname`/`exit`/`execute`/`remove`/`rename`
//! を提供する。`date` は strftime のサブセット（`%Y %m %d %H %M %S %y %j %p %A %a %B %b %%`
//! 等）と UTC 指定 `!`、テーブル形式 `*t` をサポートする。タイムゾーンは UTC 固定（簡略化）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::LuaResult;
use crate::gc::GcHandle;
use crate::state::LuaState;
use crate::value::Value;

use super::aux;

thread_local! {
    static CLOCK_START: SystemTime = SystemTime::now();
}

pub fn open(state: &mut LuaState) {
    let t = state.new_table();
    let tk = match t {
        Value::GcRef(GcHandle::Table(k)) => k,
        _ => return,
    };
    aux::register(state, tk, "time", l_time);
    aux::register(state, tk, "clock", l_clock);
    aux::register(state, tk, "date", l_date);
    aux::register(state, tk, "getenv", l_getenv);
    aux::register(state, tk, "difftime", l_difftime);
    aux::register(state, tk, "tmpname", l_tmpname);
    aux::register(state, tk, "exit", l_exit);
    aux::register(state, tk, "execute", l_execute);
    aux::register(state, tk, "remove", l_remove);
    aux::register(state, tk, "rename", l_rename);

    if let GcHandle::Table(g) = state.global.globals {
        aux::set_field(state, g, "os", t);
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as f64)
        .unwrap_or(0.0)
}

/// `os.time([table])`: 引数なしは現在の Unix 時刻。テーブル指定は UTC として合成する。
fn l_time(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let secs = match aux::opt_value(&args, 0) {
        Value::Nil => now_secs(),
        Value::GcRef(GcHandle::Table(tk)) => {
            let field = |state: &mut LuaState, name: &str, default: i64| -> i64 {
                let k = state.new_string(name.as_bytes());
                match state.global.heap.get_table(tk).map(|t| t.get(&k)) {
                    Some(Value::Number(n)) => n as i64,
                    _ => default,
                }
            };
            let year = field(state, "year", 1970);
            let month = field(state, "month", 1);
            let day = field(state, "day", 1);
            let hour = field(state, "hour", 12);
            let min = field(state, "min", 0);
            let sec = field(state, "sec", 0);
            let days = days_from_civil(year, month, day);
            (days * 86400 + hour * 3600 + min * 60 + sec) as f64
        }
        _ => return Err(aux::arg_error(state, 1, "time", "table expected")),
    };
    aux::ret(state, vec![Value::Number(secs)])
}

/// `os.clock()`: プロセス開始からの経過秒（CPU 時間の近似として実時間を返す）。
fn l_clock(state: &mut LuaState) -> LuaResult<i32> {
    let elapsed = CLOCK_START.with(|start| start.elapsed().map(|d| d.as_secs_f64()).unwrap_or(0.0));
    aux::ret(state, vec![Value::Number(elapsed)])
}

/// `os.difftime(t2, t1)`: 秒差。
fn l_difftime(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let t2 = aux::check_number(state, &args, 0, "difftime")?;
    let t1 = aux::opt_number(state, &args, 1, "difftime", 0.0)?;
    aux::ret(state, vec![Value::Number(t2 - t1)])
}

/// `os.getenv(name)`: 環境変数（無ければ nil）。
fn l_getenv(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let name = aux::check_str_bytes(state, &args, 0, "getenv")?;
    let name = String::from_utf8_lossy(&name).into_owned();
    let result = match std::env::var(&name) {
        Ok(v) => state.new_string(v.as_bytes()),
        Err(_) => Value::Nil,
    };
    aux::ret(state, vec![result])
}

static TMPNAME_SEQ: AtomicU64 = AtomicU64::new(0);

fn l_tmpname(state: &mut LuaState) -> LuaResult<i32> {
    let seq = TMPNAME_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = format!("/tmp/lua_{pid:08x}_{seq:04x}");
    let v = state.new_string(name.as_bytes());
    aux::ret(state, vec![v])
}

/// `os.execute([command])`: シェルコマンドを実行して終了コードを返す。
/// 引数なしの場合、シェルが利用可能なら true を返す。
fn l_execute(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    match aux::opt_value(&args, 0) {
        Value::Nil => aux::ret(state, vec![Value::Boolean(true)]),
        _ => {
            let cmd = aux::check_str_bytes(state, &args, 0, "execute")?;
            let cmd_str = String::from_utf8_lossy(&cmd);
            match std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(cmd_str.as_ref())
                .status()
            {
                Ok(s) => {
                    let code = s.code().unwrap_or(-1);
                    aux::ret(state, vec![Value::Number(code as f64)])
                }
                Err(e) => {
                    let msg = state.new_string(e.to_string().as_bytes());
                    aux::ret(state, vec![Value::Nil, msg])
                }
            }
        }
    }
}

/// `os.remove(filename)`: ファイルまたは空ディレクトリを削除する。
/// 成功時は true、失敗時は nil + エラーメッセージ。
fn l_remove(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let filename = aux::check_str_bytes(state, &args, 0, "remove")?;
    let filename = String::from_utf8_lossy(&filename);
    let result =
        std::fs::remove_file(filename.as_ref()).or_else(|_| std::fs::remove_dir(filename.as_ref()));
    match result {
        Ok(()) => aux::ret(state, vec![Value::Boolean(true)]),
        Err(e) => {
            let msg = state.new_string(e.to_string().as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

/// `os.rename(oldname, newname)`: ファイルまたはディレクトリをリネームする。
/// 成功時は true、失敗時は nil + エラーメッセージ。
fn l_rename(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let oldname = aux::check_str_bytes(state, &args, 0, "rename")?;
    let newname = aux::check_str_bytes(state, &args, 1, "rename")?;
    let oldname = String::from_utf8_lossy(&oldname);
    let newname = String::from_utf8_lossy(&newname);
    match std::fs::rename(oldname.as_ref(), newname.as_ref()) {
        Ok(()) => aux::ret(state, vec![Value::Boolean(true)]),
        Err(e) => {
            let msg = state.new_string(e.to_string().as_bytes());
            aux::ret(state, vec![Value::Nil, msg])
        }
    }
}

fn l_exit(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let code = match aux::opt_value(&args, 0) {
        Value::Nil | Value::Boolean(true) => 0,
        Value::Boolean(false) => 1,
        _ => aux::check_int(state, &args, 0, "exit")? as i32,
    };
    std::process::exit(code);
}

// ============================================================================
// os.date（strftime サブセット, UTC 固定）
// ============================================================================

/// 分解された日時（UTC）。
struct Tm {
    year: i64,
    month: i64, // 1..=12
    day: i64,   // 1..=31
    hour: i64,
    min: i64,
    sec: i64,
    wday: i64, // 0=日曜
    yday: i64, // 1..=366
}

fn l_date(state: &mut LuaState) -> LuaResult<i32> {
    let args = aux::args_vec(state);
    let fmt = match aux::opt_value(&args, 0) {
        Value::Nil => b"%c".to_vec(),
        _ => aux::check_str_bytes(state, &args, 0, "date")?,
    };
    let t = if matches!(aux::opt_value(&args, 1), Value::Nil) {
        now_secs() as i64
    } else {
        aux::check_number(state, &args, 1, "date")? as i64
    };

    // 先頭 '!' は UTC 指定。本実装は常に UTC（タイムゾーン非対応）。
    let fmt = fmt.strip_prefix(b"!").unwrap_or(&fmt).to_vec();
    let tm = gmtime(t);

    // "*t" はテーブルを返す。
    if fmt == b"*t" {
        let tbl = state.new_table();
        if let Value::GcRef(GcHandle::Table(tk)) = tbl {
            aux::set_field(state, tk, "year", Value::Number(tm.year as f64));
            aux::set_field(state, tk, "month", Value::Number(tm.month as f64));
            aux::set_field(state, tk, "day", Value::Number(tm.day as f64));
            aux::set_field(state, tk, "hour", Value::Number(tm.hour as f64));
            aux::set_field(state, tk, "min", Value::Number(tm.min as f64));
            aux::set_field(state, tk, "sec", Value::Number(tm.sec as f64));
            aux::set_field(state, tk, "wday", Value::Number((tm.wday + 1) as f64));
            aux::set_field(state, tk, "yday", Value::Number(tm.yday as f64));
            aux::set_field(state, tk, "isdst", Value::Boolean(false));
        }
        return aux::ret(state, vec![tbl]);
    }

    let s = strftime(&fmt, &tm);
    let v = state.new_string(s.as_bytes());
    aux::ret(state, vec![v])
}

const WDAY_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WDAY_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MON_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MON_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn strftime(fmt: &[u8], tm: &Tm) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        if i >= fmt.len() {
            out.push('%');
            break;
        }
        match fmt[i] {
            b'Y' => out.push_str(&tm.year.to_string()),
            b'y' => out.push_str(&format!("{:02}", tm.year.rem_euclid(100))),
            b'm' => out.push_str(&format!("{:02}", tm.month)),
            b'd' => out.push_str(&format!("{:02}", tm.day)),
            b'H' => out.push_str(&format!("{:02}", tm.hour)),
            b'M' => out.push_str(&format!("{:02}", tm.min)),
            b'S' => out.push_str(&format!("{:02}", tm.sec)),
            b'p' => out.push_str(if tm.hour < 12 { "AM" } else { "PM" }),
            b'A' => out.push_str(WDAY_FULL[(tm.wday.rem_euclid(7)) as usize]),
            b'a' => out.push_str(WDAY_ABBR[(tm.wday.rem_euclid(7)) as usize]),
            b'B' => out.push_str(MON_FULL[((tm.month - 1).rem_euclid(12)) as usize]),
            b'b' | b'h' => out.push_str(MON_ABBR[((tm.month - 1).rem_euclid(12)) as usize]),
            b'j' => out.push_str(&format!("{:03}", tm.yday)),
            b'w' => out.push_str(&tm.wday.to_string()),
            b'c' => out.push_str(&format!(
                "{} {} {:2} {:02}:{:02}:{:02} {}",
                WDAY_ABBR[(tm.wday.rem_euclid(7)) as usize],
                MON_ABBR[((tm.month - 1).rem_euclid(12)) as usize],
                tm.day,
                tm.hour,
                tm.min,
                tm.sec,
                tm.year
            )),
            b'x' => out.push_str(&format!(
                "{:02}/{:02}/{:02}",
                tm.month,
                tm.day,
                tm.year.rem_euclid(100)
            )),
            b'X' => out.push_str(&format!("{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec)),
            b'%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other as char);
            }
        }
        i += 1;
    }
    out
}

/// Unix 秒（UTC）を分解する。
fn gmtime(t: i64) -> Tm {
    let days = t.div_euclid(86400);
    let secs_of_day = t.rem_euclid(86400);
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;
    // 1970-01-01 は木曜（civil_from_days の基準）。wday: 0=日曜。
    let wday = (days.rem_euclid(7) + 4).rem_euclid(7);
    let (year, month, day) = civil_from_days(days);
    let yday = days_from_civil(year, month, day) - days_from_civil(year, 1, 1) + 1;
    Tm {
        year,
        month,
        day,
        hour,
        min,
        sec,
        wday,
        yday,
    }
}

/// Howard Hinnant の暦アルゴリズム: Unix 日数 → (year, month, day)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) → Unix 日数。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
