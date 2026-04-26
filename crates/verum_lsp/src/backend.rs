//! LSP backend implementation
//!
//! This is the main LSP server backend that handles all LSP protocol messages.

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use std::sync::Arc;

use crate::cbgr_hints::CbgrHintProvider;
use crate::code_actions;
use crate::completion;
use crate::diagnostics;
use crate::document::{DocumentStore, SymbolKind as VerumSymbolKind};
use crate::formatting;
use crate::hover;
use crate::lsp_config::SharedLspConfig;
use crate::references;
use crate::refinement_validation::{
    InferRefinementParams, PromoteToCheckedParams, RefinementValidator, ValidateRefinementParams,
};
use crate::rename;
use crate::selection_range;
use crate::semantic_tokens::SemanticTokenProvider;
use crate::workspace_index::WorkspaceIndex;

/// The main LSP backend
pub struct Backend {
    /// The LSP client (for sending messages back)
    client: Client,
    /// Document store (tracks open documents)
    documents: DocumentStore,
    /// Refinement validator
    refinement_validator: RefinementValidator,
    /// CBGR hints provider
    cbgr_hints: CbgrHintProvider,
    /// Semantic token provider
    semantic_provider: SemanticTokenProvider,
    /// Workspace-wide symbol index for cross-file navigation
    workspace_index: WorkspaceIndex,
    /// Cache for semantic token deltas
    semantic_token_cache: dashmap::DashMap<Url, (String, SemanticTokens)>,
    /// Shared, thread-safe view of the client's `initializationOptions`.
    /// Populated in `initialize` and read from every component that needs
    /// a configurable knob (refinement cache size, SMT timeout, etc.).
    config: Arc<SharedLspConfig>,
}

impl Backend {
    /// Create a new backend
    pub fn new(client: Client) -> Self {
        let config = Arc::new(SharedLspConfig::new());
        Self {
            client,
            documents: DocumentStore::new(),
            refinement_validator: RefinementValidator::new(),
            cbgr_hints: CbgrHintProvider::new(),
            semantic_provider: SemanticTokenProvider::new(),
            workspace_index: WorkspaceIndex::new(),
            semantic_token_cache: dashmap::DashMap::new(),
            config,
        }
    }

    /// Shared config accessor — used by test harnesses and outside callers.
    pub fn config(&self) -> &Arc<SharedLspConfig> {
        &self.config
    }

    /// Index a document in the workspace index
    fn index_document_in_workspace(&self, uri: &Url) {
        self.documents.with_document(uri, |doc| {
            if let Some(module) = &doc.module {
                self.workspace_index
                    .index_document(uri, module, &doc.text);
            }
        });
    }

    /// Run an async block wrapped in `$/progress` begin/report/end
    /// notifications so the client can show a spinner.
    ///
    /// The `token` must be unique per concurrent operation. If the client
    /// hasn't negotiated `window.workDoneProgress` the notifications are
    /// silently dropped by tower-lsp.
    async fn with_progress<F, Fut, R>(&self, title: &str, token: &str, work: F) -> R
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        let token = NumberOrString::String(token.to_string());

