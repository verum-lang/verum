//! Incremental LSP backend implementation
//!
//! This module provides an enhanced LSP backend with incremental parsing
//! and debounced diagnostics updates for optimal real-time performance.

use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::completion;
use crate::debouncer::DebouncerManager;
use crate::diagnostics;
use crate::document_cache::DocumentCache;
use crate::formatting;
use crate::goto_definition;
use crate::hover;
use crate::references;
use crate::rename;

/// Enhanced LSP backend with incremental parsing support
pub struct IncrementalBackend {
    /// The LSP client (for sending messages back)
    client: Client,
    /// Document cache with incremental parsing
    document_cache: DocumentCache,
    /// Debouncer for diagnostics updates
    debouncer: Arc<DebouncerManager>,
}

impl IncrementalBackend {
    /// Create a new incremental backend
    pub fn new(client: Client) -> Self {
        Self {
            client,
            document_cache: DocumentCache::new(),
            debouncer: Arc::new(DebouncerManager::with_default_delay()),
        }
    }

    /// Publish diagnostics for a document (debounced)
    async fn publish_diagnostics_debounced(&self, uri: Url) {
        let cache = self.document_cache.clone();
        let client = self.client.clone();
        let uri_clone = uri.clone();

        self.debouncer.schedule_async(uri, move || {
            let cache = cache.clone();
            let client = client.clone();
            let uri = uri_clone.clone();

            async move {
                let diagnostics = cache.get_diagnostics(&uri);

                if let Some(text) = cache.get_text(&uri) {
                    let lsp_diagnostics =
                        diagnostics::convert_diagnostics(diagnostics, &text, &uri);

                    client.publish_diagnostics(uri, lsp_diagnostics, None).await;
                }
            }
        });
    }

