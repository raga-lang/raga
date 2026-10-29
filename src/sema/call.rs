//! Calls: conversions such as `u8(300)`, built-in functions, and the methods of a String.
//!
//! In Raga a type name called like a function converts a value: `i64(x)`, `String(42)`, `bool(0)`. That is the only
//! conversion there is, so it is the only place a number can change type.

use super::builtins::{self, Param, Ret, Sig};
use super::checker::{Checker, is_untyped};
use super::collections;
use super::defined::{IERROR, RESULT};
use super::format;
use super::forms;
use super::ty::{DefinedKind, FloatTy, IntTy, Ty};
use crate::ast::{self, Arg, Expr, ExprKind, Ident, Trailer};

impl Checker {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call(
        &mut self,
        call: &Expr,
        callee: &Expr,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
        bare: bool,
        expected: Option<&Ty>,
    ) -> Ty {
        if let ExprKind::Name(name) = &callee.kind {
            if name.name == "method" && self.class_stack.is_empty() {
                self.error(
                    name.span,
                    "`method` belongs inside a class: outside one, write function",
                );
            }
            if name.name == "function" || name.name == "method" {
                return self.function_value(call, type_args, args, trailer);
            }
            if forms::is_form(&name.name) {
                return self.form(call, &name.name, args, trailer, false);
            }
            if collections::is_collection_name(&name.name)
                && self.scopes.lookup(&name.name).is_none()
            {
                if trailer.is_some() {
                    self.error(
                        call.span,
                        format!(
                            "creating a {} takes its values inside the parentheses, no block",
                            name.name
                        ),
                    );
                    self.check_args(args);
                    return Ty::Error;
                }
                return self.construct(call, name, type_args, args, expected);
            }
        }
        match &callee.kind {
            ExprKind::Name(name) => {
                self.call_name(call, callee, name, type_args, args, trailer, bare, expected)
            }
            ExprKind::Member { object, name } => {
                self.call_member(call, object, name, type_args, args, trailer, expected)
            }
            _ => {
                // the result of a call, or of an index, that is a function: `style(true)("a", 1)`
                let ty = self.expr(callee, None);
                match ty {
                    Ty::Function(f) => {
                        self.call_function(call, "this function", &f, type_args, args, trailer)
                    }
                    Ty::Error => {
                        self.check_args(args);
                        Ty::Error
                    }
                    other => {
                        self.error(
                            callee.span,
                            format!("{other} is not a function, so it cannot be called"),
                        );
                        self.check_args(args);
                        Ty::Error
                    }
                }
            }
        }
    }

    /// Checks the arguments of a call that cannot be checked further, so that mistakes inside them are still found.
    pub(super) fn check_arguments(&mut self, args: &[Arg]) {
        self.check_args(args);
    }

    fn check_args(&mut self, args: &[Arg]) {
        for arg in args {
            self.expr(&arg.value, None);
        }
    }