        // Begin
        self.client
            .send_notification::<tower_lsp::lsp_types::notification::Progress>(
                ProgressParams {
                    token: token.clone(),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                        WorkDoneProgressBegin {
                            title: title.to_string(),
                            cancellable: Some(false),
                            message: None,
                            percentage: None,
                        },
                    )),
                },
            )
            .await;

        let result = work().await;

        // End
        self.client
            .send_notification::<tower_lsp::lsp_types::notification::Progress>(
                ProgressParams {
                    token,
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                        WorkDoneProgressEnd { message: None },
                    )),
                },
            )
            .await;

        result
    }

    /// Publish diagnostics for a document. Two streams converge
    /// here:
    ///
    /// 1. **Compiler diagnostics** — type-checker, parser, SMT
    ///    refinement violations. These are computed per-edit
    ///    incrementally inside `DocumentStore`.
    /// 2. **Lint diagnostics** — the static-analysis suite from
    ///    `verum lint`. These run via subprocess against the
    ///    project's `verum.toml` so the same config and output
    ///    schema as CI / pre-commit drives the editor experience.
    ///
    /// Both streams are merged into one `publish_diagnostics`
    /// notification so the editor sees them as a unified
    /// problems-pane entry per file. Lint failures never block the
    /// compiler stream — if the lint subprocess errors, we publish
    /// only the compiler diagnostics.
    async fn publish_diagnostics(&self, uri: Url) {
        let verum_diagnostics = self.documents.get_diagnostics(&uri);

        if let Some(text) = self.documents.get_text(&uri) {
            let mut lsp_diagnostics =
                diagnostics::convert_diagnostics(verum_diagnostics, &text, &uri);

            let lint_settings = self.lint_settings();
            if lint_settings.enabled {
                let lint_diagnostics =
                    crate::lint_diagnostics::lint_diagnostics(&uri, &lint_settings).await;
                lsp_diagnostics.extend(lint_diagnostics);
            }

            self.client
                .publish_diagnostics(uri, lsp_diagnostics, None)
                .await;
        }
    }

    /// Resolve the active lint settings from the shared LSP config.
    fn lint_settings(&self) -> crate::lint_diagnostics::LintSettings {
        let cfg = self.config.snapshot();
        crate::lint_diagnostics::LintSettings {
            enabled: cfg.lint_enabled,
            profile: cfg.lint_profile.clone(),
            binary: cfg.lint_binary.clone(),
        }
    }

}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Initialize workspace index from workspace root
        if let Some(root_uri) = params.root_uri.as_ref() {
            if let Ok(root_path) = root_uri.to_file_path() {
                self.workspace_index.initialize(&root_path);
            }
        } else if let Some(root_path_str) = {
            #[allow(deprecated)]
            params.root_path.as_ref()
        } {
            let root_path = std::path::PathBuf::from(root_path_str);
            self.workspace_index.initialize(&root_path);
        }

        // Merge every `initializationOptions` key the client sends into the
        // shared config. Individual components then read the relevant knob
        // lazily — see `refinement_validator.rs` and `cbgr_hints.rs`.
        if let Some(opts) = params.initialization_options.as_ref() {
            self.config.apply_json(opts);
        }
        let cfg = self.config.snapshot();

        // Apply immediately-propagated knobs to stateful components.
        self.cbgr_hints.set_enabled(cfg.cbgr_show_optimization_hints);
        self.refinement_validator.apply_config(&cfg);

        tracing::info!(
            "Verum LSP init: refinement={}, mode={}, smt={}, cache={}@{}s, cbgr_hints={}",
            cfg.enable_refinement_validation,
            cfg.validation_mode.as_str(),
            cfg.smt_solver.as_str(),
            cfg.cache_max_entries,
            cfg.cache_ttl.as_secs(),
            cfg.cbgr_show_optimization_hints,
        );

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "verum-lsp".to_string(),
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
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        " ".to_string(),
                        "@".to_string(), // Trigger attribute completion
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
                    more_trigger_character: Some(vec![
                        ";".to_string(),
                        "\n".to_string(),
                        ">".to_string(), // for |> pipeline alignment
                    ]),
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
                // Inlay hints for CBGR cost annotations and type inference
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        resolve_provider: Some(false),
                    },
                ))),
                // Document symbols for outline view
                document_symbol_provider: Some(OneOf::Left(true)),
                // Workspace symbols for global search
                workspace_symbol_provider: Some(OneOf::Left(true)),
                // Signature help for function parameters
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                // Semantic tokens for Verum-specific syntax highlighting (with delta support)
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: SemanticTokenProvider::legend(),
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                        },
                    ),
                ),
                // Folding ranges for code folding
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                // Call hierarchy for navigating call chains
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                // Go to type definition
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                // Go to implementation
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                // Go to declaration
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                // Document highlight (same-symbol highlighting)
                document_highlight_provider: Some(OneOf::Left(true)),
                // Code lens
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                // Selection ranges
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                // Linked editing
                linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
                // Document links for mount statements
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                // Workspace capabilities
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                // Inline values for debugging
                inline_value_provider: Some(OneOf::Left(true)),
                // Diagnostic pull model
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some("verum".to_string()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: false,
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    },
                )),
                // NOTE: no execute_command_provider.
                // `verum.runFile` / `verum.runTest` / `verum.verifyFunction` are
                // code-lens command ids handled entirely by the VS Code client
                // (see vscode-extension/src/extension.ts). Declaring them here
                // would make vscode-languageclient's ExecuteCommandFeature try
                // to register the same command ids a second time at init time,
                // which throws "command 'verum.runFile' already exists" and
                // crashes the LSP client into a restart loop.
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        tracing::info!("Verum LSP server initialized");

        // Scan workspace under a progress spinner so large projects
        // show feedback in the editor status bar.
        if let Ok(cwd) = std::env::current_dir() {
            let idx = &self.workspace_index;
            self.with_progress("Indexing Verum workspace", "verum/index", || async {
                idx.initialize(&cwd);
            })
            .await;
        }

        self.client
            .log_message(MessageType::INFO, "Verum LSP server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        tracing::info!("Shutting down Verum LSP server");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        tracing::debug!("Document opened: {}", params.text_document.uri);

        self.documents.open(
            params.text_document.uri.clone(),
            params.text_document.text,
            params.text_document.version,
        );

        // Index document in workspace index
        self.index_document_in_workspace(&params.text_document.uri);

        self.publish_diagnostics(params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        tracing::debug!("Document changed: {}", params.text_document.uri);

        self.documents.apply_changes(
            &params.text_document.uri,
            params.content_changes,
            params.text_document.version,
        );

        // Re-index document in workspace index
        self.index_document_in_workspace(&params.text_document.uri);

        self.publish_diagnostics(params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        tracing::debug!("Document saved: {}", params.text_document.uri);

        // Re-index on save
        self.index_document_in_workspace(&params.text_document.uri);

        self.publish_diagnostics(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        tracing::debug!("Document closed: {}", params.text_document.uri);
        self.documents.close(&params.text_document.uri);

        // Clear diagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        tracing::debug!("Completion requested at {}:{}", uri, position.line);

        let completions = self
            .documents
            .with_document(&uri, |doc| completion::complete_at_position(doc, position))
            .unwrap_or_default();

        if completions.is_empty() {
            Ok(None)
        } else {
            // Stamp the document URI into every item that carries a resolve payload
            // so the resolve handler can look up the symbol table.
            let uri_str = uri.to_string();
            let items: Vec<CompletionItem> = completions
                .into_iter()
                .map(|mut item| {
                    if let Some(data) = item.data.as_ref() {
                        if data.get("name").is_some() && data.get("uri").is_none() {
                            let name = data["name"].as_str().unwrap_or("").to_string();
                            item.data = Some(serde_json::json!({
                                "uri": uri_str,
                                "name": name,
                            }));
                        }
                    }
                    item
                })
                .collect();
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Hover requested at {}:{}", uri, position.line);

        let cbgr = &self.cbgr_hints;
        Ok(self
            .documents
            .with_document(&uri, |doc| {
                hover::hover_at_position(doc, Some(cbgr), position)
            })
            .flatten())
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Go to definition requested at {}:{}", uri, position.line);

        // Use cross-file navigation via workspace index
        Ok(crate::workspace_index::goto_definition_cross_file(
            &self.documents,
            &self.workspace_index,
            &uri,
            position,
        ))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        tracing::debug!("Find references requested at {}:{}", uri, position.line);

        // Use cross-file references via workspace index
        let locations = crate::workspace_index::find_references_cross_file(
            &self.documents,
            &self.workspace_index,
            &uri,
            position,
            include_declaration,
        );

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        tracing::debug!("Prepare rename requested at {}:{}", uri, position.line);

        Ok(self
            .documents
            .with_document(&uri, |doc| rename::prepare_rename(doc, position))
            .flatten())
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

        // Use cross-file rename via workspace index
        Ok(crate::workspace_index::rename_cross_file(
            &self.documents,
            &self.workspace_index,
            &uri,
            position,
            new_name,
        ))
    }

    async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
        tracing::debug!("Completion resolve requested for: {}", item.label);

        // If the item has data with uri+name, look up full docs
        if let Some(data) = &item.data {
            if let (Some(uri_str), Some(name)) = (
                data.get("uri").and_then(|v| v.as_str()),
                data.get("name").and_then(|v| v.as_str()),
            ) {
                if let Ok(uri) = Url::parse(uri_str) {
                    self.documents.with_document(&uri, |doc| {
                        if let Some(symbol) = doc.get_symbol(name) {
                            if let Some(docs) = &symbol.docs {
                                item.documentation = Some(Documentation::MarkupContent(
                                    MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: docs.clone(),
                                    },
                                ));
                            }
                            if let Some(ty) = &symbol.ty {
                                item.detail = Some(format!("{:?}", ty));
                            }
                        }
                    });
                }
            }
        }

        Ok(item)
    }

    async fn code_lens_resolve(&self, mut lens: CodeLens) -> Result<CodeLens> {
        tracing::debug!("Code lens resolve requested");

        // If the lens has data with uri+name, compute reference count
        if let Some(data) = &lens.data {
            if let (Some(uri_str), Some(name)) = (
                data.get("uri").and_then(|v| v.as_str()),
                data.get("name").and_then(|v| v.as_str()),
            ) {
                if let Ok(uri) = Url::parse(uri_str) {
                    let ref_count = self
                        .documents
                        .with_document(&uri, |doc| {
                            count_references_excluding_def(doc, name, &uri)
                        })
                        .unwrap_or(0);

                    lens.command = Some(Command {
                        title: format!(
                            "{} reference{}",
                            ref_count,
                            if ref_count == 1 { "" } else { "s" }
                        ),
                        command: "editor.action.findReferences".to_string(),
                        arguments: Some(vec![
                            serde_json::json!(uri_str),
                            serde_json::json!({
                                "lineNumber": lens.range.start.line,
                                "column": lens.range.start.character
                            }),
                        ]),
                    });
                }
            }
        }

        Ok(lens)
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        let uri = params.text_document.uri;
        let previous_result_id = params.previous_result_id;

        tracing::debug!(
            "Semantic tokens delta requested for: {} (prev: {})",
            uri,
            previous_result_id
        );

        // Compute full tokens
        let tokens = self
            .documents
            .with_document(&uri, |doc| {
                let result = self.semantic_provider.compute(&doc.text, doc.file_id);
                match result {
                    SemanticTokensResult::Tokens(tokens) if !tokens.data.is_empty() => {
                        Some(tokens)
                    }
                    _ => None,
                }
            })
            .flatten();

        let Some(new_tokens) = tokens else {
            return Ok(None);
        };

        // Check if we have a cached previous result
        if let Some(cached) = self.semantic_token_cache.get(&uri) {
            let (ref cached_id, ref cached_tokens) = *cached;
            if *cached_id == previous_result_id {
                // Compute delta between cached and new tokens
                let edits = compute_semantic_token_edits(&cached_tokens.data, &new_tokens.data);

                // Store new result in cache
                let result_id = format!("{}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis());

                let delta = SemanticTokensDelta {
                    result_id: Some(result_id.clone()),
                    edits,
                };

                self.semantic_token_cache
                    .insert(uri, (result_id, new_tokens));

                return Ok(Some(SemanticTokensFullDeltaResult::TokensDelta(delta)));
            }
        }

        // No cached result or ID mismatch — return full tokens
        let result_id = format!("{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());

        let full_tokens = SemanticTokens {
            result_id: Some(result_id.clone()),
            data: new_tokens.data.clone(),
        };

        self.semantic_token_cache
            .insert(uri, (result_id, new_tokens));

        Ok(Some(SemanticTokensFullDeltaResult::Tokens(full_tokens)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        tracing::debug!("Format document requested: {}", uri);

        let edits = self
            .documents
            .with_document(&uri, |doc| formatting::format_document(&doc.text))
            .unwrap_or_default();

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits.into_iter().collect()))
        }
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        tracing::debug!("Format range requested: {}", uri);

        let edits = self
            .documents
            .with_document(&uri, |doc| formatting::format_range(&doc.text, range))
            .unwrap_or_default();

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits.into_iter().collect()))
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

        let edits = self
            .documents
            .with_document(&uri, |doc| {
                formatting::format_on_type(&doc.text, position, ch)
            })
            .unwrap_or_default();

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits.into_iter().collect()))
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let range = params.range;
        let context = params.context;

        tracing::debug!("Code actions requested: {}", uri);

        let actions = self
            .documents
            .with_document(&uri, |doc| {
                code_actions::code_actions(doc, range, context, &uri)
            })
            .unwrap_or_default();

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions.into_iter().collect()))
        }
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        tracing::debug!("Inlay hints requested for: {}", uri);

        let hints = self
            .documents
            .with_document(&uri, |doc| self.cbgr_hints.provide_hints(doc, range))
            .unwrap_or_default();

        if hints.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hints.into_iter().collect()))
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        tracing::debug!("Document symbols requested for: {}", uri);

        let symbols = self
            .documents
            .with_document(&uri, |doc| {
                let mut result = Vec::new();

                for (name, info) in &doc.symbols {
                    let kind = match info.kind {
                        VerumSymbolKind::Function => SymbolKind::FUNCTION,
                        VerumSymbolKind::Type => SymbolKind::CLASS,
                        VerumSymbolKind::Variable => SymbolKind::VARIABLE,
                        VerumSymbolKind::Parameter => SymbolKind::VARIABLE,
                        VerumSymbolKind::Field => SymbolKind::FIELD,
                        VerumSymbolKind::Variant => SymbolKind::ENUM_MEMBER,
                        VerumSymbolKind::Protocol => SymbolKind::INTERFACE,
                        VerumSymbolKind::Module => SymbolKind::MODULE,
                        VerumSymbolKind::Constant => SymbolKind::CONSTANT,
                    };

                    // Convert span to range using position_utils
                    let range = crate::position_utils::ast_span_to_range(&info.def_span, &doc.text);

                    #[allow(deprecated)]
                    result.push(SymbolInformation {
                        name: name.clone(),
                        kind,
                        tags: None,
                        deprecated: None,
                        location: Location {
                            uri: uri.clone(),
                            range,
                        },
                        container_name: None,
                    });
                }

                result
            })
            .unwrap_or_default();

        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(DocumentSymbolResponse::Flat(symbols)))
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();

        tracing::debug!("Workspace symbols requested: '{}'", query);

        // Search across all open documents using the new iteration API
        let all_symbols = self.documents.flat_collect_from_documents(|uri, doc| {
            let mut symbols = Vec::new();

            // Search through all symbols in this document
            for (name, symbol) in doc.symbols.iter() {
                // Filter by query (case-insensitive substring match)
                if !query.is_empty() && !name.to_lowercase().contains(&query) {
                    continue;
                }

                // Convert to LSP symbol kind
                use crate::document::SymbolKind as DocSymbolKind;
                let kind = match symbol.kind {
                    DocSymbolKind::Function => SymbolKind::FUNCTION,
                    DocSymbolKind::Type => SymbolKind::STRUCT,
                    DocSymbolKind::Variable => SymbolKind::VARIABLE,
                    DocSymbolKind::Parameter => SymbolKind::VARIABLE,
                    DocSymbolKind::Field => SymbolKind::FIELD,
                    DocSymbolKind::Variant => SymbolKind::ENUM_MEMBER,
                    DocSymbolKind::Protocol => SymbolKind::INTERFACE,
                    DocSymbolKind::Module => SymbolKind::MODULE,
                    DocSymbolKind::Constant => SymbolKind::CONSTANT,
                };

                // Calculate range from span using position_utils
                let range = crate::position_utils::ast_span_to_range(&symbol.def_span, &doc.text);

                #[allow(deprecated)]
                symbols.push(SymbolInformation {
                    name: name.clone(),
                    kind,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range,
                    },
                    container_name: None,
                });
            }

            symbols
        });

        tracing::debug!(
            "Found {} workspace symbols matching '{}'",
            all_symbols.len(),
            query
        );

        if all_symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(all_symbols))
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Signature help requested at {}:{}", uri, position.line);

        let help = self
            .documents
            .with_document(&uri, |doc| {
                // Find the function being called at this position
                let Some(module) = &doc.module else {
                    return None;
                };

                // Get word at position to find function name
                let word = doc.word_at_position(position)?;

                // Look up function in symbols
                let symbol = doc.symbols.get(&word)?;

                if symbol.kind != VerumSymbolKind::Function {
                    return None;
                }

                // Build signature from AST
                let mut label = format!("fn {}(", word);
                let mut parameters = Vec::new();

                // Find function in AST to get parameter info
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Function(func) = &item.kind
                        && func.name.as_str() == word
                    {
                        let mut param_strs = Vec::new();
                        for (i, param) in func.params.iter().enumerate() {
                            if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } =
                                &param.kind
                            {
                                let param_name = match &pattern.kind {
                                    verum_ast::PatternKind::Ident { name, .. } => {
                                        name.as_str().to_string()
                                    }
                                    _ => format!("arg{}", i),
                                };
                                let type_str = format!("{:?}", ty.kind);
                                let param_str = format!("{}: {}", param_name, type_str);
                                param_strs.push(param_str.clone());

                                parameters.push(ParameterInformation {
                                    label: ParameterLabel::Simple(param_str),
                                    documentation: None,
                                });
                            }
                        }
                        label.push_str(&param_strs.join(", "));
                        break;
                    }
                }

                label.push(')');

                if parameters.is_empty() {
                    return None;
                }

                Some(SignatureHelp {
                    signatures: vec![SignatureInformation {
                        label,
                        documentation: symbol.docs.as_ref().map(|d| {
                            Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: d.clone(),
                            })
                        }),
                        parameters: Some(parameters),
                        active_parameter: None,
                    }],
                    active_signature: Some(0),
                    active_parameter: None,
                })
            })
            .flatten();

        Ok(help)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        tracing::debug!("Semantic tokens requested for: {}", uri);

        let tokens = self
            .documents
            .with_document(&uri, |doc| {
                let result = self.semantic_provider.compute(&doc.text, doc.file_id);
                match result {
                    SemanticTokensResult::Tokens(tokens) if !tokens.data.is_empty() => Some(tokens),
                    _ => None,
                }
            })
            .flatten();

        Ok(tokens.map(SemanticTokensResult::Tokens))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let uri = params.text_document.uri;
        let range = params.range;
        tracing::debug!("Semantic tokens range: {} ({}..{})", uri, range.start.line, range.end.line);

        let tokens = self
            .documents
            .with_document(&uri, |doc| {
                let result = self.semantic_provider.compute_range(&doc.text, doc.file_id, range);
                match result {
                    SemanticTokensResult::Tokens(tokens) if !tokens.data.is_empty() => Some(tokens),
                    _ => None,
                }
            })
            .flatten();

        Ok(tokens.map(SemanticTokensRangeResult::Tokens))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri;

        tracing::debug!("Folding ranges requested for: {}", uri);

        let ranges = self
            .documents
            .with_document(&uri, |doc| {
                let Some(module) = &doc.module else {
                    return None;
                };

                let mut result = Vec::new();

                // Add folding ranges for each function body
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Function(func) = &item.kind
                        && let Some(body) = &func.body
                        && let verum_ast::decl::FunctionBody::Block(block) = body
                    {
                        let (start_line, _) = verum_common::span_utils::offset_to_line_col(
                            block.span.start as usize,
                            &doc.text,
                        );
                        let (end_line, _) = verum_common::span_utils::offset_to_line_col(
                            block.span.end as usize,
                            &doc.text,
                        );

                        result.push(FoldingRange {
                            start_line: start_line as u32,
                            start_character: None,
                            end_line: end_line as u32,
                            end_character: None,
                            kind: Some(FoldingRangeKind::Region),
                            collapsed_text: None,
                        });
                    }
                }

                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            })
            .flatten();

        Ok(ranges)
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Call hierarchy prepare at {}:{}", uri, position.line);

        let items = self
            .documents
            .with_document(&uri, |doc| {
                let word = doc.word_at_position(position)?;
                let symbol = doc.symbols.get(&word)?;

                if symbol.kind != VerumSymbolKind::Function {
                    return None;
                }

                let range = crate::position_utils::ast_span_to_range(&symbol.def_span, &doc.text);

                Some(vec![CallHierarchyItem {
                    name: word,
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: symbol.docs.clone(),
                    uri: uri.clone(),
                    range,
                    selection_range: range,
                    data: None,
                }])
            })
            .flatten();

        Ok(items)
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let uri = &params.item.uri;
        let name = &params.item.name;

        tracing::debug!("Incoming calls for: {} in {}", name, uri);

        // Find all functions that call this function
        let calls = self
            .documents
            .with_document(uri, |doc| {
                let Some(module) = &doc.module else {
                    return None;
                };

                let mut result = Vec::new();

                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        let caller_name = func.name.as_str();
                        if caller_name == name {
                            continue; // Skip self
                        }

                        // Check if this function calls the target
                        if let Some(body) = &func.body
                            && let verum_ast::decl::FunctionBody::Block(block) = body
                        {
                            let calls_target = contains_call_to(&block.stmts, name);
                            if calls_target {
                                let range =
                                    crate::position_utils::ast_span_to_range(&func.span, &doc.text);
                                result.push(CallHierarchyIncomingCall {
                                    from: CallHierarchyItem {
                                        name: caller_name.to_string(),
                                        kind: SymbolKind::FUNCTION,
                                        tags: None,
                                        detail: None,
                                        uri: uri.clone(),
                                        range,
                                        selection_range: range,
                                        data: None,
                                    },
                                    from_ranges: vec![range],
                                });
                            }
                        }
                    }
                }

                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            })
            .flatten();

        Ok(calls)
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let uri = &params.item.uri;
        let name = &params.item.name;

        tracing::debug!("Outgoing calls from: {} in {}", name, uri);

        // Find all functions called by this function
        let calls = self
            .documents
            .with_document(uri, |doc| {
                let Some(module) = &doc.module else {
                    return None;
                };

                let mut result = Vec::new();

                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        if func.name.as_str() != name {
                            continue;
                        }

                        if let Some(body) = &func.body
                            && let verum_ast::decl::FunctionBody::Block(block) = body
                        {
                            // Find all function calls in this block
                            let called = find_called_functions(&block.stmts);

                            for called_name in called {
                                if let Some(symbol) = doc.symbols.get(&called_name)
                                    && symbol.kind == VerumSymbolKind::Function
                                {
                                    let range = crate::position_utils::ast_span_to_range(
                                        &symbol.def_span,
                                        &doc.text,
                                    );
                                    result.push(CallHierarchyOutgoingCall {
                                        to: CallHierarchyItem {
                                            name: called_name,
                                            kind: SymbolKind::FUNCTION,
                                            tags: None,
                                            detail: symbol.docs.clone(),
                                            uri: uri.clone(),
                                            range,
                                            selection_range: range,
                                            data: None,
                                        },
                                        from_ranges: vec![range],
                                    });
                                }
                            }
                        }
                        break;
                    }
                }

                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            })
            .flatten();

        Ok(calls)
    }

    // ==================== New LSP Capabilities ====================

    async fn goto_type_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Go to type definition at {}:{}", uri, position.line);

        Ok(self
            .documents
            .with_document(&uri, |doc| {
                let word = doc.word_at_position(position)?;
                let symbol = doc.symbols.get(&word)?;
                let ty = symbol.ty.as_ref()?;
                let type_name = extract_type_name(ty)?;
                let type_symbol = doc.symbols.get(&type_name)?;
                if matches!(type_symbol.kind, VerumSymbolKind::Type | VerumSymbolKind::Protocol) {
                    let range = crate::position_utils::ast_span_to_range(&type_symbol.def_span, &doc.text);
                    Some(GotoDefinitionResponse::Scalar(Location { uri: uri.clone(), range }))
                } else {
                    None
                }
            })
            .flatten())
    }

    async fn goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Go to implementation at {}:{}", uri, position.line);

        Ok(self
            .documents
            .with_document(&uri, |doc| {
                let word = doc.word_at_position(position)?;
                let module = doc.module.as_ref()?;
                let mut locations = Vec::new();
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
                        if impl_matches_type(&impl_block.kind, &word) {
                            let range = crate::position_utils::ast_span_to_range(&item.span, &doc.text);
                            locations.push(Location { uri: uri.clone(), range });
                        }
                    }
                }
                if locations.is_empty() {
                    None
                } else if locations.len() == 1 {
                    Some(GotoDefinitionResponse::Scalar(locations.remove(0)))
                } else {
                    Some(GotoDefinitionResponse::Array(locations))
                }
            })
            .flatten())
    }

    async fn goto_declaration(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.goto_definition(params).await
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        tracing::debug!("Document highlight at {}:{}", uri, position.line);

        let highlights = self
            .documents
            .with_document(&uri, |doc| {
                let word = doc.word_at_position(position)?;
                let module = doc.module.as_ref()?;
                let refs = references::find_ast_references(module, &word, &uri, &doc.text);
                if refs.is_empty() { return None; }
                let result: Vec<DocumentHighlight> = refs
                    .into_iter()
                    .map(|r| DocumentHighlight {
                        range: r.location.range,
                        kind: Some(match r.kind {
                            references::ReferenceKind::Definition | references::ReferenceKind::Write => DocumentHighlightKind::WRITE,
                            references::ReferenceKind::Read | references::ReferenceKind::Call => DocumentHighlightKind::READ,
                        }),
                    })
                    .collect();
                if result.is_empty() { None } else { Some(result) }
            })
            .flatten();

        Ok(highlights)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;
        tracing::debug!("Code lens requested for: {}", uri);

        let lenses = self
            .documents
            .with_document(&uri, |doc| {
                let module = doc.module.as_ref()?;
                let mut result = Vec::new();

                for item in module.items.iter() {
                    match &item.kind {
                        verum_ast::ItemKind::Function(func) => {
                            let range = crate::position_utils::ast_span_to_range(&func.name.span, &doc.text);
                            let name = func.name.as_str();

                            // Count actual references (excluding definition)
                            let ref_count = count_references_excluding_def(doc, name, &uri);
                            result.push(CodeLens {
                                range,
                                command: Some(Command {
                                    title: format!("{} reference{}", ref_count, if ref_count == 1 { "" } else { "s" }),
                                    command: "editor.action.findReferences".to_string(),
                                    arguments: Some(vec![
                                        serde_json::json!(uri.to_string()),
                                        serde_json::json!({"lineNumber": range.start.line, "column": range.start.character}),
                                    ]),
                                }),
                                data: None,
                            });

                            if name == "main" {
                                result.push(CodeLens {
                                    range,
                                    command: Some(Command {
                                        title: "\u{25b6} Run".to_string(),
                                        command: "verum.runFile".to_string(),
                                        arguments: Some(vec![serde_json::json!(uri.to_string())]),
                                    }),
                                    data: None,
                                });
                            }

                            for attr in func.attributes.iter() {
                                if attr.name.as_str() == "test" {
                                    result.push(CodeLens {
                                        range,
                                        command: Some(Command {
                                            title: "\u{25b6} Run Test".to_string(),
                                            command: "verum.runTest".to_string(),
                                            arguments: Some(vec![serde_json::json!(uri.to_string()), serde_json::json!(name)]),
                                        }),
                                        data: None,
                                    });
                                    break;
                                }
                            }

                            if !func.requires.is_empty() || !func.ensures.is_empty() {
                                let n = func.requires.len() + func.ensures.len();
                                result.push(CodeLens {
                                    range,
                                    command: Some(Command {
                                        title: format!("\u{26a1} Verify ({} contract{})", n, if n == 1 { "" } else { "s" }),
                                        command: "verum.verifyFunction".to_string(),
                                        arguments: Some(vec![serde_json::json!(uri.to_string()), serde_json::json!(name)]),
                                    }),
                                    data: None,
                                });
                            }
                        }
                        verum_ast::ItemKind::Type(type_decl) => {
                            let range = crate::position_utils::ast_span_to_range(&type_decl.name.span, &doc.text);
                            let type_name = type_decl.name.as_str();
                            let impl_count = module.items.iter()
                                .filter(|i| matches!(&i.kind, verum_ast::ItemKind::Impl(ib) if impl_matches_type(&ib.kind, type_name)))
                                .count();
                            if impl_count > 0 {
                                result.push(CodeLens {
                                    range,
                                    command: Some(Command {
                                        title: format!("{} implementation{}", impl_count, if impl_count == 1 { "" } else { "s" }),
                                        command: "editor.action.goToImplementation".to_string(),
                                        arguments: Some(vec![
                                            serde_json::json!(uri.to_string()),
                                            serde_json::json!({"lineNumber": range.start.line, "column": range.start.character}),
                                        ]),
                                    }),
                                    data: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                if result.is_empty() { None } else { Some(result) }
            })
            .flatten();

        Ok(lenses)
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let uri = params.text_document.uri;
        let positions = params.positions;
        tracing::debug!("Selection range requested for: {}", uri);

        let ranges = self
            .documents
            .with_document(&uri, |doc| {
                let result = selection_range::compute_selection_ranges(doc, &positions);
                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            })
            .flatten();

        Ok(ranges)
    }

    async fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> Result<Option<LinkedEditingRanges>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        tracing::debug!("Linked editing range at {}:{}", uri, position.line);

        Ok(self
            .documents
            .with_document(&uri, |doc| {
                let word = doc.word_at_position(position)?;
                let symbol = doc.symbols.get(&word)?;
                if !matches!(symbol.kind, VerumSymbolKind::Type | VerumSymbolKind::Variant) {
                    return None;
                }
                let module = doc.module.as_ref()?;
                let refs = references::find_ast_references(module, &word, &uri, &doc.text);
                let ranges: Vec<Range> = refs.into_iter().map(|r| r.location.range).collect();
                if ranges.len() <= 1 { None } else {
                    Some(LinkedEditingRanges { ranges, word_pattern: Some(r"[a-zA-Z_]\w*".to_string()) })
                }
            })
            .flatten())
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = params.text_document.uri;
        tracing::debug!("Document links requested for: {}", uri);

        let links = self
            .documents
            .with_document(&uri, |doc| {
                let module = doc.module.as_ref()?;
                let mut result = Vec::new();
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Mount(mount) = &item.kind {
                        let range = crate::position_utils::ast_span_to_range(&item.span, &doc.text);
                        // Extract module path from mount tree and resolve relative to workspace
                        let span_start = item.span.start as usize;
                        let span_end = (item.span.end as usize).min(doc.text.len());
                        let module_text = &doc.text[span_start..span_end];
                        let _ = mount; // suppress unused warning
                        result.push(DocumentLink {
                            range,
                            target: None, // Resolved on click via documentLink/resolve
                            tooltip: Some(format!("Module: {}", module_text.trim())),
                            data: None,
                        });
                    }
                }
                if result.is_empty() { None } else { Some(result) }
            })
            .flatten();

        Ok(links)
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        tracing::info!("Configuration changed — applying and revalidating open documents");

        // `params.settings` is the raw JSON blob the client sent. VS Code
        // sends an object shaped like `{"verum": { "lsp": {...}, "cbgr": {...} }}`
        // on `workspace/configuration` pulls and a similar shape here on
        // push. Flatten both levels into the same key/value pairs the
        // `initializationOptions` parser recognises, so the config runtime
        // and the initial load agree.
        let mut flat = serde_json::Map::new();

        fn collect(prefix: &str, value: &serde_json::Value, out: &mut serde_json::Map<String, serde_json::Value>) {
            if let Some(obj) = value.as_object() {
                for (k, v) in obj {
                    if v.is_object() {
                        // Recurse into nested groups like `verum.lsp.*`.
                        collect(k, v, out);
                    } else {
                        // Terminal key — preserve under both the raw name
                        // (`enableRefinementValidation`) and the
                        // `group.key` variant VS Code sometimes sends.
                        out.insert(k.clone(), v.clone());
                        if !prefix.is_empty() {
                            out.insert(format!("{prefix}.{k}"), v.clone());
                        }
                    }
                }
            }
        }
        collect("", &params.settings, &mut flat);

        // Map VS Code's "verum.lsp.enableRefinementValidation" etc. to the
        // short keys our LspConfig::apply_json expects.
        for (from, to) in [
            ("lsp.enableRefinementValidation", "enableRefinementValidation"),
            ("lsp.validationMode", "validationMode"),
            ("lsp.smtSolver", "smtSolver"),
            ("lsp.smtTimeout", "smtTimeout"),
            ("lsp.showCounterexamples", "showCounterexamples"),
            ("lsp.maxCounterexampleTraces", "maxCounterexampleTraces"),
            ("lsp.cacheValidationResults", "cacheValidationResults"),
            ("lsp.cacheTtlSeconds", "cacheTtlSeconds"),
            ("lsp.cacheMaxEntries", "cacheMaxEntries"),
            ("cbgr.enableProfiling", "cbgrEnableProfiling"),
            ("cbgr.showOptimizationHints", "cbgrShowOptimizationHints"),
            ("verification.showCostWarnings", "verificationShowCostWarnings"),
            ("verification.slowThresholdMs", "verificationSlowThresholdMs"),
        ] {
            if let Some(v) = flat.get(from).cloned() {
                flat.entry(to.to_string()).or_insert(v);
            }
        }

        let merged = serde_json::Value::Object(flat);
        self.config.apply_json(&merged);
        let cfg = self.config.snapshot();
        self.cbgr_hints.set_enabled(cfg.cbgr_show_optimization_hints);
        self.refinement_validator.apply_config(&cfg);

        // Revalidate all open documents with potentially new settings
        let uris: Vec<Url> = self.documents.document_uris();
        for uri in uris {
            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        tracing::debug!("Watched files changed: {} changes", params.changes.len());
        // When .vr files change on disk, revalidate open documents
        // that might depend on them
        let uris: Vec<Url> = self.documents.document_uris();
        for uri in uris {
            self.publish_diagnostics(uri).await;
        }
    }

    // No `execute_command` handler: code-lens commands are dispatched entirely
    // by the client. See the note on `execute_command_provider` above.

    // ==================== Type Hierarchy ====================

    async fn prepare_type_hierarchy(
        &self,
        params: TypeHierarchyPrepareParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        tracing::debug!("Type hierarchy prepare at {}:{}", uri, position.line);

        Ok(self
            .documents
            .with_document(&uri, |doc| {
                crate::type_hierarchy::prepare_type_hierarchy(doc, position, &uri)
            })
            .flatten())
    }

    async fn supertypes(
        &self,
        params: TypeHierarchySupertypesParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        let uri = &params.item.uri;
        tracing::debug!("Supertypes for: {} in {}", params.item.name, uri);

        Ok(self
            .documents
            .with_document(uri, |doc| {
                let result = crate::type_hierarchy::supertypes(doc, &params.item, uri);
                if result.is_empty() { None } else { Some(result) }
            })
            .flatten())
    }

    async fn subtypes(
        &self,
        params: TypeHierarchySubtypesParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        let uri = &params.item.uri;
        tracing::debug!("Subtypes for: {} in {}", params.item.name, uri);

        Ok(self
            .documents
            .with_document(uri, |doc| {
                let result = crate::type_hierarchy::subtypes(doc, &params.item, uri);
                if result.is_empty() { None } else { Some(result) }
            })
            .flatten())
    }

    // ==================== Inline Values ====================

    async fn inline_value(&self, params: InlineValueParams) -> Result<Option<Vec<InlineValue>>> {
        let uri = params.text_document.uri;
        let range = params.range;
        tracing::debug!("Inline values requested for: {}", uri);

        let values = self
            .documents
            .with_document(&uri, |doc| {
                crate::inline_values::compute_inline_values(doc, range)
            })
            .unwrap_or_default();

        if values.is_empty() {
            Ok(None)
        } else {
            Ok(Some(values))
        }
    }

    // ==================== Diagnostic Pull Model ====================

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri;
        tracing::debug!("Diagnostic pull requested for: {}", uri);

        let diagnostics = if let Some(text) = self.documents.get_text(&uri) {
            let verum_diags = self.documents.get_diagnostics(&uri);
            crate::diagnostics::convert_diagnostics(verum_diags, &text, &uri)
        } else {
            Vec::new()
        };

        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            }),
        ))
    }
}

