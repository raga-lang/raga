//! Classes, interfaces and traits (07-classes-and-traits).
//!
//! A class is declared like every other type, `const Name = class { ... } extends(Base) implements(A) use(T)`, and what
//! a member may do is written in attributes on the line above it: who may use it (`Visibility`), how a class that extends
//! may change it (`Inheritance`), and whether it belongs to each object or to the class (`Ownership`). They are ordinary
//! enums that come with the language, so none of these words is reserved. This module reads those attributes, keeps
//! the members of the class, and checks what the rules of the template ask for.

use std::collections::HashMap;

use super::checker::Checker;
use super::defined::FieldDef;
use super::ty::{DefinedKind, FnTy, Ty, TypeId};
use crate::ast::{Arg, Attribute, Decl, DeclKind, Expr, ExprKind, Ident, Stmt, StmtKind, Trailer};
use crate::span::Span;

/// Who may use a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Visibility {
    Public,
    Protected,
    Private,
}

/// What the attributes of a member say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Modifiers {
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_final: bool,
    pub is_abstract: bool,
    pub is_virtual: bool,
    pub is_override: bool,
}

impl Default for Modifiers {
    fn default() -> Modifiers {
        Modifiers {
            visibility: Visibility::Public,
            is_static: false,
            is_final: false,
            is_abstract: false,
            is_virtual: false,
            is_override: false,
        }
    }
}

/// One version of a method.
#[derive(Debug, Clone)]
pub(super) struct MethodDef {
    pub sig: FnTy,
    pub mods: Modifiers,
    pub owner: TypeId,
    pub has_body: bool,
    pub span: Span,
}

/// An accessor: read and written like a field, but it runs code.
#[derive(Debug, Clone)]
pub(super) struct AccessorDef {
    pub ty: Ty,
    pub get: Option<Visibility>,
    pub set: Option<Visibility>,
    pub mods: Modifiers,
    pub owner: TypeId,
}

/// What a class, an interface or a trait has besides fields.
#[derive(Debug, Clone, Default)]
pub(super) struct ClassInfo {
    pub base: Option<(TypeId, Vec<Ty>)>,
    pub interfaces: Vec<(TypeId, Vec<Ty>)>,
    pub traits: Vec<TypeId>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub versions: HashMap<String, Vec<MethodDef>>,
    pub accessors: HashMap<String, AccessorDef>,
    pub constructors: Vec<FnTy>,
    pub has_destructor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Class,
    Method,
    Field,
    Accessor,
}

/// One modifier, as written: `Visibility.Private`, `Inheritance.Final`, `Ownership.Static`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Visibility(Visibility),
    Final,
    Abstract,
    Virtual,
    Override,
    Static,
    Instance,
}

impl Checker {
    // ---- the attributes