    /// `name(args)`, `name arg`, `name { ... }`
    #[allow(clippy::too_many_arguments)]
    fn call_name(
        &mut self,
        call: &Expr,
        callee: &Expr,
        name: &Ident,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
        bare: bool,
        expected: Option<&Ty>,
    ) -> Ty {
        let text = name.name.as_str();
        // `super(...)` runs the constructor of the base class
        if text == "super" && self.scopes.lookup(text).is_some() {
            return self.super_call(call, args);
        }
        if let Some(symbol) = self.scopes.lookup(text) {
            self.symbol_of.insert(callee.id, symbol);
            if self.pending.contains(&symbol) {
                self.error(
                    name.span,
                    format!("`{text}` calls itself, so what it returns cannot be worked out: write the return type"),
                );
                self.check_args(args);
                return Ty::Error;
            }
            let ty = self.scopes.symbol(symbol).ty.clone();
            return match ty {
                Ty::Function(f) => self.call_function(call, text, &f, type_args, args, trailer),
                Ty::TypeValue { id, .. } => {
                    let explicit: Vec<Ty> =
                        type_args.iter().map(|t| self.resolve_type(t)).collect();
                    self.construct_defined(call, id, explicit, args, trailer, expected)
                }
                Ty::Error => {
                    self.check_args(args);
                    Ty::Error
                }
                other => {
                    self.error(name.span, format!("`{text}` is a {other}, not a function"));
                    self.check_args(args);
                    Ty::Error
                }
            };
        }
        if let Some(id) = self.type_named(text) {
            let explicit: Vec<Ty> = type_args.iter().map(|t| self.resolve_type(t)).collect();
            return self.construct_defined(call, id, explicit, args, trailer, expected);
        }
        if text == "len" && self.scopes.lookup(text).is_none() {
            return self.len_call(call, args);
        }
        if text == "import" && self.scopes.lookup(text).is_none() {
            return self.import_call(call, args, trailer);
        }
        if text == "export" && self.scopes.lookup(text).is_none() {
            return self.export_call(call, args, trailer);
        }
        if text == "compile" && self.scopes.lookup(text).is_none() {
            return self.compile_call(call, args);
        }
        if matches!(
            text,
            "typeof" | "reflect" | "attributes_of" | "get_field" | "set_field"
        ) && self.scopes.lookup(text).is_none()
        {
            return self.reflection_call(call, name, type_args, args);
        }
        if matches!(text, "alloc" | "free" | "sizeof" | "alignof") {
            return self.memory_call(call, name, type_args, args, trailer);
        }
        if text == "ptr" && type_args.len() == 1 {
            let target = Ty::Ptr(Box::new(self.resolve_type(&type_args[0])));
            return self.conversion(call, target, args);
        }
        if !type_args.is_empty() {
            self.error(
                call.span,
                "type arguments can only be given to Array, List, Set, Map, Tuple and generic functions",
            );
            self.check_args(args);
            return Ty::Error;
        }
        let is_builtin = IntTy::from_name(text).is_some()
            || FloatTy::from_name(text).is_some()
            || matches!(text, "bool" | "char" | "String")
            || builtins::global_function(text).is_some()
            || builtins::reserved(text).is_some();
        if trailer.is_some() && is_builtin {
            self.error(
                call.span,
                format!("`{text}` takes no block after its arguments"),
            );
            self.check_args(args);
            return Ty::Error;
        }
        let target = if let Some(int) = IntTy::from_name(text) {
            Some(Ty::Int(int))
        } else if let Some(float) = FloatTy::from_name(text) {
            Some(Ty::Float(float))
        } else {
            match text {
                "bool" => Some(Ty::Bool),
                "char" => Some(Ty::Char),
                "String" => Some(Ty::String),
                _ => None,
            }
        };
        if let Some(target) = target {
            // `u8 late` is not a conversion: a type, then a name, then nothing to give it a value
            if bare
                && let [Arg { value, .. }] = args
                && let ExprKind::Name(declared) = &value.kind
            {
                self.error(
                    declared.span,
                    format!(
                        "`{0}` needs a value: a binding must be initialized when it is declared, write `{text} {0} = ...`",
                        declared.name
                    ),
                );
                return target;
            }
            return self.conversion(call, target, args);
        }
        if let Some(sig) = builtins::global_function(text) {
            return self.apply(call, text, sig, args);
        }
        if text == "range" {
            self.error(
                name.span,
                "range gives the numbers to loop over, write it as the collection of a for: for (range(0, 3)) |i| { }",
            );
            self.check_args(args);
            return Ty::Error;
        }
        if builtins::reserved(text).is_some() {
            self.error(
                name.span,
                format!("`{text}` is built in, but calls to it are not checked yet"),
            );
            self.check_args(args);
            return Ty::Error;
        }
        self.undeclared(name);
        self.check_args(args);
        Ty::Error
    }

