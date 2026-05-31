//! `ruac` — 本家 `luac.c` 相当（コンパイラ / バイトコード列挙）。
//!
//! # オプション一覧
//! - `-p` : 構文チェックのみ（出力なし、本家 `luac -p`）。
//! - `-l` : バイトコードを `luac -l` 風に列挙（2 回以上で定数/ローカル/upvalue も）。
//! - `-s` : デバッグ情報を除去。
//! - `-o` : コンパイル済みチャンクの出力先（既定 `luac.out`）。
//!
//! # 本家 `luac` との対応
//! 本家同様、`-p` を指定しない限り出力ファイルを書き出す（`-l` 併用時は列挙も行う）。
//! 複数入力ファイルは `-p`/`-l` で使用可能。チャンク出力（`-o`）は単一ファイルのみ。
//!
//! # バイトコード出力形式
//! `rua` 独自形式（マジック `\x1bRua`、常にリトルエンディアン）。
//! `rua run <file>` がマジックを検出して逆シリアライズ実行する（[`crate::bytecode`]）。

use std::process::ExitCode;
use std::rc::Rc;

use rua_core::compiler::compile;
use rua_core::state::LuaState;
use rua_core::vm::Proto;

use crate::cli::RuacCli;
use crate::{bytecode, disasm};

/// 既定の出力ファイル名（本家 `luac` と同じ）。
const DEFAULT_OUTPUT: &str = "luac.out";

/// `ruac` のエントリ。
pub fn main(args: RuacCli) -> ExitCode {
    // 出力するか（本家: `-p` で dumping=0）。
    let dumping = !args.parse_only;

    // チャンク出力は単一ファイルのみ（複数ファイルの結合は rua 独自形式未対応）。
    // `-p` や `-l` だけなら複数ファイルを受け付ける（本家同様）。
    if dumping && args.files.len() > 1 {
        eprintln!(
            "ruac: chunk output for multiple input files is not supported.\n\
             hint: use `-p` for syntax check only, or `-l` to list bytecode only;\n\
             those modes accept multiple files (add `-p` or `-l`)."
        );
        return ExitCode::from(1);
    }

    // 各ファイルをコンパイルする（state はヒープを保持＝定数文字列の解決に必要）。
    let mut state = LuaState::new();
    let mut protos: Vec<Rc<Proto>> = Vec::with_capacity(args.files.len());

    for file in &args.files {
        match read_source(file) {
            Ok(source) => {
                let chunkname = chunkname_for(file);
                match compile(&mut state.global.heap, &source, &chunkname) {
                    Ok(p) => protos.push(Rc::new(p)),
                    Err(e) => {
                        // 本家同様の形式で stderr へ出力。
                        eprintln!("rua: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("ruac: cannot open {file}: {e}");
                return ExitCode::from(1);
            }
        }
    }

    // 列挙（本家 `if (listing) luaU_print`）。
    if args.list > 0 {
        for p in &protos {
            print!("{}", disasm::disassemble(&state.global.heap, p, args.list));
        }
    }

    // 出力（本家 `if (dumping) luaU_dump`）。
    if dumping {
        let Some(proto) = protos.first() else {
            eprintln!("ruac: no input files");
            return ExitCode::from(1);
        };
        let output = args.output.as_deref().unwrap_or(DEFAULT_OUTPUT);
        let bytes = bytecode::dump(&state.global.heap, proto, args.strip);
        if let Err(e) = std::fs::write(output, &bytes) {
            eprintln!("ruac: cannot write {output}: {e}");
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}

/// ファイル（`-` なら標準入力）からソースを読む。
fn read_source(file: &str) -> std::io::Result<Vec<u8>> {
    if file == "-" {
        use std::io::Read;
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read(file)
    }
}

/// チャンク名（本家同様、ファイルは `@path`、標準入力は `=stdin`）。
fn chunkname_for(file: &str) -> String {
    if file == "-" {
        "=stdin".to_string()
    } else {
        format!("@{file}")
    }
}
