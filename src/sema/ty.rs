//! The types of Raga values, as the checker works with them.
//!
//! This is not the [`ast::Type`](crate::ast::Type), which is a type *as written* (`List<i32>` with the position of each
//! piece). A `Ty` is the meaning: `List` of `i32`, compared and copied freely. Two types are the same when the values are
//! equal, which is what makes checking `i32` against `i32` a plain `==`.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

impl IntTy {
    pub fn from_name(name: &str) -> Option<IntTy> {
        Some(match name {
            "i8" => IntTy::I8,
            "i16" => IntTy::I16,
            "i32" => IntTy::I32,
            "i64" => IntTy::I64,
            "i128" => IntTy::I128,
            "isize" => IntTy::Isize,
            "u8" => IntTy::U8,
            "u16" => IntTy::U16,
            "u32" => IntTy::U32,
            "u64" => IntTy::U64,
            "u128" => IntTy::U128,
            "usize" => IntTy::Usize,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::I128 => "i128",
            IntTy::Isize => "isize",
            IntTy::U8 => "u8",
            IntTy::U16 => "u16",
            IntTy::U32 => "u32",
            IntTy::U64 => "u64",
            IntTy::U128 => "u128",
            IntTy::Usize => "usize",
        }
    }

    pub fn is_signed(self) -> bool {
        matches!(
            self,
            IntTy::I8 | IntTy::I16 | IntTy::I32 | IntTy::I64 | IntTy::I128 | IntTy::Isize
        )
    }

    /// The width in bits. `isize` and `usize` are as wide as a pointer on the target machine; until the compiler knows
    /// its target they are taken as 64 bits.
    pub fn bits(self) -> u32 {
        match self {
            IntTy::I8 | IntTy::U8 => 8,
            IntTy::I16 | IntTy::U16 => 16,
            IntTy::I32 | IntTy::U32 => 32,
            IntTy::I64 | IntTy::U64 | IntTy::Isize | IntTy::Usize => 64,
            IntTy::I128 | IntTy::U128 => 128,
        }
    }

    /// Does the number fit? `negative` says a minus is in front of it, which is how a negative literal is written:
    /// `-128` is the number 128 with a minus, and 128 alone does not fit an `i8` while `-128` does.
    pub fn fits(self, magnitude: u128, negative: bool) -> bool {
        let bits = self.bits();
        if self.is_signed() {
            let limit = 1u128 << (bits - 1);
            if negative {
                magnitude <= limit
            } else {
                magnitude < limit
            }
        } else if negative {
            magnitude == 0
        } else {
            bits == 128 || magnitude < (1u128 << bits)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatTy {
    F32,
    F64,
    F128,
}

impl FloatTy {
    pub fn from_name(name: &str) -> Option<FloatTy> {
        Some(match name {
            "f32" => FloatTy::F32,
            "f64" => FloatTy::F64,
            "f128" => FloatTy::F128,
            _ => return None,
        })
    }

    pub fn bits(self) -> u32 {
        match self {
            FloatTy::F32 => 32,
            FloatTy::F64 => 64,
            FloatTy::F128 => 128,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FloatTy::F32 => "f32",
            FloatTy::F64 => "f64",
            FloatTy::F128 => "f128",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int(IntTy),
    Float(FloatTy),
    Bool,
    Char,
    String,
    Array(Box<Ty>),
    List(Box<Ty>),
    Set(Box<Ty>),
    /// Key type and value type.
    Map(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    /// A function: what it takes and what it gives back. A function is a value, so this is the type of a variable, a
    /// parameter, a field or a collection element like any other.
    Function(FnTy),
    /// A value of a type the program defines: a struct, an enum or a union, with the type arguments of a generic one.
    Defined {
        id: TypeId,
        name: String,
        kind: DefinedKind,
        args: Vec<Ty>,
        /// Every type this one can be used as: the class it extends and the interfaces it implements, all the way up.
        supers: Vec<TypeId>,
    },
    /// A type used as a value: `Point` in `Point(x = 1.0)`, `Shape` in `Shape.Circle(2.0)`, `Color` in `Color.Green`.
    TypeValue {
        id: TypeId,
        name: String,
        kind: DefinedKind,
        args: Vec<Ty>,
    },
    /// `ptr<T>`: a pointer. Only the type is known so far, pointers are explained in 08-memory.
    Ptr(Box<Ty>),
    /// The value of `import(path)`: the exported names of a file. Two imports of one file give the same number.
    Module(usize),
    /// A type parameter of a generic function, `T` in `function<T>(T value) T`. Inside the function it is a type nothing
    /// is known about, except that it can be passed around, stored, returned and compared with `==`.
    TypeParam(String),
    /// What a call that gives nothing back has. It is not a type a name can have.
    Void,
    /// The type of an expression that already has a reported error. It fits everything, so the one mistake is not
    /// reported again at every place the broken value is used.
    Error,
}

/// Names a type that the program defines. It indexes the table of types of the checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

/// The types that come with the language, in the order they are put in the table of types.
pub const OPTIONAL: TypeId = TypeId(0);
pub const RESULT: TypeId = TypeId(1);
pub const IERROR: TypeId = TypeId(6);
pub const ERROR: TypeId = TypeId(7);
pub const KIND: TypeId = TypeId(8);
pub const OP: TypeId = TypeId(9);
pub const FIELD_INFO: TypeId = TypeId(10);
pub const VARIANT_INFO: TypeId = TypeId(11);
pub const ATTRIBUTE_INFO: TypeId = TypeId(12);
pub const BINDING_INFO: TypeId = TypeId(13);
pub const TYPE_INFO: TypeId = TypeId(14);
pub const ANY: TypeId = TypeId(15);

/// What a defined type is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinedKind {
    /// A class: like a struct for now. Classes are explained in 07-classes-and-traits.
    Class,
    /// An interface: a list of what a class has. It is not created, a class that implements it is.
    Interface,
    /// A trait: code that a class uses. It is not a type a value can have.
    Trait,
    Struct,
    Enum,
    /// A tagged union: one of several variants, each a struct.
    Union,
    /// An untagged union: one storage, several views.
    RawUnion,
}

/// A function type: `function<T>(T, i32 count = 1) T`.
#[derive(Debug, Clone)]
pub struct FnTy {
    /// The type parameters, empty for a function that is not generic.
    pub generics: Vec<String>,
    pub params: Vec<ParamTy>,
    /// What it gives back, `Void` when nothing.
    pub ret: Box<Ty>,
}

/// One parameter of a function type.
#[derive(Debug, Clone)]
pub struct ParamTy {
    /// Its name, which a call may use to give the argument. A function type that is only written has none.
    pub name: Option<String>,
    pub ty: Ty,
    pub has_default: bool,
    /// `Type ...name`: takes any number of arguments, and `ty` is the type of each one.
    pub variadic: bool,
}

/// Two function types are the same type when they take the same types and give back the same type. The names of the
/// parameters and their defaults say how to call, not what the function is.
impl PartialEq for FnTy {
    fn eq(&self, other: &FnTy) -> bool {
        self.params.len() == other.params.len()
            && self
                .params
                .iter()
                .zip(&other.params)
                .all(|(a, b)| a.ty == b.ty)
            && self.ret == other.ret
    }
}

impl Ty {
    pub const I32: Ty = Ty::Int(IntTy::I32);
    pub const F64: Ty = Ty::Float(FloatTy::F64);
    pub const USIZE: Ty = Ty::Int(IntTy::Usize);

    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, Ty::Int(_))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float(_))
    }

    pub fn is_number(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    /// Does `==` work on this type without the program declaring it? Primitives, String and enum members.
    pub fn has_equality(&self) -> bool {
        matches!(
            self,
            Ty::Int(_)
                | Ty::Float(_)
                | Ty::Bool
                | Ty::Char
                | Ty::String
                | Ty::Ptr(_)
                | Ty::Error
                | Ty::Defined {
                    kind: DefinedKind::Enum,
                    ..
                }
        )
    }

    /// Can a value of this type be the key of a Map? A primitive value or a String: they are found by their value.
    pub fn is_key(&self) -> bool {
        matches!(
            self,
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::String | Ty::Error
        )
    }

    /// Can a value of type `other` be used where this type is wanted? Types must be equal, except that a type with an
    /// error in it fits everything.
    pub fn accepts(&self, other: &Ty) -> bool {
        self.accepts_as(other, false)
    }

    /// Like `accepts`, but a class may stand for a class it extends: for the object a method or an operator is called on,
    /// which is the object itself and not a copy of it.
    pub fn accepts_sub(&self, other: &Ty) -> bool {
        self.accepts_as(other, true)
    }

    /// `behind_pointer` is true for what a `ptr<T>` points to: only there can a class stand for a class it extends or an
    /// interface it implements. A value of the subclass stored as its base class would lose its extra fields.
    fn accepts_as(&self, other: &Ty, behind_pointer: bool) -> bool {
        match (self, other) {
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Array(a), Ty::Array(b))
            | (Ty::List(a), Ty::List(b))
            | (Ty::Set(a), Ty::Set(b)) => a.accepts(b),
            (Ty::Map(k, v), Ty::Map(k2, v2)) => k.accepts(k2) && v.accepts(v2),
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.accepts(y))
            }
            (Ty::Function(a), Ty::Function(b)) => {
                a.params.len() == b.params.len()
                    && a.params
                        .iter()
                        .zip(&b.params)
                        .all(|(x, y)| x.ty.accepts(&y.ty))
                    && a.ret.accepts(&b.ret)
            }
            // a class can be used where a class it extends or an interface it implements is wanted
            (Ty::Defined { id: a, .. }, Ty::Defined { id: b, supers, .. })
                if behind_pointer && a != b && supers.contains(a) =>
            {
                true
            }
            (Ty::Defined { id: a, args: x, .. }, Ty::Defined { id: b, args: y, .. }) => {
                a == b && x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.accepts(q))
            }
            (Ty::Ptr(a), Ty::Ptr(b)) => a.accepts_as(b, true),
            _ => self == other,
        }
    }
}

