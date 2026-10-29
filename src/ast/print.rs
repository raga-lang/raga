//! A compact text form of the tree, one parenthesized expression per node:
//!
//! ```text
//! const total = 1 + 2 * 3        ->   (const total (+ 1 (* 2 3)))
//! ```
//!
//! The shape of the tree is visible at a glance, which makes it the way to look at a parse and to write a parser test:
//! `(+ 1 (* 2 3))` shows that `*` binds tighter, without a line of code to walk the tree.

use std::fmt::{self, Display, Formatter};

use super::*;

impl Display for Program {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for stmt in &self.stmts {
            writeln!(f, "{stmt}")?;
        }
        Ok(())
    }
}

impl Display for Stmt {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StmtKind::Decl(d) => match &d.kind {
                DeclKind::Const => write!(f, "(const {} {})", d.name.name, d.init),
                DeclKind::Let => write!(f, "(let {} {})", d.name.name, d.init),
                DeclKind::Typed(ty) => write!(f, "(var {ty} {} {})", d.name.name, d.init),
            },
            StmtKind::Assign(a) => write!(f, "({} {} {})", a.op.as_str(), a.target, a.value),
            StmtKind::Expr(e) => write!(f, "{e}"),
            StmtKind::Block(stmts) => {
                write!(f, "(block")?;
                for stmt in stmts {
                    write!(f, " {stmt}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.name.name == "function" {
            write!(f, "function(")?;
            for (i, arg) in self.args.iter().enumerate() {
                write!(f, "{}{arg}", if i > 0 { ", " } else { "" })?;
            }
            write!(f, ")")?;
            if let Some(ret) = &self.ret {
                write!(f, " {ret}")?;
            }
            return Ok(());
        }
        write!(f, "{}", self.name.name)?;
        write_type_args(f, &self.args)
    }
}

fn write_type_args(f: &mut Formatter<'_>, args: &[Type]) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }
    write!(f, "<")?;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{arg}")?;
    }
    write!(f, ">")
}

fn write_group(f: &mut Formatter<'_>, head: &str, items: &[Expr]) -> fmt::Result {
    write!(f, "({head}")?;
    for item in items {
        write!(f, " {item}")?;
    }
    write!(f, ")")
}

impl Display for Expr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Int(v) => write!(f, "{v}"),
            ExprKind::Float(text) => write!(f, "{text}"),
            ExprKind::Char(c) => write!(f, "{c:?}"),
            ExprKind::Str(s) => write!(f, "{s:?}"),
            ExprKind::Bool(b) => write!(f, "{b}"),
            ExprKind::Null => write!(f, "null"),
            ExprKind::Name(n) => write!(f, "{}", n.name),
            ExprKind::Array(items) => write_group(f, "array", items),
            ExprKind::List(items) => write_group(f, "list", items),
            ExprKind::Tuple(items) => write_group(f, "tuple", items),
            ExprKind::Unary { op, operand } => {
                let name = match op {
                    UnaryOp::Neg => "neg",
                    UnaryOp::Not => "not",
                    UnaryOp::BitNot => "bitnot",
                    UnaryOp::AddrOf => "addr",
                };
                write!(f, "({name} {operand})")
            }
            ExprKind::Binary { op, lhs, rhs } => write!(f, "({} {lhs} {rhs})", op.as_str()),
            ExprKind::Member { object, name } => write!(f, "(. {object} {})", name.name),
            ExprKind::Deref(inner) => write!(f, "(deref {inner})"),
            ExprKind::Call {
                callee,
                type_args,
                args,
                trailer,
                ..
            } => {
                write!(f, "(call {callee}")?;
                write_type_args(f, type_args)?;
                for arg in args {
                    match &arg.name {
                        Some(name) => write!(f, " {}={}", name.name, arg.value)?,
                        None => write!(f, " {}", arg.value)?,
                    }
                }
                if let Some(trailer) = trailer {
                    if !trailer.params.is_empty() {
                        let names: Vec<&str> =
                            trailer.params.iter().map(|p| p.name.as_str()).collect();
                        write!(f, " |{}|", names.join(" "))?;
                    }
                    if let Some(ret) = &trailer.ret {
                        write!(f, " -> {ret}")?;
                    }
                    write!(f, " {{")?;
                    for (i, stmt) in trailer.body.iter().enumerate() {
                        write!(f, "{}{stmt}", if i > 0 { " " } else { "" })?;
                    }
                    write!(f, "}}")?;
                }
                write!(f, ")")
            }
            ExprKind::Chain(segments) => write_group(f, "chain", segments),
            ExprKind::Param {
                ty,
                name,
                default,
                variadic,
            } => {
                write!(
                    f,
                    "(param {ty} {}{}",
                    if *variadic { "..." } else { "" },
                    name.name
                )?;
                if let Some(default) = default {
                    write!(f, " = {default}")?;
                }
                write!(f, ")")
            }
            ExprKind::TypeApply { target, type_args } => {
                write!(f, "(typeapply {target}")?;
                for arg in type_args {
                    write!(f, " {arg}")?;
                }
                write!(f, ")")
            }
            ExprKind::Error => write!(f, "<error>"),
        }
    }
}
