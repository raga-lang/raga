//! Types as they are written: `i32`, `List<i32>`, `Map<String, List<i32>>`, `Function<[i32, String], bool>`.
//!
//! These functions never report an error. They answer "is there a type here?" with `Some` or `None`, because the
//! parser asks that question about text that is often not a type at all (`a < b` starts like `List<i32>`), always
//! inside [`Parser::attempt`], which backs up on `None`.

use super::Parser;
use crate::ast::{Ident, Type};
use crate::lexer::TokenKind;

impl Parser<'_> {
    /// A type that nothing but the end of the type follows: a type argument, a return type before `{`.
    pub(super) fn ty(&mut self) -> Option<Type> {
        self.ty_in()
    }

    fn ty_in(&mut self) -> Option<Type> {
        let mut name = self.ident()?;
        // a type of a module is written with the module in front: `shapes.Point`
        while self.at(TokenKind::Dot)
            && self.tok_at(1).kind == TokenKind::Ident
            && !self.tok().starts_line
        {
            self.bump();
            let part = self.ident()?;
            name = Ident {
                name: format!("{}.{}", name.name, part.name),
                span: name.span.to(part.span),
            };
        }
        if name.name == "Function" && self.at(TokenKind::Lt) {
            return self.function_type(name);
        }
        let mut args = Vec::new();
        if self.at(TokenKind::Lt) {
            args = self.type_args()?;
        }
        let span = name.span.to(self.last_span());
        Some(Type {
            name,
            args,
            ret: None,
            span,
        })
    }

    /// `Function<[i32, String], bool>`, with the cursor on the `<`. The parameters are a list of types in `[ ]`, or
    /// `void` when there are none, and the type it gives back follows after a comma, absent when it gives nothing.
    /// `Function<void>` is a mistake: a function that takes and gives nothing is written `Function`.
    fn function_type(&mut self, name: Ident) -> Option<Type> {
        self.eat(TokenKind::Lt)?;
        let mut args = Vec::new();
        if self.eat(TokenKind::LBracket).is_some() {
            args.push(self.ty()?);
            while self.eat(TokenKind::Comma).is_some() {
                args.push(self.ty()?);
            }
            self.eat(TokenKind::RBracket)?;
        } else {
            let void = self.ident()?;
            if void.name != "void" {
                return None;
            }
        }
        let ret = match self.eat(TokenKind::Comma) {
            Some(_) => Some(Box::new(self.ty()?)),
            None if args.is_empty() => return None,
            None => None,
        };
        self.eat(TokenKind::Gt)?;
        let span = name.span.to(self.last_span());
        Some(Type {
            name,
            args,
            ret,
            span,
        })
    }

    /// `<i32, List<i32>>`, with the cursor on the `<`. The lexer never joins two `>`, so the inner `>` and the outer
    /// `>` of `List<List<i32>>` arrive as two tokens and each list of arguments takes its own.
    pub(super) fn type_args(&mut self) -> Option<Vec<Type>> {
        self.eat(TokenKind::Lt)?;
        let mut args = vec![self.ty()?];
        while self.eat(TokenKind::Comma).is_some() {
            args.push(self.ty()?);
        }
        self.eat(TokenKind::Gt)?;
        Some(args)
    }
}
