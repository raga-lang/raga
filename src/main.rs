//! The `ragac` command line.

use std::process::ExitCode;

use ragac::diagnostics::Diagnostic;
use ragac::lexer::lex;
use ragac::parser::parse;
use ragac::sema::analyze_file;
use ragac::source::SourceFile;

const USAGE: &str = "\
usage: ragac tokens <file.raga>    print the tokens of a file
       ragac ast <file.raga>       print the syntax tree of a file
       ragac check <file.raga>     report the errors of a file
       ragac types <file.raga>     print the type of every top-level name";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "tokens" => run(path, tokens),
        [command, path] if command == "ast" => run(path, ast),
        [command, path] if command == "check" => run(path, check),
        [command, path] if command == "types" => run(path, types),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Reads the file, runs a command on it, prints the problems it found, and picks the exit status.
fn run(path: &str, command: fn(&SourceFile) -> Vec<Diagnostic>) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("ragac: cannot read {path}: {err}");
            return ExitCode::from(2);
        }
    };
    let file = SourceFile::new(path, text);
    let diagnostics = command(&file);
    for diagnostic in &diagnostics {
        eprint!("{}", diagnostic.render(&file));
    }
    // the files that were imported show their problems in place
    let mut failed = diagnostics.iter().any(Diagnostic::is_error);
    for (imported, problems) in IMPORTED.with(|cell| cell.take()) {
        for diagnostic in &problems {
            eprint!("{}", diagnostic.render(&imported));
        }
        failed |= problems.iter().any(Diagnostic::is_error);
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

thread_local! {
    /// The problems of the files that the last `check` imported, with their texts.
    static IMPORTED: std::cell::RefCell<Vec<(SourceFile, Vec<Diagnostic>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn tokens(file: &SourceFile) -> Vec<Diagnostic> {
    let lexed = lex(&file.text);
    for token in &lexed.tokens {
        let (line, col) = file.line_col(token.span.start as usize);
        println!(
            "{line}:{col}\t{:?}\t{:?}",
            token.kind,
            token.text(&file.text)
        );
    }
    lexed.diagnostics
}

fn ast(file: &SourceFile) -> Vec<Diagnostic> {
    let lexed = lex(&file.text);
    let parsed = parse(&file.text, &lexed.tokens);
    print!("{}", parsed.program);
    // the errors of the earlier stage come first
    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(parsed.diagnostics);
    diagnostics
}

/// Runs every stage that exists. The errors come in the order of the stages.
fn front_end(file: &SourceFile) -> (ragac::ast::Program, ragac::sema::Analysis, Vec<Diagnostic>) {
    let lexed = lex(&file.text);
    let parsed = parse(&file.text, &lexed.tokens);
    let analysis = analyze_file(&parsed.program, std::path::Path::new(&file.name));
    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(parsed.diagnostics);
    diagnostics.extend(analysis.diagnostics.clone());
    IMPORTED.with(|cell| {
        *cell.borrow_mut() = analysis
            .modules
            .iter()
            .map(|m| {
                (
                    SourceFile::new(m.path.to_string_lossy().into_owned(), m.text.clone()),
                    m.diagnostics.clone(),
                )
            })
            .collect();
    });
    (parsed.program, analysis, diagnostics)
}

fn check(file: &SourceFile) -> Vec<Diagnostic> {
    front_end(file).2
}

fn types(file: &SourceFile) -> Vec<Diagnostic> {
    use ragac::ast::StmtKind;
    let (program, analysis, diagnostics) = front_end(file);
    for stmt in &program.stmts {
        if let (StmtKind::Decl(decl), Some(symbol)) = (&stmt.kind, analysis.symbol_of.get(&stmt.id))
        {
            let ty = &analysis.symbols[symbol.0 as usize].ty;
            println!("{}: {ty}", decl.name.name);
        }
    }
    diagnostics
}
