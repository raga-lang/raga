//! The built-in collections: their types, how they are created, and their methods (02-data-structures).
//!
//! `Array<T>`, `List<T>`, `Set<T>`, `Map<K, V>` and `Tuple<A, B, ...>` are generic types the compiler knows. Each
//! method is described by what it takes and what it gives back, written with the element types of the receiver, so
//! `List<f64>.push` takes an `f64` and `Map<String, i32>.get` takes a `String` and gives an `i32`.

use super::checker::Checker;
use super::ty::{FnTy, ParamTy, Ty};
use std::collections::HashMap;

use crate::ast::{self, Arg, Expr, ExprKind, Ident, Trailer};
use crate::span::Span;

/// The names that make a collection when called: `List(1, 2)`, `Map<String, i32>()`.
pub(super) fn is_collection_name(name: &str) -> bool {
    matches!(name, "Array" | "List" | "Set" | "Map" | "Tuple")
}

/// What a method takes and what it gives back.
struct Method {
    params: Vec<Ty>,
    ret: Ty,
}

fn method(params: Vec<Ty>, ret: Ty) -> Option<Method> {
    Some(Method { params, ret })
}

/// The function a List is sorted by: it takes two elements and gives a negative number when the first comes first, 0 when
/// they are equal and a positive number when the second comes first.
fn comparison(item: Ty) -> Ty {
    let param = |ty: Ty| ParamTy {
        name: None,
        ty,
        has_default: false,
        variadic: false,
    };
    Ty::Function(FnTy {
        generics: Vec::new(),
        params: vec![param(item.clone()), param(item)],
        ret: Box::new(Ty::I32),
    })
}

/// The method `name` of a collection, with the types of its parameters and result worked out from the receiver.
fn collection_method(receiver: &Ty, name: &str) -> Option<Method> {
    let index = Ty::USIZE;
    if name == "clone" {
        return method(vec![], receiver.clone());
    }
    match receiver {
        Ty::Array(item) => {
            let item = (**item).clone();
            match name {
                "get" => method(vec![index], item),
                "set" => method(vec![index, item], Ty::Void),
                "len" => method(vec![], Ty::USIZE),
                "contains" => method(vec![item], Ty::Bool),
                _ => None,
            }
        }
        Ty::List(item) => {
            let item = (**item).clone();
            match name {
                "get" => method(vec![index], item),
                "set" => method(vec![index, item], Ty::Void),
                "len" => method(vec![], Ty::USIZE),
                "contains" => method(vec![item], Ty::Bool),
                "push" | "unshift" => method(vec![item], Ty::Void),
                "pop" | "shift" => method(vec![], item),
                "insert" => method(vec![index, item], Ty::Void),
                "sort" => method(vec![comparison(item)], Ty::Void),
                "remove" => method(vec![index], item),
                "reverse" => method(vec![], Ty::Void),
                "to_array" => method(vec![], Ty::Array(Box::new(item.clone()))),
                _ => None,
            }
        }
        Ty::Set(item) => {
            let item = (**item).clone();
            match name {
                "add" | "remove" => method(vec![item], Ty::Void),
                "contains" => method(vec![item], Ty::Bool),
                "len" => method(vec![], Ty::USIZE),
                _ => None,
            }
        }
        Ty::Map(key, value) => {
            let (key, value) = ((**key).clone(), (**value).clone());
            match name {
                "set" => method(vec![key, value], Ty::Void),
                "get" => method(vec![key], value),
                "has" => method(vec![key], Ty::Bool),
                "remove" => method(vec![key], Ty::Void),
                "len" => method(vec![], Ty::USIZE),
                "keys" => method(vec![], Ty::Array(Box::new(key))),
                "values" => method(vec![], Ty::Array(Box::new(value))),
                _ => None,
            }
        }
        _ => None,
    }
}

impl Checker {
    /// The type `name<args>` once the arguments are known. Used for a type that is written (`Map<String, i32> m`) and
    /// for a constructor that names its types (`Map<String, i32>()`), so both have the same rules.
    pub(super) fn generic_type(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        match (name, args.as_slice()) {
            ("Array", [item]) => Ty::Array(Box::new(item.clone())),
            ("List", [item]) => Ty::List(Box::new(item.clone())),
            ("Set", [item]) => Ty::Set(Box::new(item.clone())),
            ("Array" | "List" | "Set", _) => {
                self.error(span, format!("`{name}` takes one type: {name}<T>"));
                Ty::Error
            }
            ("Map", [key, value]) => {
                if !self.check_key(key, span) {
                    return Ty::Error;
                }
                Ty::Map(Box::new(key.clone()), Box::new(value.clone()))
            }
            ("Map", _) => {
                self.error(span, "`Map` takes two types: Map<K, V>");
                Ty::Error
            }
            ("Tuple", slots) if !slots.is_empty() => Ty::Tuple(args),
            _ => {
                self.error(span, "`Tuple` takes at least one type: Tuple<A, B>");
                Ty::Error
            }
        }
    }

