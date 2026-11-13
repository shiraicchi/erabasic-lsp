//! Document management with incremental parsing support.

use crate::lsp_ast::{DimDecl, Program, SourceFile};
use crate::lsp_parser::parse_file;
use crate::util::encoding::{detect_encoding, era_lines, normalize_line_endings};
use crate::util::position::{offset_to_position, span_to_range, PositionEncoding, TextRange};
use lsp_types::{
    Position, Range, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    VersionedTextDocumentIdentifier,
};
use ropey::Rope;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// A document in the workspace.
#[derive(Debug)]
pub struct Document {
    /// The document URI.
    pub uri: lsp_types::Url,
    /// The document version.
    pub version: i32,
    /// The document text (using Rope for efficient editing).
    pub text: Rope,
    /// The detected encoding.
    pub encoding: crate::util::encoding::Encoding,
    /// The parsed AST (if parsing succeeded).
    pub ast: Option<Arc<ParsedDocument>>,
    /// Parse errors.
    pub parse_errors: Vec<ParseError>,
    /// Whether the document is open in the editor.
    pub is_open: bool,
    /// File path (for ERB/ERH files on disk).
    pub file_path: Option<PathBuf>,
    /// Whether this is an ERH (header) file.
    pub is_header: bool,
}

impl Document {
    /// Create a new document from text.
    pub fn new(uri: lsp_types::Url, version: i32, text: String) -> Self {
        let encoding = detect_encoding(text.as_bytes());
        let normalized = normalize_line_endings(&text);
        let rope = Rope::from_str(&normalized);

        let is_header = uri.path().ends_with(".erh") || uri.path().ends_with(".ERH");
        let file_path = uri.to_file_path().ok();

        Self {
            uri,
            version,
            text: rope,
            encoding,
            ast: None,
            parse_errors: Vec::new(),
            is_open: true,
            file_path,
            is_header,
        }
    }

    /// Create a document from a file on disk.
    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let (text, encoding) = crate::util::encoding::read_text_file(path)?;
        let uri = lsp_types::Url::from_file_path(path).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid file path")
        })?;
        let normalized = normalize_line_endings(&text);
        let rope = Rope::from_str(&normalized);

        let is_header = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("erh"));

        Ok(Self {
            uri,
            version: 0,
            text: rope,
            encoding,
            ast: None,
            parse_errors: Vec::new(),
            is_open: true,
            file_path: Some(path.to_path_buf()),
            is_header,
        })
    }

    /// Get the full text of the document.
    pub fn get_text(&self) -> String {
        self.text.to_string()
    }

    /// Get a line of text (0-indexed).
    pub fn get_line(&self, line: usize) -> Option<String> {
        self.text.get_line(line).map(|l| l.to_string())
    }

    /// Get the number of lines.
    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }

    /// Apply a content change to the document.
    pub fn apply_change(
        &mut self,
        change: &TextDocumentContentChangeEvent,
    ) -> Result<(), DocumentError> {
        match change.range {
            Some(range) => {
                let start = self.lsp_position_to_char(range.start)?;
                let end = self.lsp_position_to_char(range.end)?;
                self.text.remove(start..end);
                let text = &change.text;
                self.text.insert(start, text);
            }
            None => {
                // Full text replacement
                let text = &change.text;
                self.text = Rope::from_str(text);
            }
        }
        Ok(())
    }

    /// Convert an LSP position to a character index in the rope.
    pub fn lsp_position_to_char(&self, pos: Position) -> Result<usize, DocumentError> {
        let line = pos.line as usize;
        let character = pos.character as usize;

        if line >= self.text.len_lines() {
            return Err(DocumentError::PositionOutOfBounds);
        }

        let line_start = self.text.line_to_char(line);
        let line_text = self
            .text
            .get_line(line)
            .ok_or(DocumentError::PositionOutOfBounds)?
            .to_string();

        // LSP positions are UTF-16 code unit counts within the line
        // Convert to char count for ropey
        let char_count: usize = line_text.encode_utf16().take(character).count();

        Ok(line_start + char_count)
    }

    /// Convert a byte offset to an LSP position.
    pub fn offset_to_lsp_position(&self, offset: usize) -> Position {
        let text = self.get_text();
        offset_to_position(&text, offset, PositionEncoding::Utf16)
    }

    /// Get the text range for a span.
    pub fn span_to_range(&self, span: crate::lsp_ast::Span) -> Range {
        let text = self.get_text();
        span_to_range(&text, span, PositionEncoding::Utf16)
    }

    /// Get the text in a range.
    pub fn get_range_text(&self, range: Range) -> Option<String> {
        let start = self.lsp_position_to_char(range.start).ok()?;
        let end = self.lsp_position_to_char(range.end).ok()?;
        Some(self.text.slice(start..end).to_string())
    }

    /// Check if the document is an ERB file.
    pub fn is_erb(&self) -> bool {
        !self.is_header
    }

    /// Check if the document is an ERH file.
    pub fn is_erh(&self) -> bool {
        self.is_header
    }
}

