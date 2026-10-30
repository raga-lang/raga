//! Functions (04-functions): defining one, calling one, and what flows in and out of it.
//!
//! A function is a value of type [`FnTy`]: the types it takes and the type it gives back. It is written as the call of
//! `function` (see the parser), so everything here hangs off two places: checking that call, which makes the type and
//! checks the body, and checking a call of a value that has such a type, which matches the arguments to the parameters.

use std::collections::{HashMap, HashSet};

use super::checker::{Checker, declared_names};
use super::scope::{Binding, Symbol};
use super::ty::{DefinedKind, FnTy, ParamTy, Ty};
use crate::ast::{self, Arg, Expr, ExprKind, NodeId, Stmt, StmtKind, Trailer};
use crate::span::Span;

/// What is known about the function whose body is being checked, for its `return` statements.
pub(super) struct FnCtx {
    /// The type written for what it gives back, or `None` when it is to be worked out from the returns.
    pub(super) declared: Option<Ty>,
    pub(super) returns: Vec<(Ty, Span)>,
    /// The first symbol the function declares: a name below this one belongs to the code around it.
    pub(super) first_symbol: usize,
}

/// A name and the type it has inside a function: the parameters.
pub(super) struct Local {
    pub(super) name: String,
    pub(super) span: Span,
    pub(super) ty: Ty,
}

/// A name that stands for a value inside a function.
pub(super) fn local(name: &str, span: Span, ty: Ty) -> Local {
    Local {
        name: name.to_string(),
        span,
        ty,
    }
}

/// Is this expression the definition of a function, `function(...) { ... }`?
pub(super) fn is_definition(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::Call { callee, .. }
        if matches!(&callee.kind, ExprKind::Name(name) if name.name == "function"))
}

impl Checker {
    // ---- defining

    /// `function<T>(Type name, ...) Type { ... }`: checks the parameters and the body, and gives the type.
    pub(super) fn function_value(
        &mut self,
        call: &Expr,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let Some(trailer) = trailer else {
            self.error(
                call.span,
                "a function needs a body: function(Type name) { ... }",
            );
            return Ty::Error;
        };
        let this = self.method_this.take();
        let sup = self.method_super.take();
        let generics = self.type_parameters(type_args);
        self.type_params.push(generics.clone());
        let (sig, mut locals) = self.signature(args, trailer.ret.as_ref(), generics);
        if let Some(this) = this {
            // inside a method, `this` is the value the method was called on, and `super` is its base class
            if let Some(sup) = sup {
                locals.insert(0, local("super", call.span, sup));
            }
            locals.insert(0, local("this", call.span, this));
        }
        let declared = trailer.ret.as_ref().map(|_| (*sig.ret).clone());
        let ret = self.function_body(locals, &trailer.body, declared);
        self.type_params.pop();
        Ty::Function(FnTy {
            ret: Box::new(ret),
            ..sig
        })
    }

    pub(super) fn type_parameters(&mut self, type_args: &[ast::Type]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for ty in type_args {
            if !ty.args.is_empty() || ty.ret.is_some() {
                self.error(ty.span, "a type parameter is a plain name like T");
            } else if names.contains(&ty.name.name) {
                self.error(ty.span, format!("`{}` is declared twice", ty.name.name));
            } else {
                names.push(ty.name.name.clone());
            }
        }
        names
    }

    /// The type of the head of a function: its parameters, and the type it gives back when one is written. The rules for
    /// the parameters are checked here. Expects the type parameters to be in scope already.
    pub(super) fn signature(
        &mut self,
        args: &[Arg],
        ret: Option<&ast::Type>,
        generics: Vec<String>,
    ) -> (FnTy, Vec<Local>) {
        let mut params: Vec<ParamTy> = Vec::new();
        let mut locals: Vec<Local> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        for arg in args {
            let ExprKind::Param {
                ty,
                name,
                default,
                variadic,
            } = &arg.value.kind
            else {
                continue;
            };
            let resolved = self.resolve_type(ty);
            if let Some(default) = default {
                let got = self.expr(default, Some(&resolved));
                self.expect_assignable(&resolved, &got, default);
            }
            if locals.iter().any(|l| l.name == name.name) {
                self.error(
                    name.span,
                    format!("`{}` is already a parameter of this function", name.name),
                );
            }
            locals.push(Local {
                name: name.name.clone(),
                span: name.span,
                // inside the function a variadic parameter is an Array of the extra arguments
                ty: if *variadic {
                    Ty::Array(Box::new(resolved.clone()))
                } else {
                    resolved.clone()
                },
            });
            params.push(ParamTy {
                name: Some(name.name.clone()),
                ty: resolved,
                has_default: default.is_some(),
                variadic: *variadic,
            });
            spans.push(arg.value.span);
        }
        self.check_parameter_rules(&params, &spans);
        let ret = ret.map_or(Ty::Void, |written| self.resolve_type(written));
        (
            FnTy {
                generics,
                params,
                ret: Box::new(ret),
            },
            locals,
        )
    }

