use std::env::consts::EXE_EXTENSION;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{exit, Command as Process};

use drachm::config::Opened;
use drachm::diag;

const USAGE: &str = "\
Drachm compiler

usage:
  drachm build [file.drh | folder] [-o <output>] [-O]   compile to a native executable
  drachm run   [file.drh | folder] [-O]                 compile and run
  drachm check [file.drh | folder] [--dump-mir | --dump-borrowck]  check for errors only (the dumps print the MIR or the borrow checker state)
  drachm new <name>                            make a project folder with drachm.toml and src/main.drh
  drachm lsp                                   run the language server on stdin and stdout (for editors)
  drachm explain <code>                        describe an error code, e.g. E0101

A file or a project folder (one with drachm.toml) is given; the default is the
current folder.

  -O   optimised build: slower to compile, much faster to run";

/// One parsed command line.
enum Command {
    Explain(String),
    New(String),
    Check { input: PathBuf, dump_mir: bool, dump_borrowck: bool },
    Build { input: PathBuf, out: Option<PathBuf>, opt: bool },
    Run { input: PathBuf, opt: bool },
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Command::New(name) => {
            let parent = Path::new(".");
            match drachm::config::new_project(parent, &name) {
                Ok(dir) => println!("made {} with drachm.toml, src/main.drh and tests/; run it with `drachm run {}`", dir.display(), name),
                Err(f) => die(f.render(color()).trim_end()),
            }
        }
        Command::Explain(id) => match diag::codes::lookup(&id) {
            Some(c) => println!("{}: {}\n\n{}", c.id, c.title, c.explain),
            None => die(&format!("there is no code `{id}`")),
        },
        Command::Check { input, dump_mir, dump_borrowck } => {
            let opened = open(&input);
            // The MIR dump is for looking at programs the ownership checks
            // reject too, so it stops after type checking.
            let (entry, hir) = front_end(&opened, !dump_mir && !dump_borrowck);
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
                    let text = std::fs::read_to_string(&entry).unwrap_or_default();
                    let map = diag::SourceMap::single(entry.display().to_string(), text);
                    eprint!("{}", diag::render_all_map(&diags, &map, color()));
                    if diag::has_errors(&diags) {
                        exit(1);
                    }
                }
                print!("{}", drachm::mir::print_program(&mir, &hir));
            }
        }
        Command::Build { input, out, opt } => {
            let opened = program(&input, "build");
            let out = out.unwrap_or_else(|| default_output(&input));
            build(&opened, &out, opt || opened.config.build.opt);
        }
        Command::Run { input, opt } => {
            let opened = program(&input, "run");
            let dir = std::env::temp_dir().join(format!("drachm-run-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&e.to_string()));
            let exe = dir.join("prog").with_extension(EXE_EXTENSION);
            build(&opened, &exe, opt || opened.config.build.opt);
            let status = Process::new(&exe).status();
            let _ = std::fs::remove_dir_all(&dir);
            match status {
                Ok(s) => exit(s.code().unwrap_or(1)),
                Err(e) => die(&format!("failed to run program: {e}")),
            }
        }
    }
}

/// Reads the arguments. Each flag is accepted only by the commands it is for.
fn parse(args: &[String]) -> Command {
    let Some(cmd) = args.first() else { die(USAGE) };
    if cmd == "new" {
        let [_, name] = args else { die("new needs one name, e.g. `drachm new mygame`") };
        return Command::New(name.clone());
    }
    if cmd == "lsp" {
        exit(drachm::lsp::run());
    }
    if cmd == "explain" {
        let Some(id) = args.get(1) else { die("explain needs a code, e.g. `drachm explain E0101`") };
        return Command::Explain(id.clone());
    }
    let mut input = None;
    let mut output = None;
    let mut opt = false;
    let mut dump_mir = false;
    let mut dump_borrowck = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match (a.as_str(), cmd.as_str()) {
            ("-o", "build") => output = Some(PathBuf::from(rest.next().unwrap_or_else(|| die("-o needs a path")))),
            ("-O", "build" | "run") => opt = true,
            ("--dump-mir", "check") => dump_mir = true,
            ("--dump-borrowck", "check") => dump_borrowck = true,
            _ if input.is_none() && !a.starts_with('-') => input = Some(PathBuf::from(a)),
            _ => die(&format!("unexpected argument '{a}'\n\n{USAGE}")),
        }
    }
    let input = input.unwrap_or_else(|| PathBuf::from("."));
    match cmd.as_str() {
        "check" => Command::Check { input, dump_mir, dump_borrowck },
        "build" => Command::Build { input, out: output, opt },
        "run" => Command::Run { input, opt },
        _ => die(USAGE),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1)
}

fn color() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Reads the config of `input` (a file or a project folder), or exits with its errors.
fn open(input: &Path) -> Opened {
    drachm::config::open(input).unwrap_or_else(|f| die(f.render(color()).trim_end()))
}

/// `open` for `build` and `run`, which need a program: a library is an error.
fn program(input: &Path, verb: &str) -> Opened {
    let opened = open(input);
    if let Err(f) = opened.program(verb) {
        die(f.render(color()).trim_end());
    }
    if let Err(f) = drachm::config::check_target(&opened.config) {
        die(f.render(color()).trim_end());
    }
    opened
}

/// `drachm build folder` writes `folder/folder` (a project is named by its folder).
fn default_output(input: &Path) -> PathBuf {
    if input.is_dir() {
        let name = input.canonicalize().ok().and_then(|p| p.file_name().map(|n| n.to_owned())).unwrap_or_else(|| "program".into());
        return input.join(name).with_extension(EXE_EXTENSION);
    }
    input.with_extension(EXE_EXTENSION)
}

/// Load and check the opened program, print its diagnostics, and exit if
/// there were errors. `ownership` is false to stop after type checking.
/// Returns the entry file (for the MIR dump's own messages) and the program.
fn front_end(opened: &Opened, ownership: bool) -> (PathBuf, drachm::hir::Program) {
    let entry = opened.entry.clone();
    let (checked, graph, diags) = if ownership {
        drachm::check_project_with_dependencies(&entry, &opened.info, &opened.deps)
    } else {
        drachm::front_project_with_dependencies(&entry, &opened.info, &opened.deps)
    };
    if !diags.is_empty() {
        if graph.map.is_empty() {
            eprintln!("{}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("\n"));
        } else {
            eprint!("{}", diag::render_all_map(&diags, &graph.map, color()));
        }
    }
    (entry, checked.unwrap_or_else(|| exit(1)))
}

fn build(opened: &Opened, out: &Path, opt: bool) {
    let (_, program) = front_end(opened, true);
    drachm::build(&program, out, opt).unwrap_or_else(|e| die(&format!("drachm: {e}")));
}
