mod ast;
mod codegen;
mod error;
mod lexer;
mod parser;
mod typeck;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");

const USAGE: &str = "\
Drachm compiler

usage:
  drachm build <file.dr> [-o <output>] [-O]   compile to a native executable
  drachm run   <file.dr> [-O]                 compile and run
  drachm check <file.dr>                      type-check only";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { die(USAGE) };

    let mut input = None;
    let mut output = None;
    let mut opt = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-o" => output = Some(PathBuf::from(rest.next().unwrap_or_else(|| die("-o needs a path")))),
            "-O" => opt = true,
            _ if input.is_none() => input = Some(PathBuf::from(a)),
            _ => die(&format!("unexpected argument '{a}'\n\n{USAGE}")),
        }
    }
    let Some(input) = input else { die(USAGE) };

    match cmd.as_str() {
        "check" => {
            front_end(&input);
            eprintln!("ok");
        }
        "build" => {
            let out = output.unwrap_or_else(|| input.with_extension(""));
            build(&input, &out, opt);
        }
        "run" => {
            let dir = std::env::temp_dir().join(format!("drachm-run-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&e.to_string()));
            let exe = dir.join("prog");
            build(&input, &exe, opt);
            let status = Command::new(&exe).status();
            let _ = std::fs::remove_dir_all(&dir);
            match status {
                Ok(s) => exit(s.code().unwrap_or(1)),
                Err(e) => die(&format!("failed to run program: {e}")),
            }
        }
        _ => die(USAGE),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1)
}

fn front_end(input: &Path) -> (ast::Program, std::collections::HashMap<String, typeck::Sig>) {
    let src = std::fs::read_to_string(input).unwrap_or_else(|e| die(&format!("{}: {e}", input.display())));
    let name = input.display().to_string();
    let result = lexer::lex(&src)
        .and_then(parser::parse)
        .and_then(|prog| typeck::check(&prog).map(|sigs| (prog, sigs)));
    result.unwrap_or_else(|e| die(&e.render(&name, &src)))
}

fn build(input: &Path, out: &Path, opt: bool) {
    let (prog, sigs) = front_end(input);
    let obj = codegen::compile(&prog, &sigs, opt).unwrap_or_else(|e| die(&format!("codegen error: {e}")));

    let tmp = std::env::temp_dir().join(format!("drachm-build-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap_or_else(|e| die(&e.to_string()));
    let obj_path = tmp.join("prog.o");
    let rt_path = tmp.join("runtime.c");
    std::fs::write(&obj_path, obj).unwrap_or_else(|e| die(&e.to_string()));
    std::fs::write(&rt_path, RUNTIME_C).unwrap_or_else(|e| die(&e.to_string()));

    let status = Command::new("cc")
        .arg(&obj_path)
        .arg(&rt_path)
        .arg(if opt { "-O2" } else { "-O0" })
        .arg("-o")
        .arg(out)
        .status();
    let _ = std::fs::remove_dir_all(&tmp);
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => die("linking failed"),
        Err(e) => die(&format!("could not run 'cc' (a C compiler is needed for linking): {e}")),
    }
}
