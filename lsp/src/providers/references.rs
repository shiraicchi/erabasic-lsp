//! References provider for EraBasic LSP.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::references::{Reference, ReferenceKind, WorkspaceReferenceIndex};
use crate::analysis::variant::Dialect;
use crate::document::Document;
use lsp_types::{Location, ReferenceContext, ReferenceParams};
use std::path::Path;

/// References provider for EraBasic.
pub struct ReferencesProvider {
    workspace_index: WorkspaceIndex,
    reference_index: WorkspaceReferenceIndex,
    dialect: Dialect,
}

impl ReferencesProvider {
    pub fn new(
        workspace_index: WorkspaceIndex,
        reference_index: WorkspaceReferenceIndex,
        dialect: Dialect,
    ) -> Self {
        Self {
            workspace_index,
            reference_index,
            dialect,
        }
    }

    /// Provide all references to the symbol at the given position.
    pub fn provide(&self, params: ReferenceParams, document: &Document) -> Vec<Location> {
        let position = params.text_document_position.position;
        let text = document.get_text();

        // Get the word at the cursor
        let word = match self.get_word_at_position(&text, position) {
            Some(w) => w,
            None => return Vec::new(),
        };
        let upper_word = word.to_uppercase();

        // Find all references in the workspace
        let refs = self.reference_index.find_all_references(&upper_word);

        // Filter by include_declaration if needed
        let filtered: Vec<_> = if params.context.include_declaration {
            refs.into_iter().collect()
        } else {
            refs.into_iter()
                .filter(|r| r.kind != ReferenceKind::Definition)
                .collect()
        };

        // Convert to LSP locations
        filtered
            .iter()
            .filter_map(|r| {
                let uri = lsp_types::Url::from_file_path(&r.file).ok()?;
                Some(Location {
                    uri,
                    range: r.range,
                })
            })
            .collect()
    }

    /// Get the word at the given position.
    fn get_word_at_position(&self, text: &str, position: lsp_types::Position) -> Option<String> {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let before = &text[..offset.min(text.len())];
        let after = &text[offset.min(text.len())..];

        let mut start = before.len();
        for (i, c) in before.char_indices().rev() {
            if c.is_alphanumeric() || c == '_' || c == '$' || c == '@' || c == '#' || c == ':' {
                start = i;
            } else {
                break;
            }
        }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::references::WorkspaceReferenceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_references_provider() {
        let index = WorkspaceIndex::new();
        let ref_index = WorkspaceReferenceIndex::new();
        let provider = ReferencesProvider::new(index, ref_index, Dialect::EmueraEM);

        let text = "CALL MYFUNC";
        let pos = lsp_types::Position::new(0, 8);
        let word = provider.get_word_at_position(text, pos);
        assert_eq!(word, Some("MYFUNC".to_string()));
    }
}