    /// `object.name(args)`: a function of `Math` or `String`, or a method of a String value.
    #[allow(clippy::too_many_arguments)]
    fn call_member(
        &mut self,
        call: &Expr,
        object: &Expr,
        name: &Ident,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
        expected: Option<&Ty>,
    ) -> Ty {
        let type_test = matches!(name.name.as_str(), "is" | "as") && type_args.len() == 1;
        if (!type_args.is_empty() && !type_test)
            || (trailer.is_some() && self.namespace(object).is_some())
        {
            self.error(
                call.span,
                "type arguments and a block are not accepted by this call",
            );
            self.check_args(args);
            return Ty::Error;
        }
        if let Some(namespace) = self.namespace(object) {
            let sig = match namespace {
                "Math" => builtins::math_function(&name.name),
                "System" => builtins::system_function(&name.name),
                "File" => builtins::file_function(&name.name),
                "Time" => builtins::time_function(&name.name),
                "Random" => builtins::random_function(&name.name),
                _ => builtins::string_function(&name.name),
            };
            let label = format!("{namespace}.{}", name.name);
            return match sig {
                Some(sig) => self.apply(call, &label, sig, args),
                None => {
                    self.error(
                        name.span,
                        format!("`{namespace}` has no function `{}`", name.name),
                    );
                    self.check_args(args);
                    Ty::Error
                }
            };
        }
        let receiver = self.expr(object, None);
        match receiver {
            Ty::Error => {
                self.check_args(args);
                Ty::Error
            }
            Ty::String if trailer.is_some() => {
                self.error(
                    name.span,
                    "a String method takes no block after its arguments",
                );
                self.check_args(args);
                Ty::Error
            }
            Ty::String => match builtins::string_method(&name.name) {
                Some(sig) => self.apply(call, &name.name, sig, args),
                None => {
                    self.error(name.span, format!("String has no method `{}`", name.name));
                    self.check_args(args);
                    Ty::Error
                }
            },
            Ty::TypeValue {
                kind: DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait,
                ..
            } => self.class_static_call(call, &receiver, name, args, trailer),
            Ty::TypeValue {
                kind: DefinedKind::Struct | DefinedKind::RawUnion,
                ..
            } if name.name == "fromRaw" => self.static_from_raw(call, &receiver, args),
            Ty::TypeValue { .. } => self.construct_variant(call, &receiver, name, args, expected),
            Ty::Defined { .. } => {
                self.instance_call(call, &receiver, name, type_args, args, trailer)
            }
            Ty::Module(module) => {
                self.module_call(call, module, name, type_args, args, trailer, expected)
            }
            Ty::Tuple(slots) => self.tuple_call(call, &slots, name, args),
            Ty::Ptr(inner) => self.pointer_call(name, &inner, args),
            receiver @ (Ty::Array(_) | Ty::List(_) | Ty::Set(_) | Ty::Map(..)) => {
                self.collection_call(call, &receiver, name, args, trailer)
            }
            Ty::TypeParam(param) => {
                self.error(
                    name.span,
                    format!(
                        "{param} is a type parameter and has no member `{}`",
                        name.name
                    ),
                );
                self.check_args(args);
                Ty::Error
            }
            other => {
                self.error(
                    name.span,
                    format!("a value of type {other} has no method `{}` yet", name.name),
                );
                self.check_args(args);
                Ty::Error
            }
        }
    }

    /// `len(value)`: the same as `value.len()`.
    fn len_call(&mut self, call: &Expr, args: &[Arg]) -> Ty {
        let [arg] = args else {
            self.error(call.span, "len takes one value: len(items)");
            self.check_args(args);
            return Ty::USIZE;
        };
        let got = self.expr(&arg.value, None);
        if !matches!(
            got,
            Ty::String | Ty::Array(_) | Ty::List(_) | Ty::Set(_) | Ty::Map(..) | Ty::Error
        ) {
            self.error(
                arg.value.span,
                format!(
                    "{got} has no length: len takes a String, an Array, a List, a Set or a Map"
                ),
            );
        }
        Ty::USIZE
    }

    // ---- conversions

    /// `u8(x)`, `f64(x)`, `String(x)`: one value in, a value of the named type out.
    pub(super) fn conversion(&mut self, call: &Expr, target: Ty, args: &[Arg]) -> Ty {
        let [arg] = args else {
            self.error(
                call.span,
                format!(
                    "{target}(...) converts one value, but {} were given",
                    args.len()
                ),
            );
            self.check_args(args);
            return target;
        };
        if arg.name.is_some() {
            self.error(call.span, "a conversion does not take a named argument");
        }
        // The value is not asked for a type: `u8(300)` converts the i32 300, it is not a u8 that does not fit.
        let from = self.expr(&arg.value, None);
        if !can_convert(&from, &target) {
            self.error(arg.value.span, format!("cannot convert {from} to {target}"));
        }
        target
    }

    // ---- built-in functions

