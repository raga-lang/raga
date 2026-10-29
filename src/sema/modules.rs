//! Modules (10-modules): `import` gives a file as a value, `export { a, b }` lists the names a file shares.
//!
//! A file is read and checked by the same checker as the file that imports it, with scopes of its own, so the types it
//! defines have one number in the whole program. The checker keeps what each file exported, and a use of the module
//! (`shapes.circle_area(2.0)`, `shapes.Point`) is looked up there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::checker::{Checker, declared_names};
use super::scope::{Defines, Scopes, Symbol};
use super::ty::Ty;
use crate::ast::{Arg, Expr, ExprKind, Ident, NodeId, Stmt, StmtKind, Trailer};
use crate::diagnostics::Diagnostic;
use crate::lexer::lex;
use crate::parser::parse;
use crate::span::Span;

/// One name that a file shares.
#[derive(Debug, Clone)]
pub(super) struct Export {
    pub name: String,
    pub ty: Ty,
    pub defines: Option<Defines>,
}

/// A file that was imported: where it is, how it was first written, and what it shares.
#[derive(Debug, Clone)]
pub(super) struct ModuleInfo {
    pub written: String,
    pub exports: Vec<Export>,
}

/// What is known about the file being checked, kept aside while another file is read.
pub(super) struct FileState {
    scopes: Scopes,
    diagnostics: Vec<Diagnostic>,
    expr_types: HashMap<NodeId, Ty>,
    symbol_of: HashMap<NodeId, super::scope::SymbolId>,
    bare_calls: std::collections::HashSet<NodeId>,
    exhaustive: std::collections::HashSet<NodeId>,
    file: Option<PathBuf>,
    exported: Option<Vec<(Ident, super::scope::SymbolId)>>,
}

/// The diagnostics of a file that was imported, with its text, to show them in place.
#[derive(Debug)]
pub struct ModuleReport {
    pub path: PathBuf,
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// The lines `name = "path"` of a `raga.project` file; `//` starts a comment.
fn project_names(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                return None;
            }
            let (name, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"');
            Some((name.trim().to_string(), value.to_string()))
        })
        .collect()
}

impl Checker {
    // ---- import

    /// `import("./geometry/shapes")` or `import("greeting")`.
    pub(super) fn import_call(
        &mut self,
        call: &Expr,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        if trailer.is_some() {
            self.error(call.span, "`import` takes no block after its argument");
        }
        let literal = match args {
            [Arg { value, name: None }] => match &value.kind {
                ExprKind::Str(text) => Some((text.clone(), value.span)),
                _ => None,
            },
            _ => None,
        };
        let Some((written, span)) = literal else {
            self.error(
                call.span,
                "import takes a path or a library name written in the code, as a String literal: a program does not import while it runs",
            );
            self.check_arguments(args);
            return Ty::Error;
        };
        if !self.fns.is_empty() || self.scopes.depth() > 0 {
            self.error(
                call.span,
                "import is declared at the top level of a file, above its first use",
            );
            return Ty::Error;
        }
        let Some(from) = self.file.clone() else {
            self.error(
                call.span,
                "import needs the place of this file on disk to find the other one",
            );
            return Ty::Error;
        };
        let path = match self.resolve_import(&from, &written) {
            Ok(path) => path,
            Err(message) => {
                self.error(span, message);
                return Ty::Error;
            }
        };
        match self.load_module(&path, &written, &from, span) {
            Some(id) => Ty::Module(id),
            None => Ty::Error,
        }
    }

    /// The file a path or a library name stands for, or why there is none.
    fn resolve_import(&self, from: &Path, written: &str) -> Result<PathBuf, String> {
        let dir = from.parent().unwrap_or(Path::new("."));
        if written.starts_with("./") || written.starts_with("../") {
            let path = dir.join(format!("{written}.raga"));
            return match path.canonicalize() {
                Ok(path) if path.is_file() => Ok(path),
                _ => Err(format!("there is no file {written}.raga")),
            };
        }
        // a library: the project is the nearest folder above that has raga.project or a libraries folder
        let project = dir
            .ancestors()
            .find(|d| d.join("raga.project").is_file() || d.join("libraries").is_dir());
        if let Some(project) = project {
            if let Ok(text) = std::fs::read_to_string(project.join("raga.project"))
                && let Some((_, target)) =
                    project_names(&text).into_iter().find(|(n, _)| n == written)
            {
                let path = project.join(format!("{target}.raga"));
                if let Ok(path) = path.canonicalize()
                    && path.is_file()
                {
                    return Ok(path);
                }
                return Err(format!(
                    "raga.project gives {written} the file {target}.raga, and there is none"
                ));
            }
            for candidate in [
                project.join(format!("libraries/{written}.raga")),
                project.join(format!("libraries/{written}/{written}.raga")),
            ] {
                if let Ok(path) = candidate.canonicalize()
                    && path.is_file()
                {
                    return Ok(path);
                }
            }
        }
        if let Some(home) = std::env::var_os("RAGA_HOME") {
            let path = Path::new(&home).join(format!("libraries/{written}.raga"));
            if let Ok(path) = path.canonicalize()
                && path.is_file()
            {
                return Ok(path);
            }
        }
        Err(format!(
            "there is no library {written}: looked in raga.project, the libraries folder and the standard location"
        ))
    }

