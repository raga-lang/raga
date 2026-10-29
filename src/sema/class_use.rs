//! Using classes: reading and changing their members, calling their methods, creating objects, and the rules of who may
//! use what (07-classes-and-traits).

use std::collections::HashMap;

use super::checker::Checker;
use super::classes::{MethodDef, Visibility, same_params};
use super::defined::{FieldDef, OPTIONAL};
use super::ty::{DefinedKind, FnTy, Ty, TypeId};
use crate::ast::{self, Arg, Expr, Ident, Trailer};
use crate::span::Span;

/// A member found on a class: where it was declared and the type it has for this use of the class.
struct Found<T> {
    item: T,
    owner: TypeId,
}

impl Checker {
    // ---- who may use what

    /// Reports an error unless the code being read may use a member that `owner` declares with this visibility.
    pub(super) fn check_access(
        &mut self,
        owner: TypeId,
        visibility: Visibility,
        name: &str,
        span: Span,
    ) -> bool {
        let current = self.class_stack.last().copied();
        let allowed = match visibility {
            Visibility::Public => true,
            Visibility::Private => current == Some(owner),
            Visibility::Protected => {
                current.is_some_and(|c| c == owner || self.def(c).supers.contains(&owner))
            }
        };
        if !allowed {
            let owner_name = self.def(owner).name.clone();
            let message = match visibility {
                Visibility::Private => format!("`{name}` is private: only {owner_name} can use it"),
                _ => format!(
                    "`{name}` is protected: it can only be used in {owner_name} and its subclasses"
                ),
            };
            self.error(span, message);
        }
        allowed
    }

    /// The class and what it extends, the nearest first, with the type arguments each one has for this use.
    fn lookup_chain(&self, tid: TypeId, args: &[Ty]) -> Vec<(TypeId, HashMap<String, Ty>)> {
        let mut out: Vec<(TypeId, HashMap<String, Ty>)> = Vec::new();
        let mut current = (tid, self.arg_map(tid, args));
        loop {
            out.push(current.clone());
            let Some((base, base_args)) = self.def(current.0).class.base.clone() else {
                break;
            };
            if out.iter().any(|(t, _)| *t == base) {
                break;
            }
            let base_args: Vec<Ty> = base_args.iter().map(|t| t.substitute(&current.1)).collect();
            current = (base, self.arg_map(base, &base_args));
        }
        // the interfaces, for an abstract class and for a value known only by its interface
        let mut index = 0;
        while index < out.len() {
            let (class, map) = out[index].clone();
            for (interface, iargs) in self.def(class).class.interfaces.clone() {
                if out.iter().any(|(t, _)| *t == interface) {
                    continue;
                }
                let iargs: Vec<Ty> = iargs.iter().map(|t| t.substitute(&map)).collect();
                out.push((interface, self.arg_map(interface, &iargs)));
            }
            index += 1;
        }
        out
    }

    fn find_field(&self, tid: TypeId, args: &[Ty], name: &str) -> Option<Found<FieldDef>> {
        for (class, map) in self.lookup_chain(tid, args) {
            if let Some(f) = self.def(class).fields.iter().find(|f| f.name == name) {
                let mut f = f.clone();
                f.ty = f.ty.substitute(&map);
                return Some(Found {
                    item: f,
                    owner: class,
                });
            }
        }
        None
    }

    fn find_accessor(
        &self,
        tid: TypeId,
        args: &[Ty],
        name: &str,
    ) -> Option<Found<super::classes::AccessorDef>> {
        for (class, map) in self.lookup_chain(tid, args) {
            if let Some(a) = self.def(class).class.accessors.get(name) {
                let mut a = a.clone();
                a.ty = a.ty.substitute(&map);
                return Some(Found {
                    item: a,
                    owner: class,
                });
            }
        }
        None
    }

    /// The versions of a method that a use of the class can reach: those of the class, then those it inherits that the
    /// class has not replaced or hidden.
    fn find_methods(&self, tid: TypeId, args: &[Ty], name: &str) -> Vec<Found<MethodDef>> {
        let mut found: Vec<Found<MethodDef>> = Vec::new();
        for (class, map) in self.lookup_chain(tid, args) {
            let Some(versions) = self.def(class).class.versions.get(name) else {
                continue;
            };
            for version in versions {
                if found.iter().any(|f| same_params(&f.item.sig, &version.sig)) {
                    continue;
                }
                let mut v = version.clone();
                if let Ty::Function(sig) = Ty::Function(v.sig.clone()).substitute(&map) {
                    v.sig = sig;
                }
                found.push(Found {
                    item: v,
                    owner: class,
                });
            }
        }
        found
    }

    // ---- fields and accessors

