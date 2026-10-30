use crate::lexer::lex;
use crate::parser::parse;

/// The tree of a source as text, one statement per line. Fails if the lexer or the parser reported anything.
fn tree(source: &str) -> String {
    let (text, errors) = run(source);
    assert!(
        errors.is_empty(),
        "unexpected errors for {source:?}: {errors:?}"
    );
    text
}

/// The tree and the messages of every error.
fn run(source: &str) -> (String, Vec<String>) {
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);
    let errors = lexed
        .diagnostics
        .iter()
        .chain(&parsed.diagnostics)
        .map(|d| d.message.clone())
        .collect();
    (parsed.program.to_string().trim_end().to_string(), errors)
}

fn errors(source: &str) -> Vec<String> {
    run(source).1
}

// ---- declarations

#[test]
fn const_and_let() {
    assert_eq!(tree("const x = 1"), "(const x 1)");
    assert_eq!(tree("let   y = 2"), "(let y 2)");
}

#[test]
fn typed_declarations() {
    assert_eq!(tree("u8 level = 255"), "(var u8 level 255)");
    assert_eq!(tree("String s1 = \"hello\""), "(var String s1 \"hello\")");
    assert_eq!(
        tree("List<i32> xs = [1, 2]"),
        "(var List<i32> xs (list 1 2))"
    );
    assert_eq!(
        tree("Map<String, List<i32>> m = x"),
        "(var Map<String, List<i32>> m x)"
    );
}

#[test]
fn a_typed_declaration_is_told_from_a_cast_and_a_comparison() {
    // `u8` followed by `(` is a call, not a type
    assert_eq!(tree("u8(300)"), "(call u8 300)");
    // `a < b` starts like `List<i32>` but is a comparison
    assert_eq!(tree("a < b"), "(< a b)");
    // `a` and `b` on two lines are two statements, not the declaration `a b`
    assert_eq!(tree("a\nb"), "a\nb");
}

#[test]
fn statements_are_separated_by_lines_or_semicolons() {
    assert_eq!(
        tree("let y = 2; let z = 3\nconst w = 4"),
        "(let y 2)\n(let z 3)\n(const w 4)"
    );
    assert_eq!(
        tree("let a1 = 1; let a2 = 2; a1 = a1 + a2").lines().count(),
        3
    );
}

// ---- assignment

#[test]
fn assignment_statements() {
    assert_eq!(tree("v = v + 1"), "(= v (+ v 1))");
    assert_eq!(tree("v += 3"), "(+= v 3)");
    assert_eq!(
        tree("a -= 1\na *= 2\na /= 3\na %= 4\na &= 5\na |= 6\na ^= 7\na <<= 8\na >>= 9"),
        "(-= a 1)\n(*= a 2)\n(/= a 3)\n(%= a 4)\n(&= a 5)\n(|= a 6)\n(^= a 7)\n(<<= a 8)\n(>>= a 9)"
    );
    assert_eq!(tree("p.* = 8"), "(= (deref p) 8)");
    assert_eq!(tree("a.b = 1"), "(= (. a b) 1)");
}

// ---- precedence

#[test]
fn multiplication_binds_tighter_than_addition() {
    assert_eq!(tree("1 + 2 * 3"), "(+ 1 (* 2 3))");
    assert_eq!(tree("(1 + 2) * 3"), "(* (+ 1 2) 3)");
}

#[test]
fn equal_operators_group_to_the_left() {
    assert_eq!(tree("a - b - c"), "(- (- a b) c)");
    assert_eq!(tree("a / b * c"), "(* (/ a b) c)");
}

#[test]
fn the_whole_precedence_table() {
    // from the loosest to the tightest: || && | ^ & == < << + *
    assert_eq!(
        tree("a || b && c | d ^ e & f == g < h << i + j * k"),
        "(|| a (&& b (| c (^ d (& e (== f (< g (<< h (+ i (* j k))))))))))"
    );
    // and read the other way round, the tight ones group first
    assert_eq!(
        tree("a * b + c << d < e == f & g ^ h | i && j || k"),
        "(|| (&& (| (^ (& (== (< (<< (+ (* a b) c) d) e) f) g) h) i) j) k)"
    );
}

#[test]
fn comparison_is_looser_than_arithmetic_and_looser_than_shifts() {
    assert_eq!(tree("a + 1 == b"), "(== (+ a 1) b)");
    assert_eq!(tree("a << 1 > b"), "(> (<< a 1) b)");
    assert_eq!(tree("a == b != c"), "(!= (== a b) c)");
}

