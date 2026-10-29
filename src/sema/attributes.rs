//! Attributes and reflection (09-attributes-and-reflection): declaring an attribute, checking where it is used, and the
//! functions that read the program while it is built: `compile`, `typeof`, `reflect`, `attributes_of`, `get_field` and
//! `set_field`.

use super::checker::Checker;
use super::defined::{
    ANY, ATTRIBUTE_INFO, BINDING_INFO, FIELD_INFO, KIND, OPTIONAL, RESULT, TYPE_INFO,
};
use super::functions::{Local, local};
use super::scope::{Binding, Defines};
use super::ty::{FnTy, ParamTy, Ty};
use crate::ast::{Arg, Attribute, Decl, DeclKind, Expr, ExprKind, Ident, NodeId, Trailer, UnaryOp};

/// What an attribute carries and where it may be put.
#[derive(Debug, Clone)]
pub(super) struct AttributeDef {
    pub params: Vec<ParamTy>,
    /// The kinds of declaration it is limited to by `[on(...)]`, as the numbers of `Kind`. Empty: any.
    pub on: Vec<i128>,
}

/// The numbers of the members of `Kind`.
pub(super) const PRIMITIVE: i128 = 0;
pub(super) const STRUCT: i128 = 1;
pub(super) const ENUM: i128 = 2;
pub(super) const UNION: i128 = 3;
pub(super) const FUNCTION: i128 = 4;
pub(super) const FIELD: i128 = 5;
pub(super) const HEAP: i128 = 6;
pub(super) const METHOD: i128 = 7;

/// The names that belong to the language, so an attribute of your own cannot use them.
const LANGUAGE: &[&str] = &[
    "class",
    "method",
    "field",
    "accessor",
    "get",
    "set",
    "call",
    "on",
    "operator",
    "deprecated",
];

fn kind_words(kind: i128) -> &'static str {
    match kind {
        PRIMITIVE => "a variable of a primitive type",
        STRUCT => "a struct or a class",
        ENUM => "an enum",
        UNION => "a union",
        FUNCTION => "a function",
        FIELD => "a field",
        HEAP => "a variable of a non-primitive type",
        _ => "a method",
    }
}

/// Is this the declaration of an attribute: `attribute(...)`, with a call block after it or not?
pub(super) fn is_attribute_definition(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Chain(parts) => parts.first().is_some_and(is_attribute_definition),
        ExprKind::Call { callee, .. } => {
            matches!(&callee.kind, ExprKind::Name(n) if n.name == "attribute")
        }
        _ => false,
    }
}

fn is_primitive(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::Ptr(_)
    )
}

impl Checker {
    // ---- declaring