    /// The modifiers that the attributes above a class or a member describe.
    ///
    /// The attributes that come with the language are `class`, `method`, `field`, `accessor`, `get` and `set`, and each
    /// takes some of the three enums. Any other attribute is not for this module.
    pub(super) fn modifiers(&mut self, attrs: &[Attribute], target: Target) -> Modifiers {
        let mut mods = Modifiers::default();
        let mut seen: Vec<(Modifier, Span)> = Vec::new();
        let mut get_set: Vec<(&str, Span)> = Vec::new();
        for attr in attrs {
            let name = attr.name.name.as_str();
            let describes = match name {
                "class" => Some(Target::Class),
                "method" => Some(Target::Method),
                "field" => Some(Target::Field),
                "accessor" | "get" | "set" => Some(Target::Accessor),
                _ => None,
            };
            let Some(describes) = describes else { continue };
            if describes != target {
                let what = match target {
                    Target::Class => "a class",
                    Target::Method => "a method",
                    Target::Field => "a field",
                    Target::Accessor => "an accessor",
                };
                self.error(attr.span, format!("[{name}(...)] does not describe {what}"));
                continue;
            }
            for arg in &attr.args {
                let Some(modifier) = self.decode_modifier(arg) else {
                    continue;
                };
                let allowed = match (name, modifier) {
                    ("get" | "set", Modifier::Visibility(_)) => true,
                    ("get" | "set", _) => false,
                    ("class", Modifier::Final | Modifier::Abstract) => true,
                    ("class", _) => false,
                    ("field", Modifier::Visibility(_) | Modifier::Static | Modifier::Instance) => {
                        true
                    }
                    ("field", _) => false,
                    (
                        "accessor",
                        Modifier::Visibility(_)
                        | Modifier::Static
                        | Modifier::Instance
                        | Modifier::Virtual
                        | Modifier::Override
                        | Modifier::Final,
                    ) => true,
                    ("accessor", _) => false,
                    _ => true,
                };
                if !allowed {
                    let takes = match name {
                        "get" | "set" => {
                            "get and set take a Visibility only, Ownership.Static belongs to [accessor(...)]"
                        }
                        "class" => "[class(...)] takes Inheritance.Final and Inheritance.Abstract",
                        "field" => "[field(...)] takes Visibility and Ownership only",
                        _ => {
                            "[accessor(...)] takes Visibility, Ownership and Inheritance.Virtual, Override or Final"
                        }
                    };
                    self.error(arg.value.span, takes);
                    continue;
                }
                if matches!(name, "get" | "set") {
                    get_set.push((name, arg.value.span));
                }
                seen.push((modifier, arg.value.span));
            }
        }
        let words = match target {
            Target::Class => "class",
            Target::Method => "method",
            Target::Field => "field",
            Target::Accessor => "accessor",
        };
        for (modifier, span) in &seen {
            match modifier {
                Modifier::Visibility(v) => {
                    mods.visibility = *v;
                }
                Modifier::Final => mods.is_final = true,
                Modifier::Abstract => mods.is_abstract = true,
                Modifier::Virtual => mods.is_virtual = true,
                Modifier::Override => mods.is_override = true,
                Modifier::Static => mods.is_static = true,
                Modifier::Instance => {}
            }
            let _ = span;
        }
        // contradictions
        let visibilities: Vec<_> = seen
            .iter()
            .filter(|(m, _)| matches!(m, Modifier::Visibility(_)))
            .collect();
        if visibilities.len() > 1 {
            self.error(
                visibilities[1].1,
                "two visibilities: a member is public, protected or private",
            );
        }
        let at = seen.first().map_or(Span::default(), |(_, s)| *s);
        if seen.iter().any(|(m, _)| *m == Modifier::Static)
            && seen.iter().any(|(m, _)| *m == Modifier::Instance)
        {
            self.error(at, "a member is static or belongs to each object, not both");
        }
        if mods.is_abstract && mods.is_final {
            let message = if target == Target::Class {
                "an abstract class must be extended, a final one cannot be"
            } else {
                "an abstract method must be overridden, a final one cannot be"
            };
            self.error(at, message);
        }
        if mods.is_static && mods.is_override {
            self.error(
                at,
                "a static method belongs to the class, there is nothing to override",
            );
        } else if mods.is_static && (mods.is_virtual || mods.is_abstract) {
            self.error(
                at,
                "a static method belongs to the class, it is not replaced",
            );
        }
        if mods.is_final
            && !mods.is_abstract
            && !mods.is_virtual
            && !mods.is_override
            && !mods.is_static
            && target != Target::Class
            && !seen.is_empty()
        {
            self.error(
                at,
                format!("final needs a virtual {words} or an override: there is nothing to forbid replacing"),
            );
        }
        let _ = get_set;
        mods
    }

