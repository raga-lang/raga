use crate::lexer::lex;
use crate::parser::parse;
use crate::sema::analyze;

/// Every message from the lexer, the parser and the checker, in that order.
fn messages(source: &str) -> Vec<String> {
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);
    let analysis = analyze(&parsed.program);
    lexed
        .diagnostics
        .iter()
        .chain(&parsed.diagnostics)
        .chain(&analysis.diagnostics)
        .map(|d| d.message.clone())
        .collect()
}

/// The program is accepted.
fn ok(source: &str) {
    let found = messages(source);
    assert!(
        found.is_empty(),
        "expected no errors for {source:?}, found {found:?}"
    );
}

/// The program has exactly one error and its message contains the text.
fn fails(source: &str, containing: &str) {
    let found = messages(source);
    assert_eq!(
        found.len(),
        1,
        "expected one error for {source:?}, found {found:?}"
    );
    assert!(
        found[0].contains(containing),
        "the error for {source:?} is {:?}, which does not contain {containing:?}",
        found[0]
    );
}

/// The program has errors, and one of them contains the text. For a mistake that is reported more than once.
fn reports(source: &str, containing: &str) {
    let found = messages(source);
    assert!(
        found.iter().any(|m| m.contains(containing)),
        "expected an error containing {containing:?} for {source:?}, found {found:?}"
    );
}

/// The type of the last variable with this name, as it prints.
fn type_of(source: &str, name: &str) -> String {
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let analysis = analyze(&parsed.program);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    analysis
        .symbol_named(name)
        .unwrap_or_else(|| panic!("no variable {name}"))
        .ty
        .to_string()
}

// ---- inference

#[test]
fn a_literal_with_no_context_takes_its_default_type() {
    assert_eq!(type_of("const n = 1", "n"), "i32");
    assert_eq!(type_of("const f = 2.5", "f"), "f64");
    assert_eq!(type_of("const b = true", "b"), "bool");
    assert_eq!(type_of("const c = 'a'", "c"), "char");
    assert_eq!(type_of("const s = \"text\"", "s"), "String");
}

#[test]
fn a_declared_type_is_the_type_of_the_name() {
    assert_eq!(type_of("u8 level = 255", "level"), "u8");
    assert_eq!(type_of("f32 half = 2.0", "half"), "f32");
    assert_eq!(
        type_of("i128 big = 170141183460469231731687303715884105727", "big"),
        "i128"
    );
}

#[test]
fn a_name_has_the_type_of_its_value() {
    assert_eq!(type_of("u8 a = 1\nconst b = a", "b"), "u8");
    assert_eq!(type_of("let v = 1\nconst w = v + 1", "w"), "i32");
}

#[test]
fn collections_get_their_element_type() {
    assert_eq!(type_of("const a = {1, 2, 3}", "a"), "Array<i32>");
    assert_eq!(type_of("const l = [1.5, 2.5]", "l"), "List<f64>");
    assert_eq!(type_of("const t = (1, 2.5)", "t"), "Tuple<i32, f64>");
    assert_eq!(type_of("List<u8> l = [1, 2]", "l"), "List<u8>");
    assert_eq!(type_of("List<u8> l = []", "l"), "List<u8>");
}

#[test]
fn the_items_of_a_collection_must_agree() {
    // whole numbers and decimals do not mix; with a type to hold them, the decimal is asked to be that type
    fails("const bad = {1, 2.5}", "the others are whole numbers");
    fails("i32 ok = 1\nconst bad = {ok, 2.5}", "never an integer");
    fails(
        "const bad = {1, \"a\"}",
        "all the items must have the same type",
    );
    ok("u8 b = 1\nconst a = {1, b}");
    assert_eq!(type_of("u8 b = 1\nconst a = {1, b}", "a"), "Array<u8>");
}

#[test]
fn an_empty_literal_needs_a_type_from_its_context() {
    fails("const xs = []", "an empty literal has no items");
    fails("const xs = {}", "an empty literal has no items");
}

// ---- literals and ranges

#[test]
fn integers_must_fit_their_type() {
    ok("u8 a = 255");
    fails("u8 a = 256", "256 does not fit in u8");
    fails("i8 a = 128", "128 does not fit in i8");
    ok("i8 a = -128");
    fails("i8 a = -129", "-129 does not fit in i8");
    fails("u8 a = -1", "-1 does not fit in u8");
    ok("i64 a = -9223372036854775808");
    fails("i64 a = 9223372036854775808", "does not fit in i64");
    ok("u64 a = 18446744073709551615");
    ok("u128 a = 340282366920938463463374607431768211455");
}

#[test]
fn a_number_with_no_context_must_fit_an_i32() {
    ok("const a = 2147483647");
    ok("const a = -2147483648");
    fails("const huge = 3000000000", "3000000000 does not fit in i32");
}

#[test]
fn a_whole_number_is_never_a_float() {
    fails("f32 half = 2", "never a float");
    fails("f64 x = 1", "write 1.0");
    ok("f32 half = 2.0");
}

#[test]
fn a_decimal_is_never_an_integer() {
    fails("i32 x = 2.5", "never an integer");
    fails("u8 x = 1.0", "never an integer");
}

#[test]
fn a_literal_takes_the_type_of_the_other_side() {
    assert_eq!(type_of("u8 b = 250\nconst r = b + 5", "r"), "u8");
    assert_eq!(type_of("usize n = 3\nconst r = n == 3", "r"), "bool");
    assert_eq!(type_of("const r = i64(1) + 1", "r"), "i64");
    ok("u8 b = 250\nconst r = 5 + b");
    fails("u8 b = 250\nconst r = b + 300", "300 does not fit in u8");
}

#[test]
fn a_literal_expression_takes_the_expected_type() {
    ok("u8 x = 1 + 2");
    // each literal fits a u8, and unsigned arithmetic wraps, so the sum is not an error
    ok("u8 x = 200 + 100");
    fails("u8 x = 300 + 1", "300 does not fit in u8");
}

// ---- types must match

#[test]
fn there_is_no_silent_conversion() {
    fails(
        "i32 whole = 5\ni64 wide = whole",
        "i32 is not i64, convert it first",
    );
    ok("i32 whole = 5\ni64 wide = i64(whole)");
    fails("u8 small = 1\nsmall = i32(1)", "i32 is not u8");
    ok("u8 small = 1\nsmall = u8(i32(1))");
}

#[test]
fn a_variable_keeps_its_type() {
    fails("let v = 1\nv = 2.5", "never an integer");
    fails("let v = 1\nv = \"text\"", "String is not i32");
    ok("u8 small = 250\nsmall = 251");
    fails("u8 small = 250\nsmall = 300", "300 does not fit in u8");
}

#[test]
fn both_sides_of_an_operator_have_the_same_type() {
    fails("i32(1) + i64(1)", "i32 and i64");
    ok("i64(i32(1)) + i64(1)");
    fails("const mixed = 1.5 * 2", "write 2.0");
    fails("const mixed = 1 + 2.5", "never an integer");
    ok("const m = 1.5 * 2.0");
    fails("u8 a = 1\ni8 b = 1\nconst c = a == b", "u8 and i8");
}

// ---- operators

#[test]
fn arithmetic_needs_numbers() {
    fails("const a = \"a\" + \"b\"", "use String.join");
    fails("const a = true + false", "needs numbers");
    fails("const a = 'a' * 2", "char and i32");
}

#[test]
fn comparison_gives_a_bool() {
    assert_eq!(type_of("const a = 1 < 2", "a"), "bool");
    assert_eq!(type_of("const a = 'a' == 'b'", "a"), "bool");
    assert_eq!(type_of("const a = \"a\" == \"a\"", "a"), "bool");
    fails("const a = true < false", "compares numbers and characters");
}

#[test]
fn logic_needs_bools() {
    assert_eq!(type_of("const a = true && false", "a"), "bool");
    fails("const a = true && 2", "needs bool operands");
    // both sides are wrong, so there are two errors
    assert_eq!(messages("const a = 1 && 2").len(), 2);
    fails("const a = !1", "`!` needs a bool");
}

#[test]
fn bit_operators_need_integers() {
    assert_eq!(type_of("const a = 0b1100 & 0b1010", "a"), "i32");
    assert_eq!(type_of("const a = ~u8(0)", "a"), "u8");
    assert_eq!(type_of("const a = 1 << 4", "a"), "i32");
    fails("const a = 1.5 & 2.5", "needs integers");
    fails("const a = ~1.5", "`~` needs an integer");
}

#[test]
fn negation_needs_a_signed_type() {
    assert_eq!(type_of("const a = -5", "a"), "i32");
    assert_eq!(type_of("const a = -2.5", "a"), "f64");
    fails("u8 x = 1\nconst y = -x", "cannot negate u8");
    fails("const y = -\"a\"", "cannot negate String");
}

#[test]
fn compound_assignment_follows_the_operator() {
    ok("let v = 1\nv += 3");
    ok("u8 b = 1\nb <<= 2");
    fails("let v = 1\nv += 2.5", "never an integer");
    fails("let s = \"a\"\ns += \"b\"", "use String.join");
    fails("let b = true\nb *= false", "needs numbers");
    // the 1 is asked to be an f64, and `<<` needs integers anyway: both are reported
    let found = messages("let f = 1.5\nf <<= 1");
    assert!(
        found.iter().any(|m| m.contains("needs integers")),
        "{found:?}"
    );
}

// ---- names and scope

#[test]
fn a_name_must_be_declared_before_it_is_used() {
    fails("print(unknown)", "`unknown` is not declared");
    fails(
        "const early = later\nconst later = 2",
        "`later` is declared below",
    );
    fails("const sum2 = 1 + missing", "`missing` is not declared");
    ok("const known = 1\nprint(known)");
}

#[test]
fn a_name_is_not_visible_in_its_own_declaration() {
    fails("const x = x + 1", "`x` is declared below");
    ok("const x = 1\n{\n  const x = x + 1\n}"
        .replace("const x = x + 1", "const y = x + 1")
        .as_str());
}

#[test]
fn a_block_has_its_own_scope() {
    ok("const outer = 1\n{\n  const inner = 2\n  print(outer + inner)\n}");
    fails(
        "{\n  const inner = 2\n}\nprint(inner)",
        "`inner` is out of scope here",
    );
}

#[test]
fn an_inner_name_hides_an_outer_one() {
    ok("const sh = 1\n{\n  const sh = 2.5\n  print(sh)\n}\nprint(sh)");
    assert_eq!(type_of("const sh = 1\n{\n  const sh = 2.5\n}", "sh"), "f64");
}

#[test]
fn a_name_is_declared_once_per_scope() {
    fails(
        "const a = 1\nconst a = 2",
        "`a` is already declared in this scope",
    );
    ok("const a = 1\n{\n  const a = 2\n}");
}

#[test]
fn const_cannot_be_assigned() {
    fails("const a = 1\na = 2", "`a` is const and cannot be assigned");
    fails("const a = 1\na += 2", "`a` is const");
    ok("let a = 1\na = 2");
}

#[test]
fn assignment_needs_a_declaration() {
    fails(
        "known2 = 5",
        "assignment needs a declaration: write `let known2 = ...`",
    );
}

#[test]
fn built_in_names_cannot_be_declared_at_the_top_level() {
    fails("const print = 1", "`print` is built in");
    fails("const List = 1", "`List` is a built-in type");
    fails("const u8 = 1", "`u8` is a built-in type");
    // inside a block it only hides the built-in name there
    ok("{\n  const print = 1\n}");
}

#[test]
fn a_value_is_needed_where_a_value_is_stored() {
    fails("const x = print(1)", "gives no value");
    fails("print(print(1))", "gives no value");
}

#[test]
fn null_is_not_a_value() {
    fails("const x = null", "null has no type of its own");
}

// ---- conversions

