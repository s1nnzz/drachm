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
  drachm test  [file.drh | folder] [NAME] [-O]          run the `test` blocks, each on its own (NAME: only those whose name contains it)
  drachm check [file.drh | folder] [--dump-mir | --dump-borrowck]  check for errors only (the dumps print the MIR or the borrow checker state)
  drachm new <name>                            make a project folder with drachm.toml and src/main.drh
  drachm add ADDRESS[@TAG | @^RANGE] [--name NAME]   add a git dependency to drachm.toml and drachm.lock
  drachm remove NAME                           remove a dependency
  drachm update [NAME]                         take newer tags (inside the ranges) for one or all dependencies
  drachm install [--locked]                     fetch deps/ again from drachm.lock
  drachm lsp                                   run the language server on stdin and stdout (for editors)
  drachm explain <code>                        describe an error code, e.g. E0101

A file or a project folder (one with drachm.toml) is given; the default is the
current folder.

  -O        optimised build: slower to compile, much faster to run
  --locked  (build, run, check, test, install) use exactly what drachm.lock
            lists: nothing is resolved or written, and a mismatch is E0818";

/// The package manager commands, run in the current folder.
enum Package {
    Add { spec: String, name: Option<String> },
    Remove(String),
    Update(Option<String>),
    Install { locked: bool },
}

