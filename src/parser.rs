use crate::ast::*;
use crate::diag::{codes, Diagnostic, Span};
use crate::lexer::{Tok, Token};

/// Never fails: syntax errors are reported to `diags` and the parser carries
/// on at the next statement or function, so several can be found in one go.
pub fn parse(tokens: Vec<Token>, diags: &mut Vec<Diagnostic>) -> Program {
    let mut p = Parser { tokens, pos: 0, diags };
    let mut program = Program { imports: Vec::new(), funcs: Vec::new(), structs: Vec::new() };
    while p.peek() != &Tok::Eof {
        let start = p.pos;
        let item = if p.peek() == &Tok::Kw("in") {
            p.import().map(|i| program.imports.push(i))
        } else if p.peek() == &Tok::Kw("struct") {
            p.struct_decl().map(|s| program.structs.push(s))
        } else {
            p.func().map(|f| program.funcs.push(f))
        };
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
    program
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    diags: &'a mut Vec<Diagnostic>,
}

type Parsed<T> = Result<T, Diagnostic>;

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

    /// A type: a name, or the keyword `none`.
    fn ty(&mut self) -> Parsed<TypeName> {
        if self.peek() == &Tok::Kw("none") {
            return Ok(TypeName { name: "none".into(), span: self.bump().span });
        }
        let (name, span) = self.ident("a type")?;
        Ok(TypeName { name, span })
    }

    /// Does a top-level item (a function, struct or import) start here?
    fn at_item_start(&self) -> bool {
        match self.peek() {
            Tok::Kw("in" | "struct") => true,
            Tok::Ident(_) | Tok::Kw("none") => {
                matches!(self.peek_at(1), Tok::Ident(_)) && self.peek_at(2) == &Tok::Sym("(")
            }
            _ => false,
        }
    }

    fn import(&mut self) -> Parsed<Import> {
        let start = self.bump().span;
        let (module, _) = self.ident("a module name")?;
        self.expect(";")?;
        Ok(Import { module, span: start.to(self.prev_span()) })
    }

    fn func(&mut self) -> Parsed<Func> {
        if !matches!(self.peek(), Tok::Ident(_) | Tok::Kw("none")) {
            return Err(self
                .unexpected("a function, `struct` or `in`")
                .help("a function starts with its return type: `none main() { }`"));
        }
        let ret = self.ty()?;
        let (name, name_span) = self.ident("a function name")?;
        self.func_rest(ret, name, name_span)
    }

    /// A function's parameters and body, after its return type and name.
    fn func_rest(&mut self, ret: TypeName, name: String, name_span: Span) -> Parsed<Func> {
        self.expect("(")?;
        let mut params = Vec::new();
        while self.peek() != &Tok::Sym(")") {
            let start = self.span();
            let mutable = self.peek() == &Tok::Kw("mut");
            if mutable {
                self.bump();
            }
            let ty = self.ty()?;
            let (pname, _) = self.ident("a parameter name")?;
            params.push(Param { mutable, ty, name: pname, span: start.to(self.prev_span()) });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let span = ret.span.to(self.prev_span());
        let body = self.block()?;
        Ok(Func { ret, name, name_span, params, body, span })
    }

    fn struct_decl(&mut self) -> Parsed<Struct> {
        self.bump();
        let (name, name_span) = self.ident("a struct name")?;
        match self.peek() {
            Tok::Sym("<") => {
                return Err(Diagnostic::at(&codes::E0901, self.span(), "generic structs are not supported yet"));
            }
            Tok::Kw("with") => {
                return Err(Diagnostic::at(&codes::E0901, self.span(), "interfaces are not supported yet"));
            }
            _ => {}
        }
        let open = self.expect("{")?;
        let mut s = Struct { name, name_span, fields: Vec::new(), methods: Vec::new() };
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
        Ok(s)
    }

    /// One field or method of struct `s`.
    fn member(&mut self, s: &mut Struct, section: Option<bool>) -> Parsed<()> {
        let is_static = if self.peek() == &Tok::Kw("static") { Some(self.bump().span) } else { None };
        let mutable = if self.peek() == &Tok::Kw("mut") { Some(self.bump().span) } else { None };
        if matches!(self.peek(), Tok::Ident(n) if *n == s.name) && self.peek_at(1) == &Tok::Sym("(") {
            return Err(Diagnostic::at(&codes::E0901, self.span(), "constructors are not supported yet")
                .help(format!("build values with the declaration shorthand for now: `{} p = 1, 2;`", s.name)));
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
            let func = self.func_rest(ty, name, name_span)?;
            s.methods.push(Method { public: section.unwrap_or(true), is_static, operator, func });
            return Ok(());
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
        self.bump();
        Ok(stmts)
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

    fn stmt(&mut self) -> Parsed<Stmt> {
        let start = self.span();
        match self.peek() {
            Tok::Kw("mut") => {
                self.bump();
                self.binding(start, true)
            }
            // `Type name ...` is a binding; a lone name starts an expression.
            // (A reserved word in the name's place gets the binding's error message.)
            Tok::Ident(_) | Tok::Kw("none") if matches!(self.peek_at(1), Tok::Ident(_) | Tok::Kw(_)) => {
                self.binding(start, false)
            }
            Tok::Kw("if") => self.if_stmt(),
            Tok::Kw("while") => {
                self.bump();
                let cond = self.condition("while")?;
                let body = self.block()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::Kw("return") => {
                self.bump();
                let value = if self.peek() == &Tok::Sym(";") { None } else { Some(self.expr()?) };
                self.expect(";")?;
                Ok(Stmt::Return { value, span: start.to(self.prev_span()) })
            }
            _ => {
                let e = self.expr()?;
                let op = match self.peek() {
                    Tok::Sym("=") => Some(None),
                    Tok::Sym(s) => compound_op(s).map(Some),
                    _ => None,
                };
                if let Some(op) = op {
                    if !is_place(&e) {
                        return Err(Diagnostic::at(&codes::E0005, e.span, "only a binding or a field can be assigned to")
                            .note("cannot assign to this"));
                    }
                    self.bump();
                    let value = self.expr()?;
                    if self.peek() == &Tok::Sym(",") {
                        return Err(self
                            .unexpected("`;`")
                            .help("building a struct from a list of values works only in a declaration: `Point p = 1, 2;`"));
                    }
                    self.expect(";")?;
                    return Ok(Stmt::Assign { target: e, op, value, span: start.to(self.prev_span()) });
                }
                self.expect(";")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// `Type name;` or `Type name = value;`, after any `mut`.
    fn binding(&mut self, start: Span, mutable: bool) -> Parsed<Stmt> {
        let ty = self.ty()?;
        let (name, name_span) = self.ident("a name for the binding")?;
        let value = if self.eat("=") { Some(self.init()?) } else { None };
        if value.is_none() && self.peek() != &Tok::Sym(";") {
            return Err(self.unexpected("`=` or `;`"));
        }
        self.expect(";")?;
        Ok(Stmt::Let { mutable, ty, name, name_span, value, span: start.to(self.prev_span()) })
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
        self.bump();
        let cond = self.condition("if")?;
        let then = self.block()?;
        let els = if self.peek() == &Tok::Kw("else") {
            self.bump();
            if self.peek() == &Tok::Kw("if") { Some(vec![self.if_stmt()?]) } else { Some(self.block()?) }
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn expr(&mut self) -> Parsed<Expr> {
        self.binary(0)
    }

    /// Precedence climbing. Higher number binds tighter.
    fn binary(&mut self, min_prec: u8) -> Parsed<Expr> {
        let mut lhs = self.unary()?;
        while let Tok::Sym(sym) = self.peek() {
            let Some((op, prec)) = binop(sym) else { break };
            if prec < min_prec {
                break;
            }
            let op_span = self.bump().span;
            // `**` chains from the right; everything else from the left.
            let rhs = self.binary(if op == BinOp::Pow { prec } else { prec + 1 })?;
            self.check_grouping(op, op_span, &lhs, &rhs);
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span, paren: false };
        }
        Ok(lhs)
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
        let Tok::Sym(sym) = *self.peek() else { return self.postfix() };
        let op = match sym {
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
        let mut e = self.primary()?;
        let mut calls = 0;
        loop {
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
            if self.eat("<") {
                loop {
                    targs.push(self.ty()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
            }
            let args = self.args()?;
            let span = e.span.to(self.prev_span());
            e = Expr { kind: ExprKind::Method { recv: Box::new(e), name, name_span, targs, args }, span, paren: false };
        }
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

    fn primary(&mut self) -> Parsed<Expr> {
        let t = self.bump();
        let kind = match t.tok {
            Tok::Num(n) => ExprKind::Num(n),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Kw("self") => ExprKind::SelfValue,
            Tok::Kw("true") => ExprKind::Bool(true),
            Tok::Kw("false") => ExprKind::Bool(false),
            Tok::Sym("(") => {
                let mut e = self.expr()?;
                self.expect(")")?;
                e.span = t.span.to(self.prev_span());
                e.paren = true;
                return Ok(e);
            }
            Tok::Ident(name) => {
                if self.peek() == &Tok::Sym("(") {
                    ExprKind::Call { name, args: self.args()? }
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

/// Can `e` be assigned to: a name, `self`, or a field of one of those?
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::SelfValue => true,
        ExprKind::Field { recv, .. } => is_place(recv),
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
