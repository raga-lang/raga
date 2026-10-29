//! The types a program defines: structs, enums, unions, and the generic tagged unions `Optional` and `Result`
//! (05-structs-enums-unions).
//!
//! A type is a value of the language like any other, `const Point = struct { ... }`, so defining one is a declaration
//! whose value is the call of `struct`, `enum` or `union` with a block. This module reads that block, keeps what it says
//! about the type in a [`TypeDef`], and knows how a value of the type is made, read, changed and copied.

use std::collections::HashMap;

use super::checker::Checker;
use super::classes::{ClassInfo, Modifiers};
use super::scope::{Binding, Defines};
use super::ty::{DefinedKind, FnTy, IntTy, ParamTy, Ty, TypeId};
use crate::ast::{
    self, Arg, Decl, DeclKind, Expr, ExprKind, Ident, Stmt, StmtKind, Trailer, UnaryOp,
};
use crate::span::Span;

/// A field of a struct or an untagged union.
#[derive(Debug, Clone)]
pub(super) struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
    /// What the attributes above a field of a class say. A field of a struct has none.
    pub mods: Modifiers,
    /// The type that declares the field, which decides who may use it.
    pub owner: TypeId,
    /// True for `const` fields, which cannot be assigned.
    pub readonly: bool,
}

impl FieldDef {
    pub(super) fn new(name: &str, ty: Ty, has_default: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            has_default,
            mods: Modifiers::default(),
            owner: TypeId(u32::MAX),
            readonly: false,
        }
    }
}

/// A variant of a tagged union: the struct that holds its data.
#[derive(Debug, Clone)]
pub(super) struct VariantDef {
    pub name: String,
    pub def: TypeId,
    /// True for a struct that exists on its own and is named in the union, false for one declared inside it.
    pub outside: bool,
}

/// What the program says about one defined type. The type parameters of a generic type appear in the types of the fields,
/// the methods and the variants as [`Ty::TypeParam`], and a use of the type fills them in.
#[derive(Debug, Clone)]
pub(super) struct TypeDef {
    pub name: String,
    pub kind: DefinedKind,
    pub generics: Vec<String>,
    /// The fields of a struct, of an untagged union, and of the struct of a variant.
    pub fields: Vec<FieldDef>,
    pub methods: HashMap<String, FnTy>,
    /// The members of an enum with their numbers.
    pub members: Vec<(String, i128)>,
    /// The variants of a tagged union.
    pub variants: Vec<VariantDef>,
    /// Every type this one can be used as, all the way up: for a class, the class it extends and the interfaces it
    /// implements, and what those extend.
    pub supers: Vec<TypeId>,
    /// What a class, an interface or a trait has besides fields.
    pub class: ClassInfo,
}

impl TypeDef {
    fn new(name: &str, kind: DefinedKind, generics: Vec<String>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            kind,
            generics,
            fields: Vec::new(),
            methods: HashMap::new(),
            members: Vec::new(),
            variants: Vec::new(),
            supers: Vec::new(),
            class: ClassInfo::default(),
        }
    }
}

pub(super) use super::ty::{
    ANY, ATTRIBUTE_INFO, BINDING_INFO, ERROR, FIELD_INFO, IERROR, KIND, OP, OPTIONAL, RESULT,
    TYPE_INFO, VARIANT_INFO, WEAK,
};

