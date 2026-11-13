use num_traits::ToPrimitive;

use crate::ast::*;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::types;
use crate::lexer::{StrPiece, Tok, Token};

/// Never fails: syntax errors are reported to `diags` and the parser carries
/// on at the next statement or function, so several can be found in one go.
pub fn parse(tokens: Vec<Token>, diags: &mut Vec<Diagnostic>) -> Program {
    let mut p = Parser { tokens, pos: 0, diags, optionals: Vec::new(), generics: Vec::new() };
    let mut program = Program {
        imports: Vec::new(),
        exports: Vec::new(),
        units: Vec::new(),
        funcs: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        interfaces: Vec::new(),
        impls: Vec::new(),
        failures: Vec::new(),
        defs: Vec::new(),
        consts: Vec::new(),
        unsafe_allowed: false,
        optionals: Vec::new(),
        generics: Vec::new(),
    };
    while p.peek() != &Tok::Eof {
        let start = p.pos;
        // `out` before a declaration (`out struct Vector3 { }`) exports it.
        let out_span = if p.peek() == &Tok::Kw("out") && !p.at_export_list() { Some(p.bump().span) } else { None };
        let consts_before = program.consts.len();
        let before = [
            program.funcs.len(),
            program.structs.len(),
            program.enums.len(),
            program.interfaces.len(),
            program.failures.len(),
            program.units.len(),
            program.impls.len() + program.defs.len() + program.imports.len(),
        ];
        let item = if p.peek() == &Tok::Kw("in") {
            p.import().map(|i| program.imports.push(i))
        } else if p.peek() == &Tok::Kw("out") {
            p.export_list().map(|e| program.exports.extend(e))
        } else if p.peek() == &Tok::Kw("struct") {
            p.struct_decl().map(|s| program.structs.push(s))
        } else if p.peek() == &Tok::Kw("interface") {
            p.interface_decl().map(|i| program.interfaces.push(i))
        } else if p.at_impl_item() {
            p.impl_decl().map(|i| program.impls.push(i))
        } else if p.peek() == &Tok::Kw("enum") {
            p.enum_decl().map(|e| program.enums.push(e))
        } else if p.peek() == &Tok::Kw("failure") {
            p.failure_decl().map(|f| program.failures.push(f))
        } else if p.at_unit_item() {
            p.unit_decl().map(|u| program.units.push(u))
        } else if p.at_def_item() {
            p.def_decl().map(|d| program.defs.push(d))
        } else if p.at_const_item() {
            p.const_decl().map(|c| program.consts.push(c))
        } else {
            p.func().map(|f| program.funcs.push(f))
        };
        if let (Some(out), Ok(())) = (out_span, &item) {
            let named = match before {
                _ if program.consts.len() > consts_before => program.consts.last().map(|x| (x.name.clone(), x.name_span)),
                [f, ..] if program.funcs.len() > f => program.funcs.last().map(|x| (x.name.clone(), x.name_span)),
                [_, s, ..] if program.structs.len() > s => program.structs.last().map(|x| (x.name.clone(), x.name_span)),
                [_, _, e, ..] if program.enums.len() > e => program.enums.last().map(|x| (x.name.clone(), x.name_span)),
                [_, _, _, i, ..] if program.interfaces.len() > i => program.interfaces.last().map(|x| (x.name.clone(), x.name_span)),
                [_, _, _, _, f, ..] if program.failures.len() > f => program.failures.last().map(|x| (x.name.clone(), x.name_span)),
                [_, _, _, _, _, u, _] if program.units.len() > u => program.units.last().map(|x| (x.name.clone(), x.name_span)),
                _ => None,
            };
            match named {
                Some((name, at)) => program.exports.push(Export { name, alias: None, span: out.to(at), on_decl: true }),
                None => p.diags.push(
                    Diagnostic::at(&codes::E0003, out, "`out` cannot go here")
                        .help("write `out` before a function, struct, enum, interface, failure or unit, or list names: `out A, B;`"),
                ),
            }
        }
        if let Err(d) = item {
            p.diags.push(d);
            // Skip to something that looks like the start of the next item.
            if p.pos == start {
                p.bump();
            }
            while !p.at_item_start() && p.peek() != &Tok::Eof {
                p.bump();
            }
        }
    }
    program.optionals = p.optionals;
    program.generics = p.generics;
    program
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    diags: &'a mut Vec<Diagnostic>,
    /// Every `T?` seen so far (see `ast::Program::optionals`).
    optionals: Vec<TypeName>,
    /// Every type with type arguments seen so far (`ast::Program::generics`).
    generics: Vec<TypeName>,
}

type Parsed<T> = Result<T, Diagnostic>;

/// State of a type lookahead. `half`: the type just read ended on the first
/// `>` of a `>>`.
struct Look {
    half: bool,
}