/// A successfully parsed document with its AST.
#[derive(Debug)]
pub struct ParsedDocument {
    /// The parsed program (functions, global dims, etc.).
    pub program: Program,
    /// The source file info.
    pub source_file: SourceFile,
    /// The line mapping for the document.
    pub line_map: Vec<LineInfo>,
    /// Parse errors collected during parsing.
    pub parse_errors: Vec<ParseError>,
}

/// Information about a source line.
#[derive(Debug, Clone)]
pub struct LineInfo {
    /// The line number in the original file (1-indexed).
    pub original_line: u32,
    /// The byte offset of the line start.
    pub start_offset: usize,
    /// The byte offset of the line end.
    pub end_offset: usize,
}

/// A parse error with location information.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub range: Range,
    pub severity: ParseErrorSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorSeverity {
    Error,
    Warning,
}

/// Errors that can occur during document operations.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("Position out of bounds")]
    PositionOutOfBounds,
    #[error("Invalid UTF-8")]
    InvalidUtf8,
    #[error("Parse error: {0}")]
    ParseError(String),
}

/// Document manager for handling multiple documents with incremental parsing.
pub struct DocumentManager {
    documents: RwLock<std::collections::HashMap<lsp_types::Url, Arc<RwLock<Document>>>>,
    parsing_queue: tokio::sync::mpsc::UnboundedSender<lsp_types::Url>,
    workspace_index: std::sync::Weak<std::sync::Mutex<crate::analysis::index::WorkspaceIndex>>,
    reference_index:
        std::sync::Weak<std::sync::Mutex<crate::analysis::references::WorkspaceReferenceIndex>>,
    dialect: std::sync::Arc<std::sync::RwLock<crate::analysis::variant::Dialect>>,
}

impl DocumentManager {
    pub fn new(
        workspace_index: std::sync::Weak<std::sync::Mutex<crate::analysis::index::WorkspaceIndex>>,
        reference_index: std::sync::Weak<
            std::sync::Mutex<crate::analysis::references::WorkspaceReferenceIndex>,
        >,
        dialect: std::sync::Arc<std::sync::RwLock<crate::analysis::variant::Dialect>>,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<lsp_types::Url>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                documents: RwLock::new(std::collections::HashMap::new()),
                parsing_queue: tx,
                workspace_index,
                reference_index,
                dialect,
            },
            rx,
        )
    }

    /// Open a document.
    pub fn open(
        &self,
        uri: lsp_types::Url,
        version: i32,
        text: String,
    ) -> Result<(), DocumentError> {
        let doc = Arc::new(RwLock::new(Document::new(uri.clone(), version, text)));
        self.documents.write().unwrap().insert(uri.clone(), doc.clone());
        self.parse_and_store(uri, doc);
        Ok(())
    }

    /// Open a document from a file.
    pub fn open_file(&self, path: &Path) -> Result<(), DocumentError> {
        let doc = Arc::new(RwLock::new(
            Document::from_file(path).map_err(|e| DocumentError::ParseError(e.to_string()))?,
        ));
        let uri = {
            let doc_guard = doc.read().unwrap();
            doc_guard.uri.clone()
        };
        self.documents.write().unwrap().insert(uri.clone(), doc.clone());
        self.parse_and_store(uri, doc);
        Ok(())
    }

    /// Close a document.
    pub fn close(&self, uri: &lsp_types::Url) {
        self.documents.write().unwrap().remove(uri);
    }

    /// Apply a change to a document.
    pub fn change(
        &self,
        uri: &lsp_types::Url,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
    ) -> Result<(), DocumentError> {
        let docs = self.documents.read().unwrap();
        let Some(doc_arc) = docs.get(uri) else {
            return Err(DocumentError::ParseError("Document not found".into()));
        };
        let doc_arc = doc_arc.clone();
        drop(docs);

        {
            let mut doc = doc_arc.write().unwrap();
            doc.version = version;
            for change in changes {
                doc.apply_change(&change)?;
            }
        }

        // Re-parse
        self.parse_and_store(uri.clone(), doc_arc);

        Ok(())
    }

    /// Get a document (read-only).
    pub fn get(&self, uri: &lsp_types::Url) -> Option<Arc<RwLock<Document>>> {
        self.documents.read().unwrap().get(uri).cloned()
    }

    /// Get a document's parsed AST.
    pub fn get_parsed(&self, uri: &lsp_types::Url) -> Option<Arc<ParsedDocument>> {
        let docs = self.documents.read().unwrap();
        let doc_arc = docs.get(uri)?.clone();
        drop(docs);
        let doc_guard = doc_arc.read().unwrap();
        doc_guard.ast.clone()
    }

    /// Get all open documents.
    pub fn all_documents(&self) -> Vec<Arc<RwLock<Document>>> {
        self.documents.read().unwrap().values().cloned().collect()
    }

    /// Parse a document and store the result.
    fn parse_and_store(&self, uri: lsp_types::Url, doc: Arc<RwLock<Document>>) {
        let text = {
            let doc_guard = doc.read().unwrap();
            doc_guard.get_text()
        };
        let source_file = SourceFile {
            path: uri.path().to_string(),
            text: text.clone(),
        };

        // Parse the document using the eager LSP parser
        let program = parse_file(source_file.clone());

        // Build line map
        let line_map = build_line_map(&text);

        // Update the document with parsed result
        let mut doc_guard = doc.write().unwrap();
        doc_guard.parse_errors = program
            .errors
            .iter()
            .map(|e| ParseError {
                message: e.clone(),
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                severity: ParseErrorSeverity::Error,
            })
            .collect();
        let parse_errors = program
            .errors
            .iter()
            .map(|e| ParseError {
                message: e.clone(),
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                severity: ParseErrorSeverity::Error,
            })
            .collect();
        doc_guard.ast = Some(Arc::new(ParsedDocument {
            program,
            source_file,
            line_map,
            parse_errors,
        }));
    }
}

