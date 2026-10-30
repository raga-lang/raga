//! Pointers and manual memory (08-memory): `addr(variable)`, `p.*`, `null`, `p.at(i)`, `alloc`, `free`, `sizeof`,
//! `alignof` and `raw()`.

use super::checker::Checker;
use super::scope::Binding;
use super::ty::{IntTy, Ty};
use crate::ast::{Arg, Assign, Expr, ExprKind, Ident};
use crate::span::Span;

/// A primitive value is a plain number, flag or address: the only kind `alloc` can make room for.
fn is_primitive(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::Ptr(_)
    )
}

/// `addr(name)`: the variable a call to addr names, when the call is one.
fn addr_operand(expr: &Expr) -> Option<&Expr> {
    let ExprKind::Call { callee, args, .. } = &expr.kind else {
        return None;
    };
    let ExprKind::Name(name) = &callee.kind else {
        return None;
    };
    match args.as_slice() {
        [arg] if name.name == "addr" => Some(&arg.value),
        _ => None,
    }
}

impl Checker {
    /// `null`: the empty pointer, which only a `ptr<T>` can hold.
    pub(super) fn null_literal(&mut self, span: Span, expected: Option<&Ty>) -> Ty {
        match expected {
            Some(ptr @ Ty::Ptr(_)) => ptr.clone(),
            Some(Ty::Error) => Ty::Error,
            Some(other) => {
                self.error(
                    span,
                    format!(
                        "only a ptr<T> can be null, but {other} is wanted: use Optional<T> for a value that may be absent"
                    ),
                );
                Ty::Error
            }
            None => {
                self.error(
                    span,
                    "null has no type of its own: write the pointer type, ptr<i32> p = null",
                );
                Ty::Error
            }
        }
    }

    /// `addr(name)`: the address of a variable. A constant, a parameter, a function and a temporary value have none to give.
    pub(super) fn addr_of(&mut self, expr: &Expr, operand: &Expr, expected: Option<&Ty>) -> Ty {
        let hint = match expected {
            Some(Ty::Ptr(pointee)) => Some((**pointee).clone()),
            _ => None,
        };
        let ExprKind::Name(name) = &operand.kind else {
            self.expr(operand, None);
            self.error(
                expr.span,
                "only a variable has an address, not a temporary value",
            );
            return Ty::Error;
        };
        let ty = self.expr(operand, hint.as_ref());
        let Some(symbol) = self.scopes.lookup(&name.name) else {
            return Ty::Error;
        };
        let binding = self.scopes.symbol(symbol).binding;
        if matches!(ty, Ty::Function(_) | Ty::TypeValue { .. }) {
            self.error(
                expr.span,
                format!(
                    "`{}` is not a variable: only a variable has an address",
                    name.name
                ),
            );
            return Ty::Error;
        }
        if matches!(binding, Binding::Const | Binding::Param) {
            let what = if binding == Binding::Const {
                "const"
            } else {
                "a parameter"
            };
            self.error(
                expr.span,
                format!(
                    "`{}` is {what}: a pointer to it could change it. Declare it with let to take its address",
                    name.name
                ),
            );
            return Ty::Error;
        }
        if ty.is_error() {
            return Ty::Error;
        }
        Ty::Ptr(Box::new(ty))
    }

    /// `return &local` hands out an address that is gone as soon as the function ends.
    pub(super) fn check_returned_address(&mut self, value: &Expr) {
        let Some(operand) = addr_operand(value) else {
            return;
        };
        let ExprKind::Name(name) = &operand.kind else {
            return;
        };
        let (Some(symbol), Some(ctx)) = (self.scopes.lookup(&name.name), self.fns.last()) else {
            return;
        };
        if symbol.0 as usize >= ctx.first_symbol {
            self.error(
                value.span,
                "the address of a local variable cannot be returned: the variable is gone when the function ends",
            );
        }
    }

