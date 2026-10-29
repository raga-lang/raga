//! What a class has to keep after its members are read: the traits it uses, the methods it replaces, and the promises
//! of the interfaces it implements and the abstract methods it inherits.

use std::collections::HashMap;

use super::checker::Checker;
use super::classes::{MethodDef, same_params};
use super::ty::{DefinedKind, FnTy, Ty, TypeId};
use crate::span::Span;

impl Checker {
    /// `use(Describes)`: everything in the trait becomes part of the class, as if it had been written there.
    ///
    /// A member the class writes itself wins over the trait, and a trait wins over what the class inherits. Two traits
    /// with the same method are an error, unless the class writes that method itself.
    pub(super) fn apply_traits(&mut self, tid: TypeId, span: Span) {
        let traits = self.def(tid).class.traits.clone();
        let class_name = self.def(tid).name.clone();
        // what each name was taken from, for the report of a clash
        let mut provided: HashMap<(String, usize), (TypeId, FnTy)> = HashMap::new();
        // what the class writes itself, before any trait adds to it
        let written: HashMap<String, Vec<FnTy>> = self
            .def(tid)
            .class
            .versions
            .iter()
            .map(|(name, versions)| {
                (
                    name.clone(),
                    versions.iter().map(|v| v.sig.clone()).collect(),
                )
            })
            .collect();
        for used in traits {
            let trait_name = self.def(used).name.clone();
            // fields
            for field in self.def(used).fields.clone() {
                if self.def(tid).fields.iter().any(|f| f.name == field.name) {
                    continue;
                }
                let mut copy = field.clone();
                copy.owner = tid;
                self.types[tid.0 as usize].fields.push(copy);
            }
            // accessors
            for (name, accessor) in self.def(used).class.accessors.clone() {
                if self.member_taken(tid, &name) {
                    continue;
                }
                let mut copy = accessor.clone();
                copy.owner = tid;
                self.types[tid.0 as usize]
                    .class
                    .accessors
                    .insert(name, copy);
            }
            // methods with a body
            for (name, versions) in self.def(used).class.versions.clone() {
                for version in versions {
                    if !version.has_body {
                        continue;
                    }
                    let own = written
                        .get(&name)
                        .is_some_and(|own| own.iter().any(|sig| same_params(sig, &version.sig)));
                    if own {
                        continue;
                    }
                    let key = (name.clone(), version.sig.params.len());
                    if let Some((other, sig)) = provided.get(&key)
                        && same_params(sig, &version.sig)
                        && *other != used
                    {
                        let other_name = self.def(*other).name.clone();
                        self.error(
                            span,
                            format!("{other_name} and {trait_name} both have {name}, write {name} in {class_name}"),
                        );
                        continue;
                    }
                    provided.insert(key, (used, version.sig.clone()));
                    let mut copy = version.clone();
                    copy.owner = tid;
                    self.types[tid.0 as usize]
                        .class
                        .versions
                        .entry(name.clone())
                        .or_default()
                        .push(copy);
                }
            }
        }
    }

    fn member_taken(&self, tid: TypeId, name: &str) -> bool {
        let def = self.def(tid);
        def.fields.iter().any(|f| f.name == name)
            || def.class.accessors.contains_key(name)
            || def.class.versions.contains_key(name)
    }

    /// The class and the classes it extends, the nearest first.
    pub(super) fn class_chain(&self, tid: TypeId) -> Vec<TypeId> {
        let mut chain = vec![tid];
        let mut current = tid;
        while let Some((base, _)) = self.def(current).class.base {
            if chain.contains(&base) {
                break;
            }
            chain.push(base);
            current = base;
        }
        chain
    }

    /// The versions of a method named `name` that the classes above `tid` have.
    fn inherited_versions(&self, tid: TypeId, name: &str) -> Vec<MethodDef> {
        let mut found = Vec::new();
        for ancestor in self.class_chain(tid).into_iter().skip(1) {
            if let Some(versions) = self.def(ancestor).class.versions.get(name) {
                found.extend(versions.iter().cloned());
            }
        }
        found
    }

    /// A method that replaces one of the class above: it must be virtual there, and not final. One that is not virtual is
    /// hidden, which is not an error.
    pub(super) fn check_replacements(&mut self, tid: TypeId) {
        if self.def(tid).kind != DefinedKind::Class {
            return;
        }
        let class_name = self.def(tid).name.clone();
        let base_name = self
            .def(tid)
            .class
            .base
            .as_ref()
            .map(|(b, _)| self.def(*b).name.clone());
        let own = self.def(tid).class.versions.clone();
        for (name, versions) in own {
            for (index, version) in versions.iter().enumerate() {
                if version.mods.is_static || version.owner != tid {
                    continue;
                }
                let base = self
                    .inherited_versions(tid, &name)
                    .into_iter()
                    .find(|b| same_params(&b.sig, &version.sig));
                match base {
                    Some(base) => {
                        let owner_name = self.def(base.owner).name.clone();
                        if base.mods.is_final {
                            self.error(
                                version.span,
                                format!("{name} is final in {owner_name}: it cannot be replaced"),
                            );
                        } else if version.mods.is_override && !base.replaceable_public() {
                            self.error(
                                version.span,
                                format!("{name} of {owner_name} is not virtual, so there is nothing to override"),
                            );
                        } else if base.replaceable_public() {
                            // a method that replaces another is virtual itself
                            if let Some(v) = self.types[tid.0 as usize]
                                .class
                                .versions
                                .get_mut(&name)
                                .and_then(|v| v.get_mut(index))
                            {
                                v.mods.is_virtual = true;
                            }
                        }
                    }
                    None if version.mods.is_override => {
                        let message = match &base_name {
                            Some(base) => format!("{base} has no virtual {name} to replace"),
                            None => format!(
                                "{class_name} extends no class, so there is no virtual {name} to replace"
                            ),
                        };
                        self.error(version.span, message);
                    }
                    None => {}
                }
            }
        }
    }

