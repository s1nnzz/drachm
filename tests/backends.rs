//! `build_unchecked`: the way tests build programs that use borrows, which
//! `build` refuses until the borrow checker exists (WP7). Every other program
//! is built and compared on both backends by `tests/run.rs`.

use std::env::consts::EXE_SUFFIX;
use std::fs;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// `build` runs the borrow checker; these programs are written to be safe by
/// hand and go straight to the code generators. Each runs on both backends
/// with the string runtime's leak check on.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "a1",
        r####"in io;
uint64 size(&string s) { return s::len(); }
string shout(&string s) { return s::upper(); }
none show(&string s) { io::outln(s); }
bool same(&string a, &string b) { return a == b; }
none add(&mut string s) { s += "!"; }
none setit(&mut string s) { s = "x"; }
none setme(&mut string s, &string t) { s = copy t; }
none main() {
    mut string a = "hello";
    string b = "hello";
    io::outln(size(&a)::text());
    show(&a);
    io::outln(shout(&a));
    io::outln(same(&a, &b)::text());
    add(&mut a);
    show(&a);
    setit(&mut a);
    show(&a);
    setme(&mut a, &b);
    show(&a);
    mut string c = "c1";
    string d = "d1";
    mut &string cur = &c;
    show(cur);
    cur = &d;
    show(cur);
    io::outln(cur);
}
"####,
        r####"5
hello
HELLO
true
hello!
x
hello
c1
d1
d1
"####,
    ),
    (
        "a2",
        r####"in io;
struct Pair { public: string a; uint32 n; string get() { return copy self.a; } uint32 num() { return self.n; } none bump() { self.n += 1; } &string name() { return &self.a; } }
none show(&Pair p) { io::outln(p.a); io::outln(p.n::text()); }
none grow(&mut Pair p) { p.a = p.a + "+"; p.n += 5; }
&string first(&Pair p) { return &p.a; }
none main() {
    mut Pair p = "x", 1;
    show(&p);
    grow(&mut p);
    show(&p);
    io::outln(p::get());
    io::outln(p::num()->text());
    p::bump();
    &string f = first(&p);
    io::outln(f);
    &string g = p::name();
    io::outln(g);
    io::outln(p.n::text());
}
"####,
        r####"x
1
x+
6
x+
6
x+
x+
7
"####,
    ),
    (
        "a3",
        r####"in io;
struct Parser lives src { public: &string src; uint32 pos; }
uint32 advance(&mut Parser p) { p.pos += 1; return p.pos; }
uint64 total(&Parser p) { return p.src::len() + p.pos; }
Parser make(&string t) lives t { Parser q = t, 0; return q; }
none main() {
    string text = "hello world";
    mut Parser p = &text, 0;
    advance(&mut p);
    advance(&mut p);
    io::outln(total(&p)::text());
    Parser q = make(&text);
    io::outln(q.src);
    mut Parser c = copy p;
    advance(&mut c);
    io::outln(c.pos::text());
    io::outln(p.pos::text());
    io::outln(text);
}
"####,
        r####"13
hello world
3
2
hello world
"####,
    ),
    (
        "a4",
        r####"in io;
none bump(&mut uint32 n) { n += 1; }
none twice(&mut uint32 n) { bump(lend n); bump(lend n); }
struct C { public: uint32 n; uint32 get() { return self.n; } none add(uint32 k) { self.n += k; } }
struct V { public: string s; none set(uint64 k) { self.s = k::text(); } uint64 get() { return 7; } }
none main() {
    mut uint32 n = 1;
    twice(&mut n);
    io::outln(n::text());
    mut C c = 4;
    c::add(c::get());
    io::outln(c.n::text());
    mut string s = "ab";
    s += s::len()->text();
    io::outln(s);
    mut V v = "q";
    v::set(v::get() + 1);
    io::outln(v.s);
}
"####,
        r####"3
8
ab2
8
"####,
    ),
    (
        "a5",
        r####"in io;
struct P { public: string name; uint32 hp; }
none main() {
    mut P p = "ann", 10;
    &string nm = &p.name;
    &mut uint32 hp = &mut p.hp;
    hp += 3;
    io::outln(nm);
    io::outln(p.hp::text());
    string x = "lo";
    &string t = &"a";
    &string u = &x::upper();
    io::outln(t);
    io::outln(u);
    mut uint32 i = 0;
    while (i < 3) {
        &string w = &x;
        mut &string cur = &x;
        string y = "k";
        cur = &y;
        io::outln(w + cur);
        i += 1;
    }
}
"####,
        r####"ann
13
a
LO
lok
lok
lok
"####,
    ),
    (
        "a6",
        r####"in io;
struct Holder lives src { public: &string src; none drop() { io::outln("drop " + self.src); } }
struct W lives r { public: &mut uint32 r; uint32 k; }
struct Pt lives name { public: &string name; uint32 n; }
none tick(&mut W w) { w.k += 1; }
none show(&string s) { io::outln(s); }
none main() {
    string a = "alpha";
    if (true) {
        Holder h = &a;
        io::outln("in");
    }
    mut uint32 c = 5;
    mut W w = &mut c, 3;
    tick(&mut w);
    tick(&mut w);
    io::outln(w.k::text()); io::outln(c::text());
    string nm = "pt";
    Pt p = &nm, 1;
    Pt q = &nm, 1;
    io::outln((p == q)::text());
    show(p.name);
    mut string m = "mm";
    &mut string r = &mut m;
    show(&r);
    r += "!";
    show(&m);
    for (mut uint32 i = 0; i < 2; i++) {
        &string t = &"x";
        show(t);
    }
}
"####,
        r####"in
drop alpha
5
5
true
pt
mm
mm!
x
x
"####,
    ),
    (
        "b1",
        r####"//--!$unsafe: true
in io;
struct P { public: uint32 hp; }
unsafe none set(*uint32 p, uint32 v) { *p = v; }
unsafe uint32 peek(*uint32 p) { return *p; }
none main() {
    mut uint32 n = 1;
    mut P q = 3;
    *uint32 p;
    *uint32 h;
    unsafe {
        p = address of n;
        *p = 7;
        set(copy p, peek(copy p) + 1);
    }
    io::outln(n::text());
    unsafe {
        h = address of q.hp;
        *h = *h + 4;
    }
    io::outln(q.hp::text());
    unsafe { io::outln((p == none)::text()); }
}
"####,
        r####"8
7
false
"####,
    ),
];

#[test]
fn borrows_through_mir() {
    for (name, source, want) in PROGRAMS {
        let (checked, diags) = drachm::check(source);
        let hir = checked.unwrap_or_else(|| panic!("{name} does not check: {:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        for optimise in [false, true] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("be-{name}-{}{EXE_SUFFIX}", optimise as u8));
            drachm::build_unchecked(&hir, &exe, optimise).unwrap_or_else(|e| panic!("{name} (-O {optimise}): {e}"));
            let run = Command::new(&exe).env("DRACHM_CHECK_LEAKS", "1").output().unwrap();
            let _ = fs::remove_file(&exe);
            assert_eq!(text(&run.stdout), *want, "{name} (-O {optimise})");
            let err = text(&run.stderr);
            assert!(!err.contains("leaked") && !err.contains("double free"), "{name} (-O {optimise}): {err}");
        }
    }
}
