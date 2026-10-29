//! Checking expressions: literals, names, operators and collections.
//!
//! [`Checker::expr`] takes the type the context *expects*, if there is one, and gives back the type it *found*. The
//! expected type matters only for literals, which have no type of their own, see the module documentation of
//! [`sema`](super).

use super::builtins;
use super::checker::{Checker, is_untyped};
use super::ty::{IntTy, Ty};
use crate::ast::{BinaryOp, Expr, ExprKind, Ident, UnaryOp};
use crate::span::Span;

impl Checker {
    /// Checks an expression and records its type.
    pub(super) fn expr(&mut self, expr: &Expr, expected: Option<&Ty>) -> Ty {
        let ty = self.expr_kind(expr, expected);
        self.expr_types.insert(expr.id, ty.clone());
        ty
    }

    fn expr_kind(&mut self, expr: &Expr, expected: Option<&Ty>) -> Ty {
        match &expr.kind {
            ExprKind::Int(value) => self.int_literal(expr.span, *value, false, expected),
            ExprKind::Float(text) => self.float_literal(expr.span, text, expected),
            ExprKind::Char(_) => Ty::Char,
            ExprKind::Str(_) => Ty::String,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Null => self.null_literal(expr.span, expected),
            ExprKind::Name(name) => self.name(expr, name, expected),
            ExprKind::Array(items) => {
                self.collection(expr.span, items, expected, Collection::Array)
            }
            ExprKind::List(items) => self.collection(expr.span, items, expected, Collection::List),
            ExprKind::Tuple(items) => self.tuple(items, expected),
            ExprKind::Unary { op, operand } => self.unary(expr, *op, operand, expected),
            ExprKind::Binary { op, lhs, rhs } => self.binary(expr, *op, lhs, rhs, expected),
            ExprKind::Member { object, name } => self.member_value(object, name, expected),
            ExprKind::Deref(inner) => self.deref(expr, inner),
            ExprKind::Call {
                callee,
                type_args,
                args,
                trailer,
                bare,
            } => self.call(
                expr,
                callee,
                type_args,
                args,
                trailer.as_ref(),
                *bare,
                expected,
            ),
            ExprKind::TypeApply { target, type_args } => self.type_apply(expr, target, type_args),
            ExprKind::Param { .. } => {
                self.error(
                    expr.span,
                    "a parameter can only stand in the head of a function",
                );
                Ty::Error
            }
            ExprKind::Chain(segments) => self.chain(segments),
            ExprKind::Error => Ty::Error,
        }
    }

    // ---- literals

    /// A whole-number literal. `negative` is true when a minus is directly in front of it.
    ///
    /// It takes the integer type that is expected and must fit it. It is never a float: a whole number where a float is
    /// needed must be written `2.0`. With nothing expected it is an `i32`.
    pub(super) fn int_literal(
        &mut self,
        span: Span,
        value: u128,
        negative: bool,
        expected: Option<&Ty>,
    ) -> Ty {
        let shown = if negative {
            format!("-{value}")
        } else {
            value.to_string()
        };
        match expected {
            Some(Ty::Int(int)) => {
                if !int.fits(value, negative) {
                    self.error(span, format!("{shown} does not fit in {}", int.name()));
                }
                Ty::Int(*int)
            }
            Some(Ty::Float(float)) => {
                self.error(
                    span,
                    format!(
                        "{shown} is a whole number and a whole-number literal is never a float: write {shown}.0 for the {}",
                        float.name()
                    ),
                );
                Ty::Float(*float)
            }
            _ => {
                if !IntTy::I32.fits(value, negative) {
                    self.error(
                        span,
                        format!(
                            "{shown} does not fit in i32, which is the type of a number with no type around it: write the type, for example `i64 x = {shown}`"
                        ),
                    );
                }
                Ty::I32
            }
        }
    }

    fn float_literal(&mut self, span: Span, text: &str, expected: Option<&Ty>) -> Ty {
        match expected {
            Some(Ty::Float(float)) => Ty::Float(*float),
            Some(Ty::Int(int)) => {
                self.error(
                    span,
                    format!("{text} is a decimal and a decimal literal is never an integer: it cannot be {}", int.name()),
                );
                Ty::Int(*int)
            }
            _ => Ty::F64,
        }
    }

    // ---- names

