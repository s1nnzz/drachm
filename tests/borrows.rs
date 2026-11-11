//! Borrows build; raw pointers type-check but cannot be built yet.

const BORROWS: &str = r#"struct Person { public: string name; uint32 age; }
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
    mut uint32 n = 1;
    bump(&mut n);
    string s = "a";
    &string r = &s;
    string t = copy r;
    mut Person p = "x", 3;
    show(&p);
    string old = take p.name leaving "y";
    Holder h = &s;
    &string both = pick(&s, &t);
    mut &string cur = &s;
    cur = &t;
}
"#;

const POINTERS: &str = r#"//--!$unsafe: true
none main() {
    mut uint32 m = 5;
    *uint32 ptr;
    unsafe {
        ptr = address of m;
        *ptr = 7;
        bool gone = ptr == none;
    }
}
"#;

#[test]
fn borrows_and_pointers_build() {
    let (program, diags) = drachm::check(BORROWS);
    let errors: Vec<String> = diags.iter().filter(|d| d.severity == drachm::diag::Severity::Error).map(|d| d.message.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let program = program.expect("checks");
    let out = std::env::temp_dir().join(format!("drachm_borrows_test{}", std::env::consts::EXE_SUFFIX));
    drachm::build(&program, &out, false).expect("borrows build");
    let _ = std::fs::remove_file(&out);

    let (program, diags) = drachm::check(POINTERS);
    let program = program.unwrap_or_else(|| panic!("{:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
    drachm::build(&program, &out, false).expect("pointers build");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn destroy_type_checks() {
    let (program, diags) = drachm::check("struct S { uint32 n; none drop() { } }\nnone main() { S x = 1; destroy x; }\n");
    assert!(program.is_some(), "{:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
}