    /// `const page = attribute(String path) |target, path| { } call |target, args, next| { }`.
    pub(super) fn attribute_declaration(&mut self, id: NodeId, decl: &Decl, attrs: &[Attribute]) {
        let parts: Vec<&Expr> = match &decl.init.kind {
            ExprKind::Chain(parts) => parts.iter().collect(),
            _ => vec![&decl.init],
        };
        if decl.kind != DeclKind::Const {
            self.error(decl.name.span, "an attribute is declared with const");
        }
        if LANGUAGE.contains(&decl.name.name.as_str()) {
            self.error(
                decl.name.span,
                format!(
                    "`{}` belongs to the language: an attribute of your own needs another name",
                    decl.name.name
                ),
            );
        }
        let ExprKind::Call {
            args,
            trailer: build,
            ..
        } = &parts[0].kind
        else {
            return;
        };
        let (sig, _) = self.signature(args, None, Vec::new());
        let on = self.on_kinds(attrs);
        let index = self.attributes.len();
        self.attributes.push(AttributeDef {
            params: sig.params.clone(),
            on: on.clone(),
        });
        let any = self.defined_ty(ANY, Vec::new());
        let info = |checker: &Checker, kind: i128| {
            let id = match kind {
                FIELD => FIELD_INFO,
                STRUCT | ENUM | UNION => TYPE_INFO,
                _ => BINDING_INFO,
            };
            checker.defined_ty(id, Vec::new())
        };
        // the declaration the hooks are given: a field, a type or a binding, as `[on(...)]` says
        let target = match on.as_slice() {
            [kind] => info(self, *kind),
            [first, rest @ ..] if rest.iter().all(|k| info(self, *k) == info(self, *first)) => {
                info(self, *first)
            }
            _ => self.defined_ty(BINDING_INFO, Vec::new()),
        };

        // the build hook: target, then one name for each declared parameter
        if let Some(build) = build {
            let wanted = 1 + sig.params.len();
            if build.params.len() != wanted {
                let names: Vec<&str> = sig
                    .params
                    .iter()
                    .filter_map(|p| p.name.as_deref())
                    .collect();
                let list = if names.is_empty() {
                    "target".to_string()
                } else {
                    format!("target and {}", names.join(", "))
                };
                self.error(
                    parts[0].span,
                    format!(
                        "the block names {} parameter{}, it takes {list}",
                        build.params.len(),
                        if build.params.len() == 1 { "" } else { "s" }
                    ),
                );
            }
            let mut locals: Vec<Local> = Vec::new();
            for (i, name) in build.params.iter().enumerate() {
                let ty = match i {
                    0 => target.clone(),
                    _ => sig.params.get(i - 1).map_or(Ty::Error, |p| {
                        if p.variadic {
                            Ty::Array(Box::new(p.ty.clone()))
                        } else {
                            p.ty.clone()
                        }
                    }),
                };
                locals.push(local(&name.name, name.span, ty));
            }
            self.function_body(locals, &build.body, Some(Ty::Void));
        }

        // the call hook
        let mut calls = 0;
        for part in &parts[1..] {
            let ExprKind::Call {
                callee,
                trailer: Some(hook),
                ..
            } = &part.kind
            else {
                self.error(part.span, "only a call block can follow an attribute");
                continue;
            };
            if !matches!(&callee.kind, ExprKind::Name(n) if n.name == "call") {
                self.error(part.span, "only a call block can follow an attribute");
                continue;
            }
            calls += 1;
            if calls > 1 {
                self.error(part.span, "an attribute has one call block");
                continue;
            }
            self.call_hook(part, hook, &on, &target, &any);
        }
        let ty = Ty::Error;
        let _ = self.declare_with(
            id,
            decl,
            Binding::Const,
            ty,
            None,
            Some(Defines::Attribute(index)),
        );
        self.check_attributes_except_on(attrs, FUNCTION);
    }

    fn call_hook(&mut self, part: &Expr, hook: &Trailer, on: &[i128], target: &Ty, any: &Ty) {
        let on_method = on == [METHOD];
        let wanted = if on_method { 4 } else { 3 };
        if hook.params.len() != wanted {
            let message = if on_method {
                "a call block on a method takes target, receiver, args and next"
            } else {
                "a call block takes target, args and next"
            };
            self.error(part.span, message);
        }
        let next = Ty::Function(FnTy {
            generics: Vec::new(),
            params: vec![ParamTy {
                name: None,
                ty: Ty::Array(Box::new(any.clone())),
                has_default: false,
                variadic: false,
            }],
            ret: Box::new(any.clone()),
        });
        let mut types = vec![if on_method {
            self.defined_ty(BINDING_INFO, Vec::new())
        } else {
            target.clone()
        }];
        if on_method {
            types.push(any.clone());
        }
        types.push(Ty::Array(Box::new(any.clone())));
        types.push(next);
        let locals: Vec<Local> = hook
            .params
            .iter()
            .enumerate()
            .map(|(i, name)| {
                local(
                    &name.name,
                    name.span,
                    types.get(i).cloned().unwrap_or(Ty::Error),
                )
            })
            .collect();
        if hook.body.is_empty() {
            if hook.params.len() == wanted {
                self.error(
                    part.span,
                    "a call block gives back the value the caller receives: return next(args)",
                );
            }
            self.function_body(locals, &hook.body, None);
            return;
        }
        self.function_body(locals, &hook.body, Some(any.clone()));
    }

    /// The numbers of the kinds that `[on(Kind.Function, ...)]` lists.
    fn on_kinds(&mut self, attrs: &[Attribute]) -> Vec<i128> {
        let mut kinds = Vec::new();
        for attr in attrs.iter().filter(|a| a.name.name == "on") {
            for arg in &attr.args {
                match self.kind_member(&arg.value) {
                    Some(k) => kinds.push(k),
                    None => self.error(
                        arg.value.span,
                        "[on(...)] takes members of Kind, like Kind.Function",
                    ),
                }
            }
        }
        kinds
    }

