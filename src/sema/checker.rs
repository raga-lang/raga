//! The checker: its state, statements, declarations and assignment.

use std::collections::{HashMap, HashSet};

use super::attributes;
use super::builtins::{self, Reserved};
use super::defined::{self, TypeDef, builtin_types};
use super::functions::{self, FnCtx};
use super::scope::{Binding, Constant, Defines, Scopes, Symbol};
use super::ty::{DefinedKind, FloatTy, FnTy, IntTy, ParamTy, Ty, TypeId};
use super::{Analysis, SymbolId};
use crate::ast::{self, Assign, Decl, DeclKind, Expr, ExprKind, NodeId, Program, Stmt, StmtKind};
use crate::diagnostics::Diagnostic;
use crate::span::Span;

pub(super) struct Checker {
    pub(super) scopes: Scopes,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) expr_types: HashMap<NodeId, Ty>,
    pub(super) symbol_of: HashMap<NodeId, SymbolId>,
    /// How many loops surround the statement being checked, which `break` and `continue` need.
    pub(super) loops: usize,
    /// The type parameters in scope, one list for each generic function being checked.
    pub(super) type_params: Vec<Vec<String>>,
    /// The functions being checked, the innermost last, for their `return` statements.
    pub(super) fns: Vec<FnCtx>,
    /// Functions whose return type is being worked out from their body: they cannot call themselves yet.
    pub(super) pending: HashSet<SymbolId>,
    pub(super) bare_calls: HashSet<NodeId>,
    /// The types the program defines, and `Optional` and `Result`.
    pub(super) types: Vec<TypeDef>,
    /// The type of `this`, set while the body of a method is about to be read.
    pub(super) method_this: Option<Ty>,
    /// The `match` statements that handle every variant, which decides whether a function may end in one.
    pub(super) exhaustive: HashSet<NodeId>,
    /// How many `finally` blocks surround the statement being checked: a `return` in one is an error.
    pub(super) in_finally: usize,
    /// The classes whose bodies are being read, the innermost last: who may use a private or protected member.
    pub(super) class_stack: Vec<TypeId>,
    /// The type of `super` for the method about to be read, set like `method_this`.
    pub(super) method_super: Option<Ty>,
    /// True while the body of a static member is read, which has no `this`.
    pub(super) in_static: bool,
    /// Set when a static field of a class is used, so that an accessor can be told to be static.
    pub(super) touched_static: bool,
    /// The attributes the program declares, by the index `Defines::Attribute` holds.
    pub(super) attributes: Vec<super::attributes::AttributeDef>,
    /// What `[deprecated("...")]` says about a declaration, to warn where it is used.
    /// The file being checked, which `import` paths are written from.
    pub(super) file: Option<std::path::PathBuf>,
    /// The files imported so far, by the number `Ty::Module` holds.
    pub(super) modules: Vec<super::modules::ModuleInfo>,
    pub(super) loaded: HashMap<std::path::PathBuf, usize>,
    /// The files being read right now, the innermost last: a file that is in it cannot be imported again.
    pub(super) loading: Vec<std::path::PathBuf>,
    pub(super) reports: Vec<super::modules::ModuleReport>,
    /// What the `export` of the file says, once it was read.
    pub(super) exported: Option<Vec<(crate::ast::Ident, SymbolId)>>,
    pub(super) operators: Vec<super::operators::OperatorDef>,
    pub(super) deprecated: HashMap<super::scope::SymbolId, String>,
}

impl Checker {
    pub(super) fn new(program: &Program) -> Checker {
        Checker {
            scopes: Scopes::new(declared_names(&program.stmts)),
            diagnostics: Vec::new(),
            expr_types: HashMap::new(),
            symbol_of: HashMap::new(),
            loops: 0,
            type_params: Vec::new(),
            fns: Vec::new(),
            pending: HashSet::new(),
            bare_calls: HashSet::new(),
            types: builtin_types(),
            method_this: None,
            exhaustive: HashSet::new(),
            in_finally: 0,
            class_stack: Vec::new(),
            method_super: None,
            in_static: false,
            touched_static: false,
            attributes: Vec::new(),
            deprecated: HashMap::new(),
            operators: Vec::new(),
            file: None,
            modules: Vec::new(),
            loaded: HashMap::new(),
            loading: Vec::new(),
            reports: Vec::new(),
            exported: None,
        }
    }

