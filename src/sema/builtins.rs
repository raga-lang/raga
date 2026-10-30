//! The names the language provides: functions, types, and the methods of `Math` and `String`.
//!
//! Each built-in function is described by a signature: what its parameters are and what it gives back. The checker reads
//! the signature, so adding a function to the language is adding a line here, not new checking code.
//!
//! Only what `01-basics` uses is described. The other reserved names are known (so they cannot be declared again) but a
//! call to one reports that it is not checked yet.

use super::ty::{FloatTy, IntTy};

/// A parameter of a built-in function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// Any number. All the `Num` arguments of one call have the same type, and so does the result: `Math.max(a, b)`.
    Num,
    F64,
    String,
    Usize,
    I32,
    I64,
    /// An error: an `Error`, or anything that implements `IError`.
    IError,
    /// Any value.
    Any,
}

/// What a built-in function gives back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ret {
    /// The type of the `Num` arguments.
    Num,
    F64,
    String,
    Bool,
    Usize,
    Char,
    ListOfString,
    ArrayOfString,
    I64,
    /// `Result<String, String>`: the text, or the reason there is none.
    ResultText,
    /// `Result<bool, String>`
    ResultFlag,
    Void,
}

#[derive(Debug, Clone, Copy)]
pub struct Sig {
    pub params: &'static [Param],
    /// The type of any further arguments after `params`. Present when the function takes any number of them.
    pub rest: Option<Param>,
    pub ret: Ret,
}

const fn sig(params: &'static [Param], ret: Ret) -> Sig {
    Sig {
        params,
        rest: None,
        ret,
    }
}

/// Names that cannot be declared again at the top level of a file, because the new name would hide the built-in one
/// (01-basics section 11). Functions first, then types.
const RESERVED_FUNCTIONS: &[&str] = &[
    "print",
    "len",
    "range",
    "read_line",
    "sleep",
    "typeof",
    "sizeof",
    "alignof",
    "alloc",
    "free",
    "compile",
    "reflect",
    "attributes_of",
    "get_field",
    "set_field",
    "import",
    "export",
    "$",
];

const RESERVED_TYPES: &[&str] = &[
    "String",
    "Array",
    "List",
    "Tuple",
    "Set",
    "Map",
    "Optional",
    "Result",
    "Error",
    "Weak",
    "Any",
    "Function",
    "TypeInfo",
    "FieldInfo",
    "VariantInfo",
    "AttributeInfo",
    "BindingInfo",
    "Kind",
    "Op",
    "Module",
    "Visibility",
    "Inheritance",
    "Ownership",
    "Math",
    "System",
    "File",
    "Time",
    "Random",
    "IError",
    "bool",
    "char",
    "ptr",
];

/// What a reserved name is, for the message that says it is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserved {
    Function,
    Type,
}

pub fn reserved(name: &str) -> Option<Reserved> {
    if RESERVED_FUNCTIONS.contains(&name) {
        Some(Reserved::Function)
    } else if RESERVED_TYPES.contains(&name)
        || IntTy::from_name(name).is_some()
        || FloatTy::from_name(name).is_some()
    {
        Some(Reserved::Type)
    } else {
        None
    }
}

/// The functions that can be called by their plain name.
pub fn global_function(name: &str) -> Option<Sig> {
    match name {
        "print" => Some(sig(&[Param::Any], Ret::Void)),
        "read_line" => Some(sig(&[], Ret::String)),
        "sleep" => Some(sig(&[Param::Num], Ret::Void)),
        "$" => Some(format_sig()),
        _ => None,
    }
}

fn format_sig() -> Sig {
    Sig {
        params: &[Param::String],
        rest: Some(Param::Any),
        ret: Ret::String,
    }
}

/// `range(from, to)` and `range(from, to, step)`: numbers of one type, only as the collection of a `for`.
pub fn range_sig() -> Sig {
    Sig {
        params: &[Param::Num, Param::Num],
        rest: Some(Param::Num),
        ret: Ret::Num,
    }
}

/// `Math.name(...)`
pub fn math_function(name: &str) -> Option<Sig> {
    use Param::{F64, Num};
    Some(match name {
        "abs" => sig(&[Num], Ret::Num),
        "min" | "max" => sig(&[Num, Num], Ret::Num),
        "pow" | "atan2" => sig(&[F64, F64], Ret::F64),
        "sqrt" | "floor" | "ceil" | "round" | "trunc" | "sin" | "cos" | "tan" | "asin" | "acos"
        | "atan" | "exp" | "log" | "log10" => sig(&[F64], Ret::F64),
        _ => return None,
    })
}

/// `System.name(...)`: the program and its surroundings.
pub fn system_function(name: &str) -> Option<Sig> {
    use Param::{I32, IError, String};
    Some(match name {
        "exit" => sig(&[I32], Ret::Void),
        "args" => sig(&[], Ret::ArrayOfString),
        "env" => sig(&[String], Ret::ResultText),
        "trace" => sig(&[IError], Ret::String),
        _ => return None,
    })
}

/// `File.name(...)`: what can fail in an ordinary way gives a Result.
pub fn file_function(name: &str) -> Option<Sig> {
    use Param::String;
    Some(match name {
        "read" => sig(&[String], Ret::ResultText),
        "write" | "append" => sig(&[String, String], Ret::ResultFlag),
        "exists" => sig(&[String], Ret::Bool),
        _ => return None,
    })
}

/// `Time.name(...)`
pub fn time_function(name: &str) -> Option<Sig> {
    match name {
        "now" => Some(sig(&[], Ret::I64)),
        _ => None,
    }
}

/// `Random.name(...)`
pub fn random_function(name: &str) -> Option<Sig> {
    use Param::{I64, Num};
    Some(match name {
        "seed" => sig(&[I64], Ret::Void),
        "int" => sig(&[Num, Num], Ret::Num),
        "float" => sig(&[], Ret::F64),
        _ => return None,
    })
}

/// `Math.pi` and `Math.e`
pub fn math_constant(name: &str) -> bool {
    matches!(name, "pi" | "e")
}

/// `String.name(...)`: functions of the type itself.
pub fn string_function(name: &str) -> Option<Sig> {
    match name {
        "join" => Some(Sig {
            params: &[Param::String],
            rest: Some(Param::String),
            ret: Ret::String,
        }),
        "format" => Some(format_sig()),
        _ => None,
    }
}

/// `text.name(...)`: methods of a String value.
pub fn string_method(name: &str) -> Option<Sig> {
    use Param::{String, Usize};
    Some(match name {
        "upper" | "lower" | "trim" | "clone" => sig(&[], Ret::String),
        "len" => sig(&[], Ret::Usize),
        "contains" | "equals" => sig(&[String], Ret::Bool),
        "split" => sig(&[String], Ret::ListOfString),
        "replace" => sig(&[String, String], Ret::String),
        "index_of" => sig(&[String], Ret::Usize),
        "get" => sig(&[Usize], Ret::Char),
        "slice" => sig(&[Usize, Usize], Ret::String),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names() {
        assert_eq!(reserved("print"), Some(Reserved::Function));
        assert_eq!(reserved("List"), Some(Reserved::Type));
        assert_eq!(reserved("u8"), Some(Reserved::Type));
        assert_eq!(reserved("f64"), Some(Reserved::Type));
        assert_eq!(reserved("total"), None);
    }

    #[test]
    fn signatures() {
        assert_eq!(math_function("max").unwrap().params.len(), 2);
        assert_eq!(string_method("len").unwrap().ret, Ret::Usize);
        assert!(string_method("nothing").is_none());
    }
}