#[test]
fn shift_right_is_two_greater_than_that_touch() {
    assert_eq!(tree("16 >> 4"), "(>> 16 4)");
    assert_eq!(tree("a >> b + 1"), "(>> a (+ b 1))");
    // with a space between them they are two comparisons
    assert!(!errors("a > > b").is_empty());
}

// ---- prefix and postfix

#[test]
fn unary_operators() {
    assert_eq!(tree("-5"), "(neg 5)");
    assert_eq!(tree("!true"), "(not true)");
    assert_eq!(tree("~u8(0)"), "(bitnot (call u8 0))");
    assert_eq!(tree("&x"), "(addr x)");
    assert_eq!(tree("- -5"), "(neg (neg 5))");
    assert_eq!(tree("-2 * 3"), "(* (neg 2) 3)");
}

#[test]
fn postfix_binds_tighter_than_prefix() {
    assert_eq!(tree("-p.*"), "(neg (deref p))");
    assert_eq!(
        tree("!items.contains(2)"),
        "(not (call (. items contains) 2))"
    );
}

#[test]
fn bit_and_is_not_address_of() {
    assert_eq!(tree("a & b"), "(& a b)");
    assert_eq!(tree("&a & &b"), "(& (addr a) (addr b))");
}

#[test]
fn calls_members_and_chains() {
    assert_eq!(tree("print(known)"), "(call print known)");
    assert_eq!(tree("Math.abs(-3)"), "(call (. Math abs) (neg 3))");
    assert_eq!(tree("text.upper()"), "(call (. text upper))");
    assert_eq!(tree("a.b.c(1)(2)"), "(call (call (. (. a b) c) 1) 2)");
    assert_eq!(
        tree("String.format(\"%d\", 1, 2)"),
        "(call (. String format) \"%d\" 1 2)"
    );
    assert_eq!(tree("$(\"%s\", name)"), "(call $ \"%s\" name)");
}

#[test]
fn named_arguments() {
    assert_eq!(tree("Point(x = 1.5, y = 2.5)"), "(call Point x=1.5 y=2.5)");
    // == is a comparison, not a name
    assert_eq!(tree("f(a == b)"), "(call f (== a b))");
}

#[test]
fn generic_calls() {
    assert_eq!(tree("List<i32>()"), "(call List<i32>)");
    assert_eq!(
        tree("Map<String, List<i32>>(\"a\", x)"),
        "(call Map<String, List<i32>> \"a\" x)"
    );
    // a `<` that does not open type arguments is the operator
    assert_eq!(tree("a < b"), "(< a b)");
    assert_eq!(tree("f(a < b, c > d)"), "(call f (< a b) (> c d))");
}

// ---- literals

#[test]
fn number_literals() {
    assert_eq!(tree("255"), "255");
    assert_eq!(tree("0xFF"), "255");
    assert_eq!(tree("0b1111_1111"), "255");
    assert_eq!(tree("0o377"), "255");
    assert_eq!(tree("1_000_000"), "1000000");
    assert_eq!(tree("2.5"), "2.5");
    assert_eq!(tree("1.5e3"), "1.5e3");
    assert_eq!(tree("1_0.5"), "10.5");
}

#[test]
fn the_largest_integer_is_a_u128() {
    assert_eq!(
        tree("340282366920938463463374607431768211455"),
        "340282366920938463463374607431768211455"
    );
    // the minus is a negation, so the smallest i128 is a negation of 2^127
    assert_eq!(
        tree("-170141183460469231731687303715884105728"),
        "(neg 170141183460469231731687303715884105728)"
    );
    assert_eq!(
        errors("340282366920938463463374607431768211456"),
        ["this integer is larger than the largest integer type, u128"]
    );
}

