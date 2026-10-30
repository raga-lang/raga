//! Function definitions and the block after a call.
//!
//! A function is a value, written like every other call of the language, `head (args) [Type] { body }`:
//!
//! ```text
//! function<T>(T value, i32 times = 1) T { ... }
//! ```
//!
//! The head is `function`, the arguments are the parameters (each a declaration without `const` or `let`), the type before
//! the block is the type it gives back, and the block is the body. So a definition is an ordinary [`ExprKind::Call`] of
//! `function`, with [`ExprKind::Param`] for its arguments. Binding it to a name is the ordinary `const name = ...`.

use super::Parser;
use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};

impl Parser<'_> {
    /// `function(...) ... { ... }`, with the cursor on `function`.
    pub(super) fn function_expr(&mut self) -> Expr {
        self.definition(false)
    }

    /// `method(...) ... { ... }` of a class, and the versions of it that follow on the line of the closing brace:
    /// `method() String { } method(String name) String { }`. A method with no block is abstract.
    pub(super) fn method_expr(&mut self) -> Expr {
        let first = self.definition(true);
        if !self.at(TokenKind::Keyword(Keyword::Method)) || self.tok().starts_line {
            return first;
        }
        let mut versions = vec![first];
        while self.at(TokenKind::Keyword(Keyword::Method)) && !self.tok().starts_line {
            versions.push(self.definition(true));
        }
        let span = versions[0].span.to(versions[versions.len() - 1].span);
        self.mk_expr(ExprKind::Chain(versions), span)
    }

    /// One definition: `function` or `method`, its parameters, the type it gives back and its block.
    fn definition(&mut self, is_method: bool) -> Expr {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);

        // the type parameters: `function<T, U>(...)`
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

        if self.expect(TokenKind::LParen).is_none() {
            self.recover();
            return self.mk_expr(ExprKind::Error, keyword.span);
        }
        self.nest += 1;
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            match self.param() {
                Some(param) => params.push(Arg {
                    name: None,
                    value: param,
                }),
                None => break,
            }
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.nest -= 1;
        if self.expect(TokenKind::RParen).is_none() {
            self.recover();
            return self.mk_expr(ExprKind::Error, keyword.span);
        }

        // the type it gives back, when one is written before the block, on the line of the parentheses
        let ret = if self.at(TokenKind::LBrace) || self.tok().starts_line {
            None
        } else {
            self.attempt(|p| p.ty())
        };
        // a method with no block is abstract: it is only described
        if is_method && !self.at(TokenKind::LBrace) {
            let span = keyword.span.to(self.last_span());
            let kind = ExprKind::Call {
                callee: Box::new(head),
                type_args: type_params,
                args: params,
                trailer: Some(Trailer {
                    params: Vec::new(),
                    ret,
                    body: Vec::new(),
                    described: true,
                }),
                bare: false,
            };
            return self.mk_expr(kind, span);
        }
        if !self.at(TokenKind::LBrace) {
            let message = format!(
                "expected `{{` to start the body of the function, found {}",
                self.found()
            );
            self.error(self.tok().span, message);
            self.recover();
            return self.mk_expr(ExprKind::Error, keyword.span);
        }
        let body = self.braced();
        let span = keyword.span.to(self.last_span());
        let kind = ExprKind::Call {
            callee: Box::new(head),
            type_args: type_params,
            args: params,
            trailer: Some(Trailer {
                params: Vec::new(),
                ret,
                body,
                described: false,
            }),
            bare: false,
        };
        self.mk_expr(kind, span)
    }

    /// `attribute(String path) |target, path| { } call |target, args, next| { }`, with the cursor on `attribute`: the
    /// declaration of an attribute. The parentheses hold what it carries, the block after them runs when the program is
    /// built, and a `call` block after that runs on every call. Each part is optional, so the whole is a chain of calls.
    pub(super) fn attribute_expr(&mut self) -> Expr {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);
        if self.expect(TokenKind::LParen).is_none() {
            self.recover();
            return self.mk_expr(ExprKind::Error, keyword.span);
        }
        self.nest += 1;
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            match self.param() {
                Some(param) => params.push(Arg {
                    name: None,
                    value: param,
                }),
                None => break,
            }
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.nest -= 1;
        if self.expect(TokenKind::RParen).is_none() {
            self.recover();
            return self.mk_expr(ExprKind::Error, keyword.span);
        }
        let trailer = if matches!(self.tok().kind, TokenKind::Pipe | TokenKind::LBrace)
            && !self.tok().starts_line
        {
            self.trailer(true)
        } else {
            None
        };
        let span = keyword.span.to(self.last_span());
        let first = self.mk_expr(
            ExprKind::Call {
                callee: Box::new(head),
                type_args: Vec::new(),
                args: params,
                trailer,
                bare: false,
            },
            span,
        );
        // the call hooks, each on the line of what came before it (a second one is for the checker to report)
        let mut parts = vec![first];
        loop {
            let next = self.tok();
            if next.kind != TokenKind::Ident
                || next.starts_line
                || self.text(next) != "call"
                || !matches!(self.tok_at(1).kind, TokenKind::Pipe | TokenKind::LBrace)
            {
                break;
            }
            let word = self.ident().expect("the current token is a name");
            let callee = self.mk_expr(ExprKind::Name(word.clone()), word.span);
            let trailer = self.trailer(true);
            let span = word.span.to(self.last_span());
            parts.push(self.mk_expr(
                ExprKind::Call {
                    callee: Box::new(callee),
                    type_args: Vec::new(),
                    args: Vec::new(),
                    trailer,
                    bare: true,
                },
                span,
            ));
        }
        if parts.len() == 1 {
            return parts.pop().expect("one part");
        }
        let span = parts[0].span.to(parts[parts.len() - 1].span);
        self.mk_expr(ExprKind::Chain(parts), span)
    }

    /// A parameter: `Type name`, `Type name = default`, `Type ...name`.
    fn param(&mut self) -> Option<Expr> {
        let start = self.tok().span;
        let Some(ty) = self.ty() else {
            let message = format!(
                "expected a parameter like `Type name`, found {}",
                self.found()
            );
            self.error(self.tok().span, message);
            return None;
        };
        let variadic = self.eat(TokenKind::Ellipsis).is_some();
        let Some(name) = self.ident() else {
            let message = format!("expected the name of the parameter, found {}", self.found());
            self.error(self.tok().span, message);
            return None;
        };
        let default = self.eat(TokenKind::Eq).map(|_| Box::new(self.expr()));
        let span = start.to(self.last_span());
        let kind = ExprKind::Param {
            ty,
            name,
            default,
            variadic,
        };
        Some(self.mk_expr(kind, span))
    }

    // ---- the block after a call

    /// The block after the arguments of a call, with the cursor on the `{`: the statements of a function body, or the
    /// values of an Array when the block is `{ 4, 8, 15 }`. Whether it is a function, an Array or a struct is for the type
    /// of the last parameter to say; a comma between values is the one thing that settles it here, because statements are
    /// separated by line breaks.
    pub(super) fn trailer_body(&mut self) -> Vec<Stmt> {
        if self.block_has_values() {
            let start = self.tok().span;
            if let Some(items) = self.attempt(|p| p.array_values()) {
                let span = start.to(self.last_span());
                let array = self.mk_expr(ExprKind::Array(items), span);
                return vec![self.mk_stmt(StmtKind::Expr(array), span)];
            }
        }
        self.braced()
    }

    /// Does a comma stand between the braces, outside any brackets and bars? Then it is a list of values.
    fn block_has_values(&self) -> bool {
        let first = self.tok_at(1);
        if matches!(first.kind, TokenKind::Keyword(keyword)
            if !matches!(keyword, Keyword::True | Keyword::False | Keyword::Null | Keyword::Function))
        {
            return false;
        }
        let (mut depth, mut in_bars) = (0usize, false);
        for ahead in 1.. {
            match self.tok_at(ahead).kind {
                TokenKind::Eof => return false,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                TokenKind::RBrace if depth == 0 => return false,
                TokenKind::RBrace => depth -= 1,
                TokenKind::Pipe if depth == 0 => in_bars = !in_bars,
                TokenKind::Comma if depth == 0 && !in_bars => return true,
                _ => {}
            }
        }
        false
    }

    /// `{ 4, 8, 15 }`, with the cursor on the `{`.
    fn array_values(&mut self) -> Option<Vec<Expr>> {
        self.eat(TokenKind::LBrace)?;
        self.nest += 1;
        let mut items = vec![self.expr()];
        while self.eat(TokenKind::Comma).is_some() {
            if self.at(TokenKind::RBrace) {
                break;
            }
            items.push(self.expr());
        }
        self.nest -= 1;
        self.eat(TokenKind::RBrace)?;
        Some(items)
    }
}