/// One parsed command line.
enum Command {
    Explain(String),
    New(String),
    Package(Package),
    Check { input: PathBuf, dump_mir: bool, dump_borrowck: bool, locked: bool },
    Build { input: PathBuf, out: Option<PathBuf>, opt: bool, locked: bool },
    Run { input: PathBuf, opt: bool, locked: bool },
    Test { input: PathBuf, opt: bool, filter: Option<String>, locked: bool },
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
        Command::Package(p) => {
            let dir = Path::new(".");
            let done = match p {
                Package::Add { spec, name } => drachm::pkg::add(dir, &spec, name.as_deref()),
                Package::Remove(name) => drachm::pkg::remove(dir, &name),
                Package::Update(name) => drachm::pkg::update(dir, name.as_deref()),
                Package::Install { locked } => drachm::pkg::install_with(dir, locked),
            };
            match done {
                Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
                Err(f) => die(f.render(color()).trim_end()),
            }
        }
        Command::Explain(id) => match diag::codes::lookup(&id) {
            Some(c) => println!("{}: {}\n\n{}", c.id, c.title, c.explain),
            None => die(&format!("there is no code `{id}`")),
        },
        Command::Check { input, dump_mir, dump_borrowck, locked } => {
            let opened = open(&input, locked);
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
        Command::Build { input, out, opt, locked } => {
            let opened = program(&input, "build", locked);
            let out = out.unwrap_or_else(|| default_output(&input));
            build(&opened, &out, opt || opened.config.build.opt);
        }
        Command::Test { input, opt, filter, locked } => {
            // A library has tests too, so this is `open`, not `program`.
            let opened = open(&input, locked);
            let (_, mut program) = front_end(&opened, true);
            let all: Vec<(usize, String)> = program.tests.iter().map(|t| (t.func, t.name.clone())).collect();
            // A NAME keeps the tests whose name contains it; the others are not built.
            let tests: Vec<(usize, String)> = all.iter().filter(|(_, name)| filter.as_deref().map_or(true, |f| name.contains(f))).cloned().collect();
            let filtered_out = filter.is_some().then(|| all.len() - tests.len());
            run_tests(&mut program, &tests, opt || opened.config.build.opt, filtered_out);
        }
        Command::Run { input, opt, locked } => {
            let opened = program(&input, "run", locked);
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
    match (cmd.as_str(), &args[1..]) {
        ("add", [spec]) => return Command::Package(Package::Add { spec: spec.clone(), name: None }),
        ("add", [spec, flag, name]) | ("add", [flag, name, spec]) if flag == "--name" => {
            return Command::Package(Package::Add { spec: spec.clone(), name: Some(name.clone()) })
        }
        ("add", _) => die("add needs an address, e.g. `drachm add github.com/someone/sdl-drh@^0.4` (and optionally `--name NAME`)"),
        ("remove", [name]) => return Command::Package(Package::Remove(name.clone())),
        ("remove", _) => die("remove needs the name of a dependency, e.g. `drachm remove sdl_drh`"),
        ("update", []) => return Command::Package(Package::Update(None)),
        ("update", [name]) => return Command::Package(Package::Update(Some(name.clone()))),
        ("update", _) => die("update takes at most one dependency name"),
        ("install", []) => return Command::Package(Package::Install { locked: false }),
        ("install", [flag]) if flag == "--locked" => return Command::Package(Package::Install { locked: true }),
        ("install", _) => die("install takes at most `--locked`"),
        _ => {}
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
    let mut filter = None;
    let mut locked = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match (a.as_str(), cmd.as_str()) {
            ("-o", "build") => output = Some(PathBuf::from(rest.next().unwrap_or_else(|| die("-o needs a path")))),
            ("-O", "build" | "run" | "test") => opt = true,
            ("--dump-mir", "check") => dump_mir = true,
            ("--dump-borrowck", "check") => dump_borrowck = true,
            ("--locked", "check" | "build" | "run" | "test") => locked = true,
            _ if input.is_none() && !a.starts_with('-') => input = Some(PathBuf::from(a)),
            // `drachm test PATH NAME`: the second name is a filter on the test names.
            _ if cmd == "test" && input.is_some() && filter.is_none() && !a.starts_with('-') => filter = Some(a.clone()),
            _ => die(&format!("unexpected argument '{a}'\n\n{USAGE}")),
        }
    }
    let input = input.unwrap_or_else(|| PathBuf::from("."));
    match cmd.as_str() {
        "check" => Command::Check { input, dump_mir, dump_borrowck, locked },
        "build" => Command::Build { input, out: output, opt, locked },
        "run" => Command::Run { input, opt, locked },
        "test" => Command::Test { input, opt, filter, locked },
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
/// With `locked`, the dependencies are those the lock file lists, exactly.
fn open(input: &Path, locked: bool) -> Opened {
    let options = drachm::deps::Options { locked, ..Default::default() };
    drachm::config::open_with(input, &options).unwrap_or_else(|f| die(f.render(color()).trim_end()))
}

/// `open` for `build` and `run`, which need a program: a library is an error.
fn program(input: &Path, verb: &str, locked: bool) -> Opened {
    let opened = open(input, locked);
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
    let (checked, graph, diags) = drachm::front_opened(opened, ownership);
    if !diags.is_empty() {
        if graph.map.is_empty() {
            eprintln!("{}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("\n"));
        } else {
            eprint!("{}", diag::render_all_map(&diags, &graph.map, color()));
        }
    }
    (entry, checked.unwrap_or_else(|| exit(1)))
}

/// `drachm test`: each test is built as a program of its own and run, so a
/// test that fails does not stop the others. A test passes when its program
/// exits with 0. A test's output is shown only when it fails.
/// `filtered_out` is the number of tests a NAME left out, if one was given.
fn run_tests(program: &mut drachm::hir::Program, tests: &[(usize, String)], opt: bool, filtered_out: Option<usize>) {
    let plural = if tests.len() == 1 { "" } else { "s" };
    println!("running {} test{plural}", tests.len());
    let dir = std::env::temp_dir().join(format!("drachm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&e.to_string()));
    let (mut passed, mut failed) = (0, 0);
    for (i, (func, name)) in tests.iter().enumerate() {
        let exe = dir.join(format!("test{i}")).with_extension(EXE_EXTENSION);
        if let Err(e) = drachm::build_test(program, *func, &exe, opt) {
            let _ = std::fs::remove_dir_all(&dir);
            die(&format!("drachm: {e}"));
        }
        print!("test {name} ... ");
        match Process::new(&exe).output() {
            Ok(run) if run.status.success() => {
                passed += 1;
                println!("ok");
            }
            Ok(run) => {
                failed += 1;
                println!("FAILED");
                let shown = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
                for line in shown.lines() {
                    println!("    {line}");
                }
                match run.status.code() {
                    Some(code) => println!("    exit code {code}"),
                    None => println!("    stopped by a signal"),
                }
            }
            Err(e) => {
                failed += 1;
                println!("FAILED");
                println!("    could not run it: {e}");
            }
        }
        let _ = std::fs::remove_file(&exe);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let verdict = if failed == 0 { "ok" } else { "FAILED" };
    match filtered_out {
        Some(n) => println!("\ntest result: {verdict}. {passed} passed; {failed} failed; {n} filtered out"),
        None => println!("\ntest result: {verdict}. {passed} passed; {failed} failed"),
    }
    if failed > 0 {
        exit(1);
    }
}

fn build(opened: &Opened, out: &Path, opt: bool) {
    let (_, program) = front_end(opened, true);
    drachm::build(&program, out, opt).unwrap_or_else(|e| die(&format!("drachm: {e}")));
}