#[test]
fn text_and_character_literals_are_unescaped() {
    assert_eq!(tree(r#""line one\nline two""#), r#""line one\nline two""#);
    assert_eq!(tree(r#""say \"hi\"""#), r#""say \"hi\"""#);
    assert_eq!(tree(r#""a\\b""#), r#""a\\b""#);
    assert_eq!(tree(r"'a'"), "'a'");
    assert_eq!(tree(r"'\n'"), r"'\n'");
    assert_eq!(tree(r"'\''"), r"'\''");
    assert_eq!(tree(r"'\0'"), r"'\0'");
    assert_eq!(tree("'é'"), "'é'");
}

#[test]
fn collection_literals() {
    // in a value position `{` is an Array and `[` is a List
    assert_eq!(tree("const a = {1, 2, 3}"), "(const a (array 1 2 3))");
    assert_eq!(tree("const a = {}"), "(const a (array))");
    assert_eq!(tree("const a = [1, 2, 3]"), "(const a (list 1 2 3))");
    assert_eq!(tree("const a = []"), "(const a (list))");
    assert_eq!(tree("const a = [1, 2,]"), "(const a (list 1 2))");
    assert_eq!(tree("(1, 2.5)"), "(tuple 1 2.5)");
    assert_eq!(tree("()"), "(tuple)");
    // and at the start of a statement `{` is a block, so `{1, 2}` there is not an Array
    assert_eq!(tree("{ }"), "(block)");
    assert!(!errors("{1, 2}").is_empty());
}

#[test]
fn one_value_in_parentheses_is_only_a_grouping() {
    assert_eq!(tree("(x)"), "x");
    assert_eq!(tree("((1 + 2))"), "(+ 1 2)");
    assert_eq!(
        errors("(x,)"),
        ["a Tuple has two or more values, write Tuple(x) for one"]
    );
}

#[test]
fn a_brace_at_the_start_of_a_statement_is_a_block_and_in_a_value_it_is_an_array() {
    assert_eq!(tree("{ const inner = 2 }"), "(block (const inner 2))");
    assert_eq!(tree("const a = {1, 2}"), "(const a (array 1 2))");
    assert_eq!(
        tree("const outer = 1\n{\n    const inner = 2\n    print(outer + inner)\n}"),
        "(const outer 1)\n(block (const inner 2) (call print (+ outer inner)))"
    );
}

#[test]
fn blocks_nest() {
    assert_eq!(
        tree("{\n  const sh = 2.5\n  {\n    sh = 3.5\n  }\n}"),
        "(block (const sh 2.5) (block (= sh 3.5)))"
    );
}

// ---- layout

#[test]
fn a_line_ending_with_an_operator_continues() {
    assert_eq!(tree("const total = 1 +\n    2"), "(const total (+ 1 2))");
}

#[test]
fn a_line_starting_with_an_operator_is_a_new_statement() {
    // the `+` does not continue the line above; it starts a statement, and `+` is not a prefix operator
    let (text, errs) = run("const wrong = 1\n    + 2");
    // the `+` is skipped, and the `2` after it is a statement of its own
    assert_eq!(text, "(const wrong 1)\n<error>\n2");
    assert_eq!(errs, ["expected an expression, found `+`"]);
}

#[test]
fn a_line_starting_with_minus_is_a_new_statement() {
    assert_eq!(tree("const a = 1\n-2"), "(const a 1)\n(neg 2)");
}

#[test]
fn line_breaks_inside_brackets_mean_nothing() {
    assert_eq!(
        tree("const xs = [\n    1,\n    2\n]"),
        "(const xs (list 1 2))"
    );
    assert_eq!(tree("print(\n    a\n    + b\n)"), "(call print (+ a b))");
}

#[test]
fn a_block_ends_line_sensitivity_inside_an_open_bracket_and_resets_it() {
    // inside the block the statements are separated by line breaks again
    assert_eq!(tree("{\n  a\n  b\n}"), "(block a b)");
}

#[test]
fn comments_do_not_matter() {
    assert_eq!(
        tree("const x = 1 // one\n// alone\nlet y = 2"),
        "(const x 1)\n(let y 2)"
    );
}

// ---- errors

#[test]
fn a_binding_must_have_a_value() {
    assert_eq!(
        errors("let late"),
        [
            "`late` needs a value: a binding must be initialized when it is declared, write `late = ...`"
        ]
    );
    // without the `=` it is not a declaration: a type and a name are not a call, a name is not an argument without parentheses
    assert_eq!(
        errors("u8 late"),
        [
            "`late` needs a value: a binding must be initialized when it is declared, write `u8 late = ...`"
        ]
    );
}

#[test]
fn const_and_a_type_cannot_be_combined() {
    let (text, errs) = run("const u8 bad = 255");
    assert_eq!(
        errs,
        [
            "`const` and a type cannot be combined: write `const name = value` or `Type name = value`"
        ]
    );
    // it read on as a declaration
    assert_eq!(text, "(const bad 255)");
}

#[test]
fn assignment_is_not_an_expression() {
    assert_eq!(
        errors("x = y = 0"),
        ["assignment is a statement and gives no value, so it cannot be used inside another one"]
    );
    assert!(!errors("const bad = (asg = 5)").is_empty());
}

#[test]
fn only_a_place_can_be_assigned_to() {
    assert_eq!(
        errors("1 + 2 = 3"),
        ["this cannot be assigned to, only a name, a member or `p.*` can"]
    );
    assert_eq!(errors("f() = 3").len(), 1);
}

#[test]
fn two_statements_on_one_line_need_a_semicolon() {
    assert_eq!(
        errors("const x = 1 const y = 2"),
        ["expected the end of the statement, found `const`"]
    );
}

#[test]
fn missing_pieces_are_reported_with_what_was_expected() {
    assert_eq!(
        errors("const = 1"),
        ["expected a name after `const`, found `=`"]
    );
    assert_eq!(
        errors("print(1"),
        ["expected `,` or `)`, found the end of the file"]
    );
    assert_eq!(
        errors("const x ="),
        ["expected an expression, found the end of the file"]
    );
    assert_eq!(
        errors("a."),
        ["expected a name after `.`, found the end of the file"]
    );
    assert_eq!(errors("{ a"), ["expected `}`, found the end of the file"]);
}

#[test]
fn a_stray_closing_brace_is_skipped() {
    assert_eq!(errors("}").len(), 1);
}

#[test]
fn the_parser_goes_on_after_an_error() {
    let (text, errs) = run("const a = 1 +\nconst b = 2\nlet c = 3");
    assert!(!errs.is_empty());
    // the statements after the mistake are still in the tree
    assert!(text.contains("(const b 2)"), "{text}");
    assert!(text.contains("(let c 3)"), "{text}");
}

#[test]
fn what_is_not_read_yet_is_said_so() {
    assert_eq!(
        errors("attribute { }")[0],
        "`attribute` is not read by the parser yet"
    );
}

#[test]
fn an_error_leaves_a_node_that_later_stages_can_skip() {
    let (text, _) = run("const x = )");
    assert_eq!(text, "(const x <error>)");
}

// ---- a block after the arguments: `for` is one of these, not a grammar of its own

#[test]
fn a_for_loop_is_a_call_with_a_trailing_block() {
    assert_eq!(
        tree("for (xs) |x| { print(x) }"),
        "(call for xs |x| {(call print x)})"
    );
    assert_eq!(
        tree("for (xs) |x, at| {\n  total += x\n  print(at)\n}"),
        "(call for xs |x at| {(+= total x) (call print at)})"
    );
    assert_eq!(
        tree("for (List<i32>()) |nothing| { }"),
        "(call for (call List<i32>) |nothing| {})"
    );
    assert_eq!(
        tree("for (m.keys()) |k| { }"),
        "(call for (call (. m keys)) |k| {})"
    );
    assert_eq!(
        tree("for (a) |x| {\n  for (b) |y| { print(y) }\n}"),
        "(call for a |x| {(call for b |y| {(call print y)})})"
    );
}

#[test]
fn any_call_can_take_the_block_after_its_arguments() {
    // the same shape as the loop: the head, the arguments, the bars, the block
    assert_eq!(
        tree("run(3) |n| { print(n) }"),
        "(call run 3 |n| {(call print n)})"
    );
    assert_eq!(tree("run(3) { print(1) }"), "(call run 3 {(call print 1)})");
    assert_eq!(
        tree("const x = run(3) |a, b| { print(a) }"),
        "(const x (call run 3 |a b| {(call print a)}))"
    );
}

#[test]
fn bars_after_a_call_are_an_operator_unless_a_block_follows() {
    assert_eq!(tree("const x = f(1) | y"), "(const x (| (call f 1) y))");
    assert_eq!(
        tree("const x = f(1) | y | z"),
        "(const x (| (| (call f 1) y) z))"
    );
    // a block on the next line is a block of its own
    assert_eq!(
        tree("f(1)\n{ print(2) }"),
        "(call f 1)\n(block (call print 2))"
    );
}

#[test]
fn a_for_loop_that_is_not_written_properly_is_reported() {
    assert_eq!(
        errors("for xs |x| { }")[0],
        "expected `(` after `for`, found `xs`"
    );
    assert_eq!(
        errors("for (xs) |x { }")[0],
        "expected names between `|` and `|`, found `{`"
    );
    assert_eq!(
        errors("for (xs) |1| { }")[0],
        "expected names between `|` and `|`, found `1`"
    );
    assert_eq!(
        errors("for (xs) |x| 5")[0],
        "expected `{` to start the block, found `5`"
    );
    // the statement after a broken loop is still read
    let (text, errors) = run("for xs |x| { }\nconst a = 1");
    assert_eq!(errors.len(), 1);
    assert!(text.ends_with("(const a 1)"), "{text}");
}

// ---- branches, loops and the calls that continue them (03-control-flow)

#[test]
fn an_if_is_a_call_and_its_elif_and_else_continue_it() {
    assert_eq!(tree("if (a) { x = 1 }"), "(call if a {(= x 1)})");
    assert_eq!(
        tree("if (a) { x = 1 } elif (b) { x = 2 } else { x = 3 }"),
        "(chain (call if a {(= x 1)}) (call elif b {(= x 2)}) (call else {(= x 3)}))"
    );
    // the clauses may be on lines of their own as long as each starts on the line of the closing brace
    assert_eq!(
        tree("if (a) {\n  x = 1\n} elif (b) {\n  x = 2\n} else {\n  x = 3\n}"),
        "(chain (call if a {(= x 1)}) (call elif b {(= x 2)}) (call else {(= x 3)}))"
    );
}

#[test]
fn a_clause_on_a_line_of_its_own_is_not_a_link() {
    // `else` starts a line, so the `if` ended and the `else` is a statement of its own
    assert_eq!(
        tree("if (a) { }\nelse { }"),
        "(call if a {})\n(call else {})"
    );
}

#[test]
fn loops_are_calls() {
    assert_eq!(
        tree("while (a < 3) { a += 1 }"),
        "(call while (< a 3) {(+= a 1)})"
    );
    assert_eq!(tree("loop { break() }"), "(call loop {(call break)})");
    assert_eq!(
        tree("for (xs) |x| { print(x) } else { y = 1 }"),
        "(chain (call for xs |x| {(call print x)}) (call else {(= y 1)}))"
    );
    assert_eq!(
        tree("while (a) { } else { }"),
        "(chain (call while a {}) (call else {}))"
    );
}

#[test]
fn break_and_continue_take_a_number_in_parentheses() {
    assert_eq!(tree("break()"), "(call break)");
    assert_eq!(tree("break(1)"), "(call break 1)");
    assert_eq!(tree("continue(2)"), "(call continue 2)");
    assert_eq!(
        tree("if (a) { continue() }"),
        "(call if a {(call continue)})"
    );
    assert_eq!(tree("break(found)"), "(call break found)");
    assert_eq!(tree("break(-1)"), "(call break (neg 1))");
    // a jump is a call, so the parentheses are not optional
    assert_eq!(errors("break"), ["`break` is a call: write break()"]);
    assert_eq!(errors("break 1"), ["`break` is a call: write break(value)"]);
    assert_eq!(
        errors("if (a) { continue }"),
        ["`continue` is a call: write continue()"]
    );
    assert_eq!(
        errors("break { 1 }"),
        ["`break` takes a number in parentheses, write break(1)"]
    );
}

#[test]
fn switch_holds_cases_that_are_calls_too() {
    assert_eq!(
        tree(
            "switch (code) {\n  case (200) { m = 1 }\n  case (500, 503) { m = 2 }\n  default { }\n}"
        ),
        "(call switch code {(call case 200 {(= m 1)}) (call case 500 503 {(= m 2)}) (call default {})})"
    );
}

#[test]
fn a_branch_or_a_loop_is_not_a_value() {
    let found = errors("const bad = if (a) { 1 } else { 2 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("`if` is a statement and gives no value"),
        "{found:?}"
    );
}

#[test]
fn a_branch_that_is_not_written_properly_is_reported() {
    assert_eq!(
        errors("if picnic { }")[0],
        "expected `(` after `if`, found `picnic`"
    );
    // no block at all is for semantic analysis to report, it knows which heads need one
    assert_eq!(tree("if (a) b = 1"), "(call if a)");
    assert_eq!(
        errors("if (a) |x| b = 1")[0],
        "expected `{` to start the block, found `b`"
    );
}

// ---- functions (04-functions)

#[test]
fn a_function_is_the_call_of_function_with_parameters_as_arguments() {
    assert_eq!(
        tree("const add = function(i32 a, i32 b) i32 { return(a + b) }"),
        "(const add (call function (param i32 a) (param i32 b) -> i32 {(call return (+ a b))}))"
    );
    // no parameters, no return type
    assert_eq!(
        tree("const noop = function() { }"),
        "(const noop (call function {}))"
    );
}

#[test]
fn a_parameter_can_have_a_default_and_the_last_can_be_variadic() {
    assert_eq!(
        tree("const f = function(i32 a, i32 b = 2, String ...rest) { }"),
        "(const f (call function (param i32 a) (param i32 b = 2) (param String ...rest) {}))"
    );
}

#[test]
fn a_generic_function_names_its_type_parameters_after_function() {
    assert_eq!(
        tree("const id = function<T, U>(T v, U w) T { return(v) }"),
        "(const id (call function<T, U> (param T v) (param U w) -> T {(call return v)}))"
    );
}

#[test]
fn a_function_type_is_written_like_the_head_of_a_function() {
    assert_eq!(
        tree("Function<[String, i32], String> current = shout"),
        "(var Function<[String, i32], String> current shout)"
    );
    assert_eq!(tree("Function f = g"), "(var Function f g)");
    // the word after the parentheses is the return type, or the name when it is followed by `=`
    assert_eq!(
        tree("Function<[i32], i32> f = g"),
        "(var Function<[i32], i32> f g)"
    );
    assert_eq!(tree("Function<[i32]> f = g"), "(var Function<[i32]> f g)");
    // a callback parameter, with and without a return type
    assert_eq!(
        tree(
            "const f = function(Function<[i32], i32> g, Function work) Function<[i32], i32> { return(g) }"
        ),
        "(const f (call function (param Function<[i32], i32> g) (param Function work) -> Function<[i32], i32> {(call return g)}))"
    );
    assert_eq!(
        tree("const l = List<Function<[i32], i32>>()"),
        "(const l (call List<Function<[i32], i32>>))"
    );
}

#[test]
fn return_gives_a_value_or_nothing() {
    assert_eq!(tree("return()"), "(call return)");
    assert_eq!(tree("return(a + 1)"), "(call return (+ a 1))");
    assert_eq!(tree("if (a) { return() }"), "(call if a {(call return)})");
    // `return (a + 1) * 2` is no longer an expression: the parentheses are the arguments
    assert_eq!(tree("return((a + 1) * 2)"), "(call return (* (+ a 1) 2))");
    assert_eq!(tree("return((1, 2))"), "(call return (tuple 1 2))");
    // a block is the one argument that needs no parentheses
    assert_eq!(tree("return { 1, 2 }"), "(call return {(array 1 2)})");
    assert_eq!(
        tree("return |x| { print(x) }"),
        "(call return |x| {(call print x)})"
    );
    assert_eq!(errors("return"), ["`return` is a call: write return()"]);
    assert_eq!(
        errors("return a"),
        ["`return` is a call: write return(value)"]
    );
}

#[test]
fn a_statement_can_follow_a_block_on_its_line() {
    assert_eq!(
        tree("if (n == 0) { return(true) } return(f(n - 1))"),
        "(call if (== n 0) {(call return true)})\n(call return (call f (- n 1)))"
    );
}

#[test]
fn a_name_is_not_an_argument_without_parentheses() {
    assert_eq!(
        errors("double base"),
        ["a name is not an argument without parentheses, only a block is: write double(base)"]
    );
    assert_eq!(tree("double(base)"), "(call double base)");
    assert_eq!(tree("double(user.age)"), "(call double (. user age))");
    assert_eq!(tree("a(b(c))"), "(call a (call b c))");
    assert_eq!(
        tree("const x = double(base) + 1"),
        "(const x (+ (call double base) 1))"
    );
    // a minus after a name is the operator
    assert_eq!(tree("const x = double -base"), "(const x (- double base))");
    assert_eq!(tree("nums.push(x)"), "(call (. nums push) x)");
    assert_eq!(
        errors("nums.push x"),
        ["a name is not an argument without parentheses, only a block is: write nums.push(x)"]
    );
    // a block is the one argument that needs no parentheses
    assert_eq!(
        tree("const t = double(sum { 1, 2, 3 })"),
        "(const t (call double (call sum {(array 1 2 3)})))"
    );
    assert_eq!(
        tree("double(sum |n| { print(n) })"),
        "(call double (call sum |n| {(call print n)}))"
    );
    // a name alone is the function, never a call
    assert_eq!(tree("const g = answer"), "(const g answer)");
}

#[test]
fn a_call_can_leave_out_the_parentheses_when_it_has_only_a_block() {
    assert_eq!(tree("timed { print(1) }"), "(call timed {(call print 1)})");
    assert_eq!(
        tree("timed |x| { print(x) }"),
        "(call timed |x| {(call print x)})"
    );
    assert_eq!(
        tree("const w = width { size = 4 }"),
        "(const w (call width {(= size 4)}))"
    );
    // after a name, bars with no block are the operator
    assert_eq!(tree("const x = f | y"), "(const x (| f y))");
}

#[test]
fn a_value_written_out_needs_parentheses() {
    let found = errors("double 5");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("write double(5)"), "{found:?}");
    assert!(errors("print \"hi\"")[0].contains("write print(\"hi\")"));
    // a name after a name is not an argument, even when a block follows it
    assert!(errors("double base { }")[0].contains("write double(base)"));
}

#[test]
fn the_last_argument_can_be_the_values_of_an_array() {
    assert_eq!(
        tree("const t = tally(\"s\") { 4, 8, 15 }"),
        "(const t (call tally \"s\" {(array 4 8 15)}))"
    );
    assert_eq!(
        tree("const t = tally(\"s\") {\n  4,\n  8,\n  15\n}"),
        "(const t (call tally \"s\" {(array 4 8 15)}))"
    );
    // a block with statements is still a block of statements
    assert_eq!(
        tree("apply(4) |n| { print(n) }"),
        "(call apply 4 |n| {(call print n)})"
    );
    assert_eq!(
        tree("apply(4) |a, b| { print(a) }"),
        "(call apply 4 |a b| {(call print a)})"
    );
}

#[test]
fn a_generic_function_can_be_called_or_given_its_types() {
    assert_eq!(
        tree("const c = identity<f64>(2.5)"),
        "(const c (call identity<f64> 2.5))"
    );
    assert_eq!(
        tree("const i = identity_of<i32>"),
        "(const i (typeapply identity_of i32))"
    );
    assert_eq!(
        tree("apply(identity_of<i32>, 5)"),
        "(call apply (typeapply identity_of i32) 5)"
    );
    // not type arguments: the operator
    assert_eq!(tree("const x = a < b"), "(const x (< a b))");
}

#[test]
fn a_call_can_be_called() {
    assert_eq!(
        tree("const q = style(false)(\"hello\", 3)"),
        "(const q (call (call style false) \"hello\" 3))"
    );
}

// ---- chaining: the dots are left out after a block

#[test]
fn a_chain_is_written_with_dots_and_after_a_block_without_the_dot() {
    // calls that return values are continued with a dot
    assert_eq!(
        tree("user().select().limit(1)"),
        "(call (. (call (. (call user) select)) limit) 1)"
    );
    // when the last argument of a call is a block, the next name follows the closing brace and the dot is left out: each
    // step is a call of a member of the step before it, with its block as the last argument
    assert_eq!(
        tree("user { } select { } limit (1)"),
        "(call (. (call (. (call user {}) select) {}) limit) 1)"
    );
    assert_eq!(
        tree("when(a) { x = 1 } or_when(b) { x = 2 } otherwise { x = 3 }"),
        "(call (. (call (. (call when a {(= x 1)}) or_when) b {(= x 2)}) otherwise) {(= x 3)})"
    );
}

#[test]
fn a_step_of_a_chain_takes_arguments_bars_and_a_block_like_any_call() {
    assert_eq!(
        tree("const r = fetch(\"u\") { } on_error |e| { print(e) } done (1)"),
        "(const r (call (. (call (. (call fetch \"u\" {}) on_error) |e| {(call print e)}) done) 1))"
    );
}

#[test]
fn a_step_can_follow_the_parentheses_of_the_call_before_it() {
    assert_eq!(
        tree("fetch(\"/items\") then |body| { x = body } on_error |reason| { x = reason }"),
        "(call (. (call (. (call fetch \"/items\") then) |body| {(= x body)}) on_error) |reason| {(= x reason)})"
    );
}

#[test]
fn a_name_after_a_block_on_another_line_is_a_new_statement() {
    assert_eq!(
        tree("run { }\nselect { }"),
        "(call run {})\n(call select {})"
    );
}

#[test]
fn a_chain_can_continue_a_call_that_is_an_argument() {
    assert_eq!(
        tree("double(sum { 1, 2 } select { 3 })"),
        "(call double (call (. (call sum {(array 1 2)}) select) {3}))"
    );
}

#[test]
fn nothing_continues_a_function_definition_or_a_keyword_branch() {
    // a branch is followed by a statement, only elif and else continue it
    assert_eq!(
        tree("if (a) { x = 1 } print(2)"),
        "(call if a {(= x 1)})\n(call print 2)"
    );
    assert!(!errors("const f = function() { } g").is_empty());
}

// ---- types (05-structs-enums-unions)

#[test]
fn a_type_is_the_call_of_struct_enum_or_union_with_a_body_of_members() {
    assert_eq!(
        tree("const Point = struct {\n  f64 x\n  f64 y\n}"),
        "(const Point (call struct {(param f64 x) (param f64 y)}))"
    );
    assert_eq!(
        tree("const P = struct { f64 w; f64 h }"),
        "(const P (call struct {(param f64 w) (param f64 h)}))"
    );
    assert_eq!(
        tree("const W = struct {\n  String title = \"untitled\"\n  List<String> tags = []\n}"),
        "(const W (call struct {(param String title = \"untitled\") (param List<String> tags = (list))}))"
    );
    assert_eq!(
        tree("const Color = enum { const Red = 0; const Green = 1 }"),
        "(const Color (call enum {(const Red 0) (const Green 1)}))"
    );
    assert_eq!(
        tree(
            "const Shape = union {\n  const Circle = struct { f64 radius }\n  const Square = Square\n}"
        ),
        "(const Shape (call union {(const Circle (call struct {(param f64 radius)})) (const Square Square)}))"
    );
}

#[test]
fn a_type_takes_type_parameters_after_its_head() {
    assert_eq!(
        tree("const Box = struct<T> { T value }"),
        "(const Box (call struct<T> {(param T value)}))"
    );
    assert_eq!(
        tree("const Either = union<L, R> { const Left = struct { L value } }"),
        "(const Either (call union<L, R> {(const Left (call struct {(param L value)}))}))"
    );
}

#[test]
fn a_method_is_a_function_in_the_body_and_this_is_a_name() {
    assert_eq!(
        tree(
            "const C = struct {\n  i32 value\n  const get = function() i32 { return(this.value) }\n}"
        ),
        "(const C (call struct {(param i32 value) (const get (call function -> i32 {(call return (. this value))}))}))"
    );
    assert_eq!(
        tree("this.value = this.value + 1"),
        "(= (. this value) (+ (. this value) 1))"
    );
}

#[test]
fn members_are_separated_by_a_line_or_a_semicolon_and_never_a_comma() {
    assert!(errors("const bad = struct { f64 w, f64 h }")[0].contains("separate the members"));
    assert!(
        errors("const bad = enum { const Red = 0, const Green = 1 }")[0]
            .contains("separate the members")
    );
    assert!(!errors("const bad = struct { f64 w f64 h }").is_empty());
    // a field of a function type
    assert_eq!(
        tree("const B = struct {\n  String label\n  Function on_click\n}"),
        "(const B (call struct {(param String label) (param Function on_click)}))"
    );
}

#[test]
fn a_generic_type_can_be_named_with_its_types_and_a_member_taken() {
    assert_eq!(
        tree("const e = Either<i32, String>.Left(1)"),
        "(const e (call (. (typeapply Either i32 String) Left) 1))"
    );
    assert_eq!(tree("const s = Shape.Empty"), "(const s (. Shape Empty))");
    assert_eq!(
        tree("match (s) {\n  case (Circle) |c| { print(c) }\n  default { }\n}"),
        "(call match s {(call case Circle |c| {(call print c)}) (call default {})})"
    );
}

// ---- errors (06-errors)

#[test]
fn try_catch_finally_are_a_chain_of_calls() {
    assert_eq!(
        tree("try { a() } catch |e| { b(e) } finally { c() }"),
        "(chain (call try {(call a)}) (call catch |e| {(call b e)}) (call finally {(call c)}))"
    );
    assert_eq!(
        tree("try { a() } finally { c() }"),
        "(chain (call try {(call a)}) (call finally {(call c)}))"
    );
    assert_eq!(
        tree("try {\n  a()\n} catch |e| {\n  b(e)\n}"),
        "(chain (call try {(call a)}) (call catch |e| {(call b e)}))"
    );
}

#[test]
fn throw_is_a_call_with_parentheses() {
    assert_eq!(
        tree("throw(Error(message = \"x\", code = 1))"),
        "(call throw (call Error message=\"x\" code=1))"
    );
    assert_eq!(tree("if (a) { throw(e) }"), "(call if a {(call throw e)})");
    assert!(errors("throw e")[0].contains("expected `(` after `throw`"));
}

#[test]
fn try_is_not_a_value() {
    let found = errors("const x = try { } catch |e| { }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`try` is a statement"), "{found:?}");
}

#[test]
fn a_helper_with_a_block_can_be_called_on_a_result() {
    assert_eq!(
        tree("const r = divide(10, 0).or_else |reason| { return(0) }"),
        "(const r (call (. (call divide 10 0) or_else) |reason| {(call return 0)}))"
    );
    assert_eq!(
        tree("const r = find(xs).or_else { return(0) }"),
        "(const r (call (. (call find xs) or_else) {(call return 0)}))"
    );
}

#[test]
fn export_is_a_call_with_a_block_of_names() {
    let (text, errs) = run("export { a, b }");
    assert!(errs.is_empty(), "{errs:?}");
    assert!(text.contains("(call export {(array a b)})"), "{text}");
}

#[test]
fn the_type_of_a_module_is_written_with_a_dot() {
    let (text, errs) = run("shapes.Point origin = shapes.Point(x = 1.0, y = 1.0)");
    assert!(errs.is_empty(), "{errs:?}");
    assert!(text.contains("(var shapes.Point origin"), "{text}");
}
