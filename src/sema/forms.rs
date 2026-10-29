//! The keyword heads: `if`, `elif`, `else`, `while`, `loop`, `for`, `switch`, `case`, `default`, `match`, `break` and
//! `continue` (03-control-flow).
//!
//! The parser reads each of them as the call it is, `head (args) |params| { body }` (see [`Trailer`]), and a branch or a
//! loop that continues after its block as a [`Chain`](ExprKind::Chain) of such calls. What is left is to say what each
//! head means, and that is a rule per head: which arguments it takes, what the names between the bars stand for, and
//! what the block may do. That is all that is special about a loop or a branch, and it is checked here.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use super::builtins;
use super::checker::{Checker, declared_names};
use super::defined::{ERROR, IERROR};
use super::scope::{Binding, Constant, Symbol};
use super::ty::{DefinedKind, Ty};
use crate::ast::{Arg, Expr, ExprKind, Stmt, StmtKind, Trailer, UnaryOp};
use crate::span::Span;

/// The heads that are keywords. A name is never one of these, because a keyword cannot be used as a name.
pub(super) fn is_form(name: &str) -> bool {
    matches!(
        name,
        "if" | "elif"
            | "else"
            | "while"
            | "loop"
            | "for"
            | "switch"
            | "match"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "return"
            | "throw"
            | "try"
            | "catch"
            | "finally"
    )
}

/// What a segment of a call is, the head and what was written after it.
struct Segment<'a> {
    call: &'a Expr,
    head: &'a str,
    args: &'a [Arg],
    trailer: Option<&'a Trailer>,
}

fn segment(call: &Expr) -> Option<Segment<'_>> {
    let ExprKind::Call {
        callee,
        args,
        trailer,
        ..
    } = &call.kind
    else {
        return None;
    };
    let ExprKind::Name(name) = &callee.kind else {
        return None;
    };
    Some(Segment {
        call,
        head: &name.name,
        args,
        trailer: trailer.as_ref(),
    })
}

impl Checker {
    /// `if (a) { } elif (b) { } else { }`, `for (xs) |x| { } else { }`: a call and the calls that continue it.
    ///
    /// The order is the rule: `elif` and `else` follow an `if`, `else` follows a `while` or a `for` too, and `else`
    /// comes last and once. Each segment is then checked like a call of its own.
    pub(super) fn chain(&mut self, segments: &[Expr]) -> Ty {
        let first = segments.first().and_then(segment).map(|s| s.head);
        let mut seen_else = false;
        let mut seen_catch = false;
        let mut seen_finally = false;
        for (i, call) in segments.iter().enumerate() {
            let Some(seg) = segment(call) else { continue };
            if !is_form(seg.head) {
                // a chain of definitions such as the versions of a method, outside the class that owns them
                self.error(
                    call.span,
                    format!("`{}` cannot be chained like this here", seg.head),
                );
                continue;
            }
            if i > 0 {
                let problem = match (seg.head, first) {
                    ("catch", Some("try")) if !seen_catch && !seen_finally => None,
                    ("catch", Some("try")) => Some(
                        "`catch` comes once, after the `try` and before the `finally`".to_string(),
                    ),
                    ("finally", Some("try")) if !seen_finally => None,
                    ("finally", Some("try")) => Some("a chain has one `finally`".to_string()),
                    ("catch" | "finally", Some(other)) => {
                        Some(format!("`{}` continues a `try`, not `{other}`", seg.head))
                    }
                    ("else", _) if seen_else => Some("a chain has one `else`".to_string()),
                    (_, _) if seen_else => Some(format!("`{}` cannot come after `else`", seg.head)),
                    ("elif", Some("if")) => None,
                    ("elif", _) => Some("`elif` continues an `if`".to_string()),
                    ("else", Some("loop")) => Some(
                        "a loop with no condition only ends with break, so its else would never run".to_string(),
                    ),
                    ("else", Some("if" | "while" | "for")) => None,
                    ("else", Some(other)) => Some(format!("`else` cannot follow `{other}`")),
                    _ => None,
                };
                if let Some(problem) = problem {
                    self.error(call.span, problem);
                }
            }
            seen_else |= seg.head == "else";
            seen_catch |= seg.head == "catch";
            seen_finally |= seg.head == "finally";
            self.form_segment(&seg, true);
            self.expr_types.insert(call.id, Ty::Void);
        }
        Ty::Void
    }

