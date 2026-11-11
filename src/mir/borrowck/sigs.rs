//! The provisional rule for what a call may assume about the borrow a function
//! returns (`LivesSig`). WP8 replaces `provisional` by the real inference; nothing
//! else may depend on how it works.

use super::regions::{param_slots, region_slots};
use crate::hir::{self, FuncId};
use crate::mir::LivesSig;

pub fn provisional(hir: &hir::Program, f: FuncId) -> LivesSig {
    let func = &hir.funcs[f];
    if region_slots(hir, func.ret) == 0 {
        return LivesSig::NoBorrowResult;
    }
    if let Some(v) = &func.lives {
        return LivesSig::Declared(v.clone());
    }
    // One parameter with a borrow: that is the one. Several (or none): all of
    // them, which checks the callers more strictly than WP8 will.
    let with_borrows: Vec<usize> = (0..func.params).filter(|&i| param_slots(hir, f, i) > 0).collect();
    LivesSig::Inferred(with_borrows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule() {
        let src = "struct Parser lives src { public: &string src; uint32 pos; }
&string one(&string a, uint32 n) { return a; }
&string two(&string a, &string b) lives b { return b; }
&string both(&string a, &string b) lives a, b { return a; }
&string many(&string a, &string b) { return a; }
uint32 plain(&string a) { return a::len(); }
Parser mk(&string t) lives t { Parser p = t, 0; return p; }
struct S { public: string name; &string get() { return &self.name; } }
none main() { }
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("{diags:?}"));
        let find = |n: &str| hir.funcs.iter().position(|f| f.symbol.ends_with(n)).unwrap();
        assert_eq!(provisional(&hir, find("one")), LivesSig::Inferred(vec![0]));
        assert_eq!(provisional(&hir, find("two")), LivesSig::Declared(vec![1]));
        assert_eq!(provisional(&hir, find("both")), LivesSig::Declared(vec![0, 1]));
        assert_eq!(provisional(&hir, find("many")), LivesSig::Inferred(vec![0, 1]));
        assert_eq!(provisional(&hir, find("plain")), LivesSig::NoBorrowResult);
        assert_eq!(provisional(&hir, find("mk")), LivesSig::Declared(vec![0]));
        assert_eq!(provisional(&hir, find("get")), LivesSig::Inferred(vec![0]));
    }
}