    /// One argument of an attribute, `Visibility.Private`, as the modifier it names.
    fn decode_modifier(&mut self, arg: &Arg) -> Option<Modifier> {
        let value = &arg.value;
        match &value.kind {
            ExprKind::Name(word) => {
                let suggestion = match word.name.as_str() {
                    "static" => "Ownership.Static",
                    "final" => "Inheritance.Final",
                    "abstract" => "Inheritance.Abstract",
                    "virtual" => "Inheritance.Virtual",
                    "override" => "Inheritance.Override",
                    "public" => "Visibility.Public",
                    "protected" => "Visibility.Protected",
                    "private" => "Visibility.Private",
                    _ => "Visibility.Private, Inheritance.Final or Ownership.Static",
                };
                self.error(
                    word.span,
                    format!("`{}` is not a modifier: write {suggestion}, a modifier is always written with its enum", word.name),
                );
                None
            }
            ExprKind::Member { object, name } => {
                let ExprKind::Name(group) = &object.kind else {
                    self.error(
                        value.span,
                        "a modifier is Visibility.X, Inheritance.X or Ownership.X",
                    );
                    return None;
                };
                let modifier = match (group.name.as_str(), name.name.as_str()) {
                    ("Visibility", "Public") => Modifier::Visibility(Visibility::Public),
                    ("Visibility", "Protected") => Modifier::Visibility(Visibility::Protected),
                    ("Visibility", "Private") => Modifier::Visibility(Visibility::Private),
                    ("Inheritance", "Final") => Modifier::Final,
                    ("Inheritance", "Abstract") => Modifier::Abstract,
                    ("Inheritance", "Virtual") => Modifier::Virtual,
                    ("Inheritance", "Override") => Modifier::Override,
                    ("Ownership", "Static") => Modifier::Static,
                    ("Ownership", "Instance") => Modifier::Instance,
                    ("Visibility" | "Inheritance" | "Ownership", member) => {
                        self.error(name.span, format!("{} has no member {member}", group.name));
                        return None;
                    }
                    (other, _) => {
                        self.error(group.span, format!("{other} is not a modifier enum: use Visibility, Inheritance or Ownership"));
                        return None;
                    }
                };
                Some(modifier)
            }
            _ => {
                self.error(
                    value.span,
                    "a modifier is Visibility.X, Inheritance.X or Ownership.X",
                );
                None
            }
        }
    }

    // ---- declaring a class, an interface or a trait

    /// The class-like type `tid`: its clauses, its members, and whether it keeps its promises.
    pub(super) fn class_like(
        &mut self,
        tid: TypeId,
        decl: &Decl,
        attrs: &[Attribute],
        body: &[Stmt],
        clauses: &[Expr],
    ) {
        let kind = self.def(tid).kind;
        let mods = self.modifiers(attrs, Target::Class);
        if kind != DefinedKind::Class && attrs.iter().any(|a| a.name.name == "class") {
            let a = attrs
                .iter()
                .find(|a| a.name.name == "class")
                .map_or(Span::default(), |a| a.span);
            self.error(a, "[class(...)] describes a class");
        }
        self.types[tid.0 as usize].class.is_abstract = mods.is_abstract;
        self.types[tid.0 as usize].class.is_final = mods.is_final;

        self.read_clauses(tid, clauses);
        self.compute_supers(tid);

        self.class_stack.push(tid);
        let pending = self.class_members(tid, body);
        self.apply_traits(tid, decl.name.span);
        self.check_replacements(tid);
        self.class_bodies(tid, pending);
        self.class_stack.pop();
        self.check_conformance(tid, decl.name.span);
    }

