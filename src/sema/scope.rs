//! Names and where they are visible.
//!
//! A scope is a table from a name to the thing it stands for. Scopes are nested like the blocks of the program: the file
//! is the outermost one, and each `{ }` opens another inside it. Looking up a name starts in the innermost scope and
//! goes outwards, so an inner name hides an outer one with the same name (shadowing) and disappears when its block ends.

use std::collections::{HashMap, HashSet};

use super::ty::Ty;
use crate::span::Span;

/// Names one declared variable. It indexes [`Scopes::symbols`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// How a variable was declared, which decides whether it can be assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// `const x = ...`: fixed.
    Const,
    /// `let x = ...`: can be assigned, keeps its type.
    Let,
    /// `u8 x = ...`: can be assigned, keeps the type written.
    Typed,
    /// A parameter of a function, or a name of a block: fixed inside the function, like a const.
    Param,
}

/// The value of a `const` that was made from a literal, known while the program is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constant {
    /// A whole number: the number without its sign, and whether a minus is in front of it.
    Int {
        magnitude: u128,
        negative: bool,
    },
    Char(char),
}

impl std::fmt::Display for Constant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Constant::Int {
                magnitude,
                negative: true,
            } if *magnitude != 0 => write!(f, "-{magnitude}"),
            Constant::Int { magnitude, .. } => write!(f, "{magnitude}"),
            Constant::Char(c) => write!(f, "{c:?}"),
        }
    }
}

/// What a name defines when it is a type rather than a value.
#[derive(Debug, Clone)]
pub enum Defines {
    /// `const Point = struct { ... }`
    Type(super::ty::TypeId),
    /// `const Id = i32`: a second name for the same type
    Alias(Ty),
    /// `const page = attribute(String path)`: the index of its description in the checker
    Attribute(usize),
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub binding: Binding,
    pub ty: Ty,
    /// Where the name was written in its declaration.
    pub span: Span,
    /// The value of a `const` made from a literal (or from another such const). Known while the program is built, which
    /// is what lets it be the position of `get` and `set` on a Tuple, or the value of a `case`.
    pub constant: Option<Constant>,
    /// Set when the name is a type: the type it names.
    pub defines: Option<Defines>,
}

#[derive(Default)]
struct Frame {
    names: HashMap<String, SymbolId>,
    /// Names that this block declares further down and has not reached yet. A name is visible only below its
    /// declaration, and knowing this lets the error say "declared below" instead of "not declared".
    later: HashSet<String>,
}

pub struct Scopes {
    frames: Vec<Frame>,
    pub symbols: Vec<Symbol>,
    /// Every name that was declared in a block that has since ended.
    ended: HashSet<String>,
}

impl Scopes {
    /// The file scope, which declares `later` further down.
    pub fn new(later: HashSet<String>) -> Scopes {
        Scopes {
            frames: vec![Frame {
                later,
                ..Frame::default()
            }],
            symbols: Vec::new(),
            ended: HashSet::new(),
        }
    }

    /// 0 is the file; each block inside adds one.
    pub fn depth(&self) -> usize {
        self.frames.len() - 1
    }

    pub fn push(&mut self, later: HashSet<String>) {
        self.frames.push(Frame {
            later,
            ..Frame::default()
        });
    }

    pub fn pop(&mut self) {
        let frame = self.frames.pop().expect("the file scope is never popped");
        self.ended.extend(frame.names.into_keys());
    }

    /// Declares a name in the innermost scope. If that scope already has the name, nothing changes and the earlier
    /// one is returned as the error.
    pub fn declare(&mut self, symbol: Symbol) -> Result<SymbolId, SymbolId> {
        let frame = self.frames.last_mut().expect("there is always a scope");
        if let Some(&existing) = frame.names.get(&symbol.name) {
            return Err(existing);
        }
        let id = SymbolId(u32::try_from(self.symbols.len()).expect("too many symbols"));
        frame.later.remove(&symbol.name);
        frame.names.insert(symbol.name.clone(), id);
        self.symbols.push(symbol);
        Ok(id)
    }

    /// The innermost declaration of a name that is visible here.
    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.names.get(name).copied())
    }

    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0 as usize]
    }

    /// Is the name declared further down in this block or in a block around it?
    pub fn declared_below(&self, name: &str) -> bool {
        self.frames.iter().any(|frame| frame.later.contains(name))
    }

    /// Was the name declared in a block that has ended?
    pub fn went_out_of_scope(&self, name: &str) -> bool {
        self.ended.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            binding: Binding::Let,
            ty: Ty::I32,
            span: Span::default(),
            constant: None,
            defines: None,
        }
    }

    #[test]
    fn an_inner_name_hides_an_outer_one_until_its_block_ends() {
        let mut scopes = Scopes::new(HashSet::new());
        let outer = scopes.declare(symbol("x")).unwrap();
        scopes.push(HashSet::new());
        let inner = scopes.declare(symbol("x")).unwrap();
        assert_ne!(outer, inner);
        assert_eq!(scopes.lookup("x"), Some(inner));
        scopes.pop();
        assert_eq!(scopes.lookup("x"), Some(outer));
    }

    #[test]
    fn a_name_cannot_be_declared_twice_in_one_scope() {
        let mut scopes = Scopes::new(HashSet::new());
        let first = scopes.declare(symbol("x")).unwrap();
        assert_eq!(scopes.declare(symbol("x")), Err(first));
    }

    #[test]
    fn a_block_remembers_what_it_declares_further_down() {
        let mut scopes = Scopes::new(HashSet::from(["later".to_string()]));
        assert!(scopes.declared_below("later"));
        scopes.declare(symbol("later")).unwrap();
        assert!(!scopes.declared_below("later"));
    }

    #[test]
    fn a_name_is_out_of_scope_after_its_block_ends() {
        let mut scopes = Scopes::new(HashSet::new());
        scopes.push(HashSet::new());
        scopes.declare(symbol("inner")).unwrap();
        scopes.pop();
        assert_eq!(scopes.lookup("inner"), None);
        assert!(scopes.went_out_of_scope("inner"));
    }
}