/// `uint8`..`uint128`, `sint*`, `float*`: the types a `<` after them opens a unit for.
fn is_number_name(name: &str) -> bool {
    ["uint", "sint", "float"].iter().any(|k| name.strip_prefix(k).is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())))
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if t.tok != Tok::Eof {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, sym: &'static str) -> bool {
        if self.peek() == &Tok::Sym(sym) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: &'static str) -> Parsed<Span> {
        if self.peek() == &Tok::Sym(sym) { Ok(self.bump().span) } else { Err(self.unexpected(&format!("`{sym}`"))) }
    }

    /// "expected X, found Y" at the current token.
    fn unexpected(&self, what: &str) -> Diagnostic {
        Diagnostic::at(&codes::E0003, self.span(), format!("expected {what}, found {}", self.peek().describe()))
    }

    fn ident(&mut self, what: &str) -> Parsed<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(name) => Ok((name, self.bump().span)),
            Tok::Kw(k) => Err(self
                .unexpected(what)
                .help(format!("`{k}` is a reserved word and cannot be used as a name"))),
            _ => Err(self.unexpected(what)),
        }
    }

    /// A name that may be qualified by modules: `Vector3`, `phys::Vector3`.
    /// The parts are joined with `::` in one string; the resolver
    /// (`resolve.rs`) turns it into the name the checker uses.
    fn qual_ident(&mut self, what: &str) -> Parsed<(String, Span)> {
        let (mut name, mut span) = self.ident(what)?;
        while self.peek() == &Tok::Sym("::") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            let (part, part_span) = self.ident(what)?;
            name = format!("{name}::{part}");
            span = span.to(part_span);
        }
        Ok((name, span))
    }

    /// The token offset after the name at `n` and any `::name` parts after it.
    fn qual_end(&self, n: usize) -> usize {
        let mut end = n + 1;
        while self.peek_at(end) == &Tok::Sym("::") && matches!(self.peek_at(end + 1), Tok::Ident(_)) {
            end += 2;
        }
        end
    }

    /// A type: `&`, `&mut` and `*` marks, then a name or the keyword `none`.
    fn ty(&mut self) -> Parsed<TypeName> {
        // `maybe &T`: an optional that holds a borrow (the same type as `Optional<&T>`).
        if self.peek() == &Tok::Kw("maybe") {
            let start = self.bump().span;
            if self.peek() != &Tok::Sym("&") {
                return Err(self.unexpected("`&` after `maybe`").help("an optional that holds a borrow is written `maybe &string`; an optional value is `string?`"));
            }
            let inner = self.ty()?;
            let span = start.to(inner.span);
            let args = vec![inner];
            self.generics.push(TypeName { args: args.clone(), ..TypeName::plain("Optional", span) });
            return Ok(TypeName { args, ..TypeName::plain("Optional", span) });
        }
        let mut mods = Vec::new();
        loop {
            let span = self.span();
            match self.peek() {
                Tok::Sym("&") => {
                    self.bump();
                    let mutable = self.eat_kw("mut");
                    if matches!(self.peek(), Tok::Sym("&" | "&&" | "*" | "**")) {
                        return Err(borrow_of_borrow(span.to(self.span())));
                    }
                    mods.push((TypeMod::Ref { mutable }, span.to(self.prev_span())));
                }
                Tok::Sym("&&") => return Err(borrow_of_borrow(span)),
                Tok::Sym("*") => {
                    self.bump();
                    // `*fixed uint32 p`: the word `fixed` before a type name.
                    // `*fixed p` (a struct called `fixed`, then a name) is not.
                    let fixed = matches!(self.peek(), Tok::Ident(w) if w == "fixed")
                        && matches!(self.peek_at(1), Tok::Ident(_))
                        && matches!(self.peek_at(2), Tok::Ident(_) | Tok::Sym("<"));
                    if fixed {
                        self.bump();
                        mods.push((TypeMod::FixedPtr, span.to(self.prev_span())));
                    } else {
                        mods.push((TypeMod::Ptr, span));
                    }
                }
                Tok::Sym("**") => {
                    self.bump();
                    mods.push((TypeMod::Ptr, span));
                    mods.push((TypeMod::Ptr, span));
                }
                _ => break,
            }
        }
        if self.eat("(") {
            let start = self.prev_span();
            let mut params = Vec::new();
            while self.peek() != &Tok::Sym(")") {
                params.push(self.ty()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            self.expect("=>")?;
            let ret = self.ty()?;
            if self.peek() == &Tok::Kw("fails") {
                return Err(Diagnostic::at(&codes::E0901, self.span(), "a failing function type is not supported yet"));
            }
            let span = start.to(self.prev_span());
            return Ok(TypeName { mods, func: Some(Box::new(FuncType { params, ret })), ..TypeName::plain("", span) });
        }
        if self.peek() == &Tok::Kw("none") {
            let span = self.bump().span;
            let unit = self.type_unit()?;
            return Ok(TypeName { mods, unit, ..TypeName::plain("none", span) });
        }
        let (name, span) = if self.peek() == &Tok::Kw("Self") {
            ("Self".to_string(), self.bump().span)
        } else if matches!(self.peek(), Tok::Ident(w) if is_number_name(w)) {
            self.ident("a type")?
        } else {
            self.qual_ident("a type")?
        };
        // A `<` after a number type opens a unit (`float64<m/s>`); after any
        // other name it opens type arguments (`Map<string, uint32>`).
        let (mut unit, mut args) = (None, Vec::new());
        if is_number_name(&name) {
            unit = self.type_unit()?;
        } else if self.peek() == &Tok::Sym("<") && self.type_start_at(1) {
            args = self.type_args()?;
        }
        let dims = self.type_dims()?;
        let optional = self.type_optional(&name, span, &unit, &args, &dims)?;
        if !args.is_empty() {
            self.generics.push(TypeName { args: args.clone(), dims: dims.clone(), ..TypeName::plain(name.clone(), span) });
        }
        Ok(TypeName { mods, unit, optional, args, dims, ..TypeName::plain(name, span) })
    }

    /// Can a type argument start at token offset `n`?
    fn type_start_at(&self, n: usize) -> bool {
        match self.peek_at(n) {
            Tok::Sym("(") => self.after_open_paren(n + 1).is_some_and(|i| self.peek_at(i) == &Tok::Sym("=>")),
            Tok::Kw("maybe") => self.peek_at(n + 1) == &Tok::Sym("&"),
            tok => matches!(tok, Tok::Ident(_) | Tok::Kw("none") | Tok::Sym("&" | "*" | "**" | ">" | "-") | Tok::Num(..)),
        }
    }

    /// `<A, B>`, at the `<`. An integer is an argument too (`Array<T, 4>`).
    fn type_args(&mut self) -> Parsed<Vec<TypeName>> {
        let open = self.bump().span;
        if self.peek() == &Tok::Sym(">") {
            return Err(Diagnostic::at(&codes::E0003, open.to(self.span()), "expected a type between `<` and `>`")
                .help("write the type arguments, for example `Stack<uint32>`"));
        }
        let mut args = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None) | Tok::Sym("-") => args.push(self.value_arg()?),
                Tok::Ident(_) if self.value_op_at(1) => args.push(self.value_arg()?),
                _ if self.unit_arg_at(0) => {
                    let start = self.bump().span;
                    let spec = self.unit_spec()?;
                    args.push(TypeName { unit_arg: Some(spec), ..TypeName::plain("", start.to(self.prev_span())) });
                }
                _ => {
                    let ty = self.ty()?;
                    if ty.func.is_some() {
                        return Err(Diagnostic::at(&codes::E0901, ty.span, "a function type as a type argument is not supported yet"));
                    }
                    args.push(ty);
                }
            }
            if !self.eat(",") {
                break;
            }
        }
        if !matches!(self.peek(), Tok::Sym(">" | ">>")) {
            return Err(self.unexpected("`,` or `>`").label(open, "the type arguments start here"));
        }
        self.close_angle()?;
        Ok(args)
    }

    /// Does an arithmetic operator (`+ - * / %`) stand at token offset `n`?
    fn value_op_at(&self, n: usize) -> bool {
        matches!(self.peek_at(n), Tok::Sym("+" | "-" | "*" | "/" | "%"))
    }

    /// A value argument: a constant expression (`4`, `-1`, `SIZE * 2`).
    fn value_arg(&mut self) -> Parsed<TypeName> {
        let e = self.const_sum()?;
        let span = e.span();
        let name = match &e {
            ConstExpr::Num(v, _) => v.to_string(),
            _ => String::new(),
        };
        Ok(TypeName { is_number: true, val: Some(Box::new(e)), ..TypeName::plain(name, span) })
    }

    fn const_sum(&mut self) -> Parsed<ConstExpr> {
        let mut left = self.const_product()?;
        while let Tok::Sym(op @ ("+" | "-")) = self.peek().clone() {
            self.bump();
            let right = self.const_product()?;
            let span = left.span().to(right.span());
            let op = if op == "+" { BinOp::Add } else { BinOp::Sub };
            left = ConstExpr::Bin(op, Box::new(left), Box::new(right), span);
        }
        Ok(left)
    }

    fn const_product(&mut self) -> Parsed<ConstExpr> {
        let mut left = self.const_unary()?;
        while let Tok::Sym(op @ ("*" | "/" | "%")) = self.peek().clone() {
            self.bump();
            let right = self.const_unary()?;
            let span = left.span().to(right.span());
            let op = match op {
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                _ => BinOp::Rem,
            };
            left = ConstExpr::Bin(op, Box::new(left), Box::new(right), span);
        }
        Ok(left)
    }

    fn const_unary(&mut self) -> Parsed<ConstExpr> {
        match self.peek().clone() {
            Tok::Sym("-") => {
                let start = self.bump().span;
                let inner = self.const_unary()?;
                let span = start.to(inner.span());
                Ok(ConstExpr::Neg(Box::new(inner), span))
            }
            Tok::Sym("~") => {
                let start = self.bump().span;
                let inner = self.const_unary()?;
                let span = start.to(inner.span());
                Ok(ConstExpr::Not(Box::new(inner), span))
            }
            Tok::Sym("(") => {
                let open = self.bump().span;
                let inner = self.const_sum()?;
                self.expect(")")?;
                let _ = open;
                Ok(inner)
            }
            Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => {
                let span = self.bump().span;
                Ok(ConstExpr::Num(v, span))
            }
            Tok::Ident(name) => {
                let span = self.bump().span;
                Ok(ConstExpr::Name(name, span))
            }
            _ => Err(self.unexpected("a number or a name")),
        }
    }

    /// Is a unit written as a type argument at token offset `n`: `unit` and
    /// then the start of a unit expression (`unit m/s`, `unit delta degC`)?
    /// Elsewhere `unit` is an ordinary name, so `Stack<unit>` is a type.
    fn unit_arg_at(&self, n: usize) -> bool {
        matches!(self.peek_at(n), Tok::Ident(w) if w == "unit")
            && match self.peek_at(n + 1) {
                Tok::Ident(_) | Tok::Sym("(") => true,
                Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => *v == num_bigint::BigInt::from(1),
                _ => false,
            }
    }

    /// The offset just after the unit expression that starts at `n` inside
    /// a unit type argument: it runs up to the `,` or `>` that follows.
    fn unit_arg_end(&self, mut n: usize) -> Option<usize> {
        let first = n;
        let mut depth = 0usize;
        loop {
            match self.peek_at(n) {
                Tok::Ident(_) | Tok::Sym("*" | "/" | "^" | "-") => {}
                Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None) => {}
                Tok::Kw("as") if self.as_unit_at(n) => {}
                Tok::Sym("(") => depth += 1,
                Tok::Sym(")") => depth = depth.checked_sub(1)?,
                _ => break,
            }
            n += 1;
        }
        (depth == 0 && n > first).then_some(n)
    }

    /// `[?]` and `[N]` after a type name, in order.
    fn type_dims(&mut self) -> Parsed<Vec<Dim>> {
        let mut dims = Vec::new();
        while self.dim_look(0) {
            let open = self.bump().span;
            let dim = match self.peek().clone() {
                // `[module::SIZE]`
                Tok::Ident(module) if self.peek_at(1) == &Tok::Sym("::") && matches!(self.peek_at(2), Tok::Ident(_)) && self.peek_at(3) == &Tok::Sym("]") => {
                    let first = self.bump().span;
                    self.bump();
                    let (name, last) = self.ident("a constant")?;
                    Dim::Param(types::intern(&format!("{module}::{name}")), first.to(last))
                }
                Tok::Ident(name) if self.peek_at(1) == &Tok::Sym("]") => {
                    let at = self.bump().span;
                    Dim::Param(types::intern(&name), at)
                }
                Tok::Ident(_) | Tok::Num(..) if self.peek_at(1) != &Tok::Sym("]") => {
                    let e = self.const_sum()?;
                    Dim::Calc(Box::leak(Box::new(e.clone())), e.span())
                }
                Tok::Num(NumLit { value: Const::Int(v), .. }, None) => {
                    let at = self.span();
                    let n = v.to_u64().unwrap_or(0);
                    if n == 0 {
                        return Err(Diagnostic::at(&codes::E0003, at, "an array needs at least one item").help("write a length of 1 or more, or `[?]` for a list that grows"));
                    }
                    self.bump();
                    Dim::Array(n, open.to(self.span()))
                }
                _ => {
                    self.bump();
                    Dim::List(open.to(self.span()))
                }
            };
            self.expect("]")?;
            dims.push(dim);
        }
        Ok(dims)
    }

    /// Is there a `[?]` or `[N]` at token offset `n`? (`[0]` counts: `ty` rejects it.)
    fn dim_look(&self, n: usize) -> bool {
        self.dim_end(n).is_some()
    }

    /// The offset after the `[?]`, `[4]` or `[N]` at token offset `n`.
    fn dim_end(&self, n: usize) -> Option<usize> {
        if self.peek_at(n) != &Tok::Sym("[") {
            return None;
        }
        let close = match self.peek_at(n + 1) {
            Tok::Sym("?") => n + 2,
            Tok::Ident(_) if self.peek_at(n + 2) == &Tok::Sym("::") && matches!(self.peek_at(n + 3), Tok::Ident(_)) && self.peek_at(n + 4) == &Tok::Sym("]") => n + 4,
            Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None) if self.peek_at(n + 2) == &Tok::Sym("]") => n + 2,
            Tok::Ident(_) | Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None) => self.dim_param_end(n + 1)?,
            _ => return None,
        };
        (self.peek_at(close) == &Tok::Sym("]")).then_some(close + 1)
    }

    /// `N]` or `N + 1]` at token offset `n`: the offset of the `]`, if what
    /// follows the name reads as a length (a name, numbers and `+ - * / %`).
    fn dim_param_end(&self, mut n: usize) -> Option<usize> {
        if self.peek_at(n + 1) == &Tok::Sym("]") {
            return Some(n + 1);
        }
        if !self.value_op_at(n + 1) {
            return None;
        }
        while matches!(self.peek_at(n), Tok::Ident(_) | Tok::Num(..) | Tok::Sym("+" | "-" | "*" | "/" | "%")) {
            n += 1;
        }
        (self.peek_at(n) == &Tok::Sym("]")).then_some(n)
    }

    /// The `?` after a type name: `uint32?`. Recorded for the checker, which
    /// makes one optional type per distinct base type. `T??` is E0901.
    fn type_optional(&mut self, name: &str, span: Span, unit: &Option<UnitSpec>, args: &[TypeName], dims: &[Dim]) -> Parsed<Option<Span>> {
        if self.peek() != &Tok::Sym("?") {
            return Ok(None);
        }
        let q = self.bump().span;
        if self.peek() == &Tok::Sym("?") {
            return Err(Diagnostic::at(&codes::E0901, q.to(self.span()), "an optional of an optional is not supported yet"));
        }
        self.optionals.push(TypeName { unit: unit.clone(), args: args.to_vec(), dims: dims.to_vec(), ..TypeName::plain(name, span) });
        Ok(Some(q))
    }

    /// The optional `<unit>` after a type name.
    fn type_unit(&mut self) -> Parsed<Option<UnitSpec>> {
        // Only when the tokens read as a unit up to a `>`: `bool <(Point o)`
        // is an operator method named `<`.
        if self.unit_close(0).is_none() {
            return Ok(None);
        }
        self.bump();
        let spec = self.unit_spec()?;
        self.close_angle()?;
        Ok(Some(spec))
    }

    /// `[delta] unit-expr`, inside angle brackets.
    fn unit_spec(&mut self) -> Parsed<UnitSpec> {
        let delta = matches!(self.peek(), Tok::Ident(w) if w == "delta") && matches!(self.peek_at(1), Tok::Ident(_));
        if delta {
            self.bump();
        }
        Ok(UnitSpec { delta, expr: self.unit_expr()? })
    }

    /// `term { (* | /) term }`, left to right.
    fn unit_expr(&mut self) -> Parsed<UnitExpr> {
        let first = self.unit_term()?;
        self.unit_expr_rest(first)
    }

    /// The `(* | /) term` steps that follow `lhs`.
    fn unit_expr_rest(&mut self, mut lhs: UnitExpr) -> Parsed<UnitExpr> {
        loop {
            match self.peek() {
                Tok::Sym("*") => {
                    self.bump();
                    let rhs = self.unit_term()?;
                    lhs = UnitExpr::Mul(Box::new(lhs), Box::new(rhs));
                }
                Tok::Sym("/") => {
                    self.bump();
                    let rhs = self.unit_term()?;
                    lhs = UnitExpr::Div(Box::new(lhs), Box::new(rhs));
                }
                _ => return Ok(lhs),
            }
        }
    }

    /// `NAME [^ [-] INTEGER]`, `( unit-expr )` or `1`.
    fn unit_term(&mut self) -> Parsed<UnitExpr> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.bump().span;
                // `T as unit`: the unit named `T`, even where a type parameter `T` is in scope.
                let base = if self.as_unit_at(0) {
                    self.bump();
                    self.bump();
                    UnitExpr::Explicit(name, span)
                } else {
                    UnitExpr::Name(name, span)
                };
                if !self.eat("^") {
                    return Ok(base);
                }
                let neg = self.eat("-");
                let (n, npos) = self.unit_exponent()?;
                Ok(UnitExpr::Pow(Box::new(base), if neg { -n } else { n }, span.to(npos)))
            }
            Tok::Sym("(") => {
                self.bump();
                let e = self.unit_expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) if v == num_bigint::BigInt::from(1) => {
                Ok(UnitExpr::One(self.bump().span))
            }
            _ => Err(self
                .unexpected("a unit name, `1` or `(`")
                .help("a unit is a name, a product or quotient such as `kg*m/s^2`, or `1` for a dimensionless unit")
                .label(start, "the unit starts here")),
        }
    }

    /// The whole number after `^`. It must fit in an `i32`.
    fn unit_exponent(&mut self) -> Parsed<(i32, Span)> {
        let span = self.span();
        match self.peek() {
            Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => match v.to_i32() {
                Some(n) => {
                    self.bump();
                    Ok((n, span))
                }
                None => Err(Diagnostic::at(&codes::E0003, span, "this unit exponent is too large")),
            },
            _ => Err(self.unexpected("a whole number after `^`")),
        }
    }

    /// Consume a `>` that closes angle brackets. A `>>` is split: its first
    /// `>` closes here and the second is left for the enclosing brackets.
    fn close_angle(&mut self) -> Parsed<()> {
        match self.peek() {
            Tok::Sym(">") => {
                self.bump();
                Ok(())
            }
            Tok::Sym(">>") => {
                let t = &mut self.tokens[self.pos];
                t.span = Span::new(t.span.start + 1, t.span.end);
                t.tok = Tok::Sym(">");
                Ok(())
            }
            _ => Err(self.unexpected("`>`")),
        }
    }

    /// Where a type written at token offset `n` ends (the offset after its
    /// name and any unit), if one is written there.
    fn type_end(&self, mut n: usize) -> Option<usize> {
        loop {
            match self.peek_at(n) {
                Tok::Sym("&" | "&&" | "*" | "**") => n += 1,
                Tok::Kw("mut") if n > 0 && matches!(self.peek_at(n - 1), Tok::Sym("&")) => n += 1,
                // `maybe &T`: the word is part of the type only before a borrow.
                Tok::Kw("maybe") if matches!(self.peek_at(n + 1), Tok::Sym("&")) => n += 1,
                Tok::Sym("(") => {
                    let after = self.after_open_paren(n + 1)?;
                    if self.peek_at(after) != &Tok::Sym("=>") {
                        return None;
                    }
                    n = after + 1;
                }
                Tok::Ident(_) | Tok::Kw("none") => return Some(self.name_end(n)),
                _ => return None,
            }
        }
    }

    fn after_open_paren(&self, first: usize) -> Option<usize> {
        let mut depth = 1;
        for n in first.. {
            match self.peek_at(n) {
                Tok::Sym("(") => depth += 1,
                Tok::Sym(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(n + 1);
                    }
                }
                Tok::Eof | Tok::Sym(";" | "{" | "}") => return None,
                _ => {}
            }
        }
        unreachable!()
    }

    /// The offset just after the type name at token offset `n`, with its
    /// unit part (`<m/s>`) or type arguments (`<A, B>`) and its `[?]`/`[N]`
    /// if they are written there.
    fn name_end(&self, n: usize) -> usize {
        let mut look = Look { half: false };
        match self.name_look(n, &mut look) {
            Some(end) if !look.half => end,
            _ => {
                let mut end = self.qual_end(n);
                // Every `?` counts, so that `T??` reaches `ty` and is E0901 there.
                while self.peek_at(end) == &Tok::Sym("?") {
                    end += 1;
                }
                end
            }
        }
    }

    /// Lookahead for the type whose name is at token offset `n`: the offset
    /// after it, if what follows the name reads as a unit part or argument list.
    fn name_look(&self, n: usize, look: &mut Look) -> Option<usize> {
        let mut end = self.qual_end(n);
        if self.peek_at(end) == &Tok::Sym("<") {
            let number = matches!(self.peek_at(n), Tok::Ident(w) if is_number_name(w));
            if number {
                if let Some(j) = self.unit_close(end) {
                    if self.peek_at(j) == &Tok::Sym(">>") {
                        // Closes this unit and the brackets around it.
                        if look.half {
                            return None;
                        }
                        look.half = true;
                        return Some(j);
                    }
                    end = j + 1;
                }
            } else if matches!(self.peek_at(n), Tok::Ident(_)) && self.type_start_at(end + 1) {
                end = self.args_look(end, look)?;
                if look.half {
                    return Some(end);
                }
            }
        }
        while let Some(next) = self.dim_end(end) {
            end = next;
        }
        while self.peek_at(end) == &Tok::Sym("?") {
            end += 1;
        }
        Some(end)
    }

    /// Lookahead for `<A, B>` at token offset `n`: the offset after the `>`.
    /// When the last `>` is the first half of a `>>`, `look.half` is set and
    /// the offset is that of the `>>` itself.
    fn args_look(&self, n: usize, look: &mut Look) -> Option<usize> {
        let mut j = n + 1;
        loop {
            let num = matches!(self.peek_at(j), Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None));
            let neg_num = self.peek_at(j) == &Tok::Sym("-") && matches!(self.peek_at(j + 1), Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None));
            if num || neg_num || (matches!(self.peek_at(j), Tok::Ident(_)) && self.value_op_at(j + 1)) {
                // A constant expression: names, numbers, operators, parentheses.
                while matches!(self.peek_at(j), Tok::Ident(_) | Tok::Num(..) | Tok::Sym("+" | "-" | "*" | "/" | "%" | "~" | "(" | ")")) {
                    j += 1;
                }
            } else if self.unit_arg_at(j) {
                j = self.unit_arg_end(j + 1)?;
            } else {
                loop {
                    match self.peek_at(j) {
                        Tok::Sym("&" | "*" | "**") => j += 1,
                        Tok::Kw("mut") if j > 0 && self.peek_at(j - 1) == &Tok::Sym("&") => j += 1,
                        Tok::Kw("maybe") if self.peek_at(j + 1) == &Tok::Sym("&") => j += 1,
                        Tok::Sym("(") => {
                            let after = self.after_open_paren(j + 1)?;
                            if self.peek_at(after) != &Tok::Sym("=>") {
                                return None;
                            }
                            j = after + 1;
                        }
                        _ => break,
                    }
                }
                if !matches!(self.peek_at(j), Tok::Ident(_) | Tok::Kw("none")) {
                    return None;
                }
                j = self.name_look(j, look)?;
            }
            if look.half {
                // The inner type ended on the first half of a `>>`; this is the second.
                look.half = false;
                return Some(j + 1);
            }
            match self.peek_at(j) {
                Tok::Sym(",") => j += 1,
                Tok::Sym(">") => return Some(j + 1),
                Tok::Sym(">>") => {
                    look.half = true;
                    return Some(j);
                }
                _ => return None,
            }
        }
    }

    /// Do type arguments for a call or path start at the `<` at offset 0:
    /// do they run to a `>` that `(` or `::` follows? (`a < b > (c)` reads as
    /// type arguments; it is a type error either way.)
    fn expr_targs_ahead(&self) -> bool {
        if self.peek() != &Tok::Sym("<") || !self.type_start_at(1) {
            return false;
        }
        let mut look = Look { half: false };
        match self.args_look(0, &mut look) {
            Some(end) if !look.half => matches!(self.peek_at(end), Tok::Sym("(" | "::")),
            _ => false,
        }
    }

    /// The token offset of the `>` or `>>` that closes a unit part starting at
    /// `n` (a `<`), if the tokens between read as a unit. A `>>` closes a
    /// nested unit: `close_angle` splits it.
    fn unit_close(&self, n: usize) -> Option<usize> {
        if self.peek_at(n) != &Tok::Sym("<") {
            return None;
        }
        let mut j = n + 1;
        loop {
            match self.peek_at(j) {
                Tok::Ident(_) | Tok::Sym("*" | "/" | "^" | "-" | "(" | ")") => j += 1,
                Tok::Num(NumLit { value: Const::Int(_), suffix: None, .. }, None) => j += 1,
                Tok::Kw("as") if self.as_unit_at(j) => j += 1,
                Tok::Sym(">" | ">>") => return Some(j),
                _ => return None,
            }
        }
    }

    /// Is the `as` at token offset `n` the `as unit` that follows a unit name
    /// (`float64<T as unit>`)? `as` is a keyword only in `same as`.
    fn as_unit_at(&self, n: usize) -> bool {
        self.peek_at(n) == &Tok::Kw("as") && matches!(self.peek_at(n + 1), Tok::Ident(w) if w == "unit")
    }

    /// Does a unit declaration (`unit NAME;` or `unit NAME = ...`) start here?
    /// `unit` is a context word: elsewhere it is an ordinary name.
    fn at_unit_item(&self) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == "unit")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && matches!(self.peek_at(2), Tok::Sym(";" | "="))
    }

    /// The offset after the optional `fixed` and `mut` of a top-level value.
    fn const_prefix(&self) -> usize {
        let mut n = 0;
        if matches!(self.peek(), Tok::Ident(w) if w == "fixed") && !matches!(self.peek_at(1), Tok::Sym("=" | ";" | "(" | "<")) {
            n += 1;
        }
        if self.peek_at(n) == &Tok::Kw("mut") {
            n += 1;
        }
        n
    }

    /// Does a top-level value (`Type NAME = ...;`) start here?
    fn at_const_item(&self) -> bool {
        let from = self.const_prefix();
        match self.type_end(from) {
            Some(n) => matches!(self.peek_at(n), Tok::Ident(_)) && matches!(self.peek_at(n + 1), Tok::Sym("=" | ";")),
            None => false,
        }
    }

    /// `[fixed] [mut] Type NAME = value;`
    fn const_decl(&mut self) -> Parsed<ConstDecl> {
        let start = self.span();
        let fixed = matches!(self.peek(), Tok::Ident(w) if w == "fixed") && self.const_prefix() > 0;
        if fixed {
            self.bump();
        }
        let mutable = self.eat_kw("mut");
        match self.binding(start, mutable, false)? {
            Stmt::Let { ty, name, name_span, value, span, .. } => Ok(ConstDecl { fixed, mutable, ty, name, name_span, value, span }),
            _ => unreachable!(),
        }
    }

    /// Does a top-level item (a function, struct or import) start here?
    fn at_item_start(&self) -> bool {
        if self.at_def_item() {
            return true;
        }
        match self.peek() {
            Tok::Kw("in" | "struct" | "enum" | "failure" | "interface") => true,
            // `out` is also a parameter word: only an export list or `out`
            // before a whole declaration counts.
            Tok::Kw("out") => {
                self.at_export_list()
                    || matches!(self.peek_at(1), Tok::Kw("struct" | "enum" | "failure" | "interface"))
                    || {
                        let mut from = 1;
                        if matches!(self.peek_at(from), Tok::Ident(w) if w == "fixed") && !matches!(self.peek_at(from + 1), Tok::Sym("=" | ";" | "(" | "<")) {
                            from += 1;
                        }
                        if self.peek_at(from) == &Tok::Kw("mut") {
                            from += 1;
                        }
                        self.type_end(from).is_some_and(|n| matches!(self.peek_at(n), Tok::Ident(_)) && matches!(self.peek_at(n + 1), Tok::Sym("=" | ";")))
                    }
                    || {
                        let from = 1 + (self.peek_at(1) == &Tok::Kw("unsafe")) as usize;
                        match self.type_end(from) {
                            Some(n) => matches!(self.peek_at(n), Tok::Ident(_)) && {
                                let after = self.tparams_end(n + 1).unwrap_or(n + 1);
                                self.peek_at(after) == &Tok::Sym("(")
                            },
                            None => false,
                        }
                    }
            }
            Tok::Ident(_) | Tok::Kw("none" | "unsafe") | Tok::Sym("&" | "&&" | "*" | "**" | "(") => {
                let from = (self.peek() == &Tok::Kw("unsafe")) as usize;
                match self.type_end(from) {
                    Some(n) => matches!(self.peek_at(n), Tok::Ident(_)) && {
                        let after = self.tparams_end(n + 1).unwrap_or(n + 1);
                        self.peek_at(after) == &Tok::Sym("(")
                    },
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// The offset after a type parameter list `<T, U with A and B>` that
    /// starts at token offset `n`, if one is written there.
    fn tparams_end(&self, n: usize) -> Option<usize> {
        if self.peek_at(n) != &Tok::Sym("<") {
            return None;
        }
        let mut depth = 0usize;
        let mut j = n;
        loop {
            match self.peek_at(j) {
                Tok::Sym("<") => depth += 1,
                Tok::Sym(">") => depth = depth.checked_sub(1)?,
                Tok::Sym(">>") => depth = depth.checked_sub(2)?,
                Tok::Ident(_) | Tok::Kw("with" | "none") | Tok::Sym("," | "&" | "*" | "**" | "?" | "[" | "]" | "/" | "^" | "(" | ")") | Tok::Num(..) => {}
                _ => return None,
            }
            j += 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }

    /// `<T, U with A and B, unit V>` after a function's name, if written.
    fn type_params(&mut self) -> Parsed<Vec<TypeParam>> {
        let mut out = Vec::new();
        if self.peek() != &Tok::Sym("<") {
            return Ok(out);
        }
        let open = self.bump().span;
        if self.peek() == &Tok::Sym(">") {
            return Err(Diagnostic::at(&codes::E0003, open.to(self.span()), "expected a type parameter name between `<` and `>`")
                .help("write the names the function is generic over: `T max<T>(T a, T b)`"));
        }
        loop {
            let is_unit = matches!(self.peek(), Tok::Ident(w) if w == "unit") && matches!(self.peek_at(1), Tok::Ident(_));
            if is_unit {
                self.bump();
            }
            // `uint32 N`: a number type and then the name makes a value parameter.
            let value = match (self.peek().clone(), self.peek_at(1).clone()) {
                (Tok::Ident(ty), Tok::Ident(_)) if !is_unit && is_number_name(&ty) => {
                    let at = self.bump().span;
                    Some((ty, at))
                }
                _ => None,
            };
            let (name, span) = self.ident("a type parameter name")?;
            let mut bounds = Vec::new();
            if self.eat_kw("with") {
                loop {
                    bounds.push(self.ty()?);
                    if matches!(self.peek(), Tok::Ident(w) if w == "and") {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            out.push(TypeParam { name, span, is_unit, bounds, value });
            if !self.eat(",") {
                break;
            }
        }
        if !matches!(self.peek(), Tok::Sym(">" | ">>")) {
            return Err(self.unexpected("`,` or `>`").label(open, "the type parameters start here"));
        }
        self.close_angle()?;
        Ok(out)
    }

    /// A dotted name: `game.physics`.
    fn dotted(&mut self, what: &str) -> Parsed<(Vec<String>, Span)> {
        let (first, start) = self.ident(what)?;
        let mut parts = vec![first];
        let mut end = start;
        while self.peek() == &Tok::Sym(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            let (name, at) = self.ident(what)?;
            parts.push(name);
            end = at;
        }
        Ok((parts, start.to(end)))
    }

    /// A module path, dotted or in words: `game.physics` or `physics inside
    /// game.sound` (outermost folder first in the result).
    fn module_path(&mut self) -> Parsed<(Vec<String>, Span, bool)> {
        let (mut path, mut span) = self.dotted("a module name")?;
        if matches!(self.peek(), Tok::Ident(w) if w == "inside") {
            self.bump();
            let (mut outer, outer_span) = self.dotted("the folder the module is inside")?;
            outer.append(&mut path);
            return Ok((outer, span.to(outer_span), true));
        }
        span = span.to(self.prev_span());
        Ok((path, span, false))
    }

    /// A module named as a path or as a string file path (`"../x/y.drh"`).
    /// Returns the path (a file path gives its stem), its span, whether it was
    /// written in words, and the file path.
    fn module_ref(&mut self) -> Parsed<(Vec<String>, Span, bool, Option<String>)> {
        if let Tok::Str(pieces) = self.peek().clone() {
            let span = self.bump().span;
            let mut text = String::new();
            for p in &pieces {
                match p {
                    StrPiece::Raw(t, _) => text.push_str(t),
                    _ => return Err(Diagnostic::at(&codes::E0003, span, "a module file path is plain text").help("write the path without escapes or `${}`: `in \"../runtime/scheduler.drh\";`")),
                }
            }
            let stem = text.rsplit('/').next().unwrap_or("").strip_suffix(".drh").unwrap_or("").to_string();
            if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || stem.starts_with(|c: char| c.is_ascii_digit()) {
                return Err(Diagnostic::at(&codes::E0230, span, format!("`{text}` is not a module file path"))
                    .note("needs a file name ending in `.drh`")
                    .help("a module file has a plain name (letters, digits, `_`): `in \"../runtime/scheduler.drh\";`"));
            }
            return Ok((vec![stem], span, false, Some(text)));
        }
        let (path, span, word) = self.module_path()?;
        Ok((path, span, word, None))
    }

    /// `in` items; see `ast::Import` for the forms.
    fn import(&mut self) -> Parsed<Import> {
        let start = self.bump().span;
        let help = "write `in io;`, `in game.physics as phys;`, `in Vector3 as V3, gravity from physics;` or `in * from game.audio;`";
        if self.eat("*") {
            if !self.eat_kw("from") {
                return Err(self.unexpected("`from` after `*`").help(help));
            }
            let (path, path_span, word_form, file) = self.module_ref()?;
            let alias = self.alias()?;
            let mut also_bare = false;
            if matches!(self.peek(), Tok::Ident(w) if w == "also") && matches!(self.peek_at(1), Tok::Ident(w) if w == "bare") {
                let at = self.bump().span.to(self.bump().span);
                if alias.is_none() {
                    return Err(Diagnostic::at(&codes::E0003, at, "`also bare` needs a name")
                        .help("without `as`, `in * from m;` already brings the names bare: `in * from game.audio as snd also bare;`"));
                }
                also_bare = true;
            }
            self.expect(";")?;
            return Ok(Import { path, path_span, word_form, items: Vec::new(), glob: true, alias, also_bare, file, span: start.to(self.prev_span()) });
        }
        if matches!(self.peek(), Tok::Str(_)) {
            let (path, path_span, word_form, file) = self.module_ref()?;
            let alias = self.alias()?;
            self.expect(";")?;
            return Ok(Import { path, path_span, word_form, items: Vec::new(), glob: false, alias, also_bare: false, file, span: start.to(self.prev_span()) });
        }
        // The first names, as dotted paths for now: `in a.b as c;` is a module,
        // `in a as b, c from m;` is a list of items. `from` tells them apart.
        let mut entries: Vec<(Vec<String>, Span, Option<(String, Span)>)> = Vec::new();
        loop {
            let (path, span) = self.dotted("a module or item name")?;
            let alias = self.alias()?;
            entries.push((path, span, alias));
            if !self.eat(",") {
                break;
            }
        }
        if self.eat_kw("from") {
            let mut items = Vec::new();
            for (path, span, alias) in entries {
                if path.len() != 1 {
                    return Err(Diagnostic::at(&codes::E0003, span, "an imported item is a single name, not a path")
                        .help("name the module after `from`: `in Vector3 from game.physics;`"));
                }
                let full = alias.as_ref().map_or(span, |a| span.to(a.1));
                items.push(ImportItem { name: path.into_iter().next().unwrap(), alias: alias.map(|a| a.0), span: full });
            }
            let (path, path_span, word_form, file) = self.module_ref()?;
            if self.peek() == &Tok::Kw("as") {
                return Err(self
                    .unexpected("`;`")
                    .help("rename an item next to its name: `in Vector3 as V3 from physics;`, or import the module with its own name: `in physics as phys;`"));
            }
            self.expect(";")?;
            return Ok(Import { path, path_span, word_form, items, glob: false, alias: None, also_bare: false, file, span: start.to(self.prev_span()) });
        }
        if entries.len() != 1 {
            return Err(self.unexpected("`from` after the imported names").help(help));
        }
        let (mut path, mut path_span, alias) = {
            let (p, s, a) = entries.pop().unwrap();
            (p, s, a)
        };
        let mut word_form = false;
        if alias.is_none() && matches!(self.peek(), Tok::Ident(w) if w == "inside") {
            self.bump();
            let (mut outer, outer_span) = self.dotted("the folder the module is inside")?;
            outer.append(&mut path);
            path = outer;
            path_span = path_span.to(outer_span);
            word_form = true;
        }
        let alias = if alias.is_none() { self.alias()? } else { alias };
        self.expect(";")?;
        Ok(Import { path, path_span, word_form, items: Vec::new(), glob: false, alias, also_bare: false, file: None, span: start.to(self.prev_span()) })
    }

    /// `as name`, if written.
    fn alias(&mut self) -> Parsed<Option<(String, Span)>> {
        if !self.eat_kw("as") {
            return Ok(None);
        }
        let (name, span) = self.ident("a name after `as`")?;
        Ok(Some((name, span)))
    }

    /// Does an export list (`out A, B as C;`) start here, as against `out`
    /// before a declaration?
    fn at_export_list(&self) -> bool {
        self.peek() == &Tok::Kw("out")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && matches!(self.peek_at(2), Tok::Sym("," | ";") | Tok::Kw("as"))
    }

    /// `out A, B as C;`
    fn export_list(&mut self) -> Parsed<Vec<Export>> {
        self.bump();
        let mut out = Vec::new();
        loop {
            let (name, at) = self.ident("a name to export")?;
            let alias = self.alias()?;
            let span = alias.as_ref().map_or(at, |a| at.to(a.1));
            out.push(Export { name, alias: alias.map(|a| a.0), span, on_decl: false });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(";")?;
        Ok(out)
    }

    fn func(&mut self) -> Parsed<Func> {
        let is_unsafe = if self.peek() == &Tok::Kw("unsafe") { Some(self.bump().span) } else { None };
        let starts_type = matches!(self.peek(), Tok::Ident(_) | Tok::Kw("none") | Tok::Sym("&" | "&&" | "*" | "**" | "("))
            || (self.peek() == &Tok::Kw("maybe") && self.peek_at(1) == &Tok::Sym("&"));
        if !starts_type {
            return Err(self
                .unexpected("a function, `struct` or `in`")
                .help("a function starts with its return type: `none main() { }`"));
        }
        let ret = self.ty()?;
        let (name, name_span) = self.ident("a function name")?;
        self.func_rest(ret, name, name_span, is_unsafe)
    }

    /// Parameters after the opening parenthesis, through the closing one.
    fn params(&mut self) -> Parsed<Vec<Param>> {
        let mut params = Vec::new();
        while self.peek() != &Tok::Sym(")") {
            let start = self.span();
            let (mutable, open) = self.binding_words();
            let ty = self.ty()?;
            let (pname, _) = self.ident("a parameter name")?;
            params.push(Param { mutable, open, ty, name: pname, span: start.to(self.prev_span()) });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(params)
    }

    /// A function's parameters and body, after its return type and name.
    fn func_rest(&mut self, ret: TypeName, name: String, name_span: Span, is_unsafe: Option<Span>) -> Parsed<Func> {
        let tparams = self.type_params()?;
        self.expect("(")?;
        let params = self.params()?;
        let span = ret.span.to(self.prev_span());
        let mut lives = Vec::new();
        let mut lives_span = None;
        let mut lives_from = false;
        let mut fails = Vec::new();
        let mut fails_span = None;
        loop {
            match self.peek() {
                Tok::Kw("lives") => {
                    if let Some(first) = lives_span {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`lives` is written twice")
                            .label(first, "first written here"));
                    }
                    let at = self.bump().span;
                    if self.peek() == &Tok::Kw("from") {
                        self.bump();
                        lives_from = true;
                        lives = self.source_list()?;
                    } else {
                        lives = self.name_list("a parameter name after `lives`")?;
                    }
                    lives_span = Some(at.to(self.prev_span()));
                }
                Tok::Ident(w) if w == "changes" && self.peek_at(1) == &Tok::Kw("self") => {
                    let first = self.bump().span;
                    let at = first.to(self.bump().span);
                    self.diags.push(
                        Diagnostic::at(&codes::E0613, at, "`changes self` is only written in interfaces")
                            .note("not an interface method")
                            .help("a method's mode is worked out from its body: remove `changes self`"),
                    );
                }
                Tok::Kw("fails") => {
                    if let Some(first) = fails_span {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`fails` is written twice")
                            .label(first, "first written here"));
                    }
                    let at = self.bump().span;
                    fails = vec![self.qual_ident("a failure type name after `fails`")?];
                    while self.eat(",") {
                        fails.push(self.qual_ident("a failure type name after `fails`")?);
                    }
                    fails_span = Some(at.to(self.prev_span()));
                }
                _ => break,
            }
        }
        let body = self.block()?;
        Ok(Func { ret, name, name_span, tparams, params, body, is_unsafe, lives, lives_span, lives_from, fails, fails_span, span })
    }

    /// `self or key or other`, after `lives from`.
    fn source_list(&mut self) -> Parsed<Vec<(String, Span)>> {
        let mut names = Vec::new();
        loop {
            if self.peek() == &Tok::Kw("self") {
                names.push(("self".to_string(), self.bump().span));
            } else {
                names.push(self.ident("a parameter name or `self` after `lives from`")?);
            }
            if self.peek() == &Tok::Ident("or".to_string()) {
                self.bump();
            } else {
                return Ok(names);
            }
        }
    }

    /// `a, b, c`
    fn name_list(&mut self, what: &str) -> Parsed<Vec<(String, Span)>> {
        let mut names = vec![self.ident(what)?];
        while self.eat(",") {
            names.push(self.ident(what)?);
        }
        Ok(names)
    }

    /// Any of the words `mut` and `open` before a binding's type, in either
    /// order. Returns `(mutable, open)`.
    fn binding_words(&mut self) -> (bool, bool) {
        let (mut m, mut o) = (false, false);
        // `exclusive &Shape s`: a parameter that takes exactly `Shape`, not a
        // sub-interface (owner, step 6 Q5). Not designed yet.
        if matches!(self.peek(), Tok::Ident(w) if w == "exclusive") && matches!(self.peek_at(1), Tok::Sym("&" | "&&")) {
            let span = self.bump().span;
            self.diags.push(
                Diagnostic::at(&codes::E0901, span, "`exclusive` is not supported yet")
                    .note("a parameter that accepts only exactly this interface, not one that extends it")
                    .help("leave `exclusive` out for now: the parameter then also accepts a sub-interface"),
            );
        }
        loop {
            match self.peek() {
                Tok::Kw("mut") => m = true,
                Tok::Kw("open") => o = true,
                _ => return (m, o),
            }
            self.bump();
        }
    }

    /// `enum Name { Variant, Variant(Type name, Type name), }`
    fn enum_decl(&mut self) -> Parsed<Enum> {
        let start = self.bump().span;
        let (name, name_span) = self.ident("an enum name")?;
        let tparams = self.type_params()?;
        let open = self.expect("{")?;
        let mut variants = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof || self.peek() == &Tok::Kw("struct") || self.peek() == &Tok::Kw("enum") {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            let (vname, vspan) = match self.peek().clone() {
                Tok::Ident(n) => (n, self.bump().span),
                _ => return Err(self.unexpected("a variant name").help("each variant of an enum is written `Name` or `Name(Type value, ...)`")),
            };
            // `float64 area() { }` in an enum body: a method.
            if matches!(self.peek(), Tok::Ident(_) | Tok::Sym("&" | "*")) {
                return Err(Diagnostic::at(&codes::E0003, vspan, "an enum has variants only, not methods")
                    .note("this looks like a method")
                    .help("write a function that takes the enum: `float64 area(Shape s) { }`"));
            }
            let mut fields = Vec::new();
            if self.eat("(") {
                while self.peek() != &Tok::Sym(")") {
                    let ty = self.ty()?;
                    let (fname, fspan) = self.ident("a name for the value")?;
                    fields.push(Field { public: true, mutable: false, ty, name: fname, name_span: fspan });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
            }
            variants.push(Variant { name: vname, name_span: vspan, fields });
            if !self.eat(",") {
                break;
            }
        }
        if self.peek() != &Tok::Sym("}") {
            return Err(self.unexpected("`,` or `}`").help("separate the variants with commas"));
        }
        self.bump();
        if variants.is_empty() {
            return Err(Diagnostic::at(&codes::E0003, start.to(name_span), "an enum needs at least one variant")
                .note("this enum has none")
                .help(format!("write one: `enum {name} {{ First, Second }}`")));
        }
        Ok(Enum { name, name_span, tparams, span: start.to(self.prev_span()), variants })
    }

    /// `failure Name { Variant "message" exit N, Variant(Type v) "msg ${v}", }`
    fn failure_decl(&mut self) -> Parsed<Failure> {
        self.bump();
        let (name, name_span) = self.ident("a failure name")?;
        let open = self.expect("{")?;
        let mut variants = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            if matches!(self.peek(), Tok::Eof | Tok::Kw("struct" | "enum" | "failure")) {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            let (vname, vspan) = match self.peek().clone() {
                Tok::Ident(n) => (n, self.bump().span),
                _ => {
                    return Err(self
                        .unexpected("a variant name")
                        .help("each variant of a failure is written `Name \"message\"` or `Name(Type value) \"message\"`"))
                }
            };
            let mut fields = Vec::new();
            if self.eat("(") {
                while self.peek() != &Tok::Sym(")") {
                    let ty = self.ty()?;
                    let (fname, fspan) = self.ident("a name for the value")?;
                    fields.push(Field { public: true, mutable: false, ty, name: fname, name_span: fspan });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
            }
            if !matches!(self.peek(), Tok::Str(_)) {
                return Err(Diagnostic::at(&codes::E0003, vspan, "a failure variant needs a message")
                    .note(format!("`{vname}` has no message"))
                    .help(format!("write the message after the name: `{vname} \"what went wrong\"`")));
            }
            let message = self.primary()?;
            let exit = if self.peek() == &Tok::Kw("exit") {
                self.bump();
                match self.peek().clone() {
                    Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => {
                        let span = self.bump().span;
                        Some((v.to_u64().unwrap_or(u64::MAX), span))
                    }
                    _ => return Err(self.unexpected("an exit code").help("write the number the program exits with: `exit 2`")),
                }
            } else {
                None
            };
            variants.push(FailVariant { name: vname, name_span: vspan, fields, message, exit });
            if !self.eat(",") {
                break;
            }
        }
        if self.peek() != &Tok::Sym("}") {
            return Err(self.unexpected("`,` or `}`").help("separate the variants with commas"));
        }
        self.bump();
        Ok(Failure { name, name_span, variants })
    }

    /// Does a `def` declaration start here? `def` is a context word: elsewhere
    /// it is an ordinary name.
    fn at_def_item(&self) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == "def")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && matches!(self.peek_at(2), Tok::Kw("of" | "from") | Tok::Sym("{"))
    }

    /// `A and B`: one or more types.
    fn and_list(&mut self) -> Parsed<Vec<TypeName>> {
        let mut out = vec![self.ty()?];
        while matches!(self.peek(), Tok::Ident(w) if w == "and") {
            self.bump();
            out.push(self.ty()?);
        }
        Ok(out)
    }

    /// `TYPE with A and B {`: a type promising interfaces outside its own declaration.
    fn at_impl_item(&self) -> bool {
        matches!(self.peek(), Tok::Ident(_)) && self.type_end(0).is_some_and(|n| self.peek_at(n) == &Tok::Kw("with"))
    }

    fn impl_decl(&mut self) -> Parsed<ImplDecl> {
        let start = self.span();
        let ty = self.ty()?;
        self.bump();
        // `trusted` is a context word: a modifier only when an interface name follows.
        let trusted = matches!(self.peek(), Tok::Ident(w) if w == "trusted") && matches!(self.peek_at(1), Tok::Ident(_));
        if trusted {
            self.bump();
        }
        let with = self.and_list()?;
        let open = self.expect("{")?;
        // A scratch struct to collect the methods with the struct member rules.
        let mut s = Struct {
            name: ty.name.clone(),
            name_span: ty.span,
            tparams: Vec::new(),
            span: start,
            with: Vec::new(),
            lives: Vec::new(),
            stays: None,
            defaults: None,
            auto_ctor: None,
            fields: Vec::new(),
            methods: Vec::new(),
        };
        let mut section: Option<bool> = None;
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof || matches!(self.peek(), Tok::Kw("struct" | "interface" | "enum")) {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            if let Tok::Kw(k @ ("public" | "private")) = *self.peek()
                && self.peek_at(1) == &Tok::Sym(":")
            {
                section = Some(k == "public");
                self.bump();
                self.bump();
                continue;
            }
            if let Err(d) = self.member(&mut s, section) {
                self.diags.push(d);
                self.skip_statement();
            }
        }
        self.bump();
        if let Some(f) = s.fields.first() {
            self.diags.push(Diagnostic::at(&codes::E0003, f.name_span, "a declaration like this adds methods, not fields").help("fields belong to the type's own declaration"));
        }
        Ok(ImplDecl { ty, with, trusted, methods: s.methods, span: start.to(self.prev_span()) })
    }

    /// `interface Name [extends A and B] { members }`
    fn interface_decl(&mut self) -> Parsed<Interface> {
        let start = self.bump().span;
        let (name, name_span) = self.ident("an interface name")?;
        let tparams = self.type_params()?;
        let mut extends = Vec::new();
        if self.peek() == &Tok::Kw("extends") {
            self.bump();
            extends = self.and_list()?;
        }
        let open = self.expect("{")?;
        let mut methods = Vec::new();
        let mut fields = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof || matches!(self.peek(), Tok::Kw("struct" | "interface" | "enum")) {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            if let Err(d) = self.interface_member(&mut methods, &mut fields) {
                self.diags.push(d);
                self.skip_statement();
            }
        }
        self.bump();
        Ok(Interface { name, name_span, tparams, extends, methods, fields, span: start.to(self.prev_span()) })
    }

    fn interface_member(&mut self, methods: &mut Vec<IMethod>, fields: &mut Vec<IField>) -> Parsed<()> {
        let is_static = if self.peek() == &Tok::Kw("static") { Some(self.bump().span) } else { None };
        if matches!(self.peek(), Tok::Kw("public" | "private" | "mut")) {
            return Err(Diagnostic::at(&codes::E0003, self.span(), "an interface member has no visibility or `mut`")
                .help("interface members are public; a required field is written `float64 x;`"));
        }
        let ty = self.ty()?;
        let operator = match *self.peek() {
            Tok::Sym(sym) if self.peek_at(1) == &Tok::Sym("(") => binop(sym).map(|(op, _)| op),
            _ => None,
        };
        let (name, name_span) = match operator {
            Some(op) => (op.symbol().to_string(), self.bump().span),
            None => self.ident("a method or field name")?,
        };
        let tps = self.type_params()?;
        if self.peek() != &Tok::Sym("(") {
            if let Some(st) = is_static {
                return Err(Diagnostic::at(&codes::E0003, st, "a required field cannot be `static`").help("only methods can be `static`"));
            }
            self.expect(";")?;
            fields.push(IField { ty, name, name_span });
            return Ok(());
        }
        if let Some(tp) = tps.first() {
            return Err(Diagnostic::at(&codes::E0901, tp.span, "interface methods with type parameters of their own are not supported yet")
                .help("write the method for one type for now"));
        }
        self.bump();
        let mut params = Vec::new();
        while self.peek() != &Tok::Sym(")") {
            let start = self.span();
            let (mutable, open) = self.binding_words();
            let pty = self.ty()?;
            let (pname, _) = self.ident("a parameter name")?;
            params.push(Param { mutable, open, ty: pty, name: pname, span: start.to(self.prev_span()) });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let mut changes_self = None;
        if matches!(self.peek(), Tok::Ident(w) if w == "changes") && self.peek_at(1) == &Tok::Kw("self") {
            let first = self.bump().span;
            let at = first.to(self.bump().span);
            if let Some(st) = is_static {
                return Err(Diagnostic::at(&codes::E0613, at, "a `static` method has no `self` to change").label(st, "the method is `static`").help("remove `changes self` or `static`"));
            }
            changes_self = Some(at);
        }
        let span = ty.span.to(self.prev_span());
        self.expect(";")?;
        methods.push(IMethod { is_static, operator, name, name_span, ret: ty, params, changes_self, span });
        Ok(())
    }

    /// `def NAME [of STRUCT] [from BASE] { field = value; ... }`
    fn def_decl(&mut self) -> Parsed<DefDecl> {
        let start = self.bump().span;
        let (name, name_span) = self.ident("a name for the def")?;
        let mut of = None;
        let mut of_args = Vec::new();
        if self.peek() == &Tok::Kw("of") {
            self.bump();
            let (n, sp) = self.qual_ident("a struct name after `of`")?;
            if self.peek() == &Tok::Sym("<") && self.type_start_at(1) {
                of_args = self.type_args()?;
                self.generics.push(TypeName { args: of_args.clone(), ..TypeName::plain(n.clone(), sp) });
            }
            of = Some((n, sp));
        }
        let mut from = None;
        if self.peek() == &Tok::Kw("from") {
            self.bump();
            from = Some(self.qual_ident("the name of the def to extend after `from`")?);
        }
        self.expect("{")?;
        let mut assigns = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof {
                return Err(self.unexpected("`}`"));
            }
            let (fname, fspan) = self.ident("a field name")?;
            self.expect("=")?;
            let value = self.expr()?;
            self.expect(";")?;
            assigns.push(DefAssign { name: fname, name_span: fspan, value });
        }
        self.bump();
        Ok(DefDecl { name, name_span, of, of_args, from, assigns, span: start.to(self.prev_span()) })
    }

    fn struct_decl(&mut self) -> Parsed<Struct> {
        let start = self.bump().span;
        // `struct default Player { }`: the default def is the one named like the struct.
        let default_word = if matches!(self.peek(), Tok::Ident(w) if w == "default") { Some(self.bump().span) } else { None };
        let (name, name_span) = self.ident("a struct name")?;
        let tparams = self.type_params()?;
        let mut defaults = default_word.map(|_| (name.clone(), name_span));
        let mut lives: Vec<(String, Span)> = Vec::new();
        let mut lives_seen = false;
        let mut stays = None;
        let mut with: Vec<TypeName> = Vec::new();
        loop {
            match self.peek() {
                Tok::Ident(w) if w == "defaults" => {
                    if let Some((_, first)) = defaults {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "the default is named twice").label(first, "first named here"));
                    }
                    self.bump();
                    defaults = Some(self.qual_ident("the name of the default def after `defaults`")?);
                }
                Tok::Kw("with") => {
                    if let Some(first) = with.first() {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`with` is written twice").label(first.span, "first written here").help("list them with `and`: `with Shape and Drawable`"));
                    }
                    self.bump();
                    with = self.and_list()?;
                }
                Tok::Kw("lives") => {
                    if lives_seen {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`lives` is written twice"));
                    }
                    lives_seen = true;
                    self.bump();
                    lives = self.name_list("a field name after `lives`")?;
                }
                Tok::Ident(w) if w == "stays" => {
                    if let Some(first) = stays {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`stays` is written twice")
                            .label(first, "first written here"));
                    }
                    stays = Some(self.bump().span);
                }
                _ => break,
            }
        }
        let open = self.expect("{")?;
        let mut s = Struct { name, name_span, tparams, span: start, with, lives, stays, defaults, auto_ctor: None, fields: Vec::new(), methods: Vec::new() };
        // `Some(true)` after `public:`, `Some(false)` after `private:`. Before
        // any label, fields are private and methods public.
        let mut section: Option<bool> = None;
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof || self.peek() == &Tok::Kw("struct") {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            if let Tok::Kw(k @ ("public" | "private")) = *self.peek()
                && self.peek_at(1) == &Tok::Sym(":")
            {
                section = Some(k == "public");
                self.bump();
                self.bump();
                continue;
            }
            if let Err(d) = self.member(&mut s, section) {
                self.diags.push(d);
                self.skip_statement();
            }
        }
        self.bump();
        s.span = start.to(self.prev_span());
        Ok(s)
    }

    /// One field or method of struct `s`.
    fn member(&mut self, s: &mut Struct, section: Option<bool>) -> Parsed<()> {
        let is_static = if self.peek() == &Tok::Kw("static") { Some(self.bump().span) } else { None };
        let is_unsafe = if self.peek() == &Tok::Kw("unsafe") { Some(self.bump().span) } else { None };
        let mutable = if self.peek() == &Tok::Kw("mut") { Some(self.bump().span) } else { None };
        if matches!(self.peek(), Tok::Ident(n) if *n == s.name) && self.peek_at(1) == &Tok::Sym("(") {
            if let Some(st) = is_static {
                return Err(Diagnostic::at(&codes::E0003, st, "a constructor cannot be `static`").help("constructors build the value themselves"));
            }
            if let Some(u) = is_unsafe {
                return Err(Diagnostic::at(&codes::E0003, u, "a constructor cannot be `unsafe`"));
            }
            if let Some(m) = mutable {
                return Err(Diagnostic::at(&codes::E0003, m, "a constructor cannot be `mut`"));
            }
            // `Person(auto);` keeps the automatic constructor next to the written ones.
            if self.peek_at(2) == &Tok::Ident("auto".to_string()) && self.peek_at(3) == &Tok::Sym(")") && self.peek_at(4) == &Tok::Sym(";") {
                let at = self.span();
                for _ in 0..5 {
                    self.bump();
                }
                if let Some(first) = s.auto_ctor {
                    return Err(Diagnostic::at(&codes::E0003, at, "`auto` is written twice").label(first, "first written here"));
                }
                s.auto_ctor = Some(at);
                return Ok(());
            }
            let (name, name_span) = self.ident("a constructor name")?;
            let ret = TypeName { self_ty: true, ..TypeName::plain(s.name.clone(), name_span) };
            let func = self.func_rest(ret, name, name_span, None)?;
            s.methods.push(Method { public: section.unwrap_or(true), is_static: None, operator: None, ctor: true, func });
            return Ok(());
        }
        let ty = self.ty()?;
        // An operator method is named by its symbol: `Vector3 +(Vector3 o)`.
        let operator = match *self.peek() {
            Tok::Sym(sym) if self.peek_at(1) == &Tok::Sym("(") => binop(sym).map(|(op, _)| op),
            _ => None,
        };
        let (name, name_span) = match operator {
            Some(op) => (op.symbol().to_string(), self.bump().span),
            None => self.ident("a field or method name")?,
        };
        if self.peek() == &Tok::Sym("(") {
            if let Some(m) = mutable {
                return Err(Diagnostic::at(&codes::E0003, m, "a method cannot be `mut`")
                    .help("`mut` goes on fields: `mut uint32 hp;`"));
            }
            let func = self.func_rest(ty, name, name_span, is_unsafe)?;
            if let Some(tp) = func.tparams.first() {
                return Err(Diagnostic::at(&codes::E0901, tp.span, "methods with type parameters of their own are not supported yet")
                    .help("a struct's methods can use the struct's own type parameters"));
            }
            s.methods.push(Method { public: section.unwrap_or(true), is_static, operator, ctor: false, func });
            return Ok(());
        }
        if let Some(u) = is_unsafe {
            return Err(Diagnostic::at(&codes::E0003, u, "a field cannot be `unsafe`").help("only functions can be `unsafe`"));
        }
        if let Some(st) = is_static {
            return Err(Diagnostic::at(&codes::E0003, st, "a field cannot be `static`").help("only methods can be `static`"));
        }
        if self.peek() == &Tok::Sym("=") {
            return Err(self
                .unexpected("`;`")
                .help(format!("fields have no default values; give them when building the value: `{} p = 1, 2;`", s.name)));
        }
        self.expect(";")?;
        s.fields.push(Field { public: section.unwrap_or(false), mutable: mutable.is_some(), ty, name, name_span });
        Ok(())
    }

    fn block(&mut self) -> Parsed<Block> {
        let open = self.expect("{")?;
        let mut stmts = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            // Functions don't nest, so the start of one means the `}` is missing.
            if self.peek() == &Tok::Eof || self.at_item_start() {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            match self.stmt() {
                Ok(s) => stmts.push(s),
                Err(d) => {
                    self.diags.push(d);
                    self.skip_statement();
                }
            }
        }
        let end = self.bump().span;
        Ok(Block { stmts, end })
    }

    /// After a syntax error: skip to just past the next `;` (or nested block),
    /// stopping early at the `}` that closes the current block.
    fn skip_statement(&mut self) {
        let mut depth = 0u32;
        loop {
            if self.peek() == &Tok::Eof || self.at_item_start() {
                return;
            }
            match self.peek() {
                Tok::Sym(";") if depth == 0 => {
                    self.bump();
                    return;
                }
                Tok::Sym("}") if depth == 0 => return,
                Tok::Sym("}") if depth == 1 => {
                    self.bump();
                    return;
                }
                Tok::Sym("{") => depth += 1,
                Tok::Sym("}") => depth -= 1,
                _ => {}
            }
            self.bump();
        }
    }

    /// Does a binding declaration (`[mut] Type name ...`) start here? A
    /// lone name starts an expression. (A reserved word in the name's place
    /// gets the binding's error message.)
    fn at_binding(&self) -> bool {
        match self.peek() {
            Tok::Kw("mut" | "open") => true,
            Tok::Ident(_) | Tok::Kw("none") => matches!(self.peek_at(self.name_end(0)), Tok::Ident(_) | Tok::Kw(_)),
            Tok::Sym("(") => self.type_end(0).is_some_and(|n| matches!(self.peek_at(n), Tok::Ident(_)) && matches!(self.peek_at(n + 1), Tok::Sym("=" | ";"))),
            Tok::Sym("&" | "&&") => true,
            // `maybe &T name` declares; a `maybe` with a name after it is a
            // misspelt borrow type, reported by `ty`.
            Tok::Kw("maybe") if self.peek_at(1) == &Tok::Sym("&") => self.type_end(0).is_some_and(|n| matches!(self.peek_at(n), Tok::Ident(_))),
            Tok::Kw("maybe") => matches!(self.peek_at(1), Tok::Ident(_)) && matches!(self.peek_at(self.name_end(1)), Tok::Ident(_)),
            // `*uint32 p = ..` declares; `*p = 7;` and `*p.f = 7;` assign.
            Tok::Sym("*" | "**") => {
                let mut n = 0;
                while matches!(self.peek_at(n), Tok::Sym("*" | "**")) {
                    n += 1;
                }
                matches!(self.peek_at(n), Tok::Ident(_) | Tok::Kw("none"))
                    && matches!(self.peek_at(self.name_end(n)), Tok::Ident(_))
            }
            _ => false,
        }
    }

    fn stmt(&mut self) -> Parsed<Stmt> {
        let start = self.span();
        match self.peek() {
            _ if self.at_fill() => self.fill_stmt(),
            _ if self.at_binding() => {
                let (mutable, open) = self.binding_words();
                self.binding(start, mutable, open)
            }
            Tok::Kw("unsafe") => {
                self.bump();
                let body = self.block()?;
                Ok(Stmt::Unsafe { body, span: start.to(self.prev_span()) })
            }
            Tok::Kw("destroy") => {
                self.bump();
                let target = self.expr()?;
                self.expect(";")?;
                Ok(Stmt::Destroy { target, span: start.to(self.prev_span()) })
            }
            Tok::Kw("if") => self.if_stmt(),
            Tok::Kw("while") => {
                self.bump();
                let cond = self.condition("while")?;
                let span = start.to(self.prev_span());
                let body = self.block()?;
                Ok(Stmt::While { cond, body, span })
            }
            Tok::Kw("for") => self.for_stmt(),
            Tok::Kw("match") => self.match_stmt(),
            Tok::Kw(k @ ("break" | "continue")) => {
                let word = *k;
                self.bump();
                let count = self.count(word)?;
                self.end_jump(word)?;
                Ok(Stmt::Break { cont: word == "continue", count, span: start.to(self.prev_span()) })
            }
            Tok::Kw("skip") => {
                self.bump();
                // `to` is a keyword only here, directly after `skip`.
                let target = if matches!(self.peek(), Tok::Ident(s) if s == "to") {
                    self.bump();
                    if !matches!(self.peek(), Tok::Ident(s) if s == "next") {
                        return Err(self
                            .unexpected("`next` after `skip to`")
                            .help("`skip to next;` jumps to the next branch of the `if` chain"));
                    }
                    self.bump();
                    SkipTarget::Next
                } else {
                    match self.count("skip")? {
                        Some((n, span)) => SkipTarget::Levels(n, span),
                        None => SkipTarget::Outermost,
                    }
                };
                self.end_jump("skip")?;
                Ok(Stmt::Skip { target, span: start.to(self.prev_span()) })
            }
            Tok::Kw("fail") => {
                self.bump();
                let value = self.expr()?;
                let because = if self.peek() == &Tok::Kw("because") {
                    self.bump();
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect(";")?;
                Ok(Stmt::Fail { value, because, span: start.to(self.prev_span()) })
            }
            Tok::Kw("maybe") => {
                self.bump();
                let value = if self.peek() == &Tok::Sym(";") { None } else { Some(self.expr()?) };
                self.expect(";")?;
                Ok(Stmt::Resume { value, span: start.to(self.prev_span()) })
            }
            Tok::Kw("return") => {
                self.bump();
                let value = if self.peek() == &Tok::Sym(";") { None } else { Some(self.expr()?) };
                self.expect(";")?;
                Ok(Stmt::Return { value, span: start.to(self.prev_span()) })
            }
            _ => {
                let s = self.simple()?;
                self.expect(";")?;
                Ok(s)
            }
        }
    }

    /// Does a `fill` statement start here? `fill` is a context word: `fill(3)`
    /// and `fill = 3;` are ordinary.
    fn at_fill(&self) -> bool {
        let word = |n: usize, w: &str| self.peek_at(n) == &Tok::Ident(w.to_string());
        word(0, "fill")
            && ((matches!(self.peek_at(1), Tok::Ident(_)) && matches!(self.peek_at(2), Tok::Sym("{") | Tok::Kw("with")))
                || (word(1, "partial") && matches!(self.peek_at(2), Tok::Ident(_))))
    }

    /// `fill [partial] x { ... }` or `fill x with defaults { ... }`.
    fn fill_stmt(&mut self) -> Parsed<Stmt> {
        let start = self.bump().span;
        let partial = if self.peek() == &Tok::Ident("partial".to_string()) {
            self.bump();
            true
        } else {
            false
        };
        let target = self.ident("a binding to fill")?;
        let with = if self.peek() == &Tok::Kw("with") {
            self.bump();
            if self.peek() == &Tok::Ident("defaults".to_string()) {
                Some(FillWith::Defaults(self.bump().span))
            } else {
                let (n, sp) = self.qual_ident("`defaults` or the name of a def after `with`")?;
                Some(FillWith::Named(n, sp))
            }
        } else {
            None
        };
        let body = self.block()?;
        Ok(Stmt::Fill { partial, target, with, body, span: start.to(self.prev_span()) })
    }

    fn eat_kw(&mut self, kw: &'static str) -> bool {
        if self.peek() == &Tok::Kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// The optional count after `break`, `continue` or `skip`: a plain whole number.
    fn count(&mut self, word: &str) -> Parsed<Option<(u64, Span)>> {
        match self.peek() {
            Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => {
                let n = v.to_u64().unwrap_or(u64::MAX);
                Ok(Some((n, self.bump().span)))
            }
            Tok::Num(..) => Err(Diagnostic::at(
                &codes::E0003,
                self.span(),
                format!("the count after `{word}` must be a plain whole number"),
            )
            .help(format!("for example `{word} 2;`"))),
            _ => Ok(None),
        }
    }

    /// The `;` that ends `break`, `continue` or `skip`.
    fn end_jump(&mut self, word: &str) -> Parsed<()> {
        if self.eat(";") {
            return Ok(());
        }
        let d = self.unexpected("`;`");
        Err(if word == "skip" {
            d.help("write `skip;`, `skip 2;` or `skip to next;`")
        } else {
            d.help(format!("the count after `{word}` is a whole number written out, like `{word} 2;`"))
        })
    }

    /// An assignment, compound assignment or expression statement, without its `;`.
    fn simple(&mut self) -> Parsed<Stmt> {
        let start = self.span();
        let e = self.expr()?;
        let op = match self.peek() {
            Tok::Sym("=") => Some(None),
            Tok::Sym(s) => compound_op(s).map(Some),
            _ => None,
        };
        let Some(op) = op else { return Ok(Stmt::Expr(e)) };
        if !is_place(&e) {
            return Err(Diagnostic::at(&codes::E0005, e.span, "only a binding or a field can be assigned to").note("cannot assign to this"));
        }
        self.bump();
        let value = self.expr()?;
        if self.peek() == &Tok::Sym(",") {
            return Err(self
                .unexpected("`;`")
                .help("building a struct from a list of values works only in a declaration: `Point p = 1, 2;`"));
        }
        Ok(Stmt::Assign { target: e, op, value, span: start.to(self.prev_span()) })
    }

    /// `for (T x of iter) { }` or `for (init; cond; step) { }`.
    fn for_stmt(&mut self) -> Parsed<Stmt> {
        let start = self.bump().span;
        if self.peek() != &Tok::Sym("(") {
            return Err(self
                .unexpected("`(` after `for`")
                .help("write `for (uint32 i of 0..9) { }` or `for (init; condition; step) { }`"));
        }
        self.bump();

        let m = (self.peek() == &Tok::Kw("mut")) as usize;
        // `for (T name of ...)`, where `T` may start with `&` (`for (&string c of text)`).
        let name_at = self.type_end(m);
        if let Some(name_at) = name_at
            && matches!(self.peek_at(name_at), Tok::Ident(_))
            && matches!(self.peek_at(name_at + 1), Tok::Kw("of") | Tok::Sym(","))
        {
            let mutable = self.eat_kw("mut");
            let ty = self.ty()?;
            let (name, name_span) = self.ident("a name for the loop variable")?;
            if self.peek() == &Tok::Sym(",") {
                // `for (K k, V v of map)`: walking a map's entries (Q16).
                self.bump();
                let value_mutable = self.eat_kw("mut");
                let value_ty = self.ty()?;
                let (value_name, value_span) = self.ident("a name for the value")?;
                if self.peek() != &Tok::Kw("of") {
                    return Err(self.unexpected("`of`"));
                }
                self.bump();
                let iter = self.expr()?;
                self.expect(")")?;
                let span = start.to(self.prev_span());
                let body = self.block()?;
                let key = LoopVar { mutable, ty, name, name_span };
                let value = LoopVar { mutable: value_mutable, ty: value_ty, name: value_name, name_span: value_span };
                return Ok(Stmt::ForEntries { key, value, iter, body, span });
            }
            self.bump();
            let iter = self.expr()?;
            let step = self.range_step(&iter)?;
            self.expect(")")?;
            let span = start.to(self.prev_span());
            let body = self.block()?;
            return Ok(Stmt::ForEach { mutable, ty, name, name_span, iter, step, body, span });
        }

        let init = if self.eat(";") {
            None
        } else if self.at_binding() {
            let at = self.span();
            let (mutable, open) = self.binding_words();
            Some(Box::new(self.binding(at, mutable, open)?))
        } else {
            let s = self.simple()?;
            self.expect(";")?;
            Some(Box::new(s))
        };
        let cond = if self.peek() == &Tok::Sym(";") { None } else { Some(self.expr()?) };
        self.expect(";")?;
        let step = if self.peek() == &Tok::Sym(")") {
            None
        } else {
            let s = self.simple()?;
            if let Stmt::Expr(e) = &s
                && !matches!(e.kind, ExprKind::IncDec { .. } | ExprKind::Call { .. } | ExprKind::Method { .. })
            {
                return Err(Diagnostic::at(&codes::E0013, e.span, "a `for` step must be an assignment, `++`/`--` or a call")
                    .note("this is none of those")
                    .help("for example `i++` or `i += 2`"));
            }
            Some(Box::new(s))
        };
        self.expect(")")?;
        let span = start.to(self.prev_span());
        let body = self.block()?;
        Ok(Stmt::For { init, cond, step, body, span })
    }

    /// The `by n` after the range of a `for ... of a..b by n`, if there is one.
    /// `by` is a context word: an ordinary name everywhere else, so it only
    /// counts right after a range.
    fn range_step(&mut self, iter: &Expr) -> Parsed<Option<Expr>> {
        if !matches!(self.peek(), Tok::Ident(w) if w == "by") {
            return Ok(None);
        }
        let at = self.span();
        if !matches!(iter.kind, ExprKind::Range(..)) {
            return Err(Diagnostic::at(&codes::E0003, at, "`by` goes after a range")
                .note("this is not a range")
                .help("write `for (uint32 i of 0..10 by 3) { }`"));
        }
        self.bump();
        Ok(Some(self.expr()?))
    }

    /// `match (value) { case p, q { } case _ { } }`
    fn match_stmt(&mut self) -> Parsed<Stmt> {
        let start = self.bump().span;
        let value = self.condition("match")?;
        let span = start.to(self.prev_span());
        let open = self.expect("{")?;
        let mut cases = Vec::new();
        while self.peek() != &Tok::Sym("}") {
            if self.peek() == &Tok::Eof || self.at_item_start() {
                return Err(Diagnostic::at(&codes::E0004, open, "this `{` is never closed")
                    .label(self.span(), format!("expected `}}` before {}", self.peek().describe())));
            }
            if self.peek() != &Tok::Kw("case") {
                let d = self.unexpected("`case` or `}`").help("each arm of a `match` is written `case pattern { }`");
                self.diags.push(d);
                self.skip_statement();
                continue;
            }
            let case_start = self.bump().span;
            let mut patterns = Vec::new();
            loop {
                if matches!(self.peek(), Tok::Ident(s) if s == "_") {
                    patterns.push(Pattern::Any(self.bump().span));
                } else if matches!(self.peek(), Tok::Ident(s) if s == "globbed") && matches!(self.peek_at(1), Tok::Str(_)) {
                    // `globbed` is a word only here, before a string: elsewhere it is a name.
                    self.bump();
                    patterns.push(Pattern::Glob(self.expr()?));
                } else if let Some(p) = self.enum_pattern()? {
                    patterns.push(p);
                } else {
                    patterns.push(Pattern::Expr(self.expr()?));
                }
                if !self.eat(",") {
                    break;
                }
            }
            let guard = if self.peek() == &Tok::Kw("if") {
                self.bump();
                Some(self.expr()?)
            } else {
                None
            };
            let case_span = case_start.to(self.prev_span());
            let body = self.block()?;
            cases.push(Case { patterns, guard, body, span: case_span });
        }
        self.bump();
        Ok(Stmt::Match { value, cases, span })
    }

    /// A `case` pattern that is not an expression: `Variant(..)`,
    /// `Enum::Variant(..)`, or `Type name` (the value inside an optional).
    fn enum_pattern(&mut self) -> Parsed<Option<Pattern>> {
        let mode = match self.peek() {
            Tok::Kw("take") => Some(BindMode::Take),
            Tok::Kw("copy") => Some(BindMode::Copy),
            _ => None,
        };
        if mode.is_some() || matches!(self.peek(), Tok::Ident(_) | Tok::Sym("&")) {
            let from = mode.is_some() as usize;
            if let Some(n) = self.type_end(from)
                && matches!(self.peek_at(n), Tok::Ident(_))
                && matches!(self.peek_at(n + 1), Tok::Sym("," | "{") | Tok::Kw("if"))
            {
                if mode.is_some() {
                    self.bump();
                }
                let ty = self.ty()?;
                let (name, name_span) = self.ident("a name for the value")?;
                return Ok(Some(Pattern::Bind { mode: mode.unwrap_or(BindMode::Plain), ty, name, name_span }));
            }
        }
        let k = if matches!(self.peek(), Tok::Ident(_)) { self.qual_end(0) } else { 1 };
        if !matches!(self.peek(), Tok::Ident(_)) || self.peek_at(k) != &Tok::Sym("(") {
            return Ok(None);
        }
        self.variant_pattern().map(Some)
    }

    /// `Name(sub, ...)` or `Enum::Name(sub, ...)`; also a bare `Name` inside another pattern.
    fn variant_pattern(&mut self) -> Parsed<Pattern> {
        let start = self.span();
        let mut enum_name = None;
        let (mut name, mut name_span) = self.qual_ident("a variant name")?;
        // `Enum::Variant` or `module::Enum::Variant`: the last part is the variant.
        if let Some((owner, last)) = name.rsplit_once("::") {
            let owner_span = Span::new(name_span.start, name_span.end - last.len() - 2);
            let variant_span = Span::new(name_span.end - last.len(), name_span.end);
            enum_name = Some((owner.to_string(), owner_span));
            name = last.to_string();
            name_span = variant_span;
        }
        let mut args = Vec::new();
        let parens = self.eat("(");
        if parens {
            while self.peek() != &Tok::Sym(")") {
                args.push(self.sub_pattern()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        Ok(Pattern::Variant { enum_name, name, name_span, args, parens, span: start.to(self.prev_span()) })
    }

    /// One slot of a variant pattern.
    fn sub_pattern(&mut self) -> Parsed<SubPat> {
        if matches!(self.peek(), Tok::Ident(w) if w == "_") && matches!(self.peek_at(1), Tok::Sym("," | ")")) {
            return Ok(SubPat::Any(self.bump().span));
        }
        let mode = match self.peek() {
            Tok::Kw("take") => Some(BindMode::Take),
            Tok::Kw("copy") => Some(BindMode::Copy),
            _ => None,
        };
        if mode.is_none() && matches!(self.peek(), Tok::Ident(w) if w == "globbed") && matches!(self.peek_at(1), Tok::Str(_)) {
            // `globbed` is a word only here, before a string: elsewhere it is a name.
            self.bump();
            return Ok(SubPat::Glob(self.expr()?));
        }
        let from = mode.is_some() as usize;
        if let Some(n) = self.type_end(from)
            && matches!(self.peek_at(n), Tok::Ident(_))
            && matches!(self.peek_at(n + 1), Tok::Sym("," | ")"))
        {
            if mode.is_some() {
                self.bump();
            }
            let ty = self.ty()?;
            let (name, name_span) = self.ident("a name for the value")?;
            return Ok(SubPat::Bind { mode: mode.unwrap_or(BindMode::Plain), ty, name, name_span });
        }
        if mode.is_some() {
            return Err(self.unexpected("a type and a name after `take` or `copy`"));
        }
        if matches!(self.peek(), Tok::Ident(_)) {
            return Ok(SubPat::Variant(Box::new(self.variant_pattern()?)));
        }
        Ok(SubPat::Value(self.expr()?))
    }

    /// `Type name;` or `Type name = value;`, after any `mut`.
    fn binding(&mut self, start: Span, mutable: bool, open: bool) -> Parsed<Stmt> {
        let ty = self.ty()?;
        let (name, name_span) = self.ident("a name for the binding")?;
        let value = if self.eat("=") { Some(self.init()?) } else { None };
        if value.is_none() && self.peek() != &Tok::Sym(";") {
            return Err(self.unexpected("`=` or `;`"));
        }
        self.expect(";")?;
        Ok(Stmt::Let { mutable, open, ty, name, name_span, value, span: start.to(self.prev_span()) })
    }

    /// A declaration's value: one expression, or the construction shorthand
    /// `a, b, c` / `x: a, y: b`.
    fn init(&mut self) -> Parsed<Init> {
        let start = self.span();
        let mut values = Vec::new();
        loop {
            let name = match self.peek().clone() {
                Tok::Ident(n) if self.peek_at(1) == &Tok::Sym(":") => {
                    let span = self.bump().span;
                    self.bump();
                    Some((n, span))
                }
                _ => None,
            };
            let value = self.expr()?;
            values.push(FieldValue { name, value });
            if !self.eat(",") {
                break;
            }
        }
        if values.len() == 1 && values[0].name.is_none() {
            return Ok(Init::Expr(values.pop().unwrap().value));
        }
        Ok(Init::Fields(values, start.to(self.prev_span())))
    }

    /// The parenthesised condition after `if` or `while`.
    fn condition(&mut self, keyword: &str) -> Parsed<Expr> {
        if self.peek() != &Tok::Sym("(") {
            return Err(self
                .unexpected(&format!("`(` after `{keyword}`"))
                .help(format!("conditions are always in parentheses: `{keyword} (...) {{ }}`")));
        }
        self.bump();
        let cond = self.expr()?;
        self.expect(")")?;
        Ok(cond)
    }

    fn if_stmt(&mut self) -> Parsed<Stmt> {
        let start = self.bump().span;
        let cond = self.condition("if")?;
        let span = start.to(self.prev_span());
        let mut branches = vec![(cond, self.block()?)];
        let mut els = None;
        while self.eat_kw("else") {
            if self.eat_kw("if") {
                let cond = self.condition("if")?;
                branches.push((cond, self.block()?));
            } else {
                els = Some(self.block()?);
                break;
            }
        }
        Ok(Stmt::If { branches, els, span })
    }

    /// An expression, or an inclusive range `a..b` (which the checker
    /// accepts only in a `for` and in `case` patterns).
    fn expr(&mut self) -> Parsed<Expr> {
        // `now` covers the whole expression after it: `p.max = now p.hp * 2;`.
        if matches!(self.peek(), Tok::Ident(w) if w == "now") && matches!(self.peek_at(1), Tok::Ident(_) | Tok::Kw("self")) {
            let start = self.bump().span;
            let e = self.expr()?;
            return Ok(Expr { span: start.to(e.span), kind: ExprKind::Now(Box::new(e)), paren: false });
        }
        let lhs = self.binary(0)?;
        if !self.eat("..") {
            return Ok(lhs);
        }
        let rhs = self.binary(0)?;
        let span = lhs.span.to(rhs.span);
        Ok(Expr { kind: ExprKind::Range(Box::new(lhs), Box::new(rhs)), span, paren: false })
    }

    /// Precedence climbing. Higher number binds tighter.
    fn binary(&mut self, min_prec: u8) -> Parsed<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec, words)) = self.binary_op_here() {
            if prec < min_prec {
                break;
            }
            let op_start = self.span();
            for _ in 0..words {
                self.bump();
            }
            let op_span = op_start.to(self.prev_span());
            // `**` chains from the right; everything else from the left.
            let rhs = self.binary(if op == BinOp::Pow { prec } else { prec + 1 })?;
            self.check_grouping(op, op_span, &lhs, &rhs);
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span, paren: false };
        }
        Ok(lhs)
    }

    /// The binary operator at the current token, its precedence and how many
    /// tokens it takes. `same as` is two context words: `same` is an ordinary
    /// name everywhere else, and `as` a keyword.
    fn binary_op_here(&self) -> Option<(BinOp, u8, usize)> {
        match self.peek() {
            Tok::Sym(sym) => binop(sym).map(|(op, prec)| (op, prec, 1)),
            Tok::Ident(w) if w == "same" && self.peek_at(1) == &Tok::Kw("as") => Some((BinOp::Same, 3, 2)),
            Tok::Ident(w) if w == "not" && matches!(self.peek_at(1), Tok::Ident(s) if s == "same") && self.peek_at(2) == &Tok::Kw("as") => {
                Some((BinOp::NotSame, 3, 3))
            }
            _ => None,
        }
    }

    /// Bitwise operators have no memorable order against other operators, so
    /// mixing them needs parentheses (shifts inside `& | ^` excepted).
    fn check_grouping(&mut self, op: BinOp, op_span: Span, lhs: &Expr, rhs: &Expr) {
        for child in [lhs, rhs] {
            let ExprKind::Binary(inner, ..) = child.kind else { continue };
            if child.paren {
                continue;
            }
            use OpClass::*;
            let clash = match (op.class(), inner.class()) {
                (Bitwise, Shift) => false,
                (Bitwise, Bitwise) | (Shift, Shift) => op != inner,
                (Bitwise | Shift, Arithmetic | Comparison) | (Arithmetic | Comparison, Bitwise | Shift) => true,
                (Shift, Bitwise) => true,
                _ => false,
            };
            if clash {
                self.diags.push(
                    Diagnostic::at(
                        &codes::E0006,
                        op_span,
                        format!("`{}` and `{}` need parentheses to show which goes first", inner.symbol(), op.symbol()),
                    )
                    .label(child.span, "put this in parentheses, or the other side")
                    .help("bitwise operators have no fixed order against other operators"),
                );
                return;
            }
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let start = self.span();
        match self.peek() {
            Tok::Kw("copy") => {
                self.bump();
                let e = self.unary()?;
                return Ok(Expr { span: start.to(e.span), kind: ExprKind::CopyOf(Box::new(e)), paren: false });
            }
            Tok::Kw("lend") => {
                self.bump();
                let e = self.unary()?;
                return Ok(Expr { span: start.to(e.span), kind: ExprKind::Lend(Box::new(e)), paren: false });
            }
            Tok::Kw("take") => {
                self.bump();
                let place = self.unary()?;
                let mut leaving = None;
                let mut reorders = false;
                if matches!(self.peek(), Tok::Ident(w) if w == "leaving") {
                    self.bump();
                    leaving = Some(Box::new(self.binary(0)?));
                } else if matches!(self.peek(), Tok::Ident(w) if w == "reorders") {
                    self.bump();
                    reorders = true;
                }
                let span = start.to(self.prev_span());
                return Ok(Expr { span, kind: ExprKind::Take { place: Box::new(place), leaving, reorders }, paren: false });
            }
            Tok::Ident(w) if w == "address" && self.peek_at(1) == &Tok::Kw("of") => {
                self.bump();
                self.bump();
                let e = self.unary()?;
                return Ok(Expr { span: start.to(e.span), kind: ExprKind::AddressOf(Box::new(e)), paren: false });
            }
            _ => {}
        }
        let Tok::Sym(sym) = *self.peek() else { return self.postfix() };
        let op = match sym {
            "&" => {
                self.bump();
                let mutable = self.eat_kw("mut");
                let operand = self.unary()?;
                let span = start.to(operand.span);
                return Ok(Expr { kind: ExprKind::Borrow { mutable, operand: Box::new(operand) }, span, paren: false });
            }
            "&&" => return Err(borrow_of_borrow(start)),
            "*" | "**" => {
                self.bump();
                let mut e = self.unary()?;
                for _ in 0..(1 + (sym == "**") as usize) {
                    e = Expr { span: start.to(e.span), kind: ExprKind::Deref(Box::new(e)), paren: false };
                }
                return Ok(e);
            }
            "-" => UnOp::Neg,
            "!" => UnOp::Not,
            "~" => UnOp::BitNot,
            "++" | "--" => {
                self.bump();
                let target = self.unary()?;
                let span = start.to(target.span);
                return Ok(Expr { kind: ExprKind::IncDec { inc: sym == "++", prefix: true, target: Box::new(target) }, span, paren: false });
            }
            _ => return self.postfix(),
        };
        self.bump();
        let e = self.unary()?;
        let span = start.to(e.span);
        Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), span, paren: false })
    }

    /// A primary expression followed by field accesses, method calls and `++`/`--`.
    fn postfix(&mut self) -> Parsed<Expr> {
        let e = self.postfix_chain()?;
        self.optional_suffix(e)
    }

    /// `value maybe fallback`, `value has value`, `value is empty` (after a
    /// primary and its calls and fields). The fallback is one value, or a
    /// parenthesised expression: a binary operator after it is E0534.
    fn optional_suffix(&mut self, mut e: Expr) -> Parsed<Expr> {
        loop {
            let start = e.span;
            if self.peek() == &Tok::Kw("maybe") {
                self.bump();
                let fallback = self.unary()?;
                if let Tok::Sym(sym) = self.peek()
                    && binop(sym).is_some()
                {
                    let op_span = self.span();
                    return Err(Diagnostic::at(&codes::E0534, op_span, "the fallback of `maybe` is one value")
                        .note("this operator would be part of the fallback")
                        .help("put the fallback in parentheses: `(x maybe 0) + 1`, or `x maybe (n * 2)`"));
                }
                let span = start.to(fallback.span);
                e = Expr { kind: ExprKind::Maybe { value: Box::new(e), fallback: Box::new(fallback) }, span, paren: false };
                continue;
            }
            let has = match (self.peek(), self.peek_at(1)) {
                (Tok::Ident(a), Tok::Ident(b)) if a == "has" && b == "value" => true,
                (Tok::Ident(a), Tok::Ident(b)) if a == "is" && b == "empty" => false,
                _ => return Ok(e),
            };
            self.bump();
            let end = self.bump().span;
            e = Expr { kind: ExprKind::OptionalTest { value: Box::new(e), has }, span: start.to(end), paren: false };
        }
    }

    fn postfix_chain(&mut self) -> Parsed<Expr> {
        let mut e = self.primary()?;
        let mut calls = 0;
        loop {
            let ignore = self.peek() == &Tok::Ident("ignore".to_string()) && self.peek_at(1) == &Tok::Kw("failure");
            if ignore || matches!(self.peek(), Tok::Kw("fails")) {
                return self.call_suffix(e);
            }
            if self.peek() == &Tok::Sym("[") {
                self.bump();
                let index = self.expr()?;
                let end = self.expect("]")?;
                let span = e.span.to(end);
                e = Expr { kind: ExprKind::Index { recv: Box::new(e), index: Box::new(index) }, span, paren: false };
                // An item is a new value: the next call on it starts with `::` again.
                calls = 0;
                continue;
            }
            let sym = match self.peek() {
                Tok::Sym(s @ ("::" | "->" | "++" | "--" | ".")) => *s,
                _ => return Ok(e),
            };
            let op_span = self.bump().span;
            if sym == "." {
                let (name, name_span) = self.ident("a field name")?;
                let span = e.span.to(name_span);
                e = Expr { kind: ExprKind::Field { recv: Box::new(e), name, name_span }, span, paren: false };
                // Undecided by the SPEC, chosen here: a field is a new value, so
                // the next call on it starts with `::` again (`p::get().x::text()`).
                calls = 0;
                continue;
            }
            if sym == "++" || sym == "--" {
                let span = e.span.to(op_span);
                e = Expr { kind: ExprKind::IncDec { inc: sym == "++", prefix: false, target: Box::new(e) }, span, paren: false };
                continue;
            }
            // `::` makes the first call on a value, `->` each one after it.
            let want = if calls == 0 { "::" } else { "->" };
            if sym != want {
                let why = if calls == 0 {
                    "the first call on a value uses `::`"
                } else {
                    "after the first call, each further call in the chain uses `->`"
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0007, op_span, format!("expected `{want}` here, found `{sym}`"))
                        .note(format!("write `{want}`"))
                        .help(format!("{why}: `value::first()->second()`")),
                );
            }
            calls += 1;
            // After `::` a reserved word cannot be anything but a name (`io::out`).
            let (name, name_span) = match *self.peek() {
                Tok::Kw(k) => (k.to_string(), self.bump().span),
                _ => self.ident("a method name")?,
            };
            let mut targs = Vec::new();
            let mut unit = None;
            if name == "in" && self.peek() == &Tok::Sym("<") {
                self.bump();
                unit = Some(self.unit_spec()?);
                self.close_angle()?;
            } else if self.eat("<") {
                loop {
                    targs.push(self.ty()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.close_angle()?;
            }
            // `Enum::Variant` without parentheses names a variant with no values.
            if calls == 1
                && targs.is_empty()
                && unit.is_none()
                && self.peek() != &Tok::Sym("(")
                && let Some(owner) = match &e.kind {
                    ExprKind::Var(owner) => Some(owner.clone()),
                    // `module::Enum::Variant`: the module and the enum are the owner.
                    ExprKind::Path { owner, name, .. } => Some(format!("{owner}::{name}")),
                    _ => None,
                }
            {
                let owner_span = e.span;
                let span = e.span.to(name_span);
                e = Expr { kind: ExprKind::Path { owner, owner_span, name, name_span }, span, paren: false };
                calls = 0;
                continue;
            }
            let args = self.args()?;
            let span = e.span.to(self.prev_span());
            e = Expr { kind: ExprKind::Method { recv: Box::new(e), name, name_span, targs, unit, args }, span, paren: false };
        }
    }

    /// `call maybe fallback` or `call fails`, after the call. The fallback is a
    /// single operand; anything longer needs parentheses.
    fn call_suffix(&mut self, call: Expr) -> Parsed<Expr> {
        let word = self.bump();
        if word.tok == Tok::Ident("ignore".to_string()) {
            let end = self.bump().span;
            let span = call.span.to(end);
            let handler = Handler { kind: HandlerKind::Ignore, bind: None, body: Block { stmts: Vec::new(), end }, span: word.span.to(end) };
            return Ok(Expr { kind: ExprKind::Handled { call: Box::new(call), handlers: vec![handler] }, span, paren: false });
        }
        if word.tok == Tok::Kw("fails") {
            if self.peek() == &Tok::Kw("with") {
                return self.handlers(call, word.span);
            }
            let span = call.span.to(word.span);
            return Ok(Expr { kind: ExprKind::PassUp(Box::new(call)), span, paren: false });
        }
        unreachable!("call_suffix is only for `fails` and `ignore failure`")
    }

    /// The handlers after `call fails`: `with none`, or one or more
    /// `with (Type e) => { }` / `with (any e) => { }`, then optionally
    /// `otherwise (any e) => { }`.
    fn handlers(&mut self, call: Expr, mut start: Span) -> Parsed<Expr> {
        let mut handlers = Vec::new();
        let mut end;
        loop {
            let otherwise = self.peek() == &Tok::Kw("otherwise");
            if !otherwise {
                if self.peek() != &Tok::Kw("with") {
                    return Err(self.unexpected("`with`"));
                }
                self.bump();
            } else {
                self.bump();
            }
            if !otherwise && self.peek() == &Tok::Kw("none") {
                end = self.bump().span;
                handlers.push(Handler { kind: HandlerKind::Ignore, bind: None, body: Block { stmts: Vec::new(), end }, span: start.to(end) });
            } else {
                self.expect("(")?;
                let kind = if self.peek() == &Tok::Kw("any") {
                    self.bump();
                    if otherwise { HandlerKind::Otherwise } else { HandlerKind::Any }
                } else if otherwise {
                    return Err(self.unexpected("`any`").help("write `otherwise (any e) => { ... }`"));
                } else {
                    let (name, span) = self.qual_ident("a failure type")?;
                    HandlerKind::Type(name, span)
                };
                let bind = self.ident("a name for the failure")?;
                self.expect(")")?;
                self.expect("=>")?;
                let body = self.block()?;
                end = body.end;
                handlers.push(Handler { kind, bind: Some(bind), body, span: start.to(end) });
            }
            if self.peek() == &Tok::Kw("fails") && self.peek_at(1) == &Tok::Kw("with") {
                start = self.bump().span;
            } else if self.peek() == &Tok::Kw("otherwise") {
                start = self.span();
            } else {
                break;
            }
        }
        let span = call.span.to(end);
        Ok(Expr { kind: ExprKind::Handled { call: Box::new(call), handlers }, span, paren: false })
    }

    /// `(a, b, c)`
    fn args(&mut self) -> Parsed<Vec<Expr>> {
        self.expect("(")?;
        let mut args = Vec::new();
        while self.peek() != &Tok::Sym(")") {
            args.push(self.expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    /// Does a unit term start here? (`NAME`, `(`, or the `1` of a factor-free unit.)
    fn starts_unit_term(&self) -> bool {
        match self.peek() {
            Tok::Ident(_) | Tok::Sym("(") => true,
            Tok::Num(NumLit { value: Const::Int(v), suffix: None, .. }, None) => *v == num_bigint::BigInt::from(1),
            _ => false,
        }
    }

    /// A unit literal's text (`9.6<s>` gives `s`; `span` covers that text) as a
    /// unit expression. The lexer has already checked the text, so it is
    /// split here with plain rules: names, whole numbers and `* / ^ - ( )`.
    fn unit_literal(&mut self, text: &str, span: Span) -> Parsed<UnitSpec> {
        let bytes = text.as_bytes();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b' ' {
                i += 1;
                continue;
            }
            let from = i;
            let tok = if c.is_ascii_alphabetic() || c == b'_' {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                match &text[from..i] {
                    "as" => Tok::Kw("as"),
                    word => Tok::Ident(word.to_string()),
                }
            } else if c.is_ascii_digit() {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let value: num_bigint::BigInt = text[from..i].parse().unwrap_or_default();
                Tok::Num(NumLit { value: Const::Int(value), suffix: None, unit: None }, None)
            } else {
                i += 1;
                Tok::Sym(match c {
                    b'*' => "*",
                    b'/' => "/",
                    b'^' => "^",
                    b'-' => "-",
                    b'(' => "(",
                    _ => ")",
                })
            };
            tokens.push(Token { tok, span: Span::new(span.start + from, span.start + i) });
        }
        tokens.push(Token { tok: Tok::Eof, span: Span::new(span.end, span.end) });
        let mut inner = Parser { tokens, pos: 0, diags: &mut *self.diags, optionals: Vec::new(), generics: Vec::new() };
        let spec = inner.unit_spec()?;
        if inner.peek() != &Tok::Eof {
            return Err(inner.unexpected("the end of the unit"));
        }
        Ok(spec)
    }

    /// `unit NAME;` or `unit NAME = def [offset SIGNED];`. Called at `unit`.
    fn unit_decl(&mut self) -> Parsed<UnitDecl> {
        let start = self.bump().span;
        let (name, name_span) = self.ident("a unit name")?;
        let mut def = None;
        let mut offset = None;
        if self.eat("=") {
            def = Some(self.unit_def()?);
            if matches!(self.peek(), Tok::Ident(w) if w == "offset") {
                self.bump();
                offset = Some(self.signed_number()?);
            }
        }
        self.expect(";")?;
        Ok(UnitDecl { name, name_span, def, offset, span: start.to(self.prev_span()) })
    }

    /// The right side of `unit NAME =`: `[NUMBER] [unit-expr]`, or
    /// `NUMBER / unit-expr` (the number is the factor, the rest is `1 / ...`).
    fn unit_def(&mut self) -> Parsed<UnitDef> {
        let mut factor = None;
        let mut expr = None;
        match self.peek().clone() {
            Tok::Num(NumLit { value, suffix, unit: None }, None) => {
                let span = self.bump().span;
                if suffix.is_some() {
                    return Err(Diagnostic::at(&codes::E0003, span, "a unit's factor is a plain number")
                        .help("write the factor without a number type, for example `unit km = 1000 m;`"));
                }
                factor = Some((value, span));
                if self.eat("/") {
                    let rhs = self.unit_term()?;
                    let one = UnitExpr::Div(Box::new(UnitExpr::One(span)), Box::new(rhs));
                    expr = Some(self.unit_expr_rest(one)?);
                } else if self.starts_unit_term() {
                    expr = Some(self.unit_expr()?);
                }
            }
            _ if self.starts_unit_term() => expr = Some(self.unit_expr()?),
            _ => return Err(self.unexpected("a number or a unit after `=`")),
        }
        Ok(UnitDef { factor, expr })
    }

    /// The value after `offset`: an optional `-` and a plain number.
    fn signed_number(&mut self) -> Parsed<(Const, Span)> {
        let start = self.span();
        let neg = self.eat("-");
        match self.peek().clone() {
            Tok::Num(NumLit { value, suffix: None, unit: None }, None) => {
                let span = start.to(self.bump().span);
                let value = match value {
                    Const::Int(v) if neg => Const::Int(-v),
                    Const::Dec(v) if neg => Const::Dec(-v),
                    other => other,
                };
                Ok((value, span))
            }
            _ => Err(self.unexpected("a plain number after `offset`")),
        }
    }

    fn lambda(&mut self, moves: bool) -> Parsed<ExprKind> {
        let params = self.params()?;
        self.expect("=>")?;
        let body = if self.peek() == &Tok::Sym("{") {
            LambdaBody::Block(self.block()?)
        } else {
            LambdaBody::Expr(Box::new(self.expr()?))
        };
        Ok(ExprKind::Lambda { moves, params, body })
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let t = self.bump();
        let kind = match t.tok {
            Tok::Num(mut n, unit) => {
                if let Some((text, span)) = unit {
                    n.unit = Some(self.unit_literal(&text, span)?);
                }
                ExprKind::Num(n)
            }
            Tok::Str(pieces) => ExprKind::Str(self.string_parts(pieces)?),
            Tok::Kw("self") => ExprKind::SelfValue,
            Tok::Kw("true") => ExprKind::Bool(true),
            Tok::Kw("false") => ExprKind::Bool(false),
            Tok::Kw("none") => ExprKind::NoneValue,
            Tok::Kw("new") => {
                let (name, name_span) = self.qual_ident("a struct name after `new`")?;
                let targs = if self.peek() == &Tok::Sym("<") && self.type_start_at(1) { self.type_args()? } else { Vec::new() };
                if !targs.is_empty() {
                    self.generics.push(TypeName { args: targs.clone(), ..TypeName::plain(name.clone(), name_span) });
                }
                if self.peek() != &Tok::Kw("of") {
                    return Err(self.unexpected("`of`").help(format!("write `new {name} of (...)`: the values go in parentheses")));
                }
                self.bump();
                self.expect("(")?;
                let mut values = Vec::new();
                while self.peek() != &Tok::Sym(")") {
                    let name = match self.peek().clone() {
                        Tok::Ident(n) if self.peek_at(1) == &Tok::Sym(":") => {
                            let span = self.bump().span;
                            self.bump();
                            Some((n, span))
                        }
                        _ => None,
                    };
                    let value = self.expr()?;
                    values.push(FieldValue { name, value });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                ExprKind::New { name, name_span, targs, values }
            }
            Tok::Sym("[") => {
                let mut items = Vec::new();
                while self.peek() != &Tok::Sym("]") {
                    items.push(self.expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("]")?;
                ExprKind::ListLit(items)
            }
            Tok::Kw("move") => {
                self.expect("(")?;
                self.lambda(true)?
            }
            Tok::Sym("(") if self.after_open_paren(0).is_some_and(|n| self.peek_at(n) == &Tok::Sym("=>")) => self.lambda(false)?,
            Tok::Sym("{") => {
                // In expression position `{` starts a map literal (a block is a statement).
                let mut pairs = Vec::new();
                while self.peek() != &Tok::Sym("}") {
                    let k = self.expr()?;
                    self.expect(":")?;
                    pairs.push((k, self.expr()?));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                ExprKind::MapLit(pairs)
            }
            Tok::Sym("(") => {
                let mut e = self.expr()?;
                self.expect(")")?;
                e.span = t.span.to(self.prev_span());
                e.paren = true;
                return Ok(e);
            }
            Tok::Ident(name) => {
                if self.peek() == &Tok::Sym("(") {
                    ExprKind::Call { name, targs: Vec::new(), args: self.args()? }
                } else if self.expr_targs_ahead() {
                    let targs = self.type_args()?;
                    self.generics.push(TypeName { args: targs.clone(), ..TypeName::plain(name.clone(), t.span) });
                    if self.peek() == &Tok::Sym("(") {
                        ExprKind::Call { name, targs, args: self.args()? }
                    } else {
                        ExprKind::TypeArgs { name, targs }
                    }
                } else {
                    ExprKind::Var(name)
                }
            }
            other => {
                if other != Tok::Eof {
                    self.pos -= 1;
                }
                return Err(Diagnostic::at(
                    &codes::E0003,
                    t.span,
                    format!("expected an expression, found {}", other.describe()),
                ));
            }
        };
        Ok(Expr { kind, span: t.span.to(self.prev_span()), paren: false })
    }
}

impl Parser<'_> {
    fn string_parts(&mut self, pieces: Vec<StrPiece>) -> Parsed<Vec<StrPart>> {
        pieces
            .into_iter()
            .map(|p| {
                Ok(match p {
                    StrPiece::Raw(text, span) => StrPart::Raw(text, span),
                    StrPiece::Escape(ch, span) => StrPart::Escape(ch, span),
                    StrPiece::Interp(tokens, span) => StrPart::Interp(self.interpolated(tokens, span)?),
                })
            })
            .collect()
    }

    /// The expression inside `${...}`, from the tokens the lexer found there
    /// (which end with the closing `}`).
    fn interpolated(&mut self, tokens: Vec<Token>, span: Span) -> Parsed<Expr> {
        if tokens.len() == 2 {
            return Err(Diagnostic::at(&codes::E0003, span, "`${}` needs an expression inside")
                .note("empty")
                .help("for example `${name}`; write `\\$` for a plain dollar sign"));
        }
        let mut inner = Parser { tokens, pos: 0, diags: &mut *self.diags, optionals: Vec::new(), generics: Vec::new() };
        let e = inner.expr()?;
        let found = std::mem::take(&mut inner.optionals);
        self.optionals.extend(found);
        let found = std::mem::take(&mut inner.generics);
        self.generics.extend(found);
        if inner.peek() != &Tok::Sym("}") {
            return Err(inner.unexpected("`}` to end the `${`"));
        }
        Ok(e)
    }
}

/// Can `e` be assigned to: a name, `self`, or a field of one of those?
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::SelfValue => true,
        ExprKind::Field { recv, .. } | ExprKind::Index { recv, .. } => is_place(recv),
        ExprKind::Deref(inner) => is_place(inner),
        _ => false,
    }
}

fn binop(sym: &str) -> Option<(BinOp, u8)> {
    use BinOp::*;
    Some(match sym {
        "||" => (Or, 1),
        "&&" => (And, 2),
        "==" => (Eq, 3),
        "!=" => (Ne, 3),
        "<" => (Lt, 3),
        "<=" => (Le, 3),
        ">" => (Gt, 3),
        ">=" => (Ge, 3),
        "&" => (BitAnd, 4),
        "|" => (BitOr, 4),
        "^" => (BitXor, 4),
        "<<" => (Shl, 5),
        ">>" => (Shr, 5),
        "+" => (Add, 6),
        "-" => (Sub, 6),
        "*" => (Mul, 7),
        "/" => (Div, 7),
        "%" => (Rem, 7),
        "**" => (Pow, 8),
        _ => return None,
    })
}

fn compound_op(sym: &str) -> Option<BinOp> {
    let op = sym.strip_suffix('=')?;
    match binop(op)? {
        (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or, _) => None,
        (op, _) => Some(op),
    }
}

fn borrow_of_borrow(span: Span) -> Diagnostic {
    Diagnostic::at(&codes::E0901, span, "borrowing a borrow is not supported yet")
        .help("use `lend r` to pass on a `&mut`")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_src(src: &str) -> (Program, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let tokens = crate::lexer::lex(src, &mut diags);
        let program = parse(tokens, &mut diags);
        (program, diags)
    }

    /// Parses `body` as the body of `none main()`.
    fn body(body: &str) -> (Vec<Stmt>, Vec<Diagnostic>) {
        let (mut p, d) = parse_src(&format!("none main() {{ {body} }}"));
        let f = p.funcs.remove(0);
        (f.body.stmts, d)
    }

    fn errors(d: &[Diagnostic]) -> Vec<String> {
        d.iter().map(|d| d.message.clone()).collect()
    }

    /// A type as text, for comparing: `Map<string, uint32[?]>?`.
    fn show_ty(t: &TypeName) -> String {
        let mut s = String::new();
        for (m, _) in &t.mods {
            s.push_str(match m {
                TypeMod::Ref { mutable: false } => "&",
                TypeMod::Ref { mutable: true } => "&mut ",
                TypeMod::Ptr => "*",
                TypeMod::FixedPtr => "*fixed ",
            });
        }
        if let Some(u) = &t.unit_arg {
            return format!("unit {}", unit_str(u));
        }
        s.push_str(&t.name);
        if let Some(u) = &t.unit {
            s.push_str(&format!("<{}>", unit_str(u)));
        }
        if !t.args.is_empty() {
            s.push_str(&format!("<{}>", t.args.iter().map(show_ty).collect::<Vec<_>>().join(", ")));
        }
        for d in &t.dims {
            match d {
                Dim::List(_) => s.push_str("[?]"),
                Dim::Array(n, _) => s.push_str(&format!("[{n}]")),
                Dim::Param(n, _) => s.push_str(&format!("[{n}]")),
                Dim::Calc(..) => s.push_str("[..]"),
            }
        }
        if t.optional.is_some() {
            s.push('?');
        }
        s
    }

    fn let_type(src: &str) -> String {
        let (s, d) = body(src);
        assert!(d.is_empty(), "{src}: {:?}", errors(&d));
        let Stmt::Let { ty, .. } = &s[0] else { panic!("{src}: not a binding: {:?}", s[0]) };
        show_ty(ty)
    }

    #[test]
    fn maybe_borrow_type() {
        // `maybe &T` is the optional of a borrow, written as `Optional<&T>`.
        assert_eq!(let_type("maybe &string f = g();"), "Optional<&string>");
        assert_eq!(let_type("maybe &mut string f;"), "Optional<&mut string>");
        assert_eq!(let_type("List<maybe &string> xs;"), "List<Optional<&string>>");
        assert_eq!(let_type("Optional<&string> f;"), "Optional<&string>");
        let (s, d) = body("maybe &string f = names::first(); mut maybe &string g = none;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(s[0], Stmt::Let { mutable: false, .. }));
        assert!(matches!(s[1], Stmt::Let { mutable: true, .. }));
        // A function may return one, and take one as a parameter.
        let (p, d) = parse_src("maybe &string first(maybe &string fallback) { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(show_ty(&p.funcs[0].ret), "Optional<&string>");
        assert_eq!(show_ty(&p.funcs[0].params[0].ty), "Optional<&string>");
    }

    #[test]
    fn maybe_keeps_its_operator_and_resume_meanings() {
        // `maybe` after a value is the fallback operator, and at a statement
        // start it resumes a failure: neither reads as a type.
        let (s, d) = body("uint32 a = b maybe 1; uint32 c = d maybe &e;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(s[0], Stmt::Let { .. }));
        let (s, d) = body("maybe &x;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(s[0], Stmt::Resume { value: Some(_), .. }));
        let (s, d) = body("maybe 3;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(s[0], Stmt::Resume { value: Some(_), .. }));
    }

    #[test]
    fn maybe_needs_a_borrow() {
        let (_, d) = body("maybe string f = g();");
        assert_eq!(errors(&d), ["expected `&` after `maybe`, found name `string`"]);
    }

    #[test]
    fn generic_type_arguments() {
        assert_eq!(let_type("Vec<uint32> v;"), "Vec<uint32>");
        assert_eq!(let_type("Map<string, uint32> m;"), "Map<string, uint32>");
        assert_eq!(let_type("A<B<C>> x;"), "A<B<C>>");
        assert_eq!(let_type("A<B<C<D>>> x;"), "A<B<C<D>>>");
        assert_eq!(let_type("&Stack<uint32> s = &t;"), "&Stack<uint32>");
        assert_eq!(let_type("Stack<uint32>? s;"), "Stack<uint32>?");
        assert_eq!(let_type("List<List<uint32>> x = y;"), "List<List<uint32>>");
        assert_eq!(let_type("Array<uint8, 4> a;"), "Array<uint8, 4>");
    }

    #[test]
    fn unit_type_arguments() {
        assert_eq!(let_type("Vec3<float64, unit m/s> v;"), "Vec3<float64, unit (m/s)>");
        assert_eq!(let_type("Vec3<T, unit U> v;"), "Vec3<T, unit U>");
        assert_eq!(let_type("Vec3<float64, unit delta degC> v;"), "Vec3<float64, unit delta degC>");
        assert_eq!(let_type("Pair<Vec3<float64, unit m>, unit s> v;"), "Pair<Vec3<float64, unit m>, unit s>");
        // `unit` is only a unit where a unit can start: a type named `unit` stays one.
        assert_eq!(let_type("Stack<unit> v;"), "Stack<unit>");
        let (s, d) = body("uint32 a = twice<unit m>(x); Vec3<float64, unit m>::zero();");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        let ExprKind::Call { targs, .. } = &e.kind else { panic!("{:?}", e.kind) };
        assert!(targs[0].unit_arg.is_some() && targs[0].name.is_empty());
    }

    #[test]
    fn units_stay_units_after_number_types() {
        let (s, d) = body("float64<m/s> v; Vec<float64<m>> w;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { ty, .. } = &s[0] else { panic!() };
        assert!(ty.args.is_empty());
        assert_eq!(type_unit(ty), "(m/s)");
        let Stmt::Let { ty, .. } = &s[1] else { panic!() };
        assert_eq!(ty.name, "Vec");
        assert!(ty.unit.is_none());
        assert_eq!(show_ty(ty), "Vec<float64<m>>");
        assert_eq!(let_type("Map<string, float64<m/s>> m;"), "Map<string, float64<(m/s)>>".replace("(m/s)", "(m/s)"));
    }

    #[test]
    fn lists_and_arrays() {
        assert_eq!(let_type("uint32[?] xs;"), "uint32[?]");
        assert_eq!(let_type("uint8[4] a;"), "uint8[4]");
        assert_eq!(let_type("uint32[?][?] g;"), "uint32[?][?]");
        assert_eq!(let_type("string[?]? m;"), "string[?]?");
        assert_eq!(let_type("&mut string[?] xs = &mut ys;"), "&mut string[?]");
        assert_eq!(let_type("Map<string, uint32[?]> m;"), "Map<string, uint32[?]>");
    }

    #[test]
    fn comparisons_and_indexing_stay_expressions() {
        let (s, d) = body("a < b; a < b && c > d; y = a < b;");
        let _ = &s;
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::Expr(..)), "{:?}", s[0]);
        assert!(matches!(&s[2], Stmt::Assign { .. }), "{:?}", s[2]);
        let (s, d) = body("bool c = a < b; bool e = f(a < b, c > d);");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn operator_method_less_than_is_not_a_type() {
        let (p, d) = parse_src("struct Point { uint32 x; bool <(Point o) { return x < o.x; } }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.structs[0].methods[0].func.name, "<");
    }

    #[test]
    fn type_arguments_in_expressions() {
        let (s, d) = body("uint32 a = max<uint32>(5, 9); Stack<uint32>::zero(); new Stack<uint32> of (1);");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        let ExprKind::Call { name, targs, args } = &e.kind else { panic!("{:?}", e.kind) };
        assert_eq!((name.as_str(), targs.len(), args.len()), ("max", 1, 2));
        let Stmt::Expr(e) = &s[1] else { panic!("{:?}", s[1]) };
        let ExprKind::Method { recv, .. } = &e.kind else { panic!() };
        assert!(matches!(&recv.kind, ExprKind::TypeArgs { name, targs } if name == "Stack" && targs.len() == 1));
        let Stmt::Expr(e) = &s[2] else { panic!() };
        assert!(matches!(&e.kind, ExprKind::New { targs, .. } if targs.len() == 1));
    }

    #[test]
    fn generic_function_heads() {
        let (p, d) = parse_src("T max<T>(T a, T b) { return a; } Stack<T> make<T, U>(U x) { } none f<T with A and B, unit U>() { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.funcs.len(), 3);
        assert_eq!(p.funcs[0].tparams.len(), 1);
        assert_eq!(p.funcs[1].tparams.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["T", "U"]);
        assert_eq!(show_ty(&p.funcs[1].ret), "Stack<T>");
        let t = &p.funcs[2].tparams;
        assert_eq!((t[0].bounds.len(), t[0].is_unit, t[1].is_unit), (2, false, true));
    }

    #[test]
    fn type_syntax_errors() {
        for (src, want) in [("Stack<> s", "expected a type between"), ("Stack<uint32 s", "expected `,` or `>`"), ("uint32[0] a", "at least one item")] {
            let (_, d) = parse_src(&format!("none f({src}) {{ }}"));
            assert!(errors(&d).iter().any(|m| m.contains(want)), "{src}: {:?}", errors(&d));
        }
    }

    #[test]
    fn borrow_types_and_bindings() {
        let (s, d) = body("mut &string r = &x; &mut uint32 m = &mut n; *uint32 p = address of n;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { mutable, ty, value, .. } = &s[0] else { panic!() };
        assert!(*mutable);
        assert_eq!(ty.mods.iter().map(|m| m.0).collect::<Vec<_>>(), [TypeMod::Ref { mutable: false }]);
        let Some(Init::Expr(e)) = value else { panic!() };
        assert!(matches!(e.kind, ExprKind::Borrow { mutable: false, .. }));
        let Stmt::Let { ty, .. } = &s[1] else { panic!() };
        assert_eq!(ty.mods[0].0, TypeMod::Ref { mutable: true });
        let Stmt::Let { ty, value: Some(Init::Expr(e)), .. } = &s[2] else { panic!() };
        assert_eq!(ty.mods[0].0, TypeMod::Ptr);
        assert!(matches!(e.kind, ExprKind::AddressOf(_)));
    }

    #[test]
    fn fixed_pointer_types() {
        let (s, d) = body("*fixed uint32 p = address of n; *fixed Player q; *fixed r;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { ty, .. } = &s[0] else { panic!() };
        assert_eq!(ty.mods.iter().map(|m| m.0).collect::<Vec<_>>(), [TypeMod::FixedPtr]);
        assert_eq!(ty.name, "uint32");
        let Stmt::Let { ty, .. } = &s[1] else { panic!() };
        assert_eq!((ty.mods[0].0, ty.name.as_str()), (TypeMod::FixedPtr, "Player"));
        // `fixed` with no type name after it is an ordinary struct name.
        let Stmt::Let { ty, .. } = &s[2] else { panic!() };
        assert_eq!((ty.mods[0].0, ty.name.as_str()), (TypeMod::Ptr, "fixed"));
    }

    #[test]
    fn open_bindings() {
        let (s, d) = body("open string s; mut open uint32 a = 1;");
        assert!(d.is_empty());
        assert!(matches!(&s[0], Stmt::Let { open: true, mutable: false, .. }));
        assert!(matches!(&s[1], Stmt::Let { open: true, mutable: true, .. }));
        let (p, d) = parse_src("none f(open string s, mut uint32 n) { }");
        assert!(d.is_empty());
        assert!(p.funcs[0].params[0].open && !p.funcs[0].params[0].mutable);
        assert!(p.funcs[0].params[1].mutable);
    }

    #[test]
    fn deref_assignment() {
        let (s, d) = body("*p = 7; *p.f = 7; *p += 1;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Assign { target, .. } = &s[0] else { panic!() };
        assert!(matches!(target.kind, ExprKind::Deref(_)));
        let Stmt::Assign { target, .. } = &s[1] else { panic!() };
        let ExprKind::Deref(inner) = &target.kind else { panic!() };
        assert!(matches!(inner.kind, ExprKind::Field { .. }));
    }

    #[test]
    fn take_copy_lend() {
        let (s, d) = body("string a = take p.name leaving \"x\"; string b = copy r; string c = take p.name; add(lend s);");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        assert!(matches!(&e.kind, ExprKind::Take { leaving: Some(_), .. }));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[1] else { panic!() };
        assert!(matches!(e.kind, ExprKind::CopyOf(_)));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[2] else { panic!() };
        assert!(matches!(&e.kind, ExprKind::Take { leaving: None, .. }));
        let Stmt::Expr(e) = &s[3] else { panic!() };
        let ExprKind::Call { args, .. } = &e.kind else { panic!() };
        assert!(matches!(args[0].kind, ExprKind::Lend(_)));
    }

    #[test]
    fn none_and_unsafe_block() {
        let (s, d) = body("unsafe { *p = 1; } bool z = p == none;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::Unsafe { body, .. } if body.len() == 1));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[1] else { panic!() };
        let ExprKind::Binary(_, _, rhs) = &e.kind else { panic!() };
        assert!(matches!(rhs.kind, ExprKind::NoneValue));
    }

    #[test]
    fn enums_and_variant_patterns_parse() {
        let (p, d) = parse_src("enum Shape { Circle(float64 r), Rect(float64 w, float64 h), Empty, }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let e = &p.enums[0];
        assert_eq!(e.variants.iter().map(|v| v.fields.len()).collect::<Vec<_>>(), [1, 2, 0]);
        let (_, d) = parse_src("enum E { }");
        assert_eq!(d.len(), 1);
        let (_, d) = parse_src("enum E { A, float64 area() { } }");
        assert_eq!(d.len(), 1);
        let (s, d) = body("match (s) { case Circle(float64 r) { } case Shape::Rect(_, float64 h) { } case Empty { } case f(1) { } }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Match { cases, .. } = &s[0] else { panic!() };
        assert!(matches!(&cases[0].patterns[0], Pattern::Variant { args, .. } if args.len() == 1));
        assert!(matches!(&cases[1].patterns[0], Pattern::Variant { enum_name: Some(_), args, .. } if args.len() == 2));
        assert!(matches!(&cases[2].patterns[0], Pattern::Expr(_)));
        assert!(matches!(&cases[3].patterns[0], Pattern::Variant { .. }));
        let (s, d) = body("match (s) { case Text(take string b) { } case Some(Rect(2.0, float64 h)), Other(copy string b) if h > 1.0 { } }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Match { cases, .. } = &s[0] else { panic!() };
        assert!(matches!(&cases[0].patterns[0], Pattern::Variant { args, .. } if matches!(&args[0], SubPat::Bind { mode: BindMode::Take, .. })));
        assert_eq!(cases[1].patterns.len(), 2);
        assert!(cases[1].guard.is_some());
        let Pattern::Variant { args, .. } = &cases[1].patterns[0] else { panic!() };
        let SubPat::Variant(inner) = &args[0] else { panic!() };
        let Pattern::Variant { args, .. } = &**inner else { panic!() };
        assert!(matches!(&args[0], SubPat::Value(_)) && matches!(&args[1], SubPat::Bind { .. }));
    }

    #[test]
    fn same_as_is_a_comparison_and_same_is_still_a_name() {
        let (s, d) = body("bool a = x same as y; bool b = !(x same as &y); uint32 same = 3; uint32 c = same + 1;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        assert!(matches!(e.kind, ExprKind::Binary(BinOp::Same, ..)));
        let (s, d) = body("bool a = x not same as y; uint32 not = 1; uint32 same = not + 1;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        assert!(matches!(e.kind, ExprKind::Binary(BinOp::NotSame, ..)));
        assert_eq!(s.len(), 3);
        let (_, d) = body("bool a = x same as;");
        assert!(!d.is_empty());
    }

    #[test]
    fn binary_and_still_works() {
        let (s, d) = body("uint32 a = b & c; uint32 d = b & &c;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        assert!(matches!(e.kind, ExprKind::Binary(BinOp::BitAnd, ..)));
    }

    #[test]
    fn unsafe_functions_and_clauses() {
        let (p, d) = parse_src("unsafe uint32 f() { } &string pick(&string a, &string b) lives a, b { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(p.funcs[0].is_unsafe.is_some());
        assert_eq!(p.funcs[1].lives.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        let (q, d) = parse_src("&string pick(&string k) lives from self or k { }");
        assert!(d.is_empty(), "{d:?}");
        assert!(q.funcs[0].lives_from);
        assert_eq!(q.funcs[0].lives.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["self", "k"]);
        assert_eq!(p.funcs[1].ret.mods.len(), 1);
    }

    #[test]
    fn failures_and_call_suffixes_parse() {
        let (p, d) = parse_src("failure E { A \"a\" exit 2, B(string k) f\"b ${k}\", } uint32 f() fails E, F lives x { fail A; }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.failures[0].variants.len(), 2);
        assert_eq!(p.failures[0].variants[0].exit.map(|e| e.0), Some(2));
        assert_eq!(p.funcs[0].fails.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["E", "F"]);
        let (_, d) = parse_src("failure E { A } none f() { }");
        assert_eq!(errors(&d), ["a failure variant needs a message"]);
        let (_, d) = parse_src("none f() { g() maybe 1 + 2; }");
        assert_eq!(errors(&d), ["the fallback of `maybe` is one value"]);
        let (_, d) = parse_src("none f() { uint32 a = g() maybe (1 + 2); uint32 b = g() fails; (g() maybe 1) + 2; }");
        assert!(d.is_empty(), "{:?}", errors(&d));
    }

    #[test]
    fn struct_clauses_any_order() {
        for src in ["struct S stays lives a { &string a; }", "struct S lives a stays { &string a; }"] {
            let (p, d) = parse_src(src);
            assert!(d.is_empty(), "{:?}", errors(&d));
            assert!(p.structs[0].stays.is_some());
            assert_eq!(p.structs[0].lives[0].0, "a");
        }
        for src in ["struct S with A and B stays { }", "struct S stays with A and B lives a { &string a; }", "struct S defaults D with A { }"] {
            let (p, d) = parse_src(src);
            assert!(d.is_empty(), "{src}: {:?}", errors(&d));
            assert!(!p.structs[0].with.is_empty());
        }
        let (p, _) = parse_src("struct S with A and B { }");
        assert_eq!(p.structs[0].with.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    }

    #[test]
    fn interface_items() {
        let (p, d) = parse_src(
            "interface Shape extends Named and Solid { float64 area(); static Self unit(); bool <(Self o); none bump(uint32 n) changes self; float64 x; }\n\
             uint32 with Zero { static uint32 zero() { return 0; } }\n\
             none f() { uint32 changes = 3; changes += 1; }",
        );
        assert!(d.is_empty(), "{:?}", errors(&d));
        let i = &p.interfaces[0];
        assert_eq!(i.extends.len(), 2);
        assert_eq!(i.methods.len(), 4);
        assert_eq!(i.fields.len(), 1);
        assert!(i.methods[1].is_static.is_some() && i.methods[1].ret.name == "Self");
        assert!(i.methods[2].operator.is_some());
        assert!(i.methods[3].changes_self.is_some() && i.methods[0].changes_self.is_none());
        assert_eq!(p.impls[0].ty.name, "uint32");
        assert_eq!(p.impls[0].with[0].name, "Zero");
        assert_eq!(p.impls[0].methods.len(), 1);
    }

    #[test]
    fn changes_self_only_in_interfaces() {
        let (_, d) = parse_src("struct S { none f() changes self { } }");
        assert_eq!(errors(&d), ["`changes self` is only written in interfaces"]);
    }

    #[test]
    fn borrow_of_borrow_is_unsupported() {
        for src in ["none f(&&string x) { }", "none f(&*uint32 x) { }", "none f() { &string r = &&x; }"] {
            let (_, d) = parse_src(src);
            assert!(errors(&d).iter().any(|m| m == "borrowing a borrow is not supported yet"), "{src}: {:?}", errors(&d));
        }
    }

    #[test]
    fn destroy_statement() {
        let (s, d) = body("destroy x;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::Destroy { .. }));
    }

    #[test]
    fn block_end() {
        let (p, d) = parse_src("none main() {\n}");
        assert!(d.is_empty());
        assert_eq!(p.funcs[0].body.end.start, 14);
    }

    /// A unit expression, fully parenthesised: `((kg*m)/s^2)`.
    fn show(e: &UnitExpr) -> String {
        match e {
            UnitExpr::Name(n, _) => n.clone(),
            UnitExpr::Explicit(n, _) => format!("{n} as unit"),
            UnitExpr::One(_) => "1".into(),
            UnitExpr::Mul(a, b) => format!("({}*{})", show(a), show(b)),
            UnitExpr::Div(a, b) => format!("({}/{})", show(a), show(b)),
            UnitExpr::Pow(a, n, _) => format!("{}^{n}", show(a)),
        }
    }

    fn unit_str(u: &UnitSpec) -> String {
        format!("{}{}", if u.delta { "delta " } else { "" }, show(&u.expr))
    }

    fn type_unit(t: &TypeName) -> String {
        t.unit.as_ref().map_or("-".into(), unit_str)
    }

    fn lit_unit(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Num(n) => n.unit.as_ref().map_or("-".into(), unit_str),
            other => panic!("not a number: {other:?}"),
        }
    }

    #[test]
    fn unit_declarations() {
        let src = "unit m; unit N = kg*m/s^2; unit Hz = 1 / s; unit km = 1000 m; unit degC = K offset -273.15; \
                   unit pct = 0.01; unit g = 0.001 kg; unit Pa = (N)/m^2; unit ft = 0.3048 m;";
        let (p, d) = parse_src(src);
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.units.len(), 9);
        assert_eq!(p.units[0].name, "m");
        assert!(p.units[0].def.is_none() && p.units[0].offset.is_none());
        let def = |i: usize| p.units[i].def.as_ref().unwrap();
        assert_eq!(show(def(1).expr.as_ref().unwrap()), "((kg*m)/s^2)");
        assert!(def(1).factor.is_none());
        assert_eq!(show(def(2).expr.as_ref().unwrap()), "(1/s)");
        assert!(matches!(&def(2).factor, Some((Const::Int(v), _)) if *v == num_bigint::BigInt::from(1)));
        assert!(matches!(&def(3).factor, Some((Const::Int(v), _)) if *v == num_bigint::BigInt::from(1000)));
        assert_eq!(show(def(3).expr.as_ref().unwrap()), "m");
        assert_eq!(show(def(4).expr.as_ref().unwrap()), "K");
        assert!(matches!(&p.units[4].offset, Some((Const::Dec(v), _)) if num_traits::Signed::is_negative(v)));
        assert!(def(5).expr.is_none() && matches!(&def(5).factor, Some((Const::Dec(_), _))));
        assert_eq!(show(def(7).expr.as_ref().unwrap()), "(N/m^2)");
        assert_eq!(show(def(8).expr.as_ref().unwrap()), "m");
    }

    #[test]
    fn unit_declaration_errors() {
        for src in ["unit x = ;", "unit x = 2 * m;", "unit x = m offset;", "unit 3 = m;", "unit x = 2u8 m;", "unit m offset 3;"] {
            let (_, d) = parse_src(src);
            assert!(!d.is_empty(), "{src} should not parse");
        }
    }

    #[test]
    fn unit_is_a_context_word() {
        let (p, d) = parse_src("struct Circle { static Circle unit() { } } unit foo() { } unit m;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.structs[0].methods[0].func.name, "unit");
        assert_eq!(p.funcs[0].name, "foo");
        assert_eq!(p.funcs[0].ret.name, "unit");
        assert_eq!(p.units.len(), 1);
        let (s, d) = body("uint32 unit = 3; unit = 4;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::Let { name, .. } if name == "unit"));
        assert!(matches!(&s[1], Stmt::Assign { .. }));
    }

    #[test]
    fn unit_types() {
        let (s, d) = body("float64<m/s> v = 3.0<m/s>; mut float64<m> dd; open uint8<kg> w; float64<delta degC> t; \
                           float64<kg*m/s^2> f; float64<1/s> r; uint8<m^2> q; float64<m / s> g;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let Stmt::Let { ty, mutable, .. } = &s[0] else { panic!() };
        assert_eq!(type_unit(ty), "(m/s)");
        let Stmt::Let { value: Some(Init::Expr(e)), .. } = &s[0] else { panic!() };
        assert_eq!(lit_unit(e), "(m/s)");
        assert!(!mutable);
        let Stmt::Let { ty, mutable, .. } = &s[1] else { panic!() };
        assert!(*mutable && type_unit(ty) == "m");
        let Stmt::Let { ty, open, .. } = &s[2] else { panic!() };
        assert!(*open && type_unit(ty) == "kg");
        let units: Vec<String> = s[3..].iter().map(|st| match st {
            Stmt::Let { ty, .. } => type_unit(ty),
            _ => panic!(),
        }).collect();
        assert_eq!(units, ["delta degC", "((kg*m)/s^2)", "(1/s)", "m^2", "(m/s)"]);
    }

    #[test]
    fn unit_literals() {
        let (s, d) = body("float64 a = 9.6<s>; uint8 b = 5u8<m>; float32 c = 2.5f32<m/s>; float64 e = 1<1/s>; \
                           float64 f = 3.0<delta degC>; float64 g = 100<kg*m/s^2>;");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let units: Vec<String> = s.iter().map(|st| match st {
            Stmt::Let { value: Some(Init::Expr(e)), .. } => lit_unit(e),
            _ => panic!(),
        }).collect();
        assert_eq!(units, ["s", "m", "(m/s)", "(1/s)", "delta degC", "((kg*m)/s^2)"]);
    }

    #[test]
    fn unit_literal_ambiguity() {
        // Less-than, as before the unit rule.
        assert_eq!(lexed("3<4"), ["3", "<", "4"]);
        assert_eq!(lexed("3 < m"), ["3", "<", "m"]);
        assert_eq!(lexed("3<m"), ["3", "<", "m"]);
        assert_eq!(lexed("3<m*>"), ["3", "<", "m", "*", ">"]);
        assert_eq!(lexed("3<>"), ["3", "<", ">"]);
        assert_eq!(lexed("3<x && y>2"), ["3", "<", "x", "&&", "y", ">", "2"]);
        assert_eq!(lexed("if (n<limit)"), ["if", "(", "n", "<", "limit", ")"]);
        // A literal, then the rest.
        assert_eq!(lexed("3<m>>2"), ["3<m>", ">", "2"]);
        assert_eq!(lexed("9<s>>3"), ["9<s>", ">", "3"]);
        assert_eq!(lexed("1<m>>=2"), ["1<m>", ">=", "2"]);
    }

    /// Tokens of a source text, unit literals shown as `3<m>`.
    #[test]
    fn fill_and_def_are_context_words() {
        // `fill` is a statement only before a name and a block; elsewhere it is a name.
        let (s, d) = body("uint32 fill = 3; fill(3); fill = 4; fill p { p.x = 1; }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::Let { name, .. } if name == "fill"));
        assert!(matches!(&s[1], Stmt::Expr(_)));
        assert!(matches!(&s[2], Stmt::Assign { .. }));
        assert!(matches!(&s[3], Stmt::Fill { partial: false, .. }));
        let (_, d) = body("fill partial p with defaults { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
    }

    #[test]
    fn new_needs_of_and_parentheses() {
        let (_, d) = body("Circle c = new Circle (5);");
        assert_eq!(errors(&d), ["expected `of`, found `(`"]);
    }

    fn lexed(src: &str) -> Vec<String> {
        let mut diags = Vec::new();
        let toks = crate::lexer::lex(src, &mut diags);
        toks.iter()
            .filter(|t| t.tok != Tok::Eof)
            .map(|t| match &t.tok {
                Tok::Num(n, unit) => {
                    let text = match &n.value {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) | Const::NegZero => "dec".into(),
                    };
                    unit.as_ref().map_or(text.clone(), |(u, _)| format!("{text}<{u}>"))
                }
                Tok::Ident(s) => s.clone(),
                Tok::Sym(s) | Tok::Kw(s) => s.to_string(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn unit_method_and_nested_close() {
        let (s, d) = body("x = d::in<km/h>(); y = d::in<delta degC>(); z = d::to<float64<km>>();");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let calls: Vec<(String, usize, String)> = s
            .iter()
            .map(|st| {
                let Stmt::Assign { value, .. } = st else { panic!() };
                let ExprKind::Method { name, targs, unit, .. } = &value.kind else { panic!() };
                (name.clone(), targs.len(), unit.as_ref().map_or("-".into(), unit_str))
            })
            .collect();
        assert_eq!(calls, [("in".into(), 0, "(km/h)".into()), ("in".into(), 0, "delta degC".into()), ("to".into(), 1, "-".into())]);
        let Stmt::Assign { value, .. } = &s[2] else { panic!() };
        let ExprKind::Method { targs, .. } = &value.kind else { panic!() };
        assert_eq!(type_unit(&targs[0]), "km");
    }

    #[test]
    fn unit_syntax_errors() {
        for src in ["none f(float64<> x) { }", "none f(float64<m*> x) { }", "none f(float64<m^x> x) { }", "none f(float64<m^99999999999> x) { }"] {
            let (_, d) = parse_src(src);
            assert!(!d.is_empty(), "{src} should not parse");
        }
        let (_, d) = parse_src("unit \u{b5}m = 0.000001 m;");
        assert!(!d.is_empty());
    }

    #[test]
    fn for_each_with_unit_type() {
        let (s, d) = body("for (float64<m> x of xs) { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert!(matches!(&s[0], Stmt::ForEach { ty, .. } if type_unit(ty) == "m"));
    }

    fn import_of(src: &str) -> Import {
        let (mut p, d) = parse_src(src);
        assert!(d.is_empty(), "{src}: {:?}", errors(&d));
        assert_eq!(p.imports.len(), 1);
        p.imports.remove(0)
    }

    #[test]
    fn import_module_forms() {
        let i = import_of("in io;");
        assert_eq!((i.plain_name(), i.binding()), (Some("io"), Some("io")));
        let dotted = import_of("in game.physics;");
        let words = import_of("in physics inside game;");
        assert_eq!(dotted.path, ["game", "physics"]);
        assert_eq!(words.path, dotted.path);
        assert!(words.word_form && !dotted.word_form);
        assert_eq!(import_of("in audio inside game.sound;").path, ["game", "sound", "audio"]);
        let i = import_of("in game.physics as phys;");
        assert_eq!((i.binding(), i.plain_name()), (Some("phys"), None));
        assert_eq!(import_of("in physics inside game as phys;").binding(), Some("phys"));
        assert_eq!(import_of("in game.physics;").binding(), Some("physics"));
    }

    #[test]
    fn import_item_forms() {
        let i = import_of("in Vector3 as V3, gravity from physics;");
        assert_eq!(i.path, ["physics"]);
        let names: Vec<_> = i.items.iter().map(|x| (x.name.as_str(), x.alias.as_deref())).collect();
        assert_eq!(names, [("Vector3", Some("V3")), ("gravity", None)]);
        assert_eq!(i.binding(), None);
        assert_eq!(import_of("in Vector3 from game.physics;").path, ["game", "physics"]);
        assert_eq!(import_of("in Vector3 from physics inside game;").path, ["game", "physics"]);
        let g = import_of("in * from game.audio;");
        assert!(g.glob && g.alias.is_none() && g.binding().is_none());
        assert_eq!(import_of("in * from game.audio as audio;").binding(), Some("audio"));
    }

    #[test]
    fn import_syntax_errors() {
        for src in [
            "in;",
            "in game.;",
            "in a, b;",
            "in game.Vector3 from physics;",
            "in Vector3 from physics as p;",
            "in * game.audio;",
            "in physics inside;",
            "in Vector3 as from physics;",
        ] {
            let (_, d) = parse_src(src);
            assert!(!d.is_empty(), "{src} should not parse");
        }
    }

    #[test]
    fn export_forms() {
        let (p, d) = parse_src("out Vector3 as Vec3, gravity;\nout audio;\nout struct S { }\nout none f() { }\nout enum E { A }\nout interface I { }\nout failure F { A \"x\" }\nout unit furlong = 201.168 m;\nstruct T { }\n");
        assert!(d.is_empty(), "{:?}", errors(&d));
        let shown: Vec<_> = p.exports.iter().map(|e| (e.name.as_str(), e.shown_as(), e.on_decl)).collect();
        assert_eq!(
            shown,
            [
                ("Vector3", "Vec3", false),
                ("gravity", "gravity", false),
                ("audio", "audio", false),
                ("S", "S", true),
                ("f", "f", true),
                ("E", "E", true),
                ("I", "I", true),
                ("F", "F", true),
                ("furlong", "furlong", true),
            ]
        );
        assert_eq!(p.structs.len(), 2);
    }

    #[test]
    fn export_errors_and_params_still_work() {
        for src in ["out;", "out A as;", "out in io;", "out Foo with Bar { }", "out A B;"] {
            let (_, d) = parse_src(src);
            assert!(!d.is_empty(), "{src} should not parse");
        }
        // a failed function still resyncs.
        let (p, d) = parse_src("none f(uint32 x) { }\nnone g() { }");
        assert!(d.is_empty(), "{:?}", errors(&d));
        assert_eq!(p.funcs.len(), 2);
        let (p, d) = parse_src("none ??? f(uint32 x) { }\nnone g() { }");
        assert!(!d.is_empty());
        assert!(p.funcs.iter().any(|f| f.name == "g"));
    }
}
