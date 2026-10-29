//! The design documents are the test input: every `.raga` file of the repository must lex without a diagnostic.

use std::path::{Path, PathBuf};

use ragac::lexer::{TokenKind, lex};
use ragac::parser::parse;
use ragac::sema::analyze_file;
use ragac::source::SourceFile;

fn raga_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
    {
        let path = entry.unwrap().path();
        if path.is_dir() {
            raga_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "raga") {
            out.push(path);
        }
    }
}

#[test]
fn every_template_lexes_without_a_diagnostic() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("hello.raga")];
    raga_files(&root.join("templates"), &mut files);
    assert!(files.len() > 10, "found only {} files", files.len());

    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let lexed = lex(&text);
        assert!(
            lexed.diagnostics.is_empty(),
            "{}: {:?}",
            path.display(),
            lexed.diagnostics.first()
        );
        assert_eq!(lexed.tokens.last().map(|t| t.kind), Some(TokenKind::Eof));
    }
}

#[test]
fn basics_parses_without_a_diagnostic() {
    // 01-basics is the template the parser is written for. The others use syntax it does not read yet.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/01-basics.raga");
    let text = std::fs::read_to_string(&path).unwrap();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 100);
}

/// Every error from every stage, with the line it is on.
fn all_errors(text: &str) -> Vec<(usize, String)> {
    let file = SourceFile::new("test.raga", text);
    let lexed = lex(text);
    let parsed = parse(text, &lexed.tokens);
    // the text may differ from the file on disk, but `import` finds the other files from the folder of the templates
    let place = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/_checked.raga");
    let analysis = analyze_file(&parsed.program, &place);
    let mut errors: Vec<(usize, String)> = lexed
        .diagnostics
        .iter()
        .chain(&parsed.diagnostics)
        .chain(&analysis.diagnostics)
        .map(|d| (file.line_col(d.span.start as usize).0, d.message.clone()))
        .collect();
    // a problem in an imported file belongs to no line of this one
    for module in &analysis.modules {
        for d in &module.diagnostics {
            errors.push((0, format!("{}: {}", module.path.display(), d.message)));
        }
    }
    errors
}

fn basics() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/01-basics.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn basics_passes_every_stage() {
    let errors = all_errors(&basics());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

/// A design file states its own errors: a line like
///
/// ```text
/// // small = 300                  // ERROR: does not fit in u8
/// ```
///
/// is code that must be rejected. This puts each such line back as code, in its place in the file (so the names it
/// uses are declared above it), and requires that the compiler reports an error on that very line. An example whose
/// first line opens a block continues on the comment lines below it, up to the line that is only `}`, and the error may
/// be on any of them. `ERROR at run time` is not a mistake the compiler can see and is not checked. A line whose code
/// `skip` accepts is not checked yet: it uses something the compiler does not read. Gives how many were checked.
fn assert_error_examples_are_rejected(
    text: &str,
    skip: impl Fn(&str) -> bool,
    ignored: &[std::ops::RangeInclusive<usize>],
) -> usize {
    let all: Vec<&str> = text.lines().collect();
    let mut checked = 0;
    let mut skipped = Vec::new();
    for (index, line) in all.iter().enumerate() {
        let Some(rest) = line.strip_prefix("// ") else {
            continue;
        };
        // an indented line is part of a larger example in a comment, it is not code that stands alone
        if rest.starts_with(' ') {
            continue;
        }
        let Some((code, marker)) = rest.rsplit_once("// ERROR") else {
            continue;
        };
        // the code of compile() runs when the program is built, which needs the interpreter that is not written yet
        if marker.starts_with(" at run time") || marker.starts_with(" at compile time") {
            skipped.push(code.trim_end().to_string());
            continue;
        }
        let code = code.trim_end();

        if skip(code) {
            skipped.push(code.to_string());
            continue;
        }

        // the lines of the example: this one, and the comment lines that finish a block it opens
        let mut last = index;
        let mut lines: Vec<String> = all.iter().map(|l| l.to_string()).collect();
        lines[index] = code.to_string();
        if code.ends_with('{') {
            loop {
                last += 1;
                let Some(next) = all.get(last).and_then(|l| l.strip_prefix("//")) else {
                    panic!("line {}: the block of `{code}` is never closed", index + 1);
                };
                let next = next.strip_prefix(' ').unwrap_or(next);
                lines[last] = next.to_string();
                if next.trim() == "}" {
                    break;
                }
            }
        }
        let source = lines.join("\n");
        let errors = without_lines(all_errors(&source), ignored);
        let range = index + 1..=last + 1;
        assert!(
            errors.iter().any(|(line, _)| range.contains(line)),
            "line {}: `{code}` should be an error, but the compiler reported {:?}",
            index + 1,
            errors
        );
        // and it is the only place that is wrong. The exception is a name declared by the bad line and then declared
        // again by the correct line that follows it in the file: that is a follow-on of the example, not a mistake.
        assert!(
            errors
                .iter()
                .all(|(line, message)| range.contains(line) || message.contains("already declared")),
            "line {}: `{code}` caused errors on other lines too: {:?}",
            index + 1,
            errors
        );
        checked += 1;
    }
    println!(
        "{checked} error examples rejected, {} not checked: {skipped:?}",
        skipped.len()
    );
    checked
}

#[test]
fn basics_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&basics(), |_| false, &[]);
    assert!(checked >= 20, "only {checked} examples were checked");
}

fn data_structures() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/02-data-structures.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn data_structures_parses_without_a_diagnostic() {
    let text = data_structures();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 40);
}

#[test]
fn data_structures_passes_every_stage() {
    let errors = all_errors(&data_structures());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn data_structures_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&data_structures(), |_| false, &[]);
    assert!(checked >= 13, "only {checked} examples were checked");
}

