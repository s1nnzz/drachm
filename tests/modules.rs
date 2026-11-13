//! The module graph (step 7, WP1): temporary project trees are loaded and the
//! graph, the resolved targets and the messages are checked. What the names
//! inside the files mean is WP2 and not tested here.

use std::fs;
use std::path::{Path, PathBuf};

use drachm::diag::{self, has_errors, Diagnostic};
use drachm::modules::{self, ProjectInfo, Target};

/// A project in a fresh temp folder: `files` are paths below `src/`.
struct Tree {
    dir: PathBuf,
    info: ProjectInfo,
}

impl Tree {
    fn new(tag: &str, name: Option<&str>, files: &[(&str, &str)]) -> Tree {
        let dir = std::env::temp_dir().join(format!("drachm-modules-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, text) in files {
            let full = dir.join("src").join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        let info = ProjectInfo { name: name.map(String::from), root: dir.join("src"), display_base: Some(dir.clone()), requires_main: true };
        Tree { dir, info }
    }

    fn entry(&self) -> PathBuf {
        self.dir.join("src/main.drh")
    }

    fn load(&self) -> (modules::ModuleGraph, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let g = modules::load(&self.entry(), &self.info, &mut diags);
        (g, diags)
    }

    fn shown(&self, diags: &[Diagnostic], g: &modules::ModuleGraph) -> String {
        diag::render_all_map(diags, &g.map, false)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// What `config::open` gives for a path: the entry file and the module info.
fn open(path: &Path) -> Result<(PathBuf, ProjectInfo), ()> {
    drachm::config::open(path).map(|o| (o.entry, o.info)).map_err(|_| ())
}

fn names(g: &modules::ModuleGraph) -> Vec<&str> {
    g.modules.iter().map(|m| m.name.as_str()).collect()
}

#[test]
fn dotted_word_and_rooted_paths_find_the_same_file() {
    let t = Tree::new(
        "same",
        Some("mygame"),
        &[
            ("main.drh", "in game.physics;\nin physics inside game as p2;\nin mygame.game.physics as p3;\nnone main() { }\n"),
            ("game/physics.drh", "none step() { }\n"),
        ],
    );
    let (g, diags) = t.load();
    assert!(!has_errors(&diags), "{}", t.shown(&diags, &g));
    assert_eq!(names(&g), ["main", "game.physics"]);
    assert_eq!(g.modules[0].targets, [Some(Target::File(1)); 3]);
}

#[test]
fn paths_are_relative_to_the_importing_files_folder() {
    let t = Tree::new(
        "relative",
        Some("mygame"),
        &[
            ("main.drh", "in game.ai.brain;\nnone main() { }\n"),
            ("game/ai/brain.drh", "in senses;\nin mygame.game.physics;\n"),
            ("game/ai/senses.drh", "\n"),
            ("game/physics.drh", "\n"),
        ],
    );
    let (g, diags) = t.load();
    assert!(!has_errors(&diags), "{}", t.shown(&diags, &g));
    assert_eq!(names(&g), ["main", "game.ai.brain", "game.ai.senses", "game.physics"]);
    // Dependencies come before the file that imports them, the entry last.
    assert_eq!(g.order(), [2, 3, 1, 0]);
}

#[test]
fn item_and_glob_imports_name_a_module_too() {
    let t = Tree::new(
        "items",
        None,
        &[
            ("main.drh", "in Vector3 as V3, gravity from physics;\nin * from game.audio;\nin * from video inside game as v;\nnone main() { }\n"),
            ("physics.drh", "out struct Vector3 { float64 x; }\nout float64 gravity() { return 9.81; }\n"),
            ("game/audio.drh", "\n"),
            ("game/video.drh", "\n"),
        ],
    );
    let (g, diags) = t.load();
    assert!(diags.is_empty(), "{}", t.shown(&diags, &g));
    assert_eq!(names(&g), ["main", "physics", "game.audio", "game.video"]);
    assert_eq!(g.modules[0].targets, [Some(Target::File(1)), Some(Target::File(2)), Some(Target::File(3))]);
}

#[test]
fn std_modules_resolve_and_stay_out_of_the_files() {
    let t = Tree::new("std", None, &[("main.drh", "in io;\nin si;\nnone main() { }\n")]);
    let (g, diags) = t.load();
    assert!(diags.is_empty());
    assert_eq!(g.modules[0].targets, [Some(Target::Std("io")), Some(Target::Std("si"))]);
    assert_eq!(g.modules.len(), 1);
}

#[test]
fn cycles_load_each_file_once() {
    let t = Tree::new(
        "cycle",
        None,
        &[("main.drh", "in a;\nnone main() { }\n"), ("a.drh", "in b;\nnone fa() { }\n"), ("b.drh", "in a;\nin main;\nnone fb() { }\n")],
    );
    let (g, diags) = t.load();
    assert!(!has_errors(&diags), "{}", t.shown(&diags, &g));
    assert_eq!(names(&g), ["main", "a", "b"]);
    assert_eq!(g.modules[2].targets, [Some(Target::File(1)), Some(Target::File(0))]);
    assert_eq!(g.order().len(), 3);
}

#[test]
fn module_not_found_says_where_it_looked() {
    let t = Tree::new("missing", Some("mygame"), &[("main.drh", "in game.physiks;\nnone main() { }\n"), ("game/physics.drh", "\n")]);
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    assert!(text.contains("error[E0230]: module `game.physiks` not found"), "{text}");
    assert!(text.contains("looked for the file `src/game/physiks.drh`"), "{text}");
    assert!(text.contains("begin with `mygame`"), "{text}");
    assert!(text.contains("--> src/main.drh:1:4"), "{text}");
}

#[test]
fn a_project_file_named_like_a_std_module_is_ambiguous() {
    let t = Tree::new("ambiguous", None, &[("main.drh", "in io;\nnone main() { }\n"), ("io.drh", "\n")]);
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    assert!(text.contains("error[E0231]: `io` could mean the standard module `io` or the file `src/io.drh`"), "{text}");
}

#[test]
fn two_modules_with_one_name_need_an_alias() {
    let t = Tree::new(
        "samename",
        None,
        &[("main.drh", "in game.audio;\nin engine.audio;\nin engine.audio;\nnone main() { }\n"), ("game/audio.drh", "\n"), ("engine/audio.drh", "\n")],
    );
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    // Each import of a different module with a taken name is reported; the same module twice is not a clash.
    assert_eq!(diags.iter().filter(|d| d.code.id == "E0232").count(), 2, "{text}");
    assert!(text.contains("help: give it a name of its own: `in engine.audio as engineAudio;`"), "{text}");
    assert!(text.contains("--> src/main.drh:2:1"), "{text}");
}

#[test]
fn alias_removes_the_clash() {
    let t = Tree::new(
        "alias",
        None,
        &[("main.drh", "in game.audio;\nin engine.audio as engineAudio;\nnone main() { }\n"), ("game/audio.drh", "\n"), ("engine/audio.drh", "\n")],
    );
    let (g, diags) = t.load();
    assert!(!has_errors(&diags), "{}", t.shown(&diags, &g));
}

#[test]
fn export_checks() {
    let t = Tree::new(
        "exports",
        None,
        &[
            ("main.drh", "in util;\nnone main() { }\n"),
            (
                "util.drh",
                "in io;\nout helper, Missing, helper2 as helper;\nout none helper() { }\nout none helper2() { }\nout io;\nout none other() { }\nout other;\n",
            ),
        ],
    );
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    let codes: Vec<_> = diags.iter().map(|d| d.code.id).collect();
    // `Missing` does not exist; `helper2 as helper` and `helper` clash; `io` is an import; `other` twice, `helper` three times.
    for want in ["E0234", "E0233", "E0901"] {
        assert!(codes.contains(&want), "{want} missing in {text}");
    }
    assert_eq!(diags.iter().filter(|d| d.code.id == "E0233").count(), 3, "{text}");
    assert!(text.contains("--> src/util.drh:2:"), "{text}");
}

#[test]
fn syntax_errors_name_their_own_file() {
    let t = Tree::new("syntax", None, &[("main.drh", "in util;\nnone main() { }\n"), ("util.drh", "none ok() { }\nnone bad( {\n")]);
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    assert!(text.contains("--> src/util.drh:2:"), "{text}");
    assert!(text.contains("2 | none bad( {"), "{text}");
}

#[test]
fn a_file_beyond_the_project_name_is_not_a_root_path() {
    // Without a project name (a script) the root form does not exist.
    let t = Tree::new("script", None, &[("main.drh", "in mygame.util;\nnone main() { }\n"), ("util.drh", "\n")]);
    let (g, diags) = t.load();
    assert!(t.shown(&diags, &g).contains("E0230"));
}

#[test]
fn front_project_checks_several_files_as_one_program() {
    let one = Tree::new("one", None, &[("main.drh", "in io;\nnone main() { io::outln(\"hi\"); }\n")]);
    let (checked, _, diags) = drachm::front_project(&one.entry(), &one.info);
    assert!(checked.is_some(), "{diags:?}");

    let two = Tree::new("two", None, &[("main.drh", "in util;\nnone main() { util::f(); }\n"), ("util.drh", "out none f() { }\n")]);
    let (checked, graph, diags) = drachm::front_project(&two.entry(), &two.info);
    assert!(checked.is_some(), "{}", diag::render_all_map(&diags, &graph.map, false));
    // Another file's names are reached through its module, and only if exported.
    let private = Tree::new("private", None, &[("main.drh", "in util;\nnone main() { util::f(); }\n"), ("util.drh", "none f() { }\n")]);
    let (checked, graph, diags) = drachm::front_project(&private.entry(), &private.info);
    assert!(checked.is_none());
    assert!(diag::render_all_map(&diags, &graph.map, false).contains("error[E0236]"));
}

#[test]
fn project_discovery() {
    let t = Tree::new("discover", Some("demo"), &[("main.drh", "none main() { }\n"), ("sub/x.drh", "\n")]);
    fs::write(t.dir.join("drachm.toml"), "[project]\nname = \"demo\"\n").unwrap();
    let (entry, info) = open(&t.dir).unwrap();
    assert_eq!(entry, t.dir.join("src/main.drh"));
    assert_eq!(info.name.as_deref(), Some("demo"));
    // A file inside a project belongs to it.
    let (_, info) = open(&t.dir.join("src/sub/x.drh")).unwrap();
    assert_eq!(info.name.as_deref(), Some("demo"));
    assert_eq!(info.root, t.dir.join("src"));
    // A folder without a toml is not a project.
    let bare = t.dir.join("src");
    assert!(open(&bare).is_err());
    // A file outside every project is a script with its own folder as root.
    let loose = std::env::temp_dir().join(format!("drachm-modules-{}-loose.drh", std::process::id()));
    fs::write(&loose, "\n").unwrap();
    let (_, info) = open(&loose).unwrap();
    assert_eq!(info.name, None);
    assert_eq!(info.root, Path::new(&std::env::temp_dir()));
    let _ = fs::remove_file(loose);
}

#[test]
fn string_paths_climb_folders() {
    let t = Tree::new(
        "strings",
        Some("demo"),
        &[
            ("main.drh", "in game.physics;\nnone main() { }\n"),
            ("game/physics.drh", "in * from \"../runtime/scheduler.drh\" as scheduler;\nin \"../nothing.drh\";\n"),
            ("runtime/scheduler.drh", "out none tick() { }\n"),
        ],
    );
    let (g, diags) = t.load();
    let text = t.shown(&diags, &g);
    assert!(text.contains("module file `../nothing.drh` not found"), "{text}");
    assert_eq!(names(&g), ["main", "game.physics", "runtime.scheduler"]);
}
