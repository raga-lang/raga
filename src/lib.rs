//! The Raga compiler.
//!
//! A compiler is a pipeline. Each stage reads the output of the stage before it and produces a
//! simpler or more precise description of the same program:
//!
//! ```text
//! source text
//!   -> lexer      (lexical analysis)    tokens
//!   -> parser     (syntax analysis)     AST, the tree of what was written
//!   -> sema       (semantic analysis)   names resolved, types checked, generics known
//! ```
//!
//! Every stage reports problems as [`diagnostics::Diagnostic`] values instead of stopping at the first one, so the user
//! sees all errors of a run, and a half-written file still gives a result.
//!
//! The lexer reads every `.raga` file. The parser and the semantic analysis cover the whole language as the templates in
//! `templates/` describe it: bindings and expressions, data structures, control flow, functions, structs, enums and unions,
//! errors, classes and traits, memory and pointers, attributes and reflection, modules and the standard library.

pub mod diagnostics;
pub mod source;
pub mod span;

pub mod lexer;

pub mod ast;
pub mod parser;
pub mod sema;