    /// A keyword head that stands alone.
    pub(super) fn form(
        &mut self,
        call: &Expr,
        head: &str,
        args: &[Arg],
        trailer: Option<&Trailer>,
        continues: bool,
    ) -> Ty {
        let seg = Segment {
            call,
            head,
            args,
            trailer,
        };
        self.form_segment(&seg, continues);
        Ty::Void
    }

    fn form_segment(&mut self, seg: &Segment, continues: bool) {
        match seg.head {
            "if" | "elif" | "else" | "while" | "loop" => self.branch_or_loop(seg, continues),
            "for" => self.for_form(seg),
            "switch" => self.switch_form(seg),
            "match" => self.match_form(seg),
            "case" | "default" => {
                self.error(
                    seg.call.span,
                    format!("`{}` belongs inside a switch or a match", seg.head),
                );
                self.check_arguments(seg.args);
            }
            "break" | "continue" => self.jump(seg),
            "return" => self.return_statement(seg.call, seg.args),
            "throw" => self.throw_form(seg),
            "try" | "catch" | "finally" => self.try_part(seg, continues),
            _ => unreachable!("is_form lists the heads"),
        }
    }

    // ---- the block of a form

    /// The block of a form, checked in a scope of its own. Says whether there was one.
    fn block(&mut self, seg: &Segment) -> bool {
        let Some(trailer) = seg.trailer else {
            self.error(
                seg.call.span,
                format!(
                    "`{}` needs a block in braces: {} (...) {{ ... }}",
                    seg.head, seg.head
                ),
            );
            return false;
        };
        if let Some(name) = trailer.params.first() {
            self.error(
                name.span,
                format!("`{}` takes no names between | |", seg.head),
            );
        }
        self.scoped(&trailer.body);
        true
    }

    fn scoped(&mut self, body: &[Stmt]) {
        self.scopes.push(declared_names(body));
        self.statements(body);
        self.scopes.pop();
    }

    /// The condition of `if`, `elif` and `while`: one value, and a bool.
    fn condition(&mut self, seg: &Segment) {
        let [condition] = seg.args else {
            self.error(
                seg.call.span,
                format!(
                    "`{0}` takes one condition in parentheses: {0} (condition) {{ ... }}",
                    seg.head
                ),
            );
            self.check_arguments(seg.args);
            return;
        };
        let ty = self.expr(&condition.value, Some(&Ty::Bool));
        if !Ty::Bool.accepts(&ty) {
            self.error(
                condition.value.span,
                format!(
                    "the condition of `{}` must be a bool, but this is {ty}: there is no truthiness, write a comparison",
                    seg.head
                ),
            );
        }
    }

    // ---- if, elif, else, while, loop

    fn branch_or_loop(&mut self, seg: &Segment, continues: bool) {
        match seg.head {
            "elif" | "else" if !continues => {
                self.error(
                    seg.call.span,
                    format!(
                        "`{}` without a matching `if`: it must start on the line of the closing brace of the one before it",
                        seg.head
                    ),
                );
            }
            _ => {}
        }
        match seg.head {
            "if" | "elif" | "while" => self.condition(seg),
            _ => {
                if !seg.args.is_empty() {
                    self.error(seg.call.span, format!("`{}` takes no condition", seg.head));
                    self.check_arguments(seg.args);
                }
            }
        }
        let is_loop = matches!(seg.head, "while" | "loop");
        self.loops += usize::from(is_loop);
        self.block(seg);
        self.loops -= usize::from(is_loop);
    }

    // ---- for

