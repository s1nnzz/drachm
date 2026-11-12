//! Checking strings: literals and interpolation, methods, operators, and
//! `io::outf` formats.

use super::*;
use crate::ast::StrPart;
use crate::hir::StrFn;

/// What `string` methods exist, for "no method" errors.
const METHODS: &str = "`len`, `isEmpty`, `reverse`, `sub`, `upper`, `lower`, `trim`, `find`, `contains`, \
    `startsWith`, `endsWith`, `count`, `join`, `replace`, `replaceLimit`, `insert`, `remove`, `repeat`, \
    `padLeft`, `padRight`, `text` and `to`";

/// Names other languages use for these methods.
fn method_hint(name: &str) -> Option<&'static str> {
    Some(match name {
        "length" | "size" | "count_chars" => "len",
        "empty" | "is_empty" => "isEmpty",
        "substring" | "substr" | "slice" => "sub",
        "toUpperCase" | "toUpper" | "to_uppercase" | "uppercase" => "upper",
        "toLowerCase" | "toLower" | "to_lowercase" | "lowercase" => "lower",
        "strip" => "trim",
        "indexOf" | "index" => "find",
        "includes" | "has" => "contains",
        "starts_with" | "hasPrefix" => "startsWith",
        "ends_with" | "hasSuffix" => "endsWith",
        "concat" | "append" => "join",
        "replaceAll" | "replace_all" => "replace",
        "toString" | "to_string" | "str" => "text",
        _ => return None,
    })
}

/// "2 placeholders", "1 value".
fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn text_of(v: hir::Expr) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Text(Box::new(v)), Type::Str)
}

fn str_call(f: StrFn, args: Vec<hir::Expr>) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::StrCall(f, args), f.ret())
}

/// A `%` placeholder in an `io::outf` format.
struct Hole {
    span: Span,
    /// `%-8`: pad on the right.
    left: bool,
    /// `%08`: pad with zeros after the sign.
    zero: bool,
    width: Option<u64>,
    /// `%.2`
    places: Option<u32>,
    /// `%x`, `%X`, `%b`, `%o`: the base and whether letters are capitals.
    base: Option<(u32, bool)>,
}

enum FmtItem {
    Text(String),
    Hole(Hole),
}

