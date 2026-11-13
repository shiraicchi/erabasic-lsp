//! EraBasic Language Server implementation.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::references::WorkspaceReferenceIndex;
use crate::analysis::variant::{Dialect, DialectConfig};
use crate::docs::{builtin_docs::BuiltinDocs, em_reference::EmReferenceIndex};
use crate::document::{Document, DocumentManager};
use crate::providers::{
    code_actions::CodeActionProvider, completion::CompletionProvider,
    diagnostics::DiagnosticsProvider, document_symbols::DocumentSymbolsProvider,
    folding::FoldingProvider, goto_definition::GotoDefinitionProvider, hover::HoverProvider,
    inlay_hints::InlayHintProvider, references::ReferencesProvider,
    semantic_tokens::SemanticTokensProvider, signature_help::SignatureHelpProvider,
    workspace_symbols::WorkspaceSymbolsProvider,
};
use crate::workspace::{WorkspaceConfig, WorkspaceFolderData, WorkspaceManager};
use lsp_types::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tower_lsp::{jsonrpc::Result, Client, LanguageServer, LspService};
use tracing::{debug, error, info};

/// EraBasic Language Server.
pub struct EraBasicLanguageServer {
    client: Client,
    document_manager: Arc<DocumentManager>,
    workspace_manager: Arc<WorkspaceManager>,
    workspace_index: Arc<Mutex<WorkspaceIndex>>,
    reference_index: Arc<Mutex<WorkspaceReferenceIndex>>,
    config: Arc<Mutex<WorkspaceConfig>>,
    dialect: Arc<RwLock<Dialect>>,
    semantic_tokens_provider: SemanticTokensProvider,
    builtin_docs: BuiltinDocs,
    em_reference: Option<EmReferenceIndex>,
    _change_receiver: Mutex<Option<broadcast::Receiver<crate::workspace::WorkspaceChange>>>,
}

impl EraBasicLanguageServer {
    /// Create a new language server.
    pub fn new(client: Client) -> Self {
        let workspace_index = Arc::new(Mutex::new(WorkspaceIndex::new()));
        let reference_index = Arc::new(Mutex::new(WorkspaceReferenceIndex::new()));
        let config = Arc::new(Mutex::new(WorkspaceConfig::default()));
        let dialect = Arc::new(RwLock::new(Dialect::Auto));

        let (doc_manager, parse_receiver) = DocumentManager::new(
            Arc::downgrade(&workspace_index),
            Arc::downgrade(&reference_index),
            dialect.clone(),
        );
        let doc_manager = Arc::new(doc_manager);

        let workspace_manager = Arc::new(WorkspaceManager::new(doc_manager.clone()));

        let semantic_tokens_provider = SemanticTokensProvider::new();
        let builtin_docs = BuiltinDocs::new();

        // Try to load EM-Reference documentation
        let em_reference = EmReferenceIndex::new(std::path::Path::new("EM-Reference")).ok();

        let server = Self {
            client,
            document_manager: doc_manager,
            workspace_manager,
            workspace_index,
            reference_index,
            config,
            dialect,
            semantic_tokens_provider,
            builtin_docs,
            em_reference,
            _change_receiver: Mutex::new(None),
        };

        // Start background parsing task
        server.start_parsing_task(parse_receiver);

        server
    }

