//! Text encoding detection and handling for EraBasic files.

use std::path::Path;

/// Detect the encoding of a file (UTF-8 with/without BOM, or Shift_JIS).
pub fn detect_encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Encoding::Utf8WithBom;
    }

    match std::str::from_utf8(bytes) {
        Ok(_) => Encoding::Utf8,
        Err(_) => Encoding::ShiftJis,
    }
}

/// File encoding types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8WithBom,
    ShiftJis,
}

impl Encoding {
    /// Decode bytes to String using this encoding.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Encoding::Utf8WithBom => String::from_utf8_lossy(&bytes[3..]).into_owned(),
            Encoding::ShiftJis => {
                let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
                cow.into_owned()
            }
        }
    }

    /// Decode with explicit lossy handling.
    pub fn decode_lossy(self, bytes: &[u8]) -> String {
        self.decode(bytes)
    }

    /// Get the BOM bytes for this encoding (if any).
    pub fn bom(self) -> Option<&'static [u8]> {
        match self {
            Encoding::Utf8WithBom => Some(&[0xEF, 0xBB, 0xBF]),
            _ => None,
        }
    }

    /// Check if this encoding uses a BOM.
    pub fn has_bom(self) -> bool {
        matches!(self, Encoding::Utf8WithBom)
    }
}

/// Read a text file with automatic encoding detection.
pub fn read_text_file(path: &Path) -> std::io::Result<(String, Encoding)> {
    let bytes = std::fs::read(path)?;
    let encoding = detect_encoding(&bytes);
    Ok((encoding.decode(&bytes), encoding))
}

/// Write a text file with the specified encoding.
pub fn write_text_file(path: &Path, text: &str, encoding: Encoding) -> std::io::Result<()> {
    let bytes = match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8WithBom => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::ShiftJis => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(text);
            cow.into_owned()
        }
    };
    std::fs::write(path, bytes)
}

/// Get the encoding of a file by reading its first few bytes.
pub fn probe_file_encoding(path: &Path) -> std::io::Result<Encoding> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 4];
    let n = std::io::Read::read(&mut file, &mut buf)?;
    Ok(detect_encoding(&buf[..n]))
}

/// Normalize line endings to LF.
pub fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Split text into lines using EraBasic's line semantics (CRLF, LF, or lone CR).
pub fn era_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    let has_lone_cr = text
        .match_indices('\r')
        .any(|(i, _)| text.as_bytes().get(i + 1) != Some(&b'\n'));

    while !rest.is_empty() {
        let nl = rest.find('\n');
        let seg = &rest[..nl.unwrap_or(rest.len())];
        if !has_lone_cr {
            rest = nl.map_or("", |i| &rest[i + 1..]);
            lines.push(seg.strip_suffix('\r').unwrap_or(seg));
        } else {
            match seg.find('\r') {
                Some(j) if j + 1 == seg.len() && nl.is_some() => {
                    rest = &rest[j + 2..];
                    lines.push(&seg[..j]);
                }
                Some(j) => {
                    rest = &rest[j + 1..];
                    lines.push(&seg[..j]);
                }
                None => {
                    rest = nl.map_or("", |i| &rest[i + 1..]);
                    lines.push(seg);
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoding_detection() {
        assert_eq!(detect_encoding(b"hello"), Encoding::Utf8);
        assert_eq!(
            detect_encoding(&[0xEF, 0xBB, 0xBF, b'h']),
            Encoding::Utf8WithBom
        );
        // Shift_JIS "あ" = 0x82 0xA0
        assert_eq!(detect_encoding(&[0x82, 0xA0]), Encoding::ShiftJis);
    }

    #[test]
    fn test_era_lines() {
        let lines: Vec<&str> = era_lines("a\r\nb\rc\n\rd");
        assert_eq!(lines, ["a", "b", "c", "", "d"]);
    }
}
