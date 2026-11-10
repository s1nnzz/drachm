use crate::ast::*;
use crate::error::{Error, Span};
use crate::lexer::{Tok, Token};

pub fn parse(tokens: Vec<Token>) -> Result<Program, Error> {
    let mut p = Parser { tokens, pos: 0 };
    let mut funcs = Vec::new();
    while p.peek() != &Tok::Eof {
        funcs.push(p.func()?);
    }
    Ok(Program { funcs })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
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

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<Span, Error> {
        if self.peek() == &tok {
            Ok(self.bump().span)
        } else {
            Err(Error::new(self.span(), format!("expected {what}, found {}", describe(self.peek()))))
        }
    }

    fn ident(&mut self) -> Result<(String, Span), Error> {
        match self.peek().clone() {
            Tok::Ident(name) => Ok((name, self.bump().span)),
            t => Err(Error::new(self.span(), format!("expected identifier, found {}", describe(&t)))),
        }
    }

    fn ty(&mut self) -> Result<Type, Error> {
        if self.eat(&Tok::LParen) {
            self.expect(Tok::RParen, "')'")?;
            return Ok(Type::Unit);
        }
        let (name, span) = self.ident()?;
        match name.as_str() {
            "i64" => Ok(Type::I64),
            "bool" => Ok(Type::Bool),
            _ => Err(Error::new(span, format!("unknown type '{name}'"))),
        }
    }

    fn func(&mut self) -> Result<Func, Error> {
        let start = self.expect(Tok::Fn, "'fn'")?;
        let (name, _) = self.ident()?;
        self.expect(Tok::LParen, "'('")?;
        let mut params = Vec::new();
        while self.peek() != &Tok::RParen {
            let (pname, _) = self.ident()?;
            self.expect(Tok::Colon, "':'")?;
            params.push((pname, self.ty()?));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "')'")?;
        let ret = if self.eat(&Tok::Arrow) { self.ty()? } else { Type::Unit };
        let span = Span::new(start.start, self.prev_span().end);
        let body = self.block()?;
        Ok(Func { name, params, ret, body, span })
    }

    fn block(&mut self) -> Result<Block, Error> {
        self.expect(Tok::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while self.peek() != &Tok::RBrace {
            if self.peek() == &Tok::Eof {
                return Err(Error::new(self.span(), "unclosed block, expected '}'"));
            }
            stmts.push(self.stmt()?);
        }
        self.bump();
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        let start = self.span();
        match self.peek() {
            Tok::Let | Tok::Var => {
                let mutable = self.bump().tok == Tok::Var;
                let (name, _) = self.ident()?;
                self.expect(Tok::Colon, "':' and a type (Drachm requires explicit types)")?;
                let ty = self.ty()?;
                self.expect(Tok::Assign, "'='")?;
                let value = self.expr()?;
                self.expect(Tok::Semi, "';'")?;
                Ok(Stmt::Let { name, ty, mutable, value, span: Span::new(start.start, self.prev_span().end) })
            }
            Tok::If => self.if_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::Return => {
                self.bump();
                let value = if self.peek() == &Tok::Semi { None } else { Some(self.expr()?) };
                self.expect(Tok::Semi, "';'")?;
                Ok(Stmt::Return { value, span: Span::new(start.start, self.prev_span().end) })
            }
            _ => {
                let e = self.expr()?;
                if self.eat(&Tok::Assign) {
                    let ExprKind::Var(name) = e.kind else {
                        return Err(Error::new(e.span, "left side of '=' must be a variable"));
                    };
                    let value = self.expr()?;
                    self.expect(Tok::Semi, "';'")?;
                    return Ok(Stmt::Assign { name, value, span: Span::new(start.start, self.prev_span().end) });
                }
                self.expect(Tok::Semi, "';'")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, Error> {
        self.expect(Tok::If, "'if'")?;
        let cond = self.expr()?;
        let then = self.block()?;
        let els = if self.eat(&Tok::Else) {
            if self.peek() == &Tok::If {
                Some(vec![self.if_stmt()?])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn expr(&mut self) -> Result<Expr, Error> {
        self.binary(0)
    }

    /// Precedence climbing. Higher number binds tighter.
    fn binary(&mut self, min_prec: u8) -> Result<Expr, Error> {
        let mut lhs = self.unary()?;
        loop {
            let Some((op, prec)) = binop(self.peek()) else { break };
            if prec < min_prec {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let span = Span::new(lhs.span.start, rhs.span.end);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        let start = self.span();
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Bang => UnOp::Not,
            _ => return self.primary(),
        };
        self.bump();
        let e = self.unary()?;
        let span = Span::new(start.start, e.span.end);
        Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), span })
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        let t = self.bump();
        let kind = match t.tok {
            Tok::Int(n) => ExprKind::Int(n),
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(Tok::RParen, "')'")?;
                return Ok(e);
            }
            Tok::Ident(name) => {
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != &Tok::RParen {
                        args.push(self.expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen, "')'")?;
                    ExprKind::Call(name, args)
                } else {
                    ExprKind::Var(name)
                }
            }
            other => return Err(Error::new(t.span, format!("expected expression, found {}", describe(&other)))),
        };
        Ok(Expr { kind, span: Span::new(t.span.start, self.prev_span().end) })
    }
}

fn binop(t: &Tok) -> Option<(BinOp, u8)> {
    Some(match t {
        Tok::OrOr => (BinOp::Or, 1),
        Tok::AndAnd => (BinOp::And, 2),
        Tok::EqEq => (BinOp::Eq, 3),
        Tok::NotEq => (BinOp::Ne, 3),
        Tok::Lt => (BinOp::Lt, 4),
        Tok::Le => (BinOp::Le, 4),
        Tok::Gt => (BinOp::Gt, 4),
        Tok::Ge => (BinOp::Ge, 4),
        Tok::Plus => (BinOp::Add, 5),
        Tok::Minus => (BinOp::Sub, 5),
        Tok::Star => (BinOp::Mul, 6),
        Tok::Slash => (BinOp::Div, 6),
        Tok::Percent => (BinOp::Rem, 6),
        _ => return None,
    })
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Int(n) => format!("integer {n}"),
        Tok::Ident(s) => format!("'{s}'"),
        Tok::Eof => "end of file".into(),
        other => format!("{other:?}"),
    }
}
