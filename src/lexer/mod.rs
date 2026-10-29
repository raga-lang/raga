//! Lexical analysis: turning text into tokens.
//!
//! The lexer is the first stage. It reads the characters of a file from left to right and groups them into tokens:
//! `const total = 1_000 + x // sum` becomes `const`, `total`, `=`, `1_000`, `+`, `x`. White space and comments are
//! thrown away (comments are remembered by position), because the stages after it care only about the tokens.
//!
//! What the lexer does not do: it does not know whether `x` was declared, whether `+` has operands, or what a
//! number's value is. It decides only where each token starts and ends and what kind of token it is. That keeps it
//! small, and it means a file with broken syntax still gives tokens.
//!
//! The whole file is lexed at once into a `Vec`. The parser looks ahead and backs up (it must try a typed
//! declaration `u8 x = 1` and fall back to an expression `u8(300)`), and a vector makes that a plain index.

mod token;

#[cfg(test)]
mod tests;

pub use token::{Keyword, Token, TokenKind};

use crate::diagnostics::Diagnostic;
use crate::span::Span;

/// Everything the lexer found in one file.
#[derive(Debug, Default)]
pub struct Lexed {
    /// The tokens in order. The last one is always [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Where each `//` comment is. Kept for documentation comments and tools.
    pub comments: Vec<Span>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Turns source text into tokens. It never fails: a bad character or an unfinished literal becomes a diagnostic
/// and the lexer carries on, so one mistake does not hide the rest of the file.
pub fn lex(source: &str) -> Lexed {
    Lexer::new(source).run()
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    /// Offset of the next unread byte. It is always on a character boundary.
    pos: usize,
    /// True until the first token of a line is emitted.
    starts_line: bool,
    out: Lexed,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Lexer<'a> {
        Lexer {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            starts_line: true,
            out: Lexed::default(),
        }
    }