    fn kind_member(&self, expr: &Expr) -> Option<i128> {
        let ExprKind::Member { object, name } = &expr.kind else {
            return None;
        };
        let ExprKind::Name(owner) = &object.kind else {
            return None;
        };
        if owner.name != "Kind" || self.scopes.lookup("Kind").is_some() {
            return None;
        }
        self.def(KIND)
            .members
            .iter()
            .find(|(n, _)| *n == name.name)
            .map(|(_, v)| *v)
    }

    // ---- using

    /// Checks the attributes above a declaration of this kind (a number of `Kind`).
    pub(super) fn check_attributes(&mut self, attrs: &[Attribute], kind: i128) {
        self.check_attributes_except_on(attrs, kind);
    }

    fn check_attributes_except_on(&mut self, attrs: &[Attribute], kind: i128) {
        for attr in attrs {
            match attr.name.name.as_str() {
                // these are read where the declaration is
                "class" | "method" | "field" | "accessor" | "get" | "set" | "on" | "operator" => {}
                "deprecated" => self.deprecated_use(attr),
                _ => self.attribute_use(attr, kind),
            }
        }
    }

    fn deprecated_use(&mut self, attr: &Attribute) {
        match attr.args.as_slice() {
            [arg] if matches!(arg.value.kind, ExprKind::Str(_)) => {}
            _ => self.error(
                attr.span,
                "[deprecated(...)] takes the reason, as one String: [deprecated(\"use other\")]",
            ),
        }
    }

    fn attribute_use(&mut self, attr: &Attribute, kind: i128) {
        let name = &attr.name;
        let Some(symbol) = self.scopes.lookup(&name.name) else {
            self.undeclared(name);
            for arg in &attr.args {
                self.expr(&arg.value, None);
            }
            return;
        };
        let Some(Defines::Attribute(index)) = self.scopes.symbol(symbol).defines.clone() else {
            self.error(name.span, format!("`{}` is not an attribute", name.name));
            return;
        };
        let def = self.attributes[index].clone();
        if !def.on.is_empty() && !def.on.contains(&kind) {
            let list: Vec<&str> = def.on.iter().map(|k| kind_words(*k)).collect();
            self.error(
                name.span,
                format!("{} can only be put on {}", name.name, list.join(" or ")),
            );
        }
        let sig = FnTy {
            generics: Vec::new(),
            params: def.params.clone(),
            ret: Box::new(Ty::Void),
        };
        let call = Expr {
            id: NodeId(u32::MAX),
            kind: ExprKind::Error,
            span: attr.span,
        };
        let before = self.diagnostics.len();
        self.call_function(&call, &name.name, &sig, &[], &attr.args, None);
        if self.diagnostics.len() == before {
            for arg in &attr.args {
                if !self.known_at_build(&arg.value) {
                    self.error(
                        arg.value.span,
                        "an attribute argument must be known at compile time: a literal, a const made from one, or an enum member",
                    );
                }
            }
        }
    }

