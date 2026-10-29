use super::TokenKind::*;
use super::*;

/// The kinds of all tokens, without the final Eof.
fn kinds(source: &str) -> Vec<TokenKind> {
    let lexed = lex(source);
    assert!(
        lexed.diagnostics.is_empty(),
        "unexpected diagnostics for {source:?}: {:?}",
        lexed.diagnostics
    );
    let mut kinds: Vec<_> = lexed.tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds.pop(), Some(Eof));
    kinds
}

/// The text of every token, without the final Eof.
fn texts(source: &str) -> Vec<&str> {
    let lexed = lex(source);
    let n = lexed.tokens.len() - 1;
    lexed.tokens[..n].iter().map(|t| t.text(source)).collect()
}

fn messages(source: &str) -> Vec<String> {
    lex(source)
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .collect()
}

// ---- layout

#[test]
fn empty_source_is_only_eof() {
    let lexed = lex("");
    assert_eq!(lexed.tokens.len(), 1);
    assert_eq!(lexed.tokens[0].kind, Eof);
}

#[test]
fn white_space_and_comments_are_dropped_but_comments_are_remembered() {
    let source = "const x = 1 // one\n// alone\nlet y = 2";
    assert_eq!(
        kinds(source),
        [
            Keyword(super::Keyword::Const),
            Ident,
            Eq,
            Int,
            Keyword(super::Keyword::Let),
            Ident,
            Eq,
            Int,
        ]
    );
    let comments = lex(source).comments;
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].slice(source), "// one");
    assert_eq!(comments[1].slice(source), "// alone");
}

#[test]
fn starts_line_marks_the_first_token_of_each_line() {
    let source = "a b\n  c ; d\n\n// note\ne";
    let flags: Vec<_> = lex(source).tokens.iter().map(|t| t.starts_line).collect();
    // a b | c ; d | e | Eof
    assert_eq!(flags, [true, false, true, false, false, true, true]);
}

#[test]
fn spans_point_back_at_the_text() {
    let source = "let total = 12";
    assert_eq!(texts(source), ["let", "total", "=", "12"]);
}

// ---- names and keywords

#[test]
fn keywords_are_told_from_identifiers() {
    use super::Keyword as K;
    assert_eq!(
        kinds("const let if elif else"),
        [
            Keyword(K::Const),
            Keyword(K::Let),
            Keyword(K::If),
            Keyword(K::Elif),
            Keyword(K::Else),
        ]
    );
    assert_eq!(kinds("constant lets iffy _x x_1 Point"), [Ident; 6]);
}

#[test]
fn contextual_words_stay_identifiers() {
    assert_eq!(kinds("get set call constructor destructor"), [Ident; 5]);
}

#[test]
fn literal_words_are_keywords() {
    use super::Keyword as K;
    assert_eq!(
        kinds("true false null this super"),
        [
            Keyword(K::True),
            Keyword(K::False),
            Keyword(K::Null),
            Keyword(K::This),
            Keyword(K::Super),
        ]
    );
}

#[test]
fn type_names_are_plain_identifiers() {
    assert_eq!(kinds("i32 u8 f64 bool char String List"), [Ident; 7]);
}

#[test]
fn dollar_is_a_name_on_its_own() {
    assert_eq!(kinds("$(x)"), [Ident, LParen, Ident, RParen]);
    assert_eq!(texts("$(x)")[0], "$");
}

// ---- numbers

#[test]
fn integers_in_every_base() {
    assert_eq!(
        kinds("255 0xFF 0XaB 0b1111_1111 0o377 1_000_000 0"),
        [Int; 7]
    );
    assert_eq!(texts("0b1111_1111 1_000"), ["0b1111_1111", "1_000"]);
}

#[test]
fn the_largest_literals_are_one_token_each() {
    let big = "340282366920938463463374607431768211455";
    assert_eq!(kinds(big), [Int]);
    assert_eq!(texts(big), [big]);
}

#[test]
fn a_minus_is_its_own_token() {
    assert_eq!(kinds("-5"), [Minus, Int]);
    assert_eq!(kinds("-9223372036854775808"), [Minus, Int]);
}

#[test]
fn floats() {
    assert_eq!(kinds("2.5 3.14159 1.5e3 2.5e-3 1e5 7.0"), [Float; 6]);
    assert_eq!(texts("1.5e3 2.5e-3"), ["1.5e3", "2.5e-3"]);
}

#[test]
fn a_dot_makes_a_float_only_before_a_digit() {
    assert_eq!(kinds("1.max"), [Int, Dot, Ident]);
    assert_eq!(kinds("x.0"), [Ident, Dot, Int]);
}

#[test]
fn an_e_after_a_number_is_a_name_error_not_an_exponent() {
    assert_eq!(messages("12e"), ["a number cannot be followed by letters"]);
}

#[test]
fn bad_numbers_are_reported() {
    assert_eq!(
        messages("0x"),
        ["expected hexadecimal digits after the prefix"]
    );
    assert_eq!(messages("0b102"), ["'2' is not a binary digit"]);
    assert_eq!(messages("0o9"), ["'9' is not a octal digit"]);
    assert_eq!(
        messages("12abc"),
        ["a number cannot be followed by letters"]
    );
}

// ---- text and characters

