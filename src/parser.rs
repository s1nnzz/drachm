use num_traits::ToPrimitive;

use crate::ast::*;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::lexer::{Tok, Token};

/// Never fails: syntax errors are reported to `diags` and the parser carries
/// on at the next statement or function, so several can be found in one go.
pub fn parse(tokens: Vec<Token>, diags: &mut Vec<Diagnostic>) -> Program {
    let mut p = Parser { tokens, pos: 0, diags };
    let mut program = Program { imports: Vec::new(), funcs: Vec::new() };
    while p.peek() != &Tok::Eof {
        let start = p.pos;
        let item = if p.peek() == &Tok::Kw("in") {
            p.import().map(|i| program.imports.push(i))
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

    /// Does a top-level item (a function or an import) start here?
    fn at_item_start(&self) -> bool {
        match self.peek() {
            Tok::Kw("in") => true,
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
                .unexpected("a function or `in`")
                .help("a function starts with its return type: `none main() { }`"));
        }
        let ret = self.ty()?;
        let (name, name_span) = self.ident("a function name")?;
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

    /// Does a binding declaration (`[mut] Type name ...`) start here? A
    /// lone name starts an expression. (A reserved word in the name's place
    /// gets the binding's error message.)
    fn at_binding(&self) -> bool {
        match self.peek() {
            Tok::Kw("mut") => true,
            Tok::Ident(_) | Tok::Kw("none") => matches!(self.peek_at(1), Tok::Ident(_) | Tok::Kw(_)),
            _ => false,
        }
    }

    fn stmt(&mut self) -> Parsed<Stmt> {
        let start = self.span();
        match self.peek() {
            _ if self.at_binding() => {
                let mutable = self.eat_kw("mut");
                self.binding(start, mutable)
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
            Tok::Num(NumLit { value: Const::Int(v), suffix: None }) => {
                let n = v.to_u64().unwrap_or(u64::MAX);
                Ok(Some((n, self.bump().span)))
            }
            Tok::Num(_) => Err(Diagnostic::at(
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
        let ExprKind::Var(name) = e.kind else {
            return Err(Diagnostic::at(&codes::E0005, e.span, "only a binding can be assigned to").note("cannot assign to this"));
        };
        self.bump();
        let value = self.expr()?;
        Ok(Stmt::Assign { name, name_span: e.span, op, value, span: start.to(self.prev_span()) })
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
        if matches!(self.peek_at(m), Tok::Ident(_) | Tok::Kw("none"))
            && matches!(self.peek_at(m + 1), Tok::Ident(_))
            && self.peek_at(m + 2) == &Tok::Kw("of")
        {
            let mutable = self.eat_kw("mut");
            let ty = self.ty()?;
            let (name, name_span) = self.ident("a name for the loop variable")?;
            self.bump();
            let iter = self.expr()?;
            self.expect(")")?;
            let span = start.to(self.prev_span());
            let body = self.block()?;
            return Ok(Stmt::ForEach { mutable, ty, name, name_span, iter, body, span });
        }

        let init = if self.eat(";") {
            None
        } else if self.at_binding() {
            let at = self.span();
            let mutable = self.eat_kw("mut");
            Some(Box::new(self.binding(at, mutable)?))
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
                return Err(Diagnostic::at(&codes::E0012, e.span, "a `for` step must be an assignment, `++`/`--` or a call")
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
                } else {
                    patterns.push(Pattern::Expr(self.expr()?));
                }
                if !self.eat(",") {
                    break;
                }
            }
            let case_span = case_start.to(self.prev_span());
            let body = self.block()?;
            cases.push(Case { patterns, body, span: case_span });
        }
        self.bump();
        Ok(Stmt::Match { value, cases, span })
    }

    /// `Type name;` or `Type name = value;`, after any `mut`.
    fn binding(&mut self, start: Span, mutable: bool) -> Parsed<Stmt> {
        let ty = self.ty()?;
        let (name, name_span) = self.ident("a name for the binding")?;
        let value = if self.eat("=") { Some(self.expr()?) } else { None };
        if value.is_none() && self.peek() != &Tok::Sym(";") {
            return Err(self.unexpected("`=` or `;`"));
        }
        self.expect(";")?;
        Ok(Stmt::Let { mutable, ty, name, name_span, value, span: start.to(self.prev_span()) })
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

    /// A primary expression followed by method calls and `++`/`--`.
    fn postfix(&mut self) -> Parsed<Expr> {
        let mut e = self.primary()?;
        let mut calls = 0;
        loop {
            let sym = match self.peek() {
                Tok::Sym(s @ ("::" | "->" | "++" | "--")) => *s,
                _ => return Ok(e),
            };
            let op_span = self.bump().span;
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