    fn name(&mut self, expr: &Expr, name: &Ident, expected: Option<&Ty>) -> Ty {
        if let Some(symbol) = self.scopes.lookup(&name.name) {
            self.symbol_of.insert(expr.id, symbol);
            if let Some(reason) = self.deprecated.get(&symbol) {
                let message = format!("`{}` is deprecated: {reason}", name.name);
                self.diagnostics
                    .push(crate::diagnostics::Diagnostic::warning(name.span, message));
            }
            if self.pending.contains(&symbol) {
                self.error(
                    name.span,
                    format!("`{}` uses itself, so what it returns cannot be worked out: write the return type", name.name),
                );
                return Ty::Error;
            }
            return match self.scopes.symbol(symbol).ty.clone() {
                Ty::Function(f) => self.function_name(expr, name, f, expected),
                ty => ty,
            };
        }
        // `Optional` and `Result` come with the language and are used by their name
        if let Some(id) = self.type_named(&name.name) {
            return self.type_value_ty(id, Vec::new());
        }
        if let Some(kind) = builtins::reserved(&name.name) {
            let message = match kind {
                builtins::Reserved::Function => {
                    format!(
                        "`{}` is built in and can only be called: {}(...)",
                        name.name, name.name
                    )
                }
                builtins::Reserved::Type => {
                    format!(
                        "`{}` is a type, not a value: write {}(...) to make or convert one",
                        name.name, name.name
                    )
                }
            };
            self.error(name.span, message);
            return Ty::Error;
        }
        self.undeclared(name);
        Ty::Error
    }

    /// A name that is not declared. Three different mistakes look the same from here, and the message tells which.
    pub(super) fn undeclared(&mut self, name: &Ident) {
        let message = if self.in_static && matches!(name.name.as_str(), "this" | "super") {
            format!(
                "a static member has no `{}`: it runs without an object",
                name.name
            )
        } else if self.scopes.declared_below(&name.name) {
            self.declared_below_message(&name.name)
        } else if self.scopes.went_out_of_scope(&name.name) {
            format!(
                "`{}` is out of scope here: it was declared in a block that has ended",
                name.name
            )
        } else {
            format!("`{}` is not declared", name.name)
        };
        self.error(name.span, message);
    }