    pub(super) fn run(mut self, program: &Program) -> Analysis {
        if let Some(path) = self.file.clone() {
            self.loading.push(path);
        }
        self.check_exports_layout(&program.stmts);
        self.statements(&program.stmts);
        Analysis {
            diagnostics: self.diagnostics,
            symbols: self.scopes.symbols,
            expr_types: self.expr_types,
            symbol_of: self.symbol_of,
            bare_calls: self.bare_calls,
            modules: self.reports,
        }
    }

    pub(super) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::error(span, message));
    }

    // ---- statements

    pub(super) fn statements(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.statement(stmt);
        }
    }

    fn statement(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Decl(decl) => self.declaration(stmt.id, decl, &stmt.attrs),
            StmtKind::Assign(assign) => self.assignment(assign),
            StmtKind::Expr(expr) => {
                self.expr(expr, None);
            }
            StmtKind::Block(stmts) => {
                self.scopes.push(declared_names(stmts));
                self.statements(stmts);
                self.scopes.pop();
            }
        }
    }

    // ---- declarations

    /// `const x = v`, `let x = v` and `Type x = v`.
    ///
    /// The value is checked *before* the name is declared, so in `const x = x + 1` the `x` on the right is not the
    /// one being declared: it is an earlier `x`, or an error.
    fn declaration(&mut self, id: NodeId, decl: &Decl, attrs: &[ast::Attribute]) {
        if decl.kind != DeclKind::Const || !attributes::is_attribute_definition(&decl.init) {
            self.plain_declaration(id, decl, attrs);
            if let Some(symbol) = self.scopes.lookup(&decl.name.name)
                && let Some(reason) =
                    attrs
                        .iter()
                        .find(|a| a.name.name == "deprecated")
                        .and_then(|a| match a.args.first().map(|a| &a.value.kind) {
                            Some(ExprKind::Str(text)) => Some(text.clone()),
                            _ => None,
                        })
            {
                self.deprecated.insert(symbol, reason);
            }
            let kind = self.declaration_kind(decl);
            self.check_attributes(attrs, kind);
            if attrs.iter().any(|a| a.name.name == "operator") {
                match self
                    .scopes
                    .lookup(&decl.name.name)
                    .map(|s| self.scopes.symbol(s).ty.clone())
                {
                    Some(Ty::Function(sig)) => {
                        self.register_operator(attrs, &sig, None, decl.name.span);
                    }
                    _ => self.error(
                        decl.name.span,
                        "[operator(...)] belongs above a function or a method",
                    ),
                }
            }
        } else {
            self.attribute_declaration(id, decl, attrs);
        }
    }

    fn plain_declaration(&mut self, id: NodeId, decl: &Decl, attrs: &[ast::Attribute]) {
        if !matches!(decl.kind, DeclKind::Typed(_)) && functions::is_definition(&decl.init) {
            self.function_declaration(id, decl);
            return;
        }
        if decl.kind == DeclKind::Const && defined::is_definition(&decl.init) {
            self.type_declaration(id, decl, attrs);
            return;
        }
        if decl.kind == DeclKind::Const && self.is_alias(&decl.init) {
            self.alias_declaration(id, decl);
            return;
        }
        let (binding, ty) = match &decl.kind {
            DeclKind::Typed(written) => {
                let ty = self.resolve_type(written);
                let got = self.expr(&decl.init, Some(&ty));
                self.expect_assignable(&ty, &got, &decl.init);
                (Binding::Typed, ty)
            }
            DeclKind::Const | DeclKind::Let => {
                let got = self.expr(&decl.init, None);
                let ty = self.must_have_value(got, &decl.init);
                let binding = if decl.kind == DeclKind::Const {
                    Binding::Const
                } else {
                    Binding::Let
                };
                (binding, ty)
            }
        };
        let constant = match decl.kind {
            DeclKind::Const => self.constant_of(&decl.init),
            _ => None,
        };
        self.declare(id, decl, binding, ty, constant);
    }

    /// `const name = function(...) ... { ... }`. The name is declared before the body is checked, so that the function can
    /// call itself, which is allowed when the type it gives back is written.
    fn function_declaration(&mut self, id: NodeId, decl: &Decl) {
        let binding = if decl.kind == DeclKind::Const {
            Binding::Const
        } else {
            Binding::Let
        };
        let ExprKind::Call {
            type_args,
            args,
            trailer: Some(trailer),
            ..
        } = &decl.init.kind
        else {
            // not a complete definition: the expression says what is wrong
            let got = self.expr(&decl.init, None);
            let ty = self.must_have_value(got, &decl.init);
            self.declare(id, decl, binding, ty, None);
            return;
        };
        // the head alone, to name the function; the reports of its mistakes come from checking the whole function below
        let reported = self.diagnostics.len();
        let generics: Vec<String> = type_args.iter().map(|t| t.name.name.clone()).collect();
        self.type_params.push(generics.clone());
        let (sig, _) = self.signature(args, trailer.ret.as_ref(), generics);
        self.type_params.pop();
        self.diagnostics.truncate(reported);

        let inferred = trailer.ret.is_none();
        let head = Ty::Function(FnTy {
            ret: Box::new(if inferred {
                Ty::Error
            } else {
                (*sig.ret).clone()
            }),
            ..sig
        });
        let symbol = self.declare(id, decl, binding, head, None);
        if let (Some(symbol), true) = (symbol, inferred) {
            self.pending.insert(symbol);
        }
        let got = self.expr(&decl.init, None);
        if let Some(symbol) = symbol {
            self.pending.remove(&symbol);
            if matches!(got, Ty::Function(_)) {
                self.scopes.symbols[symbol.0 as usize].ty = got;
            }
        }
    }

    /// The value an expression stands for while the program is built: a literal, `-` and a literal, or a `const` made
    /// from one.
    pub(super) fn constant_of(&self, expr: &Expr) -> Option<Constant> {
        match &expr.kind {
            ExprKind::Int(magnitude) => Some(Constant::Int {
                magnitude: *magnitude,
                negative: false,
            }),
            ExprKind::Char(c) => Some(Constant::Char(*c)),
            ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                operand,
            } => match operand.kind {
                ExprKind::Int(magnitude) => Some(Constant::Int {
                    magnitude,
                    negative: true,
                }),
                _ => None,
            },
            ExprKind::Name(name) => {
                let symbol = self.scopes.lookup(&name.name)?;
                self.scopes.symbol(symbol).constant
            }
            _ => None,
        }
    }

    /// The whole number, not negative, that an expression stands for while the program is built.
    pub(super) fn literal_value(&self, expr: &Expr) -> Option<u128> {
        match self.constant_of(expr)? {
            Constant::Int {
                magnitude,
                negative: false,
            } => Some(magnitude),
            _ => None,
        }
    }

    fn declare(
        &mut self,
        id: NodeId,
        decl: &Decl,
        binding: Binding,
        ty: Ty,
        constant: Option<Constant>,
    ) -> Option<SymbolId> {
        self.declare_with(id, decl, binding, ty, constant, None)
    }

    pub(super) fn declare_with(
        &mut self,
        id: NodeId,
        decl: &Decl,
        binding: Binding,
        ty: Ty,
        constant: Option<Constant>,
        defines: Option<Defines>,
    ) -> Option<SymbolId> {
        let name = &decl.name;
        // A built-in name cannot be declared again at the top level of a file, because the new name would hide the
        // built-in one for the whole file. Inside a block it only hides it there, which is allowed.
        if self.scopes.depth() == 0
            && let Some(kind) = builtins::reserved(&name.name)
        {
            let message = match kind {
                Reserved::Function => format!("`{}` is built in, choose another name", name.name),
                Reserved::Type => {
                    format!("`{}` is a built-in type, choose another name", name.name)
                }
            };
            self.error(name.span, message);
            return None;
        }
        let symbol = Symbol {
            name: name.name.clone(),
            binding,
            ty,
            span: name.span,
            constant,
            defines,
        };
        match self.scopes.declare(symbol) {
            Ok(symbol) => {
                self.symbol_of.insert(id, symbol);
                Some(symbol)
            }
            Err(_) => {
                self.error(
                    name.span,
                    format!("`{}` is already declared in this scope", name.name),
                );
                None
            }
        }
    }

    /// A value that is stored in a name must be something: a call that gives nothing back is not.
    fn must_have_value(&mut self, ty: Ty, value: &Expr) -> Ty {
        if ty == Ty::Void {
            self.error(
                value.span,
                "this gives no value, so it cannot be stored in a name",
            );
            return Ty::Error;
        }
        if let Ty::TypeValue { name, .. } = &ty {
            self.error(
                value.span,
                format!("{name} is a type, not a value: to make a value, call it"),
            );
            return Ty::Error;
        }
        ty
    }

    // ---- assignment

    fn assignment(&mut self, assign: &Assign) {
        let ExprKind::Name(name) = &assign.target.kind else {
            if let ExprKind::Member { object, name } = &assign.target.kind {
                self.member_assignment(assign, object, name);
            } else if let ExprKind::Deref(_) = &assign.target.kind {
                self.deref_assignment(assign);
            } else {
                if !matches!(assign.target.kind, ExprKind::Error) {
                    self.error(
                        assign.target.span,
                        "only a name, a field or `p.*` can be assigned",
                    );
                }
                self.expr(&assign.value, None);
            }
            return;
        };
        let Some(symbol) = self.scopes.lookup(&name.name) else {
            let message = if builtins::reserved(&name.name).is_some() {
                format!("`{}` is built in and cannot be assigned", name.name)
            } else if self.scopes.declared_below(&name.name) {
                self.declared_below_message(&name.name)
            } else {
                format!(
                    "assignment needs a declaration: write `let {} = ...` (there is no `{}` here)",
                    name.name, name.name
                )
            };
            self.error(name.span, message);
            self.expr(&assign.value, None);
            return;
        };
        self.symbol_of.insert(assign.target.id, symbol);
        let target = self.scopes.symbol(symbol).clone();
        self.expr_types.insert(assign.target.id, target.ty.clone());
        if target.binding == Binding::Param {
            self.error(
                name.span,
                format!(
                    "`{}` is a parameter and cannot be assigned: copy it into a let, write let local = {}",
                    name.name, name.name
                ),
            );
        } else if target.binding == Binding::Const {
            self.error(
                name.span,
                format!(
                    "`{}` is const and cannot be assigned. Declare it with let to change it",
                    name.name
                ),
            );
        }
        match assign.op.binary() {
            None => {
                let got = self.expr(&assign.value, Some(&target.ty));
                self.expect_assignable(&target.ty, &got, &assign.value);
            }
            Some(op) => self.compound_assignment(assign, op, &target.ty),
        }
    }

    /// `p.x = v`: a field is changed. `const` protects a name, not the value it refers to, so the field of a const can change.
    fn member_assignment(&mut self, assign: &Assign, object: &Expr, name: &ast::Ident) {
        let receiver = match self.expr(object, None) {
            Ty::Ptr(inner) if matches!(*inner, Ty::Defined { .. }) => *inner,
            other => other,
        };
        let field = match &receiver {
            Ty::Defined { .. } => self.field_type_for(&receiver, name, true),
            Ty::TypeValue {
                kind: DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait,
                ..
            } => Some(self.class_static_member(&receiver, name, true)),
            Ty::Module(_) => {
                self.error(
                    name.span,
                    format!(
                        "{} belongs to the file that exports it: a module is read-only",
                        name.name
                    ),
                );
                None
            }
            Ty::Error => None,
            other => {
                self.error(
                    name.span,
                    format!("a value of type {other} has no field `{}`", name.name),
                );
                None
            }
        };
        let wanted = field.unwrap_or(Ty::Error);
        self.expr_types.insert(assign.target.id, wanted.clone());
        match assign.op.binary() {
            None => {
                let got = self.expr(&assign.value, Some(&wanted));
                self.expect_assignable(&wanted, &got, &assign.value);
            }
            Some(op) => self.compound_assignment(assign, op, &wanted),
        }
    }

    /// `x += v` is `x = x + v`: the value is checked against the type of `x`, and `+` must be allowed on that type.
    pub(super) fn compound_assignment(&mut self, assign: &Assign, op: ast::BinaryOp, target: &Ty) {
        let hint = Some(target).filter(|t| !matches!(t, Ty::Defined { .. }));
        let got = self.expr(&assign.value, hint);
        let span = assign.target.span.to(assign.value.span);
        if let Some(result) = self.user_binary(op, target, &got, span) {
            if !result.is_error() && !target.accepts(&result) {
                self.error(
                    span,
                    format!("the operator gives {result}, but {target} is assigned"),
                );
            }
            return;
        }
        if !target.accepts(&got) {
            self.error(
                assign.value.span,
                format!(
                    "{target} and {got}: both sides of `{}` must have the same type, convert one first",
                    assign.op.as_str()
                ),
            );
            return;
        }
        let span = assign.target.span.to(assign.value.span);
        self.operator_result(op, target, span);
    }

    pub(super) fn declared_below_message(&self, name: &str) -> String {
        format!("`{name}` is declared below, a name is visible only after its declaration")
    }

    // ---- types as written

    /// The type that an alias stands for, `const Id = i32`.
    fn alias_of(&self, name: &str) -> Option<Ty> {
        let symbol = self.scopes.lookup(name)?;
        match &self.scopes.symbol(symbol).defines {
            Some(Defines::Alias(ty)) => Some(ty.clone()),
            _ => None,
        }
    }

    /// The meaning of a type written in the program.
    pub(super) fn resolve_type(&mut self, written: &ast::Type) -> Ty {
        let name = written.name.name.as_str();
        let args: Vec<Ty> = written.args.iter().map(|a| self.resolve_type(a)).collect();
        let simple = |checker: &mut Checker, ty: Ty| {
            if !written.args.is_empty() {
                checker.error(written.span, format!("`{name}` takes no type arguments"));
            }
            ty
        };
        if name.contains('.') {
            return self.module_type(written, args);
        }
        if self
            .type_params
            .iter()
            .any(|scope| scope.iter().any(|p| p == name))
        {
            return simple(self, Ty::TypeParam(name.to_string()));
        }
        if name == "function" {
            let ret = written
                .ret
                .as_ref()
                .map_or(Ty::Void, |r| self.resolve_type(r));
            let params = args
                .into_iter()
                .map(|ty| ParamTy {
                    name: None,
                    ty,
                    has_default: false,
                    variadic: false,
                })
                .collect();
            return Ty::Function(FnTy {
                generics: Vec::new(),
                params,
                ret: Box::new(ret),
            });
        }
        if let Some(int) = IntTy::from_name(name) {
            return simple(self, Ty::Int(int));
        }
        if let Some(float) = FloatTy::from_name(name) {
            return simple(self, Ty::Float(float));
        }
        match name {
            "bool" => simple(self, Ty::Bool),
            "char" => simple(self, Ty::Char),
            "String" => simple(self, Ty::String),
            "Array" | "List" | "Set" | "Map" | "Tuple" => {
                self.generic_type(name, args, written.span)
            }
            "ptr" => {
                if let [pointee] = args.as_slice() {
                    return Ty::Ptr(Box::new(pointee.clone()));
                }
                self.error(written.span, "`ptr` takes one type: ptr<T>");
                Ty::Error
            }
            _ if self.type_named(name).is_some() || self.alias_of(name).is_some() => {
                if let Some(target) = self.alias_of(name) {
                    return simple(self, target);
                }
                let id = self.type_named(name).expect("checked above");
                let expected = self.def(id).generics.len();
                if args.len() != expected {
                    self.error(
                        written.span,
                        format!(
                            "`{name}` takes {expected} type argument{}: {name}<{}>",
                            if expected == 1 { "" } else { "s" },
                            self.def(id).generics.join(", ")
                        ),
                    );
                    return Ty::Error;
                }
                self.defined_ty(id, args)
            }
            _ if builtins::reserved(name).is_some() => {
                self.error(
                    written.span,
                    format!("the type `{name}` is not checked yet"),
                );
                Ty::Error
            }
            _ => {
                let message = if self.scopes.declared_below(name) {
                    format!(
                        "`{name}` is declared below: a type is visible only after its declaration"
                    )
                } else {
                    format!("unknown type `{name}`")
                };
                self.error(written.span, message);
                Ty::Error
            }
        }
    }

    // ---- compatibility

    /// Reports an error unless a value of type `got` can be used where `wanted` is needed. There is no conversion
    /// between types, so they must be the same.
    pub(super) fn expect_assignable(&mut self, wanted: &Ty, got: &Ty, value: &Expr) {
        if wanted.accepts(got) {
            return;
        }
        let message = if *got == Ty::Void {
            format!("this gives nothing, but {wanted} is needed")
        } else if matches!(
            wanted,
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::String
        ) {
            format!("{got} is not {wanted}, convert it first: {wanted}(...)")
        } else {
            format!("{got} is not {wanted}")
        };
        self.error(value.span, message);
    }
}

/// The names that a list of statements declares directly, so that a use above the declaration can be told from a name
/// that does not exist at all.
pub(super) fn declared_names(stmts: &[Stmt]) -> HashSet<String> {
    stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Decl(decl) => Some(decl.name.name.clone()),
            _ => None,
        })
        .collect()
}

/// Is the expression a number written out, whose type is not decided by what it is but by where it is used?
/// `5`, `-5` and `1 + 2 * 3` are; `x` and `u8(5)` are not.
pub(super) fn is_untyped(expr: &Expr) -> bool {
    use ast::{BinaryOp::*, UnaryOp};
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) => true,
        ExprKind::Unary {
            op: UnaryOp::Neg | UnaryOp::BitNot,
            operand,
        } => is_untyped(operand),
        ExprKind::Binary { op, lhs, rhs } => {
            matches!(
                op,
                Mul | Div | Rem | Add | Sub | Shl | Shr | BitAnd | BitXor | BitOr
            ) && is_untyped(lhs)
                && is_untyped(rhs)
        }
        _ => false,
    }
}
