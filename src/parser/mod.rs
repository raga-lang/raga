//! Syntax analysis: turning tokens into the AST.
//!
//! The parser checks that the tokens form a program and builds the tree that shows how. It is written by hand as a
//! recursive-descent parser: one function per kind of construct, each calling the functions of the constructs inside it.
//! Expressions use precedence climbing, a loop that keeps a table of how tightly each operator binds.
//!
//! It is written by hand, not generated from a grammar, because Raga needs things a generator handles badly:
//!
//! - **Layout.** A statement ends at a line break, unless the line leaves a bracket open or ends with an operator.
//!   Each token knows whether it starts a line, and `nest` counts the open brackets.
//! - **Lookahead and backing up.** `u8 level = 255` is a declaration and `u8(300)` is a cast, and they start the same
//!   way. The parser tries the declaration, and if it does not fit, goes back and reads an expression.
//! - **One shape for calls and keyword forms.** `head (args) |params| { body }` is the shape of a call with its last
//!   argument written outside the parentheses, and `for (items) |item| { ... }` is the same shape with a keyword as the
//!   head. The parser has no rule of its own for a loop or a branch; what a head means is left to semantic analysis. A call
//!   can continue with more calls on the line of its closing brace (`if (a) { } elif (b) { } else { }`), which is one
//!   [`ExprKind::Chain`] of the segments.
//! - **Context.** `>` closes `List<List<i32>>` and is also an operator, `[` starts an attribute or a List, `{` starts
//!   a block or an Array. What the surrounding text allows decides.
//!
//! After an error the parser records it, skips to the next statement and carries on, so one mistake does not hide the
//! rest. A construct it cannot read becomes [`ExprKind::Error`] in the tree.

mod expr;
mod function;
mod ty;
mod typedef;

#[cfg(test)]
mod tests;

use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::lexer::{Keyword, Token, TokenKind};
use crate::span::Span;

/// What the parser made of one file.
#[derive(Debug)]
pub struct Parsed {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses the tokens of a file. `source` is the text the tokens point into, and `tokens` must end with
/// [`TokenKind::Eof`], as the lexer guarantees.
pub fn parse(source: &str, tokens: &[Token]) -> Parsed {
    let mut parser = Parser::new(source, tokens);
    let stmts = parser.statements(false);
    let program = Program {
        stmts,
        node_count: parser.next_id,
    };
    Parsed {
        program,
        diagnostics: parser.diagnostics,
    }
}

struct Parser<'a> {
    source: &'a str,
    tokens: &'a [Token],
    pos: usize,
    /// How many `(`, `[` or `{` of an expression are open. Inside them a line break means nothing, outside them it
    /// ends the statement. A block resets it to 0, because the statements in a block are separated by line breaks again.
    nest: u32,
    /// The id the next node gets.
    next_id: u32,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: &'a [Token]) -> Parser<'a> {
        assert!(
            tokens.last().is_some_and(|t| t.kind == TokenKind::Eof),
            "the tokens must end with Eof"
        );
        Parser {
            source,
            tokens,
            pos: 0,
            nest: 0,
            next_id: 0,
            diagnostics: Vec::new(),
        }
    }

    // ---- making nodes

    fn new_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    fn mk_expr(&mut self, kind: ExprKind, span: Span) -> Expr {
        Expr {
            id: self.new_id(),
            kind,
            span,
        }
    }

    fn mk_stmt(&mut self, kind: StmtKind, span: Span) -> Stmt {
        Stmt {
            id: self.new_id(),
            kind,
            span,
            attrs: Vec::new(),
        }
    }

    // ---- reading tokens

    fn tok(&self) -> Token {
        self.tok_at(0)
    }

    /// The token `ahead` places on. Past the end it stays on Eof.
    fn tok_at(&self, ahead: usize) -> Token {
        self.tokens[(self.pos + ahead).min(self.tokens.len() - 1)]
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.tok().kind == kind
    }

    /// Takes the current token. Eof is never taken, so the parser can always look at `tok()`.
    fn bump(&mut self) -> Token {
        let token = self.tok();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    /// The span of the token just taken. Used to close the span of a node.
    fn last_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn text(&self, token: Token) -> &'a str {
        token.text(self.source)
    }

    /// The current token as an error message names it: `;` or the end of the file.
    fn found(&self) -> String {
        let token = self.tok();
        match token.kind {
            TokenKind::Eof => token.kind.describe().to_string(),
            _ => format!("`{}`", self.text(token)),
        }
    }