#[test]
fn conversions_give_the_named_type() {
    assert_eq!(type_of("const a = u8(300)", "a"), "u8");
    assert_eq!(type_of("const a = f64(3)", "a"), "f64");
    assert_eq!(type_of("const a = i32(3.99)", "a"), "i32");
    assert_eq!(type_of("const a = bool(0)", "a"), "bool");
    assert_eq!(type_of("const a = String(42)", "a"), "String");
    assert_eq!(type_of("const a = i32(\"42\")", "a"), "i32");
    assert_eq!(type_of("const a = char(u32(66))", "a"), "char");
    assert_eq!(type_of("const a = u32(char('A'))", "a"), "u32");
}

#[test]
fn a_conversion_converts_the_value_it_is_given() {
    // 300 is an i32 here, and the conversion cuts it down: it is not a u8 literal that does not fit
    ok("const a = u8(300)");
    ok("const a = u8(i32(-1))");
}

#[test]
fn some_conversions_do_not_exist() {
    fails("const a = char(66)", "cannot convert i32 to char");
    fails("const a = bool(\"x\")", "cannot convert String to bool");
    fails("const a = bool(1.5)", "cannot convert f64 to bool");
    fails("const a = i32(true, false)", "converts one value");
}

// ---- built-in functions

#[test]
fn math_functions() {
    assert_eq!(type_of("const a = Math.abs(-3)", "a"), "i32");
    assert_eq!(type_of("const a = Math.min(2, 7)", "a"), "i32");
    assert_eq!(type_of("const a = Math.max(2.5, 1.5)", "a"), "f64");
    assert_eq!(type_of("const a = Math.sqrt(16.0)", "a"), "f64");
    assert_eq!(type_of("const a = Math.pi / 2.0", "a"), "f64");
    assert_eq!(type_of("i64 x = 5\nconst a = Math.max(x, 1)", "a"), "i64");
}

#[test]
fn math_arguments_must_agree() {
    fails("const a = Math.max(1, 2.5)", "never an integer");
    fails(
        "i32 x = 1\ni64 y = 2\nconst a = Math.max(x, y)",
        "must have the same type",
    );
    fails("const a = Math.sqrt(16)", "write 16.0");
    fails("const a = Math.abs(\"x\")", "needs a number");
}

#[test]
fn calls_check_the_argument_count() {
    fails(
        "const a = Math.min(1)",
        "Math.min takes 2 arguments, but 1 given",
    );
    fails(
        "const a = Math.sqrt(1.0, 2.0)",
        "Math.sqrt takes 1 argument, but 2 given",
    );
    fails(
        "const a = String.join()",
        "String.join takes at least 1 argument",
    );
}

#[test]
fn string_functions_and_methods() {
    assert_eq!(
        type_of("const a = String.join(\"a\", \"b\", \"c\")", "a"),
        "String"
    );
    assert_eq!(type_of("const a = String.format(\"%d\", 1)", "a"), "String");
    assert_eq!(type_of("const a = $(\"%d\", 1)", "a"), "String");
    assert_eq!(type_of("const a = \"x\".upper()", "a"), "String");
    assert_eq!(type_of("const a = \"x\".len()", "a"), "usize");
    assert_eq!(type_of("const a = \"x\".contains(\"y\")", "a"), "bool");
    assert_eq!(
        type_of("const a = \"a,b\".split(\",\")", "a"),
        "List<String>"
    );
    assert_eq!(type_of("const a = \"hello\".get(0)", "a"), "char");
    assert_eq!(type_of("const a = \"hello\".slice(1, 4)", "a"), "String");
    assert_eq!(type_of("const a = \"hello\".index_of(\"l\")", "a"), "usize");
}

#[test]
fn string_method_arguments_are_checked() {
    fails("const a = \"x\".contains(1)", "i32 is not String");
    fails("const a = \"x\".slice(1)", "slice takes 2 arguments");
    fails(
        "const a = \"x\".nothing()",
        "String has no method `nothing`",
    );
    fails("const n = 5\nconst a = n.len()", "i32 has no method `len`");
}

#[test]
fn a_variable_is_not_a_function() {
    fails("const n = 1\nn()", "`n` is a i32, not a function");
}

#[test]
fn wrong_type_names_are_reported() {
    fails("Optional o = 1", "`Optional` takes 1 type argument");
    fails("Unknown u = 1", "unknown type `Unknown`");
}

#[test]
fn type_arguments_are_checked_where_a_type_is_written() {
    fails("List l = []", "`List` takes one type");
    fails("u8<i32> x = 1", "takes no type arguments");
    fails("Tuple<i32> t = (1, 2)", "Tuple<i32, i32> is not Tuple<i32>");
    fails("const m = Map<String>()", "`Map` takes two types");
    fails("const s = Set<i32, i32>()", "`Set` takes one type");
}

#[test]
fn one_mistake_is_reported_once() {
    // the broken value has the error type, which fits everywhere
    let found = messages("const a = missing\nconst b = a + 1\nu8 c = b");
    assert_eq!(found.len(), 1, "{found:?}");
}

// ---- format strings