#[test]
fn strings_and_characters() {
    assert_eq!(kinds(r#""text" "" "a \"quoted\" word" "line\n""#), [Str; 4]);
    assert_eq!(kinds(r"'a' '\n' '\'' '\\' '\0' 'é'"), [Char; 6]);
    assert_eq!(texts(r#""a b" 'x'"#), [r#""a b""#, "'x'"]);
}

#[test]
fn a_string_may_hold_slashes_and_non_ascii_text() {
    assert_eq!(kinds("\"// not a comment é\""), [Str]);
}

#[test]
fn an_unclosed_string_is_reported_and_still_a_token() {
    let lexed = lex("const s = \"abc\nlet x = 1");
    assert_eq!(lexed.diagnostics.len(), 1);
    assert!(lexed.diagnostics[0].message.contains("never closed"));
    // the lexer went on: the second line is intact
    assert_eq!(lexed.tokens.iter().filter(|t| t.kind == Eof).count(), 1);
    assert!(
        lexed
            .tokens
            .iter()
            .any(|t| t.kind == Keyword(super::Keyword::Let))
    );
}

#[test]
fn unknown_escapes_are_reported() {
    assert_eq!(messages(r#""\q""#).len(), 1);
    // \' is for characters and \" is for strings
    assert_eq!(messages(r#""\'""#).len(), 1);
    assert_eq!(messages(r"'\\'").len(), 0);
    assert_eq!(messages(r#"'\"'"#).len(), 1);
}

#[test]
fn bad_characters_are_reported() {
    assert_eq!(
        messages("''"),
        ["a character literal needs a character between the quotes"]
    );
    assert_eq!(messages("'ab'").len(), 1);
    assert_eq!(messages("'a").len(), 1);
}

// ---- punctuation and operators

#[test]
fn single_and_double_operators() {
    assert_eq!(
        kinds("+ - * / % & | ^ ~ ! < > = ."),
        [
            Plus, Minus, Star, Slash, Percent, Amp, Pipe, Caret, Tilde, Bang, Lt, Gt, Eq, Dot,
        ]
    );
    assert_eq!(
        kinds("== != <= >= && || <<"),
        [EqEq, NotEq, LtEq, GtEq, AndAnd, OrOr, Shl]
    );
    assert_eq!(
        kinds("+= -= *= /= %= &= |= ^= <<="),
        [
            PlusEq, MinusEq, StarEq, SlashEq, PercentEq, AmpEq, PipeEq, CaretEq, ShlEq,
        ]
    );
    assert_eq!(
        kinds("( ) { } [ ] , ;"),
        [
            LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Semi,
        ]
    );
}

#[test]
fn the_longest_operator_wins() {
    assert_eq!(kinds("a<<=b"), [Ident, ShlEq, Ident]);
    assert_eq!(kinds("a<=b"), [Ident, LtEq, Ident]);
    assert_eq!(kinds("a<b"), [Ident, Lt, Ident]);
}

#[test]
fn greater_than_is_never_joined_so_generics_close_cleanly() {
    // List<List<i32>> ends with two separate '>' tokens
    assert_eq!(
        kinds("List<List<i32>>"),
        [Ident, Lt, Ident, Lt, Ident, Gt, Gt]
    );
    // x >> 1 is two tokens that touch, the parser reads them as the shift
    assert_eq!(kinds("x>>1"), [Ident, Gt, Gt, Int]);
    // x >>= 1 is '>' followed by '>=', the parser joins them
    assert_eq!(kinds("x>>=1"), [Ident, Gt, GtEq, Int]);
}

#[test]
fn ellipsis_is_one_token_and_dots_are_not() {
    assert_eq!(kinds("i32 ...rest"), [Ident, Ellipsis, Ident]);
    assert_eq!(kinds("a.b"), [Ident, Dot, Ident]);
    // p.* is a dot and a star, the parser reads it as a dereference
    assert_eq!(kinds("p.*"), [Ident, Dot, Star]);
}

#[test]
fn a_pipe_pair_is_or_and_a_single_pipe_is_a_pipe() {
    assert_eq!(kinds("|n| { }"), [Pipe, Ident, Pipe, LBrace, RBrace]);
    assert_eq!(kinds("a || b"), [Ident, OrOr, Ident]);
}

#[test]
fn unexpected_characters_are_reported_and_skipped() {
    let lexed = lex("a @ b");
    assert_eq!(lexed.diagnostics.len(), 1);
    assert_eq!(lexed.diagnostics[0].message, "unexpected character '@'");
    let kinds: Vec<_> = lexed.tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [Ident, Ident, Eof]);
    // a character that takes several bytes is skipped whole
    assert_eq!(lex("a ∑ b").diagnostics.len(), 1);
}

// ---- a few real lines of Raga

#[test]
fn a_typed_declaration() {
    assert_eq!(kinds("u8 level = 255"), [Ident, Ident, Eq, Int]);
}

#[test]
fn a_cast_and_a_call_with_a_member() {
    assert_eq!(
        kinds("const t5 = u8(300)"),
        [
            Keyword(super::Keyword::Const),
            Ident,
            Eq,
            Ident,
            LParen,
            Int,
            RParen
        ]
    );
    assert_eq!(kinds("text.upper()"), [Ident, Dot, Ident, LParen, RParen]);
}

#[test]
fn statements_on_one_line() {
    assert_eq!(
        kinds("let y = 2; let z = 3")
            .iter()
            .filter(|k| **k == Semi)
            .count(),
        1
    );
}