/// Build a line map from text.
fn build_line_map(text: &str) -> Vec<LineInfo> {
    let mut lines = Vec::new();
    let mut offset = 0;

    for (line_num, line) in era_lines(text).into_iter().enumerate() {
        let start = offset;
        offset += line.len();
        // Account for line ending
        let remaining = &text[offset..];
        let line_ending_len = if remaining.starts_with("\r\n") {
            2
        } else if remaining.starts_with('\n') || remaining.starts_with('\r') {
            1
        } else {
            0
        };
        offset += line_ending_len;

        lines.push(LineInfo {
            original_line: line_num as u32 + 1,
            start_offset: start,
            end_offset: offset,
        });
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Url;

    #[test]
    fn test_document_creation() {
        let uri = Url::parse("file:///test.erb").unwrap();
        let doc = Document::new(uri, 1, "PRINT \"hello\"\n".to_string());
        assert_eq!(doc.version, 1);
        assert!(doc.is_erb());
        assert!(!doc.is_erh());
    }

    #[test]
    fn test_document_change() {
        let uri = Url::parse("file:///test.erb").unwrap();
        let mut doc = Document::new(uri, 1, "PRINT \"hello\"\n".to_string());

        let change = TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 6), Position::new(0, 13))),
            range_length: None,
            text: "\"world\"".to_string(),
        };

        doc.apply_change(&change).unwrap();
        assert_eq!(doc.get_text(), "PRINT \"world\"\n");
    }

    #[test]
    fn test_line_map() {
        let text = "line1\nline2\nline3";
        let map = build_line_map(text);
        assert_eq!(map.len(), 3);
        assert_eq!(map[0].original_line, 1);
        assert_eq!(map[1].original_line, 2);
    }
}

#[cfg(test)]
mod test_doc_ast {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::references::WorkspaceReferenceIndex;
    use crate::analysis::variant::Dialect;
    use std::sync::{Arc, Mutex, RwLock};
    use lsp_types::Url;

    #[tokio::test]
    async fn test_document_ast() {
        let workspace_index = Arc::new(Mutex::new(WorkspaceIndex::new()));
        let reference_index = Arc::new(Mutex::new(WorkspaceReferenceIndex::new()));
        let dialect = Arc::new(RwLock::new(Dialect::Auto));
        
        let (doc_manager, _rx) = DocumentManager::new(
            Arc::downgrade(&workspace_index),
            Arc::downgrade(&reference_index),
            dialect.clone(),
        );
        
        let uri = Url::parse("file:///test.erb").unwrap();
        doc_manager.open(uri.clone(), 1, "@TEST\n#DIMS A\nIF A >= \"B\"\nENDIF\nENDFUNC\n".to_string()).unwrap();
        
        let parsed = doc_manager.get_parsed(&uri);
        assert!(parsed.is_some(), "Document should have AST");
        
        let parsed = parsed.unwrap();
        println!("Functions: {:?}", parsed.program.functions.keys().collect::<Vec<_>>());
        println!("Global dims: {:?}", parsed.program.global_dims.iter().map(|d| &d.name).collect::<Vec<_>>());
        println!("Errors: {:?}", parsed.program.errors);
        
        for (name, funcs) in &parsed.program.functions {
            for func in funcs {
                println!("Function: {} (line {})", name, func.line);
                println!("  Body code count: {}", func.body.code.len());
                for (i, instr) in func.body.code.iter().enumerate() {
                    println!("  [{}] {:?}", i, instr);
                }
            }
        }
    }
}