    /// The order of the parameters: defaults after the parameters without one, a variadic parameter last and alone, and a
    /// callback never followed by a parameter that has a default or is variadic, because the block that fills a callback
    /// is always the last argument.
    fn check_parameter_rules(&mut self, params: &[ParamTy], spans: &[Span]) {
        let mut seen_default = false;
        for (i, param) in params.iter().enumerate() {
            let last = i + 1 == params.len();
            if param.variadic {
                if !last {
                    self.error(spans[i], "the variadic parameter must be last");
                }
                if seen_default || params.iter().any(|p| p.has_default) {
                    self.error(
                        spans[i],
                        "default values and a variadic parameter cannot be mixed",
                    );
                }
            }
            // the callback that receives a trailing block is the last parameter, and may come after the defaults
            let is_callback = matches!(param.ty, Ty::Function(_));
            let ordinary = !param.has_default && !param.variadic;
            if ordinary && seen_default && !(is_callback && last) {
                self.error(
                    spans[i],
                    "a default must not come before a parameter without one",
                );
            }
            if let Some(next) = params.get(i + 1)
                && is_callback
                && (next.has_default || next.variadic)
            {
                self.error(
                    spans[i + 1],
                    "nothing may follow a callback but a parameter without a default: the block that fills a callback is the last argument",
                );
            }
            seen_default |= param.has_default;
        }
    }

    /// Checks the statements of a function, a block or a callback in a scope of their own with `locals` declared in it, and
    /// gives the type it returns: the declared one, else the type of its `return` statements.
    pub(super) fn function_body(
        &mut self,
        locals: Vec<Local>,
        body: &[Stmt],
        declared: Option<Ty>,
    ) -> Ty {
        let first_symbol = self.scopes.symbols.len();
        self.scopes.push(declared_names(body));
        for local in locals {
            let symbol = Symbol {
                name: local.name,
                binding: Binding::Param,
                ty: local.ty,
                span: local.span,
                constant: None,
                defines: None,
            };
            let _ = self.scopes.declare(symbol);
        }
        self.fns.push(FnCtx {
            declared: declared.clone(),
            returns: Vec::new(),
            first_symbol,
        });
        // a break or continue in a function belongs to a loop of the function
        let outer_loops = std::mem::replace(&mut self.loops, 0);
        let outer_finally = std::mem::replace(&mut self.in_finally, 0);
        self.statements(body);
        self.loops = outer_loops;
        self.in_finally = outer_finally;
        let ctx = self.fns.pop().expect("pushed above");
        self.scopes.pop();

        match declared {
            Some(ret) => {
                if ret != Ty::Void && !ret.is_error() && !terminates(body, &self.exhaustive) {
                    let at = body.last().map_or(Span::default(), |stmt| stmt.span);
                    self.error(
                        at,
                        format!("this function returns {ret}, but not every path returns a value"),
                    );
                }
                ret
            }
            None => self.inferred_return(&ctx.returns),
        }
    }

    /// The type of a function whose type is not written: that of its returns, which must all be the same.
    fn inferred_return(&mut self, returns: &[(Ty, Span)]) -> Ty {
        let Some((first, _)) = returns.iter().find(|(ty, _)| !ty.is_error()) else {
            return if returns.is_empty() {
                Ty::Void
            } else {
                Ty::Error
            };
        };
        let first = first.clone();
        for (ty, span) in returns {
            if !first.accepts(ty) {
                self.error(
                    *span,
                    format!(
                        "the returns disagree: this gives {ty}, but another return gives {first}"
                    ),
                );
            }
        }
        first
    }

