use std::fmt;

use crate::{FileId, Span};

/// A 1-based line and column position in a source file.
///
/// The column counts Unicode scalar values from 1, not bytes (STC-LEX-005).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    /// The file containing the position.
    pub file: FileId,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column, counted in Unicode scalar values.
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One source file and its line index.
#[derive(Clone, Debug)]
pub struct SourceFile {
    id: FileId,
    path: String,
    text: String,
    line_starts: Vec<u32>,
    bom_len: u32,
}

impl SourceFile {
    fn new(id: FileId, path: String, text: String) -> Self {
        let bom_len = if text.starts_with('\u{feff}') { 3 } else { 0 };
        let line_starts = compute_line_starts(&text, bom_len);
        Self {
            id,
            path,
            text,
            line_starts,
            bom_len,
        }
    }

    /// Returns this file's identifier.
    #[must_use]
    pub fn id(&self) -> FileId {
        self.id
    }

    /// Returns the display path of this file.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the file contents.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the byte length of a UTF-8 BOM at the start of the file, or 0.
    #[must_use]
    pub fn bom_len(&self) -> u32 {
        self.bom_len
    }

    /// Returns the number of lines. A file always has at least one line.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Returns the byte offset where a 1-based line starts.
    #[must_use]
    pub fn line_start(&self, line: u32) -> Option<u32> {
        let index = line.checked_sub(1)? as usize;
        self.line_starts.get(index).copied()
    }

    /// Returns a line's text without its line terminator.
    #[must_use]
    pub fn line_text(&self, line: u32) -> Option<&str> {
        let index = line.checked_sub(1)? as usize;
        let start = *self.line_starts.get(index)? as usize;
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .map_or(self.text.len(), |offset| offset as usize);
        let mut line_text = self.text.get(start..end)?;
        if let Some(stripped) = line_text.strip_suffix("\r\n") {
            line_text = stripped;
        } else if let Some(stripped) = line_text
            .strip_suffix('\n')
            .or_else(|| line_text.strip_suffix('\r'))
        {
            line_text = stripped;
        }
        Some(line_text)
    }

    /// Converts a byte offset to a 1-based line/column position.
    ///
    /// Offsets beyond the end of the file are clamped. Offsets that fall inside
    /// a UTF-8 scalar are floored to the start of that scalar.
    #[must_use]
    pub fn location(&self, offset: u32) -> Location {
        let offset = floor_char_boundary(&self.text, (offset as usize).min(self.text.len()));
        let line_index = self
            .line_starts
            .partition_point(|&start| start as usize <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index] as usize;
        let column = if offset <= line_start {
            1
        } else {
            self.text[line_start..offset].chars().count() as u32 + 1
        };
        Location {
            file: self.id,
            line: line_index as u32 + 1,
            column,
        }
    }
}

/// Owns source files and answers span/position queries.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// Creates an empty source map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file and returns its identifier.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` files are added or if the file text is
    /// longer than `u32::MAX` bytes.
    pub fn add_file(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        let raw = u32::try_from(self.files.len()).expect("source map file limit exceeded");
        let text = text.into();
        assert!(
            u32::try_from(text.len()).is_ok(),
            "source file exceeds the 4 GiB span limit"
        );
        self.files
            .push(SourceFile::new(FileId::new(raw), path.into(), text));
        FileId::new(raw)
    }

    /// Adds a file with a synthetic name.
    ///
    /// # Panics
    ///
    /// See [`SourceMap::add_file`].
    pub fn add_virtual_file(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let name = name.into();
        self.add_file(format!("<{name}>"), text)
    }

    /// Returns a file by identifier.
    #[must_use]
    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.index())
    }

    /// Returns the number of files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns `true` if no files have been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Returns the display path of a file.
    #[must_use]
    pub fn file_path(&self, id: FileId) -> Option<&str> {
        self.file(id).map(SourceFile::path)
    }

    /// Returns the source text covered by a span.
    ///
    /// Returns `""` for an unknown file or a span that is not on UTF-8
    /// boundaries.
    #[must_use]
    pub fn text(&self, span: Span) -> &str {
        self.file(span.file)
            .and_then(|file| file.text.get(span.start as usize..span.end as usize))
            .unwrap_or("")
    }

    /// Converts a span's start to a line/column position.
    ///
    /// Returns line 1, column 1 for an unknown file rather than panicking on an
    /// internal identifier bug.
    #[must_use]
    pub fn location(&self, span: Span) -> Location {
        self.file(span.file).map_or(
            Location {
                file: span.file,
                line: 1,
                column: 1,
            },
            |file| {
                let mut location = file.location(span.start);
                location.file = span.file;
                location
            },
        )
    }

    /// Returns the text of the line containing the span's start.
    #[must_use]
    pub fn line_snippet(&self, span: Span) -> &str {
        let Some(file) = self.file(span.file) else {
            return "";
        };
        file.line_text(self.location(span).line).unwrap_or("")
    }
}

fn compute_line_starts(text: &str, bom_len: u32) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut starts = vec![bom_len];
    let mut index = bom_len as usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                index += if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                starts.push(index as u32);
            }
            b'\n' => {
                index += 1;
                starts.push(index as u32);
            }
            _ => index += 1,
        }
    }
    starts
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
