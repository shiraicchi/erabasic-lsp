//! Workspace symbols provider for EraBasic LSP.

use crate::analysis::index::{WorkspaceIndex, WorkspaceSymbolResult};
use crate::analysis::variant::Dialect;
use lsp_types::{
    Location, Position, Range, SymbolInformation, SymbolKind as LspSymbolKind,
    WorkspaceSymbolParams,
};

/// Workspace symbols provider for EraBasic.
pub struct WorkspaceSymbolsProvider {
    workspace_index: WorkspaceIndex,
    dialect: Dialect,
}

impl WorkspaceSymbolsProvider {
    pub fn new(workspace_index: WorkspaceIndex, dialect: Dialect) -> Self {
        Self {
            workspace_index,
            dialect,
        }
    }

    /// Provide workspace symbols matching the query.
    pub fn provide(&self, params: WorkspaceSymbolParams) -> Vec<SymbolInformation> {
        let results = self.workspace_index.workspace_symbol_search(&params.query);

        results
            .into_iter()
            .map(|result| {
                let symbol = result.symbol;
                let kind = match symbol.kind {
                    crate::analysis::symbols::SymbolKind::FUNCTION => LspSymbolKind::FUNCTION,
                    crate::analysis::symbols::SymbolKind::METHOD => LspSymbolKind::METHOD,
                    crate::analysis::symbols::SymbolKind::VARIABLE => LspSymbolKind::VARIABLE,
                    crate::analysis::symbols::SymbolKind::FIELD => LspSymbolKind::FIELD,
                    crate::analysis::symbols::SymbolKind::PROPERTY => LspSymbolKind::PROPERTY,
                    crate::analysis::symbols::SymbolKind::CONSTANT => LspSymbolKind::CONSTANT,
                    crate::analysis::symbols::SymbolKind::MODULE => LspSymbolKind::MODULE,
                    crate::analysis::symbols::SymbolKind::NAMESPACE => LspSymbolKind::NAMESPACE,
                    _ => LspSymbolKind::VARIABLE,
                };

                SymbolInformation {
                    name: symbol.name.clone(),
                    kind,
                    tags: symbol.tags,
                    deprecated: symbol.deprecated,
                    location: Location {
                        uri: lsp_types::Url::from_file_path(&symbol.file).unwrap(),
                        range: symbol.range,
                    },
                    container_name: result.container_name,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_workspace_symbols() {
        let index = WorkspaceIndex::new();
        let provider = WorkspaceSymbolsProvider::new(index, Dialect::EmueraEM);
        // Would need indexed symbols to test
    }
}
