//! Workspace management for the LSP server.

use crate::document::DocumentManager;
use crate::parser::loader::{emuera_files, erb_files, walk, Listing, Order};
use crate::parser::{Program, SourceFile};
use crate::util::encoding::{detect_encoding, read_text_file};
use lsp_types::{DidChangeConfigurationParams, DidChangeWorkspaceFoldersParams, WorkspaceFolder};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// Workspace configuration.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    /// Whether to search subdirectories for ERB files.
    pub subdirs: bool,
    /// File ordering method.
    pub order: Order,
    /// The dialect to use (EM, Anchor, or Auto).
    pub dialect: Dialect,
    /// Enable/disable specific lint rules.
    pub lint_rules: LintRules,
    /// Additional include paths.
    pub include_paths: Vec<PathBuf>,
}

/// Lint rule configuration.
#[derive(Debug, Clone)]
pub struct LintRules {
    pub string_comparison: bool,
    pub malformed_names: bool,
    pub unclosed_form_holes: bool,
}

impl Default for LintRules {
    fn default() -> Self {
        Self {
            string_comparison: true,
            malformed_names: true,
            unclosed_form_holes: true,
        }
    }
}

/// EraBasic dialect variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Auto,
    EmueraEM,
    EmueraAnchor,
    EmueraEE,
}

impl Dialect {
    /// Detect dialect from file content.
    pub fn detect_from_content(text: &str) -> Self {
        let upper = text.to_uppercase();
        // Anchor-specific features
        if upper.contains("#FUNCTIONS")
            || upper.contains("#SINGLE")
            || upper.contains("PARAM")
            || upper.contains("TR_")
        {
            return Dialect::EmueraAnchor;
        }
        // EE-specific
        if upper.contains("VARI ") || upper.contains("VARS ") {
            return Dialect::EmueraEE;
        }
        Dialect::EmueraEM
    }
}

/// A workspace folder with its configuration.
#[derive(Debug, Clone)]
pub struct WorkspaceFolderData {
    pub uri: lsp_types::Url,
    pub name: String,
    pub path: PathBuf,
    pub config: WorkspaceConfig,
}

/// The workspace manager handles multiple workspace folders, file watching, and configuration.
pub struct WorkspaceManager {
    folders: RwLock<Vec<WorkspaceFolderData>>,
    config: RwLock<WorkspaceConfig>,
    document_manager: Arc<DocumentManager>,
    file_watcher: RwLock<Option<RecommendedWatcher>>,
    change_sender: broadcast::Sender<WorkspaceChange>,
}

/// Events that can happen in the workspace.
#[derive(Debug, Clone)]
pub enum WorkspaceChange {
    FileCreated(PathBuf),
    FileModified(PathBuf),
    FileDeleted(PathBuf),
    FileRenamed(PathBuf, PathBuf),
    FolderAdded(WorkspaceFolderData),
    FolderRemoved(PathBuf),
    ConfigChanged(WorkspaceConfig),
}

impl WorkspaceManager {
    pub fn new(document_manager: Arc<DocumentManager>) -> Self {
        let (tx, _rx) = broadcast::channel(100);
        Self {
            folders: RwLock::new(Vec::new()),
            config: RwLock::new(WorkspaceConfig::default()),
            document_manager,
            file_watcher: RwLock::new(None),
            change_sender: tx,
        }
    }

