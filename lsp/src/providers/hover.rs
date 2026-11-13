//! Hover provider for EraBasic LSP.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::symbols::{FunctionSignature, Parameter, Symbol, SymbolKind, VarType};
use crate::analysis::variant::Dialect;
use crate::document::Document;
use crate::parser::builtins::{
    get_command_doc, get_function_doc, get_keyword_doc, is_builtin_command, is_builtin_function,
    returns_str,
};
use lsp_types::{Hover, HoverParams, MarkupContent, MarkupKind, Position, Range};
use std::collections::HashMap;

/// Hover provider for EraBasic.
pub struct HoverProvider {
    workspace_index: WorkspaceIndex,
    dialect: Dialect,
    /// Cached documentation for built-ins
    builtin_docs: HashMap<String, String>,
}

impl HoverProvider {
    pub fn new(workspace_index: WorkspaceIndex, dialect: Dialect) -> Self {
        Self {
            workspace_index,
            dialect,
            builtin_docs: HashMap::new(),
        }
    }

    /// Provide hover information at the given position.
    pub fn provide(&self, params: HoverParams, document: &Document) -> Option<Hover> {
        let position = params.text_document_position_params.position;
        let text = document.get_text();

        // Get the word at the cursor
        let word = self.get_word_at_position(&text, position)?;
        let upper_word = word.to_uppercase();

        // Try to find the symbol in the workspace
        if let Some(symbol) = self.workspace_index.find_symbol(&upper_word) {
            return Some(self.symbol_to_hover(symbol, position, &document));
        }

        // Check built-in commands
        if is_builtin_command(&upper_word) {
            return Some(Hover {
                contents: lsp_types::HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!(
                        "**Built-in Command: {}**\n\n{}",
                        upper_word,
                        get_command_doc(&upper_word)
                    ),
                }),
                range: Some(self.word_range(&text, position, &word)),
            });
        }

        // Check built-in functions
        if is_builtin_function(&upper_word) {
            let ret_str = returns_str(&upper_word);
            return Some(Hover {
                contents: lsp_types::HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!(
                        "**Built-in Function: {}** `{}`\n\n{}",
                        upper_word,
                        if ret_str { "Str" } else { "Int" },
                        get_function_doc(&upper_word)
                    ),
                }),
                range: Some(self.word_range(&text, position, &word)),
            });
        }

        // Check keywords
        if crate::parser::builtins::KEYWORDS.contains(&upper_word.as_str()) {
            return Some(Hover {
                contents: lsp_types::HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!(
                        "**Keyword: {}**\n\n{}",
                        upper_word,
                        get_keyword_doc(&upper_word)
                    ),
                }),
                range: Some(self.word_range(&text, position, &word)),
            });
        }

        None
    }

    /// Get the word at the given position.
    fn get_word_at_position(&self, text: &str, position: Position) -> Option<String> {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let before = &text[..offset.min(text.len())];
        let after = &text[offset.min(text.len())..];

        // Find start of word
        let mut start = before.len();
        for (i, c) in before.char_indices().rev() {
            if c.is_alphanumeric() || c == '_' || c == '$' || c == '@' || c == '#' || c == ':' {
                start = i;
            } else {
                break;
            }
        }

        // Find end of word
        let mut end = 0;
        for (i, c) in after.char_indices() {
            if c.is_alphanumeric() || c == '_' || c == '$' || c == '@' || c == '#' || c == ':' {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }

        let word = &text[start..offset.min(text.len()) + end];
        if word.is_empty() {
            None
        } else {
            Some(word.to_string())
        }
    }

    /// Get the range of the word at the position.
    fn word_range(&self, text: &str, position: Position, word: &str) -> Range {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let start_offset = offset.saturating_sub(word.len());
        let start = crate::util::position::offset_to_position(
            text,
            start_offset,
            crate::util::position::PositionEncoding::Utf16,
        );
        let end = crate::util::position::offset_to_position(
            text,
            start_offset + word.len(),
            crate::util::position::PositionEncoding::Utf16,
        );
        Range::new(start, end)
    }

    /// Convert a symbol to hover information.
    fn symbol_to_hover(&self, symbol: &Symbol, position: Position, document: &Document) -> Hover {
        let mut content = String::new();

        // Symbol name and kind
        content.push_str(&format!("**{}**: ", symbol.name));

        match symbol.kind {
            SymbolKind::FUNCTION | SymbolKind::METHOD => {
                content.push_str(
                    if symbol.signature.as_ref().is_some_and(|s| s.is_function) {
                        "#FUNCTION"
                    } else {
                        "Function"
                    },
                );
                content.push_str("\n\n");

                if let Some(sig) = &symbol.signature {
                    let params = sig
                        .params
                        .iter()
                        .map(|p| format!("{}: {}", p.name, var_type_str(&p.var_type)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let ret = sig
                        .return_type
                        .as_ref()
                        .map(var_type_str)
                        .unwrap_or("Void".to_string());
                    content.push_str(&format!(
                        "```erabasic\n{} {}({}) -> {}\n```\n\n",
                        if sig.is_function { "#FUNCTION" } else { "@" },
                        symbol.name,
                        params,
                        ret
                    ));

                    if sig.is_event {
                        content.push_str(&format!("**Event**: {:?}\n\n", sig.event_order));
                    }
                }
            }
            SymbolKind::VARIABLE | SymbolKind::FIELD | SymbolKind::PROPERTY => {
                content.push_str("Variable");
                if let Some(var_type) = &symbol.var_type {
                    content.push_str(&format!(": {}", var_type_str(var_type)));
                }
                content.push_str("\n\n");
            }
            SymbolKind::NAMESPACE => {
                content.push_str("Label");
                if let Some(parent) = &symbol.parent_function {
                    content.push_str(&format!(" in `{}`", parent));
                }
                content.push_str("\n\n");
            }
            SymbolKind::CONSTANT => {
                content.push_str("Constant");
                if let Some(var_type) = &symbol.var_type {
                    content.push_str(&format!(": {}", var_type_str(var_type)));
                }
                content.push_str("\n\n");
            }
            SymbolKind::MODULE => {
                content.push_str("Macro");
                content.push_str("\n\n");
            }
            _ => {
                content.push_str(&format!("{:?}\n\n", symbol.kind));
            }
        }

        if let Some(detail) = &symbol.detail {
            content.push_str(&format!("{}\n\n", detail));
        }

        if let Some(doc) = &symbol.documentation {
            content.push_str(doc);
        }

        Hover {
            contents: lsp_types::HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(self.word_range(&document.get_text(), position, &symbol.name)),
        }
    }
}

fn var_type_str(var_type: &VarType) -> String {
    match var_type {
        VarType::Int => "Int".into(),
        VarType::Str => "Str".into(),
        VarType::IntArray(dims) => format!(
            "Int[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::StrArray(dims) => format!(
            "Str[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::Chara => "Chara".into(),
        VarType::CharaArray(dims) => format!(
            "Chara[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::Unknown => "Unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_hover_provider() {
        let index = WorkspaceIndex::new();
        let provider = HoverProvider::new(index, Dialect::EmueraEM);

        // Test word extraction
        let text = "PRINT HELLO";
        let pos = Position::new(0, 8); // On "HELLO"
        let word = provider.get_word_at_position(text, pos);
        assert_eq!(word, Some("HELLO".to_string()));
    }
}
