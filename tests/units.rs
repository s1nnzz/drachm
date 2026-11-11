//! Units cost nothing at run time: a program with units on its types and no
//! scaling generates exactly the same C as the same program with bare
//! numbers, and `2<mi> + 2<mi>` compiles to a plain addition.

/// The C text of the optimised build of `src`, from MIR.
fn c_text(src: &str) -> String {
    let (hir, diags) = drachm::check(src);
    let hir = hir.unwrap_or_else(|| panic!("does not compile: {} diagnostics", diags.len()));
    let mut mir = drachm::mir::build_program(&hir, &mut Vec::new());
    drachm::mir::cleanup::cleanup_program(&hir, &mut mir);
    for body in mir.bodies.iter_mut().flatten() {
        drachm::mir::analysis::mark_in_memory(&hir, body);
    }
    assert!(drachm::mir::cleanup::ineligible_reason(&hir, &mir).is_none(), "the MIR backends do not take this program");
    drachm::cgen_mir::generate(&hir, &mir)
}

#[test]
fn unit_types_generate_the_same_c_as_bare_types() {
    let with_units = "
unit m;
unit s;
float64<m> total(float64<m> step, uint32 n) {
    mut float64<m> sum = 0.0<m>;
    for (mut uint32 i = 0; i < n; i++) {
        sum += step;
    }
    return sum;
}
float64<m/s> speed(float64<m> d, float64<s> t) {
    return d / t;
}
float64<m> twice(float64<m> d) {
    return d * 2.0 + d - d;
}
bool longer(float64<m> a, float64<m> b) {
    return a > b;
}
none main() {
    float64<m> a = total(0.25<m>, 1000);
    float64<m/s> v = speed(a, 4.0<s>);
    float64<m> t = twice(a);
    bool b = longer(a, t);
}
";
    let bare = "
float64 total(float64 step, uint32 n) {
    mut float64 sum = 0.0;
    for (mut uint32 i = 0; i < n; i++) {
        sum += step;
    }
    return sum;
}
float64 speed(float64 d, float64 t) {
    return d / t;
}
float64 twice(float64 d) {
    return d * 2.0 + d - d;
}
bool longer(float64 a, float64 b) {
    return a > b;
}
none main() {
    float64 a = total(0.25, 1000);
    float64 v = speed(a, 4.0);
    float64 t = twice(a);
    bool b = longer(a, t);
}
";
    assert_eq!(c_text(with_units), c_text(bare));
}

#[test]
fn the_same_unit_on_both_sides_needs_no_scaling_code() {
    let with_units = "
unit ft;
unit mi = 5280 ft;
uint32<mi> double(uint32<mi> a) {
    return a + a;
}
none main() {
    uint32<mi> d = double(2<mi>);
}
";
    let bare = "
uint32 double(uint32 a) {
    return a + a;
}
none main() {
    uint32 d = double(2);
}
";
    assert_eq!(c_text(with_units), c_text(bare));
}