    /// Does the class keep what it promised: every abstract method it inherits, every method of its interfaces and of
    /// its traits, and every field an interface lists?
    pub(super) fn check_conformance(&mut self, tid: TypeId, span: Span) {
        if self.def(tid).kind != DefinedKind::Class {
            return;
        }
        let class_name = self.def(tid).name.clone();
        let is_abstract = self.def(tid).class.is_abstract;

        // (name, signature, who asks for it)
        let mut required: Vec<(String, FnTy, String)> = Vec::new();
        for ancestor in self.class_chain(tid).into_iter().skip(1) {
            let owner = self.def(ancestor).name.clone();
            for (name, versions) in &self.def(ancestor).class.versions {
                for v in versions.iter().filter(|v| v.mods.is_abstract) {
                    required.push((name.clone(), v.sig.clone(), owner.clone()));
                }
            }
        }
        let mut interfaces: Vec<TypeId> = Vec::new();
        for chain in self.class_chain(tid) {
            for (interface, _) in self.def(chain).class.interfaces.clone() {
                interfaces.push(interface);
                for s in self.def(interface).supers.clone() {
                    if self.def(s).kind == DefinedKind::Interface && !interfaces.contains(&s) {
                        interfaces.push(s);
                    }
                }
            }
        }
        interfaces.dedup();
        for interface in &interfaces {
            let owner = self.def(*interface).name.clone();
            for (name, versions) in &self.def(*interface).class.versions {
                for v in versions {
                    required.push((name.clone(), v.sig.clone(), owner.clone()));
                }
            }
        }
        for used in self.def(tid).class.traits.clone() {
            let owner = self.def(used).name.clone();
            for (name, versions) in &self.def(used).class.versions {
                for v in versions.iter().filter(|v| v.mods.is_abstract) {
                    required.push((name.clone(), v.sig.clone(), owner.clone()));
                }
            }
        }
        let mut reported: Vec<String> = Vec::new();
        for (name, sig, owner) in required {
            if is_abstract
                && !self
                    .def(tid)
                    .class
                    .traits
                    .iter()
                    .any(|t| self.def(*t).name == owner)
            {
                continue;
            }
            if !self.has_concrete(tid, &name, &sig) && !reported.contains(&name) {
                reported.push(name.clone());
                self.error(
                    span,
                    format!("{class_name} does not have {name}, which {owner} requires"),
                );
            }
        }

        // the fields an interface lists
        let mut needs: Vec<(String, Ty, String)> = Vec::new();
        for interface in &interfaces {
            let owner = self.def(*interface).name.clone();
            for f in &self.def(*interface).fields {
                needs.push((f.name.clone(), f.ty.clone(), owner.clone()));
            }
        }
        for (name, ty, owner) in needs {
            match self.class_member_type(tid, &name) {
                Some(found) if found.accepts(&ty) || ty.accepts(&found) => {}
                Some(found) => self.error(
                    span,
                    format!("{class_name}.{name} is {found}, but {owner} requires {ty}"),
                ),
                None => self.error(
                    span,
                    format!("{class_name} does not have {name}, which {owner} requires"),
                ),
            }
        }
    }

    /// Does the class, or one it extends, have a method with a body, of this name, taking these types?
    fn has_concrete(&self, tid: TypeId, name: &str, sig: &FnTy) -> bool {
        self.class_chain(tid).into_iter().any(|class| {
            self.def(class)
                .class
                .versions
                .get(name)
                .is_some_and(|versions| {
                    versions
                        .iter()
                        .any(|v| v.has_body && same_params(&v.sig, sig))
                })
        })
    }

    /// The type of the field or accessor `name` of a class, own or inherited.
    pub(super) fn class_member_type(&self, tid: TypeId, name: &str) -> Option<Ty> {
        for class in self.class_chain(tid) {
            let def = self.def(class);
            if let Some(f) = def.fields.iter().find(|f| f.name == name) {
                return Some(f.ty.clone());
            }
            if let Some(a) = def.class.accessors.get(name) {
                return Some(a.ty.clone());
            }
        }
        None
    }
}

impl MethodDef {
    pub(super) fn replaceable_public(&self) -> bool {
        self.mods.is_virtual || self.mods.is_abstract || self.mods.is_override
    }
}
