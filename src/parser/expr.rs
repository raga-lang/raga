//! Expressions.
//!
//! An expression is read in three layers, from the loosest binding to the tightest:
//!
//! 1. `binary`: operators between two values, `a + b * c`, by **precedence climbing**.
//! 2. `unary`: an operator in front of one value, `-x`, `!ok`, `~bits`, `&var`.
//! 3. `postfix`: what follows a value, `.name`, `.*` and `(args)`, then `primary`: a literal, a name, a bracket.
//!
//! Postfix binds tighter than prefix, as the language says: `-p.*` is `-(p.*)`.
//!
//! The same rule decides every line break. At `nest == 0` a token that starts a line cannot continue the expression,
//! so `1` followed by a line that starts with `+ 2` is two statements. Inside brackets, line breaks are ignored.

use std::num::IntErrorKind;

use super::Parser;
use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};
use crate::span::Span;

impl Parser<'_> {
    pub(super) fn expr(&mut self) -> Expr {
        self.binary(1)
    }

    // ---- binary operators

    /// Reads operators that bind at least as tightly as `min`. Precedence climbing: read one operand, then as long as
    /// the next operator binds tightly enough, read its right side (which may itself take tighter operators) and fold.
    /// The right side is read with `prec + 1`, so equal operators group to the left: `a - b - c` is `(a - b) - c`.
    fn binary(&mut self, min: u8) -> Expr {
        let mut lhs = self.unary();
        while let Some((op, prec, width)) = self.binary_op() {
            if prec < min {
                break;
            }
            for _ in 0..width {
                self.bump();
            }
            let rhs = self.binary(prec + 1);
            let span = lhs.span.to(rhs.span);
            let kind = ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
            lhs = self.mk_expr(kind, span);
        }
        lhs
    }

    /// The binary operator at the current token, how tightly it binds (a bigger number binds tighter), and how many
    /// tokens it is written with. These are rows 3 to 12 of the precedence table in 01-basics section 7.
    fn binary_op(&self) -> Option<(BinaryOp, u8, usize)> {
        let token = self.tok();
        if token.starts_line && self.nest == 0 {
            return None;
        }
        let (op, prec) = match token.kind {
            TokenKind::Star => (BinaryOp::Mul, 10),
            TokenKind::Slash => (BinaryOp::Div, 10),
            TokenKind::Percent => (BinaryOp::Rem, 10),
            TokenKind::Plus => (BinaryOp::Add, 9),
            TokenKind::Minus => (BinaryOp::Sub, 9),
            TokenKind::Shl => (BinaryOp::Shl, 8),
            // `>>` is two `>` that touch. `>>=` is a `>` followed by `>=` and is an assignment, not an operator.
            TokenKind::Gt if self.joined_with_next(TokenKind::Gt) => {
                return Some((BinaryOp::Shr, 8, 2));
            }
            TokenKind::Gt if self.joined_with_next(TokenKind::GtEq) => return None,
            TokenKind::Gt => (BinaryOp::Greater, 7),
            TokenKind::GtEq => (BinaryOp::GreaterEq, 7),
            TokenKind::Lt => (BinaryOp::Less, 7),
            TokenKind::LtEq => (BinaryOp::LessEq, 7),
            TokenKind::EqEq => (BinaryOp::Equal, 6),
            TokenKind::NotEq => (BinaryOp::NotEqual, 6),
            TokenKind::Amp => (BinaryOp::BitAnd, 5),
            TokenKind::Caret => (BinaryOp::BitXor, 4),
            TokenKind::Pipe => (BinaryOp::BitOr, 3),
            TokenKind::AndAnd => (BinaryOp::And, 2),
            TokenKind::OrOr => (BinaryOp::Or, 1),
            _ => return None,
        };
        Some((op, prec, 1))
    }

    // ---- prefix operators

    fn unary(&mut self) -> Expr {
        let token = self.tok();
        let op = match token.kind {
            TokenKind::Minus => UnaryOp::Neg,
            TokenKind::Bang => UnaryOp::Not,
            TokenKind::Tilde => UnaryOp::BitNot,
            TokenKind::Amp => UnaryOp::AddrOf,
            _ => return self.postfix(),
        };
        self.bump();
        let operand = self.unary();
        let span = token.span.to(operand.span);
        let kind = ExprKind::Unary {
            op,
            operand: Box::new(operand),
        };
        self.mk_expr(kind, span)
    }

    // ---- postfix: member access, dereference, calls

    fn postfix(&mut self) -> Expr {
        let mut expr = self.primary();
        loop {
            let token = self.tok();
            if token.starts_line && self.nest == 0 {
                break;
            }
            match token.kind {
                TokenKind::Dot => {
                    self.bump();
                    if self.eat(TokenKind::Star).is_some() {
                        let span = expr.span.to(self.last_span());
                        expr = self.mk_expr(ExprKind::Deref(Box::new(expr)), span);
                    } else if let Some(name) = self.ident() {
                        let span = expr.span.to(name.span);
                        let kind = ExprKind::Member {
                            object: Box::new(expr),
                            name,
                        };
                        expr = self.mk_expr(kind, span);
                    } else {
                        let message = format!("expected a name after `.`, found {}", self.found());
                        self.error(token.span, message);
                        break;
                    }
                }
                TokenKind::LParen => expr = self.call(expr, Vec::new(), false),
                // `List<i32>()`: type arguments before the parentheses. Only a name can have them, and only when the
                // `<` really opens a list of types that the `(` follows. Otherwise it is the operator `<`.
                TokenKind::Lt
                    if matches!(expr.kind, ExprKind::Name(_) | ExprKind::Member { .. })
                        && matches!(
                            self.tok_at(1).kind,
                            TokenKind::Ident | TokenKind::Keyword(Keyword::Function)
                        ) =>
                {
                    // `identity<f64>(2.5)` is a call, and `identity_of<i32>` followed by the end of the expression is
                    // the function with its type arguments filled in
                    let parsed = self.attempt(|p| {
                        let args = p.type_args()?;
                        if p.at(TokenKind::LParen) {
                            return Some((args, true));
                        }
                        let next = p.tok();
                        let ends = (next.starts_line && p.nest == 0)
                            || matches!(
                                next.kind,
                                TokenKind::Comma
                                    | TokenKind::Dot
                                    | TokenKind::RParen
                                    | TokenKind::RBrace
                                    | TokenKind::RBracket
                                    | TokenKind::Semi
                                    | TokenKind::Eof
                            );
                        ends.then_some((args, false))
                    });
                    match parsed {
                        Some((type_args, true)) => expr = self.call(expr, type_args, false),
                        Some((type_args, false)) => {
                            let span = expr.span.to(self.last_span());
                            let kind = ExprKind::TypeApply {
                                target: Box::new(expr),
                                type_args,
                            };
                            expr = self.mk_expr(kind, span);
                        }
                        None => break,
                    }
                }
                // `double base`: a call without parentheses, whose one argument is a name
                TokenKind::Ident if is_callee(&expr) => expr = self.bare_call(expr),
                // `user { } select { } limit (1)`: after a call that ended with its block, a name continues the chain
                TokenKind::Ident if self.closed_call(&expr) => expr = self.link(expr),
                // `timed { ... }` and `timed |x| { ... }`: a call with no arguments and a block. After the bars a `{`
                // must follow, because without it `f |x| y` is the operator.
                TokenKind::LBrace | TokenKind::Pipe if is_callee(&expr) => {
                    match self.trailer(false) {
                        Some(trailer) => {
                            let span = expr.span.to(self.last_span());
                            let kind = ExprKind::Call {
                                callee: Box::new(expr),
                                type_args: Vec::new(),
                                args: Vec::new(),
                                trailer: Some(trailer),
                                bare: true,
                            };
                            expr = self.mk_expr(kind, span);
                        }
                        None => break,
                    }
                }
                // `print "hi"`, `add 1`: a value written out needs the parentheses
                TokenKind::Int
                | TokenKind::Float
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::Keyword(Keyword::True | Keyword::False)
                    if is_callee(&expr) =>
                {
                    let callee = expr.span.slice(self.source).to_string();
                    let value = self.text(token).to_string();
                    self.bump();
                    self.error(
                        expr.span.to(token.span),
                        format!("a call with a written-out value needs parentheses: write {callee}({value})"),
                    );
                }
                _ => break,
            }
        }
        expr
    }

    /// Is this a call that was just closed by its `)` or its block, which a name on the same line can then continue? The name
    /// must start a step of its own: arguments, bars or a block follow it. A function definition is not continued: it is a
    /// value.
    fn closed_call(&self, expr: &Expr) -> bool {
        let ExprKind::Call { callee, .. } = &expr.kind else {
            return false;
        };
        let definition = matches!(&callee.kind, ExprKind::Name(name) if name.name == "function");
        let closed = self.pos > 0
            && matches!(
                self.tokens[self.pos - 1].kind,
                TokenKind::RBrace | TokenKind::RParen
            );
        let step = matches!(
            self.tok_at(1).kind,
            TokenKind::LParen | TokenKind::Pipe | TokenKind::LBrace
        );
        !definition && closed && step && !(self.tok().starts_line && self.nest == 0)
    }

    /// The next step of a chain, with the cursor on its name: a call of a member of the value that the call before it
    /// gave back.
    ///
    /// Calls that return values are chained with a dot, `user().select().limit(1)`. When the last argument of a call is
    /// written as a block (a callback, an Array or a struct), a dot after the closing brace would look strange, so the
    /// dot is left out and the next name follows the brace on the same line: `user { } select { } limit (1)`. It is the
    /// same chaining of calls, each step a call of a member of the step before it, with the block as its last argument.
    fn link(&mut self, previous: Expr) -> Expr {
        let name = self.ident().expect("the caller checked for a name");
        let span = previous.span.to(name.span);
        let member = ExprKind::Member {
            object: Box::new(previous),
            name,
        };
        let member = self.mk_expr(member, span);
        if self.at(TokenKind::LParen) {
            return self.call(member, Vec::new(), false);
        }
        if matches!(self.tok().kind, TokenKind::LBrace | TokenKind::Pipe)
            && let Some(trailer) = self.trailer(false)
        {
            let span = member.span.to(self.last_span());
            let kind = ExprKind::Call {
                callee: Box::new(member),
                type_args: Vec::new(),
                args: Vec::new(),
                trailer: Some(trailer),
                bare: true,
            };
            return self.mk_expr(kind, span);
        }
        member
    }

    /// `double base`, with the cursor on the name: the one argument is a name, with dots after it, and it can itself be
    /// the name of a call without parentheses, so `a b c` is `a(b(c))`, or a name with a block, so `double sum { 1, 2 }` is
    /// `double(sum({ 1, 2 }))`. Nothing else is an argument without parentheses;
    /// `double base + 1` is `(double base) + 1`.
    fn bare_call(&mut self, callee: Expr) -> Expr {
        let mut operand = self.primary();
        while self.at(TokenKind::Dot)
            && self.tok_at(1).kind == TokenKind::Ident
            && !self.tok().starts_line
        {
            self.bump();
            let name = self.ident().expect("a name follows the dot");
            let span = operand.span.to(name.span);
            let kind = ExprKind::Member {
                object: Box::new(operand),
                name,
            };
            operand = self.mk_expr(kind, span);
        }
        let next = self.tok();
        if next.kind == TokenKind::Ident && !(next.starts_line && self.nest == 0) {
            operand = self.bare_call(operand);
        } else if matches!(next.kind, TokenKind::LBrace | TokenKind::Pipe)
            && let Some(trailer) = self.trailer(false)
        {
            // `double sum { 1, 2, 3 }`: the name with a block is a call too, and it is the argument of the first
            let span = operand.span.to(self.last_span());
            let kind = ExprKind::Call {
                callee: Box::new(operand),
                type_args: Vec::new(),
                args: Vec::new(),
                trailer: Some(trailer),
                bare: true,
            };
            operand = self.mk_expr(kind, span);
            while self.closed_call(&operand) && self.at(TokenKind::Ident) {
                operand = self.link(operand);
            }
        }
        let span = callee.span.to(operand.span);
        let kind = ExprKind::Call {
            callee: Box::new(callee),
            type_args: Vec::new(),
            args: vec![Arg {
                name: None,
                value: operand,
            }],
            trailer: None,
            bare: true,
        };
        self.mk_expr(kind, span)
    }

    /// `(a, b = 2)`, with the cursor on the `(`.
    ///
    /// `strict` is true for a keyword head such as `for`: a malformed trailer is then an error, where after a name it may
    /// be an operator (`f(x) | y`).
    pub(super) fn call(&mut self, callee: Expr, type_args: Vec<Type>, strict: bool) -> Expr {
        let args = self.arguments();
        let trailer = self.trailer(strict);
        let span = callee.span.to(self.last_span());
        let kind = ExprKind::Call {
            callee: Box::new(callee),
            type_args,
            args,
            trailer,
            bare: false,
        };
        self.mk_expr(kind, span)
    }

    /// `(a, b = 2)`, with the cursor on the `(`.
    pub(super) fn arguments(&mut self) -> Vec<Arg> {
        self.bump();
        self.nest += 1;
        let mut args = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let name = if self.at(TokenKind::Ident) && self.tok_at(1).kind == TokenKind::Eq {
                let name = self.ident();
                self.bump();
                name
            } else {
                None
            };
            let value = self.expr();
            args.push(Arg { name, value });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.nest -= 1;
        if !self.at(TokenKind::RParen) {
            let message = format!("expected `,` or `)`, found {}", self.found());
            self.error(self.tok().span, message);
        } else {
            self.bump();
        }
        args
    }

    /// The last argument written outside the parentheses: `|params| { body }` or `{ body }`, on the line of the `)`.
    ///
    /// After a name the bars are taken as parameters only when a `{` follows them, because `f(x) | y` is also an
    /// operator. After a keyword head (`strict`) they are always parameters.
    pub(super) fn trailer(&mut self, strict: bool) -> Option<Trailer> {
        if self.tok().starts_line && self.nest == 0 {
            return None;
        }
        let params = if strict {
            if self.at(TokenKind::Pipe) {
                match self.block_params() {
                    Some(params) => params,
                    None => {
                        let message =
                            format!("expected names between `|` and `|`, found {}", self.found());
                        self.error(self.tok().span, message);
                        self.skip_to_block();
                        return None;
                    }
                }
            } else {
                Vec::new()
            }
        } else {
            self.attempt(|p| {
                let params = p.block_params()?;
                (p.at(TokenKind::LBrace) && !p.tok().starts_line).then_some(params)
            })
            .unwrap_or_default()
        };
        if !self.at(TokenKind::LBrace) {
            if strict {
                // bars with no block after them are a mistake of their own; no block at all is for the head to report,
                // because some heads need one and others do not
                if !params.is_empty() {
                    let message =
                        format!("expected `{{` to start the block, found {}", self.found());
                    self.error(self.tok().span, message);
                }
                self.recover();
            }
            return None;
        }
        let body = if strict {
            self.braced()
        } else {
            self.trailer_body()
        };
        Some(Trailer {
            params,
            ret: None,
            body,
            described: false,
        })
    }

    // ---- primary: literals, names, brackets

    fn primary(&mut self) -> Expr {
        let token = self.tok();
        let kind = match token.kind {
            TokenKind::Int => {
                self.bump();
                ExprKind::Int(self.int_value(token.span))
            }
            TokenKind::Float => {
                self.bump();
                ExprKind::Float(self.text(token).replace('_', ""))
            }
            TokenKind::Char => {
                self.bump();
                ExprKind::Char(
                    unescape(&self.text(token)[1..], '\'')
                        .chars()
                        .next()
                        .unwrap_or('\0'),
                )
            }
            TokenKind::Str => {
                self.bump();
                ExprKind::Str(unescape(&self.text(token)[1..], '"'))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                ExprKind::Bool(true)
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                ExprKind::Bool(false)
            }
            TokenKind::Keyword(Keyword::Null) => {
                self.bump();
                ExprKind::Null
            }
            TokenKind::Ident => {
                let name = self.ident().expect("the current token is a name");
                ExprKind::Name(name)
            }
            TokenKind::Keyword(Keyword::Function) => return self.function_expr(),
            TokenKind::Keyword(Keyword::Attribute) => return self.attribute_expr(),
            TokenKind::Keyword(
                Keyword::Struct
                | Keyword::Enum
                | Keyword::Union
                | Keyword::Class
                | Keyword::Interface
                | Keyword::Trait,
            ) => {
                return self.type_expr();
            }
            TokenKind::Keyword(Keyword::Method) => return self.method_expr(),
            TokenKind::Keyword(Keyword::This | Keyword::Super) => {
                self.bump();
                let name = Ident {
                    name: self.text(token).to_string(),
                    span: token.span,
                };
                ExprKind::Name(name)
            }
            TokenKind::LParen => return self.parenthesized(),
            TokenKind::LBracket => return self.collection(TokenKind::RBracket, ExprKind::List),
            TokenKind::LBrace => return self.collection(TokenKind::RBrace, ExprKind::Array),
            TokenKind::Keyword(keyword) if super::is_head(keyword) => {
                // read the whole construct, so that its blocks are not read again as statements
                let name = self.text(token);
                let message = format!(
                    "`{name}` is a statement and gives no value, so it cannot be used here: assign inside its blocks instead"
                );
                let parsed = self.chain();
                self.error(token.span, message);
                let span = parsed.map_or(token.span, |p| p.span);
                return self.mk_expr(ExprKind::Error, span);
            }
            TokenKind::Keyword(Keyword::Break | Keyword::Continue) => {
                self.bump();
                let message = format!("`{}` is a statement and gives no value", self.text(token));
                self.error(token.span, message);
                ExprKind::Error
            }
            _ => {
                // Nothing is taken: the token may be the `)` or `;` that the caller is waiting for.
                let message = format!("expected an expression, found {}", self.found());
                self.error(token.span, message);
                ExprKind::Error
            }
        };
        let span = match kind {
            ExprKind::Error => token.span,
            _ => token.span.to(self.last_span()),
        };
        self.mk_expr(kind, span)
    }

    /// `(x)` is a grouping, `(a, b)` is a Tuple, `()` is the empty Tuple.
    fn parenthesized(&mut self) -> Expr {
        let open = self.bump();
        self.nest += 1;
        let mut items = Vec::new();
        let mut trailing_comma = false;
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            items.push(self.expr());
            trailing_comma = self.eat(TokenKind::Comma).is_some();
            if !trailing_comma {
                break;
            }
        }
        self.nest -= 1;
        self.expect(TokenKind::RParen);
        let span = open.span.to(self.last_span());
        if items.len() == 1 && !trailing_comma {
            return items.pop().expect("one item");
        }
        if items.len() == 1 {
            self.error(
                span,
                "a Tuple has two or more values, write Tuple(x) for one",
            );
        }
        self.mk_expr(ExprKind::Tuple(items), span)
    }

    /// `{1, 2, 3}` or `[1, 2, 3]`, with the cursor on the opening bracket.
    fn collection(&mut self, close: TokenKind, make: fn(Vec<Expr>) -> ExprKind) -> Expr {
        let open = self.bump();
        self.nest += 1;
        let mut items = Vec::new();
        while !self.at(close) && !self.at(TokenKind::Eof) {
            items.push(self.expr());
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.nest -= 1;
        self.expect(close);
        let span = open.span.to(self.last_span());
        self.mk_expr(make(items), span)
    }

    // ---- literal values

    /// The value of an integer literal: `255`, `0xFF`, `0b1111_1111`, `0o377`, `1_000`. The lexer has already checked
    /// the digits, so the only mistake left is a number too large for the largest integer type.
    fn int_value(&mut self, span: Span) -> u128 {
        let text = span.slice(self.source).replace('_', "");
        let (digits, radix) = match text.get(..2) {
            Some("0x" | "0X") => (&text[2..], 16),
            Some("0b" | "0B") => (&text[2..], 2),
            Some("0o" | "0O") => (&text[2..], 8),
            _ => (text.as_str(), 10),
        };
        match u128::from_str_radix(digits, radix) {
            Ok(value) => value,
            Err(err) if *err.kind() == IntErrorKind::PosOverflow => {
                self.error(
                    span,
                    "this integer is larger than the largest integer type, u128",
                );
                0
            }
            // an empty or malformed literal: the lexer reported it already
            Err(_) => 0,
        }
    }
}

/// The text of a string or character literal with its escapes resolved. `rest` is what follows the opening quote;
/// reading stops at the closing quote, or at the end if the literal was never closed.
fn unescape(rest: &str, quote: char) -> String {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c == quote => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                // `\\`, an escaped quote, and anything unknown (the lexer reported it) stand for themselves
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

/// Can this expression be what a call without parentheses names: a name, or a name with dots after it?
fn is_callee(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(_) => true,
        // a member of anything can be called: `divide(10, 0).or_else |reason| { ... }`
        ExprKind::Member { .. } => true,
        _ => false,
    }
}
