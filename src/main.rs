mod lexer;
mod parser;
mod compiler;
mod vm;
mod value;
mod object;
mod table;
mod stdlib;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    match args.len() {
        1 => run_repl(),
        2 => run_file(&args[1]),
        _ => {
            eprintln!("Usage: rust-lua [script]");
            process::exit(64);
        }
    }
}

fn run_file(path: &str) {
    match fs::read_to_string(path) {
        Ok(source) => {
            if let Err(err) = run(&source) {
                eprintln!("Error: {}", err);
                process::exit(70);
            }
        }
        Err(err) => {
            eprintln!("Could not read file '{}': {}", path, err);
            process::exit(74);
        }
    }
}

fn run_repl() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut input = String::new();
    
    loop {
        print!("> ");
        stdout.flush().unwrap();
        
        input.clear();
        if stdin.read_line(&mut input).unwrap() == 0 {
            println!();
            break;
        }
        
        if let Err(err) = run(&input) {
            eprintln!("Error: {}", err);
        }
    }
}

fn run(source: &str) -> Result<(), String> {
    // この関数は後で実装します
    // 現在はソースコードを表示するだけ
    println!("Source: {}", source);
    Ok(())
}