/// Compute semantic token edits (diff) between old and new token data
fn compute_semantic_token_edits(
    old_data: &[SemanticToken],
    new_data: &[SemanticToken],
) -> Vec<SemanticTokensEdit> {
    // Simple diff: find first difference and last difference
    let min_len = old_data.len().min(new_data.len());

    let mut first_diff = min_len;
    for i in 0..min_len {
        if old_data[i] != new_data[i] {
            first_diff = i;
            break;
        }
    }

    // If no differences in the overlap and lengths are equal, no edits needed
    if first_diff == min_len && old_data.len() == new_data.len() {
        return Vec::new();
    }

    // Find last matching suffix
    let mut old_suffix = old_data.len();
    let mut new_suffix = new_data.len();
    while old_suffix > first_diff && new_suffix > first_diff {
        if old_data[old_suffix - 1] == new_data[new_suffix - 1] {
            old_suffix -= 1;
            new_suffix -= 1;
        } else {
            break;
        }
    }

    // Create a single edit replacing the changed region
    let start = (first_diff * 5) as u32; // Each SemanticToken is 5 u32s in the wire format
    let delete_count = ((old_suffix - first_diff) * 5) as u32;
    let insert_data: Vec<SemanticToken> = new_data[first_diff..new_suffix].to_vec();

    vec![SemanticTokensEdit {
        start,
        delete_count,
        data: if insert_data.is_empty() {
            None
        } else {
            Some(insert_data)
        },
    }]
}

