//! The vocabulary of the lexer: what a token is.

use crate::span::Span;

/// One piece of the source: its kind, where it is, and whether it begins a line.
///
/// A token does not own its text. The text is `span.slice(source)`, so a token is small and `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// True when this is the first token on its line. Raga reads layout: a statement ends at a line break, a
    /// clause such as `else` must start on the line of the previous `}`, and a `[` at the start of a line is an
    /// attribute while any other `[` is a List. The parser needs this to decide, so the lexer records it.
    pub starts_line: bool,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        self.span.slice(source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // names and literals
    Ident,
    Keyword(Keyword),
    /// `255`, `0xFF`, `0b1010`, `0o17`, `1_000`. Its value is worked out later, because it may not fit any machine integer.
    Int,
    /// `2.5`, `1.5e3`
    Float,
    /// `'a'`, `'\n'`
    Char,
    /// `"text"`
    Str,

    // brackets
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // separators
    Comma,
    Semi,
    Dot,
    /// `...`, the variadic parameter
    Ellipsis,

    // arithmetic and bit operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    /// `<<`. There is no `>>` token, see [`TokenKind::Gt`].
    Shl,

    // comparison and logic
    Lt,
    LtEq,
    /// `>`. The lexer never joins two of them: `>>` in `List<List<i32>>` closes two type argument lists. The parser
    /// joins two `>` that touch into the shift operator, and `>` followed by `>=` into `>>=`.
    Gt,
    GtEq,
    EqEq,
    NotEq,
    AndAnd,
    OrOr,

    // assignment
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,

    /// The end of the file. The parser can always look at the current token without checking for the end.
    Eof,
}

impl TokenKind {
    /// How an error message names this kind of token: "expected `)`", "expected a name".
    pub fn describe(self) -> &'static str {
        use TokenKind::*;
        match self {
            Ident => "a name",
            Keyword(_) => "a keyword",
            Int => "an integer",
            Float => "a decimal number",
            Char => "a character",
            Str => "a string",
            LParen => "`(`",
            RParen => "`)`",
            LBrace => "`{`",
            RBrace => "`}`",
            LBracket => "`[`",
            RBracket => "`]`",
            Comma => "`,`",
            Semi => "`;`",
            Dot => "`.`",
            Ellipsis => "`...`",
            Plus => "`+`",
            Minus => "`-`",
            Star => "`*`",
            Slash => "`/`",
            Percent => "`%`",
            Amp => "`&`",
            Pipe => "`|`",
            Caret => "`^`",
            Tilde => "`~`",
            Bang => "`!`",
            Shl => "`<<`",
            Lt => "`<`",
            LtEq => "`<=`",
            Gt => "`>`",
            GtEq => "`>=`",
            EqEq => "`==`",
            NotEq => "`!=`",
            AndAnd => "`&&`",
            OrOr => "`||`",
            Eq => "`=`",
            PlusEq => "`+=`",
            MinusEq => "`-=`",
            StarEq => "`*=`",
            SlashEq => "`/=`",
            PercentEq => "`%=`",
            AmpEq => "`&=`",
            PipeEq => "`|=`",
            CaretEq => "`^=`",
            ShlEq => "`<<=`",
            Eof => "the end of the file",
        }
    }
}

macro_rules! keywords {
    ($($variant:ident => $text:literal,)*) => {
        /// A reserved word. It cannot be used as a name.
        ///
        /// `get`, `set`, `call`, `constructor` and `destructor` are not here: they mean something only in one place
        /// (a class body, an attribute), so they stay ordinary identifiers and the parser decides.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Keyword {
            $($variant,)*
        }

        impl Keyword {
            pub fn from_text(text: &str) -> Option<Keyword> {
                match text {
                    $($text => Some(Keyword::$variant),)*
                    _ => None,
                }
            }

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Keyword::$variant => $text,)*
                }
            }
        }
    };
}

keywords! {
    Const => "const",
    Let => "let",
    Function => "function",
    Method => "method",
    Class => "class",
    Interface => "interface",
    Trait => "trait",
    Struct => "struct",
    Union => "union",
    Enum => "enum",
    Attribute => "attribute",
    If => "if",
    Elif => "elif",
    Else => "else",
    For => "for",
    While => "while",
    Loop => "loop",
    Break => "break",
    Continue => "continue",
    Return => "return",
    Try => "try",
    Catch => "catch",
    Finally => "finally",
    Throw => "throw",
    Match => "match",
    Switch => "switch",
    Case => "case",
    Default => "default",
    Extends => "extends",
    Implements => "implements",
    Use => "use",
    True => "true",
    False => "false",
    Null => "null",
    This => "this",
    Super => "super",
}
