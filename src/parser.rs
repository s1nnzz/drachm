use num_traits::ToPrimitive;

use crate::ast::*;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::lexer::{StrPiece, Tok, Token};

/// Never fails: syntax errors are reported to `diags` and the parser carries
/// on at the next statement or function, so several can be found in one go.
pub fn parse(tokens: Vec<Token>, diags: &mut Vec<Diagnostic>) -> Program {
    let mut p = Parser { tokens, pos: 0, diags };
    let mut program = Program { imports: Vec::new(), funcs: Vec::new(), structs: Vec::new(), unsafe_allowed: false };
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

    /// A type: `&`, `&mut` and `*` marks, then a name or the keyword `none`.
    fn ty(&mut self) -> Parsed<TypeName> {
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
                    mods.push((TypeMod::Ptr, span));
                }
                Tok::Sym("**") => {
                    self.bump();
                    mods.push((TypeMod::Ptr, span));
                    mods.push((TypeMod::Ptr, span));
                }
                _ => break,
            }
        }
        if self.peek() == &Tok::Kw("none") {
            let span = self.bump().span;
            return Ok(TypeName { name: "none".into(), span, mods });
        }
        let (name, span) = self.ident("a type")?;
        Ok(TypeName { name, span, mods })
    }

    /// Where a type written at token offset `n` ends (the offset after its
    /// name), if one is written there.
    fn type_end(&self, mut n: usize) -> Option<usize> {
        loop {
            match self.peek_at(n) {
                Tok::Sym("&" | "&&" | "*" | "**") => n += 1,
                Tok::Kw("mut") if n > 0 && matches!(self.peek_at(n - 1), Tok::Sym("&")) => n += 1,
                Tok::Ident(_) | Tok::Kw("none") => return Some(n + 1),
                _ => return None,
            }
        }
    }

    /// Does a top-level item (a function, struct or import) start here?
    fn at_item_start(&self) -> bool {
        match self.peek() {
            Tok::Kw("in" | "struct") => true,
            Tok::Ident(_) | Tok::Kw("none" | "unsafe") | Tok::Sym("&" | "&&" | "*" | "**") => {
                let from = (self.peek() == &Tok::Kw("unsafe")) as usize;
                match self.type_end(from) {
                    Some(n) => matches!(self.peek_at(n), Tok::Ident(_)) && self.peek_at(n + 1) == &Tok::Sym("("),
                    None => false,
                }
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
        let is_unsafe = if self.peek() == &Tok::Kw("unsafe") { Some(self.bump().span) } else { None };
        if !matches!(self.peek(), Tok::Ident(_) | Tok::Kw("none") | Tok::Sym("&" | "&&" | "*" | "**")) {
            return Err(self
                .unexpected("a function, `struct` or `in`")
                .help("a function starts with its return type: `none main() { }`"));
        }
        let ret = self.ty()?;
        let (name, name_span) = self.ident("a function name")?;
        self.func_rest(ret, name, name_span, is_unsafe)
    }

    /// A function's parameters and body, after its return type and name.
    fn func_rest(&mut self, ret: TypeName, name: String, name_span: Span, is_unsafe: Option<Span>) -> Parsed<Func> {
        self.expect("(")?;
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
        let span = ret.span.to(self.prev_span());
        let mut lives = Vec::new();
        let mut lives_span = None;
        loop {
            match self.peek() {
                Tok::Kw("lives") => {
                    if let Some(first) = lives_span {
                        return Err(Diagnostic::at(&codes::E0003, self.span(), "`lives` is written twice")
                            .label(first, "first written here"));
                    }
                    let at = self.bump().span;
                    lives = self.name_list("a parameter name after `lives`")?;
                    lives_span = Some(at.to(self.prev_span()));
                }
                Tok::Kw("fails") => {
                    return Err(Diagnostic::at(&codes::E0901, self.span(), "`fails` is not supported yet"));
                }
                _ => break,
            }
        }
        let body = self.block()?;
        Ok(Func { ret, name, name_span, params, body, is_unsafe, lives, lives_span, span })
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
        loop {
            match self.peek() {
                Tok::Kw("mut") => m = true,
                Tok::Kw("open") => o = true,
                _ => return (m, o),
            }
            self.bump();
        }
    }

    fn struct_decl(&mut self) -> Parsed<Struct> {
        self.bump();
        let (name, name_span) = self.ident("a struct name")?;
        let mut lives: Vec<(String, Span)> = Vec::new();
        let mut lives_seen = false;
        let mut stays = None;
        loop {
            match self.peek() {
                Tok::Sym("<") => {
                    return Err(Diagnostic::at(&codes::E0901, self.span(), "generic structs are not supported yet"));
                }
                Tok::Kw("with") => {
                    return Err(Diagnostic::at(&codes::E0901, self.span(), "interfaces are not supported yet"));
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
        let mut s = Struct { name, name_span, lives, stays, fields: Vec::new(), methods: Vec::new() };
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
        let is_unsafe = if self.peek() == &Tok::Kw("unsafe") { Some(self.bump().span) } else { None };
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
            let func = self.func_rest(ty, name, name_span, is_unsafe)?;
            s.methods.push(Method { public: section.unwrap_or(true), is_static, operator, func });
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
            Tok::Ident(_) | Tok::Kw("none") => matches!(self.peek_at(1), Tok::Ident(_) | Tok::Kw(_)),
            Tok::Sym("&" | "&&") => true,
            // `*uint32 p = ..` declares; `*p = 7;` and `*p.f = 7;` assign.
            Tok::Sym("*" | "**") => {
                let mut n = 0;
                while matches!(self.peek_at(n), Tok::Sym("*" | "**")) {
                    n += 1;
                }
                matches!(self.peek_at(n), Tok::Ident(_) | Tok::Kw("none")) && matches!(self.peek_at(n + 1), Tok::Ident(_))
            }
            _ => false,
        }
    }

    fn stmt(&mut self) -> Parsed<Stmt> {
        let start = self.span();
        match self.peek() {
            _ if self.at_binding() => {
                let (mutable, open) = self.binding_words();
                self.binding(start, mutable, open)
            }
            Tok::Kw("unsafe") => {
                self.bump();
                let body = self.block()?;
                Ok(Stmt::Unsafe { body, span: start.to(self.prev_span()) })
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
        match self.peek() {
            Tok::Kw("copy") => {
                self.bump();
                let e = self.unary()?;
                return Ok(Expr { span: start.to(e.span), kind: ExprKind::Copy(Box::new(e)), paren: false });
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
                if matches!(self.peek(), Tok::Ident(w) if w == "leaving") {
                    self.bump();
                    leaving = Some(Box::new(self.binary(0)?));
                }
                let span = start.to(self.prev_span());
                return Ok(Expr { span, kind: ExprKind::Take { place: Box::new(place), leaving }, paren: false });
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
            Tok::Str(pieces) => ExprKind::Str(self.string_parts(pieces)?),
            Tok::Kw("self") => ExprKind::SelfValue,
            Tok::Kw("true") => ExprKind::Bool(true),
            Tok::Kw("false") => ExprKind::Bool(false),
            Tok::Kw("none") => ExprKind::NoneValue,
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
        let mut inner = Parser { tokens, pos: 0, diags: &mut *self.diags };
        let e = inner.expr()?;
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
        ExprKind::Field { recv, .. } => is_place(recv),
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
        assert!(matches!(e.kind, ExprKind::Copy(_)));
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
        assert_eq!(p.funcs[1].ret.mods.len(), 1);
        let (_, d) = parse_src("none f() fails { }");
        assert_eq!(errors(&d), ["`fails` is not supported yet"]);
    }

    #[test]
    fn struct_clauses_any_order() {
        for src in ["struct S stays lives a { &string a; }", "struct S lives a stays { &string a; }"] {
            let (p, d) = parse_src(src);
            assert!(d.is_empty(), "{:?}", errors(&d));
            assert!(p.structs[0].stays.is_some());
            assert_eq!(p.structs[0].lives[0].0, "a");
        }
        let (_, d) = parse_src("struct S with Foo { }");
        assert_eq!(errors(&d), ["interfaces are not supported yet"]);
    }

    #[test]
    fn borrow_of_borrow_is_unsupported() {
        for src in ["none f(&&string x) { }", "none f(&*uint32 x) { }", "none f() { &string r = &&x; }"] {
            let (_, d) = parse_src(src);
            assert!(errors(&d).iter().any(|m| m == "borrowing a borrow is not supported yet"), "{src}: {:?}", errors(&d));
        }
    }

    #[test]
    fn block_end() {
        let (p, d) = parse_src("none main() {\n}");
        assert!(d.is_empty());
        assert_eq!(p.funcs[0].body.end.start, 14);
    }
}