    /// A name of a variable of a function around the code being checked is a capture: the function being checked
    /// uses the variable itself, so it must not outlive it.
    pub(super) fn note_capture(&mut self, symbol: super::scope::SymbolId) {
        let (Some(outermost), Some(innermost)) = (self.fns.first(), self.fns.last()) else {
            return;
        };
        let index = symbol.0 as usize;
        if index >= outermost.first_symbol
            && index < innermost.first_symbol
            && let Some(ctx) = self.fns.last_mut()
        {
            ctx.captures = true;
        }
    }

    /// `return next` where `next` uses the variables of the function that returns it: they are gone when it ends.
    pub(super) fn check_returned_closure(&mut self, value: &Expr) {
        let captures = match &value.kind {
            ExprKind::Name(name) => self
                .scopes
                .lookup(&name.name)
                .is_some_and(|symbol| self.closures.contains(&symbol)),
            ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Name(n) if n.name == "function") => {
                self.last_function_captures
            }
            _ => false,
        };
        if captures {
            self.error(
                value.span,
                "this function uses variables of the function that returns it, and they are gone when it ends: keep the state in a struct, or in memory from alloc",
            );
        }
    }

    /// `p.*`: the value a pointer points to.
    pub(super) fn deref(&mut self, expr: &Expr, inner: &Expr) -> Ty {
        match self.expr(inner, None) {
            Ty::Ptr(pointee) => *pointee,
            Ty::Error => Ty::Error,
            other => {
                self.error(
                    expr.span,
                    format!("`.*` follows a pointer, but this is {other}"),
                );
                Ty::Error
            }
        }
    }

    /// `p.* = v` and `p.* += v`: the variable the pointer points to is changed.
    pub(super) fn deref_assignment(&mut self, assign: &Assign) {
        let wanted = self.expr(&assign.target, None);
        match assign.op.binary() {
            None => {
                let got = self.expr(&assign.value, Some(&wanted));
                self.expect_assignable(&wanted, &got, &assign.value);
            }
            Some(op) => self.compound_assignment(assign, op, &wanted),
        }
    }

    /// `p.at(i)`: the pointer to element `i`, the only other thing a pointer can do.
    pub(super) fn pointer_call(&mut self, name: &Ident, pointee: &Ty, args: &[Arg]) -> Ty {
        if name.name != "at" {
            self.error(
                name.span,
                format!(
                    "a pointer has no method `{}`: p.* is the value and p.at(i) the pointer to element i",
                    name.name
                ),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let [arg] = args else {
            self.error(name.span, "at takes the position: p.at(i)");
            self.check_arguments(args);
            return Ty::Error;
        };
        let got = self.expr(&arg.value, Some(&Ty::USIZE));
        if !got.is_integer() && !got.is_error() {
            self.error(
                arg.value.span,
                format!("the position is a whole number, but this is {got}"),
            );
        }
        Ty::Ptr(Box::new(pointee.clone()))
    }

    /// `alloc<T>(count)`, `free(p)`, `sizeof<T>()` and `alignof<T>()`.
    pub(super) fn memory_call(
        &mut self,
        call: &Expr,
        name: &Ident,
        type_args: &[crate::ast::Type],
        args: &[Arg],
        trailer: Option<&crate::ast::Trailer>,
    ) -> Ty {
        let text = name.name.as_str();
        if trailer.is_some() {
            self.error(
                call.span,
                format!("`{text}` takes no block after its arguments"),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        if text == "free" {
            return self.free_call(call, type_args, args);
        }
        if text == "addr" {
            return self.addr_call(call, type_args, args);
        }
        let [written] = type_args else {
            self.error(
                call.span,
                format!(
                    "{text} takes one type: {text}<T>({})",
                    if text == "alloc" { "count" } else { "" }
                ),
            );
            self.check_arguments(args);
            return Ty::Error;
        };
        let ty = self.resolve_type(written);
        if text == "alloc" {
            if !is_primitive(&ty) && !ty.is_error() {
                self.error(
                    written.span,
                    format!("alloc is for primitive types: a {ty} manages itself"),
                );
                self.check_arguments(args);
                return Ty::Error;
            }
            match args {
                [] => {}
                [arg] => {
                    let got = self.expr(&arg.value, Some(&Ty::USIZE));
                    if !got.is_integer() && !got.is_error() {
                        self.error(
                            arg.value.span,
                            format!("the count is a whole number, but this is {got}"),
                        );
                    }
                }
                _ => {
                    self.error(call.span, "alloc takes the count: alloc<T>(count)");
                    self.check_arguments(args);
                }
            }
            return if ty.is_error() {
                Ty::Error
            } else {
                Ty::Ptr(Box::new(ty))
            };
        }
        // sizeof and alignof
        if !args.is_empty() {
            self.error(call.span, format!("{text} takes no arguments: {text}<T>()"));
            self.check_arguments(args);
        }
        if matches!(
            ty,
            Ty::String
                | Ty::Array(_)
                | Ty::List(_)
                | Ty::Set(_)
                | Ty::Map(..)
                | Ty::Tuple(_)
                | Ty::Function(_)
        ) {
            self.error(
                written.span,
                format!("{ty} has no fixed size: it grows, so there is no {text}"),
            );
        }
        Ty::USIZE
    }

    /// `addr(name)`: the pointer to a variable.
    fn addr_call(&mut self, call: &Expr, type_args: &[crate::ast::Type], args: &[Arg]) -> Ty {
        if !type_args.is_empty() {
            self.error(call.span, "addr takes no type: addr(variable)");
        }
        let [arg] = args else {
            self.error(call.span, "addr takes one variable: addr(variable)");
            self.check_arguments(args);
            return Ty::Error;
        };
        self.addr_of(call, &arg.value, None)
    }

    fn free_call(&mut self, call: &Expr, type_args: &[crate::ast::Type], args: &[Arg]) -> Ty {
        if !type_args.is_empty() {
            self.error(call.span, "free takes no type: free(p)");
        }
        let [arg] = args else {
            self.error(call.span, "free takes the pointer: free(p)");
            self.check_arguments(args);
            return Ty::Void;
        };
        if let Some(operand) = addr_operand(&arg.value) {
            self.expr(&arg.value, None);
            let what = match &operand.kind {
                ExprKind::Name(n) => format!("`{}` is a variable", n.name),
                _ => "this is a variable".to_string(),
            };
            self.error(
                arg.value.span,
                format!("{what}, it was not allocated: free takes the memory of alloc"),
            );
            return Ty::Void;
        }
        let any = Ty::Ptr(Box::new(Ty::Error));
        let got = self.expr(&arg.value, Some(&any));
        if !matches!(got, Ty::Ptr(_) | Ty::Error) {
            self.error(
                arg.value.span,
                format!("free takes a pointer, but this is {got}"),
            );
        }
        Ty::Void
    }

    /// `Spot.from_raw(p)`: a new struct copied from the bytes a `ptr<u8>` points to.
    pub(super) fn static_from_raw(&mut self, call: &Expr, meta: &Ty, args: &[Arg]) -> Ty {
        let Ty::TypeValue {
            id, args: targs, ..
        } = meta
        else {
            return Ty::Error;
        };
        let want = Ty::Ptr(Box::new(Ty::Int(IntTy::U8)));
        match args {
            [arg] => {
                let got = self.expr(&arg.value, Some(&want));
                if !want.accepts(&got) {
                    self.error(arg.value.span, format!("{got} is not {want}"));
                }
            }
            _ => {
                self.error(call.span, "from_raw takes the pointer: Type.from_raw(p)");
                self.check_arguments(args);
            }
        }
        self.defined_ty(*id, targs.clone())
    }
}