    /// Reads, checks and remembers the file at `path`, once; the number of its module.
    fn load_module(
        &mut self,
        path: &Path,
        written: &str,
        from: &Path,
        span: Span,
    ) -> Option<usize> {
        if let Some(id) = self.loaded.get(path) {
            return Some(*id);
        }
        if let Some(position) = self.loading.iter().position(|p| p == path) {
            let first = stem(&self.loading[position]);
            let second = stem(self.loading.last().map_or(from, |p| p.as_path()));
            self.error(
                span,
                format!("{first} and {second} import each other: modules cannot import each other in a circle"),
            );
            return None;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            self.error(span, format!("cannot read {}", path.display()));
            return None;
        };
        let lexed = lex(&text);
        let parsed = parse(&text, &lexed.tokens);

        let saved = self.begin_file(path, &parsed.program.stmts);
        self.loading.push(path.to_path_buf());
        self.check_exports_layout(&parsed.program.stmts);
        self.statements(&parsed.program.stmts);
        self.loading.pop();
        let exports = self.collect_exports();
        let mut diagnostics = lexed.diagnostics;
        diagnostics.extend(parsed.diagnostics);
        diagnostics.extend(std::mem::take(&mut self.diagnostics));
        self.end_file(saved);

        let id = self.modules.len();
        self.modules.push(ModuleInfo {
            written: written.to_string(),
            exports,
        });
        self.loaded.insert(path.to_path_buf(), id);
        self.reports.push(ModuleReport {
            path: path.to_path_buf(),
            text,
            diagnostics,
        });
        Some(id)
    }

    /// Sets the checker up for another file and returns what it had for the one it leaves.
    pub(super) fn begin_file(&mut self, path: &Path, stmts: &[Stmt]) -> FileState {
        FileState {
            scopes: std::mem::replace(&mut self.scopes, Scopes::new(declared_names(stmts))),
            diagnostics: std::mem::take(&mut self.diagnostics),
            expr_types: std::mem::take(&mut self.expr_types),
            symbol_of: std::mem::take(&mut self.symbol_of),
            bare_calls: std::mem::take(&mut self.bare_calls),
            exhaustive: std::mem::take(&mut self.exhaustive),
            file: self.file.replace(path.to_path_buf()),
            exported: self.exported.take(),
        }
    }

    fn end_file(&mut self, saved: FileState) {
        self.scopes = saved.scopes;
        self.diagnostics = saved.diagnostics;
        self.expr_types = saved.expr_types;
        self.symbol_of = saved.symbol_of;
        self.bare_calls = saved.bare_calls;
        self.exhaustive = saved.exhaustive;
        self.file = saved.file;
        self.exported = saved.exported;
    }

    // ---- export

    /// At most one `export` in a file, and it is the last statement.
    pub(super) fn check_exports_layout(&mut self, stmts: &[Stmt]) {
        let exports: Vec<usize> = stmts
            .iter()
            .enumerate()
            .filter(|(_, s)| is_export(s))
            .map(|(i, _)| i)
            .collect();
        if exports.len() > 1 {
            self.error(stmts[exports[1]].span, "a file has at most one export");
        }
        if let Some(&first) = exports.first()
            && first != stmts.len() - 1
            && exports.len() == 1
        {
            self.error(
                stmts[first].span,
                "export is the last statement of the file",
            );
        }
    }

    /// `export { pi, Point, circle_area }`.
    pub(super) fn export_call(
        &mut self,
        call: &Expr,
        args: &[Arg],
        trailer: Option<&Trailer>,
    ) -> Ty {
        let Some(trailer) = trailer.filter(|_| args.is_empty()) else {
            self.error(
                call.span,
                "export takes a block of names: write export { a, b }",
            );
            return Ty::Void;
        };
        if !self.fns.is_empty() || self.scopes.depth() > 0 {
            self.error(
                call.span,
                "export is the last statement of the file, at the top level",
            );
            return Ty::Void;
        }
        // the names are one to a line, or an Array block with commas
        let mut names: Vec<&Expr> = Vec::new();
        for stmt in &trailer.body {
            match &stmt.kind {
                StmtKind::Expr(e) => match &e.kind {
                    ExprKind::Array(items) => names.extend(items.iter()),
                    _ => names.push(e),
                },
                _ => self.error(stmt.span, "export lists names: export { a, b }"),
            }
        }
        let mut done: Vec<(Ident, super::scope::SymbolId)> = Vec::new();
        for item in names {
            let ExprKind::Name(name) = &item.kind else {
                self.error(
                    item.span,
                    "export takes names, not values: export { pi } shares the name pi",
                );
                continue;
            };
            let Some(symbol) = self.scopes.lookup(&name.name) else {
                let message = if self.scopes.declared_below(&name.name) {
                    format!(
                        "{} is declared below the export, the export is the last statement",
                        name.name
                    )
                } else {
                    format!("{} is not declared in this file", name.name)
                };
                self.error(name.span, message);
                continue;
            };
            if done.iter().any(|(n, _)| n.name == name.name) {
                self.error(name.span, format!("{} is exported twice", name.name));
                continue;
            }
            self.symbol_of.insert(item.id, symbol);
            done.push((name.clone(), symbol));
        }
        self.exported = Some(done);
        Ty::Void
    }