/// `Optional<T>` and `Result<T, E>`, which come with the language and are made like any tagged union:
/// `union<T> { const Some = struct { T value }; const None = struct { } }`.
pub(super) fn builtin_types() -> Vec<TypeDef> {
    let param = |name: &str| Ty::TypeParam(name.to_string());
    let field = |name: &str, ty: Ty| FieldDef::new(name, ty, false);
    let variant_struct = |name: &str, generics: &[&str], fields: Vec<FieldDef>| {
        let mut def = TypeDef::new(
            name,
            DefinedKind::Struct,
            generics.iter().map(ToString::to_string).collect(),
        );
        def.fields = fields;
        def
    };
    let function = |params: Vec<(&str, Ty)>, ret: Ty| FnTy {
        generics: Vec::new(),
        params: params
            .into_iter()
            .map(|(name, ty)| ParamTy {
                name: Some(name.to_string()),
                ty,
                has_default: false,
                variadic: false,
            })
            .collect(),
        ret: Box::new(ret),
    };
    let callback = |params: Vec<Ty>, ret: Ty| {
        Ty::Function(FnTy {
            generics: Vec::new(),
            params: params
                .into_iter()
                .map(|ty| ParamTy {
                    name: None,
                    ty,
                    has_default: false,
                    variadic: false,
                })
                .collect(),
            ret: Box::new(ret),
        })
    };
    let mut optional = TypeDef::new("Optional", DefinedKind::Union, vec!["T".to_string()]);
    let mut result = TypeDef::new(
        "Result",
        DefinedKind::Union,
        vec!["T".to_string(), "E".to_string()],
    );
    optional.variants = vec![
        VariantDef {
            name: "Some".into(),
            def: TypeId(2),
            outside: false,
        },
        VariantDef {
            name: "None".into(),
            def: TypeId(3),
            outside: false,
        },
    ];
    result.variants = vec![
        VariantDef {
            name: "Ok".into(),
            def: TypeId(4),
            outside: false,
        },
        VariantDef {
            name: "Err".into(),
            def: TypeId(5),
            outside: false,
        },
    ];
    // what a failure can be turned into: the value, a fallback, the result of a block, or a thrown Error
    for (def, failure) in [(&mut optional, vec![]), (&mut result, vec![param("E")])] {
        def.methods
            .insert("or_throw".into(), function(vec![], param("T")));
        def.methods
            .insert("unwrap".into(), function(vec![], param("T")));
        def.methods.insert(
            "unwrapOr".into(),
            function(vec![("fallback", param("T"))], param("T")),
        );
        def.methods.insert(
            "or_else".into(),
            function(vec![("handler", callback(failure, param("T")))], param("T")),
        );
    }
    // Error is a class that comes with the language and implements IError, the list of what every error has
    let mut ierror = TypeDef::new("IError", DefinedKind::Interface, Vec::new());
    ierror.fields = vec![field("message", Ty::String), field("code", Ty::I32)];
    let mut error = TypeDef::new("Error", DefinedKind::Class, Vec::new());
    error.supers = vec![IERROR];
    error.fields = vec![
        FieldDef {
            has_default: true,
            ..field("message", Ty::String)
        },
        FieldDef {
            has_default: true,
            ..field("code", Ty::I32)
        },
    ];
    // Weak<T> refers to a non-primitive value without keeping it alive (08-memory)
    let mut weak = TypeDef::new("Weak", DefinedKind::Struct, vec!["T".to_string()]);
    weak.methods.insert(
        "get".into(),
        function(
            vec![],
            Ty::Defined {
                id: OPTIONAL,
                name: "Optional".into(),
                kind: DefinedKind::Union,
                args: vec![param("T")],
                supers: Vec::new(),
            },
        ),
    );
    weak.methods
        .insert("alive".into(), function(vec![], Ty::Bool));
    // what reflection gives back (09-attributes-and-reflection)
    let named = |id: TypeId, name: &str, kind: DefinedKind, args: Vec<Ty>| Ty::Defined {
        id,
        name: name.to_string(),
        kind,
        args,
        supers: Vec::new(),
    };
    let info = |id: TypeId, name: &str| named(id, name, DefinedKind::Struct, Vec::new());
    let array = |ty: Ty| Ty::Array(Box::new(ty));
    let mut kind_enum = TypeDef::new("Kind", DefinedKind::Enum, Vec::new());
    kind_enum.members = [
        "Primitive",
        "Struct",
        "Enum",
        "Union",
        "Function",
        "Field",
        "Heap",
        "Method",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| (n.to_string(), i as i128))
    .collect();
    let mut op_enum = TypeDef::new("Op", DefinedKind::Enum, Vec::new());
    op_enum.members = [
        "Add", "Sub", "Mul", "Div", "Mod", "Neg", "Not", "Eq", "Lt", "Le", "Gt", "Ge", "BitAnd",
        "BitOr", "BitXor", "BitNot", "Shl", "Shr",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| (n.to_string(), i as i128))
    .collect();
    let kind_ty = named(KIND, "Kind", DefinedKind::Enum, Vec::new());
    let attributes = field("attributes", array(info(ATTRIBUTE_INFO, "AttributeInfo")));
    let mut field_info = variant_struct(
        "FieldInfo",
        &[],
        vec![
            field("name", Ty::String),
            field("type_name", Ty::String),
            attributes.clone(),
        ],
    );
    field_info.kind = DefinedKind::Struct;
    let variant_info = variant_struct(
        "VariantInfo",
        &[],
        vec![
            field("name", Ty::String),
            field("fields", array(info(FIELD_INFO, "FieldInfo"))),
        ],
    );
    let any_ty = named(ANY, "Any", DefinedKind::Struct, Vec::new());
    let attribute_info = variant_struct(
        "AttributeInfo",
        &[],
        vec![
            field("name", Ty::String),
            field("arguments", array(any_ty.clone())),
        ],
    );
    let mut binding_info = variant_struct(
        "BindingInfo",
        &[],
        vec![
            field("name", Ty::String),
            field("kind", kind_ty.clone()),
            field("type_name", Ty::String),
            attributes.clone(),
        ],
    );
    binding_info.methods.insert(
        "invoke".into(),
        function(
            vec![("arguments", array(any_ty.clone()))],
            named(
                RESULT,
                "Result",
                DefinedKind::Union,
                vec![any_ty.clone(), Ty::String],
            ),
        ),
    );
    let type_info = variant_struct(
        "TypeInfo",
        &[],
        vec![
            field("name", Ty::String),
            field("kind", kind_ty),
            field("size", Ty::USIZE),
            field("alignment", Ty::USIZE),
            field("fields", array(info(FIELD_INFO, "FieldInfo"))),
            field("variants", array(info(VARIANT_INFO, "VariantInfo"))),
            attributes,
        ],
    );
    let mut any = TypeDef::new("Any", DefinedKind::Struct, Vec::new());
    any.methods
        .insert("typeName".into(), function(vec![], Ty::String));
    vec![
        optional,
        result,
        variant_struct("Some", &["T"], vec![field("value", param("T"))]),
        variant_struct("None", &["T"], vec![]),
        variant_struct("Ok", &["T", "E"], vec![field("value", param("T"))]),
        variant_struct("Err", &["T", "E"], vec![field("reason", param("E"))]),
        ierror,
        error,
        weak,
        kind_enum,
        op_enum,
        field_info,
        variant_info,
        attribute_info,
        binding_info,
        type_info,
        any,
    ]
}

/// The head of a definition: `struct`, `enum` or `union`.
pub(super) fn is_definition(expr: &Expr) -> bool {
    match &expr.kind {
        // a class with clauses after its body: the definition and the clauses are the parts of a chain
        ExprKind::Chain(parts) => parts.first().is_some_and(is_definition),
        ExprKind::Call { callee, .. } => matches!(&callee.kind, ExprKind::Name(name)
            if matches!(name.name.as_str(), "struct" | "enum" | "union" | "class" | "interface" | "trait")),
        _ => false,
    }
}

fn head_of(expr: &Expr) -> Option<&str> {
    let ExprKind::Call { callee, .. } = &expr.kind else {
        return None;
    };
    match &callee.kind {
        ExprKind::Name(name) => Some(name.name.as_str()),
        _ => None,
    }
}

impl Checker {
    // ---- the table of types

    pub(super) fn def(&self, id: TypeId) -> &TypeDef {
        &self.types[id.0 as usize]
    }

    fn new_type(&mut self, name: &str, kind: DefinedKind, generics: Vec<String>) -> TypeId {
        self.types.push(TypeDef::new(name, kind, generics));
        TypeId(u32::try_from(self.types.len() - 1).expect("too many types"))
    }

    /// A value of the type `id`, with the type arguments filled in.
    pub(super) fn defined_ty(&self, id: TypeId, args: Vec<Ty>) -> Ty {
        let def = self.def(id);
        Ty::Defined {
            id,
            name: def.name.clone(),
            kind: def.kind,
            args,
            supers: def.supers.clone(),
        }
    }

    /// The type itself as a value.
    pub(super) fn type_value_ty(&self, id: TypeId, args: Vec<Ty>) -> Ty {
        let def = self.def(id);
        Ty::TypeValue {
            id,
            name: def.name.clone(),
            kind: def.kind,
            args,
        }
    }

    /// The type of `this` inside the type: its own type parameters are its arguments.
    pub(super) fn this_ty(&self, id: TypeId) -> Ty {
        let args = self
            .def(id)
            .generics
            .iter()
            .map(|g| Ty::TypeParam(g.clone()))
            .collect();
        self.defined_ty(id, args)
    }

    /// Which type parameter is which type, for a use of the type with these arguments.
    pub(super) fn arg_map(&self, id: TypeId, args: &[Ty]) -> HashMap<String, Ty> {
        self.def(id)
            .generics
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect()
    }

