//! What the language server asks the checker about: the types of
//! expressions, what each name refers to, which locals are in scope where,
//! and the members of structs and enums.
//!
//! The checker records into a thread-local table while `collect` is running
//! and does nothing otherwise, so the compiler pays only a flag check. Every
//! span is in the program's byte-offset space (`diag::SourceMap`).

use std::cell::RefCell;

use super::{short, Globals};
use crate::diag::Span;

/// One use of a name (or a declaration of it) and what it refers to.
#[derive(Debug, Clone)]
pub struct Ref {
    pub at: Span,
    /// Where the thing is declared; `None` for something built in.
    pub def: Option<Span>,
    /// The declaration as the hover shows it: `sint32 add(sint32 a, sint32 b)`.
    pub code: String,
    /// A line of prose under it: "field of `Point`".
    pub note: String,
}

/// A local, from its declaration to the end of its block.
#[derive(Debug, Clone)]
pub struct LocalRec {
    pub name: String,
    pub ty: String,
    pub decl: Span,
    pub mutable: bool,
    pub is_param: bool,
    /// The offset where it goes out of scope.
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct FieldRec {
    pub name: String,
    pub ty: String,
    pub public: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MethodRec {
    pub name: String,
    /// The signature as written.
    pub sig: String,
    pub is_static: bool,
    pub public: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructRec {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<FieldRec>,
    pub methods: Vec<MethodRec>,
}

#[derive(Debug, Clone)]
pub struct VariantRec {
    pub name: String,
    /// `Rect(float64 w, float64 h)`
    pub shape: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumRec {
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<VariantRec>,
}

#[derive(Debug, Clone)]
pub struct FuncRec {
    pub name: String,
    pub sig: String,
    pub name_span: Span,
}

#[derive(Debug, Default)]
pub struct Index {
    /// The checker ran (a program with syntax errors is not checked).
    pub ran: bool,
    /// The type of the expression at each span, outermost checked first.
    pub types: Vec<(Span, String)>,
    pub refs: Vec<Ref>,
    pub locals: Vec<LocalRec>,
    /// The enum type a value is wanted to have at each span (the bare
    /// variants it can be written as are the enum's).
    pub expected: Vec<(Span, String)>,
    pub structs: Vec<StructRec>,
    pub enums: Vec<EnumRec>,
    pub funcs: Vec<FuncRec>,
}

thread_local! {
    static CURRENT: RefCell<Option<Index>> = const { RefCell::new(None) };
}

/// Run `f`, and return what the checker recorded meanwhile (the tables of
/// the last program it checked).
pub fn collect<R>(f: impl FnOnce() -> R) -> (R, Index) {
    let old = CURRENT.with(|c| c.borrow_mut().replace(Index::default()));
    let r = f();
    let ix = CURRENT.with(|c| std::mem::replace(&mut *c.borrow_mut(), old)).unwrap_or_default();
    (r, ix)
}

/// Record something, if anyone is collecting.
pub(super) fn with(f: impl FnOnce(&mut Index)) {
    CURRENT.with(|c| {
        if let Some(ix) = c.borrow_mut().as_mut() {
            f(ix);
        }
    });
}

/// Drops the module part of a program-wide name (`game.physics::Vector3` is
/// `Vector3` in the editor, where the module is its file). A part that starts
/// with a lowercase letter and ends in `::` is a module.
pub fn demangle(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let word_start = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.');
        if word_start && b[i].is_ascii_lowercase() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
                j += 1;
            }
            if text[j..].starts_with("::") {
                i = j + 2;
                continue;
            }
        }
        let c = text[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The span of `name` in the signature text at `sig` (the word before its `(`),
/// or the whole signature if it is not found.
pub(super) fn name_in(src: &str, sig: Span, name: &str) -> Span {
    let name = name.rsplit("::").next().unwrap_or(name);
    match short(src, sig).find(&format!("{name}(")) {
        Some(i) => Span::new(sig.start + i, sig.start + i + name.len()),
        None => sig,
    }
}

/// The length of the (possibly qualified) name written at `start`.
pub(super) fn written_len(src: &str, start: usize) -> usize {
    src.get(start..).map_or(0, |t| t.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':'))).unwrap_or(t.len()))
}

/// The number of locals recorded so far; `close` ends the scope of those after it.
pub(super) fn mark() -> usize {
    let mut n = 0;
    with(|ix| n = ix.locals.len());
    n
}

/// The locals declared since `mark` that are still open go out of scope at `end`.
pub(super) fn close(mark: usize, end: usize) {
    with(|ix| {
        for l in ix.locals.iter_mut().skip(mark).filter(|l| l.end == usize::MAX) {
            l.end = end;
        }
    });
}

/// Fill the tables of structs, enums and functions once the declarations are known.
pub(super) fn finish(g: &Globals, src: &str) {
    with(|ix| {
        ix.ran = true;
        // A declaration of the prelude (`ConvertError`) has spans in text that is not the program's.
        let real = |name: &str, at: Span| short(src, at) == name.rsplit("::").next().unwrap_or(name);
        for s in &g.structs {
            // Stand-ins of type parameters are not declarations.
            if s.placeholder.is_some() || !real(&s.name, s.name_span) {
                continue;
            }
            ix.structs.push(StructRec {
                name: demangle(&s.name),
                name_span: s.name_span,
                fields: s.fields.iter().map(|f| FieldRec { name: f.name.clone(), ty: f.ty.map_or("?".into(), |t| demangle(&t.to_string())), public: f.public, span: f.span }).collect(),
                methods: s
                    .methods
                    .iter()
                    .map(|m| {
                        let sig = &g.sigs[m.func];
                        MethodRec { name: m.name.clone(), sig: short(src, sig.span).to_string(), is_static: m.is_static, public: m.public, span: sig.span }
                    })
                    .collect(),
            });
        }
        for e in &g.enums {
            if (e.kind != crate::hir::EnumKind::User && e.kind != crate::hir::EnumKind::Failure) || !real(&e.name, e.name_span) {
                continue;
            }
            let hidden = (e.kind == crate::hir::EnumKind::Failure) as usize;
            ix.enums.push(EnumRec {
                name: demangle(&e.name),
                name_span: e.name_span,
                variants: e
                    .variants
                    .iter()
                    .map(|v| {
                        let shape = if v.fields.len() <= hidden {
                            v.name.clone()
                        } else {
                            let fs: Vec<String> = v.fields.iter().skip(hidden).map(|f| format!("{} {}", f.ty.map_or("?".into(), |t| demangle(&t.to_string())), f.name)).collect();
                            format!("{}({})", v.name, fs.join(", "))
                        };
                        VariantRec { name: v.name.clone(), shape, span: v.name_span }
                    })
                    .collect(),
            });
        }
        for (name, &id) in &g.funcs_by_name {
            let sig = &g.sigs[id];
            let text = short(src, sig.span);
            let name_span = name_in(src, sig.span, name);
            ix.funcs.push(FuncRec { name: demangle(name), sig: text.to_string(), name_span });
        }
        for (name, t) in &g.mono.templates {
            let text = short(src, t.head.span);
            ix.funcs.push(FuncRec { name: demangle(name), sig: text.to_string(), name_span: t.head.name_span });
        }
    });
}
