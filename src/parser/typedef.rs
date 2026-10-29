//! Type definitions: `struct { ... }`, `enum { ... }`, `union { ... }`, and the same with type parameters.
//!
//! A type is written like every other value of the language, as the call of a head with a block: the head is `struct`,
//! `enum` or `union`, the type parameters follow it in angle brackets, and the block is the body. Binding it to a name is
//! the ordinary `const Name = struct { ... }`. So a definition is an [`ExprKind::Call`] of the head, as a function is.
//!
//! What the body holds is members, one per line or separated by `;`:
//!
//! - a field is a declaration without `const` or `let`, `Type name` or `Type name = default`. It is read as the
//!   [`ExprKind::Param`] that a parameter of a function is, because it is the same thing: a type, a name, a default;
//! - `const name = value` is a member too: an enum member, a method, a variant of a union.

use super::Parser;
use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};

impl Parser<'_> {
    /// `struct<T> { ... }`, with the cursor on the keyword.
    pub(super) fn type_expr(&mut self) -> Expr {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);
        let type_params = if self.at(TokenKind::Lt) {
            match self.attempt(|p| p.type_args()) {
                Some(params) => params,
                None => {
                    let message =
                        format!("expected type parameters like <T>, found {}", self.found());
                    self.error(self.tok().span, message);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if !self.at(TokenKind::LBrace) {
            let message = format!(
                "expected `{{` after `{}`, found {}",
                self.text(keyword),
                self.found()
            );
            self.error(self.tok().span, message);
            return self.mk_expr(ExprKind::Error, keyword.span);
        }
        let body = self.type_body();
        let span = keyword.span.to(self.last_span());
        let kind = ExprKind::Call {
            callee: Box::new(head),
            type_args: type_params,
            args: Vec::new(),
            trailer: Some(Trailer {
                params: Vec::new(),
                ret: None,
                body,
                described: false,
            }),
            bare: false,
        };
        let definition = self.mk_expr(kind, span);
        // the clauses that follow the closing brace on its line: extends(Base) implements(A, B) use(T)
        let mut clauses = Vec::new();
        while matches!(
            self.tok().kind,
            TokenKind::Keyword(Keyword::Extends | Keyword::Implements | Keyword::Use)
        ) && !self.tok().starts_line
        {
            clauses.push(self.clause());
        }
        if clauses.is_empty() {
            return definition;
        }
        let end = clauses[clauses.len() - 1].span;
        let mut segments = vec![definition];
        segments.extend(clauses);
        self.mk_expr(ExprKind::Chain(segments), span.to(end))
    }

    /// `extends(Base)`, `implements(A, B)` or `use(T)`: a head and its arguments, which are type names.
    fn clause(&mut self) -> Expr {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);
        let args = if self.at(TokenKind::LParen) {
            self.arguments()
        } else {
            let message = format!(
                "expected `(` after `{}`, found {}",
                self.text(keyword),
                self.found()
            );
            self.error(self.tok().span, message);
            Vec::new()
        };
        let span = keyword.span.to(self.last_span());
        let kind = ExprKind::Call {
            callee: Box::new(head),
            type_args: Vec::new(),
            args,
            trailer: None,
            bare: false,
        };
        self.mk_expr(kind, span)
    }

    /// The members between `{` and `}`, with the cursor on the `{`.
    fn type_body(&mut self) -> Vec<Stmt> {
        self.bump();
        let outer = std::mem::replace(&mut self.nest, 0);
        let mut members = Vec::new();
        loop {
            while self.eat(TokenKind::Semi).is_some() {}
            if self.at(TokenKind::Eof) || self.at(TokenKind::RBrace) {
                break;
            }
            let start = self.pos;
            if let Some(member) = self.member() {
                members.push(member);
            }
            // the members are separated by a line break or `;`, never by a comma
            if self.at(TokenKind::Comma) {
                let comma = self.bump();
                self.error(
                    comma.span,
                    "separate the members with `;` or a new line, a comma is not used between them",
                );
            } else {
                let next = self.tok();
                let ends = next.starts_line
                    || matches!(
                        next.kind,
                        TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
                    );
                if !ends {
                    let message = format!("expected the end of the member, found {}", self.found());
                    self.error(next.span, message);
                    self.recover();
                }
            }
            if self.pos == start {
                self.bump();
            }
        }
        self.nest = outer;
        self.expect(TokenKind::RBrace);
        members
    }

    /// One member: a field, or a `const` or `let` declaration, with the attributes above it.
    pub(super) fn member(&mut self) -> Option<Stmt> {
        if self.at(TokenKind::LBracket) {
            return self.attributed_in(true);
        }
        match self.tok().kind {
            TokenKind::Keyword(Keyword::Const | Keyword::Let) => self.inferred_declaration(),
            TokenKind::Ident | TokenKind::Keyword(Keyword::Function) => {
                if let Some(field) = self.attempt(|p| p.field()) {
                    return Some(field);
                }
                Some(self.expression_or_assignment())
            }
            _ => Some(self.expression_or_assignment()),
        }
    }

    /// `Type name` or `Type name = default`
    fn field(&mut self) -> Option<Stmt> {
        let start = self.tok().span;
        let ty = self.ty_named()?;
        if !self.at(TokenKind::Ident) || self.tok().starts_line {
            return None;
        }
        let name = self.ident()?;
        let default = self.eat(TokenKind::Eq).map(|_| Box::new(self.expr()));
        let span = start.to(self.last_span());
        let kind = ExprKind::Param {
            ty,
            name,
            default,
            variadic: false,
        };
        let field = self.mk_expr(kind, span);
        Some(self.mk_stmt(StmtKind::Expr(field), span))
    }
}