    fn ident(&mut self) -> Option<Ident> {
        let token = self.tok();
        if token.kind != TokenKind::Ident {
            return None;
        }
        self.bump();
        Some(Ident {
            name: self.text(token).to_string(),
            span: token.span,
        })
    }

    // ---- errors and backing up

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::error(span, message));
    }

    /// Takes a token of the given kind or reports what was expected instead.
    fn expect(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            return Some(self.bump());
        }
        let message = format!("expected {}, found {}", kind.describe(), self.found());
        self.error(self.tok().span, message);
        None
    }

    /// Tries to read something. When `read` gives `None`, the parser goes back to where it was and forgets the errors
    /// that the attempt reported, as if it had never tried.
    fn attempt<T>(&mut self, read: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let (pos, nest, errors) = (self.pos, self.nest, self.diagnostics.len());
        let result = read(self);
        if result.is_none() {
            self.pos = pos;
            self.nest = nest;
            self.diagnostics.truncate(errors);
        }
        result
    }

    /// After a mistake: skips tokens up to the next place a statement can start.
    fn recover(&mut self) {
        loop {
            let token = self.tok();
            if matches!(
                token.kind,
                TokenKind::Eof | TokenKind::RBrace | TokenKind::Semi
            ) || token.starts_line
            {
                return;
            }
            self.bump();
        }
    }

    /// Reports a construct the parser does not read yet, and skips the line it is on.
    fn not_yet(&mut self, what: &str) {
        let token = self.bump();
        self.error(token.span, format!("{what} is not read by the parser yet"));
        self.recover();
    }

    // ---- statements

    /// Statements up to the end of the file, or up to the `}` that closes a block.
    fn statements(&mut self, in_block: bool) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        loop {
            while self.eat(TokenKind::Semi).is_some() {}
            match self.tok().kind {
                TokenKind::Eof => break,
                TokenKind::RBrace if in_block => break,
                _ => {}
            }
            let start = self.pos;
            let mut ends_with_block = false;
            if let Some(stmt) = self.statement() {
                ends_with_block = self.ends_with_block(&stmt);
                stmts.push(stmt);
            }
            if !ends_with_block {
                self.end_of_statement();
            }
            // a mistake must never leave the parser where it was, or it would loop forever
            if self.pos == start {
                self.bump();
            }
        }
        stmts
    }

    /// Does a statement end with the closing brace of its block, as a branch or a loop does? Then the next statement can
    /// follow on the same line: `if (n == 0) { return true } return is_odd(n - 1)`.
    fn ends_with_block(&self, stmt: &Stmt) -> bool {
        let StmtKind::Expr(expr) = &stmt.kind else {
            return false;
        };
        let is_block_call = |e: &Expr| {
            matches!(
                &e.kind,
                ExprKind::Call {
                    trailer: Some(_),
                    ..
                }
            )
        };
        let block = match &expr.kind {
            ExprKind::Chain(_) => true,
            _ => is_block_call(expr),
        };
        block && self.pos > 0 && self.tokens[self.pos - 1].kind == TokenKind::RBrace
    }

    /// A statement ends with `;`, at a line break, or at the `}` or the end of the file that closes what holds it.
    fn end_of_statement(&mut self) {
        let token = self.tok();
        if self.eat(TokenKind::Semi).is_some() {
            return;
        }
        if matches!(token.kind, TokenKind::Eof | TokenKind::RBrace) || token.starts_line {
            return;
        }
        let message = if self.assign_op().is_some() {
            "assignment is a statement and gives no value, so it cannot be used inside another one"
                .to_string()
        } else {
            format!("expected the end of the statement, found {}", self.found())
        };
        self.error(token.span, message);
        self.recover();
    }

    fn statement(&mut self) -> Option<Stmt> {
        let token = self.tok();
        match token.kind {
            TokenKind::Keyword(Keyword::Const | Keyword::Let) => self.inferred_declaration(),
            TokenKind::LBrace => Some(self.block()),
            // a `[` at the start of a line is an attribute, anywhere else it is a List
            TokenKind::LBracket if token.starts_line => self.attributed(),
            TokenKind::Keyword(
                Keyword::True | Keyword::False | Keyword::Null | Keyword::This | Keyword::Super,
            ) => Some(self.expression_or_assignment()),
            TokenKind::Keyword(keyword) if is_head(keyword) => self.form(),
            TokenKind::Keyword(Keyword::Break | Keyword::Continue | Keyword::Return) => {
                Some(self.jump())
            }
            TokenKind::Keyword(Keyword::Function) => match self.typed_declaration() {
                Some(stmt) => Some(stmt),
                None => Some(self.expression_or_assignment()),
            },
            TokenKind::Keyword(keyword) => {
                self.not_yet(&format!("`{}`", keyword.as_str()));
                None
            }
            TokenKind::Ident => match self.typed_declaration() {
                Some(stmt) => Some(stmt),
                None => Some(self.expression_or_assignment()),
            },
            _ => Some(self.expression_or_assignment()),
        }
    }

    /// Attributes and the statement below them. The attributes are kept on the statement.
    pub(super) fn attributed(&mut self) -> Option<Stmt> {
        self.attributed_in(false)
    }

    /// The same inside the body of a type, where `Type name` is a field and not a call.
    pub(super) fn attributed_in(&mut self, in_type: bool) -> Option<Stmt> {
        let attrs = self.attributes();
        let first = attrs.first().map(|a| a.span);
        let next = self.tok();
        // the statement the attributes describe may follow on the same line or on a line below
        if matches!(next.kind, TokenKind::Eof | TokenKind::RBrace) {
            if let Some(span) = first {
                self.error(
                    span,
                    "an attribute describes the declaration below it, and there is none",
                );
            }
            return None;
        }
        let mut stmt = if in_type {
            self.member()?
        } else {
            self.statement()?
        };
        let describes = match &stmt.kind {
            StmtKind::Decl(_) => true,
            StmtKind::Expr(e) => matches!(e.kind, ExprKind::Param { .. }),
            _ => false,
        };
        if !describes && let Some(span) = first {
            self.error(
                span,
                "an attribute describes a declaration, and the statement below it is not one",
            );
        }
        stmt.attrs = attrs;
        Some(stmt)
    }

    /// `[a(1), b] [c]`: one or more groups of attributes.
    pub(super) fn attributes(&mut self) -> Vec<Attribute> {
        let mut attrs = Vec::new();
        while self.at(TokenKind::LBracket) {
            self.bump();
            self.nest += 1;
            loop {
                // an attribute is named after what it describes, which is often a keyword: class, method
                let token = self.tok();
                if !matches!(token.kind, TokenKind::Ident | TokenKind::Keyword(_)) {
                    let message =
                        format!("expected the name of an attribute, found {}", self.found());
                    self.error(token.span, message);
                    break;
                }
                self.bump();
                let name = Ident {
                    name: self.text(token).to_string(),
                    span: token.span,
                };
                let args = if self.at(TokenKind::LParen) {
                    self.arguments()
                } else {
                    Vec::new()
                };
                let span = name.span.to(self.last_span());
                attrs.push(Attribute { name, args, span });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.nest -= 1;
            if self.expect(TokenKind::RBracket).is_none() {
                self.recover();
                break;
            }
        }
        attrs
    }

    /// `const name = value` or `let name = value`.
    pub(super) fn inferred_declaration(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let kind = if keyword.kind == TokenKind::Keyword(Keyword::Const) {
            DeclKind::Const
        } else {
            DeclKind::Let
        };
        // `const u8 x = 1` mixes the two ways to declare: say so, skip the type, and read on as a declaration
        if self.at(TokenKind::Ident) && self.tok_at(1).kind == TokenKind::Ident {
            let span = self.tok().span;
            self.error(
                span,
                format!(
                    "`{}` and a type cannot be combined: write `{} name = value` or `Type name = value`",
                    self.text(keyword),
                    self.text(keyword)
                ),
            );
            self.bump();
        }
        let Some(name) = self.ident() else {
            let message = format!(
                "expected a name after `{}`, found {}",
                self.text(keyword),
                self.found()
            );
            self.error(self.tok().span, message);
            self.recover();
            return None;
        };
        let init = self.initializer(&name);
        let span = keyword.span.to(init.span);
        Some(self.mk_stmt(StmtKind::Decl(Decl { kind, name, init }), span))
    }

    /// `Type name = value`. The type is tried first and, when what follows it is not a name, the parser backs up:
    /// the line was an expression such as `u8(300)` or a comparison such as `a < b`.
    fn typed_declaration(&mut self) -> Option<Stmt> {
        // `Type name` is written on one line: `a` and `b` on two lines are two statements
        if self.at(TokenKind::Ident) {
            let next = self.tok_at(1);
            // `shapes.Point name`: a type of a module has dots in it
            let dotted = next.kind == TokenKind::Dot
                && self.tok_at(2).kind == TokenKind::Ident
                && !next.starts_line;
            if !(matches!(next.kind, TokenKind::Ident | TokenKind::Lt) || dotted)
                || next.starts_line
            {
                return None;
            }
        }
        let start = self.tok().span;
        let before = self.pos;
        let ty = self.attempt(|p| {
            let ty = p.ty_named()?;
            (p.at(TokenKind::Ident) && !p.tok().starts_line).then_some(ty)
        })?;
        // `Type name = value` declares. Without the `=` it is not a declaration: `double base` is a call
        if self.tok_at(1).kind != TokenKind::Eq {
            self.pos = before;
            return None;
        }
        let name = self.ident()?;
        let init = self.initializer(&name);
        let span = start.to(init.span);
        let decl = Decl {
            kind: DeclKind::Typed(ty),
            name,
            init,
        };
        Some(self.mk_stmt(StmtKind::Decl(decl), span))
    }

    /// `= value` after the name of a declaration. A binding must be given a value when it is declared.
    fn initializer(&mut self, name: &Ident) -> Expr {
        if self.eat(TokenKind::Eq).is_some() {
            return self.expr();
        }
        self.error(
            name.span,
            format!(
                "`{}` needs a value: a binding must be initialized when it is declared, write `{} = ...`",
                name.name, name.name
            ),
        );
        self.mk_expr(ExprKind::Error, name.span)
    }

    /// `name = value`, `name += value`, or an expression used as a statement.
    pub(super) fn expression_or_assignment(&mut self) -> Stmt {
        let target = self.expr();
        let Some((op, width)) = self.assign_op() else {
            let span = target.span;
            return self.mk_stmt(StmtKind::Expr(target), span);
        };
        for _ in 0..width {
            self.bump();
        }
        if !matches!(
            target.kind,
            ExprKind::Name(_) | ExprKind::Member { .. } | ExprKind::Deref(_) | ExprKind::Error
        ) {
            self.error(
                target.span,
                "this cannot be assigned to, only a name, a member or `p.*` can",
            );
        }
        let value = self.expr();
        let span = target.span.to(value.span);
        self.mk_stmt(StmtKind::Assign(Assign { target, op, value }), span)
    }

    /// The assignment operator at the current token and how many tokens it takes. `>>=` arrives as `>` and `>=`
    /// because the lexer never joins a `>`.
    fn assign_op(&self) -> Option<(AssignOp, usize)> {
        let token = self.tok();
        if token.starts_line && self.nest == 0 {
            return None;
        }
        let op = match token.kind {
            TokenKind::Eq => AssignOp::Set,
            TokenKind::PlusEq => AssignOp::Add,
            TokenKind::MinusEq => AssignOp::Sub,
            TokenKind::StarEq => AssignOp::Mul,
            TokenKind::SlashEq => AssignOp::Div,
            TokenKind::PercentEq => AssignOp::Rem,
            TokenKind::AmpEq => AssignOp::BitAnd,
            TokenKind::PipeEq => AssignOp::BitOr,
            TokenKind::CaretEq => AssignOp::BitXor,
            TokenKind::ShlEq => AssignOp::Shl,
            TokenKind::Gt if self.joined_with_next(TokenKind::GtEq) => {
                return Some((AssignOp::Shr, 2));
            }
            _ => return None,
        };
        Some((op, 1))
    }

    /// True when the next token is of this kind and touches the current one, with nothing between them.
    fn joined_with_next(&self, kind: TokenKind) -> bool {
        let (current, next) = (self.tok(), self.tok_at(1));
        next.kind == kind && next.span.start == current.span.end
    }

    /// A statement that starts with a keyword: `for (items) |item| { ... }`, `if (c) { } else { }`, `while (c) { }`.
    ///
    /// It is read as the call it is, `head (args) |params| { body }`, with the keyword as the name that is called, and
    /// followed by the segments that continue it (`elif`, `else`) when they start on the line of the closing brace. There
    /// is no grammar of its own for a loop or a branch: the parentheses, the bars and the block are those of every call,
    /// and what a head means is decided by semantic analysis.
    fn form(&mut self) -> Option<Stmt> {
        let expr = self.chain()?;
        let span = expr.span;
        Some(self.mk_stmt(StmtKind::Expr(expr), span))
    }

    /// One call and the segments that continue it: a [`ExprKind::Chain`] when there are any, else the call itself.
    pub(super) fn chain(&mut self) -> Option<Expr> {
        let mut segments = vec![self.segment()?];
        while let TokenKind::Keyword(keyword) = self.tok().kind
            && is_link(keyword)
            && !self.tok().starts_line
        {
            match self.segment() {
                Some(segment) => segments.push(segment),
                None => break,
            }
        }
        if segments.len() == 1 {
            return segments.pop();
        }
        let span = segments[0].span.to(segments[segments.len() - 1].span);
        Some(self.mk_expr(ExprKind::Chain(segments), span))
    }

    /// One segment: a keyword, then `(args)`, `|params|` and `{ body }`, of which the parentheses may be left out when
    /// there are no arguments (`loop { }`, `else { }`, `default { }`).
    pub(super) fn segment(&mut self) -> Option<Expr> {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);
        if self.at(TokenKind::LParen) {
            return Some(self.call(head, Vec::new(), true));
        }
        // `try { }`, `finally { }` and `catch |e| { }` have no parentheses: a block, or bars and a block
        if self.at(TokenKind::LBrace) || self.at(TokenKind::Pipe) {
            let trailer = self.trailer(true);
            let span = keyword.span.to(self.last_span());
            let kind = ExprKind::Call {
                callee: Box::new(head),
                type_args: Vec::new(),
                args: Vec::new(),
                trailer,
                bare: false,
            };
            return Some(self.mk_expr(kind, span));
        }
        let message = format!(
            "expected `(` after `{}`, found {}",
            self.text(keyword),
            self.found()
        );
        self.error(self.tok().span, message);
        self.skip_to_block();
        None
    }

    /// `break` and `continue`, with the number of further loops to leave when one is written: `break 1`.
    fn jump(&mut self) -> Stmt {
        let keyword = self.bump();
        let head = Ident {
            name: self.text(keyword).to_string(),
            span: keyword.span,
        };
        let head = self.mk_expr(ExprKind::Name(head), keyword.span);
        let next = self.tok();
        let mut args = Vec::new();
        let ends = next.starts_line
            || matches!(
                next.kind,
                TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
            );
        if !ends {
            args.push(Arg {
                name: None,
                value: self.expr(),
            });
        }
        let span = keyword.span.to(self.last_span());
        let kind = ExprKind::Call {
            callee: Box::new(head),
            type_args: Vec::new(),
            args,
            trailer: None,
            bare: false,
        };
        let call = self.mk_expr(kind, span);
        self.mk_stmt(StmtKind::Expr(call), span)
    }

    /// After a mistake in the head of a form: reads the block on the same line anyway, or its statements would be read
    /// as if they stood alone and report mistakes of their own.
    fn skip_to_block(&mut self) {
        while !self.at(TokenKind::LBrace) && !self.at(TokenKind::Eof) && !self.tok().starts_line {
            self.bump();
        }
        if self.at(TokenKind::LBrace) {
            self.braced();
        }
    }

    /// `|a, b|` with the cursor on the first bar. `None` when it is not a list of names between bars.
    pub(super) fn block_params(&mut self) -> Option<Vec<Ident>> {
        self.eat(TokenKind::Pipe)?;
        let mut names = vec![self.ident()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.ident()?);
        }
        self.eat(TokenKind::Pipe)?;
        Some(names)
    }

    /// `{ ... }` where a statement can start: a scope.
    fn block(&mut self) -> Stmt {
        let open = self.tok().span;
        let stmts = self.braced();
        let span = open.to(self.last_span());
        self.mk_stmt(StmtKind::Block(stmts), span)
    }

    /// The statements between `{` and `}`, with the cursor on the `{`.
    fn braced(&mut self) -> Vec<Stmt> {
        self.bump();
        let outer = std::mem::replace(&mut self.nest, 0);
        let stmts = self.statements(true);
        self.nest = outer;
        self.expect(TokenKind::RBrace);
        stmts
    }
}

/// The keywords that start a call of their own, a branch or a loop: they are statements.
pub(super) fn is_head(keyword: Keyword) -> bool {
    use Keyword::*;
    matches!(
        keyword,
        If | Elif
            | Else
            | While
            | Loop
            | For
            | Switch
            | Match
            | Case
            | Default
            | Try
            | Catch
            | Finally
            | Throw
    )
}

/// The keywords that continue a call after its block, on the line of the closing brace.
fn is_link(keyword: Keyword) -> bool {
    matches!(
        keyword,
        Keyword::Elif | Keyword::Else | Keyword::Catch | Keyword::Finally
    )
}
