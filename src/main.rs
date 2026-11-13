use std::env::consts::EXE_EXTENSION;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use drachm::diag;

const USAGE: &str = "\
Drachm compiler

usage:
  drachm build <file.drh | folder> [-o <output>] [-O]   compile to a native executable
  drachm run   <file.drh | folder> [-O]                 compile and run
  drachm check <file.drh | folder> [--dump-mir | --dump-borrowck]  check for errors only (the dumps print the MIR or the borrow checker state)
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
    let mut dump_borrowck = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-o" => output = Some(PathBuf::from(rest.next().unwrap_or_else(|| die("-o needs a path")))),
            "-O" => opt = true,
            "--dump-mir" => dump_mir = true,
            "--dump-borrowck" => dump_borrowck = true,
            _ if input.is_none() => input = Some(PathBuf::from(a)),
            _ => die(&format!("unexpected argument '{a}'\n\n{USAGE}")),
        }
    }
    let Some(input) = input else { die(USAGE) };

    match cmd.as_str() {
        "check" => {
            // The MIR dump is for looking at programs the ownership checks
            // reject too, so it stops after type checking.
            let (entry, hir) = front_end(&input, !dump_mir && !dump_borrowck);
            if dump_borrowck {
                let mut diags = Vec::new();
                let mir = drachm::mir::build_program(&hir, &mut diags);
                print!("{}", drachm::mir::borrowck::dump(&hir, &mir));
                return;
            }
            if dump_mir {
                let mut diags = Vec::new();
                let mut mir = drachm::mir::build_program(&hir, &mut diags);
                // The dump shows what the code generators get, minus the cleanup:
                // drops decided, flags in place.
                drachm::mir::drops::elaborate_program(&hir, &mut mir);
                if !diags.is_empty() {
                    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
                    let text = std::fs::read_to_string(&entry).unwrap_or_default();
                    let map = diag::SourceMap::single(entry.display().to_string(), text);
                    eprint!("{}", diag::render_all_map(&diags, &map, color));
                    if diag::has_errors(&diags) {
                        exit(1);
                    }
                }
                print!("{}", drachm::mir::print_program(&mir, &hir));
            }
        }
        "build" => {
            let out = output.unwrap_or_else(|| default_output(&input));
            build(&input, &out, opt);
        }
        "run" => {
            let dir = std::env::temp_dir().join(format!("drachm-run-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&e.to_string()));
            let exe = dir.join("prog").with_extension(EXE_EXTENSION);
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

/// `drachm build folder` writes `folder/folder` (a project is named by its folder).
fn default_output(input: &Path) -> PathBuf {
    if input.is_dir() {
        let name = input.canonicalize().ok().and_then(|p| p.file_name().map(|n| n.to_owned())).unwrap_or_else(|| "program".into());
        return input.join(name).with_extension(EXE_EXTENSION);
    }
    input.with_extension(EXE_EXTENSION)
}

/// Load and check the file or project folder, print its diagnostics, and exit
/// if there were errors. `ownership` is false to stop after type checking.
/// Returns the entry file (for the MIR dump's own messages) and the program.
fn front_end(input: &Path, ownership: bool) -> (PathBuf, drachm::hir::Program) {
    let (entry, info) = drachm::modules::ProjectInfo::open(input).unwrap_or_else(|e| die(&e));
    let (checked, graph, diags) = if ownership { drachm::check_project(&entry, &info) } else { drachm::front_project(&entry, &info) };
    if !diags.is_empty() {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        if graph.map.is_empty() {
            eprintln!("{}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("\n"));
        } else {
            eprint!("{}", diag::render_all_map(&diags, &graph.map, color));
        }
    }
    (entry, checked.unwrap_or_else(|| exit(1)))
}

fn build(input: &Path, out: &Path, opt: bool) {
    let (_, program) = front_end(input, true);
    drachm::build(&program, out, opt).unwrap_or_else(|e| die(&format!("drachm: {e}")));
}