    /// Start the background parsing task.
    fn start_parsing_task(
        &self,
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<lsp_types::Url>,
    ) {
        let doc_manager = self.document_manager.clone();
        let workspace_index = self.workspace_index.clone();
        let reference_index = self.reference_index.clone();
        let dialect = self.dialect.clone();
        let builtin_docs = self.builtin_docs.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            while let Some(uri) = receiver.recv().await {
                if let Some(doc_arc) = doc_manager.get(&uri) {
                    // Parse and index
                    let (ast, file_path, current_dialect) = {
                        let doc_guard = doc_arc.read().unwrap();
                        if let Some(ast) = &doc_guard.ast {
                            let file_path = doc_guard
                                .file_path
                                .clone()
                                .unwrap_or_else(|| PathBuf::from(uri.path()));
                            let current_dialect = *dialect.read().unwrap();
                            (ast.clone(), file_path, current_dialect)
                        } else {
                            continue;
                        }
                    };

                    // Build symbol table
                    let symbol_table = crate::analysis::symbols::build_symbol_table(
                        &ast.program,
                        file_path.clone(),
                        current_dialect,
                    );

                    // Update workspace index
                    {
                        let mut index = workspace_index.lock().unwrap();
                        index.update_file(file_path.clone(), symbol_table);
                    }

                    // Resolve references
                    {
                        let mut ref_index = reference_index.lock().unwrap();
                        let mut resolver = crate::analysis::references::ReferenceResolver::new();
                        let refs = resolver.resolve_program_lsp(&ast.program, file_path.clone());
                        ref_index.add_file(file_path, refs);
                    }

                    // Publish diagnostics
                    let diagnostics_provider = DiagnosticsProvider::new();
                    let params = diagnostics_provider.provide_from_ast(&ast, uri.clone());
                    client
                        .publish_diagnostics(params.uri, params.diagnostics, params.version)
                        .await;
                }
            }
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for EraBasicLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("Initializing EraBasic Language Server");

        // Store workspace folders
        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                if let Err(e) = self.workspace_manager.add_folder(folder) {
                    error!("Failed to add workspace folder: {}", e);
                }
            }
        }

        // Initialize file watcher
        if let Err(e) = self.workspace_manager.init_watcher() {
            error!("Failed to initialize file watcher: {}", e);
        }

        // Start workspace change listener
        let change_receiver = self.workspace_manager.subscribe();
        *self._change_receiver.lock().unwrap() = Some(change_receiver);

        let config = self.config.lock().unwrap().clone();
        let mut dialect = self.dialect.write().unwrap();

        // Set dialect from config
        *dialect = match config.dialect {
            crate::workspace::Dialect::Auto => crate::analysis::variant::Dialect::Auto,
            crate::workspace::Dialect::EmueraEM => crate::analysis::variant::Dialect::EmueraEM,
            crate::workspace::Dialect::EmueraAnchor => {
                crate::analysis::variant::Dialect::EmueraAnchor
            }
            crate::workspace::Dialect::EmueraEE => crate::analysis::variant::Dialect::EmueraEE,
        };

        // Update workspace index dialect
        self.workspace_index
            .lock()
            .unwrap()
            .set_dialect(match config.dialect {
                crate::workspace::Dialect::Auto => crate::analysis::variant::Dialect::Auto,
                crate::workspace::Dialect::EmueraEM => crate::analysis::variant::Dialect::EmueraEM,
                crate::workspace::Dialect::EmueraAnchor => {
                    crate::analysis::variant::Dialect::EmueraAnchor
                }
                crate::workspace::Dialect::EmueraEE => crate::analysis::variant::Dialect::EmueraEE,
            });

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        ".".into(),
                        "@".into(),
                        "#".into(),
                        "$".into(),
                        ":".into(),
                    ]),
                    all_commit_characters: None,
                    work_done_progress_options: Default::default(),
                    completion_item: None,
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".into(), ",".into()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
                        SemanticTokensRegistrationOptions {
                            text_document_registration_options: TextDocumentRegistrationOptions {
                                document_selector: Some(vec![DocumentFilter {
                                    language: Some("erabasic".into()),
                                    scheme: Some("file".into()),
                                    pattern: None,
                                }]),
                            },
                            semantic_tokens_options: SemanticTokensOptions {
                                work_done_progress_options: Default::default(),
                                legend: self.semantic_tokens_provider.legend().clone(),
                                range: Some(true),
                                full: Some(SemanticTokensFullOptions::Bool(true)),
                            },
                            static_registration_options: Default::default(),
                        },
                    ),
                ),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                document_formatting_provider: None,
                document_range_formatting_provider: None,
                document_on_type_formatting_provider: None,
                rename_provider: None,
                execute_command_provider: None,
                selection_range_provider: None,
                linked_editing_range_provider: None,
                call_hierarchy_provider: None,
                inline_value_provider: None,
                moniker_provider: None,
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: None,
                        inter_file_dependencies: false,
                        workspace_diagnostics: true,
                        work_done_progress_options: Default::default(),
                    },
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "EraBasic Language Server".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("Server initialized");

        // Load EM-Reference if available
        if self.em_reference.is_some() {
            info!("EM-Reference documentation loaded");
        }
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        let text = params.text_document.text;

        debug!("Opened document: {}", uri);

        if let Err(e) = self.document_manager.open(uri, version, text) {
            error!("Failed to open document: {}", e);
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let changes = params.content_changes;

        debug!("Changed document: {}", uri);

        if let Err(e) = self.document_manager.change(&uri, version, changes) {
            error!("Failed to apply change: {}", e);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        debug!("Closed document: {}", uri);

        self.document_manager.close(&uri);
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        if let Err(e) = self.workspace_manager.update_folders(params) {
            error!("Failed to update workspace folders: {}", e);
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        if let Err(e) = self.workspace_manager.update_config(params) {
            error!("Failed to update configuration: {}", e);
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = CompletionProvider::new(
                (*self.workspace_index.lock().unwrap()).clone(),
                *self.dialect.read().unwrap(),
            );

            let response = provider.provide(params, &doc);
            Ok(Some(response))
        } else {
            Ok(None)
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = HoverProvider::new(
                (*self.workspace_index.lock().unwrap()).clone(),
                *self.dialect.read().unwrap(),
            );

            let hover = provider.provide(params, &doc);
            Ok(hover)
        } else {
            Ok(None)
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = SignatureHelpProvider::new(
                (*self.workspace_index.lock().unwrap()).clone(),
                *self.dialect.read().unwrap(),
            );

            let help = provider.provide(params, &doc);
            Ok(help)
        } else {
            Ok(None)
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = GotoDefinitionProvider::new(
                (*self.workspace_index.lock().unwrap()).clone(),
                (*self.reference_index.lock().unwrap()).clone(),
                *self.dialect.read().unwrap(),
            );

            let response = provider.provide(params, &doc);
            Ok(response)
        } else {
            Ok(None)
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = ReferencesProvider::new(
                (*self.workspace_index.lock().unwrap()).clone(),
                (*self.reference_index.lock().unwrap()).clone(),
                *self.dialect.read().unwrap(),
            );

            let locations = provider.provide(params, &doc);
            Ok(Some(locations))
        } else {
            Ok(None)
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = DocumentSymbolsProvider::new();
            let symbols = provider.provide(params, &doc);
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        } else {
            Ok(None)
        }
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let result = self.semantic_tokens_provider.provide(params, &doc);
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<Vec<CodeActionOrCommand>>> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = CodeActionProvider::new();
            let actions = provider.provide(params, &doc);
            Ok(Some(actions))
        } else {
            Ok(None)
        }
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = FoldingProvider::new();
            let ranges = provider.provide(params, &doc);
            Ok(Some(ranges))
        } else {
            Ok(None)
        }
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = InlayHintProvider::new();
            let hints = provider.provide(params, &doc);
            Ok(Some(hints))
        } else {
            Ok(None)
        }
    }

    /// Pull diagnostics for a specific document (textDocument/diagnostic)
    async fn diagnostic(&self, params: DocumentDiagnosticParams) -> Result<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri.clone();

        if let Some(doc_arc) = self.document_manager.get(&uri) {
            let doc = doc_arc.read().unwrap();
            let provider = DiagnosticsProvider::new();
            let diag_params = provider.provide(&doc);

            let report = DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diag_params.diagnostics,
                },
            });

            Ok(DocumentDiagnosticReportResult::Report(report))
        } else {
            // Return empty report for unknown documents
            let report = DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: vec![],
                },
            });
            Ok(DocumentDiagnosticReportResult::Report(report))
        }
    }

    /// Pull diagnostics for workspace (workspace/diagnostic)
    async fn workspace_diagnostic(&self, params: WorkspaceDiagnosticParams) -> Result<WorkspaceDiagnosticReportResult> {
        let mut reports = vec![];

        // Get all documents and their diagnostics
        let docs = self.document_manager.all_documents();
        for doc_arc in docs {
            let doc = doc_arc.read().unwrap();
            let uri = doc.uri.clone();
            let version = Some(doc.version as i64);
            let provider = DiagnosticsProvider::new();
            let diag_params = provider.provide(&doc);

            if !diag_params.diagnostics.is_empty() {
                let full_report = FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diag_params.diagnostics,
                };
                let workspace_report = WorkspaceFullDocumentDiagnosticReport {
                    uri,
                    version,
                    full_document_diagnostic_report: full_report,
                };
                reports.push(WorkspaceDocumentDiagnosticReport::Full(workspace_report));
            }
        }

        let workspace_diag_report = WorkspaceDiagnosticReport { items: reports };
        Ok(WorkspaceDiagnosticReportResult::Report(workspace_diag_report))
    }
}
