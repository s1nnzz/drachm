use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use drachm::diag::{self, Source};

const USAGE: &str = "\
Drachm compiler

usage:
  drachm build <file.drh> [-o <output>] [-O]   compile to a native executable
  drachm run   <file.drh> [-O]                 compile and run
  drachm check <file.drh> [--dump-mir]         check for errors only (--dump-mir also prints the MIR)
  drachm explain <code>                        describe an error code, e.g. E0101

  -O   optimised build: slower to compile, much faster to run";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { die(USAGE) };

    if cmd == "explain" {
        let Some(id) = args.get(1) else { die("explain needs a code, e.g. `drachm explain E0101`") };
        match diag::codes::lookup(id) {
            Some(c) => println!("{}: {}\n\n{}", c.id, c.title, c.explain),
            None => die(&format!("there is no code `{id}`")),
        }
        return;
    }

    let mut input = None;
    let mut output = None;
    let mut opt = false;
    let mut dump_mir = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-o" => output = Some(PathBuf::from(rest.next().unwrap_or_else(|| die("-o needs a path")))),
            "-O" => opt = true,
            "--dump-mir" => dump_mir = true,
            _ if input.is_none() => input = Some(PathBuf::from(a)),
            _ => die(&format!("unexpected argument '{a}'\n\n{USAGE}")),
        }
    }
    let Some(input) = input else { die(USAGE) };

    match cmd.as_str() {
        "check" => {
            // The MIR dump is for looking at programs the ownership checks
            // reject too, so it stops after type checking.
            let hir = front_end(&input, !dump_mir);
            if dump_mir {
                let mut diags = Vec::new();
                let mir = drachm::mir::build_program(&hir, &mut diags);
                if !diags.is_empty() {
                    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
                    let text = std::fs::read_to_string(&input).unwrap_or_default();
                    let src = Source::new(input.display().to_string(), text);
                    eprint!("{}", diag::render_all(&diags, &src, color));
                    if diag::has_errors(&diags) {
                        exit(1);
                    }
                }
                print!("{}", drachm::mir::print_program(&mir, &hir));
            }
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

/// Check the file, print its diagnostics, and exit if there were errors.
/// `ownership` is false to stop after type checking.
fn front_end(input: &Path, ownership: bool) -> drachm::hir::Program {
    let text = std::fs::read_to_string(input).unwrap_or_else(|e| die(&format!("{}: {e}", input.display())));
    let (checked, diags) = if ownership { drachm::check(&text) } else { drachm::front(&text) };
    if !diags.is_empty() {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let src = Source::new(input.display().to_string(), text);
        eprint!("{}", diag::render_all(&diags, &src, color));
    }
    checked.unwrap_or_else(|| exit(1))
}

fn build(input: &Path, out: &Path, opt: bool) {
    let program = front_end(input, true);
    drachm::build(&program, out, opt).unwrap_or_else(|e| die(&format!("drachm: {e}")));
}
