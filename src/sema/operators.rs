//! Operator overloading (09-attributes-and-reflection): `[operator(Op.Add)]` on a function, a method of a struct or a
//! method of a class makes `+` work on the types it names.

use super::checker::Checker;
use super::defined::OP;
use super::ty::{FnTy, Ty, TypeId};
use crate::ast::{Attribute, BinaryOp, ExprKind, UnaryOp};
use crate::span::Span;

/// The types that come with the language take the first numbers; a type you declare comes after them.
pub(super) const BUILTIN_TYPES: u32 = 16;

/// One operator that the program declared: the types of its operands, in order, and what it gives back.
#[derive(Debug, Clone)]
pub(super) struct OperatorDef {
    pub op: &'static str,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

const UNARY: &[&str] = &["Neg", "Not", "BitNot"];
const NAMES: &[&str] = &[
    "Add", "Sub", "Mul", "Div", "Mod", "Neg", "Not", "Eq", "Lt", "Le", "Gt", "Ge", "BitAnd",
    "BitOr", "BitXor", "BitNot", "Shl", "Shr",
];

fn is_declared(ty: &Ty) -> bool {
    matches!(ty, Ty::Defined { id, .. } if id.0 >= BUILTIN_TYPES)
}

/// Which member of `Op` stands for a binary operator of the language.
pub(super) fn op_of(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Add => "Add",
        BinaryOp::Sub => "Sub",
        BinaryOp::Mul => "Mul",
        BinaryOp::Div => "Div",
        BinaryOp::Rem => "Mod",
        BinaryOp::Equal | BinaryOp::NotEqual => "Eq",
        BinaryOp::Less => "Lt",
        BinaryOp::LessEq => "Le",
        BinaryOp::Greater => "Gt",
        BinaryOp::GreaterEq => "Ge",
        BinaryOp::BitAnd => "BitAnd",
        BinaryOp::BitOr => "BitOr",
        BinaryOp::BitXor => "BitXor",
        BinaryOp::Shl => "Shl",
        BinaryOp::Shr => "Shr",
        BinaryOp::And | BinaryOp::Or => return None,
    })
}

impl Checker {
    /// Reads `[operator(Op.Add)]` above a function (`owner` is `None`) or a method of `owner`, and remembers the operator.
    pub(super) fn register_operator(
        &mut self,
        attrs: &[Attribute],
        sig: &FnTy,
        owner: Option<TypeId>,
        span: Span,
    ) {
        for attr in attrs.iter().filter(|a| a.name.name == "operator") {
            let [arg] = attr.args.as_slice() else {
                self.error(
                    attr.span,
                    "[operator(...)] takes one member of Op, like Op.Add",
                );
                continue;
            };
            let Some(name) = self.op_member(&arg.value) else {
                continue;
            };
            let unary = UNARY.contains(&name);
            let mut params: Vec<Ty> = Vec::new();
            if let Some(owner) = owner {
                params.push(self.this_ty(owner));
            }
            params.extend(sig.params.iter().map(|p| p.ty.clone()));
            let wanted = if unary { 1 } else { 2 };
            if params.len() != wanted {
                let message = match (owner.is_some(), unary) {
                    (true, true) => format!("Op.{name} is unary: a method takes no parameter"),
                    (true, false) => {
                        format!(
                            "Op.{name} is binary: a method takes one parameter, the right operand"
                        )
                    }
                    (false, true) => format!("Op.{name} is unary: a function takes one parameter"),
                    (false, false) => {
                        format!("Op.{name} is binary: a function takes two parameters")
                    }
                };
                self.error(attr.span, message);
                continue;
            }
            if !params.iter().any(is_declared) {
                self.error(
                    attr.span,
                    "no parameter is a type you declared: an operator on i32 or String can never be changed",
                );
                continue;
            }
            if matches!(name, "Eq" | "Lt" | "Le" | "Gt" | "Ge")
                && !sig.ret.is_error()
                && *sig.ret != Ty::Bool
            {
                self.error(
                    span,
                    format!("Op.{name} gives a bool, but this gives {}", sig.ret),
                );
                continue;
            }
            let name = NAMES
                .iter()
                .find(|n| **n == name)
                .copied()
                .expect("a member of Op");
            if self
                .operators
                .iter()
                .any(|o| o.op == name && o.params == params)
            {
                self.error(
                    attr.span,
                    format!("Op.{name} is already declared for these types"),
                );
                continue;
            }
            self.operators.push(OperatorDef {
                op: name,
                params,
                ret: (*sig.ret).clone(),
            });
        }
    }