    fn run(mut self) -> Lexed {
        while let Some(b) = self.peek() {
            match b {
                b'\n' => {
                    self.pos += 1;
                    self.starts_line = true;
                }
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'/' if self.peek_at(1) == Some(b'/') => self.line_comment(),
                b'"' => self.string(),
                b'\'' => self.character(),
                b'0'..=b'9' => self.number(),
                b'$' => {
                    // `$` is the name of the built-in format function, $("%d", n). It is a whole name on its own.
                    let start = self.pos;
                    self.pos += 1;
                    self.emit(TokenKind::Ident, start);
                }
                b if is_ident_start(b) => self.word(),
                _ => self.punctuation(),
            }
        }
        let end = self.source.len();
        self.out.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(end, end),
            starts_line: true,
        });
        self.out
    }

    // ---- reading helpers

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    /// Reads one whole character, which may be several bytes.
    fn bump_char(&mut self) -> Option<char> {
        let c = self.source[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn emit(&mut self, kind: TokenKind, start: usize) {
        self.out.tokens.push(Token {
            kind,
            span: Span::new(start, self.pos),
            starts_line: self.starts_line,
        });
        self.starts_line = false;
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        self.out
            .diagnostics
            .push(Diagnostic::error(Span::new(start, end), message));
    }

    // ---- comments and names

    /// `//` to the end of the line. There are no block comments.
    fn line_comment(&mut self) {
        let start = self.pos;
        // a line break is one ASCII byte and never part of a longer character, so scanning bytes is safe
        while self.peek().is_some_and(|b| b != b'\n') {
            self.pos += 1;
        }
        self.out.comments.push(Span::new(start, self.pos));
    }

    /// An identifier or a keyword. They are read the same way and told apart by looking the text up.
    fn word(&mut self) {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_continue) {
            self.pos += 1;
        }
        let kind = match Keyword::from_text(&self.source[start..self.pos]) {
            Some(keyword) => TokenKind::Keyword(keyword),
            None => TokenKind::Ident,
        };
        self.emit(kind, start);
    }

    // ---- numbers

    /// `255`, `0xFF`, `0b1111_1111`, `0o377`, `1_000_000`, `3.14`, `1.5e-3`.
    ///
    /// The lexer finds the end of the number and checks its digits. It does not compute the value: a literal such as
    /// `340282366920938463463374607431768211455` is a u128 and `-9223372036854775808` is a minus in front of a number
    /// that does not fit an i64 by itself. Whether it fits is decided once its type is known.
    fn number(&mut self) {
        let start = self.pos;
        let radix = match (self.peek(), self.peek_at(1)) {
            (Some(b'0'), Some(b'x' | b'X')) => Some(16),
            (Some(b'0'), Some(b'b' | b'B')) => Some(2),
            (Some(b'0'), Some(b'o' | b'O')) => Some(8),
            _ => None,
        };
        if let Some(radix) = radix {
            self.pos += 2;
            let digits = self.pos;
            while self.peek().is_some_and(is_ident_continue) {
                self.pos += 1;
            }
            self.check_radix_digits(start, digits, radix);
            self.emit(TokenKind::Int, start);
            return;
        }

        let mut kind = TokenKind::Int;
        self.digits();
        // a `.` makes a float only when a digit follows, so `1.max` and `t.0.1` are not read as floats
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            kind = TokenKind::Float;
            self.pos += 1;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let sign = matches!(self.peek_at(1), Some(b'+' | b'-'));
            let first = self.peek_at(if sign { 2 } else { 1 });
            if first.is_some_and(|b| b.is_ascii_digit()) {
                kind = TokenKind::Float;
                self.pos += if sign { 2 } else { 1 };
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
        }
        // a letter glued to a number (12abc) is a mistake, not a number followed by a name
        if self.peek().is_some_and(is_ident_continue) {
            let bad = self.pos;
            while self.peek().is_some_and(is_ident_continue) {
                self.pos += 1;
            }
            self.error(bad, self.pos, "a number cannot be followed by letters");
        }
        self.emit(kind, start);
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'_') {
            self.pos += 1;
        }
    }

    /// Checks the digits of `0x...`, `0b...` and `0o...` once the whole run of letters and digits is read.
    fn check_radix_digits(&mut self, start: usize, digits: usize, radix: u32) {
        let text = &self.source[digits..self.pos];
        let name = match radix {
            16 => "hexadecimal",
            8 => "octal",
            _ => "binary",
        };
        if !text.chars().any(|c| c != '_') {
            self.error(
                start,
                self.pos,
                format!("expected {name} digits after the prefix"),
            );
            return;
        }
        if let Some((i, c)) = text
            .char_indices()
            .find(|&(_, c)| c != '_' && !c.is_digit(radix))
        {
            let at = digits + i;
            self.error(
                at,
                at + c.len_utf8(),
                format!("'{c}' is not a {name} digit"),
            );
        }
    }

    // ---- text and characters

    /// `"text"`. The token covers the quotes and the escapes exactly as written; the text itself is read later.
    /// A string ends at the closing quote and cannot run over a line break.
    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    self.error(
                        start,
                        self.pos,
                        "this string is never closed, it needs a closing \"",
                    );
                    break;
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => self.escape(b'"'),
                Some(_) => {
                    self.bump_char();
                }
            }
        }
        self.emit(TokenKind::Str, start);
    }

    /// `'a'`, `'\n'`, `'é'`: exactly one Unicode character between single quotes.
    fn character(&mut self) {
        let start = self.pos;
        self.pos += 1;
        match self.peek() {
            None | Some(b'\n') => {
                self.error(
                    start,
                    self.pos,
                    "this character is never closed, it needs a closing '",
                );
                self.emit(TokenKind::Char, start);
                return;
            }
            Some(b'\'') => {
                self.pos += 1;
                self.error(
                    start,
                    self.pos,
                    "a character literal needs a character between the quotes",
                );
                self.emit(TokenKind::Char, start);
                return;
            }
            Some(b'\\') => self.escape(b'\''),
            Some(_) => {
                self.bump_char();
            }
        }
        if self.peek() == Some(b'\'') {
            self.pos += 1;
        } else {
            // more than one character, as in 'ab': skip to the closing quote on this line, so that one mistake is
            // one error and the quote is not read as the start of another literal
            let rest = &self.bytes[self.pos..];
            let line = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            if let Some(close) = rest[..line].iter().position(|&b| b == b'\'') {
                self.pos += close + 1;
            }
            self.error(
                start,
                self.pos,
                "a character literal holds one character and ends with '",
            );
        }
        self.emit(TokenKind::Char, start);
    }

    /// A backslash escape, with the cursor on the backslash. The allowed escapes are `\n \t \r \\ \0` and the quote
    /// that surrounds the literal: `\"` in a string, `\'` in a character.
    fn escape(&mut self, quote: u8) {
        let start = self.pos;
        self.pos += 1;
        match self.peek() {
            Some(b'n' | b't' | b'r' | b'\\' | b'0') => self.pos += 1,
            Some(b) if b == quote => self.pos += 1,
            // the line break or the end of the file is reported by the caller as an unclosed literal
            None | Some(b'\n') => {}
            Some(_) => {
                self.bump_char();
                self.error(
                    start,
                    self.pos,
                    "unknown escape, use \\n \\t \\r \\\\ \\0 or an escaped quote",
                );
            }
        }
    }

    // ---- punctuation and operators

    fn punctuation(&mut self) {
        use TokenKind::*;
        let start = self.pos;
        let c0 = self.peek();
        let c1 = self.peek_at(1);
        let c2 = self.peek_at(2);
        // longest match first: `<<=` before `<<` before `<`
        let (kind, len) = match (c0, c1, c2) {
            (Some(b'<'), Some(b'<'), Some(b'=')) => (ShlEq, 3),
            (Some(b'.'), Some(b'.'), Some(b'.')) => (Ellipsis, 3),
            (Some(b'<'), Some(b'<'), _) => (Shl, 2),
            (Some(b'<'), Some(b'='), _) => (LtEq, 2),
            (Some(b'>'), Some(b'='), _) => (GtEq, 2),
            (Some(b'='), Some(b'='), _) => (EqEq, 2),
            (Some(b'!'), Some(b'='), _) => (NotEq, 2),
            (Some(b'&'), Some(b'&'), _) => (AndAnd, 2),
            (Some(b'|'), Some(b'|'), _) => (OrOr, 2),
            (Some(b'+'), Some(b'='), _) => (PlusEq, 2),
            (Some(b'-'), Some(b'='), _) => (MinusEq, 2),
            (Some(b'*'), Some(b'='), _) => (StarEq, 2),
            (Some(b'/'), Some(b'='), _) => (SlashEq, 2),
            (Some(b'%'), Some(b'='), _) => (PercentEq, 2),
            (Some(b'&'), Some(b'='), _) => (AmpEq, 2),
            (Some(b'|'), Some(b'='), _) => (PipeEq, 2),
            (Some(b'^'), Some(b'='), _) => (CaretEq, 2),
            (Some(b'('), ..) => (LParen, 1),
            (Some(b')'), ..) => (RParen, 1),
            (Some(b'{'), ..) => (LBrace, 1),
            (Some(b'}'), ..) => (RBrace, 1),
            (Some(b'['), ..) => (LBracket, 1),
            (Some(b']'), ..) => (RBracket, 1),
            (Some(b','), ..) => (Comma, 1),
            (Some(b';'), ..) => (Semi, 1),
            (Some(b'.'), ..) => (Dot, 1),
            (Some(b'+'), ..) => (Plus, 1),
            (Some(b'-'), ..) => (Minus, 1),
            (Some(b'*'), ..) => (Star, 1),
            (Some(b'/'), ..) => (Slash, 1),
            (Some(b'%'), ..) => (Percent, 1),
            (Some(b'&'), ..) => (Amp, 1),
            (Some(b'|'), ..) => (Pipe, 1),
            (Some(b'^'), ..) => (Caret, 1),
            (Some(b'~'), ..) => (Tilde, 1),
            (Some(b'!'), ..) => (Bang, 1),
            (Some(b'<'), ..) => (Lt, 1),
            (Some(b'>'), ..) => (Gt, 1),
            (Some(b'='), ..) => (Eq, 1),
            _ => {
                // not part of the language: report it, skip it, and go on with the next character
                let c = self.bump_char().unwrap_or('\0');
                self.error(start, self.pos, format!("unexpected character '{c}'"));
                return;
            }
        };
        self.pos += len;
        self.emit(kind, start);
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