    /// `for (items) |item, position| { ... }`, and `for (range(a, b)) |i| { ... }`
    ///
    /// The names between the bars are new constants inside the body, one scope that ends with the loop. What they stand
    /// for depends on what is looped over (03-control-flow section 5): the items of an Array or a List and their
    /// position, the values of a Set, the key and the value of a Map, the characters of a String and their position, the
    /// numbers of a range.
    fn for_form(&mut self, seg: &Segment) {
        let call = seg.call;
        let [items] = seg.args else {
            self.error(
                call.span,
                format!(
                    "`for` takes one collection, but {} were given",
                    seg.args.len()
                ),
            );
            self.check_arguments(seg.args);
            return;
        };
        let (types, allowed, iterable) = self.loop_names(&items.value);
        let Some(trailer) = seg.trailer else {
            self.error(call.span, "`for` needs a block: for (items) |item| { ... }");
            return;
        };

        if !allowed.contains(&trailer.params.len()) {
            let shape = match iterable {
                Ty::Set(_) => "a Set loop has one name: |value|",
                Ty::Map(..) => "a Map loop has two names: |key, value|",
                Ty::Void => "a range loop has one name: |number|",
                _ => "this loop has one name, or two with the position: |item| or |item, position|",
            };
            let at = trailer.params.first().map_or(call.span, |name| name.span);
            self.error(at, shape);
        }

        self.scopes.push(declared_names(&trailer.body));
        let types = types.into_iter().chain(std::iter::repeat(Ty::Error));
        for (name, ty) in trailer.params.iter().zip(types) {
            let symbol = Symbol {
                name: name.name.clone(),
                binding: Binding::Const,
                ty,
                span: name.span,
                constant: None,
                defines: None,
            };
            if self.scopes.declare(symbol).is_err() {
                self.error(
                    name.span,
                    format!("`{}` is already declared in this scope", name.name),
                );
            }
        }
        self.loops += 1;
        self.statements(&trailer.body);
        self.loops -= 1;
        self.scopes.pop();
    }

    /// The types of the names of a loop over this, how many names it takes, and the type of the collection (`Void` for
    /// a range, which is not a value).
    fn loop_names(&mut self, items: &Expr) -> (Vec<Ty>, RangeInclusive<usize>, Ty) {
        if let ExprKind::Call { callee, args, .. } = &items.kind
            && matches!(&callee.kind, ExprKind::Name(name) if name.name == "range")
        {
            let number = self.range_numbers(items, args);
            return (vec![number], 1..=1, Ty::Void);
        }
        let iterable = self.expr(items, None);
        let (types, allowed): (Vec<Ty>, RangeInclusive<usize>) = match &iterable {
            Ty::Array(item) | Ty::List(item) => (vec![(**item).clone(), Ty::USIZE], 1..=2),
            Ty::Set(item) => (vec![(**item).clone()], 1..=1),
            Ty::Map(key, value) => (vec![(**key).clone(), (**value).clone()], 2..=2),
            Ty::String => (vec![Ty::Char, Ty::USIZE], 1..=2),
            Ty::Tuple(_) => {
                self.error(
                    items.span,
                    "a Tuple cannot be looped over: its slots have different types",
                );
                (Vec::new(), 0..=usize::MAX)
            }
            Ty::Error => (Vec::new(), 0..=usize::MAX),
            other => {
                self.error(items.span, format!("{other} cannot be looped over"));
                (Vec::new(), 0..=usize::MAX)
            }
        };
        (types, allowed, iterable)
    }

    /// `range(from, to)` or `range(from, to, step)`: whole numbers of one type, which the loop name then has.
    fn range_numbers(&mut self, call: &Expr, args: &[Arg]) -> Ty {
        if !(2..=3).contains(&args.len()) {
            self.error(
                call.span,
                format!(
                    "range takes 2 or 3 arguments, from, to and an optional step, but {} given",
                    args.len()
                ),
            );
            self.check_arguments(args);
            return Ty::Error;
        }
        let number = self.apply(call, "range", builtins::range_sig(), args);
        self.expr_types.insert(call.id, number.clone());
        if !number.is_integer() && !number.is_error() {
            self.error(
                call.span,
                format!("range counts whole numbers, but these are {number}"),
            );
            return Ty::Error;
        }
        number
    }

    // ---- break and continue

