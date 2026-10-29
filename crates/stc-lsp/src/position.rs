//! UTF-16 LSP positions and their byte-offset mapping.
//!
//! Compiler positions are 1-based and count Unicode scalar values; LSP
//! positions are 0-based and count UTF-16 code units by default. This module
//! is the only place that converts between the two (WL-1.1 §5.4). It is tested
//! with non-ASCII text, including astral characters.

use serde::{Deserialize, Serialize};

/// An LSP position: a 0-based line and a 0-based UTF-16 code-unit column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-16 code-unit offset in the line.
    pub character: u32,
}

/// An LSP half-open range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    /// Inclusive start.
    pub start: Position,
    /// Exclusive end.
    pub end: Position,
}

impl Range {
    /// Creates a range.
    #[must_use]
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// Maps byte offsets to LSP positions and back over one text.
///
/// Line breaks follow `stc-span`: `\r\n`, `\r`, and `\n` each start a new
/// line, and a leading UTF-8 BOM is not part of the first line's text.
#[derive(Clone, Debug)]
pub struct PositionMap<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> PositionMap<'a> {
    /// Builds a map over `text`.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let bom_len = if text.starts_with('\u{feff}') { 3 } else { 0 };
        let bytes = text.as_bytes();
        let mut line_starts = vec![bom_len];
        let mut index = bom_len;
        while index < bytes.len() {
            match bytes[index] {
                b'\r' => {
                    index += if bytes.get(index + 1) == Some(&b'\n') {
                        2
                    } else {
                        1
                    };
                    line_starts.push(index);
                }
                b'\n' => {
                    index += 1;
                    line_starts.push(index);
                }
                _ => index += 1,
            }
        }
        Self { text, line_starts }
    }

    /// Returns the mapped text.
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// Returns the number of lines; a text always has at least one.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        u32::try_from(self.line_starts.len()).expect("line count fits in u32")
    }

    /// Returns a line's text without its terminator.
    #[must_use]
    pub fn line_text(&self, line: u32) -> Option<&'a str> {
        let index = usize::try_from(line).ok()?;
        let start = *self.line_starts.get(index)?;
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut text = self.text.get(start..end)?;
        if let Some(stripped) = text.strip_suffix("\r\n") {
            text = stripped;
        } else if let Some(stripped) = text.strip_suffix('\n').or_else(|| text.strip_suffix('\r')) {
            text = stripped;
        }
        Some(text)
    }

    /// Converts a byte offset to an LSP position.
    ///
    /// Offsets are clamped to the text length and floored to a UTF-8 scalar
    /// boundary. A byte inside a line terminator maps to the end of that
    /// line's content.
    #[must_use]
    pub fn position(&self, offset: usize) -> Position {
        let offset = floor_char_boundary(self.text, offset.min(self.text.len()));
        let line_index = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let (line_start, content_end) = self.line_bounds(line_index);
        let offset = offset.clamp(line_start, content_end);
        let character = if offset <= line_start {
            0
        } else {
            u32::try_from(self.text[line_start..offset].encode_utf16().count())
                .expect("UTF-16 column fits in u32")
        };
        Position {
            line: u32::try_from(line_index).expect("line index fits in u32"),
            character,
        }
    }

    /// Converts an LSP position to a byte offset.
    ///
    /// Positions beyond the text are clamped to the end of the text. A
    /// character offset inside a surrogate pair is floored to the start of the
    /// character, so the result is always a valid byte offset.
    #[must_use]
    pub fn offset(&self, position: Position) -> usize {
        let Ok(line) = usize::try_from(position.line) else {
            return self.text.len();
        };
        if line >= self.line_starts.len() {
            return self.text.len();
        }
        let (line_start, content_end) = self.line_bounds(line);
        let line_text = &self.text[line_start..content_end];
        let mut utf16 = 0_u32;
        for (byte, character) in line_text.char_indices() {
            let width = u32::try_from(character.len_utf16()).expect("UTF-16 width fits");
            if utf16 + width > position.character {
                // The position falls inside this character (inside a
                // surrogate pair for astral scalars): floor it.
                return line_start + byte;
            }
            utf16 += width;
            if utf16 >= position.character {
                return line_start + byte + character.len_utf8();
            }
        }
        content_end
    }

    /// Returns a line's start offset and the end of its content (the line
    /// terminator excluded).
    fn line_bounds(&self, index: usize) -> (usize, usize) {
        let start = self.line_starts[index];
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut content_end = end;
        if let Some(stripped) = self.text[start..end].strip_suffix("\r\n") {
            content_end = start + stripped.len();
        } else if let Some(stripped) = self.text[start..end]
            .strip_suffix('\n')
            .or_else(|| self.text[start..end].strip_suffix('\r'))
        {
            content_end = start + stripped.len();
        }
        (start, content_end)
    }

    /// Converts a 1-based Unicode-scalar column to a 0-based UTF-16 column.
    #[must_use]
    pub fn utf16_column(line: &str, scalar_column: u32) -> u32 {
        line.chars()
            .take(usize::try_from(scalar_column.saturating_sub(1)).unwrap_or(usize::MAX))
            .map(|character| u32::try_from(character.len_utf16()).expect("UTF-16 width fits"))
            .sum()
    }
}

