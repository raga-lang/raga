//! The abstract syntax tree: the program as the user wrote it, as a tree.
//!
//! The parser builds it and every later stage reads it. It is plain data with no behavior, and it holds no resolved
//! meaning: it says that `x` is *a name*, not which variable it is, and it does not know that `1 + 2` is an `i32`.
//! Finding that out is semantic analysis, and it will not change this tree but build a new one with the meaning attached.
//!
//! "Abstract" means the tree keeps what matters and drops what does not: parentheses used only for grouping are gone
//! (the shape of the tree already says what is grouped), and so are separators and layout. Every node keeps its
//! [`Span`], so an error can still point at the text.
//!
//! This is the subset needed for `templates/01-basics.raga`: bindings, assignment, expressions and blocks.
//! Functions, `if`, loops, structs and the rest are added with the template that introduces them.

mod print;

use crate::span::Span;

/// Names one statement or expression. The parser numbers them 0, 1, 2, ... in the order it makes them, and a later
/// stage that learns something about a node (its type, the declaration a name refers to) keeps it in a table indexed by
/// this number, so the tree itself never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

/// A whole file: its top-level statements, run from the first to the last.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    /// How many ids were given out, so a table can be made with one entry per node.
    pub node_count: u32,
}

/// A name, with where it was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

// ---- statements

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub id: NodeId,
    pub kind: StmtKind,
    pub span: Span,
    /// The attributes on the lines above: `[method(Ownership.Static)]`. They describe the binding that follows.
    pub attrs: Vec<Attribute>,
}

/// One attribute: a name and its arguments, `class(Inheritance.Final)`. A line like `[a(1), b]` holds two of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: Ident,
    pub args: Vec<Arg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `const x = 1`, `let y = 2`, `u8 z = 3`
    Decl(Decl),
    /// `x = 1`, `x += 2`. Assignment is a statement and gives no value.
    Assign(Assign),
    /// An expression whose result is dropped: `print(x)`, `1 + 2`.
    Expr(Expr),
    /// `{ ... }`: a scope.
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub kind: DeclKind,
    pub name: Ident,
    pub init: Expr,
}

/// How a name is bound. A declaration is exactly one of the three, never a mix.
#[derive(Debug, Clone, PartialEq)]
pub enum DeclKind {
    /// `const name = value`: the type is inferred and the name cannot be rebound.
    Const,
    /// `let name = value`: the type is inferred and the name can be rebound.
    Let,
    /// `Type name = value`: the type is written and the name can be rebound.
    Typed(Type),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
    pub target: Expr,
    pub op: AssignOp,
    pub value: Expr,
}

// ---- types

/// A type as written: a name with optional type arguments, `i32`, `List<i32>`, `Map<String, List<i32>>`.
///
/// A function type is written like the head of a function: `function(i32, String) bool`. Its name is `function`, its
/// `args` are the types of the parameters, and `ret` is the type it gives back, absent when it gives nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: Ident,
    pub args: Vec<Type>,
    pub ret: Option<Box<Type>>,
    pub span: Span,
}

// ---- expressions

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub id: NodeId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A whole-number literal. Every legal literal fits a `u128`; a minus in front is a separate [`ExprKind::Unary`],
    /// so `-128` is a negation of 128 and whether it fits the type it ends up with is checked later.
    Int(u128),
    /// A decimal literal, kept as text without its `_` separators so that no precision is lost before the type is known.
    Float(String),
    Char(char),
    /// The text of a string with its escapes already resolved.
    Str(String),
    Bool(bool),
    Null,
    /// A use of a name: `x`, `String`, `print`.
    Name(Ident),
    /// `{1, 2, 3}`
    Array(Vec<Expr>),
    /// `[1, 2, 3]`
    List(Vec<Expr>),
    /// `(1, 2.5)`. A single value in parentheses is only a grouping and has no node of its own.
    Tuple(Vec<Expr>),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `object.name`
    Member {
        object: Box<Expr>,
        name: Ident,
    },
    /// `pointer.*`
    Deref(Box<Expr>),
    /// `f(a, b)`, `u8(300)`, `List<i32>()`, `Point(x = 1.0)`
    Call {
        callee: Box<Expr>,
        type_args: Vec<Type>,
        args: Vec<Arg>,
        /// What is written after the parentheses: `|item| { ... }`.
        trailer: Option<Trailer>,
        /// True when there were no parentheses: `double base` is `double(base)`.
        bare: bool,
    },
    /// A parameter in the head of a function definition: `Type name`, `Type name = default`, `Type ...name`. It is an
    /// argument of the call of `function`, the one place it can stand.
    Param {
        ty: Type,
        name: Ident,
        default: Option<Box<Expr>>,
        variadic: bool,
    },
    /// A generic function with its type arguments filled in, not called: `identity_of<i32>`.
    TypeApply {
        target: Box<Expr>,
        type_args: Vec<Type>,
    },
    /// A call that continues with more calls after its block: `if (a) { } elif (b) { } else { }`. The core pattern
    /// `(head [(args)] [|params|] [{ body }])+` repeats, and each repeat is one of these segments, a [`Call`](Self::Call).
    /// A call with nothing after it is just a Call, not a chain of one.
    Chain(Vec<Expr>),
    /// Put in place of an expression that could not be parsed, so the tree is complete and later stages can skip it.
    Error,
}

/// The last argument of a call written outside the parentheses: `|params| { body }`, or just `{ body }`.
///
/// `for (items) |item| { ... }` is the call `for(items, ...)` whose last argument is the block, so a loop, an `if` and a
/// call with a callback are one shape. What the block is (a function body here) is decided by the type of the last
/// parameter, which is for semantic analysis to know.
#[derive(Debug, Clone, PartialEq)]
pub struct Trailer {
    pub params: Vec<Ident>,
    /// The type written between the parameters and the block, `function(i32 n) i32 { ... }`. Only a function definition
    /// has one.
    pub ret: Option<Type>,
    pub body: Vec<Stmt>,
    /// True for a method that has no block, only its head and the type it gives back: an abstract method.
    pub described: bool,
}

/// One argument of a call, positional or `name = value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `&x`, the address of a variable
    AddrOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Equal,
    NotEqual,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl AssignOp {
    /// The operator that `x op= y` applies before storing: `+=` applies `+`. Plain `=` applies none.
    pub fn binary(self) -> Option<BinaryOp> {
        Some(match self {
            AssignOp::Set => return None,
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Sub => BinaryOp::Sub,
            AssignOp::Mul => BinaryOp::Mul,
            AssignOp::Div => BinaryOp::Div,
            AssignOp::Rem => BinaryOp::Rem,
            AssignOp::BitAnd => BinaryOp::BitAnd,
            AssignOp::BitOr => BinaryOp::BitOr,
            AssignOp::BitXor => BinaryOp::BitXor,
            AssignOp::Shl => BinaryOp::Shl,
            AssignOp::Shr => BinaryOp::Shr,
        })
    }
}

impl UnaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::BitNot => "~",
            UnaryOp::AddrOf => "&",
        }
    }
}

impl BinaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
            BinaryOp::Less => "<",
            BinaryOp::LessEq => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEq => ">=",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitXor => "^",
            BinaryOp::BitOr => "|",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }
}

impl AssignOp {
    pub fn as_str(self) -> &'static str {
        match self {
            AssignOp::Set => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Rem => "%=",
            AssignOp::BitAnd => "&=",
            AssignOp::BitOr => "|=",
            AssignOp::BitXor => "^=",
            AssignOp::Shl => "<<=",
            AssignOp::Shr => ">>=",
        }
    }
}