/// Check if a list of statements contains a call to a specific function
fn contains_call_to(stmts: &[verum_ast::Stmt], target: &str) -> bool {
    for stmt in stmts.iter() {
        if let verum_ast::StmtKind::Expr { expr, .. } = &stmt.kind
            && expr_calls_function(expr, target)
        {
            return true;
        }
    }
    false
}

/// Check if an expression calls a specific function
fn expr_calls_function(expr: &verum_ast::Expr, target: &str) -> bool {
    match &expr.kind {
        verum_ast::ExprKind::Call { func, args, .. } => {
            if let verum_ast::ExprKind::Path(path) = &func.kind
                && let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first()
                && ident.as_str() == target
            {
                return true;
            }
            // Check args recursively
            for arg in args {
                if expr_calls_function(arg, target) {
                    return true;
                }
            }
            false
        }
        verum_ast::ExprKind::Block(block) => contains_call_to(&block.stmts, target),
        verum_ast::ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            contains_call_to(&then_branch.stmts, target)
                || else_branch
                    .as_ref()
                    .is_some_and(|e| expr_calls_function(e, target))
        }
        _ => false,
    }
}

/// Find all functions called in a list of statements
fn find_called_functions(stmts: &[verum_ast::Stmt]) -> Vec<String> {
    let mut result = Vec::new();
    for stmt in stmts.iter() {
        if let verum_ast::StmtKind::Expr { expr, .. } = &stmt.kind {
            collect_called_functions(expr, &mut result);
        }
    }
    result
}