/// Returns a 1-based Unicode-scalar column as a 0-based UTF-16 column.
#[must_use]
pub fn utf16_column(line: &str, scalar_column: u32) -> u32 {
    PositionMap::utf16_column(line, scalar_column)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "aé😀z\nsecond\r\nthird";

    #[test]
    fn maps_byte_offsets_to_utf16_positions() {
        let map = PositionMap::new(TEXT);
        assert_eq!(
            map.position(0),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            map.position(1),
            Position {
                line: 0,
                character: 1
            }
        );
        // `é` is one UTF-16 unit; `😀` is two.
        assert_eq!(
            map.position(3),
            Position {
                line: 0,
                character: 2
            }
        );
        assert_eq!(
            map.position(7),
            Position {
                line: 0,
                character: 4
            }
        );
        assert_eq!(
            map.position(8),
            Position {
                line: 0,
                character: 5
            }
        );
        assert_eq!(
            map.position(9),
            Position {
                line: 1,
                character: 0
            }
        );
        // The CRLF pair starts line 2, not line 1's end.
        assert_eq!(
            map.position(17),
            Position {
                line: 2,
                character: 0
            }
        );
    }

    #[test]
    fn maps_utf16_positions_to_byte_offsets() {
        let map = PositionMap::new(TEXT);
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 0
            }),
            0
        );
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 1
            }),
            1
        );
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 2
            }),
            3
        );
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 4
            }),
            7
        );
        // Inside the emoji surrogate pair: floored to the scalar start.
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 3
            }),
            3
        );
        assert_eq!(
            map.offset(Position {
                line: 1,
                character: 3
            }),
            12
        );
        assert_eq!(
            map.offset(Position {
                line: 2,
                character: 0
            }),
            17
        );
        // Out-of-range positions clamp to the text boundaries.
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 99
            }),
            "aé😀z".len()
        );
        assert_eq!(
            map.offset(Position {
                line: 99,
                character: 0
            }),
            TEXT.len()
        );
    }

    #[test]
    fn round_trips_every_scalar_boundary() {
        let map = PositionMap::new(TEXT);
        for (offset, _) in TEXT.char_indices() {
            // The LF of a CRLF pair has no LSP position of its own: both
            // bytes of the terminator map to the end of the line's content.
            if offset > 0
                && TEXT.as_bytes()[offset - 1] == b'\r'
                && TEXT.as_bytes()[offset] == b'\n'
            {
                continue;
            }
            let position = map.position(offset);
            assert_eq!(map.offset(position), offset, "offset {offset}");
        }
        assert_eq!(map.offset(map.position(TEXT.len())), TEXT.len());
    }

    #[test]
    fn terminators_map_to_the_line_content_end() {
        let map = PositionMap::new("ab\r\ncd");
        assert_eq!(
            map.position(3),
            Position {
                line: 0,
                character: 2
            }
        );
        assert_eq!(
            map.position(4),
            Position {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            map.offset(Position {
                line: 0,
                character: 2
            }),
            2
        );
    }

    #[test]
    fn converts_scalar_columns_to_utf16_columns() {
        assert_eq!(utf16_column("aé😀z", 1), 0);
        assert_eq!(utf16_column("aé😀z", 2), 1);
        assert_eq!(utf16_column("aé😀z", 3), 2);
        assert_eq!(utf16_column("aé😀z", 4), 4);
        assert_eq!(utf16_column("aé😀z", 99), 5);
    }

    #[test]
    fn handles_a_bom_like_the_source_map() {
        let map = PositionMap::new("\u{feff}ab");
        assert_eq!(
            map.position(0),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            map.position(3),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            map.position(4),
            Position {
                line: 0,
                character: 1
            }
        );
    }
}