    /// `return` and `return value`.
    pub(super) fn return_statement(
        &mut self,
        call: &Expr,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) {
        let Some(declared) = self.fns.last().map(|ctx| ctx.declared.clone()) else {
            self.error(call.span, "`return` belongs to functions only");
            self.check_arguments(args);
            return;
        };
        if self.in_finally > 0 {
            self.error(
                call.span,
                "`return` in a finally block is not allowed: it would hide the error or the value being returned",
            );
        }
        if let Some(trailer) = trailer {
            // `return |x| { }` and `return { 1, 2 }`: the block is the value, and the declared type says what it is
            let Some(ret) = declared.as_ref().filter(|ret| **ret != Ty::Void) else {
                self.error(
                    call.span,
                    "a block after `return` needs the function to declare the type it gives back",
                );
                self.scoped_statements(&trailer.body);
                return;
            };
            self.block_argument("return", ret, trailer, &[], &mut HashMap::new());
            return;
        }
        let (ty, span) = match args {
            [] => (Ty::Void, call.span),
            [arg] => {
                let wanted = declared.as_ref().filter(|ret| **ret != Ty::Void);
                let got = self.expr(&arg.value, wanted);
                self.check_returned_address(&arg.value);
                (got, arg.value.span)
            }
            _ => {
                self.error(call.span, "`return` gives one value");
                self.check_arguments(args);
                return;
            }
        };
        match declared {
            Some(ret) => {
                if ret == Ty::Void && ty != Ty::Void && !ty.is_error() {
                    self.error(
                        span,
                        "this function gives nothing back, so `return` has no value",
                    );
                } else if ret != Ty::Void && ty == Ty::Void {
                    self.error(
                        span,
                        format!("this function returns {ret}, so `return` needs a value"),
                    );
                } else if ret != Ty::Void
                    && let Some(arg) = args.first()
                {
                    self.expect_assignable(&ret, &ty, &arg.value);
                }
            }
            None => {
                if let Some(ctx) = self.fns.last_mut() {
                    ctx.returns.push((ty, span));
                }
            }
        }
    }

    // ---- calling

    /// A call of a value that is a function: matches the arguments to the parameters, works out the type parameters,
    /// checks the block that fills the last parameter, and gives the type it returns.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_function(
        &mut self,
        call: &Expr,
        label: &str,
        f: &FnTy,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let generics = f.generics.as_slice();
        let mut known: HashMap<String, Ty> = HashMap::new();
        if !type_args.is_empty() {
            let written: Vec<Ty> = type_args.iter().map(|t| self.resolve_type(t)).collect();
            if written.len() == generics.len() {
                known.extend(generics.iter().cloned().zip(written));
            } else {
                self.error(
                    call.span,
                    format!(
                        "{label} has {} type parameter{}, but {} type argument{} given",
                        generics.len(),
                        if generics.len() == 1 { "" } else { "s" },
                        type_args.len(),
                        if type_args.len() == 1 {
                            " was"
                        } else {
                            "s were"
                        },
                    ),
                );
            }
        }

        let count = f.params.len();
        let block = trailer.map(|_| count.checked_sub(1));
        if let Some(None) = block {
            self.error(
                call.span,
                format!("{label} takes no parameters, so there is nothing for the block to fill"),
            );
        }
        let block_at = block.flatten();

        // which argument goes to which parameter
        let mut given: Vec<Option<&Expr>> = vec![None; count];
        let mut extra: Vec<&Expr> = Vec::new();
        let variadic_at = f.params.iter().position(|p| p.variadic);
        let mut position = 0;
        let mut named_seen = false;
        for arg in args {
            match &arg.name {
                None => {
                    if named_seen {
                        self.error(
                            arg.value.span,
                            "a positional argument cannot follow a named one",
                        );
                    }
                    match variadic_at {
                        Some(v) if position >= v => extra.push(&arg.value),
                        _ if position < count => given[position] = Some(&arg.value),
                        _ => {
                            self.error(
                                arg.value.span,
                                format!(
                                    "{label} takes {count} argument{}, but more were given",
                                    if count == 1 { "" } else { "s" }
                                ),
                            );
                            self.expr(&arg.value, None);
                        }
                    }
                    position += 1;
                }
                Some(name) => {
                    named_seen = true;
                    let at = f
                        .params
                        .iter()
                        .position(|p| p.name.as_deref() == Some(name.name.as_str()));
                    match at {
                        None => {
                            self.error(
                                name.span,
                                format!("{label} has no parameter `{}`", name.name),
                            );
                            self.expr(&arg.value, None);
                        }
                        Some(i) if f.params[i].variadic => {
                            self.error(
                                name.span,
                                "a variadic parameter takes its arguments by position, not by name",
                            );
                            self.expr(&arg.value, None);
                        }
                        Some(i) if given[i].is_some() => {
                            self.error(name.span, format!("`{}` is given twice", name.name));
                            self.expr(&arg.value, None);
                        }
                        Some(i) => given[i] = Some(&arg.value),
                    }
                }
            }
        }