impl Ty {
    /// Is a type parameter of `generics` that has no type in `known` left in this type?
    pub fn has_unknown(&self, generics: &[String], known: &HashMap<String, Ty>) -> bool {
        match self {
            Ty::TypeParam(name) => generics.contains(name) && !known.contains_key(name),
            Ty::Array(t) | Ty::List(t) | Ty::Set(t) => t.has_unknown(generics, known),
            Ty::Map(k, v) => k.has_unknown(generics, known) || v.has_unknown(generics, known),
            Ty::Tuple(items) => items.iter().any(|t| t.has_unknown(generics, known)),
            Ty::Defined { args, .. } | Ty::TypeValue { args, .. } => {
                args.iter().any(|t| t.has_unknown(generics, known))
            }
            Ty::Ptr(t) => t.has_unknown(generics, known),
            Ty::Function(f) => {
                f.params.iter().any(|p| p.ty.has_unknown(generics, known))
                    || f.ret.has_unknown(generics, known)
            }
            _ => false,
        }
    }

    /// This type with each type parameter in `known` replaced by its type.
    pub fn substitute(&self, known: &HashMap<String, Ty>) -> Ty {
        match self {
            Ty::TypeParam(name) => known.get(name).cloned().unwrap_or_else(|| self.clone()),
            Ty::Array(t) => Ty::Array(Box::new(t.substitute(known))),
            Ty::List(t) => Ty::List(Box::new(t.substitute(known))),
            Ty::Set(t) => Ty::Set(Box::new(t.substitute(known))),
            Ty::Map(k, v) => Ty::Map(Box::new(k.substitute(known)), Box::new(v.substitute(known))),
            Ty::Tuple(items) => Ty::Tuple(items.iter().map(|t| t.substitute(known)).collect()),
            Ty::Defined {
                id,
                name,
                kind,
                args,
                supers,
            } => Ty::Defined {
                id: *id,
                name: name.clone(),
                kind: *kind,
                args: args.iter().map(|t| t.substitute(known)).collect(),
                supers: supers.clone(),
            },
            Ty::TypeValue {
                id,
                name,
                kind,
                args,
            } => Ty::TypeValue {
                id: *id,
                name: name.clone(),
                kind: *kind,
                args: args.iter().map(|t| t.substitute(known)).collect(),
            },
            Ty::Ptr(t) => Ty::Ptr(Box::new(t.substitute(known))),
            Ty::Function(f) => Ty::Function(FnTy {
                generics: f
                    .generics
                    .iter()
                    .filter(|g| !known.contains_key(*g))
                    .cloned()
                    .collect(),
                params: f
                    .params
                    .iter()
                    .map(|p| ParamTy {
                        ty: p.ty.substitute(known),
                        ..p.clone()
                    })
                    .collect(),
                ret: Box::new(f.ret.substitute(known)),
            }),
            _ => self.clone(),
        }
    }