    /// `break`, `break 1`, `continue`, `continue 1`: the number is how many loops to leave besides the innermost one.
    fn jump(&mut self, seg: &Segment) {
        let head = seg.head;
        let loops = self.loops;
        if loops == 0 {
            self.error(
                seg.call.span,
                format!("`{head}` outside a loop: it belongs to while, loop and for"),
            );
            self.check_arguments(seg.args);
            return;
        }
        let Some(arg) = seg.args.first() else { return };
        self.expr(&arg.value, None);
        match &arg.value.kind {
            ExprKind::Int(more) => {
                let leaves = more.saturating_add(1);
                if leaves > loops as u128 {
                    self.error(
                        arg.value.span,
                        format!(
                            "{head} {more} leaves {leaves} loop{}, but only {loops} loop{} surround{} it",
                            if leaves == 1 { "" } else { "s" },
                            if loops == 1 { "" } else { "s" },
                            if loops == 1 { "s" } else { "" },
                        ),
                    );
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Neg, ..
            } => self.error(
                arg.value.span,
                "the number of loops to leave cannot be negative",
            ),
            ExprKind::Error => {}
            _ => self.error(
                arg.value.span,
                "the number of loops to leave is a whole number written in the code",
            ),
        }
    }

    // ---- switch and match

    /// `switch (subject) { case (value, ...) { ... } default { ... } }`
    fn switch_form(&mut self, seg: &Segment) {
        let [subject] = seg.args else {
            self.error(
                seg.call.span,
                "`switch` takes one value in parentheses: switch (value) { ... }",
            );
            self.check_arguments(seg.args);
            return;
        };
        let ty = self.expr(&subject.value, None);
        let problem = match &ty {
            Ty::Int(_) | Ty::Char | Ty::Error => None,
            Ty::Float(_) => Some("a float cannot be switched on: compare it in an if".to_string()),
            Ty::Bool => Some("a bool cannot be switched on: use an if".to_string()),
            Ty::String => Some(
                "a String cannot be switched on: it is compared by reference, compare its text with equals in an if"
                    .to_string(),
            ),
            other => Some(format!(
                "{other} cannot be switched on: switch takes an integer or a char, write match for an enum or a tagged union"
            )),
        };
        if let Some(problem) = &problem {
            self.error(subject.value.span, problem.clone());
        }
        let Some(trailer) = seg.trailer else {
            self.error(
                seg.call.span,
                "`switch` needs a block: switch (value) { ... }",
            );
            return;
        };
        if let Some(name) = trailer.params.first() {
            self.error(name.span, "`switch` takes no names between | |");
        }

        // a subject that cannot be switched on still has its cases checked, against nothing
        let subject_ty = if problem.is_some() { Ty::Error } else { ty };
        let mut seen: HashSet<Constant> = HashSet::new();
        let mut default_at: Option<Span> = None;
        for stmt in &trailer.body {
            let StmtKind::Expr(expr) = &stmt.kind else {
                self.error(
                    stmt.span,
                    "only `case` and `default` can stand directly inside a switch",
                );
                continue;
            };
            let Some(case) = segment(expr).filter(|s| matches!(s.head, "case" | "default")) else {
                self.error(
                    stmt.span,
                    "only `case` and `default` can stand directly inside a switch",
                );
                continue;
            };
            if default_at.is_some() && case.head == "case" {
                self.error(case.call.span, "`default` must come last");
            }
            if case.head == "default" {
                if default_at.is_some() {
                    self.error(case.call.span, "a switch has one `default`");
                }
                default_at = Some(case.call.span);
                if !case.args.is_empty() {
                    self.error(case.call.span, "`default` takes no value");
                    self.check_arguments(case.args);
                }
            } else {
                self.case_values(&case, &subject_ty, &mut seen);
            }
            self.expr_types.insert(case.call.id, Ty::Void);
            self.block(&case);
        }
    }

    /// The values of one `case (a, b)`: each is a literal, or a const made from one, of the type of the subject, and a
    /// value appears in one case only.
    fn case_values(&mut self, case: &Segment, subject: &Ty, seen: &mut HashSet<Constant>) {
        if case.args.is_empty() {
            self.error(case.call.span, "`case` needs a value: case (value) { ... }");
        }
        for arg in case.args {
            let value = &arg.value;
            let ty = self.expr(value, Some(subject));
            self.expect_assignable(subject, &ty, value);
            let Some(constant) = self.constant_of(value) else {
                // a literal of another type has been reported already, as the wrong type
                let literal = matches!(
                    value.kind,
                    ExprKind::Str(_) | ExprKind::Float(_) | ExprKind::Bool(_)
                );
                if !ty.is_error() && !literal {
                    let what = match &value.kind {
                        ExprKind::Name(name) => format!("`{}` is a variable", name.name),
                        _ => "this is computed".to_string(),
                    };
                    self.error(
                        value.span,
                        format!("a case value must be a literal or a const made from one, {what}"),
                    );
                }
                continue;
            };
            if !seen.insert(constant) {
                self.error(value.span, format!("{constant} appears twice"));
            }
        }
    }

    /// `match (subject) { case (Circle) |c| { ... } default { ... } }`
    ///
    /// Takes an enum or a tagged union apart. Each case names a variant and may bind its data, a struct, with the name
    /// between the bars. Every variant must be handled or there must be a `default`.
    fn match_form(&mut self, seg: &Segment) {
        let [subject] = seg.args else {
            self.error(
                seg.call.span,
                "`match` takes one value in parentheses: match (value) { ... }",
            );
            self.check_arguments(seg.args);
            return;
        };
        let ty = self.expr(&subject.value, None);
        let Some(variants) = self.match_cases(&ty) else {
            match &ty {
                Ty::Error => {}
                Ty::Int(_) | Ty::Char => self.error(
                    subject.value.span,
                    format!("match needs an enum or a tagged union, but this is {ty}: write switch"),
                ),
                Ty::Defined {
                    kind: DefinedKind::RawUnion,
                    ..
                } => self.error(
                    subject.value.span,
                    "match needs a tagged union or an enum: an untagged union has no tag to look at",
                ),
                other => self.error(
                    subject.value.span,
                    format!("match needs an enum or a tagged union, but this is {other}"),
                ),
            }
            // the cases are still read, for their own mistakes
            if let Some(trailer) = seg.trailer {
                for stmt in &trailer.body {
                    let case = match &stmt.kind {
                        StmtKind::Expr(expr) => {
                            segment(expr).filter(|s| matches!(s.head, "case" | "default"))
                        }
                        _ => None,
                    };
                    if let Some(case) = case
                        && let Some(inner) = case.trailer
                    {
                        self.check_arguments(case.args);
                        self.scopes.push(declared_names(&inner.body));
                        self.statements(&inner.body);
                        self.scopes.pop();
                    }
                }
            }
            return;
        };
        let Some(trailer) = seg.trailer else {
            self.error(
                seg.call.span,
                "`match` needs a block: match (value) { ... }",
            );
            return;
        };
        if let Some(name) = trailer.params.first() {
            self.error(name.span, "`match` takes no names between | |");
        }

        let mut handled: HashSet<String> = HashSet::new();
        let mut default_at = false;
        for stmt in &trailer.body {
            let case = match &stmt.kind {
                StmtKind::Expr(expr) => {
                    segment(expr).filter(|s| matches!(s.head, "case" | "default"))
                }
                _ => None,
            };
            let Some(case) = case else {
                self.error(
                    stmt.span,
                    "only `case` and `default` can stand directly inside a match",
                );
                continue;
            };
            self.expr_types.insert(case.call.id, Ty::Void);
            if default_at && case.head == "case" {
                self.error(case.call.span, "`default` must come last");
            }
            let mut data: Option<Ty> = None;
            if case.head == "default" {
                if default_at {
                    self.error(case.call.span, "a match has one `default`");
                }
                default_at = true;
                if !case.args.is_empty() {
                    self.error(case.call.span, "`default` takes no value");
                    self.check_arguments(case.args);
                }
            } else {
                match case.args {
                    [arg] => match &arg.value.kind {
                        ExprKind::Name(name) => {
                            match variants.iter().find(|(n, _)| *n == name.name) {
                                Some((_, bound)) => {
                                    if !handled.insert(name.name.clone()) {
                                        self.error(
                                            name.span,
                                            format!("{} appears twice", name.name),
                                        );
                                    }
                                    data = bound.clone();
                                }
                                None => self.error(
                                    name.span,
                                    format!(
                                        "{ty} has no {} {}",
                                        if matches!(
                                            ty,
                                            Ty::Defined {
                                                kind: DefinedKind::Enum,
                                                ..
                                            }
                                        ) {
                                            "member"
                                        } else {
                                            "variant"
                                        },
                                        name.name
                                    ),
                                ),
                            }
                        }
                        _ => {
                            self.error(
                                arg.value.span,
                                "a case names a variant, like case (Circle)",
                            );
                            self.expr(&arg.value, None);
                        }
                    },
                    _ => {
                        self.error(
                            case.call.span,
                            "a case names one variant: case (Circle) |c| { ... }",
                        );
                        self.check_arguments(case.args);
                    }
                }
            }
            self.case_block(&case, data);
        }

        let missing: Vec<&str> = variants
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| !handled.contains(*name))
            .collect();
        if default_at || missing.is_empty() {
            self.exhaustive.insert(seg.call.id);
        } else {
            self.error(
                seg.call.span,
                format!(
                    "not every case is handled, {} is missing: handle it or write a `default`",
                    missing.join(", ")
                ),
            );
        }
    }