    /// The type of `receiver.name` for a class, an interface or a trait, or `None` after reporting why there is none.
    pub(super) fn class_field_type(
        &mut self,
        receiver: &Ty,
        name: &Ident,
        write: bool,
    ) -> Option<Ty> {
        let Ty::Defined {
            id,
            args,
            name: type_name,
            ..
        } = receiver
        else {
            return None;
        };
        if let Some(found) = self.find_field(*id, args, &name.name) {
            let field = found.item;
            self.check_access(found.owner, field.mods.visibility, &name.name, name.span);
            if field.mods.is_static {
                self.error(
                    name.span,
                    format!(
                        "{} is static: write {}.{}",
                        name.name,
                        self.def(found.owner).name,
                        name.name
                    ),
                );
            }
            if write && field.readonly {
                self.error(
                    name.span,
                    format!("`{}` is const and cannot be assigned", name.name),
                );
            }
            return Some(field.ty);
        }
        if let Some(found) = self.find_accessor(*id, args, &name.name) {
            let accessor = found.item;
            if accessor.mods.is_static {
                self.error(
                    name.span,
                    format!(
                        "{} is static: write {}.{}",
                        name.name,
                        self.def(found.owner).name,
                        name.name
                    ),
                );
            }
            let (part, word) = if write {
                (accessor.set, "has no set, it is read-only")
            } else {
                (accessor.get, "has no get, it is write-only")
            };
            match part {
                Some(visibility) => {
                    self.check_access(found.owner, visibility, &name.name, name.span);
                }
                None => self.error(name.span, format!("{} {word}", name.name)),
            }
            return Some(accessor.ty);
        }
        if !self.find_methods(*id, args, &name.name).is_empty() {
            self.error(
                name.span,
                format!(
                    "`{}` is a method: call it, {}.{}(...). A method on its own is not a value",
                    name.name, type_name, name.name
                ),
            );
            return None;
        }
        self.error(
            name.span,
            format!("{type_name} has no field `{}`", name.name),
        );
        None
    }

    /// `Class.name`: a static field or a static accessor, read or written on the class itself.
    pub(super) fn class_static_member(&mut self, meta: &Ty, name: &Ident, write: bool) -> Ty {
        let Ty::TypeValue {
            id,
            args,
            name: type_name,
            ..
        } = meta
        else {
            return Ty::Error;
        };
        if let Some(found) = self.find_field(*id, args, &name.name) {
            let field = found.item;
            if !field.mods.is_static {
                self.error(
                    name.span,
                    format!(
                        "`{}` belongs to each object: read it on a value, not on {type_name}",
                        name.name
                    ),
                );
                return Ty::Error;
            }
            self.check_access(found.owner, field.mods.visibility, &name.name, name.span);
            if write && field.readonly {
                self.error(
                    name.span,
                    format!("`{}` is const and cannot be assigned", name.name),
                );
            }
            self.touched_static = true;
            return field.ty;
        }
        if let Some(found) = self.find_accessor(*id, args, &name.name) {
            let accessor = found.item;
            if !accessor.mods.is_static {
                self.error(
                    name.span,
                    format!(
                        "`{}` belongs to each object: read it on a value, not on {type_name}",
                        name.name
                    ),
                );
                return Ty::Error;
            }
            let (part, word) = if write {
                (accessor.set, "has no set, it is read-only")
            } else {
                (accessor.get, "has no get, it is write-only")
            };
            match part {
                Some(visibility) => {
                    self.check_access(found.owner, visibility, &name.name, name.span);
                }
                None => self.error(name.span, format!("{} {word}", name.name)),
            }
            return accessor.ty;
        }
        self.error(
            name.span,
            format!("{type_name} has no static member `{}`", name.name),
        );
        Ty::Error
    }

    // ---- calling methods