        if let Some(at) = block_at {
            if given[at].is_some() {
                self.error(
                    given[at].map_or(call.span, |v| v.span),
                    "the last parameter is given inside the parentheses already, so the block is one argument too many",
                );
            } else if f.params[at].variadic {
                self.error(call.span, "a variadic parameter cannot take a block");
            }
        }

        // the arguments, in the order of the parameters, so that what one decides about a type parameter is known to the
        // next
        for (i, param) in f.params.iter().enumerate() {
            let Some(value) = given[i] else {
                let is_block = block_at == Some(i);
                if !param.has_default && !param.variadic && !is_block {
                    let name = param.name.as_deref().unwrap_or("a parameter");
                    self.error(
                        call.span,
                        format!("`{name}` has no default and was not given"),
                    );
                }
                continue;
            };
            if param.variadic {
                continue;
            }
            self.argument_for(value, &param.ty, generics, &mut known);
        }
        if let Some(v) = variadic_at {
            for value in extra {
                self.argument_for(value, &f.params[v].ty, generics, &mut known);
            }
        }
        if let (Some(trailer), Some(at)) = (trailer, block_at)
            && !f.params[at].variadic
        {
            let wanted = f.params[at].ty.clone();
            self.block_argument(label, &wanted, trailer, generics, &mut known);
        } else if let Some(trailer) = trailer {
            self.scoped_statements(&trailer.body);
        }

