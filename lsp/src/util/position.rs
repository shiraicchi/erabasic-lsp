//! Position conversion utilities for LSP ↔ AST mapping.

use crate::parser::ast::Span;
use lsp_types::{Position, Range};

/// Text encoding used for position tracking.
/// LSP 3.17+ supports UTF-8, UTF-16, and UTF-32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    Utf8 = 0,
    #[default]
    Utf16 = 1,
    Utf32 = 2,
}

/// Convert a byte offset to an LSP Position (line/character).
pub fn offset_to_position(text: &str, offset: usize, encoding: PositionEncoding) -> Position {
    let offset = offset.min(text.len());

    match encoding {
        PositionEncoding::Utf8 => {
            let before = &text[..offset];
            let line = before.matches('\n').count() as u32;
            let last_newline = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let character = offset - last_newline;
            Position::new(line, character as u32)
        }
        PositionEncoding::Utf16 => {
            // Count UTF-16 code units
            let before = &text[..offset];
            let line = before.matches('\n').count() as u32;
            let last_newline = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let before_line = &text[last_newline..offset];
            let character = before_line.encode_utf16().count() as u32;
            Position::new(line, character)
        }
        PositionEncoding::Utf32 => {
            let before = &text[..offset];
            let line = before.matches('\n').count() as u32;
            let last_newline = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let before_line = &text[last_newline..offset];
            let character = before_line.chars().count() as u32;
            Position::new(line, character)
        }
    }
}

/// Convert an LSP Position to a byte offset.
pub fn position_to_offset(text: &str, position: Position, encoding: PositionEncoding) -> usize {
    let line = position.line as usize;
    let character = position.character as usize;

    // Find the start of the requested line
    let mut line_start = 0;
    let mut current_line = 0;

    for (i, c) in text.char_indices() {
        if current_line == line {
            break;
        }
        if c == '\n' {
            current_line += 1;
            line_start = i + 1;
        }
    }

    // Now advance by character count based on encoding
    match encoding {
        PositionEncoding::Utf8 => {
            let line_text = &text[line_start..];
            let mut byte_offset = 0;
            for (char_count, (i, c)) in line_text.char_indices().enumerate() {
                if char_count >= character {
                    break;
                }
                byte_offset = i + c.len_utf8();
            }
            line_start + byte_offset
        }
        PositionEncoding::Utf16 => {
            let line_text = &text[line_start..];
            let mut utf16_count = 0;
            let mut byte_offset = 0;
            for c in line_text.chars() {
                let units = c.len_utf16();
                if utf16_count + units > character {
                    break;
                }
                utf16_count += units;
                byte_offset += c.len_utf8();
            }
            line_start + byte_offset
        }
        PositionEncoding::Utf32 => {
            let line_text = &text[line_start..];
            let mut byte_offset = 0;
            for (char_count, c) in line_text.chars().enumerate() {
                if char_count >= character {
                    break;
                }
                byte_offset += c.len_utf8();
            }
            line_start + byte_offset
        }
    }
}

/// Convert a Span to an LSP Range.
pub fn span_to_range(text: &str, span: impl SpanLike, encoding: PositionEncoding) -> Range {
    let start = offset_to_position(text, span.start(), encoding);
    let end = offset_to_position(text, span.end(), encoding);
    Range::new(start, end)
}

/// Trait for types that can provide start/end byte offsets.
pub trait SpanLike {
    fn start(&self) -> usize;
    fn end(&self) -> usize;
}

/// Convert an LSP Range to a Span.
pub fn range_to_span(text: &str, range: Range, encoding: PositionEncoding) -> Span {
    let start = position_to_offset(text, range.start, encoding);
    let end = position_to_offset(text, range.end, encoding);
    Span::new(start, end)
}

/// Line/column position within a file (1-indexed line, 0-indexed column for LSP compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

impl TextPosition {
    pub fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

/// A text range with start and end positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

impl TextRange {
    pub fn new(start: TextPosition, end: TextPosition) -> Self {
        Self { start, end }
    }

    pub fn single_line(line: u32, start_col: u32, end_col: u32) -> Self {
        Self {
            start: TextPosition::new(line, start_col),
            end: TextPosition::new(line, end_col),
        }
    }
}

/// Convert from LSP Position to our internal TextPosition (0-indexed).
pub fn lsp_to_text_position(pos: Position) -> TextPosition {
    TextPosition::new(pos.line, pos.character)
}

/// Convert from our internal TextPosition to LSP Position.
pub fn text_position_to_lsp(pos: TextPosition) -> Position {
    Position::new(pos.line, pos.column)
}

/// Convert from LSP Range to our internal TextRange.
pub fn lsp_to_text_range(range: Range) -> TextRange {
    TextRange::new(
        lsp_to_text_position(range.start),
        lsp_to_text_position(range.end),
    )
}

/// Convert from our internal TextRange to LSP Range.
pub fn text_range_to_lsp(range: TextRange) -> Range {
    Range::new(
        text_position_to_lsp(range.start),
        text_position_to_lsp(range.end),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_position_conversion() {
        let text = "hello\nworld";
        let pos = offset_to_position(text, 6, PositionEncoding::Utf8); // 'w'
        assert_eq!(pos, Position::new(1, 0));

        let offset = position_to_offset(text, pos, PositionEncoding::Utf8);
        assert_eq!(offset, 6);
    }

    #[test]
    fn test_utf16_position_conversion() {
        let text = "hello\n世界";
        let pos = offset_to_position(text, 6, PositionEncoding::Utf16); // '世'
        assert_eq!(pos, Position::new(1, 0));

        // After '世' (1 UTF-16 unit, since U+4E16 is in BMP)
        let pos2 = offset_to_position(text, 6 + 3, PositionEncoding::Utf16); // '界'
        assert_eq!(pos2, Position::new(1, 1));
    }

    #[test]
    fn test_span_to_range() {
        let text = "hello\nworld";
        let span = Span::new(0, 5); // "hello"
        let range = span_to_range(text, span, PositionEncoding::Utf8);
        assert_eq!(range.start, Position::new(0, 0));
        assert_eq!(range.end, Position::new(0, 5));
    }

    #[test]
    fn test_multiline_span() {
        let text = "hello\nworld";
        let span = Span::new(3, 8); // "lo\nwo" (bytes 3-7)
        let range = span_to_range(text, span, PositionEncoding::Utf8);
        assert_eq!(range.start, Position::new(0, 3));
        assert_eq!(range.end, Position::new(1, 2));
    }
}
