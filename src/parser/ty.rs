//! Types as they are written: `i32`, `List<i32>`, `Map<String, List<i32>>`, `function(i32, String) bool`.
//!
//! These functions never report an error. They answer "is there a type here?" with `Some` or `None`, because the
//! parser asks that question about text that is often not a type at all (`a < b` starts like `List<i32>`), always
//! inside [`Parser::attempt`], which backs up on `None`.

use super::Parser;
use crate::ast::{Ident, Type};
use crate::lexer::{Keyword, TokenKind};

impl Parser<'_> {
    /// A type that nothing but the end of the type follows: a type argument, a return type before `{`.
    pub(super) fn ty(&mut self) -> Option<Type> {
        self.ty_in(false)
    }

    /// A type that a name follows: `Type name`, as in a declaration or a parameter. It matters for a function type, whose
    /// return type is optional: in `function(i32) i32 transform` the `i32` is the return type, and in
    /// `function(i32) action` the `action` is the name.
    pub(super) fn ty_named(&mut self) -> Option<Type> {
        self.ty_in(true)
    }

    fn ty_in(&mut self, name_follows: bool) -> Option<Type> {
        if self.at(TokenKind::Keyword(Keyword::Function)) {
            return self.function_type(name_follows);
        }
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

    /// `function(i32, String) bool`. The parameters are types, each may be followed by a name that is not kept.
    fn function_type(&mut self, name_follows: bool) -> Option<Type> {
        let keyword = self.bump();
        let name = Ident {
            name: "function".to_string(),
            span: keyword.span,
        };
        self.eat(TokenKind::LParen)?;
        let mut args = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                args.push(self.ty()?);
                if self.at(TokenKind::Ident) {
                    self.bump();
                }
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.eat(TokenKind::RParen)?;
        let ret = self.function_ret(name_follows).map(Box::new);
        let span = keyword.span.to(self.last_span());
        Some(Type {
            name,
            args,
            ret,
            span,
        })
    }

    /// The type a function type gives back, when one is written.
    fn function_ret(&mut self, name_follows: bool) -> Option<Type> {
        let next = self.tok();
        let starts_type = matches!(
            next.kind,
            TokenKind::Ident | TokenKind::Keyword(Keyword::Function)
        );
        if !starts_type || next.starts_line {
            return None;
        }
        self.attempt(|p| {
            let ret = p.ty()?;
            // with a name after the type, the return type is the word before the name
            (!name_follows || (p.at(TokenKind::Ident) && !p.tok().starts_line)).then_some(ret)
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