        let ret = f.ret.substitute(&known);
        if ret.has_unknown(generics, &known) {
            self.error(
                call.span,
                format!(
                    "{label} cannot tell {ret}: write the type arguments, like {label}<i32>(...)"
                ),
            );
            return Ty::Error;
        }
        ret
    }

    /// One argument for a parameter of type `param`, which may be written with the type parameters of the function.
    pub(super) fn argument_for(
        &mut self,
        value: &Expr,
        param: &Ty,
        generics: &[String],
        known: &mut HashMap<String, Ty>,
    ) {
        let wanted = param.substitute(known);
        // a literal takes the type that is wanted, which can be said only once no type parameter is left in it
        let hint = (!wanted.has_unknown(generics, known)).then_some(&wanted);
        let got = self.expr(value, hint);
        if wanted.has_unknown(generics, known) {
            if !wanted.unify(&got, generics, known) {
                self.error(value.span, format!("{got} is not {wanted}"));
            }
        } else {
            self.expect_assignable(&wanted, &got, value);
        }
    }

    /// The block after a call, which fills the last parameter. The type of that parameter says what the block is: the body of
    /// a function, or the values of an Array. Any other type is passed inside the parentheses.
    pub(super) fn block_argument(
        &mut self,
        label: &str,
        param: &Ty,
        trailer: &Trailer,
        generics: &[String],
        known: &mut HashMap<String, Ty>,
    ) {
        let wanted = param.substitute(known);
        match &wanted {
            Ty::Function(callback) => {
                if trailer.params.len() != callback.params.len() {
                    let at = trailer.params.first().map_or(Span::default(), |p| p.span);
                    self.error(
                        at,
                        format!(
                            "{label}: the callback takes {} parameter{}, but the block names {}",
                            callback.params.len(),
                            if callback.params.len() == 1 { "" } else { "s" },
                            trailer.params.len()
                        ),
                    );
                }
                let locals = trailer
                    .params
                    .iter()
                    .zip(&callback.params)
                    .map(|(name, param)| Local {
                        name: name.name.clone(),
                        span: name.span,
                        ty: param.ty.clone(),
                    })
                    .collect();
                let ret = (*callback.ret).clone();
                let unknown = ret.has_unknown(generics, known);
                let declared = (!unknown).then(|| ret.clone());
                let found = self.function_body(locals, &trailer.body, declared);
                if unknown && !ret.unify(&found, generics, known) {
                    self.error(
                        trailer.body.first().map_or(Span::default(), |s| s.span),
                        format!("the block gives {found}, but the callback gives {ret}"),
                    );
                }
            }
            Ty::Array(item) => {
                if let Some(name) = trailer.params.first() {
                    self.error(
                        name.span,
                        "an Array block gives values, it has no names between | |",
                    );
                }
                let values: Vec<&Expr> = match trailer.body.as_slice() {
                    [
                        Stmt {
                            kind: StmtKind::Expr(expr),
                            ..
                        },
                    ] => match &expr.kind {
                        ExprKind::Array(items) => items.iter().collect(),
                        _ => vec![expr],
                    },
                    _ => {
                        let at = trailer.body.first().map_or(Span::default(), |s| s.span);
                        self.error(at, "an Array block is a list of values separated by commas");
                        Vec::new()
                    }
                };
                for value in values {
                    self.argument_for(value, item, generics, known);
                }
            }
            Ty::Defined {
                kind: DefinedKind::Struct,
                ..
            } => self.struct_block(Span::default(), &wanted, trailer, generics, known),
            Ty::Error => self.scoped_statements(&trailer.body),
            other => {
                let at = trailer.body.first().map_or(Span::default(), |s| s.span);
                self.error(
                    at,
                    format!(
                        "the last parameter is {other}, which cannot be written as a block: pass it inside the parentheses"
                    ),
                );
            }
        }
    }

    /// Statements checked in a scope of their own, for a block that could not be given a meaning.
    fn scoped_statements(&mut self, body: &[Stmt]) {
        self.scopes.push(declared_names(body));
        self.statements(body);
        self.scopes.pop();
    }

    // ---- names that stand for functions

    /// A name that stands for a function, used as a value. `expected` is the type the place wants, if it wants one.
    ///
    /// - A place that wants a function gets the function itself. A generic function takes its type arguments from the
    ///   type the place wants.
    /// - A name is never a call: `echo` is the function, `echo()` calls it.
    pub(super) fn function_name(
        &mut self,
        name: &ast::Ident,
        f: FnTy,
        expected: Option<&Ty>,
    ) -> Ty {
        if !f.generics.is_empty() {
            if let Some(Ty::Function(want)) = expected {
                let mut known = HashMap::new();
                let fits = f.params.len() == want.params.len()
                    && f.params
                        .iter()
                        .zip(&want.params)
                        .all(|(p, w)| p.ty.unify(&w.ty, &f.generics, &mut known))
                    && f.ret.unify(&want.ret, &f.generics, &mut known);
                let whole = Ty::Function(f.clone());
                if fits && !whole.has_unknown(&f.generics, &known) {
                    return whole.substitute(&known);
                }
            }
            self.error(
                name.span,
                format!(
                    "`{}` is generic and has no single type: write {}<T> with the types filled in",
                    name.name, name.name
                ),
            );
            return Ty::Error;
        }
        Ty::Function(f)
    }

    /// `identity_of<i32>`: a generic function with its type arguments filled in.
    pub(super) fn type_apply(&mut self, expr: &Expr, target: &Expr, type_args: &[ast::Type]) -> Ty {
        // the function itself, not a call of it: the name of a generic function has no value until its types are filled in
        let ty = match &target.kind {
            ExprKind::Name(name) => match self.scopes.lookup(&name.name) {
                Some(symbol) => {
                    self.symbol_of.insert(target.id, symbol);
                    self.scopes.symbol(symbol).ty.clone()
                }
                None => self.expr(target, None),
            },
            _ => self.expr(target, None),
        };
        let written: Vec<Ty> = type_args.iter().map(|t| self.resolve_type(t)).collect();
        if let Ty::TypeValue { id, name, kind, .. } = &ty {
            let expected = self.def(*id).generics.len();
            if expected != written.len() {
                self.error(
                    expr.span,
                    format!(
                        "{name} has {expected} type parameter{}, but {} type argument{} given",
                        if expected == 1 { "" } else { "s" },
                        written.len(),
                        if written.len() == 1 { " was" } else { "s were" },
                    ),
                );
                return Ty::Error;
            }
            return Ty::TypeValue {
                id: *id,
                name: name.clone(),
                kind: *kind,
                args: written,
            };
        }
        let Ty::Function(f) = ty else {
            if !ty.is_error() {
                self.error(expr.span, format!("{ty} has no type parameters"));
            }
            return Ty::Error;
        };
        if f.generics.len() != written.len() {
            self.error(
                expr.span,
                format!(
                    "this function has {} type parameter{}, but {} type argument{} given",
                    f.generics.len(),
                    if f.generics.len() == 1 { "" } else { "s" },
                    written.len(),
                    if written.len() == 1 { " was" } else { "s were" },
                ),
            );
            return Ty::Error;
        }
        let known: HashMap<String, Ty> = f.generics.iter().cloned().zip(written).collect();
        Ty::Function(f).substitute(&known)
    }
}