    /// A Map key is a primitive value or a String (02-data-structures section 5).
    fn check_key(&mut self, key: &Ty, span: Span) -> bool {
        if key.is_key() {
            return true;
        }
        self.error(
            span,
            format!("a Map key is a primitive value or a String, not {key}"),
        );
        false
    }

    // ---- creating

    /// `List(1, 2, 3)`, `Set<i32>()`, `Map("a", 1, "b", 2)`, `Tuple(1, 2.5)`.
    ///
    /// The types come from the type arguments if they are written, else from the type the context expects, else from
    /// the values. A collection with no values has nothing to infer from, so it needs one of the first two.
    pub(super) fn construct(
        &mut self,
        call: &Expr,
        name: &Ident,
        type_args: &[ast::Type],
        args: &[Arg],
        expected: Option<&Ty>,
    ) -> Ty {
        let kind = name.name.as_str();
        if let Some(named) = args.iter().find(|a| a.name.is_some()) {
            self.error(
                named.value.span,
                format!("{kind}(...) takes values, not named arguments"),
            );
        }
        let values: Vec<&Expr> = args.iter().map(|a| &a.value).collect();

        // the type, when it is written or expected
        let known = if type_args.is_empty() {
            expected.filter(|ty| shape(kind, ty)).cloned()
        } else {
            let written: Vec<Ty> = type_args.iter().map(|a| self.resolve_type(a)).collect();
            // `Map<String> m = Map<String>()`: the declared type has said what is wrong with the type arguments already
            let reported = self.diagnostics.len();
            let made = self.generic_type(kind, written, call.span);
            if matches!(expected, Some(Ty::Error)) {
                self.diagnostics.truncate(reported);
            }
            match made {
                Ty::Error => {
                    for value in &values {
                        self.expr(value, None);
                    }
                    return Ty::Error;
                }
                ty => Some(ty),
            }
        };
        if matches!(expected, Some(Ty::Error)) && known.is_none() {
            for value in &values {
                self.expr(value, None);
            }
            return Ty::Error;
        }

        match kind {
            "Map" => self.construct_map(call, &values, known),
            "Tuple" => self.construct_tuple(call, &values, known),
            _ => {
                let item = match &known {
                    Some(Ty::Array(item) | Ty::List(item) | Ty::Set(item)) => {
                        Some((**item).clone())
                    }
                    _ => None,
                };
                let item = if item.is_none() && values.is_empty() {
                    self.error(
                        call.span,
                        format!("{kind}() has no values to decide its type: write {kind}<i32>()"),
                    );
                    return Ty::Error;
                } else {
                    self.element_type(call.span, &values, item)
                };
                let Some(item) = item else { return Ty::Error };
                let item = Box::new(item);
                match kind {
                    "Array" => Ty::Array(item),
                    "List" => Ty::List(item),
                    _ => Ty::Set(item),
                }
            }
        }
    }

    /// `Map(key, value, key, value)`
    fn construct_map(&mut self, call: &Expr, values: &[&Expr], known: Option<Ty>) -> Ty {
        if values.len() % 2 == 1 {
            self.error(
                call.span,
                format!(
                    "Map takes pairs, key then value, but {} values were given",
                    values.len()
                ),
            );
            for value in values {
                self.expr(value, None);
            }
            return Ty::Error;
        }
        let (key, value) = match &known {
            Some(Ty::Map(key, value)) => (Some((**key).clone()), Some((**value).clone())),
            _ => (None, None),
        };
        if key.is_none() && values.is_empty() {
            self.error(
                call.span,
                "Map() has no values to decide its types: write Map<String, i32>()",
            );
            return Ty::Error;
        }
        let keys: Vec<&Expr> = values.iter().step_by(2).copied().collect();
        let items: Vec<&Expr> = values.iter().skip(1).step_by(2).copied().collect();
        let key = self.element_type(call.span, &keys, key);
        let value = self.element_type(call.span, &items, value);
        let (Some(key), Some(value)) = (key, value) else {
            return Ty::Error;
        };
        if known.is_none() && !self.check_key(&key, call.span) {
            return Ty::Error;
        }
        Ty::Map(Box::new(key), Box::new(value))
    }

    /// `Tuple(a, b)`: one value for each slot.
    fn construct_tuple(&mut self, call: &Expr, values: &[&Expr], known: Option<Ty>) -> Ty {
        let slots = match &known {
            Some(Ty::Tuple(slots)) => Some(slots.as_slice()),
            _ => None,
        };
        if let Some(slots) = slots
            && slots.len() != values.len()
        {
            self.error(
                call.span,
                format!(
                    "{} has {} slots, but {} values were given",
                    known.as_ref().expect("slots come from it"),
                    slots.len(),
                    values.len()
                ),
            );
        }
        let types = values
            .iter()
            .enumerate()
            .map(|(i, value)| self.expr(value, slots.and_then(|s| s.get(i))))
            .collect();
        Ty::Tuple(types)
    }