    /// `Math` or `String` written before a dot, when no variable of that name hides them.
    pub(super) fn namespace(&self, object: &Expr) -> Option<&'static str> {
        let ExprKind::Name(name) = &object.kind else {
            return None;
        };
        if self.scopes.lookup(&name.name).is_some() {
            return None;
        }
        match name.name.as_str() {
            "Math" => Some("Math"),
            "String" => Some("String"),
            "System" => Some("System"),
            "File" => Some("File"),
            "Time" => Some("Time"),
            "Random" => Some("Random"),
            _ => None,
        }
    }

    /// `object.name` used as a value, not called: `Math.pi`.
    fn member_value(&mut self, object: &Expr, name: &Ident, expected: Option<&Ty>) -> Ty {
        if let Some(namespace) = self.namespace(object) {
            if namespace == "Math" && builtins::math_constant(&name.name) {
                return Ty::F64;
            }
            self.error(
                name.span,
                format!("`{namespace}.{}` is not a value", name.name),
            );
            return Ty::Error;
        }
        let ty = self.expr(object, None);
        if let Ty::Module(module) = ty {
            return self.module_value(module, name);
        }
        // a field is reached through a pointer to a struct without `.*`
        let ty = match ty {
            Ty::Ptr(inner) if matches!(*inner, Ty::Defined { .. }) => *inner,
            ty => ty,
        };
        match &ty {
            Ty::TypeValue { .. } => return self.static_member(&ty, name, expected, object.span),
            Ty::Defined { .. } => return self.field_type(&ty, name).unwrap_or(Ty::Error),
            Ty::TypeParam(param) => {
                self.error(
                    name.span,
                    format!(
                        "{param} is a type parameter and has no member `{}`",
                        name.name
                    ),
                );
                return Ty::Error;
            }
            Ty::Error => return Ty::Error,
            _ => {}
        }
        self.error(
            name.span,
            format!("a value of type {ty} has no field `{}`", name.name),
        );
        Ty::Error
    }

    // ---- operators

    fn unary(&mut self, expr: &Expr, op: UnaryOp, operand: &Expr, expected: Option<&Ty>) -> Ty {
        match op {
            UnaryOp::Neg => {
                // `-128` is the number 128 with a minus, and must fit as a negative number
                if let ExprKind::Int(value) = operand.kind {
                    let ty = self.int_literal(operand.span, value, true, expected);
                    self.expr_types.insert(operand.id, ty.clone());
                    return ty;
                }
                let ty = self.expr(operand, expected);
                if let Some(result) = self.user_unary(op, &ty, expr.span) {
                    return result;
                }
                match &ty {
                    Ty::Error | Ty::Float(_) => ty,
                    Ty::Int(int) if int.is_signed() => ty,
                    _ => {
                        self.error(
                            expr.span,
                            format!("cannot negate {ty}: `-` needs a signed integer or a decimal"),
                        );
                        Ty::Error
                    }
                }
            }
            UnaryOp::Not => {
                let ty = self.expr(operand, None);
                if let Some(result) = self.user_unary(op, &ty, expr.span) {
                    return result;
                }
                if !Ty::Bool.accepts(&ty) {
                    self.error(expr.span, format!("`!` needs a bool, but this is {ty}"));
                }
                Ty::Bool
            }
            UnaryOp::BitNot => {
                let ty = self.expr(operand, expected);
                if let Some(result) = self.user_unary(op, &ty, expr.span) {
                    return result;
                }
                if ty.is_integer() || ty.is_error() {
                    ty
                } else {
                    self.error(expr.span, format!("`~` needs an integer, but this is {ty}"));
                    Ty::Error
                }
            }
            UnaryOp::AddrOf => self.addr_of(expr, operand, expected),
        }
    }

    fn binary(
        &mut self,
        expr: &Expr,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        expected: Option<&Ty>,
    ) -> Ty {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            for side in [lhs, rhs] {
                let ty = self.expr(side, None);
                if !Ty::Bool.accepts(&ty) {
                    self.error(
                        side.span,
                        format!("`{}` needs bool operands, but this is {ty}", op.as_str()),
                    );
                }
            }
            return Ty::Bool;
        }
        let compares = matches!(
            op,
            BinaryOp::Less
                | BinaryOp::LessEq
                | BinaryOp::Greater
                | BinaryOp::GreaterEq
                | BinaryOp::Equal
                | BinaryOp::NotEqual
        );
        // a comparison gives a bool, so what the context expects says nothing about its operands
        let (left, right) = self.operand_types(lhs, rhs, if compares { None } else { expected });
        if let Some(result) = self.user_binary(op, &left, &right, expr.span) {
            return result;
        }
        if matches!(left, Ty::Ptr(_)) && !compares {
            self.error(
                expr.span,
                "a pointer is not a number: there is no pointer arithmetic, use p.at(i) on memory from alloc",
            );
            return Ty::Error;
        }
        if !left.accepts(&right) {
            self.error(
                expr.span,
                format!(
                    "{left} and {right}: both sides of `{}` must have the same type, convert one first",
                    op.as_str()
                ),
            );
            return if compares { Ty::Bool } else { Ty::Error };
        }
        // a side that is already an error makes the result an error, so the one mistake is not reported again
        if left.is_error() || right.is_error() {
            return if compares { Ty::Bool } else { Ty::Error };
        }
        self.operator_result(op, &left, expr.span)
    }

    /// The types of the two sides of an operator.
    ///
    /// Both sides must have the same type, and a literal side takes the type of the other side. So the side that has a
    /// type of its own is checked first, and the literal is then checked against it: in `byte + 300` the `300` is
    /// asked to be a `u8`, and does not fit.
    fn operand_types(&mut self, lhs: &Expr, rhs: &Expr, hint: Option<&Ty>) -> (Ty, Ty) {
        match (is_untyped(lhs), is_untyped(rhs)) {
            // `null == p`: the pointer decides what null is
            _ if matches!(lhs.kind, ExprKind::Null) && !matches!(rhs.kind, ExprKind::Null) => {
                let right = self.expr(rhs, hint);
                let left = self.expr(lhs, Some(&right));
                (left, right)
            }
            (true, false) => {
                let right = self.expr(rhs, hint);
                // a literal next to a type of the program says nothing about it: an operator decides
                let left = self.expr(
                    lhs,
                    Some(&right).filter(|r| !matches!(r, Ty::Defined { .. })),
                );
                (left, right)
            }
            _ => {
                let left = self.expr(lhs, hint);
                let right = self.expr(
                    rhs,
                    Some(&left).filter(|l| !matches!(l, Ty::Defined { .. })),
                );
                (left, right)
            }
        }
    }

    /// Is the operator allowed on this type, and what does it give back?
    pub(super) fn operator_result(&mut self, op: BinaryOp, operand: &Ty, span: Span) -> Ty {
        use BinaryOp::*;
        if operand.is_error() {
            return match op {
                Less | LessEq | Greater | GreaterEq | Equal | NotEqual => Ty::Bool,
                _ => Ty::Error,
            };
        }
        let symbol = op.as_str();
        match op {
            Add | Sub | Mul | Div | Rem => {
                if operand.is_number() {
                    return operand.clone();
                }
                let message = if op == Add && *operand == Ty::String {
                    "there is no `+` on Strings, use String.join(a, b)".to_string()
                } else {
                    format!("`{symbol}` needs numbers, but this is {operand}")
                };
                self.error(span, message);
                Ty::Error
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                if operand.is_integer() {
                    return operand.clone();
                }
                self.error(
                    span,
                    format!("`{symbol}` needs integers, but this is {operand}"),
                );
                Ty::Error
            }
            Less | LessEq | Greater | GreaterEq => {
                if !(operand.is_number() || *operand == Ty::Char) {
                    self.error(
                        span,
                        format!(
                            "`{symbol}` compares numbers and characters, but this is {operand}"
                        ),
                    );
                }
                Ty::Bool
            }
            Equal | NotEqual => Ty::Bool,
            And | Or => Ty::Bool,
        }
    }

    // ---- collections

    /// `{1, 2, 3}` and `[1, 2, 3]`. The contents decide the element type, so `{1, 2.5}` is an error. An empty literal
    /// has no contents, so its type must come from the context.
    fn collection(
        &mut self,
        span: Span,
        items: &[Expr],
        expected: Option<&Ty>,
        kind: Collection,
    ) -> Ty {
        if let Some(Ty::Error) = expected {
            // the declared type is already reported: check the items for their own mistakes and say no more
            for item in items {
                self.expr(item, None);
            }
            return Ty::Error;
        }
        let wanted = match (kind, expected) {
            (Collection::Array, Some(Ty::Array(element)))
            | (Collection::List, Some(Ty::List(element))) => Some((**element).clone()),
            _ => None,
        };
        let refs: Vec<&Expr> = items.iter().collect();
        let Some(element) = self.element_type(span, &refs, wanted) else {
            return Ty::Error;
        };
        match kind {
            Collection::Array => Ty::Array(Box::new(element)),
            Collection::List => Ty::List(Box::new(element)),
        }
    }

    /// The one type shared by all the items. `None` after an error was reported.
    pub(super) fn element_type(
        &mut self,
        span: Span,
        items: &[&Expr],
        wanted: Option<Ty>,
    ) -> Option<Ty> {
        if items.is_empty() {
            if wanted.is_none() {
                self.error(
                    span,
                    "an empty literal has no items to decide its type: give it a type, for example `List<i32> xs = []`",
                );
            }
            return wanted;
        }
        if wanted.is_none() && !self.number_kinds_agree(items) {
            for item in items {
                self.expr(item, None);
            }
            return None;
        }
        // The element type is the one the context wants, or else the type of the first item that has a type of its own,
        // or else the first item. The items are then checked against it, which is what makes `{x, 1}` work for any `x`.
        let mut element = wanted;
        let mut done = vec![None; items.len()];
        if element.is_none()
            && let Some(first) = items.iter().position(|item| !is_untyped(item))
        {
            let ty = self.expr(items[first], None);
            element = Some(ty.clone());
            done[first] = Some(ty);
        }
        for (i, item) in items.iter().enumerate() {
            let ty = match done[i].take() {
                Some(ty) => ty,
                None => self.expr(item, element.as_ref()),
            };
            match &element {
                None => element = Some(ty),
                Some(wanted) if !wanted.accepts(&ty) => {
                    self.error(
                        item.span,
                        format!("all the items must have the same type, but this is {ty} and the others are {wanted}: use a Tuple for mixed types"),
                    );
                }
                Some(_) => {}
            }
        }
        element
    }

    /// Items that are all numbers written out, with nothing around them to give a type, are whole numbers or decimals: a
    /// mix of the two is the mistake `{1, 2.5}` makes, and it is said as that, not as a literal that does not fit.
    fn number_kinds_agree(&mut self, items: &[&Expr]) -> bool {
        if !items.iter().all(|item| is_untyped(item)) {
            return true;
        }
        let first = is_decimal(items[0]);
        if let Some(other) = items.iter().find(|item| is_decimal(item) != first) {
            let (this, others) = if first {
                ("a whole number", "decimals")
            } else {
                ("a decimal", "whole numbers")
            };
            self.error(
                other.span,
                format!("all the items must have the same type, but this is {this} and the others are {others}: use a Tuple for mixed types"),
            );
            return false;
        }
        true
    }

    fn tuple(&mut self, items: &[Expr], expected: Option<&Ty>) -> Ty {
        let slots = match expected {
            Some(Ty::Tuple(slots)) if slots.len() == items.len() => Some(slots.as_slice()),
            _ => None,
        };
        let types = items
            .iter()
            .enumerate()
            .map(|(i, item)| self.expr(item, slots.map(|s| &s[i])))
            .collect();
        Ty::Tuple(types)
    }
}

#[derive(Clone, Copy)]
enum Collection {
    Array,
    List,
}

/// Is a number written out a decimal? `2.5` and `1 + 2.5` are; `2` is not.
fn is_decimal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Float(_) => true,
        ExprKind::Unary { operand, .. } => is_decimal(operand),
        ExprKind::Binary { lhs, rhs, .. } => is_decimal(lhs) || is_decimal(rhs),
        _ => false,
    }
}
