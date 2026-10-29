//! Semantic analysis: finding out what the program means, and whether it makes sense.
//!
//! A program can have the right shape and still be wrong: a name that was never declared, `1 + 2.5` mixing an integer
//! and a float, a number too large for its type, an assignment to a `const`. The parser cannot see these, because they
//! depend on what names stand for and what types values have. This stage finds them.
//!
//! It does two jobs at once, walking the tree from the first statement to the last:
//!
//! - **Name resolution.** Every use of a name is tied to the declaration it refers to, using the scopes in [`scope`].
//!   A name is visible only below its declaration, an inner block can hide an outer name, and a block's names end with it.
//! - **Type checking.** Every expression gets a type, and every place that needs a certain type is checked against it.
//!   Raga has no implicit conversion, so most of the work is saying "this is `i32` but `u8` is needed".
//!
//! The one place types flow downwards is a literal. `5` has no type of its own: it takes the type its context asks for
//! (`u8 x = 5`, `byte + 5`), or `i32` when nothing asks. So `check_expr` is given the type that is *expected*, if any, as
//! well as returning the type it found. This is called bidirectional checking.
//!
//! Functions are values with a type (`functions.rs`): the parameters and the body of a definition, the arguments of a call
//! matched to the parameters, type parameters worked out from the arguments, and the block after a call.
//! The types a program defines (`defined.rs`): a struct, an enum and a union are values of the language like any other,
//! `const Point = struct { ... }`, so a definition is a declaration whose value is the call of `struct`, `enum` or `union`.
//! The checker keeps a table of them, and `Optional` and `Result` are the first two entries of it.
//! The collections are checked as built-in generic types (`collections.rs`): their constructors, their methods and the
//! rule for a Map key. What it does not check yet: the types and functions of the later templates. A name or call it cannot check yet
//! reports "not checked yet" instead of passing.
//!
//! The result is not a new tree. The [`Analysis`] keeps what was learned in tables indexed by [`NodeId`], next to the
//! unchanged AST. Later stages read both.

mod attributes;
mod builtins;
mod call;
mod checker;
mod class_rules;
mod class_use;
mod classes;
mod collections;
mod defined;
mod expr;
mod format;
mod forms;
mod functions;
mod modules;
mod operators;
mod pointers;
mod scope;
mod ty;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::ast::{NodeId, Program};
use crate::diagnostics::Diagnostic;

pub use scope::{Binding, Constant, Defines, Symbol, SymbolId};
pub use ty::{DefinedKind, FloatTy, FnTy, IntTy, ParamTy, Ty, TypeId};

/// What semantic analysis learned about a program.
#[derive(Debug)]
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    /// Every variable that was declared, in the order of declaration.
    pub symbols: Vec<Symbol>,
    /// The type of every expression that was checked.
    pub expr_types: HashMap<NodeId, Ty>,
    /// Which variable a node stands for: the id of a [`Name`](crate::ast::ExprKind::Name) expression is mapped to the
    /// variable it uses, and the id of a declaration statement to the variable it creates.
    pub symbol_of: HashMap<NodeId, SymbolId>,
    /// The names that stand for a call of a function with no parameters, `echo`, keyed by the id of the name.
    pub bare_calls: HashSet<NodeId>,
    /// The files that were imported, with what was found in each. `NodeId`s and symbols belong to one file, so the maps
    /// above are those of the file that was checked.
    pub modules: Vec<modules::ModuleReport>,
}

impl Analysis {
    /// The most recent variable with this name. Meant for tests and tools that look at a file as a whole.
    pub fn symbol_named(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().rev().find(|s| s.name == name)
    }
}

/// Checks a whole program that is not in a file: `import` cannot find anything.
pub fn analyze(program: &Program) -> Analysis {
    checker::Checker::new(program).run(program)
}

/// Checks a program that was read from `path`, so that `import` finds the other files from there.
pub fn analyze_file(program: &Program, path: &std::path::Path) -> Analysis {
    let mut checker = checker::Checker::new(program);
    checker.file = Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    checker.run(program)
}

pub use modules::ModuleReport;