    /// The type that a name stands for, if it is a defined type or `Optional` or `Result`.
    pub(super) fn type_named(&self, name: &str) -> Option<TypeId> {
        if let Some(symbol) = self.scopes.lookup(name) {
            return match &self.scopes.symbol(symbol).defines {
                Some(Defines::Type(id)) => Some(*id),
                _ => None,
            };
        }
        match name {
            "Optional" => Some(OPTIONAL),
            "Result" => Some(RESULT),
            "IError" => Some(IERROR),
            "Error" => Some(ERROR),
            "Weak" => Some(WEAK),
            "Kind" => Some(KIND),
            "Op" => Some(OP),
            "FieldInfo" => Some(FIELD_INFO),
            "VariantInfo" => Some(VARIANT_INFO),
            "AttributeInfo" => Some(ATTRIBUTE_INFO),
            "BindingInfo" => Some(BINDING_INFO),
            "TypeInfo" => Some(TYPE_INFO),
            "Any" => Some(ANY),
            _ => None,
        }
    }

    /// The fields of a type with its type arguments filled in.
    fn fields_of(&self, id: TypeId, args: &[Ty]) -> Vec<FieldDef> {
        let map = self.arg_map(id, args);
        self.def(id)
            .fields
            .iter()
            .map(|f| FieldDef {
                ty: f.ty.substitute(&map),
                ..f.clone()
            })
            .collect()
    }

    // ---- defining

    /// `const Name = struct { ... }`, `enum { ... }`, `union { ... }`
    pub(super) fn type_declaration(
        &mut self,
        id: crate::ast::NodeId,
        decl: &Decl,
        attrs: &[crate::ast::Attribute],
    ) {
        // the clauses of a class, `extends(Base) implements(A) use(T)`, follow the definition in a chain
        let (definition, clauses): (&Expr, &[Expr]) = match &decl.init.kind {
            ExprKind::Chain(parts) => (&parts[0], &parts[1..]),
            _ => (&decl.init, &[]),
        };
        let ExprKind::Call {
            type_args,
            trailer: Some(trailer),
            ..
        } = &definition.kind
        else {
            self.error(decl.init.span, "a type needs a body: struct { ... }");
            return;
        };
        let head = head_of(definition).unwrap_or("struct");
        let generics = self.type_parameters(type_args);
        if head == "enum" && !generics.is_empty() {
            self.error(
                decl.init.span,
                "an enum has no data, so it takes no type parameter",
            );
        }
        let kind = match head {
            "enum" => DefinedKind::Enum,
            "union" => {
                let fields = trailer.body.iter().any(|stmt| {
                    matches!(&stmt.kind, StmtKind::Expr(e) if matches!(e.kind, ExprKind::Param { .. }))
                });
                let variants = trailer
                    .body
                    .iter()
                    .any(|stmt| matches!(&stmt.kind, StmtKind::Decl(d) if !is_method(d)));
                if fields && variants {
                    self.error(
                        decl.init.span,
                        "a union holds either fields (untagged) or variants (tagged), not both",
                    );
                }
                if fields {
                    DefinedKind::RawUnion
                } else {
                    DefinedKind::Union
                }
            }
            "class" => DefinedKind::Class,
            "interface" => DefinedKind::Interface,
            "trait" => DefinedKind::Trait,
            _ => DefinedKind::Struct,
        };
        let tid = self.new_type(&decl.name.name, kind, generics.clone());
        // the name is declared before the body is read, so that a type can name itself
        let meta = self.type_value_ty(tid, Vec::new());
        self.declare_with(
            id,
            decl,
            Binding::Const,
            meta,
            None,
            Some(Defines::Type(tid)),
        );
        self.type_params.push(generics);
        if matches!(
            kind,
            DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait
        ) {
            self.class_like(tid, decl, attrs, &trailer.body, clauses);
        } else {
            if let Some(clause) = clauses.first() {
                let word = match kind {
                    DefinedKind::Struct => "a struct",
                    DefinedKind::Enum => "an enum",
                    _ => "a union",
                };
                self.error(
                    clause.span,
                    format!(
                        "{word} has no extends, implements or use clause: only a class takes them"
                    ),
                );
            }
            self.type_body(tid, &trailer.body);
        }
        self.type_params.pop();
    }

    /// Is this expression the name of a type, with type arguments or without, as in `const Id = i32` and
    /// `const Names = List<String>`?
    pub(super) fn is_alias(&self, init: &Expr) -> bool {
        match &init.kind {
            ExprKind::Name(name) => self.names_a_type(&name.name),
            ExprKind::TypeApply { target, .. } => {
                matches!(&target.kind, ExprKind::Name(name) if self.names_a_type(&name.name))
            }
            ExprKind::Member { object, name } => self.module_names_a_type(object, name),
            _ => false,
        }
    }

    /// `const Id = i32`: a second name for a type.
    pub(super) fn alias_declaration(&mut self, id: crate::ast::NodeId, decl: &Decl) {
        let written = match &decl.init.kind {
            ExprKind::Name(name) => crate::ast::Type {
                name: name.clone(),
                args: Vec::new(),
                ret: None,
                span: decl.init.span,
            },
            ExprKind::Member { object, name } => {
                let ExprKind::Name(module) = &object.kind else {
                    return;
                };
                crate::ast::Type {
                    name: Ident {
                        name: format!("{}.{}", module.name, name.name),
                        span: decl.init.span,
                    },
                    args: Vec::new(),
                    ret: None,
                    span: decl.init.span,
                }
            }
            ExprKind::TypeApply { target, type_args } => {
                let ExprKind::Name(name) = &target.kind else {
                    return;
                };
                crate::ast::Type {
                    name: name.clone(),
                    args: type_args.clone(),
                    ret: None,
                    span: decl.init.span,
                }
            }
            _ => return,
        };
        let target = self.resolve_type(&written);
        self.declare_with(
            id,
            decl,
            Binding::Const,
            Ty::Error,
            None,
            Some(Defines::Alias(target)),
        );
    }

    /// Is this name, written where a value is expected, the name of a type?
    pub(super) fn names_a_type(&self, name: &str) -> bool {
        use super::ty::{FloatTy, IntTy};
        if let Some(symbol) = self.scopes.lookup(name) {
            return self.scopes.symbol(symbol).defines.is_some();
        }
        IntTy::from_name(name).is_some()
            || FloatTy::from_name(name).is_some()
            || matches!(
                name,
                "bool"
                    | "char"
                    | "String"
                    | "Array"
                    | "List"
                    | "Set"
                    | "Map"
                    | "Tuple"
                    | "Optional"
                    | "Result"
            )
    }