    /// The block of a case, with the name between the bars standing for the data of the variant.
    fn case_block(&mut self, case: &Segment, data: Option<Ty>) {
        let Some(trailer) = case.trailer else {
            self.error(
                case.call.span,
                format!("`{}` needs a block in braces", case.head),
            );
            return;
        };
        self.scopes.push(declared_names(&trailer.body));
        match (trailer.params.as_slice(), data) {
            ([], _) => {}
            ([name], Some(ty)) => {
                let symbol = Symbol {
                    name: name.name.clone(),
                    binding: Binding::Param,
                    ty,
                    span: name.span,
                    constant: None,
                    defines: None,
                };
                let _ = self.scopes.declare(symbol);
            }
            ([name], None) => self.error(
                name.span,
                "this case has no data to name: a variant without fields, and `default`, take no name between | |",
            ),
            (more, _) => self.error(
                more[1].span,
                "a case names the data of its variant once: |name|",
            ),
        }
        self.statements(&trailer.body);
        self.scopes.pop();
    }

    // ---- throw, try, catch, finally

    /// `throw(error)`: only an Error, or something that implements IError, can be thrown.
    fn throw_form(&mut self, seg: &Segment) {
        let [arg] = seg.args else {
            self.error(
                seg.call.span,
                "`throw` takes one error in parentheses: throw(Error(message = \"...\"))",
            );
            self.check_arguments(seg.args);
            return;
        };
        let ty = self.expr(&arg.value, None);
        match &ty {
            Ty::Error => {}
            Ty::Defined { id, supers, .. }
                if *id == ERROR || *id == IERROR || supers.contains(&IERROR) => {}
            Ty::TypeValue { name, .. } => self.error(
                arg.value.span,
                format!("throw takes an error value: write {name}(message = ...), not the type"),
            ),
            other => self.error(
                arg.value.span,
                format!("only an Error can be thrown, but this is {other}"),
            ),
        }
        if let Some(trailer) = seg.trailer {
            self.error(
                trailer.body.first().map_or(seg.call.span, |s| s.span),
                "`throw` takes no block",
            );
        }
    }