fn control_flow() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/03-control-flow.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn control_flow_parses_without_a_diagnostic() {
    let text = control_flow();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 80);
}

#[test]
fn control_flow_passes_every_stage() {
    let errors = all_errors(&control_flow());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn control_flow_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&control_flow(), |_| false, &[]);
    assert!(checked >= 20, "only {checked} examples were checked");
}

/// The errors that are not on one of the lines.
fn without_lines(
    errors: Vec<(usize, String)>,
    ranges: &[std::ops::RangeInclusive<usize>],
) -> Vec<(usize, String)> {
    errors
        .into_iter()
        .filter(|(line, _)| !ranges.iter().any(|range| range.contains(line)))
        .collect()
}

fn functions() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/04-functions.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn functions_parse_without_a_diagnostic() {
    let text = functions();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 100);
}

#[test]
fn functions_pass_every_stage() {
    let errors = all_errors(&functions());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn functions_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&functions(), |_| false, &[]);
    assert!(checked >= 43, "only {checked} examples were checked");
}

fn structs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/05-structs-enums-unions.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn structs_parse_without_a_diagnostic() {
    let text = structs();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 100);
}

#[test]
fn structs_pass_every_stage() {
    let errors = all_errors(&structs());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn structs_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&structs(), |_| false, &[]);
    assert!(checked >= 49, "only {checked} examples were checked");
}

fn errors_template() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/06-errors.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn errors_parse_without_a_diagnostic() {
    let text = errors_template();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 40);
}

#[test]
fn errors_pass_every_stage() {
    let errors = all_errors(&errors_template());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn errors_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&errors_template(), |_| false, &[]);
    assert!(checked >= 4, "only {checked} examples were checked");
}

fn classes() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/07-classes-and-traits.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn classes_parse_without_a_diagnostic() {
    let text = classes();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 60);
}

#[test]
fn classes_pass_every_stage() {
    let errors = all_errors(&classes());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn classes_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&classes(), |_| false, &[]);
    assert!(checked >= 40, "only {checked} examples were checked");
}

fn memory() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/08-memory.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn memory_parses_without_a_diagnostic() {
    let text = memory();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 60);
}

#[test]
fn memory_passes_every_stage() {
    let errors = all_errors(&memory());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn memory_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&memory(), |_| false, &[]);
    assert!(checked >= 12, "only {checked} examples were checked");
}

fn reflection() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/09-attributes-and-reflection.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn reflection_parses_without_a_diagnostic() {
    let text = reflection();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
    assert!(parsed.program.stmts.len() > 60);
}

#[test]
fn reflection_passes_every_stage() {
    let errors = all_errors(&reflection());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn reflection_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&reflection(), |_| false, &[]);
    assert!(checked >= 14, "only {checked} examples were checked");
}

fn modules() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/10-modules.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn modules_parse_without_a_diagnostic() {
    let text = modules();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "{} errors, the first: {:?}",
        parsed.diagnostics.len(),
        parsed.diagnostics.first()
    );
}

#[test]
fn modules_pass_every_stage() {
    let errors = all_errors(&modules());
    assert!(
        errors.is_empty(),
        "{} errors, the first: {:?}",
        errors.len(),
        errors.first()
    );
}

#[test]
fn modules_error_examples_are_rejected() {
    let checked = assert_error_examples_are_rejected(&modules(), |_| false, &[]);
    assert!(checked >= 6, "only {checked} examples were checked");
}

/// A folder with these files, to try `import` on.
fn project(files: &[(&str, &str)]) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ragac-modules-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn problems(dir: &Path, main: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join(main)).unwrap();
    let lexed = lex(&text);
    let parsed = parse(&text, &lexed.tokens);
    let analysis = analyze_file(&parsed.program, &dir.join(main));
    let mut out: Vec<String> = parsed
        .diagnostics
        .iter()
        .chain(&analysis.diagnostics)
        .map(|d| d.message.clone())
        .collect();
    for module in &analysis.modules {
        out.extend(module.diagnostics.iter().map(|d| d.message.clone()));
    }
    out
}

#[test]
fn modules_that_import_each_other_are_reported() {
    let dir = project(&[
        ("a.raga", "const b = import(\"./b\")\nexport { b }\n"),
        ("b.raga", "const a = import(\"./a\")\nexport { a }\n"),
    ]);
    let found = problems(&dir, "a.raga");
    assert!(
        found.iter().any(|m| m.contains("import each other")),
        "{found:?}"
    );
}

#[test]
fn an_export_must_name_declared_names_once_and_come_last() {
    let dir = project(&[(
        "m.raga",
        "const a = 1\nexport { a, a, nope }\nconst z = 2\n",
    )]);
    let found = problems(&dir, "m.raga");
    for wanted in [
        "exported twice",
        "not declared in this file",
        "last statement",
    ] {
        assert!(
            found.iter().any(|m| m.contains(wanted)),
            "{wanted}: {found:?}"
        );
    }
}

#[test]
fn a_module_shares_only_what_it_exports() {
    let dir = project(&[
        (
            "lib.raga",
            "const open = 1\nconst hidden = 2\nconst Box = struct {\n    i32 n\n}\nexport { open, Box }\n",
        ),
        (
            "main.raga",
            "const lib = import(\"./lib\")\nconst a = lib.open\nlib.Box b = lib.Box(n = 1)\nconst c = lib.hidden\n",
        ),
    ]);
    let found = problems(&dir, "main.raga");
    assert_eq!(found, vec!["hidden is not exported by ./lib".to_string()]);
}

fn library() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/11-standard-library.raga");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn library_passes_every_stage() {
    let errors = all_errors(&library());
    assert!(errors.is_empty(), "{errors:?}");
}
