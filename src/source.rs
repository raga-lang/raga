//! A source file and the arithmetic between byte offsets and line numbers.

/// The text of one file, with the offsets where each line starts.
///
/// The stages work with byte offsets because they are cheap and exact. A person wants a line and a
/// column, so they are worked out here, only when an error is shown.
pub struct SourceFile {
    pub name: String,
    pub text: String,
    /// Byte offset of the first character of each line. The first entry is always 0.
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            name: name.into(),
            text,
            line_starts,
        }
    }

    /// Line and column of a byte offset, both counted from 1. The column counts characters, not bytes.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self.text[start..offset].chars().count();
        (line + 1, col + 1)
    }

    /// The text of a line (1-based) without its line break.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_column_start_at_one() {
        let f = SourceFile::new("t.raga", "ab\ncd\n\nef");
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(1), (1, 2));
        assert_eq!(f.line_col(3), (2, 1));
        assert_eq!(f.line_col(7), (4, 1));
        assert_eq!(f.line_text(2), "cd");
        assert_eq!(f.line_text(4), "ef");
    }

    #[test]
    fn column_counts_characters_not_bytes() {
        let f = SourceFile::new("t.raga", "é = 1");
        // 'é' takes two bytes, so the '=' is at byte 3 but in column 3 (é, space, =)
        assert_eq!(f.line_col(3), (1, 3));
    }
}