    /// `extends(Base)`, `implements(A, B)`, `use(T)`: in this order, each once.
    fn read_clauses(&mut self, tid: TypeId, clauses: &[Expr]) {
        let kind = self.def(tid).kind;
        let name = self.def(tid).name.clone();
        let mut last_rank = None;
        for clause in clauses {
            let ExprKind::Call { callee, args, .. } = &clause.kind else {
                continue;
            };
            let ExprKind::Name(head) = &callee.kind else {
                continue;
            };
            let rank = match head.name.as_str() {
                "extends" => 0,
                "implements" => 1,
                _ => 2,
            };
            if last_rank.is_some_and(|last| rank <= last) {
                self.error(
                    clause.span,
                    "the clauses come in the order extends, implements, use, each once",
                );
            }
            last_rank = Some(rank);
            let allowed = matches!(
                (kind, head.name.as_str()),
                (DefinedKind::Class, _)
                    | (DefinedKind::Interface, "extends")
                    | (DefinedKind::Trait, "use")
            );
            if !allowed {
                let word = match kind {
                    DefinedKind::Struct => "a struct",
                    DefinedKind::Enum => "an enum",
                    DefinedKind::Union | DefinedKind::RawUnion => "a union",
                    DefinedKind::Interface => "an interface",
                    DefinedKind::Trait => "a trait",
                    DefinedKind::Class => "a class",
                };
                self.error(
                    clause.span,
                    format!(
                        "{word} has no {} clause: only a class takes extends, implements and use",
                        head.name
                    ),
                );
                continue;
            }
            for arg in args {
                let Some((id, targs)) = self.type_reference(&arg.value) else {
                    continue;
                };
                let target_kind = self.def(id).kind;
                let target_name = self.def(id).name.clone();
                match head.name.as_str() {
                    "extends" if kind == DefinedKind::Interface => {
                        if target_kind != DefinedKind::Interface {
                            self.error(arg.value.span, format!("{target_name} is not an interface: an interface extends interfaces"));
                        } else {
                            self.types[tid.0 as usize]
                                .class
                                .interfaces
                                .push((id, targs));
                        }
                    }
                    "extends" => {
                        if target_kind != DefinedKind::Class {
                            self.error(
                                arg.value.span,
                                format!(
                                    "{target_name} is not a class: only a class can be extended"
                                ),
                            );
                        } else if self.def(id).class.is_final {
                            self.error(
                                arg.value.span,
                                format!("{target_name} is final: it cannot be extended"),
                            );
                        } else if self.types[tid.0 as usize].class.base.is_some() {
                            self.error(arg.value.span, "a class extends one class");
                        } else {
                            self.types[tid.0 as usize].class.base = Some((id, targs));
                        }
                    }
                    "implements" => {
                        if target_kind != DefinedKind::Interface {
                            let hint = if target_kind == DefinedKind::Class {
                                format!(
                                    "{target_name} is a class: write implements(IError) or extends({target_name})"
                                )
                            } else {
                                format!(
                                    "{target_name} is not an interface: only an interface can be implemented"
                                )
                            };
                            self.error(arg.value.span, hint);
                        } else {
                            self.types[tid.0 as usize]
                                .class
                                .interfaces
                                .push((id, targs));
                        }
                    }
                    _ => {
                        if target_kind != DefinedKind::Trait {
                            self.error(
                                arg.value.span,
                                format!("{target_name} is not a trait: only a trait can be used"),
                            );
                        } else {
                            self.types[tid.0 as usize].class.traits.push(id);
                        }
                    }
                }
            }
        }
        let _ = name;
    }

    /// A type named in a clause: `People`, or `Base<i32>`.
    fn type_reference(&mut self, expr: &Expr) -> Option<(TypeId, Vec<Ty>)> {
        match &expr.kind {
            ExprKind::Name(name) => match self.type_named(&name.name) {
                Some(id) => Some((id, Vec::new())),
                None => {
                    self.undeclared(name);
                    None
                }
            },
            ExprKind::TypeApply { target, type_args } => {
                let ExprKind::Name(name) = &target.kind else {
                    return None;
                };
                let id = self.type_named(&name.name);
                let Some(id) = id else {
                    self.undeclared(name);
                    return None;
                };
                let args = type_args.iter().map(|t| self.resolve_type(t)).collect();
                Some((id, args))
            }
            _ => {
                self.error(expr.span, "a clause names a type");
                None
            }
        }
    }

    /// Every type this one can be used as: the class it extends and the interfaces it implements, and what they extend.
    fn compute_supers(&mut self, tid: TypeId) {
        let mut supers: Vec<TypeId> = Vec::new();
        let info = self.def(tid).class.clone();
        let add = |supers: &mut Vec<TypeId>, id: TypeId| {
            if id != tid && !supers.contains(&id) {
                supers.push(id);
            }
        };
        if let Some((base, _)) = &info.base {
            add(&mut supers, *base);
            for s in self.def(*base).supers.clone() {
                add(&mut supers, s);
            }
        }
        for (interface, _) in &info.interfaces {
            add(&mut supers, *interface);
            for s in self.def(*interface).supers.clone() {
                add(&mut supers, s);
            }
        }
        self.types[tid.0 as usize].supers = supers;
    }

    // ---- the members