    /// Initialize the file watcher.
    pub fn init_watcher(&self) -> notify::Result<()> {
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    Self::handle_event(event);
                }
            },
            Config::default(),
        )?;

        // Watch all workspace folders
        let folders = self.folders.read().unwrap();
        for folder in folders.iter() {
            watcher.watch(&folder.path, RecursiveMode::Recursive)?;
        }

        *self.file_watcher.write().unwrap() = Some(watcher);
        Ok(())
    }

    fn handle_event(event: Event) {
        // Process file system events
        for path in event.paths {
            match event.kind {
                EventKind::Create(_) => {
                    // Handle file creation
                }
                EventKind::Modify(_) => {
                    // Handle file modification
                }
                EventKind::Remove(_) => {
                    // Handle file deletion
                }
                _ => {}
            }
        }
    }

    /// Add a workspace folder.
    pub fn add_folder(&self, folder: WorkspaceFolder) -> Result<(), WorkspaceError> {
        let path = folder
            .uri
            .to_file_path()
            .map_err(|_| WorkspaceError::InvalidPath)?;
        let data = WorkspaceFolderData {
            uri: folder.uri,
            name: folder.name,
            path: path.clone(),
            config: self.config.read().unwrap().clone(),
        };

        self.folders.write().unwrap().push(data.clone());

        // Start watching this folder
        if let Some(watcher) = self.file_watcher.write().unwrap().as_mut() {
            watcher.watch(&path, RecursiveMode::Recursive)?;
        }

        // Discover and load ERB/ERH files
        self.discover_files(&path)?;

        let _ = self.change_sender.send(WorkspaceChange::FolderAdded(data));
        Ok(())
    }

    /// Remove a workspace folder.
    pub fn remove_folder(&self, uri: &lsp_types::Url) -> Result<(), WorkspaceError> {
        let path = uri
            .to_file_path()
            .map_err(|_| WorkspaceError::InvalidPath)?;

        self.folders.write().unwrap().retain(|f| f.uri != *uri);

        if let Some(watcher) = self.file_watcher.write().unwrap().as_mut() {
            watcher.unwatch(&path)?;
        }

        let _ = self
            .change_sender
            .send(WorkspaceChange::FolderRemoved(path));
        Ok(())
    }

    /// Update workspace folders (for didChangeWorkspaceFolders).
    pub fn update_folders(
        &self,
        params: DidChangeWorkspaceFoldersParams,
    ) -> Result<(), WorkspaceError> {
        for folder in params.event.removed {
            self.remove_folder(&folder.uri)?;
        }
        for folder in params.event.added {
            self.add_folder(folder)?;
        }
        Ok(())
    }

    /// Update configuration.
    pub fn update_config(
        &self,
        params: DidChangeConfigurationParams,
    ) -> Result<(), WorkspaceError> {
        if let Some(settings) = params.settings.as_object() {
            let mut config = self.config.write().unwrap();

            if let Some(subdirs) = settings.get("subdirs").and_then(|v| v.as_bool()) {
                config.subdirs = subdirs;
            }
            if let Some(order) = settings.get("order").and_then(|v| v.as_str()) {
                config.order = match order {
                    "icu" => Order::Icu,
                    "nls" => Order::Nls,
                    _ => Order::Ntfs,
                };
            }
            if let Some(dialect) = settings.get("dialect").and_then(|v| v.as_str()) {
                config.dialect = match dialect {
                    "em" => Dialect::EmueraEM,
                    "anchor" => Dialect::EmueraAnchor,
                    "ee" => Dialect::EmueraEE,
                    _ => Dialect::Auto,
                };
            }

            let new_config = config.clone();
            drop(config);

            // Update all folders
            for folder in self.folders.write().unwrap().iter_mut() {
                folder.config = new_config.clone();
            }

            let _ = self
                .change_sender
                .send(WorkspaceChange::ConfigChanged(new_config));
        }
        Ok(())
    }

    /// Discover ERB/ERH files in a directory.
    fn discover_files(&self, root: &Path) -> Result<(), WorkspaceError> {
        let config = self.config.read().unwrap();
        let listing = Listing {
            subdirs: config.subdirs,
            order: config.order,
        };

        // Find ERB files
        let erb_files = erb_files(root, listing);
        for file in erb_files {
            self.load_file(&file)?;
        }

        // Find ERH files
        let erh_files = emuera_files(root, &["erh"], listing);
        for file in erh_files {
            self.load_file(&file)?;
        }

        // Find CSV files (for reference)
        let _csv_files = emuera_files(root, &["csv"], listing);

        Ok(())
    }

    /// Load a single file into the document manager.
    fn load_file(&self, path: &Path) -> Result<(), WorkspaceError> {
        if self
            .document_manager
            .get(&lsp_types::Url::from_file_path(path).unwrap())
            .is_some()
        {
            return Ok(()); // Already loaded
        }

        self.document_manager.open_file(path)?;
        Ok(())
    }

    /// Get all workspace folders.
    pub fn folders(&self) -> Vec<WorkspaceFolderData> {
        self.folders.read().unwrap().clone()
    }

    /// Get the current configuration.
    pub fn config(&self) -> WorkspaceConfig {
        self.config.read().unwrap().clone()
    }

    /// Get a subscription to workspace changes.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkspaceChange> {
        self.change_sender.subscribe()
    }

    /// Find all ERB files in the workspace.
    pub fn find_erb_files(&self) -> Vec<PathBuf> {
        let config = self.config.read().unwrap();
        let listing = Listing {
            subdirs: config.subdirs,
            order: config.order,
        };

        let mut files = Vec::new();
        for folder in self.folders.read().unwrap().iter() {
            files.extend(erb_files(&folder.path, listing));
        }
        files
    }

    /// Find all ERH files in the workspace.
    pub fn find_erh_files(&self) -> Vec<PathBuf> {
        let config = self.config.read().unwrap();
        let listing = Listing {
            subdirs: config.subdirs,
            order: config.order,
        };

        let mut files = Vec::new();
        for folder in self.folders.read().unwrap().iter() {
            files.extend(emuera_files(&folder.path, &["erh"], listing));
        }
        files
    }

    /// Get the dialect for a specific file (auto-detect if needed).
    pub fn get_dialect_for_file(&self, path: &Path) -> Dialect {
        let config = self.config.read().unwrap();
        if config.dialect != Dialect::Auto {
            return config.dialect;
        }

        // Try to read the file and detect
        if let Ok((text, _)) = read_text_file(path) {
            Dialect::detect_from_content(&text)
        } else {
            Dialect::EmueraEM
        }
    }

    /// Check if a file is in the workspace.
    pub fn contains_file(&self, path: &Path) -> bool {
        for folder in self.folders.read().unwrap().iter() {
            if path.starts_with(&folder.path) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Invalid path")]
    InvalidPath,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
    #[error("Document error: {0}")]
    Document(#[from] crate::document::DocumentError),
}

/// Default workspace configuration.
impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            subdirs: false,
            order: Order::Ntfs,
            dialect: Dialect::Auto,
            lint_rules: LintRules::default(),
            include_paths: Vec::new(),
        }
    }
}