    /// Publish diagnostics immediately (no debouncing)
    async fn publish_diagnostics_immediate(&self, uri: Url) {
        let diagnostics = self.document_cache.get_diagnostics(&uri);

        if let Some(text) = self.document_cache.get_text(&uri) {
            let lsp_diagnostics = diagnostics::convert_diagnostics(diagnostics, &text, &uri);

            self.client
                .publish_diagnostics(uri, lsp_diagnostics, None)
                .await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for IncrementalBackend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        tracing::info!("Initializing Verum LSP server (incremental)");

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "verum-lsp-incremental".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        will_save: None,
                        will_save_wait_until: None,
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(false),
                        })),
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        " ".to_string(),
                    ]),
                    all_commit_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                    completion_item: None,
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                    first_trigger_character: "}".to_string(),
                    more_trigger_character: Some(vec![";".to_string(), "\n".to_string()]),
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR,
                            CodeActionKind::REFACTOR_EXTRACT,
                        ]),
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        resolve_provider: Some(false),
                    },
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        tracing::info!("Verum LSP server initialized (incremental)");
        self.client
            .log_message(
                MessageType::INFO,
                "Verum LSP server initialized with incremental parsing",
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        tracing::info!("Shutting down Verum LSP server");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        tracing::debug!("Document opened: {}", params.text_document.uri);

        self.document_cache.open_document(
            params.text_document.uri.clone(),
            params.text_document.text,
            params.text_document.version,
        );

        // Publish diagnostics immediately on open
        self.publish_diagnostics_immediate(params.text_document.uri)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        tracing::debug!("Document changed: {}", params.text_document.uri);

        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        let changes: Vec<_> = params.content_changes;

        // Apply incremental changes
        if let Err(e) = self.document_cache.update_document(&uri, &changes, version) {
            tracing::error!("Failed to update document {}: {}", uri, e);
            return;
        }

        // Log parsing statistics
        if let Some(stats) = self.document_cache.get_stats(&uri) {
            tracing::debug!("Parse stats for {}: {}", uri, stats);
        }

        // Publish diagnostics with debouncing
        self.publish_diagnostics_debounced(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        tracing::debug!("Document saved: {}", params.text_document.uri);

        // Cancel any pending debounced updates
        self.debouncer.cancel(&params.text_document.uri);

        // Publish diagnostics immediately on save
        self.publish_diagnostics_immediate(params.text_document.uri)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        tracing::debug!("Document closed: {}", params.text_document.uri);

        // Cancel any pending updates
        self.debouncer.cancel(&params.text_document.uri);

        // Close the document
        self.document_cache
            .close_document(&params.text_document.uri);

        // Clear diagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        tracing::debug!("Completion requested at {}:{}", uri, position.line);

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            let completions = completion::complete_at_position(&doc, position);
            if completions.is_empty() {
                return Ok(None);
            }
            return Ok(Some(CompletionResponse::Array(
                completions.into_iter().collect(),
            )));
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Hover requested at {}:{}", uri, position.line);

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            // Incremental backend does not own a CbgrHintProvider; hover still
            // works, only the reference-sigil path is disabled here.
            return Ok(hover::hover_at_position(&doc, None, position));
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Go to definition requested at {}:{}", uri, position.line);

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            return Ok(goto_definition::goto_definition(&doc, position, &uri));
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        tracing::debug!("Find references requested at {}:{}", uri, position.line);

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            let refs = references::find_references(&doc, position, &uri, include_declaration);
            if refs.is_empty() {
                return Ok(None);
            }
            return Ok(Some(refs.into_iter().collect()));
        }

        Ok(None)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        tracing::debug!("Prepare rename requested at {}:{}", uri, position.line);

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            return Ok(rename::prepare_rename(&doc, position));
        }

        Ok(None)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        tracing::debug!(
            "Rename requested at {}:{} to {}",
            uri,
            position.line,
            new_name
        );

        // Get document state from cache
        let document = self.document_cache.get_document_state(&uri);

        if let Some(doc) = document {
            return Ok(rename::rename(&doc, position, new_name, &uri));
        }

        Ok(None)
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        tracing::debug!("Format document requested: {}", uri);

        if let Some(text) = self.document_cache.get_text(&uri) {
            let edits = formatting::format_document(&text);
            if edits.is_empty() {
                Ok(None)
            } else {
                Ok(Some(edits.into_iter().collect()))
            }
        } else {
            Ok(None)
        }
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        tracing::debug!("Format range requested: {}", uri);

        if let Some(text) = self.document_cache.get_text(&uri) {
            let edits = formatting::format_range(&text, range);
            if edits.is_empty() {
                Ok(None)
            } else {
                Ok(Some(edits.into_iter().collect()))
            }
        } else {
            Ok(None)
        }
    }

    async fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let ch = params.ch.chars().next().unwrap_or(' ');

        tracing::debug!("Format on type requested: {} at {}", ch, uri);

        if let Some(text) = self.document_cache.get_text(&uri) {
            let edits = formatting::format_on_type(&text, position, ch);
            if edits.is_empty() {
                Ok(None)
            } else {
                Ok(Some(edits.into_iter().collect()))
            }
        } else {
            Ok(None)
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let _range = params.range;
        let _context = params.context;

        tracing::debug!("Code actions requested: {}", uri);

        // Get diagnostics and generate quick fixes
        let verum_diagnostics = self.document_cache.get_diagnostics(&uri);
        let text = self.document_cache.get_text(&uri);

        if let Some(text) = text {
            let mut actions = Vec::new();

            // Generate quick fixes for diagnostics in the requested range
            for diag in verum_diagnostics.iter() {
                let quick_fixes = diagnostics::generate_quick_fixes(diag, &text, &uri);
                for fix in quick_fixes {
                    actions.push(CodeActionOrCommand::CodeAction(fix));
                }
            }

            if actions.is_empty() {
                Ok(None)
            } else {
                Ok(Some(actions))
            }
        } else {
            Ok(None)
        }
    }
}