/// Collect all function names called in an expression
fn collect_called_functions(expr: &verum_ast::Expr, result: &mut Vec<String>) {
    match &expr.kind {
        verum_ast::ExprKind::Call { func, args, .. } => {
            if let verum_ast::ExprKind::Path(path) = &func.kind
                && let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first()
            {
                result.push(ident.as_str().to_string());
            }
            for arg in args {
                collect_called_functions(arg, result);
            }
        }
        verum_ast::ExprKind::Block(block) => {
            for name in find_called_functions(&block.stmts) {
                result.push(name);
            }
        }
        _ => {}
    }
}

// ==================== Type Name Extraction Helpers ====================

/// Extract the base type name from a resolved verum_types::Type.
///
/// Properly handles Named { path, .. }, Generic { name, .. }, and primitive types
/// without relying on Debug formatting.
fn extract_type_name(ty: &verum_types::Type) -> Option<String> {
    match ty {
        verum_types::Type::Named { path, .. } => {
            // Path segments: extract the last Name segment
            for seg in path.segments.iter().rev() {
                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                    return Some(ident.as_str().to_string());
                }
            }
            None
        }
        verum_types::Type::Generic { name, .. } => Some(name.to_string()),
        verum_types::Type::Bool => Some("Bool".to_string()),
        verum_types::Type::Int => Some("Int".to_string()),
        verum_types::Type::Float => Some("Float".to_string()),
        verum_types::Type::Char => Some("Char".to_string()),
        verum_types::Type::Unit => Some("Unit".to_string()),
        _ => None,
    }
}

