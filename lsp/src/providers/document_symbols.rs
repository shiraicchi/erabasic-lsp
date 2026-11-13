//! Document symbols provider for EraBasic LSP (outline view).

use crate::document::Document;
use crate::lsp_ast::{Body, DimDecl, Function, Program};
use crate::util::position::{span_to_range, PositionEncoding};
use lsp_types::{
    DocumentSymbol, DocumentSymbolParams, Position, Range, SymbolKind as LspSymbolKind,
};

/// Document symbols provider for EraBasic.
pub struct DocumentSymbolsProvider;

impl DocumentSymbolsProvider {
    pub fn new() -> Self {
        Self
    }

    /// Provide document symbols (outline) for a document.
    pub fn provide(
        &self,
        params: DocumentSymbolParams,
        document: &Document,
    ) -> Vec<DocumentSymbol> {
        let mut symbols = Vec::new();

        if let Some(ast) = &document.ast {
            // Add global variables
            for dim in &ast.program.global_dims {
                symbols.push(self.dim_to_symbol(dim, true, &document.get_text()));
            }

            // Add macros
            for (name, body) in &ast.program.defines {
                symbols.push(self.macro_to_symbol(name, body));
            }

            // Add functions
            for func_list in ast.program.functions.values() {
                for func in func_list {
                    symbols.push(self.function_to_symbol(func, &ast.line_map));
                }
            }
        }

        symbols
    }

    /// Convert a function to a document symbol.
    fn function_to_symbol(&self, func: &Function, line_map: &[crate::document::LineInfo]) -> DocumentSymbol {
        let body = &func.body;

        // Build children: labels and local variables
        let mut children = Vec::new();

        // Local variables
        for dim in &body.dims {
            children.push(self.dim_to_symbol(dim, false, ""));
        }

        // Labels
        for (label, pc) in &body.labels {
            if let Some(span) = body.lines.get(*pc) {
                if let Some(line_info) = line_map.iter()
                    .find(|li| li.start_offset <= span.start && span.start < li.end_offset) {
                    let line = line_info.original_line.saturating_sub(1);
                    children.push(DocumentSymbol {
                        name: format!("${}", label),
                        kind: LspSymbolKind::FIELD, // Label
                        tags: None,
                        deprecated: None,
                        range: Range::new(
                            Position::new(line, 0),
                            Position::new(line, 100),
                        ),
                        selection_range: Range::new(
                            Position::new(line, 0),
                            Position::new(line, label.len() as u32 + 1),
                        ),
                        children: None,
                        detail: Some(format!("Label in {}", func.name)),
                    });
                }
            }
        }

        let kind = if func.is_function {
            LspSymbolKind::METHOD
        } else {
            LspSymbolKind::FUNCTION
        };

        let (start_line, end_line) = if let (Some(first_span), Some(last_span)) = (body.lines.first(), body.lines.last()) {
            let start = line_map.iter()
                .find(|li| li.start_offset <= first_span.start && first_span.start < li.end_offset)
                .map(|li| li.original_line.saturating_sub(1))
                .unwrap_or(0);
            let end = line_map.iter()
                .find(|li| li.start_offset <= last_span.start && last_span.start < li.end_offset)
                .map(|li| li.original_line.saturating_sub(1))
                .unwrap_or(0);
            (start, end)
        } else {
            (0, 0)
        };

        DocumentSymbol {
            name: func.name.clone(),
            kind,
            tags: None,
            deprecated: None,
            range: Range::new(Position::new(start_line, 0), Position::new(end_line, 100)),
            selection_range: Range::new(
                Position::new(start_line, 0),
                Position::new(start_line, func.name.len() as u32 + 1),
            ),
            children: Some(children),
            detail: Some(
                if func.is_function {
                    "#FUNCTION"
                } else {
                    "Function"
                }
                .into(),
            ),
        }
    }

    /// Convert a dimension to a document symbol.
    fn dim_to_symbol(&self, dim: &DimDecl, is_global: bool, text: &str) -> DocumentSymbol {
        let kind = if dim.is_str {
            LspSymbolKind::VARIABLE
        } else {
            LspSymbolKind::VARIABLE
        };

        // Use span if available, otherwise default range
        let range = if !dim.span.is_empty() && !text.is_empty() {
            span_to_range(
                text,
                crate::parser::ast::Span::new(dim.span.start, dim.span.end),
                PositionEncoding::Utf16,
            )
        } else {
            Range::default()
        };

        DocumentSymbol {
            name: dim.name.clone(),
            kind,
            tags: if dim.is_const {
                Some(vec![lsp_types::SymbolTag::DEPRECATED])
            } else {
                None
            },
            deprecated: None,
            range,
            selection_range: range,
            children: None,
            detail: Some(format!(
                "{} {}",
                if is_global { "Global" } else { "Local" },
                if dim.is_str { "Str" } else { "Int" }
            )),
        }
    }

    /// Convert a macro to a document symbol.
    fn macro_to_symbol(&self, name: &str, body: &str) -> DocumentSymbol {
        DocumentSymbol {
            name: name.to_string(),
            kind: LspSymbolKind::MODULE,
            tags: None,
            deprecated: None,
            range: Range::default(),
            selection_range: Range::default(),
            children: None,
            detail: Some("Macro".into()),
        }
    }
}

impl Default for DocumentSymbolsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_symbols() {
        let provider = DocumentSymbolsProvider::new();
        // Would need a document with AST to test
    }
}