impl Checker<'_> {
    pub(super) fn string_literal(&mut self, parts: &[StrPart]) -> Option<Val> {
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut interpolated = false;
        let mut ok = true;
        for part in parts {
            match part {
                StrPart::Raw(t, _) => text.push_str(t),
                StrPart::Escape(c, _) => text.push(*c),
                StrPart::Interp(e) => {
                    interpolated = true;
                    if !text.is_empty() {
                        pieces.push(str_lit(std::mem::take(&mut text)));
                    }
                    match self.as_text(e) {
                        Some(p) => pieces.push(p),
                        None => ok = false,
                    }
                }
            }
        }
        if !interpolated {
            return Some(Val::Expr(str_lit(text)));
        }
        if !text.is_empty() {
            pieces.push(str_lit(text));
        }
        if !ok {
            return None;
        }
        Some(Val::Expr(self.join(pieces)))
    }

    /// The value of `${e}` (or an `io::outf` value) as text, as a place:
    /// strings as themselves, numbers and bools as by `::text()`.
    fn as_text(&mut self, e: &ast::Expr) -> Option<hir::Expr> {
        let v = self.expr(e)?;
        let v = self.materialise(v, e.span)?;
        match v.ty {
            Type::Str => Some(v),
            Type::None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0104, e.span, "this produces no value to turn into text").note("returns `none`"),
                );
                None
            }
            Type::Struct { .. } => {
                self.struct_not_text(v.ty, e.span);
                None
            }
            Type::Enum { .. } => {
                self.enum_not_text(v.ty, e.span);
                None
            }
            _ => Some(text_of(v)),
        }
    }

    /// A struct where text is wanted (`${p}`, `io::outf`'s `%`).
    // OPEN: whether these use a struct's own `text()` method instead.
    fn struct_not_text(&mut self, ty: Type, span: Span) {
        let Type::Struct { id, name } = ty else { return };
        let what = self.quote(span, "this value");
        let mut d = Diagnostic::at(&codes::E0101, span, format!("{what} is a `{name}`, which cannot be turned into text by itself"))
            .note(format!("this is `{name}`"));
        if self.g.method(id, "text").is_some() {
            d = d.help(format!("call its `text` method: `{}::text()`", self.text(span)));
        } else {
            d = d.help("put its fields in the text one by one");
        }
        self.diags.push(d);
    }

    /// One new string made of `pieces` (places) in order.
    fn join(&mut self, mut pieces: Vec<hir::Expr>) -> hir::Expr {
        match pieces.len() {
            0 => str_lit(String::new()),
            1 => {
                let p = pieces.pop().unwrap();
                // A place is copied by joining it on its own: that makes a new
                // string without using the place up. A value made only to be
                // joined on its own is already the result.
                if p.is_place() { hir::Expr::new(hir::ExprKind::Join(vec![p]), Type::Str) } else { p }
            }
            _ => hir::Expr::new(hir::ExprKind::Join(pieces), Type::Str),
        }
    }

    /// `recv::name(args)` on a string.
    pub(super) fn string_method(&mut self, recv: hir::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr]) -> Option<Val> {
        if name == "to" {
            return self.parse_text(recv, name_span, targs, args);
        }
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`{name}` takes no type arguments")));
        }
        let f = match name {
            "text" => {
                if let Some(a) = args.first() {
                    self.diags.push(Diagnostic::at(&codes::E0103, a.span, "`text` takes no arguments"));
                }
                // A new string made from `recv`, which stays where it is.
                return Some(Val::Expr(self.join(vec![recv])));
            }
            "len" => StrFn::Len,
            "isEmpty" => StrFn::IsEmpty,
            "reverse" => StrFn::Reverse,
            "sub" => StrFn::Sub,
            "upper" => StrFn::Upper,
            "lower" => StrFn::Lower,
            "trim" => StrFn::Trim,
            "find" => StrFn::Find,
            "contains" => StrFn::Contains,
            "startsWith" => StrFn::StartsWith,
            "endsWith" => StrFn::EndsWith,
            "count" => StrFn::Count,
            "join" => StrFn::Concat,
            "replace" => StrFn::Replace,
            "replaceLimit" => StrFn::ReplaceLimit,
            "insert" => StrFn::Insert,
            "remove" => StrFn::Remove,
            "repeat" => StrFn::Repeat,
            "padLeft" => StrFn::PadLeft,
            "padRight" => StrFn::PadRight,
            "split" | "lines" | "chars" => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, name_span, format!("`{name}` returns a list of strings, and lists are not supported yet"))
                        .note("needs lists"),
                );
                for a in args {
                    self.expr(a);
                }
                return None;
            }
            _ => {
                let help = match method_hint(name) {
                    Some(m) => format!("Drachm calls this `{m}`"),
                    None => format!("string methods are {METHODS}"),
                };
                self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("`string` has no method `{name}`")).help(help));
                for a in args {
                    self.expr(a);
                }
                return None;
            }
        };
        let params = &f.params()[1..];
        let n = params.len();
        // The fill text of `padLeft` and `padRight` may be left out: one space.
        let fill_optional = matches!(f, StrFn::PadLeft | StrFn::PadRight);
        if args.len() != n && !(fill_optional && args.len() == n - 1) {
            let takes = if fill_optional { format!("{} or {n} arguments", n - 1) } else { count(n, "argument") };
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, format!("`{name}` takes {takes}, but {} given", args.len()))
                    .note(format!("takes {takes}")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let mut out = vec![recv];
        let mut ok = true;
        for (a, &p) in args.iter().zip(params) {
            match self.coerce(a, p) {
                Some(e) if p == Type::Str => out.push(e),
                Some(e) => out.push(e),
                None => ok = false,
            }
        }
        if args.len() < n {
            out.push(str_lit(" ".into()));
        }
        ok.then(|| Val::Expr(str_call(f, out)))
    }

    /// `text::to<T>()`: the number the text reads as. Bad text stops the
    /// program with a panic until failing calls exist (step 5).
    fn parse_text(&mut self, recv: hir::Expr, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr]) -> Option<Val> {
        for a in args {
            self.diags.push(Diagnostic::at(&codes::E0103, a.span, "`to` takes no arguments"));
        }
        let [target] = targs else {
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, "`to` needs the type to convert to").help("for example `text::to<uint32>()`"),
            );
            return None;
        };
        let to = self.g.resolve(target, self.diags)?;
        if to.mentions_unit() {
            self.diags.push(Diagnostic::at(&codes::E0901, target.span, "reading text as a number with a unit is not supported yet"));
            return None;
        }
        if to == Type::Bool {
            self.diags.push(Diagnostic::at(&codes::E0901, target.span, "reading text as `bool` is not supported yet"));
            return None;
        }
        if !to.is_numeric() {
            self.diags.push(Diagnostic::at(&codes::E0101, target.span, format!("`to` on text reads a number, not `{to}`")));
            return None;
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::ParseText { text: Box::new(recv), to }, to)))
    }

    /// A two-sided operator with a string on the left: `+` joins, the
    /// comparisons compare bytes.
    pub(super) fn string_binary(&mut self, op: BinOp, lhs: hir::Expr, lo: Origin, rv: Val, ro: Origin) -> Option<Val> {
        let sym = op.symbol();
        if op != BinOp::Add && op.class() != OpClass::Comparison {
            let wants = if matches!(op.class(), OpClass::Arithmetic) { "numbers" } else { "whole numbers" };
            let mut d = Diagnostic::at(&codes::E0101, lo.span, format!("`{sym}` needs {wants}, found `string`")).note("this is `string`");
            if op == BinOp::Mul {
                d = d.help("repeat text with `::repeat(n)`");
            } else if op.class() == OpClass::Arithmetic {
                d = d.help("strings join with `+`");
            }
            self.diags.push(d);
            return None;
        }
        let rhs = match rv {
            Val::Expr(r) if r.ty == Type::Str => r,
            other => {
                let found = match &other {
                    Val::Const(_) | Val::UConst(..) => "a number".to_string(),
                    Val::Expr(r) => format!("`{}`", r.ty),
                    Val::NoneLit(_) => "`none`".to_string(),
                };
                let message = if op == BinOp::Add {
                    format!("`+` joins strings, but the right side is {found}")
                } else {
                    format!("`{sym}` has `string` on the left but {found} on the right")
                };
                let mut d = Diagnostic::at(&codes::E0101, ro.span, message).note(format!("this is {found}")).label(lo.span, "this is `string`");
                let has_text = match &other {
                    Val::Const(_) | Val::UConst(..) => true,
                    Val::NoneLit(_) => false,
                    Val::Expr(r) => match r.ty.struct_id() {
                        Some(s) => self.g.method(s, "text").is_some(),
                        None => r.ty != Type::None,
                    },
                };
                if has_text {
                    d = d.help(self.text_fix(ro));
                }
                self.diags.push(d);
                return None;
            }
        };
        let args = vec![lhs, rhs];
        let e = match op {
            BinOp::Add => str_call(StrFn::Concat, args),
            BinOp::Eq => str_call(StrFn::Eq, args),
            BinOp::Ne => hir::Expr::new(hir::ExprKind::Not(Box::new(str_call(StrFn::Eq, args))), Type::Bool),
            _ => {
                let hop = match op {
                    BinOp::Lt => hir::BinOp::Lt,
                    BinOp::Le => hir::BinOp::Le,
                    BinOp::Gt => hir::BinOp::Gt,
                    _ => hir::BinOp::Ge,
                };
                let cmp = str_call(StrFn::Cmp, args);
                let zero = int_lit(0, Type::S32);
                hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(cmp), Box::new(zero)), Type::Bool)
            }
        };
        Some(Val::Expr(e))
    }

    /// `io::outf(format, values...)`. The format must be a literal; it is
    /// taken apart here, and the call becomes one `Out` of the joined text.
    pub(super) fn outf(&mut self, args: &[ast::Expr], span: Span) -> Option<Val> {
        let Some((fmt, values)) = args.split_first() else {
            self.diags.push(
                Diagnostic::at(&codes::E0103, span, "`io::outf` needs a format string")
                    .help("for example `io::outf(\"% apples\\n\", count)`"),
            );
            return None;
        };
        let parts = match &fmt.kind {
            ExprKind::Str(parts) if !parts.iter().any(|p| matches!(p, StrPart::Interp(_))) => parts,
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, fmt.span, "an `io::outf` format that is not a plain string literal is not supported yet")
                        .note("not a plain string literal")
                        .help("write the format as a literal, or make the text with an `f\"...\"` string and print it with `io::out`"),
                );
                for a in args {
                    self.expr(a);
                }
                return None;
            }
        };
        let items = self.parse_format(parts);
        let holes: Vec<Span> = items
            .iter()
            .flatten()
            .filter_map(|i| match i {
                FmtItem::Hole(h) => Some(h.span),
                FmtItem::Text(_) => None,
            })
            .collect();
        let (h, n) = (holes.len(), values.len());
        let counts_match = items.is_none() || h == n;
        if !counts_match {
            let message = format!("the format has {}, but {} {} given", count(h, "placeholder"), count(n, "value"), if n == 1 { "is" } else { "are" });
            let d = if n < h {
                Diagnostic::at(&codes::E0103, holes[n], message)
                    .note("this `%` has no value")
                    .help("give a value for each `%`; write `\\%` for a percent sign")
            } else {
                Diagnostic::at(&codes::E0103, values[h].span, message)
                    .note("no `%` for this value")
                    .label(fmt.span, format!("has {}", count(h, "placeholder")))
                    .help("add a `%` to the format for each value")
            };
            self.diags.push(d);
        }

        let mut pieces = Vec::new();
        let mut ok = items.is_some() && counts_match;
        let mut values = values.iter();
        for item in items.iter().flatten() {
            match item {
                FmtItem::Text(t) => pieces.push(str_lit(t.clone())),
                FmtItem::Hole(hole) => match values.next().map(|v| self.fmt_value(hole, v)) {
                    Some(Some(p)) => pieces.push(p),
                    _ => ok = false,
                },
            }
        }
        // Values without a placeholder, or after a bad format, are still checked.
        for v in values {
            self.expr(v);
        }
        if !ok {
            return None;
        }
        let text = self.join(pieces);
        let arg = text;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Out { newline: false, arg: Box::new(arg) }, Type::None)))
    }

    /// Split a format into text and placeholders. Only `%` written directly
    /// in the source is a placeholder; an escape (`\%` in particular) is
    /// always text. `None` if anything was wrong (reported).
    fn parse_format(&mut self, parts: &[StrPart]) -> Option<Vec<FmtItem>> {
        let mut items = Vec::new();
        let mut text = String::new();
        let mut ok = true;
        for part in parts {
            let (raw, span) = match part {
                StrPart::Raw(raw, span) => (raw, span),
                StrPart::Escape(c, _) => {
                    text.push(*c);
                    continue;
                }
                StrPart::Interp(_) => unreachable!("checked by outf"),
            };
            let mut i = 0;
            while let Some(c) = raw[i..].chars().next() {
                if c != '%' {
                    text.push(c);
                    i += c.len_utf8();
                    continue;
                }
                if !text.is_empty() {
                    items.push(FmtItem::Text(std::mem::take(&mut text)));
                }
                let (hole, end) = self.placeholder(raw, i, span.start);
                match hole {
                    Some(h) => items.push(FmtItem::Hole(h)),
                    None => ok = false,
                }
                i = end;
            }
        }
        if !text.is_empty() {
            items.push(FmtItem::Text(text));
        }
        ok.then_some(items)
    }

    /// The placeholder at byte `at` of `raw` (a `%`), where `raw` starts at
    /// byte `base` of the file. Returns it (`None` if it is invalid, which is
    /// reported) and the byte where it ends.
    ///
    /// The syntax is `%`, then `-` or `0` (each only when a digit follows),
    /// a width, `.N` (only when a digit follows the dot), and one of
    /// `x X b o`. So `%` followed by a space, `.`, `-` or punctuation is a
    /// bare placeholder followed by that text.
    fn placeholder(&mut self, raw: &str, at: usize, base: usize) -> (Option<Hole>, usize) {
        let b = raw.as_bytes();
        let digit = |k: usize| b.get(k).is_some_and(u8::is_ascii_digit);
        let mut j = at + 1;
        let left = b.get(j) == Some(&b'-') && digit(j + 1);
        if left {
            j += 1;
        }
        let zero = b.get(j) == Some(&b'0') && digit(j + 1);
        if zero {
            j += 1;
        }
        let w0 = j;
        while digit(j) {
            j += 1;
        }
        let width = &raw[w0..j];
        let mut places = None;
        if b.get(j) == Some(&b'.') && digit(j + 1) {
            j += 1;
            let p0 = j;
            while digit(j) {
                j += 1;
            }
            places = Some(&raw[p0..j]);
        }
        let radix = match b.get(j) {
            Some(b'x') => Some((16, false)),
            Some(b'X') => Some((16, true)),
            Some(b'b') => Some((2, false)),
            Some(b'o') => Some((8, false)),
            _ => None,
        };
        if radix.is_some() {
            j += 1;
        }
        let spec = &raw[at..j];
        let span = Span::new(base + at, base + j);
        let bad = |cx: &mut Self, span: Span, message: String, help: &str| {
            cx.diags.push(Diagnostic::at(&codes::E0009, span, message).note("in this placeholder").help(help));
        };

        // OPEN (language): a letter right after a placeholder could be meant
        // as text ("%d", "%s", "100%x"); x X b o always choose a base, and any
        // other letter is rejected until the owner decides.
        if radix.is_none()
            && let Some(c) = raw[j..].chars().next().filter(char::is_ascii_alphabetic)
        {
            let span = Span::new(base + at, base + j + 1);
            let help = "options are a width (`%8`, `%-8`, `%08`), decimals (`%.2`) and a base (`%x`, `%X`, `%b`, `%o`); \
                        to put a letter right after a value, use an `f\"...\"` string";
            bad(self, span, format!("`{spec}{c}` is not a format placeholder"), help);
            return (None, j + 1);
        }
        // OPEN (language): `%%` reads like C's percent sign; it is rejected
        // rather than taken as two placeholders.
        if j == at + 1 && b.get(j) == Some(&b'%') {
            let span = Span::new(base + at, base + j + 1);
            bad(self, span, "`%%` is two placeholders, not a percent sign".into(), "write `\\%` for a percent sign");
            return (None, j + 1);
        }
        let mut ok = true;
        if left && zero {
            bad(self, span, format!("`{spec}` asks for zeros on the right"), "zeros pad only on the left (`%08`); `%-8` pads with spaces on the right");
            ok = false;
        }
        let width = match width.parse::<u64>() {
            _ if width.is_empty() => None,
            Ok(0) => {
                bad(self, span, format!("`{spec}` has a width of 0"), "a width says how many characters to pad to, like `%8`");
                ok = false;
                None
            }
            Ok(w) => Some(w),
            Err(_) => {
                bad(self, span, format!("the width in `{spec}` is too large"), "use a smaller width");
                ok = false;
                None
            }
        };
        let places = match places.map(|p| p.parse::<u32>().ok().filter(|&n| n <= i32::MAX as u32)) {
            None => None,
            Some(Some(n)) => Some(n),
            Some(None) => {
                bad(self, span, format!("`{spec}` asks for too many decimal places"), "use fewer decimal places");
                ok = false;
                None
            }
        };
        if places.is_some() && radix.is_some() {
            bad(self, span, format!("`{spec}` mixes decimal places with a base"), "decimal places (`%.2`) are for floats, bases (`%x`) for whole numbers");
            ok = false;
        }
        (ok.then_some(Hole { span, left, zero, width, places, base: radix }), j)
    }

    /// The text an `io::outf` value prints as, for its placeholder, as a place.
    fn fmt_value(&mut self, hole: &Hole, arg: &ast::Expr) -> Option<hir::Expr> {
        let v = self.expr(arg)?;
        let v = self.materialise(v, arg.span)?;
        let ty = v.ty;
        let spec = self.text(hole.span).to_string();
        let mismatch = |cx: &mut Self, wants: &str, help: String| {
            let d = Diagnostic::at(&codes::E0108, arg.span, format!("`{spec}` needs {wants}, found `{ty}`"))
                .note(format!("this is `{ty}`"))
                .label(hole.span, "the placeholder");
            cx.diags.push(if help.is_empty() { d } else { d.help(help) });
        };
        if ty == Type::None {
            self.diags.push(Diagnostic::at(&codes::E0104, arg.span, "this produces no value to print").note("returns `none`"));
            return None;
        }
        if ty.struct_id().is_some() {
            self.struct_not_text(ty, arg.span);
            return None;
        }
        if ty.enum_id().is_some() {
            self.enum_not_text(ty, arg.span);
            return None;
        }
        let text = if let Some((base, upper)) = hole.base {
            if !ty.is_int() {
                mismatch(self, "a whole number", String::new());
                return None;
            }
            hir::Expr::new(hir::ExprKind::Radix { value: Box::new(v), base, upper }, Type::Str)
        } else if let Some(n) = hole.places {
            if !ty.is_float() {
                let help = if ty.is_int() {
                    let fix = self.text(arg.span);
                    format!("decimal places are for floats; convert it with `{fix}::to<float64>()`, or drop the `.{n}`")
                } else {
                    String::new()
                };
                mismatch(self, "a float", help);
                return None;
            }
            let v = if ty == Type::F64 { v } else { hir::Expr::new(hir::ExprKind::Convert(Box::new(v)), Type::F64) };
            str_call(StrFn::Fixed, vec![v, int_lit(n as u128, Type::U32)])
        } else if ty == Type::Str {
            v
        } else {
            text_of(v)
        };
        let Some(width) = hole.width else { return Some(text) };
        let width = int_lit(width as u128, Type::Int { bits: 64, signed: false });
        let padded = if hole.zero {
            if !ty.is_numeric() {
                let w = &spec[2..];
                mismatch(self, "a number", format!("only numbers pad with zeros; `%{w}` pads with spaces"));
                return None;
            }
            let t = text;
            str_call(StrFn::ZeroPad, vec![t, width])
        } else {
            let t = text;
            let f = if hole.left { StrFn::PadRight } else { StrFn::PadLeft };
            str_call(f, vec![t, width, str_lit(" ".into())])
        };
        Some(padded)
    }
}