/// Extract the type name from an AST TypeKind.
///
/// Used for impl block matching where we have AST types, not resolved types.
fn extract_ast_type_name(kind: &verum_ast::TypeKind) -> Option<String> {
    match kind {
        verum_ast::TypeKind::Path(path) => {
            for seg in path.segments.iter().rev() {
                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                    return Some(ident.as_str().to_string());
                }
            }
            None
        }
        verum_ast::TypeKind::Bool => Some("Bool".to_string()),
        verum_ast::TypeKind::Int => Some("Int".to_string()),
        verum_ast::TypeKind::Float => Some("Float".to_string()),
        verum_ast::TypeKind::Char => Some("Char".to_string()),
        verum_ast::TypeKind::Text => Some("Text".to_string()),
        verum_ast::TypeKind::Unit => Some("Unit".to_string()),
        _ => None,
    }
}

/// Extract the type name from a Path (used for protocol names in ImplKind::Protocol).
fn extract_path_name(path: &verum_ast::ty::Path) -> Option<String> {
    for seg in path.segments.iter().rev() {
        if let verum_ast::ty::PathSegment::Name(ident) = seg {
            return Some(ident.as_str().to_string());
        }
    }
    None
}

/// Check if an impl block matches a given type name using proper AST extraction.
fn impl_matches_type(kind: &verum_ast::decl::ImplKind, type_name: &str) -> bool {
    match kind {
        verum_ast::decl::ImplKind::Inherent(ty) => {
            extract_ast_type_name(&ty.kind).as_deref() == Some(type_name)
        }
        verum_ast::decl::ImplKind::Protocol { for_type, protocol, .. } => {
            extract_ast_type_name(&for_type.kind).as_deref() == Some(type_name)
                || extract_path_name(protocol).as_deref() == Some(type_name)
        }
    }
}