// ---- does every path return?

/// Does running these statements always end in a `return`?
fn terminates(body: &[Stmt], exhaustive: &HashSet<NodeId>) -> bool {
    body.iter().any(|stmt| match &stmt.kind {
        StmtKind::Expr(expr) => expr_terminates(expr, exhaustive),
        StmtKind::Block(inner) => terminates(inner, exhaustive),
        _ => false,
    })
}

fn head(expr: &Expr) -> Option<(&str, Option<&Trailer>)> {
    let ExprKind::Call {
        callee, trailer, ..
    } = &expr.kind
    else {
        return None;
    };
    let ExprKind::Name(name) = &callee.kind else {
        return None;
    };
    Some((name.name.as_str(), trailer.as_ref()))
}

fn expr_terminates(expr: &Expr, exhaustive: &HashSet<NodeId>) -> bool {
    if let ExprKind::Chain(segments) = &expr.kind {
        let heads: Vec<_> = segments.iter().filter_map(head).collect();
        // a try ends in whichever way it ran: its block and its catch must end, or the finally ends
        if heads.first().is_some_and(|(name, _)| *name == "try") {
            let ends = |name: &str| {
                heads
                    .iter()
                    .find(|(h, _)| *h == name)
                    .map(|(_, t)| t.is_some_and(|t| terminates(&t.body, exhaustive)))
            };
            let finally_ends = ends("finally") == Some(true);
            let try_ends = ends("try") == Some(true);
            let catch_ends = ends("catch").unwrap_or(true);
            return finally_ends || (try_ends && catch_ends);
        }
        // an if with an else: every branch must return. A loop with an else can be left by running out.
        let has_else = heads.iter().any(|(name, _)| *name == "else");
        let all_if = heads.first().is_some_and(|(name, _)| *name == "if");
        return all_if
            && has_else
            && heads
                .iter()
                .all(|(_, trailer)| trailer.is_some_and(|t| terminates(&t.body, exhaustive)));
    }
    let Some((name, trailer)) = head(expr) else {
        return false;
    };
    match name {
        "return" | "throw" => true,
        // a loop with no condition ends only by a break, and with no break it does not end
        "loop" => trailer.is_some_and(|t| !contains_break(&t.body)),
        "switch" => {
            let Some(trailer) = trailer else { return false };
            let cases: Vec<_> = trailer
                .body
                .iter()
                .filter_map(|stmt| match &stmt.kind {
                    StmtKind::Expr(e) => head(e),
                    _ => None,
                })
                .collect();
            cases.iter().any(|(name, _)| *name == "default")
                && cases
                    .iter()
                    .all(|(_, t)| t.is_some_and(|t| terminates(&t.body, exhaustive)))
        }
        // a match that handles every variant ends in whichever case runs
        "match" => {
            let Some(trailer) = trailer else { return false };
            exhaustive.contains(&expr.id)
                && trailer
                    .body
                    .iter()
                    .filter_map(|stmt| match &stmt.kind {
                        StmtKind::Expr(e) => head(e),
                        _ => None,
                    })
                    .all(|(_, t)| t.is_some_and(|t| terminates(&t.body, exhaustive)))
        }
        _ => false,
    }
}

fn contains_break(body: &[Stmt]) -> bool {
    fn in_expr(expr: &Expr) -> bool {
        if let ExprKind::Chain(segments) = &expr.kind {
            return segments.iter().any(in_expr);
        }
        match head(expr) {
            Some(("break", _)) => true,
            Some((_, Some(trailer))) => contains_break(&trailer.body),
            _ => false,
        }
    }
    body.iter().any(|stmt| match &stmt.kind {
        StmtKind::Expr(expr) => in_expr(expr),
        StmtKind::Block(inner) => contains_break(inner),
        _ => false,
    })
}
