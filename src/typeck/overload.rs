//! Overload resolution (S5B.1). Each argument is ranked against the parameter
//! of each candidate; a candidate is as good as its worst argument; the best
//! candidate wins. Two equally good candidates are ambiguous, unless one is
//! better on some argument and no worse on any.

use super::*;

/// How well one argument fits one parameter, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    /// The same type, or a whole constant that fits the type.
    Exact,
    /// A conversion that loses nothing.
    Lossless,
    /// A conversion that may lose information (W0102 when it is used).
    Lossy,
}

pub enum Pick {
    Fit(usize),
    NoFit,
    Ambiguous(Vec<usize>),
}

/// `cands[i]` holds the rank of each argument for candidate `i`.
pub fn pick(cands: &[Vec<Rank>]) -> Pick {
    let worst: Vec<Rank> = cands.iter().map(|r| r.iter().copied().max().unwrap_or(Rank::Exact)).collect();
    let Some(&best) = worst.iter().min() else { return Pick::NoFit };
    let winners: Vec<usize> = (0..cands.len()).filter(|&i| worst[i] == best).collect();
    if let [only] = winners[..] {
        return Pick::Fit(only);
    }
    let dominates = |a: usize, b: usize| {
        let (ra, rb) = (&cands[a], &cands[b]);
        ra.iter().zip(rb).all(|(x, y)| x <= y) && ra.iter().zip(rb).any(|(x, y)| x < y)
    };
    match winners.iter().copied().find(|&a| winners.iter().all(|&b| a == b || dominates(a, b))) {
        Some(a) => Pick::Fit(a),
        None => Pick::Ambiguous(winners),
    }
}

impl Checker<'_> {
    /// How well the checked value `v` fits a parameter of type `to`; `None`
    /// if it cannot be passed there at all.
    pub(super) fn rank_val(&self, v: &Val, to: Type) -> Option<Rank> {
        if to.is_borrow() {
            return match v {
                Val::Expr(x) if x.ty == to => Some(Rank::Exact),
                _ => None,
            };
        }
        if let Val::NoneLit(_) = v {
            return (super::optionals::is_optional(self.g, to) || to.is_ptr()).then_some(Rank::Exact);
        }
        // A `T` fits a `T?` parameter: the wrap is part of the conversion.
        if let Some(inner) = super::optionals::inner_of(self.g, to) {
            return match v {
                Val::Expr(x) if x.ty == to => Some(Rank::Exact),
                _ => self.rank_val(v, inner),
            };
        }
        match v {
            // A whole constant adapts to any integer type it fits; its default
            // type (`uint32` for 5) is the exact one.
            Val::Const(c @ Const::Int(_)) if to.is_int() => Some(if !c.fits(to) {
                Rank::Lossy
            } else if c.default_type() == Some(to) {
                Rank::Exact
            } else {
                Rank::Lossless
            }),
            // A decimal constant (`5.0`) is a float; as an integer it loses its fraction.
            Val::Const(Const::Dec(_) | Const::NegZero) if to.is_int() => Some(Rank::Lossy),
            Val::Const(c @ Const::Int(_)) if to.is_float() => Some(if c.fits(to) { Rank::Lossless } else { Rank::Lossy }),
            Val::Const(Const::Dec(_) | Const::NegZero) if to == Type::F64 => Some(Rank::Exact),
            Val::Const(Const::Dec(_) | Const::NegZero) if to.is_float() => Some(Rank::Lossy),
            Val::Const(_) => None,
            Val::UConst(..) => to.is_numeric().then_some(Rank::Lossless),
            Val::NoneLit(_) => None,
            Val::Expr(x) => {
                let t = x.ty;
                if t == to || (t.is_borrow() && t.target() == Some(to)) {
                    Some(Rank::Exact)
                } else if t.is_numeric() && to.is_numeric() && !t.mentions_unit() && !to.mentions_unit() {
                    Some(if t.converts_losslessly_to(to) { Rank::Lossless } else { Rank::Lossy })
                } else {
                    None
                }
            }
        }
    }
}