    /// The parts of a `try`: its block, the `catch |e|` that receives the error, and the `finally` that always runs.
    fn try_part(&mut self, seg: &Segment, continues: bool) {
        if !seg.args.is_empty() {
            self.error(
                seg.call.span,
                format!(
                    "`{}` takes no parentheses: it is followed by its block",
                    seg.head
                ),
            );
            self.check_arguments(seg.args);
        }
        match seg.head {
            "try" => {
                if !continues {
                    self.error(
                        seg.call.span,
                        "`try` needs a `catch` or a `finally` after its block, on the line of the closing brace",
                    );
                }
                self.block(seg);
            }
            "catch" => {
                if !continues {
                    self.error(
                        seg.call.span,
                        "`catch` without a matching `try`: it must start on the line of the closing brace of the block before it",
                    );
                }
                self.catch_block(seg);
            }
            _ => {
                if !continues {
                    self.error(
                        seg.call.span,
                        "`finally` without a matching `try`: it must start on the line of the closing brace of the block before it",
                    );
                }
                self.in_finally += 1;
                self.block(seg);
                self.in_finally -= 1;
            }
        }
    }

    /// `catch |e| { ... }`: `e` is the error, as an IError, fixed inside the block.
    fn catch_block(&mut self, seg: &Segment) {
        let Some(trailer) = seg.trailer else {
            self.error(seg.call.span, "`catch` needs a block: catch |e| { ... }");
            return;
        };
        self.scopes.push(declared_names(&trailer.body));
        match trailer.params.as_slice() {
            [name] => {
                let symbol = Symbol {
                    name: name.name.clone(),
                    binding: Binding::Param,
                    ty: self.defined_ty(IERROR, Vec::new()),
                    span: name.span,
                    constant: None,
                    defines: None,
                };
                let _ = self.scopes.declare(symbol);
            }
            [] => self.error(
                seg.call.span,
                "`catch` names the error it receives between bars: catch |e| { ... }",
            ),
            [_, extra, ..] => {
                self.error(extra.span, "`catch` receives one error: catch |e| { ... }")
            }
        }
        self.statements(&trailer.body);
        self.scopes.pop();
    }
}
