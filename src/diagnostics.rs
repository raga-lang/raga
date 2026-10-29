//! Errors and warnings found by any stage, and how they are shown.

use crate::source::SourceFile;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One problem, with the text it is about. A stage collects these in a list and keeps going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span,
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            message: message.into(),
            span,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The message with the file, the position, the line of source and a marker under the text:
    ///
    /// ```text
    /// error: unterminated string
    ///  --> hello.raga:3:11
    ///   |
    /// 3 | const s = "abc
    ///   |           ^^^^
    /// ```
    pub fn render(&self, file: &SourceFile) -> String {
        let (line, col) = file.line_col(self.span.start as usize);
        let label = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let text = file.line_text(line);
        let gutter = line.to_string().len();
        // the marker never runs past the end of its line, a span can cover several lines
        let room = text.chars().count().saturating_sub(col - 1).max(1);
        let width = file.text[self.span.start as usize..self.span.end as usize]
            .chars()
            .count()
            .clamp(1, room);
        format!(
            "{label}: {msg}\n{pad} --> {name}:{line}:{col}\n{pad} |\n{line} | {text}\n{pad} | {indent}{marks}\n",
            msg = self.message,
            pad = " ".repeat(gutter),
            name = file.name,
            indent = " ".repeat(col - 1),
            marks = "^".repeat(width),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_marker_under_the_text() {
        let file = SourceFile::new("t.raga", "const s = \"abc\n");
        let d = Diagnostic::error(Span::new(10, 14), "unterminated string");
        let shown = d.render(&file);
        assert!(shown.contains("error: unterminated string"));
        assert!(shown.contains("t.raga:1:11"));
        assert!(shown.contains("1 | const s = \"abc"));
        assert!(shown.contains("  |           ^^^^"));
    }
}