    /// `receiver.name(args)` for a value of a class, an interface or a trait.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn class_call(
        &mut self,
        call: &Expr,
        receiver: &Ty,
        name: &Ident,
        type_args: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let Ty::Defined {
            id,
            args: type_values,
            name: type_name,
            ..
        } = receiver
        else {
            return Ty::Error;
        };
        // what a value really is: `value.is<Student>()` and `value.as<Student>()`
        if matches!(name.name.as_str(), "is" | "as") && type_args.len() == 1 {
            let target = self.resolve_type(&type_args[0]);
            if !matches!(
                target,
                Ty::Defined {
                    kind: DefinedKind::Class | DefinedKind::Interface,
                    ..
                }
            ) && !target.is_error()
            {
                self.error(
                    type_args[0].span,
                    format!("{target} is not a class or an interface"),
                );
            }
            self.check_arguments(args);
            return if name.name == "is" {
                Ty::Bool
            } else {
                self.defined_ty(OPTIONAL, vec![target])
            };
        }
        if !type_args.is_empty() {
            self.error(
                call.span,
                format!("{type_name}.{} takes no type arguments", name.name),
            );
        }
        let candidates = self.find_methods(*id, type_values, &name.name);
        if candidates.is_empty() {
            if self.find_accessor(*id, type_values, &name.name).is_some() {
                self.error(
                    name.span,
                    format!(
                        "`{}` is an accessor: it is read and written like a field, never called",
                        name.name
                    ),
                );
                self.check_arguments(args);
                return Ty::Error;
            }
            if name.name == "clone" {
                if trailer.is_some() {
                    self.error(call.span, "clone takes no block");
                }
                return self.clone_call(call, receiver, args);
            }
            if let Some(field) = self.find_field(*id, type_values, &name.name)
                && let Ty::Function(f) = field.item.ty
            {
                return self.call_function(call, &name.name, &f, &[], args, trailer);
            }
            self.error(
                name.span,
                format!("{type_name} has no method `{}`", name.name),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        if candidates.iter().all(|c| c.item.mods.is_static) {
            let owner = self.def(candidates[0].owner).name.clone();
            self.error(
                name.span,
                format!(
                    "`{}` is static: write {owner}.{}(...), it is not reached through a value",
                    name.name, name.name
                ),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let usable: Vec<&Found<MethodDef>> = candidates
            .iter()
            .filter(|c| !c.item.mods.is_static)
            .collect();
        let mut allowed = Vec::new();
        for found in &usable {
            let current = self.class_stack.last().copied();
            let ok = match found.item.mods.visibility {
                Visibility::Public => true,
                Visibility::Private => current == Some(found.owner),
                Visibility::Protected => current
                    .is_some_and(|c| c == found.owner || self.def(c).supers.contains(&found.owner)),
            };
            if ok {
                allowed.push(found.item.sig.clone());
            }
        }
        if allowed.is_empty() {
            let first = usable[0];
            self.check_access(
                first.owner,
                first.item.mods.visibility,
                &name.name,
                name.span,
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let label = format!("{type_name}.{}", name.name);
        self.pick_version(call, &label, allowed, args, trailer)
    }

    /// Chooses among the versions of a method by the arguments: how many, their types and their names.
    pub(super) fn pick_version(
        &mut self,
        call: &Expr,
        label: &str,
        versions: Vec<FnTy>,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        if versions.len() == 1 {
            return self.call_function(call, label, &versions[0], &[], args, trailer);
        }
        let mut fits = Vec::new();
        for (i, version) in versions.iter().enumerate() {
            let before = self.diagnostics.len();
            self.call_function(call, label, version, &[], args, trailer);
            if self.diagnostics.len() == before {
                fits.push(i);
            }
            self.diagnostics.truncate(before);
        }
        match fits.as_slice() {
            [i] => self.call_function(call, label, &versions[*i], &[], args, trailer),
            [] => {
                let shown: Vec<String> = versions
                    .iter()
                    .map(|v| {
                        let params: Vec<String> =
                            v.params.iter().map(|p| p.ty.to_string()).collect();
                        format!("({})", params.join(", "))
                    })
                    .collect();
                self.error(
                    call.span,
                    format!(
                        "no version of {label} takes these arguments. It has {}",
                        shown.join(", ")
                    ),
                );
                self.check_arguments(args);
                Ty::Error
            }
            _ => {
                self.error(
                    call.span,
                    format!("two versions of {label} fit these arguments equally well: name them to tell them apart"),
                );
                self.check_arguments(args);
                Ty::Error
            }
        }
    }

    /// `Class.name(args)`: a static method, called on the class.
    pub(super) fn class_static_call(
        &mut self,
        call: &Expr,
        meta: &Ty,
        name: &Ident,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let Ty::TypeValue {
            id,
            args: type_values,
            name: type_name,
            ..
        } = meta
        else {
            return Ty::Error;
        };
        let candidates = self.find_methods(*id, type_values, &name.name);
        let statics: Vec<&Found<MethodDef>> = candidates
            .iter()
            .filter(|c| c.item.mods.is_static)
            .collect();
        if statics.is_empty() {
            let message = if candidates.is_empty() {
                format!("{type_name} has no function `{}`", name.name)
            } else {
                format!(
                    "`{}` belongs to each object: call it on a value, not on {type_name}",
                    name.name
                )
            };
            self.error(name.span, message);
            self.check_arguments(args);
            return Ty::Error;
        }
        let mut allowed = Vec::new();
        for found in &statics {
            if self.check_access(
                found.owner,
                found.item.mods.visibility,
                &name.name,
                name.span,
            ) {
                allowed.push(found.item.sig.clone());
            }
        }
        if allowed.is_empty() {
            self.check_arguments(args);
            return Ty::Error;
        }
        let label = format!("{type_name}.{}", name.name);
        self.pick_version(call, &label, allowed, args, trailer)
    }

    // ---- creating objects

    /// `Person(name = "ann")`, `Pupil(name = "sam", roll_number = 7)`, `Stack<i32>()`.
    pub(super) fn construct_class(
        &mut self,
        call: &Expr,
        id: TypeId,
        explicit: Vec<Ty>,
        args: &[Arg],
        expected: Option<&Ty>,
    ) -> Ty {
        let name = self.def(id).name.clone();
        if self.def(id).class.is_abstract {
            self.error(
                call.span,
                format!("{name} is abstract: it cannot be created"),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let mut known = self.seed_arguments(call, id, explicit, expected);
        self.creation_arguments(call, id, &name, args, &mut known);
        self.finish_arguments(call, id, &known, None)
    }

    /// Checks the arguments of a creation against the constructors of the class, or against its fields.
    pub(super) fn creation_arguments(
        &mut self,
        call: &Expr,
        id: TypeId,
        label: &str,
        args: &[Arg],
        known: &mut HashMap<String, Ty>,
    ) {
        let generics = self.def(id).generics.clone();
        // the nearest class with a constructor decides how a value is created
        let constructors = self
            .class_chain(id)
            .into_iter()
            .find_map(|c| {
                let ctors = &self.def(c).class.constructors;
                (!ctors.is_empty()).then(|| ctors.clone())
            })
            .unwrap_or_default();
        if !constructors.is_empty() {
            let map = known.clone();
            let versions: Vec<FnTy> = constructors
                .into_iter()
                .filter_map(|c| match Ty::Function(c).substitute(&map) {
                    Ty::Function(f) => Some(f),
                    _ => None,
                })
                .collect();
            let label = format!("the constructor of {label}");
            self.pick_version(call, &label, versions, args, None);
            return;
        }
        // no constructor: the fields by name, and the accessors that have a set
        let current = self.class_stack.last().copied();
        let mut fields: Vec<FieldDef> = Vec::new();
        for (class, map) in self.lookup_chain(
            id,
            &generics
                .iter()
                .map(|g| Ty::TypeParam(g.clone()))
                .collect::<Vec<_>>(),
        ) {
            for f in &self.def(class).fields {
                if f.mods.is_static || fields.iter().any(|g| g.name == f.name) || f.readonly {
                    continue;
                }
                let visible = match f.mods.visibility {
                    Visibility::Public => true,
                    Visibility::Private => current == Some(class),
                    Visibility::Protected => {
                        current.is_some_and(|c| c == class || self.def(c).supers.contains(&class))
                    }
                };
                if !visible {
                    continue;
                }
                let mut f = f.clone();
                f.ty = f.ty.substitute(&map);
                f.has_default = true;
                fields.push(f);
            }
            for (accessor_name, a) in &self.def(class).class.accessors {
                if a.set.is_some()
                    && !a.mods.is_static
                    && !fields.iter().any(|g| g.name == *accessor_name)
                {
                    let mut f = FieldDef::new(accessor_name, a.ty.substitute(&map), true);
                    f.owner = class;
                    fields.push(f);
                }
            }
        }
        let pairs: Vec<(Option<&Ident>, &Expr)> =
            args.iter().map(|a| (a.name.as_ref(), &a.value)).collect();
        // a field that exists but may not be used from here is private or protected, not unknown
        for (name, _) in &pairs {
            if let Some(n) = name
                && !fields.iter().any(|f| f.name == n.name)
                && let Some(found) = self.find_field(
                    id,
                    &generics
                        .iter()
                        .map(|g| Ty::TypeParam(g.clone()))
                        .collect::<Vec<_>>(),
                    &n.name,
                )
                && !found.item.mods.is_static
            {
                self.check_access(found.owner, found.item.mods.visibility, &n.name, n.span);
            }
        }
        let pairs: Vec<(Option<&Ident>, &Expr)> = pairs
            .into_iter()
            .filter(|(name, _)| {
                name.is_none_or(|n| {
                    fields.iter().any(|f| f.name == n.name)
                        || self.find_field(id, &[], &n.name).is_none()
                })
            })
            .collect();
        self.fill_fields(call, label, &fields, &generics, &pairs, known, false, false);
    }

    /// `super(name = name, roll_number = 0)`: the constructor of the base class, run first.
    pub(super) fn super_call(&mut self, call: &Expr, args: &[Arg]) -> Ty {
        let Some(current) = self.class_stack.last().copied() else {
            self.error(call.span, "`super` is used inside a class");
            self.check_arguments(args);
            return Ty::Void;
        };
        let Some((base, base_args)) = self.def(current).class.base.clone() else {
            self.error(call.span, "`super` needs a class that extends another one");
            self.check_arguments(args);
            return Ty::Void;
        };
        let label = self.def(base).name.clone();
        let mut known = self.arg_map(base, &base_args);
        self.creation_arguments(call, base, &label, args, &mut known);
        Ty::Void
    }
}