    /// Checks a call against the signature of a built-in function and gives its result type.
    pub(super) fn apply(&mut self, call: &Expr, label: &str, sig: Sig, args: &[Arg]) -> Ty {
        // the two ways to write text with values in it read their first argument as a format
        if label == "$" || label == "String.format" {
            return self.apply_format(call, label, args);
        }
        let fixed = sig.params.len();
        let count = args.len();
        if count < fixed || (sig.rest.is_none() && count > fixed) {
            let plural = if fixed == 1 { "" } else { "s" };
            let how = if sig.rest.is_some() { "at least " } else { "" };
            self.error(
                call.span,
                format!("{label} takes {how}{fixed} argument{plural}, but {count} given"),
            );
            self.check_args(args);
            return self.result_type(sig.ret, None);
        }
        if let Some(named) = args.iter().find(|a| a.name.is_some()) {
            self.error(
                named.value.span,
                "named arguments are not checked yet for built-in functions",
            );
        }
        let params: Vec<Param> = (0..count)
            .map(|i| {
                sig.params
                    .get(i)
                    .copied()
                    .or(sig.rest)
                    .expect("counted above")
            })
            .collect();

        // All the number arguments of one call have the same type. One that has a type of its own decides it, and a
        // literal among them then takes that type: Math.max(x, 1) with x an i64 asks for an i64 one.
        let mut number: Option<Ty> = None;
        let mut done: Vec<bool> = vec![false; count];
        for (i, arg) in args.iter().enumerate() {
            if params[i] == Param::Num && !is_untyped(&arg.value) {
                let ty = self.expr(&arg.value, None);
                self.unify_number(label, &mut number, ty, &arg.value);
                done[i] = true;
            }
        }
        for (i, arg) in args.iter().enumerate() {
            if done[i] {
                continue;
            }
            let value = &arg.value;
            match params[i] {
                Param::Num => {
                    let ty = self.expr(value, number.as_ref());
                    self.unify_number(label, &mut number, ty, value);
                }
                Param::F64 => self.argument(value, Ty::F64),
                Param::String => self.argument(value, Ty::String),
                Param::Usize => self.argument(value, Ty::USIZE),
                Param::I32 => self.argument(value, Ty::I32),
                Param::I64 => self.argument(value, Ty::Int(IntTy::I64)),
                Param::IError => {
                    let wanted = self.defined_ty(IERROR, Vec::new());
                    self.argument(value, wanted);
                }
                Param::Any => {
                    let ty = self.expr(value, None);
                    if ty == Ty::Void {
                        self.error(
                            value.span,
                            "this gives no value, so it cannot be an argument",
                        );
                    }
                }
            }
        }
        self.result_type(sig.ret, number)
    }

    /// `String.format(format, values...)` and `$(format, values...)`.
    ///
    /// When the format is a string literal it is read here and checked against the values: how many it takes, and the kind
    /// of value each place takes. A format that is not a literal cannot be checked, so its values are only checked to be
    /// values. The expected type of a place also decides the type of a literal given there: in `"%u"` the `5` is a `u32`.
    fn apply_format(&mut self, call: &Expr, label: &str, args: &[Arg]) -> Ty {
        let Some((format, values)) = args.split_first() else {
            self.error(
                call.span,
                format!("{label} takes at least 1 argument, but 0 given"),
            );
            return Ty::String;
        };
        if let Some(named) = args.iter().find(|a| a.name.is_some()) {
            self.error(
                named.value.span,
                "named arguments are not checked yet for built-in functions",
            );
        }
        self.argument(&format.value, Ty::String);

        let ExprKind::Str(text) = &format.value.kind else {
            for arg in values {
                self.any_value(&arg.value);
            }
            return Ty::String;
        };
        let places = match format::parse(text) {
            Ok(places) => places,
            Err(message) => {
                self.error(format.value.span, message);
                for arg in values {
                    self.any_value(&arg.value);
                }
                return Ty::String;
            }
        };
        if places.len() != values.len() {
            let (n, m) = (places.len(), values.len());
            self.error(
                call.span,
                format!(
                    "the format has {n} value{} but {m} {} given",
                    if n == 1 { "" } else { "s" },
                    if m == 1 { "was" } else { "were" }
                ),
            );
            for arg in values {
                self.any_value(&arg.value);
            }
            return Ty::String;
        }
        for (place, arg) in places.iter().zip(values) {
            self.format_value(place, &arg.value);
        }
        Ty::String
    }

    /// One value given to one place of a format.
    fn format_value(&mut self, place: &format::Place, value: &Expr) {
        use format::Kind;
        let spec = &place.spec;
        // the type a literal takes here, and what the place accepts
        let (hint, needs): (Option<Ty>, &str) = match place.kind {
            Kind::Signed => (Some(Ty::I32), "a signed integer"),
            Kind::Unsigned => (Some(Ty::Int(IntTy::U32)), "an unsigned integer"),
            Kind::Integer => (Some(Ty::I32), "an integer"),
            Kind::Star => (Some(Ty::I32), "a whole number"),
            Kind::Float => (Some(Ty::F64), "a decimal number"),
            Kind::Char => (Some(Ty::Char), "a char"),
            Kind::Str => (Some(Ty::String), "a String"),
            Kind::Pointer => (None, "a pointer"),
        };
        let got = self.expr(value, hint.as_ref());
        let fits = match (place.kind, &got) {
            (_, Ty::Error) => true,
            (Kind::Signed, Ty::Int(int)) => int.is_signed(),
            (Kind::Unsigned, Ty::Int(int)) => !int.is_signed(),
            (Kind::Integer | Kind::Star, Ty::Int(_)) => true,
            (Kind::Float, Ty::Float(_)) => true,
            (Kind::Char, Ty::Char) | (Kind::Str, Ty::String) => true,
            (Kind::Pointer, _) => {
                self.error(
                    value.span,
                    format!("`{spec}` takes a pointer, which is not checked yet"),
                );
                return;
            }
            _ => false,
        };
        if !fits {
            let message = if place.kind == format::Kind::Star {
                format!("the `*` in `{spec}` needs {needs}, but this is {got}")
            } else {
                format!("`{spec}` needs {needs}, but this is {got}")
            };
            self.error(value.span, message);
        }
    }

