use crate::error::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I64,
    Bool,
    Unit,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Type::I64 => "i64",
            Type::Bool => "bool",
            Type::Unit => "()",
        })
    }
}

#[derive(Debug)]
pub struct Program {
    pub funcs: Vec<Func>,
}

#[derive(Debug)]
pub struct Func {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Block,
    pub span: Span,
}

pub type Block = Vec<Stmt>;

#[derive(Debug)]
pub enum Stmt {
    Let { name: String, ty: Type, mutable: bool, value: Expr, span: Span },
    Assign { name: String, value: Expr, span: Span },
    If { cond: Expr, then: Block, els: Option<Block> },
    While { cond: Expr, body: Block },
    Return { value: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind {
    Int(i64),
    Bool(bool),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}
