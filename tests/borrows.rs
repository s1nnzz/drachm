//! Programs that use borrows and pointers type-check but cannot be built yet.

const SOURCE: &str = r#"//--!$unsafe: true
struct Person { public: string name; uint32 age; }
struct Counter { uint32 n; }
struct Holder lives src { &string src; }

none bump(&mut uint32 n) {
    n += 1;
    n = n + 1;
}
none show(&Person p) { }
none pass(&mut uint32 n) {
    bump(lend n);
}
&string pick(&string a, &string b) lives a, b {
    return a;
}
none main() {
    uint32 n = 1;
    bump(&mut n);
    string s = "a";
    &string r = &s;
    string t = copy r;
    Person p = "x", 3;
    show(&p);
    string old = take p.name leaving "y";
    mut uint32 m = 5;
    *uint32 ptr = address of m;
    unsafe {
        *ptr = 7;
        bool gone = ptr == none;
    }
    Holder h = &s;
    &string both = pick(&s, &t);
    mut &string cur = &s;
    cur = &t;
}
"#;

#[test]
fn borrows_type_check_but_do_not_build() {
    let (program, diags) = drachm::check(SOURCE);
    let errors: Vec<String> = diags.iter().filter(|d| d.severity == drachm::diag::Severity::Error).map(|d| d.message.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let program = program.expect("type-checks");
    assert!(program.uses_borrows);
    let out = std::env::temp_dir().join("drachm_borrows_test");
    let err = drachm::build(&program, &out, false).expect_err("must not build");
    assert!(err.contains("cannot be built yet"), "{err}");
}

#[test]
fn destroy_type_checks() {
    let (program, diags) = drachm::check("struct S { uint32 n; none drop() { } }\nnone main() { S x = 1; destroy x; }\n");
    assert!(program.is_some(), "{:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
}