    /// An argument that can be any value, but not nothing.
    fn any_value(&mut self, value: &Expr) {
        let ty = self.expr(value, None);
        if ty == Ty::Void {
            self.error(
                value.span,
                "this gives no value, so it cannot be an argument",
            );
        }
    }

    /// An argument that must have a certain type; a literal takes it.
    pub(super) fn argument(&mut self, value: &Expr, wanted: Ty) {
        let got = self.expr(value, Some(&wanted));
        self.expect_assignable(&wanted, &got, value);
    }

    /// Records the type of a number argument, or reports it if it is not a number or differs from the others.
    fn unify_number(&mut self, label: &str, number: &mut Option<Ty>, ty: Ty, value: &Expr) {
        if !ty.is_number() && !ty.is_error() {
            self.error(
                value.span,
                format!("{label} needs a number, but this is {ty}"),
            );
            return;
        }
        match number {
            None => *number = Some(ty),
            Some(known) if !known.accepts(&ty) => {
                let known = known.clone();
                self.error(
                    value.span,
                    format!("{known} and {ty}: the arguments of {label} must have the same type, convert one first"),
                );
            }
            Some(_) => {}
        }
    }

    fn result_type(&self, ret: Ret, number: Option<Ty>) -> Ty {
        match ret {
            Ret::Num => number.unwrap_or(Ty::Error),
            Ret::F64 => Ty::F64,
            Ret::String => Ty::String,
            Ret::Bool => Ty::Bool,
            Ret::Usize => Ty::USIZE,
            Ret::Char => Ty::Char,
            Ret::ListOfString => Ty::List(Box::new(Ty::String)),
            Ret::ArrayOfString => Ty::Array(Box::new(Ty::String)),
            Ret::I64 => Ty::Int(IntTy::I64),
            Ret::ResultText => self.defined_ty(RESULT, vec![Ty::String, Ty::String]),
            Ret::ResultFlag => self.defined_ty(RESULT, vec![Ty::Bool, Ty::String]),
            Ret::Void => Ty::Void,
        }
    }
}

/// Which conversions exist (01-basics section 9).
///
/// - numbers to numbers, in any direction: the value is kept when it fits, truncated or rounded when it does not
/// - `bool` to and from integers
/// - `char` to integers, and from `u32` only, because not every integer is a character
/// - anything printable to `String`, and a `String` to a number (a run-time error when it is not one)
fn can_convert(from: &Ty, to: &Ty) -> bool {
    matches!(
        (from, to),
        (Ty::Error, _)
            | (_, Ty::Error)
            | (Ty::Int(_) | Ty::Float(_), Ty::Int(_) | Ty::Float(_))
            | (Ty::Ptr(_), Ty::Ptr(_))
            | (Ty::Ptr(_), Ty::Int(IntTy::Usize))
            | (Ty::Int(_), Ty::Ptr(_))
            | (Ty::Bool, Ty::Int(_))
            | (Ty::Int(_), Ty::Bool)
            | (Ty::Char, Ty::Int(_))
            | (Ty::Int(IntTy::U32), Ty::Char)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Char, Ty::Char)
            | (Ty::String, Ty::String)
            | (Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char, Ty::String)
            | (Ty::String, Ty::Int(_) | Ty::Float(_))
            | (
                Ty::Defined {
                    kind: DefinedKind::Enum,
                    ..
                },
                Ty::Int(_),
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert!(can_convert(&Ty::F64, &Ty::Int(IntTy::I32)));
        assert!(can_convert(&Ty::Int(IntTy::U32), &Ty::Char));
        assert!(!can_convert(&Ty::Int(IntTy::I32), &Ty::Char));
        assert!(can_convert(&Ty::String, &Ty::F64));
        assert!(!can_convert(&Ty::String, &Ty::Bool));
        assert!(!can_convert(&Ty::Float(FloatTy::F64), &Ty::Bool));
    }
}
