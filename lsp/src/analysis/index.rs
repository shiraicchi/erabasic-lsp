//! Workspace-wide symbol index for cross-file symbol search.

use crate::analysis::references::{Reference, WorkspaceReferenceIndex};
use crate::analysis::symbols::{FileSymbolTable, Symbol, SymbolKind};
use crate::analysis::variant::Dialect;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A workspace-wide symbol index.
#[derive(Debug, Clone)]
pub struct WorkspaceIndex {
    /// File symbol tables
    files: HashMap<PathBuf, Arc<FileSymbolTable>>,
    /// All symbols by name (for quick lookup)
    symbols_by_name: HashMap<String, Vec<Arc<Symbol>>>,
    /// Functions by name
    functions: HashMap<String, Vec<Arc<Symbol>>>,
    /// Labels by name
    labels: HashMap<String, Vec<Arc<Symbol>>>,
    /// Variables by name
    variables: HashMap<String, Vec<Arc<Symbol>>>,
    /// Reference index
    references: WorkspaceReferenceIndex,
    /// Current dialect
    dialect: Dialect,
}

impl WorkspaceIndex {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            symbols_by_name: HashMap::new(),
            functions: HashMap::new(),
            labels: HashMap::new(),
            variables: HashMap::new(),
            references: WorkspaceReferenceIndex::new(),
            dialect: Dialect::Auto,
        }
    }

    /// Set the dialect for the workspace.
    pub fn set_dialect(&mut self, dialect: Dialect) {
        self.dialect = dialect;
    }

    /// Add a file's symbol table to the index.
    pub fn add_file(&mut self, file_path: PathBuf, table: FileSymbolTable) {
        let arc_table = Arc::new(table);

        // Index all symbols
        for symbol in arc_table.all_symbols() {
            let name = symbol.name.to_uppercase();
            let arc_symbol = Arc::new(symbol.clone());

            self.symbols_by_name
                .entry(name.clone())
                .or_default()
                .push(arc_symbol.clone());

            match symbol.kind {
                SymbolKind::FUNCTION | SymbolKind::METHOD => {
                    self.functions.entry(name).or_default().push(arc_symbol);
                }
                SymbolKind::NAMESPACE => {
                    // Labels
                    self.labels.entry(name).or_default().push(arc_symbol);
                }
                SymbolKind::VARIABLE | SymbolKind::FIELD | SymbolKind::PROPERTY => {
                    self.variables.entry(name).or_default().push(arc_symbol);
                }
                _ => {}
            }
        }

        self.files.insert(file_path, arc_table);
    }

    /// Remove a file from the index.
    pub fn remove_file(&mut self, file_path: &Path) {
        if let Some(table) = self.files.remove(file_path) {
            // Remove symbols from indexes
            for symbol in table.all_symbols() {
                let name = symbol.name.to_uppercase();

                if let Some(vec) = self.symbols_by_name.get_mut(&name) {
                    vec.retain(|s| s.file != symbol.file);
                    if vec.is_empty() {
                        self.symbols_by_name.remove(&name);
                    }
                }

                match symbol.kind {
                    SymbolKind::FUNCTION | SymbolKind::METHOD => {
                        if let Some(vec) = self.functions.get_mut(&name) {
                            vec.retain(|s| s.file != symbol.file);
                            if vec.is_empty() {
                                self.functions.remove(&name);
                            }
                        }
                    }
                    SymbolKind::NAMESPACE => {
                        if let Some(vec) = self.labels.get_mut(&name) {
                            vec.retain(|s| s.file != symbol.file);
                            if vec.is_empty() {
                                self.labels.remove(&name);
                            }
                        }
                    }
                    SymbolKind::VARIABLE | SymbolKind::FIELD | SymbolKind::PROPERTY => {
                        if let Some(vec) = self.variables.get_mut(&name) {
                            vec.retain(|s| s.file != symbol.file);
                            if vec.is_empty() {
                                self.variables.remove(&name);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Update a file in the index (remove old, add new).
    pub fn update_file(&mut self, file_path: PathBuf, table: FileSymbolTable) {
        self.remove_file(&file_path);
        self.add_file(file_path, table);
    }

    /// Find a symbol by name across all files.
    pub fn find_symbol(&self, name: &str) -> Option<&Arc<Symbol>> {
        self.symbols_by_name.get(&name.to_uppercase())?.first()
    }

    /// Find all symbols with a given name.
    pub fn find_all_symbols(&self, name: &str) -> Vec<&Arc<Symbol>> {
        self.symbols_by_name
            .get(&name.to_uppercase())
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Find all functions with a given name.
    pub fn find_functions(&self, name: &str) -> Vec<&Arc<Symbol>> {
        self.functions
            .get(&name.to_uppercase())
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Find all labels with a given name.
    pub fn find_labels(&self, name: &str) -> Vec<&Arc<Symbol>> {
        self.labels
            .get(&name.to_uppercase())
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Find all variables with a given name.
    pub fn find_variables(&self, name: &str) -> Vec<&Arc<Symbol>> {
        self.variables
            .get(&name.to_uppercase())
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Search for symbols matching a prefix.
    pub fn search_prefix(&self, prefix: &str) -> Vec<&Arc<Symbol>> {
        let upper_prefix = prefix.to_uppercase();
        self.symbols_by_name
            .iter()
            .filter(|(name, _)| name.starts_with(&upper_prefix))
            .flat_map(|(_, vec)| vec.iter())
            .collect()
    }

    /// Search for symbols matching a pattern (simple substring).
    pub fn search_substring(&self, substr: &str) -> Vec<&Arc<Symbol>> {
        let upper_substr = substr.to_uppercase();
        self.symbols_by_name
            .iter()
            .filter(|(name, _)| name.contains(&upper_substr))
            .flat_map(|(_, vec)| vec.iter())
            .collect()
    }

    /// Get all function symbols.
    pub fn all_functions(&self) -> Vec<&Arc<Symbol>> {
        self.functions.values().flat_map(|v| v.iter()).collect()
    }

    /// Get all label symbols.
    pub fn all_labels(&self) -> Vec<&Arc<Symbol>> {
        self.labels.values().flat_map(|v| v.iter()).collect()
    }

    /// Get all variable symbols.
    pub fn all_variables(&self) -> Vec<&Arc<Symbol>> {
        self.variables.values().flat_map(|v| v.iter()).collect()
    }

    /// Get all symbols in a specific file.
    pub fn file_symbols(&self, file_path: &Path) -> Option<&FileSymbolTable> {
        self.files.get(file_path).map(|arc| arc.as_ref())
    }

    /// Get the symbol table for a file.
    pub fn get_file_table(&self, file_path: &Path) -> Option<Arc<FileSymbolTable>> {
        self.files.get(file_path).cloned()
    }

    /// Add references from a file.
    pub fn add_references(&mut self, file: PathBuf, refs: Vec<Reference>) {
        self.references.add_file(file, refs);
    }

    /// Find all references to a symbol across the workspace.
    pub fn find_references(&self, symbol_name: &str) -> Vec<&Reference> {
        self.references.find_all_references(symbol_name)
    }

    /// Find the definition of a symbol.
    pub fn find_definition(&self, symbol_name: &str) -> Option<&Reference> {
        self.references.find_definition(symbol_name)
    }

    /// Get all files in the index.
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.files.keys()
    }

    /// Get the number of files indexed.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get the number of symbols indexed.
    pub fn symbol_count(&self) -> usize {
        self.symbols_by_name.values().map(|v| v.len()).sum()
    }

    /// Clear the index.
    pub fn clear(&mut self) {
        self.files.clear();
        self.symbols_by_name.clear();
        self.functions.clear();
        self.labels.clear();
        self.variables.clear();
        self.references = WorkspaceReferenceIndex::new();
    }
}

impl Default for WorkspaceIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Symbol search result for workspace symbol requests.
#[derive(Debug, Clone)]
pub struct WorkspaceSymbolResult {
    pub symbol: Symbol,
    pub container_name: Option<String>, // e.g., function name for labels
}

impl WorkspaceIndex {
    /// Search for workspace symbols matching a query.
    pub fn workspace_symbol_search(&self, query: &str) -> Vec<WorkspaceSymbolResult> {
        let mut results = Vec::new();
        let upper_query = query.to_uppercase();

        for (name, symbols) in &self.symbols_by_name {
            if name.contains(&upper_query) {
                for symbol in symbols {
                    let container = match symbol.kind {
                        SymbolKind::NAMESPACE => symbol.parent_function.clone(),
                        SymbolKind::VARIABLE | SymbolKind::FIELD | SymbolKind::PROPERTY => {
                            symbol.parent_function.clone()
                        }
                        _ => None,
                    };

                    results.push(WorkspaceSymbolResult {
                        symbol: symbol.as_ref().clone(),
                        container_name: container,
                    });
                }
            }
        }

        // Sort by relevance: exact matches first, then prefix, then substring
        results.sort_by(|a, b| {
            let a_exact = a.symbol.name.to_uppercase() == upper_query;
            let b_exact = b.symbol.name.to_uppercase() == upper_query;
            let a_prefix = a.symbol.name.to_uppercase().starts_with(&upper_query);
            let b_prefix = b.symbol.name.to_uppercase().starts_with(&upper_query);

            match (a_exact, b_exact) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => match (a_prefix, b_prefix) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.symbol.name.cmp(&b.symbol.name),
                },
            }
        });

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_workspace_index() {
        let mut index = WorkspaceIndex::new();

        let mut table = FileSymbolTable::new(PathBuf::from("test.erb"));
        let symbol = Symbol {
            name: "TEST_FUNC".into(),
            kind: SymbolKind::FUNCTION,
            range: lsp_types::Range::default(),
            selection_range: lsp_types::Range::default(),
            detail: None,
            documentation: None,
            tags: None,
            deprecated: None,
            file: PathBuf::from("test.erb"),
            var_type: None,
            signature: None,
            parent_function: None,
            is_builtin: false,
        };
        table.add_symbol(symbol);

        index.add_file(PathBuf::from("test.erb"), table);

        assert_eq!(index.file_count(), 1);
        assert_eq!(index.symbol_count(), 1);
        assert!(index.find_symbol("TEST_FUNC").is_some());
        assert!(index.find_symbol("test_func").is_some());
    }
}