    /// Reads the signatures of the members and keeps the bodies to read after them.
    fn class_members<'a>(&mut self, tid: TypeId, body: &'a [Stmt]) -> Vec<Pending<'a>> {
        let kind = self.def(tid).kind;
        let name = self.def(tid).name.clone();
        let mut pending = Vec::new();
        for stmt in body {
            match &stmt.kind {
                StmtKind::Expr(expr) => match &expr.kind {
                    ExprKind::Param {
                        ty,
                        name: field,
                        default,
                        ..
                    } => {
                        let resolved = self.resolve_type(ty);
                        if let Some(default) = default
                            && let Some((get, set)) = accessor_parts(default)
                        {
                            self.accessor_member(
                                tid,
                                stmt,
                                field,
                                Some(resolved),
                                get,
                                set,
                                &mut pending,
                            );
                            continue;
                        }
                        let mods = self.modifiers(&stmt.attrs, Target::Field);
                        if self.member_exists(tid, &field.name) {
                            self.error(
                                field.span,
                                format!("`{}` is already a member of {name}", field.name),
                            );
                            continue;
                        }
                        if default.is_none() && kind == DefinedKind::Class {
                            self.error(
                                field.span,
                                format!(
                                    "a field of a class always has a value: write {} {} = ...",
                                    ty, field.name
                                ),
                            );
                        }
                        if let Some(default) = default {
                            let got = self.expr(default, Some(&resolved));
                            self.expect_assignable(&resolved, &got, default);
                        }
                        self.types[tid.0 as usize].fields.push(FieldDef {
                            name: field.name.clone(),
                            ty: resolved,
                            has_default: default.is_some(),
                            mods,
                            owner: tid,
                            readonly: false,
                        });
                    }
                    _ => self.error(stmt.span, "a class holds fields, methods and accessors"),
                },
                StmtKind::Decl(decl) => self.declaration_member(tid, stmt, decl, &mut pending),
                _ => self.error(stmt.span, "a class holds fields, methods and accessors"),
            }
        }
        pending
    }

    fn member_exists(&self, tid: TypeId, name: &str) -> bool {
        let def = self.def(tid);
        def.fields.iter().any(|f| f.name == name)
            || def.class.versions.contains_key(name)
            || def.class.accessors.contains_key(name)
    }

    fn declaration_member<'a>(
        &mut self,
        tid: TypeId,
        stmt: &Stmt,
        decl: &'a Decl,
        pending: &mut Vec<Pending<'a>>,
    ) {
        let name = self.def(tid).name.clone();
        let member = &decl.name;
        // the versions of a method are a chain of method definitions
        let versions: Vec<&'a Expr> = match &decl.init.kind {
            ExprKind::Chain(parts) if parts.iter().all(is_method_definition) => {
                parts.iter().collect()
            }
            _ if is_method_definition(&decl.init) => vec![&decl.init],
            _ => Vec::new(),
        };
        if !versions.is_empty() {
            self.method_member(tid, stmt, member, versions, pending);
            return;
        }
        if let Some((get, set)) = accessor_parts(&decl.init) {
            self.accessor_member(tid, stmt, member, None, get, set, pending);
            return;
        }
        if super::functions::is_definition(&decl.init) {
            self.error(
                decl.init.span,
                "a method of a class is written method, not function: const name = method() { ... }",
            );
            return;
        }
        // a field whose type comes from its value: `let age = 0`, `const species = "human"`
        if self.member_exists(tid, &member.name) {
            self.error(
                member.span,
                format!("`{}` is already a member of {name}", member.name),
            );
            return;
        }
        let mods = self.modifiers(&stmt.attrs, Target::Field);
        let got = self.expr(&decl.init, None);
        let ty = if got == Ty::Void {
            self.error(
                decl.init.span,
                "this gives no value, so it cannot be a field",
            );
            Ty::Error
        } else {
            got
        };
        self.types[tid.0 as usize].fields.push(FieldDef {
            name: member.name.clone(),
            ty,
            has_default: true,
            mods,
            owner: tid,
            readonly: decl.kind == DeclKind::Const,
        });
    }

    /// `const greet = method() String { ... } method(String name) String { ... }`
    fn method_member<'a>(
        &mut self,
        tid: TypeId,
        stmt: &Stmt,
        member: &Ident,
        versions: Vec<&'a Expr>,
        pending: &mut Vec<Pending<'a>>,
    ) {
        let kind = self.def(tid).kind;
        let owner_name = self.def(tid).name.clone();
        let special = matches!(member.name.as_str(), "constructor" | "destructor");
        if special && !stmt.attrs.is_empty() {
            self.error(
                stmt.attrs[0].span,
                format!("a {} takes no attributes", member.name),
            );
        }
        let mods = if special {
            Modifiers::default()
        } else {
            self.modifiers(&stmt.attrs, Target::Method)
        };
        if !special && self.member_exists(tid, &member.name) {
            self.error(member.span, format!("`{}` is already a member of {owner_name}: all versions of a name are written in one chain", member.name));
            return;
        }
        for call in versions {
            let ExprKind::Call {
                type_args,
                args,
                trailer: Some(trailer),
                ..
            } = &call.kind
            else {
                continue;
            };
            let own = self.type_parameters(type_args);
            self.type_params.push(own.clone());
            let (mut sig, _) = self.signature(args, trailer.ret.as_ref(), own);
            self.type_params.pop();
            let has_body = !trailer.described;
            if trailer.ret.is_none() {
                sig.ret = Box::new(if has_body { Ty::Error } else { Ty::Void });
            }
            match member.name.as_str() {
                "constructor" => {
                    if trailer.ret.is_some() {
                        self.error(call.span, "the constructor has no return type");
                    }
                    if !has_body {
                        self.error(call.span, "a constructor needs a body");
                    }
                    if kind != DefinedKind::Class {
                        self.error(call.span, "only a class has a constructor");
                    }
                    if self
                        .def(tid)
                        .class
                        .constructors
                        .iter()
                        .any(|c| same_params(c, &sig))
                    {
                        self.error(call.span, "the same parameter types twice: two versions of the constructor cannot be told apart");
                    }
                    sig.ret = Box::new(Ty::Void);
                    self.types[tid.0 as usize].class.constructors.push(sig);
                    pending.push(Pending::Method {
                        call,
                        is_static: false,
                    });
                    continue;
                }
                "destructor" => {
                    if !sig.params.is_empty() {
                        self.error(call.span, "a destructor has no parameters");
                    }
                    if trailer.ret.is_some() {
                        self.error(call.span, "a destructor has no return type");
                    }
                    if self.def(tid).class.has_destructor {
                        self.error(call.span, "a class has one destructor");
                    }
                    self.types[tid.0 as usize].class.has_destructor = true;
                    pending.push(Pending::Method {
                        call,
                        is_static: false,
                    });
                    continue;
                }
                _ => {}
            }
            // abstract and body
            let mut mods = mods;
            if kind == DefinedKind::Interface && has_body {
                self.error(call.span, "an interface has only abstract methods: write [method(Inheritance.Abstract)] and no body");
            }
            if !has_body && !mods.is_abstract {
                self.error(
                    call.span,
                    "write [method(Inheritance.Abstract)], or give a body: a method without a body is always abstract",
                );
                mods.is_abstract = true;
            }
            if has_body && mods.is_abstract && kind != DefinedKind::Interface {
                self.error(call.span, "an abstract method has no body");
            }
            if mods.is_abstract && kind == DefinedKind::Class && !self.def(tid).class.is_abstract {
                self.error(call.span, format!("{owner_name} has an abstract method, so the class must be abstract: [class(Inheritance.Abstract)]"));
            }
            if self
                .def(tid)
                .class
                .versions
                .get(&member.name)
                .is_some_and(|v| v.iter().any(|m| same_params(&m.sig, &sig)))
            {
                self.error(
                    call.span,
                    format!(
                        "the same parameter types twice: two versions of {} cannot be told apart",
                        member.name
                    ),
                );
                continue;
            }
            if !mods.is_static {
                self.register_operator(&stmt.attrs, &sig, Some(tid), call.span);
            }
            self.check_attributes(&stmt.attrs, super::attributes::METHOD);
            self.types[tid.0 as usize]
                .class
                .versions
                .entry(member.name.clone())
                .or_default()
                .push(MethodDef {
                    sig,
                    mods,
                    owner: tid,
                    has_body,
                    span: call.span,
                });
            if has_body {
                pending.push(Pending::Method {
                    call,
                    is_static: mods.is_static,
                });
            }
        }
    }

    /// `const celsius = get { ... } set |value| { ... }`, or `i32 pin = set |value| { ... }`.
    #[allow(clippy::too_many_arguments)]
    fn accessor_member<'a>(
        &mut self,
        tid: TypeId,
        stmt: &Stmt,
        member: &Ident,
        declared: Option<Ty>,
        get: Option<&'a Trailer>,
        set: Option<&'a Trailer>,
        pending: &mut Vec<Pending<'a>>,
    ) {
        let owner_name = self.def(tid).name.clone();
        if self.member_exists(tid, &member.name) {
            self.error(
                member.span,
                format!("`{}` is already a member of {owner_name}", member.name),
            );
            return;
        }
        let mods = self.modifiers(&stmt.attrs, Target::Accessor);
        // [get(...)] and [set(...)] set the visibility of one part, [accessor(...)] of both
        let part =
            |name: &str, fallback: Visibility, attrs: &[Attribute], checker: &mut Checker| {
                for attr in attrs.iter().filter(|a| a.name.name == name) {
                    for arg in &attr.args {
                        if let Some(Modifier::Visibility(v)) = checker.decode_modifier_quiet(arg) {
                            return v;
                        }
                    }
                }
                fallback
            };
        let get_vis = part("get", mods.visibility, &stmt.attrs, self);
        let set_vis = part("set", mods.visibility, &stmt.attrs, self);
        if get.is_none() && set.is_none() {
            self.error(member.span, "an accessor has a get, a set, or both");
            return;
        }
        let def = AccessorDef {
            ty: declared.clone().unwrap_or(Ty::Error),
            get: get.map(|_| get_vis),
            set: set.map(|_| set_vis),
            mods,
            owner: tid,
        };
        self.types[tid.0 as usize]
            .class
            .accessors
            .insert(member.name.clone(), def);
        pending.push(Pending::Accessor {
            name: member.name.clone(),
            span: member.span,
            get,
            set,
            declared,
            mods,
        });
    }

    /// Reads a modifier without reporting: the attribute was checked already.
    fn decode_modifier_quiet(&mut self, arg: &Arg) -> Option<Modifier> {
        let reported = self.diagnostics.len();
        let m = self.decode_modifier(arg);
        self.diagnostics.truncate(reported);
        m
    }

    // ---- the bodies

    fn class_bodies(&mut self, tid: TypeId, pending: Vec<Pending<'_>>) {
        let this = self.this_ty_of(tid);
        let sup = self
            .def(tid)
            .class
            .base
            .clone()
            .map(|(b, a)| self.defined_ty(b, a));
        // accessors first: a method can use the type that an accessor gives
        for item in &pending {
            if let Pending::Accessor {
                name,
                get,
                set,
                declared,
                mods,
                span,
            } = item
            {
                self.check_accessor(
                    tid,
                    name,
                    *get,
                    *set,
                    declared.clone(),
                    *mods,
                    *span,
                    &this,
                    sup.as_ref(),
                );
            }
        }
        for item in &pending {
            if let Pending::Method {
                call, is_static, ..
            } = item
            {
                self.method_this = if *is_static { None } else { Some(this.clone()) };
                self.method_super = if *is_static { None } else { sup.clone() };
                let outer = std::mem::replace(&mut self.in_static, *is_static);
                let got = self.expr(call, None);
                self.in_static = outer;
                self.method_this = None;
                self.method_super = None;
                // a method whose type was worked out from its body is known now
                if let (Ty::Function(f), ExprKind::Call { .. }) = (got, &call.kind) {
                    self.fill_inferred(tid, call, f);
                }
            }
        }
    }

    /// Writes the type that was worked out for a method into the version it belongs to.
    fn fill_inferred(&mut self, tid: TypeId, call: &Expr, f: FnTy) {
        let span = call.span;
        for versions in self.types[tid.0 as usize].class.versions.values_mut() {
            for version in versions.iter_mut() {
                if version.span == span {
                    version.sig = f.clone();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_accessor(
        &mut self,
        tid: TypeId,
        name: &str,
        get: Option<&Trailer>,
        set: Option<&Trailer>,
        declared: Option<Ty>,
        mods: Modifiers,
        span: Span,
        this: &Ty,
        sup: Option<&Ty>,
    ) {
        let outer_static = std::mem::replace(&mut self.in_static, mods.is_static);
        let outer_touched = std::mem::replace(&mut self.touched_static, false);
        let locals = |extra: Option<(&str, Ty)>| {
            let mut l = Vec::new();
            if !mods.is_static {
                l.push(super::functions::local("this", span, this.clone()));
                if let Some(s) = sup {
                    l.push(super::functions::local("super", span, s.clone()));
                }
            }
            if let Some((n, t)) = extra {
                l.push(super::functions::local(n, span, t));
            }
            l
        };
        let mut ty = declared;
        if let Some(get) = get {
            let found = self.function_body(locals(None), &get.body, None);
            if ty.is_none() {
                ty = Some(found);
            }
        }
        if let (Some(set), Some(ty)) = (set, ty.as_ref()) {
            let param = set.params.first();
            if set.params.len() != 1 {
                self.error(
                    span,
                    "set receives the new value as one name: set |value| { ... }",
                );
            }
            let value = param.map_or("value".to_string(), |p| p.name.clone());
            let ps = param.map_or(span, |p| p.span);
            let local = super::functions::local(&value, ps, ty.clone());
            let mut l = locals(None);
            l.push(local);
            self.function_body(l, &set.body, Some(Ty::Void));
        }
        let touched = std::mem::replace(&mut self.touched_static, outer_touched);
        if touched && !mods.is_static {
            self.error(span, "this accessor reads or assigns a static field, so it must be static too: [accessor(Ownership.Static)]");
        }
        self.in_static = outer_static;
        let ty = ty.unwrap_or(Ty::Error);
        if let Some(def) = self.types[tid.0 as usize].class.accessors.get_mut(name) {
            def.ty = ty;
        }
    }

    pub(super) fn this_ty_of(&self, tid: TypeId) -> Ty {
        let args = self
            .def(tid)
            .generics
            .iter()
            .map(|g| Ty::TypeParam(g.clone()))
            .collect();
        self.defined_ty(tid, args)
    }
}

/// A body to read after all the signatures are known.
pub(super) enum Pending<'a> {
    Method {
        call: &'a Expr,
        is_static: bool,
    },
    Accessor {
        name: String,
        span: Span,
        get: Option<&'a Trailer>,
        set: Option<&'a Trailer>,
        declared: Option<Ty>,
        mods: Modifiers,
    },
}

fn is_method_definition(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::Call { callee, .. }
        if matches!(&callee.kind, ExprKind::Name(name) if name.name == "method"))
}

/// The parts of an accessor, `get { } set |value| { }`: the block of each.
pub(super) fn accessor_parts(expr: &Expr) -> Option<(Option<&Trailer>, Option<&Trailer>)> {
    let ExprKind::Call {
        callee, trailer, ..
    } = &expr.kind
    else {
        return None;
    };
    match &callee.kind {
        ExprKind::Name(name) if name.name == "get" => trailer.as_ref().map(|t| (Some(t), None)),
        ExprKind::Name(name) if name.name == "set" => trailer.as_ref().map(|t| (None, Some(t))),
        ExprKind::Member { object, name } if name.name == "set" => {
            let ExprKind::Call {
                callee: inner,
                trailer: Some(get),
                ..
            } = &object.kind
            else {
                return None;
            };
            let ExprKind::Name(head) = &inner.kind else {
                return None;
            };
            (head.name == "get").then_some((Some(get), trailer.as_ref()))
        }
        _ => None,
    }
}

/// Do two versions take the same types, whatever the names are?
pub(super) fn same_params(a: &FnTy, b: &FnTy) -> bool {
    a.params.len() == b.params.len() && a.params.iter().zip(&b.params).all(|(x, y)| x.ty == y.ty)
}