#[test]
fn the_values_of_a_format_match_its_places() {
    ok(r#"const s = String.format("hello %s, you are %d", "ann", 31)"#);
    ok(r#"const s = $("%s costs %.2f", "tea", 3.5)"#);
    ok(r#"const s = String.format("[%5d] [%-5d] [%05d]", 42, 42, 42)"#);
    ok(r#"const s = String.format("%x %X %o", 255, 255, 8)"#);
    ok(r#"const s = String.format("%c = %s%%", 'x', "ok")"#);
    ok(r#"const s = String.format("%*d|%.*f", 5, 42, 2, 3.14159)"#);
    ok(r#"const s = String.format("no places at all")"#);
    assert_eq!(
        type_of(r#"const s = String.format("%d", 1)"#, "s"),
        "String"
    );
}

#[test]
fn a_place_takes_the_kind_of_value_its_letter_says() {
    fails(
        r#"String.format("%d", "a")"#,
        "`%d` needs a signed integer, but this is String",
    );
    fails(
        r#"String.format("%s", 1)"#,
        "`%s` needs a String, but this is i32",
    );
    fails(r#"String.format("%c", "a")"#, "`%c` needs a char");
    fails(
        r#"String.format("%f", true)"#,
        "`%f` needs a decimal number",
    );
    // a literal is asked to be the type of its place, and a decimal is never an integer
    fails(r#"String.format("%d", 2.5)"#, "never an integer");
    // a decimal that is not a literal gets the message of the place
    fails(
        "let f = 2.5
const s = String.format(\"%d\", f)",
        "`%d` needs a signed integer, but this is f64",
    );
}

#[test]
fn a_whole_number_is_never_given_to_a_decimal_place() {
    // there is no silent conversion in a format either
    fails(r#"String.format("%f", 3)"#, "never a float");
    ok(r#"String.format("%f", 3.0)"#);
}

#[test]
fn signed_and_unsigned_places_are_told_apart() {
    ok("u8 b = 1\nconst s = String.format(\"%u\", b)");
    fails(
        "u8 b = 1\nconst s = String.format(\"%d\", b)",
        "`%d` needs a signed integer, but this is u8",
    );
    fails(
        "i8 b = 1\nconst s = String.format(\"%u\", b)",
        "`%u` needs an unsigned integer, but this is i8",
    );
    // %x and %o take either
    ok("u8 a = 1\ni8 b = 1\nconst s = String.format(\"%x %o\", a, b)");
    // a literal takes the type of its place
    ok(r#"const s = String.format("%u", 5)"#);
    fails(
        r#"const s = String.format("%u", -5)"#,
        "-5 does not fit in u32",
    );
}

#[test]
fn a_star_takes_a_whole_number() {
    fails(r#"String.format("%*d", 2.5, 1)"#, "never an integer");
    fails(
        "let f = 2.5
const s = String.format(\"%*d\", f, 1)",
        "the `*` in `%*d` needs a whole number, but this is f64",
    );
    ok("u8 width = 4\nconst s = String.format(\"%*d\", width, 1)");
}

#[test]
fn the_number_of_values_must_match() {
    fails(
        r#"String.format("%s and %s", "a")"#,
        "the format has 2 values but 1 was given",
    );
    fails(
        r#"String.format("%d", 1, 2)"#,
        "the format has 1 value but 2 were given",
    );
    fails(
        r#"String.format("hello", 1)"#,
        "the format has 0 values but 1 was given",
    );
    // a * is a value of its own
    fails(
        r#"String.format("%*d", 1)"#,
        "the format has 2 values but 1 was given",
    );
}

#[test]
fn a_format_that_is_not_a_format() {
    fails(r#"String.format("%q", 1)"#, "`%q` is not a format");
    fails(r#"String.format("100%")"#, "lone `%`");
    fails(r#"$("%5", 1)"#, "ends inside");
}

#[test]
fn a_format_that_is_not_a_literal_cannot_be_checked() {
    ok("const f = \"%d\"\nconst s = String.format(f, \"anything\")");
    fails("const s = String.format(1)", "i32 is not String");
    fails("const s = String.format()", "takes at least 1 argument");
}

#[test]
fn a_pointer_place_is_not_checked_yet() {
    fails(
        r#"String.format("%p", 1)"#,
        "takes a pointer, which is not checked yet",
    );
}

#[test]
fn a_value_that_is_nothing_cannot_be_formatted() {
    fails(
        r#"String.format("%s", print(1))"#,
        "`%s` needs a String, but this is nothing",
    );
}

// ---- collections (02-data-structures)

#[test]
fn the_type_of_a_collection_comes_from_its_values_or_its_type_arguments() {
    assert_eq!(type_of("const a = {1, 2, 3}", "a"), "Array<i32>");
    assert_eq!(type_of("const a = Set(1, 2, 3)", "a"), "Set<i32>");
    assert_eq!(
        type_of("const a = Map(\"a\", 1.5)", "a"),
        "Map<String, f64>"
    );
    assert_eq!(type_of("const a = List<u8>()", "a"), "List<u8>");
    assert_eq!(
        type_of("const a = Map<i32, String>()", "a"),
        "Map<i32, String>"
    );
    assert_eq!(
        type_of("const a = Tuple(1, 2.5, true)", "a"),
        "Tuple<i32, f64, bool>"
    );
    assert_eq!(type_of("const a = Tuple(1)", "a"), "Tuple<i32>");
    assert_eq!(
        type_of("const a = [{1, 2}, {3, 4}]", "a"),
        "List<Array<i32>>"
    );
    // a type written on the left decides the type of the literals on the right
    assert_eq!(type_of("Set<u8> a = Set(1, 2)", "a"), "Set<u8>");
    assert_eq!(type_of("List<f64> a = List(1.5)", "a"), "List<f64>");
}

#[test]
fn a_constructor_with_nothing_to_decide_its_type_is_an_error() {
    fails("const a = List()", "no values to decide its type");
    fails("const a = Set()", "no values to decide its type");
    fails("const a = Map()", "no values to decide its types");
}

#[test]
fn the_values_of_a_collection_have_one_type() {
    fails(
        "const a = Set(1, 2.5)",
        "this is a decimal and the others are whole numbers",
    );
    fails(
        "const a = Map(\"a\", 1, \"b\", 2.5)",
        "the others are whole numbers",
    );
    fails(
        "const a = Map(\"a\", 1, 2, 3)",
        "all the items must have the same type",
    );
    fails(
        "const a = List<i32>(1, 2.5)",
        "decimal literal is never an integer",
    );
}

#[test]
fn a_map_takes_pairs() {
    fails("const a = Map(\"a\", 1, \"b\")", "Map takes pairs");
}

#[test]
fn a_map_key_is_a_primitive_or_a_string() {
    ok("const a = Map<i32, String>()");
    ok("const a = Map<char, List<i32>>()");
    fails(
        "const a = Map<List<i32>, String>()",
        "a Map key is a primitive value or a String, not List<i32>",
    );
    fails(
        "const a = Map({1}, 2)",
        "a Map key is a primitive value or a String, not Array<i32>",
    );
}

#[test]
fn a_tuple_has_a_type_for_each_slot() {
    fails("Tuple<i32, f64> t = (1, 2)", "whole number");
    fails(
        "const t = Tuple<i32, f64>(1)",
        "has 2 slots, but 1 values were given",
    );
}

#[test]
fn the_methods_of_a_list() {
    let list = "const xs = [1, 2, 3]\n";
    ok(&format!(
        "{list}xs.push(4)\nxs.unshift(0)\nxs.insert(1, 9)\nxs.set(0, 5)\nxs.reverse()"
    ));
    assert_eq!(type_of(&format!("{list}const a = xs.pop()"), "a"), "i32");
    assert_eq!(type_of(&format!("{list}const a = xs.shift()"), "a"), "i32");
    assert_eq!(type_of(&format!("{list}const a = xs.get(0)"), "a"), "i32");
    assert_eq!(
        type_of(&format!("{list}const a = xs.remove(0)"), "a"),
        "i32"
    );
    assert_eq!(type_of(&format!("{list}const a = xs.len()"), "a"), "usize");
    assert_eq!(
        type_of(&format!("{list}const a = xs.contains(2)"), "a"),
        "bool"
    );
    assert_eq!(
        type_of(&format!("{list}const a = xs.clone()"), "a"),
        "List<i32>"
    );
    fails(
        &format!("{list}xs.push(2.5)"),
        "decimal literal is never an integer",
    );
    fails(&format!("{list}xs.push(\"a\")"), "String is not i32");
    fails(
        &format!("{list}xs.push()"),
        "push takes 1 argument, but 0 given",
    );
    fails(
        &format!("{list}xs.nothing()"),
        "List<i32> has no method `nothing`",
    );
}

#[test]
fn an_array_cannot_grow() {
    let array = "const xs = {1, 2, 3}\n";
    ok(&format!("{array}xs.set(0, 9)"));
    fails(
        &format!("{array}xs.push(4)"),
        "an Array has a fixed size, use a List",
    );
    fails(&format!("{array}xs.remove(0)"), "an Array has a fixed size");
}

#[test]
fn the_methods_of_a_set_and_a_map() {
    assert_eq!(
        type_of("const s = Set(1)\nconst a = s.contains(1)", "a"),
        "bool"
    );
    ok("const s = Set(1)\ns.add(2)\ns.remove(1)");
    let map = "const m = Map(\"a\", 1)\n";
    ok(&format!("{map}m.set(\"b\", 2)\nm.remove(\"a\")"));
    assert_eq!(type_of(&format!("{map}const a = m.get(\"a\")"), "a"), "i32");
    assert_eq!(
        type_of(&format!("{map}const a = m.has(\"a\")"), "a"),
        "bool"
    );
    assert_eq!(
        type_of(&format!("{map}const a = m.keys()"), "a"),
        "Array<String>"
    );
    assert_eq!(
        type_of(&format!("{map}const a = m.values()"), "a"),
        "Array<i32>"
    );
    fails(&format!("{map}m.get(1)"), "i32 is not String");
    fails(
        &format!("{map}m.set(\"a\", 2.5)"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{map}m.contains(\"a\")"),
        "Map<String, i32> has no method `contains`",
    );
}

#[test]
fn a_tuple_slot_is_chosen_by_a_literal_or_a_const_made_from_one() {
    let tuple = "const pair = (1, 2.5)\n";
    assert_eq!(
        type_of(&format!("{tuple}const a = pair.get(0)"), "a"),
        "i32"
    );
    assert_eq!(
        type_of(&format!("{tuple}const a = pair.get(1)"), "a"),
        "f64"
    );
    assert_eq!(
        type_of(
            &format!("{tuple}const slot = 1\nconst a = pair.get(slot)"),
            "a"
        ),
        "f64"
    );
    ok(&format!("{tuple}pair.set(0, 9)\npair.set(1, 7.5)"));
    fails(
        &format!("{tuple}pair.set(1, 7)"),
        "whole-number literal is never a float",
    );
    fails(
        &format!("{tuple}pair.set(2, 1)"),
        "a Tuple of 2 has no slot 2",
    );
    fails(
        &format!("{tuple}const a = pair.get(5)"),
        "a Tuple of 2 has no slot 5",
    );
    fails(
        &format!("{tuple}let which = 0\nconst a = pair.get(which)"),
        "must be a literal or a const made from one",
    );
    fails(
        &format!("{tuple}const a = pair.get(0, 1)"),
        "get takes 1 argument",
    );
}

#[test]
fn a_const_made_from_a_literal_remembers_its_value() {
    let source = "const slot = 1\nconst other = slot\nlet moving = 1";
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);
    let analysis = analyze(&parsed.program);
    let one = Some(crate::sema::Constant::Int {
        magnitude: 1,
        negative: false,
    });
    assert_eq!(analysis.symbol_named("slot").unwrap().constant, one);
    assert_eq!(analysis.symbol_named("other").unwrap().constant, one);
    assert_eq!(analysis.symbol_named("moving").unwrap().constant, None);
}

#[test]
fn every_collection_can_be_cloned_and_compared_by_identity() {
    ok("const a = [1]\nconst b = a.clone()\nconst same = a == b");
    ok("const a = {1}\nconst same = a == {1}");
    ok("const a = Set(1)\nconst same = a == a.clone()");
    ok("const a = Map(\"k\", 1)\nconst same = a != a.clone()");
    ok("const a = (1, 2.5)\nconst b = a.clone()");
    ok("const a = \"hi\"\nconst b = a.clone()");
    fails(
        "const a = [1]\nconst b = a.equals(a)",
        "List<i32> has no method `equals`",
    );
    fails(
        "const a = [1]\nconst b = a < a",
        "compares numbers and characters",
    );
    fails(
        "const a = [1]\nconst b = a == {1}",
        "List<i32> and Array<i32>",
    );
}

#[test]
fn a_for_loop_names_what_it_goes_over() {
    ok("const xs = [1, 2]\nfor (xs) |x| { const y = x + 1 }");
    ok("const xs = [1, 2]\nfor (xs) |x, at| { const y = x + 1\nconst z = at + 1 }");
    ok("const s = Set(\"a\")\nfor (s) |v| { const n = v.len() }");
    ok("const m = Map(\"a\", 1)\nfor (m) |k, v| { const n = k.len() }");
    ok("for (\"raga\") |ch, at| { const c = ch == 'r' }");
    fails(
        "const xs = [1]\nfor (xs) |x| { x = 2 }",
        "`x` is const and cannot be assigned",
    );
    fails("const xs = [1]\nfor (xs) |x| { }\nprint(x)", "out of scope");
    fails(
        "const xs = [1]\nfor (xs) |x| { const n = x.len() }",
        "has no method",
    );
    fails(
        "const s = Set(1)\nfor (s) |a, b| { }",
        "a Set loop has one name",
    );
    fails(
        "const m = Map(\"a\", 1)\nfor (m) |k| { }",
        "a Map loop has two names",
    );
    fails("for ((1, 2.5)) |slot| { }", "a Tuple cannot be looped over");
    fails("for (5) |n| { }", "i32 cannot be looped over");
}

#[test]
fn the_head_of_a_form_decides_what_its_arguments_and_block_mean() {
    fails(
        "for (1, 2) |x| { }",
        "`for` takes one collection, but 2 were given",
    );
    fails("const xs = [1]\nfor (xs)", "`for` needs a block");
    fails("const xs = [1]\nfor (xs) { }", "this loop has one name");
    // a call that is not a form takes the block after its arguments only once functions are checked
    fails("print(1) { }", "`print` takes no block after its arguments");
}

// ---- branches and loops (03-control-flow)

#[test]
fn a_condition_is_a_bool() {
    ok("const a = 1\nif (a > 0) { }\nwhile (a < 0) { }");
    fails(
        "const a = 1\nif (a) { }",
        "the condition of `if` must be a bool, but this is i32",
    );
    fails(
        "const a = \"x\"\nwhile (a) { }",
        "must be a bool, but this is String",
    );
    fails("if (1, 2) { }", "takes one condition in parentheses");
    fails("if () { }", "takes one condition in parentheses");
    fails(
        "const a = true\nif (a) { } elif (1) { }",
        "the condition of `elif` must be a bool",
    );
}

#[test]
fn a_chain_picks_its_order() {
    ok("const a = true\nif (a) { } elif (a) { } elif (a) { } else { }");
    ok("const xs = [1]\nfor (xs) |x| { } else { }");
    ok("while (true) { break } else { }");
    fails("else { }", "`else` without a matching `if`");
    fails("elif (true) { }", "`elif` without a matching `if`");
    fails(
        "loop { break } else { }",
        "a loop with no condition only ends with break",
    );
    fails("if (true) { } else { } else { }", "a chain has one `else`");
    fails(
        "if (true) { } else { } elif (true) { }",
        "`elif` cannot come after `else`",
    );
    fails(
        "const xs = [1]\nfor (xs) |x| { } elif (true) { }",
        "`elif` continues an `if`",
    );
    fails("switch (1) { } else { }", "`else` cannot follow `switch`");
}

#[test]
fn each_block_has_a_scope_of_its_own() {
    ok("if (true) { const inner = 1 }\nconst inner = 2");
    fails(
        "if (true) { const inner = 1 }\nprint(inner)",
        "out of scope",
    );
    fails(
        "if (true) { } else { const e = 1 }\nprint(e)",
        "out of scope",
    );
    ok("let total = 0\nwhile (total < 3) { total = total + 1 }");
}

#[test]
fn a_block_is_needed_and_takes_no_names() {
    fails("if (true)", "`if` needs a block");
    fails("if (true) |x| { }", "`if` takes no names between | |");
    fails("loop (true) { break }", "`loop` takes no condition");
}

#[test]
fn break_and_continue_belong_to_loops() {
    ok("loop { break }");
    ok("while (true) { if (true) { continue } }");
    ok("const xs = [1]\nfor (xs) |x| { if (x > 0) { break } }");
    fails("break", "`break` outside a loop");
    fails("if (true) { continue }", "`continue` outside a loop");
    // the else of a loop is outside the loop
    fails(
        "const xs = [1]\nfor (xs) |x| { } else { break }",
        "`break` outside a loop",
    );
}

#[test]
fn break_and_continue_can_leave_more_loops() {
    let nested = "const xs = [1]\nfor (xs) |a| {\n  for (xs) |b| {\n";
    ok(&format!("{nested}    break 1\n  }}\n}}"));
    ok(&format!("{nested}    continue 1\n  }}\n}}"));
    ok(&format!("{nested}    break 0\n  }}\n}}"));
    fails(
        &format!("{nested}    break 2\n  }}\n}}"),
        "break 2 leaves 3 loops, but only 2 loops surround it",
    );
    fails(
        &format!("{nested}    continue 5\n  }}\n}}"),
        "continue 5 leaves 6 loops",
    );
    fails(
        &format!("{nested}    break -1\n  }}\n}}"),
        "cannot be negative",
    );
    fails(
        &format!("{nested}    let n = 1\n    break n\n  }}\n}}"),
        "a whole number written in the code",
    );
    fails(
        "loop { break 1 }",
        "break 1 leaves 2 loops, but only 1 loop surrounds it",
    );
}

#[test]
fn a_switch_takes_an_integer_or_a_char() {
    ok("const code = 404\nswitch (code) {\ncase (200) { }\ncase (404, 500) { }\ndefault { } }");
    ok("const l = 'e'\nswitch (l) {\ncase ('a', 'e') { }\ndefault { } }");
    ok("const code = 1\nswitch (code) {\ncase (1) { } }");
    ok("u8 b = 3\nswitch (b) {\ncase (1) { }\ncase (255) { } }");
    fails("switch (1.5) { }", "a float cannot be switched on");
    fails("switch (true) { }", "a bool cannot be switched on");
    fails(
        "const s = \"go\"\nswitch (s) {\ncase (\"go\") { } }",
        "a String cannot be switched on",
    );
    fails(
        "const xs = [1]\nswitch (xs) { }",
        "List<i32> cannot be switched on",
    );
}

#[test]
fn the_cases_of_a_switch_are_checked_against_its_subject() {
    fails(
        "const code = 1\nswitch (code) {\ncase (200) { }\ncase (200) { } }",
        "200 appears twice",
    );
    fails(
        "const code = 1\nswitch (code) {\ncase (1, 2, 1) { } }",
        "1 appears twice",
    );
    fails(
        "const code = 1\nswitch (code) {\ncase (\"x\") { } }",
        "String is not i32",
    );
    fails(
        "let tip = 1\nconst code = 1\nswitch (code) {\ncase (tip) { } }",
        "a case value must be a literal or a const made from one, `tip` is a variable",
    );
    fails(
        "const code = 1\nswitch (code) {\ncase (code + 1) { } }",
        "a case value must be a literal",
    );
    fails(
        "u8 b = 1\nswitch (b) {\ncase (300) { } }",
        "300 does not fit in u8",
    );
    fails(
        "const code = 1\nswitch (code) {\ncase () { } }",
        "`case` needs a value",
    );
    fails(
        "const code = 1\nswitch (code) { default { }\ncase (1) { } }",
        "`default` must come last",
    );
    fails(
        "const code = 1\nswitch (code) { default { }\ndefault { } }",
        "a switch has one `default`",
    );
    fails(
        "const code = 1\nswitch (code) {\ndefault (1) { }\n}",
        "`default` takes no value",
    );
    fails(
        "const code = 1\nswitch (code) {\nprint(1)\n}",
        "only `case` and `default`",
    );
    fails("case (1) { }", "`case` belongs inside a switch or a match");
}

#[test]
fn a_case_may_be_a_const_made_from_a_literal() {
    ok("const ok = 200\nconst code = 1\nswitch (code) {\ncase (ok) { } }");
    // the subject itself is a const made from a literal too
    ok("const code = 1\nswitch (code) {\ncase (code) { } }");
    fails(
        "const ok = 200\nconst code = 1\nswitch (code) {\ncase (ok) { }\ncase (200) { } }",
        "200 appears twice",
    );
    ok("const code = -1\nswitch (code) {\ncase (-1) { }\ncase (1) { } }");
    fails(
        "const code = 1\nswitch (code) {\ncase (-1) { }\ncase (-1) { } }",
        "-1 appears twice",
    );
}

#[test]
fn a_match_is_for_enums_and_tagged_unions() {
    fails(
        "match (404) { }",
        "match needs an enum or a tagged union, but this is i32: write switch",
    );
    fails(
        "match (true) { }",
        "match needs an enum or a tagged union, but this is bool",
    );
}

#[test]
fn a_range_gives_the_numbers_of_a_for() {
    ok("for (range(0, 3)) |i| { const n = i + 1 }");
    ok("for (range(0, 10, 3)) |i| { }");
    ok("for (range(3, 0, -1)) |i| { }");
    ok("for (range(u8(0), u8(3))) |i| { u8 b = i }");
    ok("const xs = [\"a\"]\nfor (range(0, xs.len())) |at| { const s = xs.get(at) }");
    fails("for (range(0, 3)) |i| { u8 b = i }", "i32 is not u8");
    fails(
        "const xs = [\"a\"]\nfor (range(0, xs.len())) |at| { i32 n = at }",
        "usize is not i32",
    );
    fails("for (range(0)) |i| { }", "range takes 2 or 3 arguments");
    fails(
        "for (range(0, 1, 2, 3)) |i| { }",
        "range takes 2 or 3 arguments",
    );
    fails(
        "for (range(0.5, 2.5)) |i| { }",
        "range counts whole numbers",
    );
    fails("for (range(0, 3)) |i, j| { }", "a range loop has one name");
    fails(
        "const r = range(0, 3)",
        "range gives the numbers to loop over",
    );
}

#[test]
fn a_loop_variable_is_fixed_inside_the_loop() {
    fails(
        "const xs = [1]\nfor (xs) |x| { x = 0 }",
        "`x` is const and cannot be assigned",
    );
}

#[test]
fn a_value_of_a_branch_is_not_a_value_for_a_name() {
    let found = messages("const bad = if (true) { 1 } else { 2 }");
    assert_eq!(found.len(), 1, "{found:?}");
}

// ---- functions (04-functions)

const ADD: &str = "const add = function(i32 a, i32 b) i32 { return a + b }\n";

#[test]
fn a_function_has_a_type_made_of_its_parameters_and_what_it_returns() {
    assert_eq!(type_of(ADD, "add"), "function(i32, i32) i32");
    assert_eq!(type_of("const f = function() { }", "f"), "function()");
    assert_eq!(
        type_of("const f = function<T>(T v) T { return v }", "f"),
        "function<T>(T) T"
    );
    assert_eq!(type_of(&format!("{ADD}const r = add(1, 2)"), "r"), "i32");
}

#[test]
fn the_return_type_is_written_or_worked_out_from_the_returns() {
    assert_eq!(
        type_of("const f = function(f64 x) { return x * x * x }", "f"),
        "function(f64) f64"
    );
    assert_eq!(
        type_of("const f = function() { return 0.5 }", "f"),
        "function() f64"
    );
    assert_eq!(
        type_of("const f = function(String s) { print(s) }", "f"),
        "function(String)"
    );
    ok("const f = function(i32 n) { if (n < 1) { return } print(n) }");
    fails(
        "const f = function(bool b) {\nif (b) { return 1 }\nreturn 2.5\n}",
        "the returns disagree",
    );
    fails(
        "const f = function() i32 { return 2.5 }",
        "decimal literal is never an integer",
    );
    fails(
        "const f = function() i32 { return \"x\" }",
        "String is not i32",
    );
}

#[test]
fn a_return_belongs_to_a_function_and_gives_what_it_promised() {
    fails("return 1", "`return` belongs to functions only");
    fails("if (true) { return }", "`return` belongs to functions only");
    fails(
        "const f = function() { return 1 }\nconst x = function() i32 { return }",
        "so `return` needs a value",
    );
    fails(
        "const f = function() { }\nconst g = function() { return 1 }\nconst h = function() { f()\nreturn }\nconst bad = function(i32 n) { return }\nconst i = function() i32 { }",
        "not every path returns a value",
    );
}

#[test]
fn a_function_with_a_return_type_returns_on_every_path() {
    ok("const f = function(i32 n) i32 { if (n > 0) { return 1 } else { return 2 } }");
    ok("const f = function(i32 n) i32 { if (n > 0) { return 1 }\nreturn 0 }");
    ok(
        "const f = function(List<i32> xs) i32 {\nfor (xs) |x| { if (x > 0) { return x } }\nreturn 0\n}",
    );
    ok(
        "const f = function(i32 n) i32 {\nswitch (n) {\ncase (1) { return 1 }\ndefault { return 2 }\n}\n}",
    );
    ok("const f = function() i32 { loop { } }");
    fails(
        "const f = function(i32 n) i32 { if (n > 0) { return 1 } }",
        "not every path returns a value",
    );
    fails(
        "const f = function(i32 n) i32 { if (n > 0) { return 1 } elif (n < 0) { return 2 } }",
        "not every path returns a value",
    );
    fails(
        "const f = function(i32 n) i32 {\nswitch (n) {\ncase (1) { return 1 }\n}\n}",
        "not every path returns a value",
    );
    fails(
        "const f = function() i32 { loop { break } }",
        "not every path returns a value",
    );
}

#[test]
fn a_parameter_is_fixed_inside_the_function() {
    ok("const f = function(i32 n) i32 { let local = n\nlocal = local * 2\nreturn local }");
    fails(
        "const f = function(i32 n) { n = 5 }",
        "`n` is a parameter and cannot be assigned",
    );
    fails(
        "const f = function(List<i32> xs) { xs = [1] }",
        "`xs` is a parameter and cannot be assigned",
    );
}

#[test]
fn parameters_have_an_order() {
    ok("const f = function(i32 a, i32 b = 2) { }");
    ok("const f = function(i32 a, i32 ...rest) { }");
    ok("const f = function(i32 a, i32 b = 1, function(i32) work) { }");
    fails(
        "const f = function(i32 a = 2, i32 b) { }",
        "a default must not come before a parameter without one",
    );
    fails(
        "const f = function(i32 ...a, i32 b) { }",
        "the variadic parameter must be last",
    );
    fails(
        "const f = function(i32 n = 1, i32 ...rest) { }",
        "cannot be mixed",
    );
    fails(
        "const f = function(function() work, i32 pause = 100) { }",
        "nothing may follow a callback",
    );
    fails(
        "const f = function(function() work, i32 ...rest) { }",
        "nothing may follow a callback",
    );
    fails(
        "const f = function(i32 a, i32 a) { }",
        "`a` is already a parameter",
    );
    fails(
        "const f = function(i32 a = 1.5) { }",
        "decimal literal is never an integer",
    );
}

#[test]
fn arguments_go_to_parameters_by_position_and_by_name() {
    let scale = "const scale = function(i32 value, i32 factor = 2) i32 { return value * factor }\n";
    ok(&format!(
        "{scale}scale(3, 2)\nscale(3)\nscale(value = 3, factor = 2)\nscale(factor = 4, value = 2)\nscale(3, factor = 4)"
    ));
    assert_eq!(type_of(&format!("{scale}const r = scale(3)"), "r"), "i32");
    fails(
        &format!("{scale}scale(factor = 4, 3)"),
        "a positional argument cannot follow a named one",
    );
    fails(
        &format!("{scale}scale(value = 3, times = 2)"),
        "scale has no parameter `times`",
    );
    fails(
        &format!("{scale}scale()"),
        "`value` has no default and was not given",
    );
    fails(
        &format!("{scale}scale(1.5, 2)"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{scale}scale(1, 2, 3)"),
        "scale takes 2 arguments, but more were given",
    );
    fails(
        &format!("{scale}scale(1, value = 2)"),
        "`value` is given twice",
    );
}

#[test]
fn extra_arguments_go_to_a_variadic_parameter() {
    let count =
        "const count = function(String label, i32 ...values) usize { return values.len() }\n";
    assert_eq!(
        type_of(&format!("{count}const n = count(\"none\")"), "n"),
        "usize"
    );
    ok(&format!("{count}count(\"three\", 1, 2, 3)"));
    fails(
        &format!("{count}count(\"x\", 1, 2.5)"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{count}count(\"x\", values = 1)"),
        "by position, not by name",
    );
    // inside the function the variadic name is an Array of the extra arguments
    assert_eq!(
        type_of("const f = function(i32 ...v) { const a = v }", "f"),
        "function(i32)"
    );
    ok(
        "const f = function(i32 fallback, i32 ...v) i32 { if (v.len() == 0) { return fallback }\nreturn v.get(0) }",
    );
}

#[test]
fn a_function_can_call_itself_when_it_says_what_it_returns() {
    ok("const f = function(u64 n) u64 { if (n <= 1) { return 1 }\nreturn n * f(n - 1) }");
    fails(
        "const f = function(u64 n) { if (n <= 1) { return 1 }\nreturn n * f(n - 1) }",
        "write the return type",
    );
    // two functions cannot call each other: a name is visible only below its declaration
    fails(
        "const a = function(u64 n) bool { return b(n) }\nconst b = function(u64 n) bool { return true }",
        "`b` is declared below",
    );
}

#[test]
fn a_function_is_a_value() {
    let shout = "const shout = function(String t, i32 n) String { return t }\nconst whisper = function(String t, i32 n) String { return t }\n";
    assert_eq!(
        type_of(&format!("{shout}const inferred = shout"), "inferred"),
        "function(String, i32) String"
    );
    ok(&format!(
        "{shout}function(String, i32) String current = shout\ncurrent = whisper\nconst same = current == whisper"
    ));
    ok(&format!(
        "{shout}const style = function(bool loud) function(String, i32) String {{ if (loud) {{ return shout }}\nreturn whisper }}\nconst speak = style(true)\nconst line = speak(\"hi\", 3)\nconst quiet = style(false)(\"hi\", 3)"
    ));
    fails(
        &format!(
            "{shout}const plus = function(i32 a, i32 b) i32 {{ return a }}\nfunction(String, i32) String current = shout\ncurrent = plus"
        ),
        "is not function(String, i32) String",
    );
    fails(
        &format!(
            "{shout}const plus = function(i32 a, i32 b) i32 {{ return a }}\nconst mixed = function(bool b) {{ if (b) {{ return plus }}\nreturn shout }}"
        ),
        "the returns disagree",
    );
    fails(
        &format!("{shout}const x = 5\nx(1)"),
        "`x` is a i32, not a function",
    );
}

#[test]
fn a_function_with_no_parameters_is_called_by_its_name() {
    let answer = "const answer = function() i32 { return 42 }\nconst double = function(i32 n) i32 { return n * 2 }\n";
    assert_eq!(type_of(&format!("{answer}const a = answer"), "a"), "i32");
    assert_eq!(
        type_of(&format!("{answer}const a = double answer"), "a"),
        "i32"
    );
    assert_eq!(type_of(&format!("{answer}const a = answer()"), "a"), "i32");
    ok(&format!("{answer}answer"));
    // where a function is wanted, the name is the function
    let timed = "const echo = function() { }\nconst run = function(function() work) { work() }\n";
    ok(&format!("{timed}run(echo)"));
    // a function with parameters is the function itself, not a call
    assert_eq!(
        type_of(&format!("{answer}const d = double"), "d"),
        "function(i32) i32"
    );
}

#[test]
fn a_call_without_parentheses_has_a_name_for_its_argument() {
    let double = "const double = function(i32 n) i32 { return n * 2 }\nconst base = 7\n";
    assert_eq!(
        type_of(&format!("{double}const d = double base"), "d"),
        "i32"
    );
    assert_eq!(
        type_of(&format!("{double}const d = double double base"), "d"),
        "i32"
    );
    assert_eq!(
        type_of(&format!("{double}const d = double base + 1"), "d"),
        "i32"
    );
    fails(&format!("{double}const d = double 5"), "write double(5)");
    // `double -base` is a subtraction, and a function cannot be subtracted from
    fails(
        &format!("{double}const d = double -base"),
        "both sides of `-`",
    );
    fails("u8 late", "`late` needs a value");
}

#[test]
fn a_callback_is_a_parameter_whose_type_is_a_function() {
    let apply = "const apply = function(i32 value, function(i32) i32 transform) i32 { return transform(value) }\nconst double = function(i32 n) i32 { return n * 2 }\n";
    ok(&format!(
        "{apply}apply(4, double)\napply(value = 4, transform = double)"
    ));
    assert_eq!(
        type_of(&format!("{apply}const r = apply(4, double)"), "r"),
        "i32"
    );
    fails(
        &format!("{apply}apply(4, 5)"),
        "i32 is not function(i32) i32",
    );
    fails(
        &format!("{apply}const other = function(String s) String {{ return s }}\napply(4, other)"),
        "function(String) String is not function(i32) i32",
    );
}

#[test]
fn the_last_argument_can_be_a_block() {
    let apply = "const apply = function(i32 value, function(i32) i32 transform) i32 { return transform(value) }\n";
    ok(&format!("{apply}const r = apply(4) |n| {{ return n * 3 }}"));
    assert_eq!(
        type_of(
            &format!("{apply}const r = apply(4) |n| {{ return n * 3 }}"),
            "r"
        ),
        "i32"
    );
    // the names of the block are the parameters of the callback, so they must match
    fails(
        &format!("{apply}apply(4) |a, b| {{ return a }}"),
        "the callback takes 1 parameter, but the block names 2",
    );
    fails(
        &format!("{apply}apply(4) |n| {{ return \"x\" }}"),
        "String is not i32",
    );
    // a return inside a block leaves the block, and its loops are its own
    ok(&format!(
        "{apply}apply(4) |n| {{\nfor (range(0, n)) |i| {{ if (i == 2) {{ return i }} }}\nreturn 0\n}}"
    ));
    fails(
        &format!("{apply}apply(4) |n| {{ break\nreturn 0 }}"),
        "`break` outside a loop",
    );
    // a callback with no parameters, and one that returns nothing
    ok(
        "const timed = function(function() work) { work() }\ntimed() { print(1) }\ntimed { print(2) }",
    );
    ok(
        "const twice = function(i32 v, function(i32) action) { action(v)\naction(v) }\ntwice(5) |n| { print(n) }",
    );
    // a block can use the names around it, and change a let
    ok(
        "let running = 0\nconst repeat = function(i32 t, function(i32) action) { action(t) }\nrepeat(4) |pass| { running = running + pass }",
    );
    // the block fills the last parameter, which may come after defaults
    ok(
        "const attempts = function(i32 limit, i32 pause = 100, function(i32 number) work) { work(limit) }\nattempts(3) |number| { print(number) }\nattempts(3, pause = 50) |number| { print(number) }",
    );
}

#[test]
fn a_block_fills_the_last_parameter_and_nothing_else() {
    let attempt = "const attempt = function(i32 v, function(String r) failed, function(i32 r) done) { done(v) }\nconst report = function(String r) { print(r) }\n";
    ok(&format!(
        "{attempt}attempt(5, failed = report) |result| {{ print(result) }}\nattempt(-5, report) |result| {{ print(result) }}"
    ));
    reports(
        &format!("{attempt}attempt(5, report, report) |result| {{ print(result) }}"),
        "one argument too many",
    );
    fails(
        "const scale = function(i32 v, i32 f = 2) i32 { return v }\nscale(2) { 5 }",
        "the last parameter is i32, which cannot be written as a block",
    );
    reports(
        "const none = function() { }\nnone(1) { }",
        "takes 0 arguments, but more were given",
    );
    fails(
        "const f = function(i32 ...rest) { }\nf { }",
        "a variadic parameter cannot take a block",
    );
}

#[test]
fn an_array_can_be_written_as_the_block() {
    let tally = "const tally = function(String title, Array<i32> numbers) i32 { return 1 }\n";
    ok(&format!(
        "{tally}tally(\"scores\") {{ 4, 8, 15 }}\ntally(\"scores\", {{ 4, 8, 15 }})"
    ));
    ok(&format!(
        "{tally}const unit = 4\ntally(\"scores\") {{ unit, unit * 2 }}"
    ));
    fails(
        &format!("{tally}tally(\"scores\") |n| {{ 1, 2 }}"),
        "an Array block gives values",
    );
    fails(
        &format!("{tally}tally(\"scores\") {{ 1, 2.5 }}"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{tally}tally(\"scores\") {{ 1 }}\ntally(\"scores\", {{ 1 }}) {{ 1 }}"),
        "one argument too many",
    );
}

#[test]
fn a_sort_takes_a_callback() {
    let sorted = "const by_size = function(i32 a, i32 b) i32 { return a - b }\nconst ranking = [30, 10, 20]\n";
    ok(&format!("{sorted}ranking.sort(by_size)"));
    ok(&format!("{sorted}ranking.sort() |a, b| {{ return b - a }}"));
    fails(
        &format!("{sorted}ranking.sort() |a| {{ return a }}"),
        "takes 2 parameters, but the block names 1",
    );
    fails(
        &format!("{sorted}ranking.sort() |a, b| {{ return \"x\" }}"),
        "String is not i32",
    );
}

#[test]
fn a_function_declared_inside_another_uses_the_names_around_it() {
    let counter = "const make_counter = function(i32 start) function() i32 {\nlet count = start\nconst next = function() i32 {\ncount = count + 1\nreturn count\n}\nreturn next\n}\n";
    ok(counter);
    assert_eq!(
        type_of(
            &format!("{counter}const c = make_counter(10)\nconst a = c()"),
            "a"
        ),
        "i32"
    );
    // only names declared above are visible
    fails(
        "const early = function() { later() }\nconst later = function() { }",
        "`later` is declared below",
    );
    reports(
        "const bad = function() { ticks = ticks + 1 }",
        "`ticks` is not declared",
    );
    // a parameter or a const of the outer function is never changed
    fails(
        "const fixed = function(i32 n) function() { const g = function() { n = 5 }\nreturn g }",
        "`n` is a parameter and cannot be assigned",
    );
    ok(
        "const items = List<function(i32) i32>()\nfor (range(0, 3)) |step| { items.push(function(i32 v) i32 { return v + step }) }\nconst added = items.get(2)(10)",
    );
}

#[test]
fn a_generic_function_takes_its_types_from_its_arguments_or_from_the_call() {
    let id = "const identity = function<T>(T value) T { return value }\n";
    assert_eq!(type_of(&format!("{id}const a = identity(5)"), "a"), "i32");
    assert_eq!(
        type_of(&format!("{id}const a = identity(\"hi\")"), "a"),
        "String"
    );
    assert_eq!(
        type_of(&format!("{id}const a = identity<f64>(2.5)"), "a"),
        "f64"
    );
    fails(
        &format!("{id}const a = identity<i32>(2.5)"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{id}const a = identity<i32, i32>(2)"),
        "has 1 type parameter, but 2 type arguments were given",
    );
    let first = "const first_of = function<T>(Array<T> items) T { return items.get(0) }\n";
    assert_eq!(
        type_of(&format!("{first}const a = first_of({{1, 2, 3}})"), "a"),
        "i32"
    );
    assert_eq!(
        type_of(
            &format!("{first}const a = first_of<String>({{\"a\"}})"),
            "a"
        ),
        "String"
    );
    let pair = "const pair_of = function<A, B>(A first, B second) Tuple<A, B> { return (first, second) }\n";
    assert_eq!(
        type_of(&format!("{pair}const t = pair_of(1, 2.5)"), "t"),
        "Tuple<i32, f64>"
    );
}

#[test]
fn a_type_parameter_has_only_what_every_type_has() {
    ok("const pick = function<T>(T a, T b) bool { return a == b }");
    ok("const keep = function<T>(T a) List<T> { const xs = List<T>()\nxs.push(a)\nreturn xs }");
    fails(
        "const bad = function<T>(T a, T b) T { return a + b }",
        "`+` needs numbers, but this is T",
    );
    fails(
        "const bad = function<T>(T value) i32 { return value.size() }",
        "T is a type parameter and has no member `size`",
    );
    fails("const bad = function(T value) { }", "unknown type `T`");
}

#[test]
fn a_generic_callback_gets_its_types_from_the_other_arguments() {
    let larger = "const larger = function<T>(T a, T b, function(T, T) i32 compare) T { if (compare(a, b) > 0) { return a }\nreturn b }\n";
    assert_eq!(
        type_of(
            &format!("{larger}const r = larger(3, 7) |x, y| {{ return x - y }}"),
            "r"
        ),
        "i32"
    );
    assert_eq!(
        type_of(
            &format!(
                "{larger}const r = larger(\"ab\", \"abc\") |x, y| {{ return i32(x.len()) - i32(y.len()) }}"
            ),
            "r"
        ),
        "String"
    );
    let each = "const each = function<T>(List<T> items, function(T, usize) action) { }\n";
    ok(&format!(
        "{each}each([\"a\", \"b\"]) |item, position| {{ print(position) }}"
    ));
}

#[test]
fn a_generic_function_as_a_value_needs_its_types() {
    let generic = "const identity_of = function<T>(T value) T { return value }\nconst apply_twice = function(function(i32) i32 f, i32 n) i32 { return f(f(n)) }\n";
    assert_eq!(
        type_of(&format!("{generic}const g = identity_of<i32>"), "g"),
        "function(i32) i32"
    );
    ok(&format!(
        "{generic}apply_twice(identity_of<i32>, 5)\napply_twice(identity_of, 5)"
    ));
    ok(&format!(
        "{generic}const make = function() function(String) String {{ return identity_of<String> }}\nconst s = make()(\"x\")"
    ));
    fails(
        &format!("{generic}const bad = identity_of"),
        "is generic and has no single type",
    );
    fails(
        &format!("{generic}apply_twice(identity_of<String>, 5)"),
        "function(String) String is not function(i32) i32",
    );
    fails(
        &format!("{generic}const g = identity_of<i32, i32>"),
        "has 1 type parameter, but 2 type arguments were given",
    );
}

#[test]
fn a_function_type_is_checked_where_it_is_written() {
    ok("function(i32) i32 f = function(i32 n) i32 { return n }");
    fails(
        "function(i32) i32 f = function(String s) String { return s }",
        "is not function(i32) i32",
    );
    ok("const l = List<function(i32) i32>()");
    fails(
        "const l = List<function(i32) i32>()\nl.push(function(String s) String { return s })",
        "is not function(i32) i32",
    );
}

#[test]
fn a_call_with_a_block_that_is_not_a_function_is_reported() {
    fails("const x = 1\nx { }", "`x` is a i32, not a function");
    fails("print(1) { }", "`print` takes no block after its arguments");
}

#[test]
fn recursion_in_a_loop_and_a_switch_inside_a_function() {
    ok(
        "const f = function(List<i32> codes) i32 {\nlet ok = 0\nfor (codes) |status| {\nswitch (status) {\ncase (200) { ok = ok + 1 }\ncase (404) { continue }\ndefault { }\n}\n}\nreturn ok\n}",
    );
    ok(
        "const text = function(i32 status) String {\nswitch (status) {\ncase (200) { return \"ok\" }\ndefault { }\n}\nreturn \"unknown\"\n}",
    );
}

#[test]
fn a_name_with_a_block_can_be_the_one_argument_of_a_call() {
    let sum = "const sum = function(Array<i32> numbers) i32 { return 1 }\nconst double = function(i32 n) i32 { return n * 2 }\n";
    assert_eq!(
        type_of(&format!("{sum}const d = double sum {{ 1, 2, 3 }}"), "d"),
        "i32"
    );
    ok(&format!("{sum}double sum {{ 1, 2, 3 }}"));
    // the inner call is checked as a call: its block is the Array, and what it returns is the argument of the outer one
    fails(
        &format!("{sum}const d = double sum {{ 1, 2.5 }}"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{sum}const base = 7\nconst d = double base {{ }}"),
        "`base` is a i32, not a function",
    );
}

// ---- structs, enums, unions (05-structs-enums-unions)

const POINT: &str = "const Point = struct {\n  f64 x\n  f64 y\n}\n";

#[test]
fn a_struct_value_is_made_by_naming_its_fields() {
    assert_eq!(
        type_of(&format!("{POINT}const p = Point(x = 1.5, y = 2.5)"), "p"),
        "Point"
    );
    ok(&format!(
        "{POINT}const p = Point(y = 2.5, x = 1.5)\nPoint q = Point(x = 3.0, y = 4.0)"
    ));
    fails(
        &format!("{POINT}const p = Point(x = 1.5)"),
        "y was not given, and has no default",
    );
    fails(
        &format!("{POINT}const p = Point()"),
        "x and y were not given",
    );
    reports(
        &format!("{POINT}const p = Point(z = 1.0)"),
        "Point has no field `z`",
    );
    reports(
        &format!("{POINT}const p = Point(x = 1, y = 2)"),
        "write 1.0",
    );
    fails(
        &format!("{POINT}const p = Point(x = 1.0, x = 2.0, y = 1.0)"),
        "`x` is given twice",
    );
    fails(
        &format!("{POINT}const p = Piont(x = 1.0)"),
        "`Piont` is not declared",
    );
    reports(
        &format!("{POINT}const p = Point(1.5, 2.5)"),
        "is created with names",
    );
}

#[test]
fn a_struct_with_one_field_may_be_made_by_position() {
    ok("const UserId = struct { i32 value }\nconst a = UserId(7)\nconst b = UserId(value = 7)");
    fails(
        "const UserId = struct { i32 value }\nconst a = UserId(7.5)",
        "decimal literal is never an integer",
    );
}

#[test]
fn a_field_with_a_default_may_be_left_out() {
    let window = "const Window = struct {\n  String title = \"untitled\"\n  i32 width = 800\n  List<String> tags = []\n}\n";
    ok(&format!(
        "{window}const a = Window()\nconst b = Window(title = \"x\", width = 1)"
    ));
    fails(
        "const Bad = struct { i32 width = \"wide\" }",
        "String is not i32",
    );
    fails(
        "const Bad = struct { u8 level = 300 }",
        "300 does not fit in u8",
    );
    fails(
        "const S = struct { i32 a; i32 a }",
        "`a` is already a member of S",
    );
    fails(
        "const bad = struct { f64 w, f64 h }",
        "separate the members",
    );
}

#[test]
fn a_field_is_read_and_changed_with_a_dot() {
    let p = format!("{POINT}const p = Point(x = 1.5, y = 2.5)\n");
    assert_eq!(type_of(&format!("{p}const px = p.x"), "px"), "f64");
    ok(&format!("{p}p.x = 9.0\np.y += 1.0"));
    fails(&format!("{p}p.x = 1"), "write 1.0");
    fails(&format!("{p}p.z = 1.0"), "Point has no field `z`");
    fails(&format!("{p}const bad = p.z"), "Point has no field `z`");
    let line = "const Line = struct { Point from; Point to }\n";
    assert_eq!(
        type_of(
            &format!("{p}{line}const l = Line(from = p, to = p)\nconst lx = l.to.x"),
            "lx"
        ),
        "f64"
    );
}

#[test]
fn a_struct_is_compared_by_identity_and_cloned_with_changes() {
    let p = format!("{POINT}const p = Point(x = 1.0, y = 2.0)\n");
    ok(&format!(
        "{p}const same = p == p\nconst c = p.clone()\nconst d = p.clone(x = 9.0)\nconst e = p.clone(x = 0.0, y = 0.0)"
    ));
    assert_eq!(
        type_of(&format!("{p}const d = p.clone(x = 9.0)"), "d"),
        "Point"
    );
    fails(
        &format!("{p}const d = p.clone(z = 1.0)"),
        "Point has no field `z`",
    );
    fails(&format!("{p}const d = p.clone(x = 1)"), "write 1.0");
    fails(
        &format!("{p}const d = p.clone(1.0)"),
        "clone takes the names of the fields to change",
    );
    fails(
        &format!("{p}const bad = p.equals(p)"),
        "Point has no method `equals`",
    );
}

#[test]
fn a_method_is_a_function_in_the_body_and_this_is_the_value() {
    let counter = "const Counter = struct {\n  i32 value\n  const increment = function() { this.value = this.value + 1 }\n  const add = function(i32 n) { this.value = this.value + n }\n  const get = function() i32 { return this.value }\n}\nlet c = Counter(value = 0)\n";
    ok(&format!(
        "{counter}c.increment()\nc.add(5)\nconst now = c.get()"
    ));
    assert_eq!(
        type_of(&format!("{counter}const now = c.get()"), "now"),
        "i32"
    );
    fails(
        &format!("{counter}c.add(2.5)"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{counter}c.nothing()"),
        "Counter has no method `nothing`",
    );
    fails(
        &format!("{counter}const x = c.increment()"),
        "this gives no value",
    );
    // a method sees its own type, and returns a new value of it
    ok(
        "const Vec = struct {\n f64 x\n const scaled = function(f64 k) Vec { return Vec(x = this.x * k) }\n}\nconst v = Vec(x = 1.0).scaled(2.0).scaled(0.5)",
    );
    fails(
        "const S = struct {\n i32 a\n const a = function() { }\n}",
        "`a` is already a member of S",
    );
}

#[test]
fn a_type_may_name_itself_through_a_collection_or_an_optional() {
    ok(
        "const Tree = struct { i32 value; List<Tree> children }\nconst leaf = Tree(value = 3, children = [])",
    );
    ok(
        "const Link = struct { i32 value; Optional<Link> next }\nconst l = Link(value = 1, next = Optional.None)",
    );
    fails(
        "const Loop = struct { i32 value; Loop next }",
        "could never be created",
    );
    fails(
        "const First = struct { Second other }\nconst Second = struct { First other }",
        "`Second` is declared below",
    );
}

#[test]
fn an_enum_has_numbered_members() {
    let color = "const Color = enum {\n const Red = 0\n const Green = 1\n const Blue = 2\n}\n";
    assert_eq!(
        type_of(&format!("{color}const c = Color.Green"), "c"),
        "Color"
    );
    ok(&format!(
        "{color}Color c = Color.Blue\nconst n = i32(Color.Green)\nconst d = Color(2)\nconst e = Color()\nconst same = Color.Red == Color.Red"
    ));
    fails(
        &format!("{color}const c = Color.Purple"),
        "`Purple` is not in Color",
    );
    fails(
        "const bad = enum { const Red = 0; const Green = 0 }",
        "Red and Green have the same value",
    );
    fails(
        "const bad = enum { const Red = 0, const Green = 1 }",
        "separate the members",
    );
    reports(
        "const bad = enum { Red, Green }",
        "a bare name is not a member",
    );
    fails(
        "const bad = enum { const Red = \"x\" }",
        "a number written in the code",
    );
    fails(
        &format!("{color}const n = Color(\"x\")"),
        "made from a whole number",
    );
    ok(
        "const Light = enum {\n const Off = 0\n const On = 1\n const is_on = function() bool {\n match (this) {\n case (On) { return true }\n default { return false }\n }\n }\n}\nconst l = Light.On.is_on()",
    );
}

#[test]
fn an_untagged_union_is_made_from_one_of_its_fields() {
    let bits = "const Bits32 = union {\n i32 i = 0\n f32 f\n}\n";
    ok(&format!(
        "{bits}let n = Bits32(i = 0)\nconst a = Bits32()\nconst b = Bits32(f = 1.0)\nn.f = 1.0\nconst x = n.i"
    ));
    assert_eq!(
        type_of(&format!("{bits}let n = Bits32(i = 0)\nconst x = n.f"), "x"),
        "f32"
    );
    fails(
        &format!("{bits}const a = Bits32(i = 0, f = 1.0)"),
        "a union holds one view at a time",
    );
    fails(
        "const U = union { i32 a; f32 b }\nconst u = U()",
        "needs a view to start with",
    );
    ok("const CValue = union { i64 number; ptr<u8> address }");
    fails(
        &format!("{bits}let n = Bits32(i = 0)\nmatch (n) {{ default {{ }} }}"),
        "an untagged union has no tag",
    );
}

#[test]
fn a_tagged_union_is_made_through_a_variant() {
    let shape = "const Shape = union {\n const Circle = struct { f64 radius }\n const Rect = struct { f64 w; f64 h }\n const Empty = struct { }\n}\n";
    assert_eq!(
        type_of(&format!("{shape}const s = Shape.Circle(radius = 2.0)"), "s"),
        "Shape"
    );
    ok(&format!(
        "{shape}const a = Shape.Circle(2.0)\nconst b = Shape.Rect(w = 3.0, h = 4.0)\nconst c = Shape.Empty\nShape d = Shape.Circle(0.5)\nlet cur = Shape.Empty\ncur = Shape.Circle(3.0)"
    ));
    ok(&format!(
        "{shape}const shapes = [Shape.Circle(1.0), Shape.Rect(w = 1.0, h = 2.0), Shape.Empty]"
    ));
    fails(
        &format!("{shape}const s = Shape.Rect(3.0, 4.0)"),
        "is created with names",
    );
    fails(
        &format!("{shape}const s = Shape.Circle(radius = 1)"),
        "write 1.0",
    );
    fails(
        &format!("{shape}const s = Shape.Triangle(1.0)"),
        "Shape has no variant Triangle",
    );
    fails(
        &format!("{shape}const s = Shape.Circle(1.0)\nconst r = s.radius"),
        "which is read with match",
    );
    fails(&format!("{shape}const s = Shape.Circle"), "holds data");
    fails(
        &format!("{shape}const s = Shape(radius = 1.0)"),
        "is created through one of its variants",
    );
}

#[test]
fn a_variant_can_be_an_existing_struct() {
    let figure = "const Square = struct { f64 side }\nconst Triangle = struct { f64 base; f64 height }\nconst Figure = union {\n const Square = Square\n const Triangle = Triangle\n const Dot = struct { }\n}\n";
    ok(&format!(
        "{figure}const sq = Square(side = 2.0)\nconst a = Figure.Square(sq)\nconst b = Figure.Triangle(Triangle(base = 3.0, height = 4.0))\nconst c = Figure.Dot"
    ));
    fails(
        &format!("{figure}const a = Figure.Square(2.0)"),
        "give it a Square value",
    );
    fails(
        &format!("{figure}const a = Figure.Triangle(base = 3.0, height = 4.0)"),
        "wraps one Triangle value",
    );
    fails(
        "const U = union { const Missing = Missing }",
        "`Missing` is not declared",
    );
    fails(
        "const Sq = struct { f64 side }\nconst U = union { const A = Sq\n const B = Sq }",
        "twice",
    );
    fails(
        "const Sq = struct { f64 side }\nconst U = union { const Sq = struct { f64 side } }",
        "pick one",
    );
}

#[test]
fn match_takes_a_tagged_union_apart() {
    let shape = "const Shape = union {\n const Circle = struct { f64 radius }\n const Rect = struct { f64 w; f64 h }\n const Empty = struct { }\n}\n";
    let area = format!(
        "{shape}const area = function(Shape s) f64 {{\n match (s) {{\n case (Circle) |c| {{ return 3.14 * c.radius * c.radius }}\n case (Rect) |r| {{ return r.w * r.h }}\n case (Empty) {{ return 0.0 }}\n }}\n}}\n"
    );
    ok(&area);
    // not every variant is handled
    reports(
        &format!(
            "{shape}const f = function(Shape s) f64 {{\n match (s) {{\n case (Circle) |c| {{ return c.radius }}\n case (Empty) {{ return 0.0 }}\n }}\n}}"
        ),
        "Rect is missing",
    );
    ok(&format!(
        "{shape}const f = function(Shape s) f64 {{\n match (s) {{\n case (Circle) |c| {{ return c.radius }}\n default {{ return 0.0 }}\n }}\n}}"
    ));
    fails(
        &format!(
            "{shape}const f = function(Shape s) f64 {{\n match (s) {{\n case (Circle) |c| {{ return c.radius }}\n case (Circle) |c| {{ return 1.0 }}\n default {{ return 0.0 }}\n }}\n}}"
        ),
        "Circle appears twice",
    );
    fails(
        &format!(
            "{shape}const f = function(Shape s) f64 {{\n match (s) {{\n case (Triangle) {{ return 1.0 }}\n default {{ return 0.0 }}\n }}\n}}"
        ),
        "Shape has no variant Triangle",
    );
    // |c| exists only inside its case
    fails(
        &format!(
            "{shape}const f = function(Shape s) {{\n match (s) {{\n case (Circle) |c| {{ print(c.radius) }}\n default {{ }}\n }}\n print(c)\n}}"
        ),
        "out of scope",
    );
    // a function that ends in a match that handles everything and returns in every case
    fails(
        &format!(
            "{shape}const f = function(Shape s) f64 {{\n match (s) {{\n case (Circle) |c| {{ print(1) }}\n default {{ return 0.0 }}\n }}\n}}"
        ),
        "not every path returns a value",
    );
    fails(
        &format!(
            "{shape}const f = function(Shape s) {{\n match (s) {{\n case (Empty) |e| {{ }}\n default {{ }}\n }}\n}}"
        ),
        "has no data to name",
    );
    fails("match (ghost) { default { } }", "`ghost` is not declared");
}

#[test]
fn match_on_an_enum_and_at_the_top_level() {
    let color = "const Color = enum { const Red = 0; const Green = 1 }\n";
    ok(&format!(
        "{color}let label = \"\"\nmatch (Color.Red) {{\n case (Red) {{ label = \"r\" }}\n case (Green) {{ label = \"g\" }}\n}}"
    ));
    fails(
        &format!("{color}match (Color.Red) {{\n case (Red) {{ }}\n}}"),
        "Green is missing",
    );
    fails("match (404) { default { } }", "write switch");
    fails(
        "match (true) { default { } }",
        "match needs an enum or a tagged union",
    );
}

#[test]
fn an_alias_is_the_same_type_and_a_newtype_is_another() {
    ok("const Id = i32\nId a = 5\ni32 b = a");
    ok("const Names = List<String>\n");
    fails(
        "const UserId = struct { i32 value }\nconst OrderId = struct { i32 value }\nconst oid = OrderId(7)\nUserId bad = oid",
        "OrderId is not UserId",
    );
}

#[test]
fn a_generic_type_takes_its_arguments_from_the_values_or_the_context() {
    let boxed = "const Box = struct<T> { T value }\n";
    assert_eq!(
        type_of(&format!("{boxed}const b = Box<i32>(1)"), "b"),
        "Box<i32>"
    );
    assert_eq!(
        type_of(&format!("{boxed}const b = Box(2.5)"), "b"),
        "Box<f64>"
    );
    assert_eq!(
        type_of(&format!("{boxed}const b = Box(\"hi\")"), "b"),
        "Box<String>"
    );
    assert_eq!(
        type_of(&format!("{boxed}Box<u8> b = Box(255)"), "b"),
        "Box<u8>"
    );
    assert_eq!(
        type_of(
            &format!("{boxed}const b = Box<i32>(1)\nconst v = b.value"),
            "v"
        ),
        "i32"
    );
    fails(
        &format!("{boxed}const b = Box<i32>(1)\nb.value = 2.5"),
        "decimal literal is never an integer",
    );
    fails(
        &format!("{boxed}const a = Box(2.5)\nBox<i32> b = a"),
        "Box<f64> is not Box<i32>",
    );
    reports(
        &format!("{boxed}const none = Box()"),
        "T cannot be inferred",
    );
    fails(
        &format!("{boxed}const b = Box<i32, i32>(1)"),
        "has 1 type parameter, but 2 type arguments were given",
    );
    fails(
        &format!("{boxed}Box b = Box(1)"),
        "`Box` takes 1 type argument",
    );
    let pair = "const Pair = struct<A, B> { A first; B second }\n";
    assert_eq!(
        type_of(
            &format!("{pair}const p = Pair(first = 1, second = 2.5)"),
            "p"
        ),
        "Pair<i32, f64>"
    );
    ok(
        "const Holder = struct<T> {\n T value\n const get = function() T { return this.value }\n const set = function(T v) { this.value = v }\n}\nlet h = Holder(1)\nh.set(2)\nconst hv = h.get()",
    );
    fails(
        "const Holder = struct<T> {\n T value\n const set = function(T v) { this.value = v }\n}\nlet h = Holder(1)\nh.set(2.5)",
        "decimal literal is never an integer",
    );
    ok(
        "const Either = union<L, R> {\n const Left = struct { L value }\n const Right = struct { R value }\n}\nconst e1 = Either<i32, String>.Left(1)\nEither<i32, String> e2 = Either.Right(\"x\")",
    );
}

#[test]
fn optional_and_result_come_with_the_language() {
    ok("const found = Optional.Some(5)");
    assert_eq!(
        type_of("const found = Optional.Some(5)", "found"),
        "Optional<i32>"
    );
    ok("Optional<String> missing = Optional.None");
    ok("Result<i32, String> good = Result.Ok(1)\nResult<i32, String> fail = Result.Err(\"x\")");
    fails(
        "const unknown = Optional.None",
        "nothing says what T is: write Optional<String>.None",
    );
    fails("const half = Result.Ok(1)", "nothing says what E is");
    ok(
        "const shown = function(Optional<i32> o) i32 {\n match (o) {\n case (Some) |s| { return s.value }\n case (None) { return 0 }\n }\n}\nconst sv = shown(Optional.Some(5))",
    );
    ok(
        "const outcome = function(Result<i32, String> r) i32 {\n match (r) {\n case (Ok) |ok| { return ok.value }\n case (Err) |err| { return -1 }\n }\n}",
    );
    ok("const f = function() Optional<i32> { return Optional.None }");
}

#[test]
fn the_last_argument_can_be_the_fields_of_a_struct() {
    let column = "const Column = struct { String name; i32 size }\nconst table = function(String name, Column column) String { return name }\n";
    ok(&format!(
        "{column}const users = table(\"users\") {{\n name = \"id\"\n size = 4\n}}"
    ));
    ok(&format!(
        "{column}const same = table(\"users\", Column(name = \"id\", size = 4))"
    ));
    let width = "const Column = struct { String name; i32 size }\nconst width = function(Column column) i32 { return column.size }\n";
    ok(&format!(
        "{width}const w = width {{\n name = \"id\"\n size = 4\n}}"
    ));
    fails(
        &format!("{column}const bad = table(\"users\") {{ name = \"id\" }}"),
        "size was not given",
    );
    fails(
        &format!(
            "{column}const bad = table(\"users\") {{\n name = \"id\"\n size = 4\n color = 1\n}}"
        ),
        "has no field `color`",
    );
    fails(
        &format!("{column}const bad = table(\"users\") |n| {{ name = \"id\"\n size = 4 }}"),
        "a struct block gives fields",
    );
    fails(
        &format!("{column}const bad = Column(name = \"id\", size = 4) {{ }}"),
        "takes its values inside the parentheses",
    );
}

#[test]
fn chaining_with_structs_is_a_chain_of_methods() {
    let chain = "const Closed = struct { }\nconst AfterWhen = struct {\n bool taken\n const or_when = function(bool condition, function() body) AfterWhen {\n if (this.taken) { return this }\n if (condition) { body()\n return AfterWhen(taken = true) }\n return this\n }\n const otherwise = function(function() body) Closed {\n if (!this.taken) { body() }\n return Closed()\n }\n}\nconst when = function(bool condition, function() body) AfterWhen {\n if (condition) { body()\n return AfterWhen(taken = true) }\n return AfterWhen(taken = false)\n}\nlet weather = \"\"\n";
    ok(&format!(
        "{chain}when(true) {{ weather = \"a\" }} or_when(false) {{ weather = \"b\" }} otherwise {{ weather = \"c\" }}"
    ));
    fails(
        &format!("{chain}when(true) {{ }} otherwise {{ }} or_when(false) {{ }}"),
        "Closed has no method `or_when`",
    );
    fails(
        &format!("{chain}when(true) {{ }} or_when(1) {{ }}"),
        "i32 is not bool",
    );
    fails(
        &format!("{chain}otherwise {{ }}"),
        "`otherwise` is not declared",
    );
}

#[test]
fn a_struct_can_hold_a_function() {
    ok(
        "const Button = struct {\n String label\n function() on_click\n}\nlet clicks = 0\nconst b = Button(label = \"save\", on_click = function() { clicks = clicks + 1 })\nb.on_click()",
    );
}

#[test]
fn a_type_is_not_a_value_and_a_map_key_is_not_a_struct() {
    ok(POINT);
    fails(
        &format!("{POINT}Map<Point, String> bad = Map<Point, String>()"),
        "a Map key is a primitive value or a String, not Point",
    );
    fails(
        "const List = struct { }",
        "`List` is a built-in type, choose another name",
    );
}

#[test]
fn an_iterator_is_a_struct_with_a_next_method() {
    ok(
        "const Countdown = struct {\n i32 current\n const next = function() Optional<i32> {\n if (this.current <= 0) { return Optional.None }\n const value = this.current\n this.current = this.current - 1\n return Optional.Some(value)\n }\n}\nconst steps = Countdown(current = 3)\nlet total = 0\nloop {\n match (steps.next()) {\n case (Some) |s| { total = total + s.value }\n case (None) { break }\n }\n}",
    );
}

// ---- errors (06-errors)

const CHECK_AGE: &str = "const check_age = function(i32 age) {\n if (age < 0) { throw(Error(message = \"negative\", code = 1)) }\n}\n";

#[test]
fn only_an_error_can_be_thrown() {
    ok(CHECK_AGE);
    ok("throw(Error())");
    ok("throw(Error(message = \"x\"))");
    fails(
        "throw(\"x\")",
        "only an Error can be thrown, but this is String",
    );
    fails("throw(Error)", "write Error(message = ...), not the type");
    fails("throw(1, 2)", "`throw` takes one error");
    fails("throw()", "`throw` takes one error");
    fails(
        "const e = Error(code = 1.5)",
        "decimal literal is never an integer",
    );
    fails("const e = Error(text = \"x\")", "Error has no field `text`");
}

#[test]
fn try_catch_finally_are_a_chain_with_their_own_order() {
    ok(&format!(
        "{CHECK_AGE}try {{ check_age(1) }} catch |e| {{ print(e.message) }}"
    ));
    ok(&format!(
        "{CHECK_AGE}try {{ check_age(1) }} catch |e| {{ }} finally {{ print(1) }}"
    ));
    ok(&format!(
        "{CHECK_AGE}try {{ check_age(1) }} finally {{ print(1) }}"
    ));
    fails("try { }", "`try` needs a `catch` or a `finally`");
    fails("catch |e| { }", "`catch` without a matching `try`");
    fails("finally { }", "`finally` without a matching `try`");
    fails(
        "try { } finally { } catch |e| { }",
        "`catch` comes once, after the `try` and before the `finally`",
    );
    fails("try { } catch |e| { } catch |f| { }", "`catch` comes once");
    fails(
        "try { } finally { } finally { }",
        "a chain has one `finally`",
    );
    fails(
        "if (true) { } catch |e| { }",
        "`catch` continues a `try`, not `if`",
    );
    fails("try (1) { } catch |e| { }", "takes no parentheses");
    fails(
        "try { } catch { }",
        "`catch` names the error it receives between bars",
    );
    fails("try { } catch |a, b| { }", "`catch` receives one error");
    fails(
        "try { } catch |e| { } finally |x| { }",
        "`finally` takes no names between | |",
    );
}

#[test]
fn the_error_of_a_catch_is_an_ierror_fixed_inside_the_block() {
    assert_eq!(
        type_of(
            "let m = \"\"\ntry { throw(Error()) } catch |e| { m = e.message }\nconst code = 1",
            "m"
        ),
        "String"
    );
    ok("try { throw(Error()) } catch |e| { const n = e.code + 1 }");
    fails(
        "try { throw(Error()) } catch |e| { e = Error() }",
        "`e` is a parameter and cannot be assigned",
    );
    fails(
        "try { throw(Error()) } catch |e| { const x = e.size }",
        "IError has no field `size`",
    );
    ok("try { throw(Error()) } catch |e| { throw(e) }");
    fails(
        "try { throw(Error()) } catch |e| { }\nprint(e)",
        "`e` is out of scope",
    );
}

#[test]
fn a_name_in_one_block_of_a_try_is_not_visible_in_the_others() {
    fails(
        "try { const inside = 1 } catch |e| { print(inside) }",
        "out of scope",
    );
    fails(
        "try { } catch |e| { const inside = 1 } finally { print(inside) }",
        "out of scope",
    );
}

#[test]
fn return_in_a_finally_block_is_an_error() {
    ok(
        "const f = function() String {\n try { return \"a\" } catch |e| { return e.message } finally { print(1) }\n}",
    );
    fails(
        "const f = function() i32 {\n try { } finally { return 1 }\n return 2\n}",
        "`return` in a finally block is not allowed",
    );
    // a function inside a finally block has its own returns
    ok("try { } finally { const g = function() i32 { return 1 }\n print(g()) }");
}

#[test]
fn a_function_may_end_in_a_try_or_a_throw() {
    ok("const f = function(i32 n) i32 {\n try { return 1 } catch |e| { return 2 }\n}");
    ok("const f = function(i32 n) i32 {\n throw(Error())\n}");
    fails(
        "const f = function(i32 n) i32 {\n try { return 1 } catch |e| { print(e.message) }\n}",
        "not every path returns a value",
    );
}

#[test]
fn break_and_continue_work_through_a_try() {
    ok("for ([1, 2]) |n| { try { print(n) } catch |e| { continue } }");
    ok("loop { try { break } finally { print(1) } }");
}

#[test]
fn optional_and_result_have_helpers_for_a_failure() {
    let divide = "const divide = function(i32 top, i32 bottom) Result<i32, String> {\n if (bottom == 0) { return Result.Err(\"zero\") }\n return Result.Ok(top / bottom)\n}\n";
    assert_eq!(
        type_of(&format!("{divide}const a = divide(10, 2).or_throw()"), "a"),
        "i32"
    );
    assert_eq!(
        type_of(&format!("{divide}const a = divide(10, 2).unwrap()"), "a"),
        "i32"
    );
    assert_eq!(
        type_of(
            &format!("{divide}const a = divide(10, 0).unwrapOr(-1)"),
            "a"
        ),
        "i32"
    );
    assert_eq!(
        type_of(
            &format!(
                "{divide}const a = divide(10, 0).or_else |reason| {{ return i32(reason.len()) }}"
            ),
            "a"
        ),
        "i32"
    );
    fails(
        &format!("{divide}const a = divide(10, 0).or_else |reason| {{ return \"x\" }}"),
        "String is not i32",
    );
    fails(
        &format!("{divide}const a = divide(10, 0).or_else {{ return 0 }}"),
        "the callback takes 1 parameter, but the block names 0",
    );
    fails(
        &format!("{divide}const a = divide(10, 0).unwrapOr(\"x\")"),
        "String is not i32",
    );
    let find =
        "const find = function(List<i32> xs, i32 t) Optional<usize> { return Optional.None }\n";
    assert_eq!(
        type_of(&format!("{find}const a = find([1], 1).or_throw()"), "a"),
        "usize"
    );
    assert_eq!(
        type_of(
            &format!("{find}const a = find([1], 1).or_else {{ return 0 }}"),
            "a"
        ),
        "usize"
    );
    fails(
        &format!("{find}const a = find([1], 1).or_else |r| {{ return 0 }}"),
        "the callback takes 0 parameters, but the block names 1",
    );
    fails(
        &format!("{find}const a = find([1], 1).value"),
        "keeps its data in a variant",
    );
}

#[test]
fn the_system_gives_results_instead_of_throwing_for_what_can_fail() {
    assert_eq!(type_of("const a = System.args()", "a"), "Array<String>");
    assert_eq!(
        type_of("const a = System.env(\"HOME\")", "a"),
        "Result<String, String>"
    );
    assert_eq!(
        type_of("const a = File.read(\"x\")", "a"),
        "Result<String, String>"
    );
    assert_eq!(
        type_of("const a = File.write(\"x\", \"y\")", "a"),
        "Result<bool, String>"
    );
    assert_eq!(type_of("const a = File.exists(\"x\")", "a"), "bool");
    assert_eq!(type_of("const a = Time.now()", "a"), "i64");
    assert_eq!(type_of("const a = Time.now() - Time.now()", "a"), "i64");
    assert_eq!(type_of("const a = Random.int(1, 7)", "a"), "i32");
    assert_eq!(type_of("const a = Random.float()", "a"), "f64");
    ok("Random.seed(42)\nSystem.exit(1)");
    ok("try { } catch |e| { const t = System.trace(e) }");
    fails("const a = System.trace(\"x\")", "String is not IError");
    fails("const a = File.read(1)", "i32 is not String");
    fails(
        "const a = System.nothing()",
        "`System` has no function `nothing`",
    );
}

#[test]
fn modifiers_come_from_attributes() {
    let base = "[class(Inheritance.Abstract)]\nconst Shape = class {\n    [method(Inheritance.Abstract)]\n    const area = method() f64\n}\n";
    ok(&format!(
        "{base}const Sq = class {{\n    [method(Inheritance.Override)]\n    const area = method() f64 {{ return 1.0 }}\n}} extends(Shape)"
    ));
    fails(&format!("{base}const s = Shape()"), "abstract");
    fails(
        "const Account = class {\n    [field(Visibility.Private)]\n    i32 pin = 1\n}\nconst a = Account()\nconst p = a.pin",
        "private",
    );
    fails(
        "const Plain = class {\n    const go = method() String { return \"a\" }\n}\n[class(Inheritance.Final)]\nconst Sealed = class { }\nconst Sub = class { } extends(Sealed)",
        "final",
    );
}

#[test]
fn a_trait_clash_must_be_written_out_in_the_class() {
    let traits = "const Greets = trait {\n    const hello = method() String { return \"a\" }\n}\nconst Welcomes = trait {\n    const hello = method() String { return \"b\" }\n}\n";
    fails(
        &format!("{traits}const Clash = class {{ }} use(Greets, Welcomes)"),
        "both have hello",
    );
    ok(&format!(
        "{traits}const Host = class {{\n    const hello = method() String {{ return \"c\" }}\n}} use(Greets, Welcomes)"
    ));
}

#[test]
fn pointers_point_to_variables() {
    ok("i32 t = 1\nptr<i32> p = &t\np.* = 2\np.* += 1\nconst v = p.*");
    ok("ptr<i32> e = null\nconst b = e == null");
    fails("i32 t = 1\nptr<f64> p = &t", "ptr<i32> is not ptr<f64>");
    fails("const c = 5\nconst p = &c", "const");
    fails("const p = &(1 + 2)", "only a variable has an address");
    fails("i32 n = null", "only a ptr<T> can be null");
    fails("const n = null", "null has no type of its own");
    fails(
        "i32 t = 1\nptr<i32> p = &t\nconst q = p + 1",
        "no pointer arithmetic",
    );
    fails(
        "const f = function() ptr<i32> {\n    i32 local = 1\n    return &local\n}",
        "cannot be returned",
    );
}

#[test]
fn manual_memory_and_weak_references() {
    ok("const b = alloc<u8>(4)\nb.at(1).* = 66\nfree(b)");
    fails("const b = alloc<List<i32>>()", "manages itself");
    fails("i32 t = 1\nfree(&t)", "was not allocated");
    ok("const s = sizeof<i32>()");
    fails("const s = sizeof<String>()", "no fixed size");
    ok("const l = [1]\nconst w = Weak(l)\nconst a = w.alive()");
    fails("const w = Weak(5)", "primitive");
}

#[test]
fn attributes_are_declared_and_checked_where_they_are_used() {
    let decl = "const page = attribute(String path)\n";
    ok(&format!(
        "{decl}[page(\"/\")]\nconst home = function() String {{ return \"a\" }}"
    ));
    fails(&format!("{decl}[page(5)] const m = 1"), "is not String");
    fails("[missing] const m = 1", "`missing` is not declared");
    fails(
        "[on(Kind.Function)]\nconst only_fn = attribute()\n[only_fn] const m = 1",
        "can only be put on a function",
    );
    fails("const get = attribute()", "belongs to the language");
    fails(
        "const route = attribute(String path) |target| { }",
        "names 1 parameter",
    );
    ok("const route = attribute(String path) |target, path| { }");
    ok("const logged = attribute() call |target, args, next| { return next(args) }");
    fails(
        "const bad = attribute() call |target, args| { }",
        "takes target, args and next",
    );
}

#[test]
fn reflection_reads_types_and_attributes() {
    let s = "const P = struct {\n    i32 x\n}\n";
    ok(&format!(
        "{s}const info = reflect<P>()\nconst n = info.name\nconst c = info.fields.len()"
    ));
    ok(&format!(
        "{s}let p = P(x = 1)\nconst ptr1 = &p\nconst v = get_field(ptr1, \"x\").unwrap().as<i32>().unwrap()"
    ));
    fails(
        "let n = 1\nconst c = compile(n)",
        "variable of the running program",
    );
    ok("const b = Any(42)\nconst t = b.typeName()\nconst i = b.is<i32>()");
}

#[test]
fn operators_are_functions_marked_with_an_attribute() {
    let v = "const Vec = struct {\n    f64 x\n}\n[operator(Op.Add)]\nconst add_vec = function(Vec a, Vec b) Vec { return a }\n";
    ok(&format!("{v}const s = Vec(x = 1.0) + Vec(x = 2.0)"));
    fails(
        &format!("{v}const s = Vec(x = 1.0) * Vec(x = 2.0)"),
        "no operator `*`",
    );
    fails(
        "[operator(Op.Add)] const bad = function(i32 a, i32 b) i32 { return a + b }",
        "no parameter is a type you declared",
    );
    fails(
        "const V = struct { f64 x }\n[operator(Op.And)] const bad = function(V a, V b) bool { return true }",
        "Op has no And",
    );
}

#[test]
fn the_plain_functions_of_the_library_are_checked() {
    ok("const l = read_line()\nsleep(10)\nconst n = len([1, 2])\nconst k = len(\"ab\")");
    ok("const a = [1, 2].to_array()");
    fails("const n = len(5)", "has no length");
    fails("sleep(\"x\")", "needs a number");
}