    /// The name of the member of `Op` that an expression like `Op.Add` stands for.
    fn op_member(&mut self, expr: &crate::ast::Expr) -> Option<&'static str> {
        let ExprKind::Member { object, name } = &expr.kind else {
            self.error(
                expr.span,
                "[operator(...)] takes a member of Op, like Op.Add",
            );
            return None;
        };
        if !matches!(&object.kind, ExprKind::Name(n) if n.name == "Op") {
            self.error(
                expr.span,
                "[operator(...)] takes a member of Op, like Op.Add",
            );
            return None;
        }
        let found = self.def(OP).members.iter().any(|(n, _)| *n == name.name);
        if !found {
            let why = if matches!(name.name.as_str(), "And" | "Or") {
                "&& and || cannot be overloaded, they must stop early"
            } else if name.name == "Set" || name.name == "AddrOf" || name.name == "Deref" {
                "= addr and .* are part of the language"
            } else {
                "see the members of Op"
            };
            self.error(name.span, format!("Op has no {}: {why}", name.name));
            return None;
        }
        NAMES.iter().find(|n| **n == name.name).copied()
    }

    /// The operator that fits these operands, the one whose types match best, and what it gives back.
    pub(super) fn find_operator(&self, op: &str, operands: &[Ty]) -> Option<Ty> {
        let mut best: Option<(usize, &OperatorDef)> = None;
        for def in self.operators.iter().filter(|o| o.op == op) {
            if def.params.len() != operands.len()
                || !def
                    .params
                    .iter()
                    .zip(operands)
                    .all(|(p, o)| p.accepts_sub(o))
            {
                continue;
            }
            let exact = def
                .params
                .iter()
                .zip(operands)
                .filter(|(p, o)| p == o)
                .count();
            if best.is_none_or(|(score, _)| exact > score) {
                best = Some((exact, def));
            }
        }
        best.map(|(_, def)| def.ret.clone())
    }

    /// `a + b` where an operand is a type of the program: the operator that was declared for it.
    pub(super) fn user_binary(
        &mut self,
        op: BinaryOp,
        left: &Ty,
        right: &Ty,
        span: Span,
    ) -> Option<Ty> {
        if !(is_declared(left) || is_declared(right)) {
            return None;
        }
        let name = op_of(op)?;
        match self.find_operator(name, &[left.clone(), right.clone()]) {
            Some(ret) => Some(if op == BinaryOp::NotEqual {
                Ty::Bool
            } else {
                ret
            }),
            None if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) => None,
            None => {
                self.error(
                    span,
                    format!(
                        "no operator `{}` for {left} and {right}: declare one with [operator(Op.{name})]",
                        op.as_str()
                    ),
                );
                Some(Ty::Error)
            }
        }
    }

    /// `-a`, `!a` and `~a` where the operand is a type of the program.
    pub(super) fn user_unary(&mut self, op: UnaryOp, operand: &Ty, span: Span) -> Option<Ty> {
        if !is_declared(operand) {
            return None;
        }
        let (name, symbol) = match op {
            UnaryOp::Neg => ("Neg", "-"),
            UnaryOp::Not => ("Not", "!"),
            UnaryOp::BitNot => ("BitNot", "~"),
        };
        match self.find_operator(name, std::slice::from_ref(operand)) {
            Some(ret) => Some(ret),
            None => {
                self.error(
                    span,
                    format!("no operator `{symbol}` for {operand}: declare one with [operator(Op.{name})]"),
                );
                Some(Ty::Error)
            }
        }
    }
}