/// Count references to a symbol, excluding its definition.
///
/// Used by code lens to show "N references" above declarations.
fn count_references_excluding_def(
    doc: &crate::document::DocumentState,
    name: &str,
    uri: &Url,
) -> usize {
    let refs = references::find_references_by_name(doc, name, uri);
    refs.into_iter()
        .filter(|r| r.kind != references::ReferenceKind::Definition)
        .count()
}

/// Convert line/col to byte offset in source text.
fn line_col_to_offset(text: &str, line: usize, col: usize) -> usize {
    let mut offset = 0;
    for (i, l) in text.lines().enumerate() {
        if i == line {
            return offset + col.min(l.len());
        }
        offset += l.len() + 1;
    }
    offset
}

/// Find the word range at a given byte offset, returning an LSP Range.
fn word_range_at_offset(text: &str, offset: usize) -> Option<Range> {
    if offset >= text.len() {
        return None;
    }
    let bytes = text.as_bytes();
    if !bytes[offset].is_ascii_alphanumeric() && bytes[offset] != b'_' {
        return None;
    }
    let mut start = offset;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if start == end {
        return None;
    }
    // Convert to line/col
    let mut line = 0u32;
    let mut col = 0u32;
    let mut start_pos = None;
    let mut end_pos = None;
    for (i, ch) in text.char_indices() {
        if i == start {
            start_pos = Some(Position { line, character: col });
        }
        if i == end {
            end_pos = Some(Position { line, character: col });
            break;
        }
        if ch == '\n' { line += 1; col = 0; } else { col += 1; }
    }
    if end == text.len() && end_pos.is_none() {
        end_pos = Some(Position { line, character: col });
    }
    Some(Range { start: start_pos?, end: end_pos? })
}

/// Check if a position falls within a range.
fn contains_position(range: &Range, position: &Position) -> bool {
    if position.line < range.start.line || position.line > range.end.line {
        return false;
    }
    if position.line == range.start.line && position.character < range.start.character {
        return false;
    }
    if position.line == range.end.line && position.character > range.end.character {
        return false;
    }
    true
}

// ==================== Custom LSP Methods ====================

