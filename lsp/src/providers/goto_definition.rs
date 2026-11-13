//! Go to definition provider for EraBasic LSP.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::references::{Reference, ReferenceKind, WorkspaceReferenceIndex};
use crate::analysis::variant::Dialect;
use crate::document::Document;
use lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink, Position, Range,
};
use std::path::Path;

/// Go to definition provider for EraBasic.
pub struct GotoDefinitionProvider {
    workspace_index: WorkspaceIndex,
    reference_index: WorkspaceReferenceIndex,
    dialect: Dialect,
}

impl GotoDefinitionProvider {
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

    /// Provide go-to-definition locations.
    pub fn provide(
        &self,
        params: GotoDefinitionParams,
        document: &Document,
    ) -> Option<GotoDefinitionResponse> {
        let position = params.text_document_position_params.position;
        let text = document.get_text();

        // Get the word at the cursor
        let word = self.get_word_at_position(&text, position)?;
        let upper_word = word.to_uppercase();

        // Try to find the definition in the reference index
        if let Some(def) = self.reference_index.find_definition(&upper_word) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: lsp_types::Url::from_file_path(&def.file).ok()?,
                range: def.range,
            }));
        }

        // Try to find in workspace index
        if let Some(symbol) = self.workspace_index.find_symbol(&upper_word) {
            let uri = lsp_types::Url::from_file_path(&symbol.file).ok()?;
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri,
                range: symbol.range,
            }));
        }

        // Check built-ins (no definition location)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::references::WorkspaceReferenceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_goto_definition() {
        let index = WorkspaceIndex::new();
        let ref_index = WorkspaceReferenceIndex::new();
        let provider = GotoDefinitionProvider::new(index, ref_index, Dialect::EmueraEM);

        let text = "CALL MYFUNC";
        let pos = Position::new(0, 8); // On "MYFUNC"
        let word = provider.get_word_at_position(text, pos);
        assert_eq!(word, Some("MYFUNC".to_string()));
    }
}
