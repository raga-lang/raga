//! Positions in the source text.

/// A range of bytes in one source file: `start` is included, `end` is not.
///
/// Every token and every node of the tree carries a span, so an error can point at the exact text
/// that caused it. A span is two numbers, not a copy of the text: the text itself stays in the
/// [`SourceFile`](crate::source::SourceFile) and is read back with [`Span::slice`].
///
/// Offsets are `u32`, which limits a file to 4 GiB and keeps every token small.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        debug_assert!(start <= end);
        Span {
            start: u32::try_from(start).expect("a source file is larger than 4 GiB"),
            end: u32::try_from(end).expect("a source file is larger than 4 GiB"),
        }
    }

    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span that covers both. Used to give a node the range from its first to its last token.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// The text this span covers.
    pub fn slice(self, text: &str) -> &str {
        &text[self.start as usize..self.end as usize]
    }
}