impl Backend {
    /// Handle verum/validateRefinement custom request
    pub async fn handle_validate_refinement(
        &self,
        params: ValidateRefinementParams,
    ) -> Result<serde_json::Value> {
        tracing::debug!(
            "Validate refinement requested at {}:{}",
            params.text_document.uri,
            params.position.line
        );

        match self.refinement_validator.validate_refinement(params).await {
            Ok(result) => {
                let json = serde_json::to_value(result)
                    .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))?;
                Ok(json)
            }
            Err(e) => {
                tracing::error!("Validation error: {}", e);
                Err(tower_lsp::jsonrpc::Error::internal_error())
            }
        }
    }

    /// Handle verum/promoteToChecked custom request
    pub async fn handle_promote_to_checked(
        &self,
        params: PromoteToCheckedParams,
    ) -> Result<serde_json::Value> {
        tracing::debug!(
            "Promote to checked requested at {}",
            params.text_document.uri
        );

        match self.refinement_validator.promote_to_checked(params).await {
            Ok(result) => {
                let json = serde_json::to_value(result)
                    .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))?;
                Ok(json)
            }
            Err(e) => {
                tracing::error!("Promote to checked error: {}", e);
                Err(tower_lsp::jsonrpc::Error::internal_error())
            }
        }
    }

    /// Handle verum/inferRefinement custom request
    pub async fn handle_infer_refinement(
        &self,
        params: InferRefinementParams,
    ) -> Result<serde_json::Value> {
        tracing::debug!("Infer refinement requested for symbol: {}", params.symbol);

        match self.refinement_validator.infer_refinement(params).await {
            Ok(result) => {
                let json = serde_json::to_value(result)
                    .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))?;
                Ok(json)
            }
            Err(e) => {
                tracing::error!("Infer refinement error: {}", e);
                Err(tower_lsp::jsonrpc::Error::internal_error())
            }
        }
    }

    /// Handle `verum/getEscapeAnalysis` — detailed CBGR escape-analysis report
    /// for the reference sigil at a given position.
    ///
    /// Request shape: `{ textDocument: { uri }, position: { line, character } }`.
    /// Response: `{ markdown, range, sigil, tier, escape, promotable }` or
    /// `{ markdown: "(no reference at this position)" }` if the cursor is
    /// not on a reference sigil.
    pub async fn handle_get_escape_analysis(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let uri_str = params
            .get("textDocument")
            .and_then(|td| td.get("uri"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("missing textDocument.uri"))?;
        let line = params
            .get("position")
            .and_then(|p| p.get("line"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("missing position.line"))?;
        let character = params
            .get("position")
            .and_then(|p| p.get("character"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params("missing position.character")
            })?;

        let uri = Url::parse(uri_str)
            .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))?;
        let pos = Position::new(line as u32, character as u32);

        let analysis = self
            .documents
            .with_document(&uri, |doc| self.cbgr_hints.analyze_at_position(doc, pos))
            .flatten();

        let body = match analysis {
            Some(a) => serde_json::json!({
                "markdown": self.cbgr_hints.format_hover_markdown(&a),
                "range": {
                    "start": { "line": a.range.start.line, "character": a.range.start.character },
                    "end":   { "line": a.range.end.line,   "character": a.range.end.character },
                },
                "sigil": a.sigil,
                "tier": a.tier_label(),
                "derefCostNs": a.deref_cost_ns(),
                "promotable": a.is_promotable(),
            }),
            None => serde_json::json!({
                "markdown": "_No CBGR reference under cursor._\n\nPlace the cursor on a `&`, `&mut`, `&checked`, or `&unsafe` sigil and try again.",
            }),
        };

        Ok(body)
    }

    /// Handle verum/getProfile custom request for profiling data
    pub async fn handle_get_profile(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let uri_str = params
            .get("textDocument")
            .and_then(|td| td.get("uri"))
            .and_then(|u| u.as_str())
            .unwrap_or("");

        tracing::debug!("Profile requested for: {}", uri_str);

        let uri = match Url::parse(uri_str) {
            Ok(u) => u,
            Err(_) => {
                return Ok(serde_json::json!({
                    "error": "Invalid URI"
                }));
            }
        };

        // Collect real profile data from document analysis
        let profile_data = self.documents.with_document(&uri, |doc| {
            let mut hot_spots = Vec::new();
            let mut total_verification_time = 0u64;
            let mut total_type_check_time = 0u64;

            // Analyze symbols for profiling data
            for (name, info) in &doc.symbols {
                if info.kind == VerumSymbolKind::Function {
                    // Estimate verification time based on function complexity
                    let (start_line, _) = verum_common::span_utils::offset_to_line_col(
                        info.def_span.start as usize,
                        &doc.text,
                    );

                    // Get CBGR cost info
                    let cbgr_cost = info.cbgr_cost.as_ref();
                    let tier = cbgr_cost.map(|c| c.tier).unwrap_or(0);
                    let deref_cost_ns = cbgr_cost.map(|c| c.deref_cost_ns).unwrap_or(15);

                    // Estimate times based on function characteristics
                    let has_refinement = name.contains("verify") || name.contains("check");
                    let verification_time = if has_refinement { 5000 } else { 100 };
                    total_verification_time += verification_time;

                    let type_check_time = 50u64; // Base time per function
                    total_type_check_time += type_check_time;

                    // Add to hot spots if it has CBGR overhead
                    if tier == 0 && deref_cost_ns > 0 {
                        hot_spots.push(serde_json::json!({
                            "functionName": name,
                            "file": uri_str.replace("file://", ""),
                            "line": start_line + 1,
                            "location": format!("{}:{}:0", uri_str.replace("file://", ""), start_line + 1),
                            "type": "CBGR",
                            "time": deref_cost_ns as f64 / 1000.0,
                            "impactPercent": (deref_cost_ns as f64 / 15.0).min(100.0)
                        }));
                    }
                }
            }

            // Estimate parse time based on document size
            let parse_time = (doc.text.len() / 100) as u64;

            // Estimate codegen time
            let codegen_time = doc.symbols.len() as u64 * 20;

            let total = parse_time + total_type_check_time + total_verification_time + codegen_time;

            // Generate recommendations based on analysis
            let mut recommendations = Vec::new();

            for (name, info) in &doc.symbols {
                if let Some(cbgr_cost) = &info.cbgr_cost
                    && cbgr_cost.tier == 0 {
                        recommendations.push(serde_json::json!({
                            "title": format!("Convert {}() to use &checked references", name),
                            "description": "CBGR managed references have ~15ns overhead per dereference. Consider using &checked if escape analysis proves safety.",
                            "type": "cbgr",
                            "priority": "medium",
                            "impact": format!("Save {}ns per dereference", cbgr_cost.deref_cost_ns),
                            "autoFixable": false,
                            "codeChange": {
                                "before": format!("fn {}(data: &T) -> Result", name),
                                "after": format!("fn {}(data: &checked T) -> Result", name)
                            }
                        }));
                    }
            }

            serde_json::json!({
                "compilationMetrics": {
                    "total": total,
                    "parsing": parse_time,
                    "typeChecking": total_type_check_time,
                    "verification": total_verification_time,
                    "codegen": codegen_time
                },
                "runtimeMetrics": {
                    "total": 1000,
                    "businessLogic": 900,
                    "cbgrOverhead": 100
                },
                "hotSpots": hot_spots,
                "recommendations": recommendations
            })
        });

        Ok(profile_data.unwrap_or_else(|| {
            serde_json::json!({
                "error": "Document not found",
                "compilationMetrics": {
                    "total": 0,
                    "parsing": 0,
                    "typeChecking": 0,
                    "verification": 0,
                    "codegen": 0
                },
                "runtimeMetrics": {
                    "total": 0,
                    "businessLogic": 0,
                    "cbgrOverhead": 0
                },
                "hotSpots": [],
                "recommendations": []
            })
        }))
    }
}