    /// Finds the types of the type parameters of `generics` that make `self` (written with them) the same as `actual`,
    /// and adds them to `known`. Says whether `actual` fits.
    pub fn unify(&self, actual: &Ty, generics: &[String], known: &mut HashMap<String, Ty>) -> bool {
        match (self, actual) {
            (_, Ty::Error) | (Ty::Error, _) => true,
            (Ty::TypeParam(name), _) if generics.contains(name) => match known.get(name) {
                Some(bound) => bound.accepts(actual),
                None => {
                    known.insert(name.clone(), actual.clone());
                    true
                }
            },
            (Ty::Array(a), Ty::Array(b))
            | (Ty::List(a), Ty::List(b))
            | (Ty::Set(a), Ty::Set(b)) => a.unify(b, generics, known),
            (Ty::Map(k, v), Ty::Map(k2, v2)) => {
                k.unify(k2, generics, known) && v.unify(v2, generics, known)
            }
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.unify(y, generics, known))
            }
            (Ty::Function(a), Ty::Function(b)) => {
                a.params.len() == b.params.len()
                    && a.params
                        .iter()
                        .zip(&b.params)
                        .all(|(x, y)| x.ty.unify(&y.ty, generics, known))
                    && a.ret.unify(&b.ret, generics, known)
            }
            (Ty::Defined { id: a, args: x, .. }, Ty::Defined { id: b, args: y, .. }) => {
                a == b
                    && x.len() == y.len()
                    && x.iter().zip(y).all(|(p, q)| p.unify(q, generics, known))
            }
            (Ty::Ptr(a), Ty::Ptr(b)) => a.unify(b, generics, known),
            _ => self.accepts(actual),
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int(t) => write!(f, "{}", t.name()),
            Ty::Float(t) => write!(f, "{}", t.name()),
            Ty::Bool => write!(f, "bool"),
            Ty::Char => write!(f, "char"),
            Ty::String => write!(f, "String"),
            Ty::Array(t) => write!(f, "Array<{t}>"),
            Ty::List(t) => write!(f, "List<{t}>"),
            Ty::Set(t) => write!(f, "Set<{t}>"),
            Ty::Map(k, v) => write!(f, "Map<{k}, {v}>"),
            Ty::Tuple(items) => {
                write!(f, "Tuple<")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ">")
            }
            Ty::Function(function) => {
                if !function.generics.is_empty() {
                    // a generic function has no single type to write, so it is shown as its head
                    write!(f, "function<{}>(", function.generics.join(", "))?;
                    for (i, param) in function.params.iter().enumerate() {
                        write!(f, "{}{}", if i > 0 { ", " } else { "" }, param.ty)?;
                    }
                    write!(f, ")")?;
                    if *function.ret != Ty::Void {
                        write!(f, " {}", function.ret)?;
                    }
                    return Ok(());
                }
                if function.params.is_empty() && *function.ret == Ty::Void {
                    return write!(f, "Function");
                }
                write!(f, "Function<")?;
                if function.params.is_empty() {
                    write!(f, "void")?;
                } else {
                    write!(f, "[")?;
                    for (i, param) in function.params.iter().enumerate() {
                        write!(f, "{}{}", if i > 0 { ", " } else { "" }, param.ty)?;
                    }
                    write!(f, "]")?;
                }
                if *function.ret != Ty::Void {
                    write!(f, ", {}", function.ret)?;
                }
                write!(f, ">")
            }
            Ty::Defined { name, args, .. } => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    let shown: Vec<String> = args.iter().map(ToString::to_string).collect();
                    write!(f, "<{}>", shown.join(", "))?;
                }
                Ok(())
            }
            Ty::TypeValue { name, .. } => write!(f, "the type {name}"),
            Ty::Ptr(t) => write!(f, "ptr<{t}>"),
            Ty::Module(_) => write!(f, "Module"),
            Ty::TypeParam(name) => write!(f, "{name}"),
            Ty::Void => write!(f, "nothing"),
            Ty::Error => write!(f, "<error>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_fit_exactly_their_range() {
        assert!(IntTy::U8.fits(255, false));
        assert!(!IntTy::U8.fits(256, false));
        assert!(IntTy::I8.fits(127, false));
        assert!(!IntTy::I8.fits(128, false));
        // -128 is the number 128 with a minus in front
        assert!(IntTy::I8.fits(128, true));
        assert!(!IntTy::I8.fits(129, true));
        assert!(!IntTy::U8.fits(1, true));
        assert!(IntTy::U8.fits(0, true));
    }

    #[test]
    fn the_widest_integers_fit_their_limits() {
        assert!(IntTy::U128.fits(u128::MAX, false));
        assert!(IntTy::I128.fits((1u128 << 127) - 1, false));
        assert!(!IntTy::I128.fits(1u128 << 127, false));
        assert!(IntTy::I128.fits(1u128 << 127, true));
        assert!(IntTy::I64.fits(9223372036854775808, true));
        assert!(!IntTy::I64.fits(9223372036854775808, false));
        assert!(IntTy::U64.fits(18446744073709551615, false));
    }

    #[test]
    fn the_error_type_fits_everything() {
        assert!(Ty::I32.accepts(&Ty::Error));
        assert!(Ty::Error.accepts(&Ty::String));
        assert!(!Ty::I32.accepts(&Ty::F64));
        assert!(Ty::List(Box::new(Ty::I32)).accepts(&Ty::List(Box::new(Ty::Error))));
    }

    #[test]
    fn types_print_the_way_they_are_written() {
        assert_eq!(Ty::List(Box::new(Ty::String)).to_string(), "List<String>");
        assert_eq!(
            Ty::Tuple(vec![Ty::I32, Ty::F64]).to_string(),
            "Tuple<i32, f64>"
        );
        assert_eq!(Ty::Set(Box::new(Ty::I32)).to_string(), "Set<i32>");
        assert_eq!(
            Ty::Map(Box::new(Ty::String), Box::new(Ty::I32)).to_string(),
            "Map<String, i32>"
        );
    }
}
