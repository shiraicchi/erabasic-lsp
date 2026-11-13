//! LSP-owned AST: a simplified, fully-owned version of the parser AST for LSP analysis.
//!
//! This mirrors the parser's AST but owns all text data instead of borrowing.
//! Used for LSP features like completion, hover, goto-definition, etc.

use crate::fxhash::{HashMap, HashSet};
use crate::util::position::SpanLike;
use std::sync::Arc as Rc;

/// A shared, immutable name (reference-counted string).
pub type Name = Rc<str>;

/// Text span for LSP position tracking (byte offset and length).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

impl SpanLike for Span {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
}

/// Variable reference with index chain: `CFLAG:TARGET:好感度`
#[derive(Debug, Clone)]
pub struct VarRef {
    pub name: Name,
    pub idx: Vec<Expr>,
    pub span: Span,
}

impl VarRef {
    pub fn new(name: Name, idx: Vec<Expr>) -> Self {
        Self {
            name,
            idx,
            span: Span::empty(),
        }
    }
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
    Nand,
    Nor,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub enum FormPart {
    Lit(String),
    Str(Expr, Option<Box<Expr>>, Align),
    Int(Expr, Option<Box<Expr>>, Align),
    Cond(Box<(Expr, Form, Form)>),
}

/// FORM string with interpolated holes.
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub parts: Vec<FormPart>,
    pub span: Span,
}

impl Form {
    pub fn lit(s: &str) -> Self {
        Self {
            parts: vec![FormPart::Lit(s.to_string())],
            span: Span::empty(),
        }
    }
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }
}

/// Resolved call target: 0=unknown, 1=user function, 2=built-in.
#[derive(Debug, Default)]
pub struct Resolved(std::sync::atomic::AtomicU8);

impl Resolved {
    pub fn new() -> Self {
        Self(std::sync::atomic::AtomicU8::new(0))
    }
    pub fn get(&self) -> u8 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn set(&self, k: u8) {
        self.0.store(k, std::sync::atomic::Ordering::Relaxed)
    }
}

impl Clone for Resolved {
    fn clone(&self) -> Self {
        Self(std::sync::atomic::AtomicU8::new(
            self.0.load(std::sync::atomic::Ordering::Relaxed),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Str(String),
    Form(Rc<Form>),
    Var(Rc<VarRef>),
    Call(Box<(String, Vec<Expr>, Resolved)>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    IncDec(Rc<VarRef>, bool, i64),
    Empty,
    Span(Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Span(s) => *s,
            Expr::Form(f) => f.span,
            Expr::Var(v) => v.span,
            Expr::Call(_) => Span::empty(),
            Expr::Unary(_, e) => e.span(),
            Expr::IncDec(_, _, _) => Span::empty(),
            Expr::Empty => Span::empty(),
            Expr::Binary(_, a, b) => Span::new(a.span().start, b.span().end),
            Expr::Ternary(c, a, b) => Span::new(c.span().start, b.span().end),
            Expr::Int(_) | Expr::Str(_) => Span::empty(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AsgOp {
    Set,
    SetExpr,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PrintFlags {
    pub newline: bool,
    pub wait: bool,
    pub default_color: bool,
    pub column: Option<Align>,
    pub single: bool,
    pub plain: bool,
    pub no_auto_button: bool,
}

#[derive(Debug, Clone)]
pub enum PrintContent {
    Raw(String),
    Form(Form),
    Expr(Expr),
    Exprs(Vec<Expr>),
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    Static(String),
    Form(Form),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CallKind {
    Call,
    Jump,
    CallF,
}

#[derive(Debug, Clone)]
pub enum CaseCond {
    Eq(Expr),
    Range(Expr, Expr),
    Is(BinOp, Expr),
}

#[derive(Debug, Clone)]
pub enum Args {
    None,
    Exprs(Vec<Expr>),
    Form(Form),
    Raw(String),
}

#[derive(Debug, Clone, Default)]
pub struct CaseTable {
    pub strs: HashMap<String, usize>,
    pub ints: HashMap<i64, usize>,
    pub default: usize,
}

#[derive(Debug, Clone)]
pub enum Instr {
    BadLine(Rc<str>, bool),
    Print(PrintContent, PrintFlags),
    PrintData(Vec<Vec<Form>>, PrintFlags),
    StrData(Option<Rc<VarRef>>, Vec<Vec<Form>>),
    Assign(Rc<VarRef>, AsgOp, Vec<Expr>),
    AssignForm(Rc<VarRef>, AsgOp, Form),
    Eval(Expr),
    Jump(usize),
    JumpIfNot(Expr, usize),
    ForBegin(Box<ForBegin>),
    ForNext {
        var: Rc<VarRef>,
        slot: usize,
        body: usize,
    },
    ForBreak {
        var: Rc<VarRef>,
        slot: usize,
        exit: usize,
    },
    SelectEval(Expr, usize, Box<std::sync::OnceLock<Option<CaseTable>>>),
    CaseTest(usize, Vec<CaseCond>, usize),
    Call(Box<CallInstr>),
    CallList(Vec<(CallTarget, Vec<Expr>)>, CallKind),
    Goto(Box<(CallTarget, bool, Option<usize>)>),
    GotoList(Vec<CallTarget>),
    Return(Vec<Expr>),
    ReturnF(Option<Expr>),
    End,
    Begin(String),
    Restart,
    Quit,
    Throw(Form),
    Command(Box<(String, Args)>),
}

#[derive(Debug, Clone)]
pub struct ForBegin {
    pub var: Rc<VarRef>,
    pub start: Expr,
    pub end: Expr,
    pub step: Option<Expr>,
    pub slot: usize,
    pub exit: usize,
}

#[derive(Debug, Clone)]
pub struct CallInstr {
    pub target: CallTarget,
    pub args: Vec<Expr>,
    pub kind: CallKind,
    pub try_: bool,
    pub catch: Option<usize>,
}

/// #DIM / #DIMS declaration.
#[derive(Debug, Clone)]
pub struct DimDecl {
    pub name: String,
    pub decl_name: String,
    pub is_str: bool,
    pub dims: Vec<Expr>,
    pub init: Vec<Expr>,
    pub is_const: bool,
    pub is_static: bool,
    pub is_global: bool,
    pub is_savedata: bool,
    pub is_chara: bool,
    pub is_ref: bool,
    pub is_dynamic: bool,
    pub span: Span,
}

/// Function parameter.
#[derive(Debug, Clone)]
pub struct Param {
    pub target: Rc<VarRef>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventOrder {
    Pri,
    Normal,
    Later,
}

/// A function's compiled body (eagerly compiled for LSP).
#[derive(Debug, Default, Clone)]
pub struct Body {
    pub code: Vec<Instr>,
    pub lines: Vec<Span>,
    pub labels: HashMap<String, usize>,
    pub n_slots: usize,
    pub dims: Vec<DimDecl>,
}

/// A function with eagerly compiled body.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Body,
    pub is_function: bool,
    pub returns_str: bool,
    pub order: EventOrder,
    pub single: bool,
    pub only: bool,
    pub kojo: bool,
    pub local_size: u32,
    pub locals_size: u32,
    pub file: u32,
    pub line: u32,
    pub span: Span,
}

/// The complete parsed program (owned, no lifetimes).
#[derive(Debug, Default, Clone)]
pub struct Program {
    pub files: Vec<String>,
    pub functions: HashMap<String, Vec<Function>>,
    pub global_dims: Vec<DimDecl>,
    pub defines: HashMap<String, String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Source file for LSP parsing.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    pub text: String,
}
