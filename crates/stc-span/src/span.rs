use std::fmt;

/// Identifies a source file owned by a [`crate::SourceMap`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    /// Creates a file identifier from its raw value.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns the identifier as a slice index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileId({})", self.0)
    }
}

/// A half-open byte range `start..end` in a source file.
///
/// Spans are ordered by file, then by start offset, then by end offset.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    /// The file this span belongs to.
    pub file: FileId,
    /// Inclusive byte offset.
    pub start: u32,
    /// Exclusive byte offset.
    pub end: u32,
}

impl Span {
    /// Creates a span. The caller must ensure `start <= end`.
    #[must_use]
    pub const fn new(file: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        Self { file, start, end }
    }

    /// Creates a zero-width span at `offset`.
    #[must_use]
    pub const fn empty(file: FileId, offset: u32) -> Self {
        Self {
            file,
            start: offset,
            end: offset,
        }
    }

    /// Returns the length in bytes.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` if the span is zero-width.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns `true` if `offset` is inside the half-open span.
    #[must_use]
    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns `true` if `other` lies entirely inside this span.
    #[must_use]
    pub const fn contains_span(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Returns the smallest span covering both spans.
    ///
    /// # Panics
    ///
    /// Panics if the spans belong to different files.
    #[must_use]
    pub fn join(self, other: Self) -> Self {
        assert_eq!(
            self.file, other.file,
            "cannot join spans from different files"
        );
        Self {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}