    // ---- methods

    /// `receiver.name(args)` where the receiver is an Array, a List, a Set or a Map.
    pub(super) fn collection_call(
        &mut self,
        call: &Expr,
        receiver: &Ty,
        name: &Ident,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let label = name.name.as_str();
        let Some(Method { params, ret }) = collection_method(receiver, label) else {
            let hint = match (receiver, label) {
                (
                    Ty::Array(_),
                    "push" | "pop" | "shift" | "unshift" | "insert" | "remove" | "reverse",
                ) => ": an Array has a fixed size, use a List",
                (_, "equals") => ": == compares references and there is no equals for this type",
                _ => "",
            };
            self.error(
                name.span,
                format!("{receiver} has no method `{label}`{hint}"),
            );
            self.check_arguments(args);
            return Ty::Error;
        };
        // a block after the arguments fills the last parameter
        let mut params = params;
        let block = match trailer {
            Some(trailer) => match params.pop() {
                Some(last) => Some((last, trailer)),
                None => {
                    self.error(
                        call.span,
                        format!("{label} takes no block after its arguments"),
                    );
                    None
                }
            },
            None => None,
        };
        if !self.arity(call, label, args, params.len()) {
            return ret;
        }
        for (param, arg) in params.into_iter().zip(args) {
            self.argument(&arg.value, param);
        }
        if let Some((last, trailer)) = block {
            self.block_argument(label, &last, trailer, &[], &mut HashMap::new());
        }
        ret
    }

    /// `tuple.get(position)`, `tuple.set(position, value)` and `tuple.clone()`.
    ///
    /// Each slot has its own type, so the position decides the type of the result and must be known while the program is
    /// built: a literal, or a `const` made from one. A position outside the Tuple is an error here, not a thrown Error.
    pub(super) fn tuple_call(
        &mut self,
        call: &Expr,
        slots: &[Ty],
        name: &Ident,
        args: &[Arg],
    ) -> Ty {
        let label = name.name.as_str();
        let wanted = match label {
            "get" => 1,
            "set" => 2,
            "clone" => 0,
            _ => {
                self.error(
                    name.span,
                    format!("a Tuple has no method `{label}`, only get, set and clone"),
                );
                self.check_arguments(args);
                return Ty::Error;
            }
        };
        if !self.arity(call, label, args, wanted) {
            return Ty::Error;
        }
        if wanted == 0 {
            return Ty::Tuple(slots.to_vec());
        }
        let slot = self.slot_position(&args[0].value, slots.len());
        let slot_ty = slot.map(|at| slots[at].clone());
        if label == "set" {
            match &slot_ty {
                Some(ty) => self.argument(&args[1].value, ty.clone()),
                None => {
                    self.expr(&args[1].value, None);
                }
            }
            return Ty::Void;
        }
        slot_ty.unwrap_or(Ty::Error)
    }

    /// The slot a position names, or `None` after reporting why it is not one.
    fn slot_position(&mut self, position: &Expr, count: usize) -> Option<usize> {
        self.expr(position, Some(&Ty::USIZE));
        let Some(at) = self.literal_value(position) else {
            if !matches!(position.kind, ExprKind::Error) {
                self.error(
                    position.span,
                    "the position must be a literal or a const made from one: each slot has its own type, so the type of the result has to be known when the program is built",
                );
            }
            return None;
        };
        if at >= count as u128 {
            self.error(
                position.span,
                format!("a Tuple of {count} has no slot {at}"),
            );
            return None;
        }
        Some(at as usize)
    }

    /// Reports a call with the wrong number of arguments. Says whether the number is right.
    fn arity(&mut self, call: &Expr, label: &str, args: &[Arg], wanted: usize) -> bool {
        if args.len() == wanted {
            if let Some(named) = args.iter().find(|a| a.name.is_some()) {
                self.error(
                    named.value.span,
                    "named arguments are not checked yet for built-in functions",
                );
            }
            return true;
        }
        let plural = if wanted == 1 { "" } else { "s" };
        self.error(
            call.span,
            format!(
                "{label} takes {wanted} argument{plural}, but {} given",
                args.len()
            ),
        );
        self.check_arguments(args);
        false
    }
}

/// Does a type have the shape that the constructor `kind` makes?
fn shape(kind: &str, ty: &Ty) -> bool {
    matches!(
        (kind, ty),
        ("Array", Ty::Array(_))
            | ("List", Ty::List(_))
            | ("Set", Ty::Set(_))
            | ("Map", Ty::Map(..))
            | ("Tuple", Ty::Tuple(_))
    )
}