    fn type_body(&mut self, tid: TypeId, body: &[Stmt]) {
        match self.def(tid).kind {
            DefinedKind::Struct
            | DefinedKind::RawUnion
            | DefinedKind::Class
            | DefinedKind::Trait
            | DefinedKind::Interface => self.fields_and_methods(tid, body),
            DefinedKind::Enum => self.enum_body(tid, body),
            DefinedKind::Union => self.union_body(tid, body),
        }
    }

    /// The fields and the methods of a struct or an untagged union.
    fn fields_and_methods(&mut self, tid: TypeId, body: &[Stmt]) {
        let kind = self.def(tid).kind;
        let name = self.def(tid).name.clone();
        let mut methods: Vec<&Decl> = Vec::new();
        let mut operator_methods: Vec<(&Decl, &Vec<crate::ast::Attribute>)> = Vec::new();
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
                        if self.def(tid).fields.iter().any(|f| f.name == field.name)
                            || self.def(tid).methods.contains_key(&field.name)
                        {
                            self.error(
                                field.span,
                                format!("`{}` is already a member of {name}", field.name),
                            );
                            continue;
                        }
                        if matches!(&resolved, Ty::Defined { id, .. } if *id == tid) {
                            self.error(
                                ty.span,
                                format!("a {name} that holds a {name} could never be created: use Optional<{name}>, List<{name}> or Array<{name}>"),
                            );
                        }
                        if let Some(default) = default {
                            let got = self.expr(default, Some(&resolved));
                            self.expect_assignable(&resolved, &got, default);
                        }
                        let mut def = FieldDef::new(&field.name, resolved, default.is_some());
                        def.owner = tid;
                        self.types[tid.0 as usize].fields.push(def);
                        self.check_attributes(&stmt.attrs, super::attributes::FIELD);
                    }
                    _ => self.error(
                        stmt.span,
                        format!("a {} holds fields, `Type name`, and methods, `const name = function(...) {{ ... }}`", kind_word(kind)),
                    ),
                },
                StmtKind::Decl(decl) if is_method(decl) => {
                    self.method_signature(tid, decl);
                    self.check_attributes(&stmt.attrs, super::attributes::METHOD);
                    methods.push(decl);
                    operator_methods.push((decl, &stmt.attrs));
                }
                _ => self.error(
                    stmt.span,
                    format!("a {} holds fields, `Type name`, and methods, `const name = function(...) {{ ... }}`", kind_word(kind)),
                ),
            }
        }
        self.method_bodies(tid, &methods);
        for (decl, attrs) in operator_methods {
            if let Some(sig) = self.def(tid).methods.get(&decl.name.name).cloned() {
                self.register_operator(attrs, &sig, Some(tid), decl.name.span);
            }
        }
    }

    /// The type of a method, without its body: the body can use every field and every method of the type.
    fn method_signature(&mut self, tid: TypeId, decl: &Decl) {
        let ExprKind::Call {
            type_args,
            args,
            trailer: Some(trailer),
            ..
        } = &decl.init.kind
        else {
            return;
        };
        if self
            .def(tid)
            .fields
            .iter()
            .any(|f| f.name == decl.name.name)
            || self.def(tid).methods.contains_key(&decl.name.name)
        {
            self.error(
                decl.name.span,
                format!(
                    "`{}` is already a member of {}",
                    decl.name.name,
                    self.def(tid).name
                ),
            );
            return;
        }
        let own = self.type_parameters(type_args);
        self.type_params.push(own.clone());
        let (mut sig, _) = self.signature(args, trailer.ret.as_ref(), own);
        self.type_params.pop();
        // a method with no written type is known when its body has been read
        if trailer.ret.is_none() {
            sig.ret = Box::new(Ty::Error);
        }
        self.types[tid.0 as usize]
            .methods
            .insert(decl.name.name.clone(), sig);
    }

    /// Reads the bodies of the methods, with `this` standing for a value of the type.
    fn method_bodies(&mut self, tid: TypeId, methods: &[&Decl]) {
        for decl in methods {
            self.method_this = Some(self.this_ty(tid));
            let got = self.expr(&decl.init, None);
            self.method_this = None;
            if let Ty::Function(f) = got
                && self.def(tid).methods.contains_key(&decl.name.name)
            {
                self.types[tid.0 as usize]
                    .methods
                    .insert(decl.name.name.clone(), f);
            }
        }
    }

    fn enum_body(&mut self, tid: TypeId, body: &[Stmt]) {
        let mut methods: Vec<&Decl> = Vec::new();
        for stmt in body {
            match &stmt.kind {
                StmtKind::Decl(decl) if is_method(decl) => {
                    self.method_signature(tid, decl);
                    methods.push(decl);
                }
                StmtKind::Decl(Decl {
                    kind: DeclKind::Const,
                    name,
                    init,
                }) => {
                    let Some(number) = whole_number(init) else {
                        self.error(
                            init.span,
                            "an enum member is a number written in the code: const Red = 0",
                        );
                        continue;
                    };
                    self.expr(init, None);
                    let def = &self.types[tid.0 as usize];
                    if let Some((other, _)) = def.members.iter().find(|(n, _)| *n == name.name) {
                        let message = format!("`{other}` is declared twice");
                        self.error(name.span, message);
                        continue;
                    }
                    if let Some((other, _)) = def.members.iter().find(|(_, v)| *v == number) {
                        let message = format!("{other} and {} have the same value", name.name);
                        self.error(name.span, message);
                        continue;
                    }
                    self.types[tid.0 as usize]
                        .members
                        .push((name.name.clone(), number));
                }
                StmtKind::Expr(Expr {
                    kind: ExprKind::Name(bare),
                    ..
                }) => self.error(
                    bare.span,
                    format!(
                        "a bare name is not a member: write const {} = number",
                        bare.name
                    ),
                ),
                _ => self.error(
                    stmt.span,
                    "an enum holds members, `const Name = number`, and methods",
                ),
            }
        }
        self.method_bodies(tid, &methods);
    }

    /// The variants of a tagged union: structs declared inside it, or existing structs named in it.
    fn union_body(&mut self, tid: TypeId, body: &[Stmt]) {
        let generics = self.def(tid).generics.clone();
        let mut methods: Vec<&Decl> = Vec::new();
        for stmt in body {
            match &stmt.kind {
                StmtKind::Decl(decl) if is_method(decl) => {
                    self.method_signature(tid, decl);
                    methods.push(decl);
                }
                StmtKind::Decl(decl) if head_of(&decl.init) == Some("struct") => {
                    // declared inside
                    let name = decl.name.name.clone();
                    if self.variant_exists(tid, &name) {
                        self.error(decl.name.span, format!("`{name}` is already a variant"));
                        continue;
                    }
                    if self.type_named(&name).is_some() {
                        self.error(
                            decl.name.span,
                            format!("`{name}` is a struct declared outside already: a name in a union means one thing, pick one"),
                        );
                        continue;
                    }
                    let ExprKind::Call {
                        trailer: Some(inner),
                        ..
                    } = &decl.init.kind
                    else {
                        continue;
                    };
                    let vid = self.new_type(&name, DefinedKind::Struct, generics.clone());
                    self.fields_and_methods(vid, &inner.body);
                    self.types[tid.0 as usize].variants.push(VariantDef {
                        name,
                        def: vid,
                        outside: false,
                    });
                }
                StmtKind::Decl(Decl {
                    kind: DeclKind::Const,
                    name,
                    init:
                        Expr {
                            kind: ExprKind::Name(target),
                            ..
                        },
                }) => {
                    // named from outside
                    let Some(sid) = self.type_named(&target.name) else {
                        self.error(
                            target.span,
                            format!("`{}` is not declared: a name in a union must be an existing struct", target.name),
                        );
                        continue;
                    };
                    if self.def(sid).kind != DefinedKind::Struct {
                        self.error(
                            target.span,
                            format!("`{}` is not a struct, only a struct can be a variant", target.name),
                        );
                        continue;
                    }
                    if self.variant_exists(tid, &name.name) {
                        self.error(name.span, format!("`{}` is already a variant", name.name));
                        continue;
                    }
                    if self.def(tid).variants.iter().any(|v| v.def == sid) {
                        self.error(
                            target.span,
                            format!("the struct {} is in this union twice: the tag is the type of the variant, wrap one in a newtype", target.name),
                        );
                        continue;
                    }
                    self.types[tid.0 as usize].variants.push(VariantDef {
                        name: name.name.clone(),
                        def: sid,
                        outside: true,
                    });
                }
                _ => self.error(
                    stmt.span,
                    "a tagged union holds variants: `const Circle = struct { f64 radius }`, or the name of an existing struct, `const Square = Square`",
                ),
            }
        }
        self.method_bodies(tid, &methods);
    }

    fn variant_exists(&self, tid: TypeId, name: &str) -> bool {
        self.def(tid).variants.iter().any(|v| v.name == name)
    }

    // ---- making values

    /// `Point(x = 1.0, y = 2.0)`, `Color(2)`, `Bits32(i = 0)`, `Box<i32>(1)`: a value of a defined type.
    pub(super) fn construct_defined(
        &mut self,
        call: &Expr,
        id: TypeId,
        explicit: Vec<Ty>,
        args: &[Arg],
        trailer: Option<&Trailer>,
        expected: Option<&Ty>,
    ) -> Ty {
        let name = self.def(id).name.clone();
        if trailer.is_some() {
            self.error(
                call.span,
                format!("creating a {name} takes its values inside the parentheses, no block"),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        if id == WEAK {
            return self.construct_weak(call, args, expected);
        }
        if id == ANY {
            return self.construct_any(call, args);
        }
        match self.def(id).kind {
            DefinedKind::Enum => self.construct_enum(call, id, args),
            DefinedKind::Union => {
                self.error(
                    call.span,
                    format!(
                        "a {name} is created through one of its variants: write {name}.Variant(...)"
                    ),
                );
                self.check_arguments(args);
                Ty::Error
            }
            DefinedKind::Trait => {
                self.error(
                    call.span,
                    format!(
                        "{name} is a trait: a trait cannot be created, a class that uses it is"
                    ),
                );
                self.check_arguments(args);
                Ty::Error
            }
            DefinedKind::Interface => {
                self.error(
                    call.span,
                    format!("{name} is an interface: it lists what a class has, and a class that implements it is created"),
                );
                self.check_arguments(args);
                Ty::Error
            }
            DefinedKind::Class => self.construct_class(call, id, explicit, args, expected),
            DefinedKind::Struct | DefinedKind::RawUnion => {
                let mut known = self.seed_arguments(call, id, explicit, expected);
                let fields = self.def(id).fields.clone();
                let generics = self.def(id).generics.clone();
                let pairs: Vec<(Option<&Ident>, &Expr)> =
                    args.iter().map(|a| (a.name.as_ref(), &a.value)).collect();
                if self.def(id).kind == DefinedKind::RawUnion {
                    self.construct_raw_union(call, &name, &fields, &generics, &pairs, &mut known);
                } else {
                    self.fill_fields(
                        call, &name, &fields, &generics, &pairs, &mut known, true, true,
                    );
                }
                self.finish_arguments(call, id, &known, None)
            }
        }
    }

    /// The type arguments already known when a value is made: those written, else those of the expected type.
    pub(super) fn seed_arguments(
        &mut self,
        call: &Expr,
        id: TypeId,
        explicit: Vec<Ty>,
        expected: Option<&Ty>,
    ) -> HashMap<String, Ty> {
        let generics = self.def(id).generics.clone();
        let name = self.def(id).name.clone();
        if !explicit.is_empty() {
            if explicit.len() == generics.len() {
                return generics.into_iter().zip(explicit).collect();
            }
            self.error(
                call.span,
                format!(
                    "{name} has {} type parameter{}, but {} type argument{} given",
                    generics.len(),
                    if generics.len() == 1 { "" } else { "s" },
                    explicit.len(),
                    if explicit.len() == 1 {
                        " was"
                    } else {
                        "s were"
                    },
                ),
            );
            return HashMap::new();
        }
        match expected {
            Some(Ty::Defined {
                id: wanted, args, ..
            }) if *wanted == id => generics.into_iter().zip(args.iter().cloned()).collect(),
            _ => HashMap::new(),
        }
    }

    /// The type of the value once its arguments are known. A type parameter that nothing decided is an error.
    pub(super) fn finish_arguments(
        &mut self,
        call: &Expr,
        id: TypeId,
        known: &HashMap<String, Ty>,
        variant: Option<&str>,
    ) -> Ty {
        let generics = self.def(id).generics.clone();
        let name = self.def(id).name.clone();
        let mut args = Vec::new();
        for generic in &generics {
            match known.get(generic) {
                Some(ty) => args.push(ty.clone()),
                None => {
                    let written: Vec<String> = generics
                        .iter()
                        .map(|g| {
                            known
                                .get(g)
                                .map_or_else(|| "String".to_string(), ToString::to_string)
                        })
                        .collect();
                    let how = match variant {
                        Some(variant) => format!(
                            "nothing says what {generic} is: write {name}<{}>.{variant}(...)",
                            written.join(", ")
                        ),
                        None => format!(
                            "{generic} cannot be inferred: write {name}<{}>(...)",
                            written.join(", ")
                        ),
                    };
                    self.error(call.span, how);
                    return Ty::Error;
                }
            }
        }
        self.defined_ty(id, args)
    }

    /// Matches the arguments of a creation to the fields: by name, a single field also by position. A field with a
    /// default may be left out. `known` collects the type arguments that the values decide.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill_fields(
        &mut self,
        call: &Expr,
        label: &str,
        fields: &[FieldDef],
        generics: &[String],
        pairs: &[(Option<&Ident>, &Expr)],
        known: &mut HashMap<String, Ty>,
        positional_single: bool,
        require_all: bool,
    ) {
        let mut given: Vec<Option<&Expr>> = vec![None; fields.len()];
        let mut reported = false;
        let mut next_position = 0;
        for (name, value) in pairs {
            match name {
                None => {
                    if positional_single && fields.len() == 1 && given[0].is_none() {
                        given[0] = Some(value);
                    } else {
                        if !reported {
                            let shown: Vec<String> =
                                fields.iter().map(|f| format!("{} = ...", f.name)).collect();
                            self.error(
                                value.span,
                                format!(
                                    "a {label} is created with names: write {label}({})",
                                    shown.join(", ")
                                ),
                            );
                            reported = true;
                        }
                        // the values still fill the fields in order, so that one mistake is reported once
                        match given.get_mut(next_position) {
                            Some(slot @ None) => *slot = Some(value),
                            _ => {
                                self.expr(value, None);
                            }
                        }
                        next_position += 1;
                    }
                }
                Some(field) => match fields.iter().position(|f| f.name == field.name) {
                    None => {
                        self.error(field.span, format!("{label} has no field `{}`", field.name));
                        self.expr(value, None);
                    }
                    Some(i) if given[i].is_some() => {
                        self.error(field.span, format!("`{}` is given twice", field.name));
                        self.expr(value, None);
                    }
                    Some(i) => given[i] = Some(value),
                },
            }
        }
        let mut missing: Vec<&str> = Vec::new();
        for (field, value) in fields.iter().zip(&given) {
            match value {
                Some(value) => self.argument_for(value, &field.ty, generics, known),
                None if require_all && !field.has_default => missing.push(&field.name),
                None => {}
            }
        }
        if !missing.is_empty() {
            let plural = missing.len() > 1;
            self.error(
                call.span,
                format!(
                    "{} {} not given, and {} no default",
                    missing.join(" and "),
                    if plural { "were" } else { "was" },
                    if plural { "have" } else { "has" },
                ),
            );
        }
    }

    fn construct_raw_union(
        &mut self,
        call: &Expr,
        name: &str,
        fields: &[FieldDef],
        generics: &[String],
        pairs: &[(Option<&Ident>, &Expr)],
        known: &mut HashMap<String, Ty>,
    ) {
        match pairs {
            [] => {
                if !fields.first().is_some_and(|f| f.has_default) {
                    self.error(
                        call.span,
                        format!("{name}() needs a view to start with: name one field, or give the first field a default"),
                    );
                }
            }
            [(Some(field), value)] => match fields.iter().find(|f| f.name == field.name) {
                Some(f) => self.argument_for(value, &f.ty, generics, known),
                None => {
                    self.error(field.span, format!("{name} has no field `{}`", field.name));
                    self.expr(value, None);
                }
            },
            [(None, value)] => {
                self.error(
                    value.span,
                    format!("name the field to start with: {name}(field = value)"),
                );
                self.expr(value, None);
            }
            more => {
                self.error(
                    call.span,
                    format!(
                        "a union holds one view at a time: give {name} one field, not {}",
                        more.len()
                    ),
                );
                for (_, value) in more {
                    self.expr(value, None);
                }
            }
        }
    }

    /// `Color(2)` and `Color()`: the member with that number, or the first member.
    fn construct_enum(&mut self, call: &Expr, id: TypeId, args: &[Arg]) -> Ty {
        let ty = self.defined_ty(id, Vec::new());
        match args {
            [] => {
                if self.def(id).members.is_empty() {
                    self.error(call.span, format!("{} has no members", self.def(id).name));
                }
            }
            [arg] => {
                let got = self.expr(&arg.value, Some(&Ty::I32));
                if !got.is_integer() && !got.is_error() {
                    self.error(
                        arg.value.span,
                        format!(
                            "{} is made from a whole number, but this is {got}",
                            self.def(id).name
                        ),
                    );
                }
            }
            _ => {
                self.error(call.span, "an enum is made from one number");
                self.check_arguments(args);
            }
        }
        ty
    }

    // ---- reading and changing

    /// The type of the field `name` of a value of a defined type, or `None` after reporting why there is none.
    pub(super) fn field_type(&mut self, receiver: &Ty, name: &Ident) -> Option<Ty> {
        self.field_type_for(receiver, name, false)
    }

    /// Like [`field_type`](Self::field_type), for a field that is about to be assigned when `write` is true.
    pub(super) fn field_type_for(
        &mut self,
        receiver: &Ty,
        name: &Ident,
        write: bool,
    ) -> Option<Ty> {
        if let Ty::Defined {
            kind: DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait,
            ..
        } = receiver
        {
            return self.class_field_type(receiver, name, write);
        }
        let Ty::Defined {
            id,
            kind,
            args,
            name: type_name,
            ..
        } = receiver
        else {
            return None;
        };
        match kind {
            DefinedKind::Struct
            | DefinedKind::RawUnion
            | DefinedKind::Class
            | DefinedKind::Trait
            | DefinedKind::Interface => {
                let found = self
                    .fields_of(*id, args)
                    .into_iter()
                    .find(|f| f.name == name.name);
                match found {
                    Some(field) => Some(field.ty),
                    None => {
                        self.error(
                            name.span,
                            format!("{type_name} has no field `{}`", name.name),
                        );
                        None
                    }
                }
            }
            DefinedKind::Union => {
                self.error(
                    name.span,
                    format!(
                        "{type_name} keeps its data in a variant, which is read with match: there is no `{}` on it",
                        name.name
                    ),
                );
                None
            }
            DefinedKind::Enum => {
                self.error(
                    name.span,
                    format!("{type_name} has no field `{}`", name.name),
                );
                None
            }
        }
    }

    /// `Type.name` used as a value: a member of an enum, or a variant that has no data.
    pub(super) fn static_member(
        &mut self,
        meta: &Ty,
        name: &Ident,
        expected: Option<&Ty>,
        span: Span,
    ) -> Ty {
        let Ty::TypeValue {
            id,
            name: type_name,
            kind,
            args,
        } = meta
        else {
            return Ty::Error;
        };
        match kind {
            DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait => {
                self.class_static_member(meta, name, false)
            }
            DefinedKind::Enum => {
                if self.def(*id).members.iter().any(|(m, _)| *m == name.name) {
                    return self.defined_ty(*id, Vec::new());
                }
                self.error(name.span, format!("`{}` is not in {type_name}", name.name));
                Ty::Error
            }
            DefinedKind::Union => {
                let Some(variant) = self
                    .def(*id)
                    .variants
                    .iter()
                    .find(|v| v.name == name.name)
                    .cloned()
                else {
                    self.error(
                        name.span,
                        format!("{type_name} has no variant {}", name.name),
                    );
                    return Ty::Error;
                };
                if variant.outside || !self.def(variant.def).fields.is_empty() {
                    self.error(
                        name.span,
                        format!(
                            "{} holds data: write {type_name}.{}(...)",
                            name.name, name.name
                        ),
                    );
                    return Ty::Error;
                }
                let mut known = self.seed_arguments_from(*id, args, expected);
                let call = Expr {
                    id: crate::ast::NodeId(u32::MAX),
                    kind: ExprKind::Error,
                    span,
                };
                self.finish_arguments_named(&call, *id, &mut known, name)
            }
            _ => {
                self.error(
                    name.span,
                    format!("{type_name} has no member `{}`", name.name),
                );
                Ty::Error
            }
        }
    }

    fn seed_arguments_from(
        &mut self,
        id: TypeId,
        explicit: &[Ty],
        expected: Option<&Ty>,
    ) -> HashMap<String, Ty> {
        let generics = self.def(id).generics.clone();
        if !explicit.is_empty() {
            return generics.into_iter().zip(explicit.iter().cloned()).collect();
        }
        match expected {
            Some(Ty::Defined {
                id: wanted, args, ..
            }) if *wanted == id => generics.into_iter().zip(args.iter().cloned()).collect(),
            _ => HashMap::new(),
        }
    }

    /// Like [`finish_arguments`](Self::finish_arguments), for a variant written without arguments such as `Optional.None`.
    fn finish_arguments_named(
        &mut self,
        call: &Expr,
        id: TypeId,
        known: &mut HashMap<String, Ty>,
        variant: &Ident,
    ) -> Ty {
        let generics = self.def(id).generics.clone();
        let name = self.def(id).name.clone();
        let mut args = Vec::new();
        for generic in &generics {
            match known.get(generic) {
                Some(ty) => args.push(ty.clone()),
                None => {
                    let written: Vec<String> = generics
                        .iter()
                        .map(|g| {
                            if g == generic {
                                "String".to_string()
                            } else {
                                g.clone()
                            }
                        })
                        .collect();
                    self.error(
                        variant.span,
                        format!(
                            "nothing says what {generic} is: write {name}<{}>.{}",
                            written.join(", "),
                            variant.name
                        ),
                    );
                    let _ = call;
                    return Ty::Error;
                }
            }
        }
        self.defined_ty(id, args)
    }

    /// `Type.Variant(args)`: a variant of a tagged union is made through the union.
    pub(super) fn construct_variant(
        &mut self,
        call: &Expr,
        meta: &Ty,
        variant: &Ident,
        args: &[Arg],
        expected: Option<&Ty>,
    ) -> Ty {
        let Ty::TypeValue {
            id,
            name: type_name,
            kind,
            args: explicit,
        } = meta
        else {
            return Ty::Error;
        };
        if *kind != DefinedKind::Union {
            self.error(
                variant.span,
                format!("{type_name} has no function `{}`", variant.name),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let Some(found) = self
            .def(*id)
            .variants
            .iter()
            .find(|v| v.name == variant.name)
            .cloned()
        else {
            self.error(
                variant.span,
                format!("{type_name} has no variant {}", variant.name),
            );
            self.check_arguments(args);
            return Ty::Error;
        };
        let generics = self.def(*id).generics.clone();
        let mut known = self.seed_arguments_from(*id, explicit, expected);
        if found.outside {
            // the variant wraps a value of an existing struct
            let struct_name = self.def(found.def).name.clone();
            let [arg] = args else {
                self.error(
                    call.span,
                    format!("{type_name}.{} wraps one {struct_name} value", variant.name),
                );
                self.check_arguments(args);
                return Ty::Error;
            };
            if arg.name.is_some() {
                self.error(
                    arg.value.span,
                    format!(
                        "{} is named from outside: give it a {struct_name} value, not its fields",
                        variant.name
                    ),
                );
                self.expr(&arg.value, None);
            } else {
                let wanted = self.defined_ty(found.def, Vec::new());
                let got = self.expr(&arg.value, Some(&wanted));
                if !wanted.accepts(&got) {
                    self.error(
                        arg.value.span,
                        format!("{} is named from outside: give it a {struct_name} value, but this is {got}", variant.name),
                    );
                }
            }
        } else {
            let fields = self.def(found.def).fields.clone();
            let label = format!("{type_name}.{}", variant.name);
            let pairs: Vec<(Option<&Ident>, &Expr)> =
                args.iter().map(|a| (a.name.as_ref(), &a.value)).collect();
            self.fill_fields(
                call, &label, &fields, &generics, &pairs, &mut known, true, true,
            );
        }
        self.finish_arguments(call, *id, &known, Some(&variant.name))
    }

    /// `value.clone()` and `value.clone(x = 9.0)`: a copy, with some fields of a struct changed.
    pub(super) fn clone_call(&mut self, call: &Expr, receiver: &Ty, args: &[Arg]) -> Ty {
        let Ty::Defined {
            id,
            kind,
            args: type_args,
            name,
            ..
        } = receiver
        else {
            return Ty::Error;
        };
        if *kind != DefinedKind::Struct || args.is_empty() {
            if !args.is_empty() {
                self.error(call.span, format!("clone of a {name} takes no arguments"));
                self.check_arguments(args);
            }
            return receiver.clone();
        }
        let fields = self.fields_of(*id, type_args);
        let mut seen: Vec<&str> = Vec::new();
        for arg in args {
            let Some(field) = &arg.name else {
                self.error(
                    arg.value.span,
                    format!(
                        "clone takes the names of the fields to change: write clone({} = ...)",
                        fields.first().map_or("x", |f| f.name.as_str())
                    ),
                );
                self.expr(&arg.value, None);
                continue;
            };
            if seen.contains(&field.name.as_str()) {
                self.error(field.span, format!("`{}` is given twice", field.name));
            }
            seen.push(&field.name);
            match fields.iter().find(|f| f.name == field.name) {
                Some(f) => {
                    let ty = f.ty.clone();
                    let got = self.expr(&arg.value, Some(&ty));
                    self.expect_assignable(&ty, &got, &arg.value);
                }
                None => {
                    self.error(field.span, format!("{name} has no field `{}`", field.name));
                    self.expr(&arg.value, None);
                }
            }
        }
        receiver.clone()
    }

    /// `receiver.name(args)` for a value of a defined type: one of its methods, or a field that holds a function.
    pub(super) fn instance_call(
        &mut self,
        call: &Expr,
        receiver: &Ty,
        name: &Ident,
        written: &[ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let Ty::Defined {
            id,
            args: type_args,
            name: type_name,
            kind,
            ..
        } = receiver
        else {
            return Ty::Error;
        };
        if matches!(
            kind,
            DefinedKind::Class | DefinedKind::Interface | DefinedKind::Trait
        ) {
            return self.class_call(call, receiver, name, written, args, trailer);
        }
        if *id == ANY
            && let Some(ty) = self.any_test(call, name, written, args)
        {
            return ty;
        }
        if !written.is_empty() {
            self.error(
                call.span,
                format!("{type_name}.{} takes no type arguments", name.name),
            );
        }
        if name.name == "clone" && !self.def(*id).methods.contains_key("clone") {
            if trailer.is_some() {
                self.error(call.span, "clone takes no block");
            }
            return self.clone_call(call, receiver, args);
        }
        if name.name == "raw"
            && matches!(kind, DefinedKind::Struct | DefinedKind::RawUnion)
            && !self.def(*id).methods.contains_key("raw")
        {
            if !args.is_empty() || trailer.is_some() {
                self.error(call.span, "raw takes nothing");
                self.check_arguments(args);
            }
            return Ty::Ptr(Box::new(Ty::Int(IntTy::U8)));
        }
        let map = self.arg_map(*id, type_args);
        if let Some(method) = self.def(*id).methods.get(&name.name).cloned() {
            let Ty::Function(f) = Ty::Function(method).substitute(&map) else {
                return Ty::Error;
            };
            if f.ret.is_error() {
                // a method whose type is worked out from its body, which has not been read yet
                self.check_arguments(args);
                return Ty::Error;
            }
            let label = format!("{type_name}.{}", name.name);
            return self.call_function(call, &label, &f, &[], args, trailer);
        }
        if matches!(
            kind,
            DefinedKind::Struct | DefinedKind::RawUnion | DefinedKind::Class
        ) && let Some(field) = self
            .fields_of(*id, type_args)
            .into_iter()
            .find(|f| f.name == name.name)
            && let Ty::Function(f) = field.ty
        {
            return self.call_function(call, &name.name, &f, &[], args, trailer);
        }
        self.error(
            name.span,
            format!("{receiver} has no method `{}`", name.name),
        );
        self.check_arguments(args);
        Ty::Error
    }

    /// The variants or members of an enum or tagged union, for `match`: each with the type its `|name|` has.
    pub(super) fn match_cases(&self, subject: &Ty) -> Option<Vec<(String, Option<Ty>)>> {
        let Ty::Defined { id, kind, args, .. } = subject else {
            return None;
        };
        match kind {
            DefinedKind::Enum => Some(
                self.def(*id)
                    .members
                    .iter()
                    .map(|(name, _)| (name.clone(), None))
                    .collect(),
            ),
            DefinedKind::Union => Some(
                self.def(*id)
                    .variants
                    .iter()
                    .map(|v| {
                        let data = if v.outside || !self.def(v.def).fields.is_empty() {
                            let vargs = if v.outside { Vec::new() } else { args.clone() };
                            Some(self.defined_ty(v.def, vargs))
                        } else {
                            None
                        };
                        (v.name.clone(), data)
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The fields that a struct block names, one `name = value` line each, for the last argument of a call.
    pub(super) fn struct_block(
        &mut self,
        call_span: Span,
        wanted: &Ty,
        trailer: &Trailer,
        generics: &[String],
        known: &mut HashMap<String, Ty>,
    ) {
        let Ty::Defined {
            id,
            name,
            args: type_args,
            ..
        } = wanted
        else {
            return;
        };
        if let Some(param) = trailer.params.first() {
            self.error(
                param.span,
                "a struct block gives fields, it has no names between | |",
            );
        }
        let mut pairs: Vec<(Option<&Ident>, &Expr)> = Vec::new();
        for stmt in &trailer.body {
            match &stmt.kind {
                StmtKind::Assign(assign)
                    if assign.op == crate::ast::AssignOp::Set
                        && matches!(&assign.target.kind, ExprKind::Name(_)) =>
                {
                    if let ExprKind::Name(field) = &assign.target.kind {
                        pairs.push((Some(field), &assign.value));
                    }
                }
                _ => self.error(
                    stmt.span,
                    "a struct block is one line per field: name = value",
                ),
            }
        }
        let fields = self.fields_of(*id, type_args);
        let call = Expr {
            id: crate::ast::NodeId(u32::MAX),
            kind: ExprKind::Error,
            span: trailer.body.first().map_or(call_span, |s| s.span),
        };
        self.fill_fields(&call, name, &fields, generics, &pairs, known, false, true);
    }
}

fn kind_word(kind: DefinedKind) -> &'static str {
    match kind {
        DefinedKind::Struct => "struct",
        DefinedKind::RawUnion => "union",
        DefinedKind::Class => "class",
        DefinedKind::Trait => "trait",
        DefinedKind::Interface => "interface",
        DefinedKind::Enum => "enum",
        DefinedKind::Union => "tagged union",
    }
}

/// `const name = function(...) { ... }` in a type body is a method.
fn is_method(decl: &Decl) -> bool {
    decl.kind == DeclKind::Const && super::functions::is_definition(&decl.init)
}

/// The integer an enum member is given: written in the code, with a minus when it is negative.
fn whole_number(expr: &Expr) -> Option<i128> {
    match &expr.kind {
        ExprKind::Int(value) => i128::try_from(*value).ok(),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => match operand.kind {
            ExprKind::Int(value) => i128::try_from(value).ok().map(|v| -v),
            _ => None,
        },
        _ => None,
    }
}