    /// Is the value known while the program is built?
    fn known_at_build(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::Null => true,
            ExprKind::Unary { operand, .. } => self.known_at_build(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.known_at_build(lhs) && self.known_at_build(rhs)
            }
            ExprKind::Array(items) | ExprKind::List(items) | ExprKind::Tuple(items) => {
                items.iter().all(|i| self.known_at_build(i))
            }
            ExprKind::Name(name) => self
                .scopes
                .lookup(&name.name)
                .is_some_and(|s| self.scopes.symbol(s).binding == Binding::Const),
            ExprKind::Member { object, .. } => {
                matches!(&object.kind, ExprKind::Name(n) if self.type_named(&n.name).is_some())
            }
            ExprKind::Call { callee, .. } => {
                matches!(&callee.kind, ExprKind::Name(n) if n.name == "compile")
            }
            _ => false,
        }
    }

    /// The kind a declaration has, for `[on(...)]`.
    pub(super) fn declaration_kind(&self, decl: &Decl) -> i128 {
        use super::{defined, functions};
        if functions::is_definition(&decl.init) {
            return FUNCTION;
        }
        if defined::is_definition(&decl.init) {
            let head = match &decl.init.kind {
                ExprKind::Chain(parts) => parts.first(),
                _ => Some(&decl.init),
            };
            if let Some(Expr {
                kind: ExprKind::Call { callee, .. },
                ..
            }) = head
                && let ExprKind::Name(n) = &callee.kind
            {
                return match n.name.as_str() {
                    "enum" => ENUM,
                    "union" => UNION,
                    _ => STRUCT,
                };
            }
            return STRUCT;
        }
        match self
            .scopes
            .lookup(&decl.name.name)
            .map(|s| self.scopes.symbol(s).ty.clone())
        {
            Some(ty) if !is_primitive(&ty) && !ty.is_error() => HEAP,
            _ => PRIMITIVE,
        }
    }

    // ---- reading the program

    /// `compile(expression)`: the expression runs while the program is built, so it can only use what is known then.
    pub(super) fn compile_call(&mut self, call: &Expr, args: &[Arg]) -> Ty {
        let [arg] = args else {
            self.error(
                call.span,
                "compile takes one expression: compile(expression)",
            );
            self.check_arguments(args);
            return Ty::Error;
        };
        self.needs_only_build_time(&arg.value);
        self.expr(&arg.value, None)
    }

    fn needs_only_build_time(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                if let ExprKind::Name(n) = &callee.kind {
                    if matches!(
                        n.name.as_str(),
                        "print" | "read_line" | "sleep" | "alloc" | "free"
                    ) && self.scopes.lookup(&n.name).is_none()
                    {
                        self.error(
                            n.span,
                            format!(
                                "{} needs the running program: compile runs while the program is built",
                                n.name
                            ),
                        );
                    }
                } else {
                    self.needs_only_build_time(callee);
                }
                for a in args {
                    self.needs_only_build_time(&a.value);
                }
            }
            ExprKind::Name(name) => {
                if let Some(symbol) = self.scopes.lookup(&name.name)
                    && matches!(
                        self.scopes.symbol(symbol).binding,
                        Binding::Let | Binding::Typed
                    )
                {
                    self.error(
                        name.span,
                        format!(
                            "`{}` is a variable of the running program: compile can use only consts",
                            name.name
                        ),
                    );
                }
            }
            ExprKind::Unary { op, operand } => {
                if *op == UnaryOp::AddrOf {
                    self.error(
                        expr.span,
                        "a pointer to a variable needs the running program: compile runs while the program is built",
                    );
                }
                self.needs_only_build_time(operand);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.needs_only_build_time(lhs);
                self.needs_only_build_time(rhs);
            }
            ExprKind::Member { object, .. } | ExprKind::Deref(object) => {
                self.needs_only_build_time(object);
            }
            ExprKind::Array(items) | ExprKind::List(items) | ExprKind::Tuple(items) => {
                for item in items {
                    self.needs_only_build_time(item);
                }
            }
            _ => {}
        }
    }

    /// `typeof(value)`, `reflect<T>()`, `attributes_of(thing, name)`, `get_field(p, name)` and `set_field(p, name, value)`.
    pub(super) fn reflection_call(
        &mut self,
        call: &Expr,
        name: &Ident,
        type_args: &[crate::ast::Type],
        args: &[Arg],
    ) -> Ty {
        let text = name.name.as_str();
        let any = self.defined_ty(ANY, Vec::new());
        let count = |checker: &mut Checker, wanted: usize, usage: &str| {
            if args.len() == wanted {
                return true;
            }
            checker.error(call.span, format!("{text} takes {usage}"));
            checker.check_arguments(args);
            false
        };
        if text != "reflect" && !type_args.is_empty() {
            self.error(call.span, format!("{text} takes no type argument"));
        }
        match text {
            "typeof" => {
                if !count(self, 1, "one value: typeof(value)") {
                    return Ty::String;
                }
                self.expr(&args[0].value, None);
                Ty::String
            }
            "reflect" => {
                let [written] = type_args else {
                    self.error(call.span, "reflect takes one type: reflect<T>()");
                    self.check_arguments(args);
                    return Ty::Error;
                };
                self.resolve_type(written);
                if !args.is_empty() {
                    self.error(call.span, "reflect takes no arguments: reflect<T>()");
                    self.check_arguments(args);
                }
                self.defined_ty(TYPE_INFO, Vec::new())
            }
            "attributes_of" => {
                let info = Ty::Array(Box::new(self.defined_ty(ATTRIBUTE_INFO, Vec::new())));
                if !count(
                    self,
                    2,
                    "the declaration and the name: attributes_of(thing, \"name\")",
                ) {
                    return info;
                }
                self.attribute_subject(&args[0].value);
                let got = self.expr(&args[1].value, Some(&Ty::String));
                if !Ty::String.accepts(&got) {
                    self.error(
                        args[1].value.span,
                        format!("the name is a String, but this is {got}"),
                    );
                }
                info
            }
            "get_field" | "set_field" => {
                let wanted = if text == "get_field" { 2 } else { 3 };
                let usage = if text == "get_field" {
                    "the pointer and the name: get_field(p, \"name\")"
                } else {
                    "the pointer, the name and the value: set_field(p, \"name\", Any(value))"
                };
                let result = if text == "get_field" {
                    self.defined_ty(OPTIONAL, vec![any.clone()])
                } else {
                    self.defined_ty(RESULT, vec![Ty::Bool, Ty::String])
                };
                if !count(self, wanted, usage) {
                    return result;
                }
                let pointer = self.expr(&args[0].value, None);
                if !matches!(pointer, Ty::Ptr(_) | Ty::Error) {
                    self.error(
                        args[0].value.span,
                        format!(
                            "{text} needs a pointer to the struct, &variable, but this is {pointer}"
                        ),
                    );
                }
                let got = self.expr(&args[1].value, Some(&Ty::String));
                if !Ty::String.accepts(&got) {
                    self.error(
                        args[1].value.span,
                        format!("the name is a String, but this is {got}"),
                    );
                }
                if text == "set_field" {
                    let value = self.expr(&args[2].value, Some(&any));
                    if !any.accepts(&value) {
                        self.error(
                            args[2].value.span,
                            format!("{value} is not Any: box it, Any(value)"),
                        );
                    }
                }
                result
            }
            _ => Ty::Error,
        }
    }

    /// The first argument of `attributes_of`: a declaration named in the code, or a value that describes one.
    fn attribute_subject(&mut self, expr: &Expr) {
        if let ExprKind::Name(name) = &expr.kind
            && let Some(symbol) = self.scopes.lookup(&name.name)
        {
            let symbol = self.scopes.symbol(symbol);
            if matches!(symbol.ty, Ty::Function(_) | Ty::TypeValue { .. })
                || symbol.defines.is_some()
            {
                self.symbol_of.insert(
                    expr.id,
                    self.scopes.lookup(&name.name).expect("found above"),
                );
                return;
            }
        }
        let got = self.expr(expr, None);
        let described = matches!(
            &got,
            Ty::Defined { id, .. } if matches!(*id, i if i == TYPE_INFO || i == BINDING_INFO || i == FIELD_INFO)
        ) || got.is_error()
            || matches!(got, Ty::TypeValue { .. });
        if !described {
            self.error(
                expr.span,
                format!("{got} has no attributes: name a function or a type, or give a TypeInfo, a BindingInfo or a FieldInfo"),
            );
        }
    }

    /// `Any(value)`: a box that holds a value of any type.
    pub(super) fn construct_any(&mut self, call: &Expr, args: &[Arg]) -> Ty {
        let [arg] = args else {
            self.error(call.span, "Any takes the value it holds: Any(value)");
            self.check_arguments(args);
            return Ty::Error;
        };
        let got = self.expr(&arg.value, None);
        if got == Ty::Void {
            self.error(
                arg.value.span,
                "this gives no value, so there is nothing to box",
            );
            return Ty::Error;
        }
        self.defined_ty(ANY, Vec::new())
    }

    /// `any.is<T>()` and `any.as<T>()`.
    pub(super) fn any_test(
        &mut self,
        call: &Expr,
        name: &Ident,
        written: &[crate::ast::Type],
        args: &[Arg],
    ) -> Option<Ty> {
        if !matches!(name.name.as_str(), "is" | "as") {
            return None;
        }
        let [written] = written else {
            self.error(
                call.span,
                format!("{} takes one type: any.{}<T>()", name.name, name.name),
            );
            self.check_arguments(args);
            return Some(Ty::Error);
        };
        let target = self.resolve_type(written);
        if !args.is_empty() {
            self.error(call.span, format!("{} takes no arguments", name.name));
            self.check_arguments(args);
        }
        Some(if name.name == "is" {
            Ty::Bool
        } else {
            self.defined_ty(OPTIONAL, vec![target])
        })
    }
}
