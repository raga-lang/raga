# Raga

A programming language built around one shape: every construct is the same binding, `name = head (args) { block }`, whether it is
a variable, a function, a type, a class, a loop or a call with a block.

`hello.raga` is the quick start, read it top to bottom. `templates/` explains each area in full, in the order to read them:
`01-basics`, `02-data-structures`, `03-control-flow`, `04-functions`, `05-structs-enums-unions`, `06-errors`,
`07-classes-and-traits`, `08-memory`, `09-attributes-and-reflection`, `10-modules`, `11-standard-library`.
When a template says what a line does or prints, that is the meaning of the language.

## The compiler

`ragac` is written in Rust. It reads a `.raga` file, reports every problem it finds, and follows `import` into the other files.

| Stage | Module | What it does |
|---|---|---|
| Lexical analysis | `src/lexer` | text to tokens |
| Syntax analysis | `src/parser`, `src/ast` | tokens to the syntax tree |
| Semantic analysis | `src/sema` | names, types, generics, classes, modules |

Shared by every stage: `span` (positions in the text), `source` (a file, lines and columns), `diagnostics` (errors and how they are shown).

```
cargo run -- tokens hello.raga      print the tokens of a file
cargo run -- ast hello.raga         print the syntax tree of a file
cargo run -- check hello.raga       report the errors of a file
cargo run -- types hello.raga       print the type of every top-level name
cargo test                          every .raga file must lex, every template must parse and check without a diagnostic,
                                    and every `ERROR:` example in a template must be rejected
cargo clippy --all-targets          lints
cargo fmt                           format
```

Rules of the code:

- A stage is a function from its input to its output plus a list of diagnostics. It does not stop at the first error.
- A stage does not read the output of a later one. `lexer` knows nothing about the parser, and so on down the pipeline.
- No dependencies. No `unsafe` code.
- A token or a node keeps a `Span`, never a copy of the text.

## Editor

`../vs-code-extension` adds Raga to VS Code: highlighting, snippets, and an analyzer that infers types and reports mistakes while you type.