    fn collect_exports(&mut self) -> Vec<Export> {
        let done = self.exported.take().unwrap_or_default();
        done.into_iter()
            .map(|(name, symbol)| {
                let s: &Symbol = self.scopes.symbol(symbol);
                Export {
                    name: name.name,
                    ty: s.ty.clone(),
                    defines: s.defines.clone(),
                }
            })
            .collect()
    }

    // ---- using a module

    fn export_of(&self, module: usize, name: &str) -> Option<&Export> {
        self.modules[module].exports.iter().find(|e| e.name == name)
    }

    /// The export `name` of a module, or an error that says it is not shared.
    pub(super) fn module_export(&mut self, module: usize, name: &Ident) -> Option<Export> {
        match self.export_of(module, &name.name) {
            Some(export) => Some(export.clone()),
            None => {
                let written = self.modules[module].written.clone();
                self.error(
                    name.span,
                    format!("{} is not exported by {written}", name.name),
                );
                None
            }
        }
    }

    /// `shapes.pi`, `shapes.circle_area` and `shapes.Point` used as values.
    pub(super) fn module_value(&mut self, module: usize, name: &Ident) -> Ty {
        let Some(export) = self.module_export(module, name) else {
            return Ty::Error;
        };
        match (&export.defines, &export.ty) {
            (Some(Defines::Type(id)), _) => self.type_value_ty(*id, Vec::new()),
            (Some(Defines::Alias(ty)), _) => ty.clone(),
            (_, ty) => ty.clone(),
        }
    }

    /// Is `shapes.Point`, written where a value is, the name of a type of a module?
    pub(super) fn module_names_a_type(&self, object: &Expr, name: &Ident) -> bool {
        let ExprKind::Name(module) = &object.kind else {
            return false;
        };
        let Some(symbol) = self.scopes.lookup(&module.name) else {
            return false;
        };
        let Ty::Module(id) = self.scopes.symbol(symbol).ty else {
            return false;
        };
        self.export_of(id, &name.name)
            .is_some_and(|e| matches!(e.defines, Some(Defines::Type(_) | Defines::Alias(_))))
    }

    /// `shapes.circle_area(2.0)` and `shapes.Point(x = 0.0, y = 0.0)`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn module_call(
        &mut self,
        call: &Expr,
        module: usize,
        name: &Ident,
        type_args: &[crate::ast::Type],
        args: &[Arg],
        trailer: Option<&Trailer>,
        expected: Option<&Ty>,
    ) -> Ty {
        let Some(export) = self.module_export(module, name) else {
            self.check_arguments(args);
            return Ty::Error;
        };
        match (&export.defines, &export.ty) {
            (Some(Defines::Type(id)), _) => {
                let explicit: Vec<Ty> = type_args.iter().map(|t| self.resolve_type(t)).collect();
                self.construct_defined(call, *id, explicit, args, trailer, expected)
            }
            (_, Ty::Function(f)) => {
                let label = format!("{}.{}", self.modules[module].written, name.name);
                self.call_function(call, &label, f, type_args, args, trailer)
            }
            (_, Ty::Error) => {
                self.check_arguments(args);
                Ty::Error
            }
            (_, other) => {
                self.error(
                    name.span,
                    format!("`{}` is a {other}, not a function", name.name),
                );
                self.check_arguments(args);
                Ty::Error
            }
        }
    }

    /// The type that `shapes.Point` names, when written where a type is.
    pub(super) fn module_type(&mut self, written: &crate::ast::Type, args: Vec<Ty>) -> Ty {
        let (module_name, member) = written.name.name.split_once('.').expect("a dotted name");
        let ident = Ident {
            name: member.to_string(),
            span: written.span,
        };
        let module = self
            .scopes
            .lookup(module_name)
            .map(|s| self.scopes.symbol(s).ty.clone());
        let Some(Ty::Module(module)) = module else {
            self.error(written.span, format!("`{module_name}` is not a module"));
            return Ty::Error;
        };
        let Some(export) = self.module_export(module, &ident) else {
            return Ty::Error;
        };
        match export.defines {
            Some(Defines::Type(id)) => {
                let expected = self.def(id).generics.len();
                if args.len() != expected {
                    self.error(
                        written.span,
                        format!("`{}` takes {expected} type arguments", written.name.name),
                    );
                    return Ty::Error;
                }
                self.defined_ty(id, args)
            }
            Some(Defines::Alias(ty)) => ty,
            _ => {
                self.error(written.span, format!("{} is not a type", written.name.name));
                Ty::Error
            }
        }
    }
}

fn is_export(stmt: &Stmt) -> bool {
    matches!(&stmt.kind, StmtKind::Expr(e)
        if matches!(&e.kind, ExprKind::Call { callee, .. }
            if matches!(&callee.kind, ExprKind::Name(n) if n.name == "export")))
}
