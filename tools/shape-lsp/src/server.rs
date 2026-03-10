//! Main LSP server implementation
//!
//! Implements the Language Server Protocol for Shape.

use crate::annotation_discovery::AnnotationDiscovery;
use crate::call_hierarchy::{
    incoming_calls as ch_incoming, outgoing_calls as ch_outgoing,
    prepare_call_hierarchy as ch_prepare,
};
use crate::code_actions::get_code_actions;
use crate::code_lens::{get_code_lenses, resolve_code_lens};
use crate::completion::get_completions_with_context;
use crate::definition::{get_definition, get_references_with_fallback};
use crate::diagnostics::error_to_diagnostic;
use crate::document::DocumentManager;
use crate::document_symbols::{get_document_symbols, get_workspace_symbols};
use crate::folding::get_folding_ranges;
use crate::formatting::{format_document, format_on_type, format_range};
use crate::hover::get_hover;
use crate::inlay_hints::{InlayHintConfig, get_inlay_hints_with_context};
use crate::rename::{prepare_rename, rename};
use crate::semantic_tokens::{get_legend, get_semantic_tokens};
use crate::signature_help::get_signature_help;
use crate::util::{
    mask_leading_prefix_for_parse, offset_to_line_col, parser_source, position_to_offset,
};
use dashmap::DashMap;
use shape_ast::ParseErrorKind;
use shape_ast::ast::Program;
use shape_ast::parser::parse_program;
use std::collections::HashSet;
use tower_lsp_server::ls_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CallHierarchyServerCapability, CodeActionKind, CodeActionOptions, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CodeLens, CodeLensOptions, CodeLensParams,
    CompletionOptions, CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeConfigurationParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentOnTypeFormattingOptions,
    DocumentOnTypeFormattingParams, DocumentRangeFormattingParams, DocumentSymbolParams,
    DocumentSymbolResponse, FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, InlayHint, InlayHintOptions,
    InlayHintParams, InlayHintServerCapabilities, Location, MessageType, OneOf, Position,
    PrepareRenameResponse, Range, ReferenceParams, RenameOptions, RenameParams, SemanticToken,
    SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, SignatureHelp,
    SignatureHelpOptions, SignatureHelpParams, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri, WorkDoneProgressOptions,
    WorkspaceEdit, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use tower_lsp_server::{Client, LanguageServer, jsonrpc::Result};

/// The main Shape Language Server
pub struct ShapeLanguageServer {
    /// LSP client for sending notifications and requests
    client: Client,
    /// Document manager for tracking open files
    documents: DocumentManager,
    /// Project root detected from workspace folder via shape.toml
    project_root: std::sync::OnceLock<std::path::PathBuf>,
    /// Cache of last successfully parsed programs per URI.
    /// Used as fallback when current parse fails (e.g., during editing).
    last_good_programs: DashMap<Uri, Program>,
    /// Manager for child language servers handling foreign function blocks.
    foreign_lsp: crate::foreign_lsp::ForeignLspManager,
}

impl ShapeLanguageServer {
    /// Create a new language server instance
    pub fn new(client: Client) -> Self {
        // Use current directory as default workspace; updated in initialize().
        let default_workspace =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        Self {
            client,
            documents: DocumentManager::new(),
            project_root: std::sync::OnceLock::new(),
            last_good_programs: DashMap::new(),
            foreign_lsp: crate::foreign_lsp::ForeignLspManager::new(default_workspace),
        }
    }

    /// Check if a URI points to a shape.toml file.
    fn is_shape_toml(uri: &Uri) -> bool {
        uri.as_str().ends_with("shape.toml")
    }

    /// Analyze a shape.toml document and publish diagnostics.
    async fn analyze_toml_document(&self, uri: &Uri) {
        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return,
        };

        let text = doc.text();
        let diagnostics = crate::toml_support::diagnostics::validate_toml(&text);

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// Analyze a document and publish diagnostics
    async fn analyze_document(&self, uri: &Uri) {
        // Get the document
        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return,
        };

        // Parse the document
        let text = doc.text();

        // Validate frontmatter if present (check for project-level sections)
        let mut frontmatter_diagnostics = Vec::new();
        let frontmatter_prefix_len;
        {
            use shape_runtime::frontmatter::{
                FrontmatterDiagnosticSeverity, parse_frontmatter, parse_frontmatter_validated,
            };
            let (config, fm_diags, rest) = parse_frontmatter_validated(&text);
            frontmatter_prefix_len = text.len().saturating_sub(rest.len());
            for diag in fm_diags {
                let severity = match diag.severity {
                    FrontmatterDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                    FrontmatterDiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                };
                let range = diag
                    .location
                    .map(|loc| Range {
                        start: Position {
                            line: loc.line,
                            character: loc.character,
                        },
                        end: Position {
                            line: loc.line,
                            character: loc.character + loc.length.max(1),
                        },
                    })
                    .unwrap_or_else(frontmatter_fallback_range);
                frontmatter_diagnostics.push(Diagnostic {
                    range,
                    severity: Some(severity),
                    message: diag.message,
                    source: Some("shape".to_string()),
                    ..Default::default()
                });
            }

            // Frontmatter and shape.toml are mutually exclusive.
            if config.is_some() {
                if let (Some(project_root), Some(path)) =
                    (self.project_root.get(), uri.to_file_path())
                {
                    if path.as_ref().starts_with(project_root) {
                        frontmatter_diagnostics.push(Diagnostic {
                            range: Range {
                                start: Position {
                                    line: 0,
                                    character: 0,
                                },
                                end: Position {
                                    line: 0,
                                    character: 3,
                                },
                            },
                            severity: Some(DiagnosticSeverity::ERROR),
                            message: "Frontmatter and shape.toml are mutually exclusive; use one configuration source.".to_string(),
                            source: Some("shape".to_string()),
                            ..Default::default()
                        });
                    }
                }
            }

            // Validate frontmatter extension paths when running as a standalone script.
            if let (Some(frontmatter), Some(script_path)) =
                (parse_frontmatter(&text).0, uri.to_file_path())
            {
                let path_ranges = frontmatter_extension_path_ranges(&text);
                if let Some(script_dir) = script_path.as_ref().parent() {
                    for (index, extension) in frontmatter.extensions.into_iter().enumerate() {
                        let resolved = if extension.path.is_absolute() {
                            extension.path.clone()
                        } else {
                            script_dir.join(&extension.path)
                        };
                        if !resolved.exists() {
                            let range = path_ranges
                                .get(index)
                                .cloned()
                                .unwrap_or_else(frontmatter_fallback_range);
                            frontmatter_diagnostics.push(Diagnostic {
                                range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: format!(
                                    "Extension '{}' path does not exist: {}",
                                    extension.name,
                                    resolved.display()
                                ),
                                source: Some("shape".to_string()),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }

        let parse_source = mask_leading_prefix_for_parse(&text, frontmatter_prefix_len);
        let diagnostics = match parse_program(parse_source.as_ref()) {
            Ok(program) => {
                // Parsing succeeded — cache the good program for fallback
                self.last_good_programs.insert(uri.clone(), program.clone());

                // Update virtual documents for foreign function blocks
                let file_path = uri.to_file_path();
                let foreign_startup_diagnostics = self
                    .foreign_lsp
                    .update_documents(
                        uri.as_str(),
                        &text,
                        &program.items,
                        file_path.as_deref(),
                        self.project_root.get().map(|p| p.as_path()),
                    )
                    .await;

                let module_cache = self.documents.get_module_cache();
                let mut diagnostics = crate::analysis::analyze_program_semantics(
                    &program,
                    &text,
                    uri.to_file_path().as_deref(),
                    Some(&module_cache),
                    self.project_root.get().map(|p| p.as_path()),
                );
                diagnostics.extend(crate::doc_diagnostics::validate_program_docs(
                    &program,
                    &text,
                    Some(&module_cache),
                    uri.to_file_path().as_deref(),
                    self.project_root.get().map(|p| p.as_path()),
                ));
                diagnostics.extend(foreign_startup_diagnostics);
                diagnostics.extend(self.foreign_lsp.get_diagnostics(uri.as_str()).await);
                diagnostics
            }
            Err(error) => {
                // Parsing failed — try resilient parse for partial results
                let partial = shape_ast::parse_program_resilient(parse_source.as_ref());
                let mut diagnostics = Vec::new();
                let has_non_grammar_partial_error = partial
                    .errors
                    .iter()
                    .any(|e| !matches!(e.kind, ParseErrorKind::GrammarFailure));

                // Prefer strict parser diagnostics for pure grammar failures.
                // Add resilient diagnostics when they provide specific recovered spans/messages.
                if partial.errors.is_empty() || !has_non_grammar_partial_error {
                    diagnostics.extend(error_to_diagnostic(&error));
                }

                // Add diagnostics for recovery errors
                if has_non_grammar_partial_error {
                    for parse_error in &partial.errors {
                        if matches!(parse_error.kind, ParseErrorKind::GrammarFailure) {
                            continue;
                        }
                        let (start_line, start_col) = offset_to_line_col(&text, parse_error.span.0);
                        let (end_line, end_col) = offset_to_line_col(&text, parse_error.span.1);
                        diagnostics.push(Diagnostic {
                            range: Range {
                                start: Position {
                                    line: start_line,
                                    character: start_col,
                                },
                                end: Position {
                                    line: end_line,
                                    character: end_col,
                                },
                            },
                            severity: Some(DiagnosticSeverity::ERROR),
                            message: parse_error.message.clone(),
                            source: Some("shape".to_string()),
                            ..Default::default()
                        });
                    }
                }

                if !partial.items.is_empty() {
                    // Cache the partial result so hover/completions have something to work with
                    self.last_good_programs
                        .insert(uri.clone(), partial.into_program());
                }

                diagnostics
            }
        };

        // Combine frontmatter diagnostics with parse/semantic diagnostics
        let mut all_diagnostics = frontmatter_diagnostics;
        all_diagnostics.extend(diagnostics);

        // Publish diagnostics to the client
        self.client
            .publish_diagnostics(uri.clone(), all_diagnostics, None)
            .await;
    }
}

impl LanguageServer for ShapeLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Log initialization
        self.client
            .log_message(MessageType::INFO, "Shape Language Server initializing")
            .await;

        // Detect project root from workspace folders
        let mut workspace_folder: Option<std::path::PathBuf> = None;
        if let Some(folders) = params.workspace_folders.as_ref() {
            if let Some(folder) = folders.first() {
                if let Some(folder_path) = folder.uri.to_file_path() {
                    workspace_folder = Some(folder_path.to_path_buf());
                    if let Some(project) = shape_runtime::project::find_project_root(&folder_path) {
                        self.client
                            .log_message(
                                MessageType::INFO,
                                format!(
                                    "Detected project root: {} ({})",
                                    project.config.project.name,
                                    project.root_path.display()
                                ),
                            )
                            .await;
                        let _ = self.project_root.set(project.root_path);
                    }
                }
            }
        }

        // Update the foreign LSP manager's workspace dir so child servers
        // receive the correct rootUri and can discover project config files
        // (e.g. pyrightconfig.json, virtualenvs).
        if let Some(dir) = self
            .project_root
            .get()
            .cloned()
            .or_else(|| workspace_folder.clone())
        {
            self.foreign_lsp.set_workspace_dir(dir);
        }

        let workspace_hint = self
            .project_root
            .get()
            .map(|path| path.as_path())
            .or(workspace_folder.as_deref());
        let configured_extensions = configured_extensions_from_lsp_value(
            params.initialization_options.as_ref(),
            workspace_hint,
        );
        if !configured_extensions.is_empty() {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "Configured {} always-load extension(s) from LSP initialization options",
                        configured_extensions.len()
                    ),
                )
                .await;
        }
        self.foreign_lsp
            .set_configured_extensions(configured_extensions)
            .await;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // Use full text document sync (incremental requires proper range handling)
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),

                // Enable completion support
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        "(".to_string(),
                        " ".to_string(),
                        "@".to_string(),
                        ":".to_string(),
                    ]),
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                    all_commit_characters: None,
                    completion_item: None,
                }),

                // Enable hover support
                hover_provider: Some(HoverProviderCapability::Simple(true)),

                // Enable signature help
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                }),

                // Enable document symbols (outline view)
                document_symbol_provider: Some(OneOf::Left(true)),

                // Enable workspace symbols (find symbol across workspace)
                workspace_symbol_provider: Some(OneOf::Left(true)),

                // Enable go-to-definition
                definition_provider: Some(OneOf::Left(true)),

                // Enable find references
                references_provider: Some(OneOf::Left(true)),

                // Enable semantic tokens for syntax highlighting
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions {
                                work_done_progress: None,
                            },
                            legend: get_legend(),
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),

                // Enable inlay hints (type hints, parameter hints)
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                        resolve_provider: Some(false),
                    },
                ))),

                // Enable code actions (quick fixes, refactoring)
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR,
                            CodeActionKind::REFACTOR_EXTRACT,
                            CodeActionKind::REFACTOR_REWRITE,
                            CodeActionKind::SOURCE,
                            CodeActionKind::SOURCE_ORGANIZE_IMPORTS,
                            CodeActionKind::SOURCE_FIX_ALL,
                        ]),
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                        resolve_provider: Some(false),
                    },
                )),

                // Enable document formatting
                document_formatting_provider: Some(OneOf::Left(true)),

                // Enable range formatting
                document_range_formatting_provider: Some(OneOf::Left(true)),

                // Enable on-type formatting for indentation while typing.
                document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                    first_trigger_character: "}".to_string(),
                    more_trigger_character: Some(vec!["\n".to_string()]),
                }),

                // Enable rename support
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                })),

                // Enable code lens
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(true),
                }),

                // Enable folding ranges
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),

                // Enable call hierarchy
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),

                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "Shape Language Server".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            ..InitializeResult::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Shape Language Server initialized")
            .await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let workspace_hint = self.project_root.get().map(|path| path.as_path());
        let configured_extensions =
            configured_extensions_from_lsp_value(Some(&params.settings), workspace_hint);
        self.foreign_lsp
            .set_configured_extensions(configured_extensions.clone())
            .await;
        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "Updated always-load extensions from configuration change ({} configured)",
                    configured_extensions.len()
                ),
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.client
            .log_message(MessageType::INFO, "Shape Language Server shutting down")
            .await;
        // Gracefully shut down all child language servers
        self.foreign_lsp.shutdown().await;
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let text = params.text_document.text;

        self.client
            .log_message(
                MessageType::INFO,
                format!("Document opened: {}", uri.as_str()),
            )
            .await;

        // Store the document
        self.documents.open(uri.clone(), version, text);

        // Route to TOML analysis for shape.toml files
        if Self::is_shape_toml(&uri) {
            self.analyze_toml_document(&uri).await;
            return;
        }

        // Analyze and publish diagnostics
        self.analyze_document(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        // For now, we only handle full document updates
        // In the future, we can optimize this to handle incremental changes
        if let Some(change) = params.content_changes.into_iter().next() {
            let text = change.text;
            self.documents.update(&uri, version, text);

            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Document changed: {} (version {})", uri.as_str(), version),
                )
                .await;

            // Route to TOML analysis for shape.toml files
            if Self::is_shape_toml(&uri) {
                self.analyze_toml_document(&uri).await;
                return;
            }

            // Analyze and publish diagnostics
            self.analyze_document(&uri).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        self.client
            .log_message(
                MessageType::INFO,
                format!("Document closed: {}", uri.as_str()),
            )
            .await;

        // Clear diagnostics for the closed document
        self.client
            .publish_diagnostics(uri.clone(), vec![], None)
            .await;

        // Remove cached program for closed document
        self.last_good_programs.remove(&uri);

        self.documents.close(&uri);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "Completion requested at {}:{}:{}",
                    uri.as_str(),
                    position.line,
                    position.character
                ),
            )
            .await;

        // Route to TOML completions for shape.toml files
        if Self::is_shape_toml(&uri) {
            let doc = match self.documents.get(&uri) {
                Some(doc) => doc,
                None => return Ok(None),
            };
            let text = doc.text();
            let items = crate::toml_support::completions::get_toml_completions(&text, position);
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Route to frontmatter completions for script frontmatter blocks.
        if is_position_in_frontmatter(&text, position) {
            let items =
                crate::toml_support::completions::get_frontmatter_completions(&text, position);
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // Check if the cursor is inside a foreign function body
        if let Some(cached_program) = self.last_good_programs.get(&uri) {
            if crate::foreign_lsp::is_position_in_foreign_block(
                &cached_program.items,
                &text,
                position,
            ) {
                let completions = self
                    .foreign_lsp
                    .handle_completion(uri.as_str(), position, &cached_program.items, &text)
                    .await;
                if let Some(items) = completions {
                    return Ok(Some(CompletionResponse::Array(items)));
                }
                // If delegation failed, fall through to Shape completions
            }
        }

        let cached_symbols = self.documents.get_cached_symbols(&uri);
        let cached_types = self.documents.get_cached_types(&uri);

        // Get completions (with cached symbols as fallback)
        let module_cache = self.documents.get_module_cache();
        let file_path = uri.to_file_path();
        let (completions, updated_symbols, updated_types) = get_completions_with_context(
            &text,
            position,
            &cached_symbols,
            &cached_types,
            Some(&module_cache),
            file_path.as_deref(),
            self.project_root.get().map(|p| p.as_path()),
        );

        // Update cached symbols if parsing succeeded
        if let Some(symbols) = updated_symbols {
            self.documents.update_cached_symbols(&uri, symbols);
        }
        if let Some(types) = updated_types {
            self.documents.update_cached_types(&uri, types);
        }

        Ok(Some(CompletionResponse::Array(completions)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "Hover requested at {}:{}:{}",
                    uri.as_str(),
                    position.line,
                    position.character
                ),
            )
            .await;

        // Route to TOML hover for shape.toml files
        if Self::is_shape_toml(&uri) {
            let doc = match self.documents.get(&uri) {
                Some(doc) => doc,
                None => return Ok(None),
            };
            let text = doc.text();
            return Ok(crate::toml_support::hover::get_toml_hover(&text, position));
        }

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Check if the cursor is inside a foreign function body
        if let Some(cached_program) = self.last_good_programs.get(&uri) {
            if crate::foreign_lsp::is_position_in_foreign_block(
                &cached_program.items,
                &text,
                position,
            ) {
                let hover = self
                    .foreign_lsp
                    .handle_hover(uri.as_str(), position, &cached_program.items, &text)
                    .await;
                if hover.is_some() {
                    return Ok(hover);
                }
                // If delegation failed, fall through to Shape hover
            }
        }

        // Get hover information, passing module cache for imported symbol lookup
        let module_cache = self.documents.get_module_cache();
        let file_path = uri.to_file_path();
        let cached = self.last_good_programs.get(&uri);
        let cached_ref = cached.as_ref().map(|r| r.value());
        let hover = get_hover(
            &text,
            position,
            Some(&module_cache),
            file_path.as_deref(),
            cached_ref,
        );

        Ok(hover)
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        if let Some(cached_program) = self.last_good_programs.get(&uri) {
            if crate::foreign_lsp::is_position_in_foreign_block(
                &cached_program.items,
                &text,
                position,
            ) {
                let signature_help = self
                    .foreign_lsp
                    .handle_signature_help(uri.as_str(), position, &cached_program.items, &text)
                    .await;
                if signature_help.is_some() {
                    return Ok(signature_help);
                }
            }
        }

        // Get signature help
        let sig_help = get_signature_help(&text, position);

        Ok(sig_help)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Get document symbols
        let symbols = get_document_symbols(&text);

        Ok(symbols)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let query = params.query;
        let mut all_symbols = Vec::new();

        // Get symbols from all open documents
        for uri in self.documents.all_uris() {
            if let Some(doc) = self.documents.get(&uri) {
                let text = doc.text();
                let symbols = get_workspace_symbols(&text, &uri, &query);
                all_symbols.extend(symbols);
            }
        }

        if all_symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(WorkspaceSymbolResponse::Flat(all_symbols)))
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        if let Some(cached_program) = self.last_good_programs.get(&uri) {
            if crate::foreign_lsp::is_position_in_foreign_block(
                &cached_program.items,
                &text,
                position,
            ) {
                let definition = self
                    .foreign_lsp
                    .handle_definition(uri.as_str(), position, &cached_program.items, &text)
                    .await;
                if definition.is_some() {
                    return Ok(definition);
                }
            }
        }

        // Get module cache for cross-file navigation
        let module_cache = self.documents.get_module_cache();

        // Discover annotations for go-to-definition on annotation names
        let mut annotation_discovery = AnnotationDiscovery::new();
        let parse_source = parser_source(&text);
        if let Ok(program) = parse_program(parse_source.as_ref()) {
            annotation_discovery.discover_from_program(&program);
            if let Some(file_path) = uri.to_file_path() {
                annotation_discovery.discover_from_imports_with_cache(
                    &program,
                    &file_path,
                    &module_cache,
                    self.project_root.get().map(|p| p.as_path()),
                );
            }
        }

        let cached = self.last_good_programs.get(&uri);
        let cached_ref = cached.as_ref().map(|r| r.value());
        let definition = get_definition(
            &text,
            position,
            &uri,
            Some(&module_cache),
            Some(&annotation_discovery),
            cached_ref,
        );

        Ok(definition)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        if let Some(cached_program) = self.last_good_programs.get(&uri) {
            if crate::foreign_lsp::is_position_in_foreign_block(
                &cached_program.items,
                &text,
                position,
            ) {
                let references = self
                    .foreign_lsp
                    .handle_references(uri.as_str(), position, &cached_program.items, &text)
                    .await;
                if references.is_some() {
                    return Ok(references);
                }
            }
        }

        // Get references with cached program fallback
        let cached = self.last_good_programs.get(&uri);
        let cached_ref = cached.as_ref().map(|r| r.value());
        let references = get_references_with_fallback(&text, position, &uri, cached_ref);

        Ok(references)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;

        self.client
            .log_message(
                MessageType::INFO,
                format!("Semantic tokens requested for {}", uri.as_str()),
            )
            .await;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Base Shape semantic tokens
        let mut tokens = get_semantic_tokens(&text);

        if let Some(ref mut base_tokens) = tokens {
            let mut absolute = decode_semantic_tokens(&base_tokens.data);

            let frontmatter_tokens =
                crate::toml_support::semantic_tokens::collect_frontmatter_semantic_tokens(&text);
            absolute.extend(
                frontmatter_tokens
                    .into_iter()
                    .map(|token| AbsoluteSemanticToken {
                        line: token.line,
                        start_char: token.start_char,
                        length: token.length,
                        token_type: token.token_type,
                        modifiers: token.modifiers,
                    }),
            );

            if self.last_good_programs.contains_key(&uri) {
                let foreign_tokens = self.foreign_lsp.collect_semantic_tokens(uri.as_str()).await;
                absolute.extend(
                    foreign_tokens
                        .into_iter()
                        .map(|token| AbsoluteSemanticToken {
                            line: token.line,
                            start_char: token.start_char,
                            length: token.length,
                            token_type: token.token_type,
                            modifiers: token.token_modifiers_bitset,
                        }),
                );
            }

            absolute.sort_by_key(|token| {
                (
                    token.line,
                    token.start_char,
                    token.length,
                    token.token_type,
                    token.modifiers,
                )
            });
            absolute.dedup_by_key(|token| {
                (
                    token.line,
                    token.start_char,
                    token.length,
                    token.token_type,
                    token.modifiers,
                )
            });
            base_tokens.data = encode_semantic_tokens(&absolute);
        }

        Ok(tokens.map(SemanticTokensResult::Tokens))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Get inlay hints with default config, using cached program as fallback
        let config = InlayHintConfig::default();
        let cached = self.last_good_programs.get(&uri);
        let cached_ref = cached.as_ref().map(|r| r.value());
        let file_path = uri.to_file_path();
        let hints = get_inlay_hints_with_context(
            &text,
            range,
            &config,
            cached_ref,
            file_path.as_deref(),
            self.project_root.get().map(|p| p.as_path()),
        );

        if hints.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hints))
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let range = params.range;
        let diagnostics = params.context.diagnostics;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Get code actions
        let module_cache = self.documents.get_module_cache();
        let actions = get_code_actions(
            &text,
            &uri,
            range,
            &diagnostics,
            Some(&module_cache),
            params.context.only.as_deref(),
        );

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Format the document
        let edits = format_document(&text, &params.options);

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Format the range
        let edits = format_range(&text, range, &params.options);

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }

    async fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();
        let edits = format_on_type(&text, position, &params.ch, &params.options);

        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Prepare rename
        let response = prepare_rename(&text, position);

        Ok(response)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        let cached = self.last_good_programs.get(&uri);
        let cached_ref = cached.as_ref().map(|r| r.value());
        let edit = rename(&text, &uri, position, &new_name, cached_ref);

        Ok(edit)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;

        // Get the document
        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();

        // Get code lenses
        let lenses = get_code_lenses(&text, &uri);

        if lenses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(lenses))
        }
    }

    async fn code_lens_resolve(&self, lens: CodeLens) -> Result<CodeLens> {
        Ok(resolve_code_lens(lens))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri;

        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();
        let parse_source = parser_source(&text);
        let program = match parse_program(parse_source.as_ref()) {
            Ok(p) => p,
            Err(_) => {
                // Fall back to cached program
                match self.last_good_programs.get(&uri) {
                    Some(cached) => cached.value().clone(),
                    None => return Ok(None),
                }
            }
        };

        let ranges = get_folding_ranges(&text, &program);
        if ranges.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ranges))
        }
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.documents.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();
        Ok(ch_prepare(&text, position, &uri))
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let uri = &params.item.uri;

        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();
        let results = ch_incoming(&text, &params.item, uri);
        if results.is_empty() {
            Ok(None)
        } else {
            Ok(Some(results))
        }
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let uri = &params.item.uri;

        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let text = doc.text();
        let results = ch_outgoing(&text, &params.item, uri);
        if results.is_empty() {
            Ok(None)
        } else {
            Ok(Some(results))
        }
    }
}

fn frontmatter_fallback_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 3,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AbsoluteSemanticToken {
    line: u32,
    start_char: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

fn decode_semantic_tokens(tokens: &[SemanticToken]) -> Vec<AbsoluteSemanticToken> {
    let mut decoded = Vec::with_capacity(tokens.len());
    let mut line = 0u32;
    let mut col = 0u32;

    for token in tokens {
        line += token.delta_line;
        if token.delta_line == 0 {
            col += token.delta_start;
        } else {
            col = token.delta_start;
        }
        decoded.push(AbsoluteSemanticToken {
            line,
            start_char: col,
            length: token.length,
            token_type: token.token_type,
            modifiers: token.token_modifiers_bitset,
        });
    }

    decoded
}

fn encode_semantic_tokens(tokens: &[AbsoluteSemanticToken]) -> Vec<SemanticToken> {
    let mut encoded = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for token in tokens {
        let delta_line = token.line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            token.start_char.saturating_sub(prev_start)
        } else {
            token.start_char
        };
        encoded.push(SemanticToken {
            delta_line,
            delta_start,
            length: token.length,
            token_type: token.token_type,
            token_modifiers_bitset: token.modifiers,
        });
        prev_line = token.line;
        prev_start = token.start_char;
    }

    encoded
}

fn frontmatter_extension_path_ranges(source: &str) -> Vec<Range> {
    let lines: Vec<&str> = source.split('\n').collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let delimiter_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_end_matches('\r').trim();
            if trimmed == "---" { Some(idx) } else { None }
        })
        .take(2)
        .collect();

    if delimiter_lines.len() < 2 {
        return Vec::new();
    }

    let start = delimiter_lines[0] + 1;
    let end = delimiter_lines[1];
    let mut in_extensions = false;
    let mut ranges = Vec::new();

    for (line_idx, raw_line) in lines.iter().enumerate().take(end).skip(start) {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("[[extensions]]") {
            in_extensions = true;
            continue;
        }

        if trimmed.starts_with("[[") || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
            in_extensions = false;
            continue;
        }

        if !in_extensions {
            continue;
        }

        let Some(eq_pos) = raw_line.find('=') else {
            continue;
        };
        let key = raw_line[..eq_pos].trim();
        if key != "path" {
            continue;
        }

        let key_start = raw_line.find("path").unwrap_or_else(|| {
            raw_line[..eq_pos]
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(0)
        });
        let line_len = raw_line.trim_end_matches('\r').len();
        let end_char = line_len.max(key_start + 4);

        ranges.push(Range {
            start: Position {
                line: line_idx as u32,
                character: key_start as u32,
            },
            end: Position {
                line: line_idx as u32,
                character: end_char as u32,
            },
        });
    }

    ranges
}

fn is_position_in_frontmatter(source: &str, position: Position) -> bool {
    if source.starts_with("#!") && position.line == 0 {
        return false;
    }

    let (_, _, rest) = shape_runtime::frontmatter::parse_frontmatter_validated(source);
    let prefix_len = source.len().saturating_sub(rest.len());
    if prefix_len == 0 {
        return false;
    }

    position_to_offset(source, position)
        .map(|offset| offset < prefix_len)
        .unwrap_or(false)
}

fn configured_extensions_from_lsp_value(
    options: Option<&serde_json::Value>,
    workspace_root: Option<&std::path::Path>,
) -> Vec<crate::foreign_lsp::ConfiguredExtensionSpec> {
    let mut specs = collect_configured_extensions_from_options(options, workspace_root);

    // Auto-discover globally installed extensions from ~/.shape/extensions/
    collect_global_extensions(&mut specs);

    dedup_extension_specs(specs)
}

/// Parse configured extensions from LSP options JSON only (no global discovery).
fn collect_configured_extensions_from_options(
    options: Option<&serde_json::Value>,
    workspace_root: Option<&std::path::Path>,
) -> Vec<crate::foreign_lsp::ConfiguredExtensionSpec> {
    let mut specs = Vec::new();

    if let Some(options) = options {
        collect_configured_extensions_from_array(
            options.get("alwaysLoadExtensions"),
            workspace_root,
            &mut specs,
        );
        collect_configured_extensions_from_array(
            options.get("always_load_extensions"),
            workspace_root,
            &mut specs,
        );

        if let Some(shape) = options.get("shape") {
            collect_configured_extensions_from_array(
                shape.get("alwaysLoadExtensions"),
                workspace_root,
                &mut specs,
            );
            collect_configured_extensions_from_array(
                shape.get("always_load_extensions"),
                workspace_root,
                &mut specs,
            );
        }
    }

    specs
}

fn dedup_extension_specs(
    specs: Vec<crate::foreign_lsp::ConfiguredExtensionSpec>,
) -> Vec<crate::foreign_lsp::ConfiguredExtensionSpec> {
    let mut seen = HashSet::new();
    specs
        .into_iter()
        .filter(|spec| {
            let key = format!(
                "{}|{}|{}",
                spec.name,
                spec.path.display(),
                serde_json::to_string(&spec.config).unwrap_or_default()
            );
            seen.insert(key)
        })
        .collect()
}

fn collect_global_extensions(out: &mut Vec<crate::foreign_lsp::ConfiguredExtensionSpec>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let ext_dir = home.join(".shape").join("extensions");
    if !ext_dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&ext_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_lib = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext == "so" || ext == "dylib" || ext == "dll")
            .unwrap_or(false);
        if !is_lib {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| {
                s.strip_prefix("libshape_ext_")
                    .or_else(|| s.strip_prefix("shape_ext_"))
                    .unwrap_or(s)
                    .to_string()
            })
            .unwrap_or_else(|| "extension".to_string());
        out.push(crate::foreign_lsp::ConfiguredExtensionSpec {
            name,
            path,
            config: serde_json::json!({}),
        });
    }
}

fn collect_configured_extensions_from_array(
    value: Option<&serde_json::Value>,
    workspace_root: Option<&std::path::Path>,
    out: &mut Vec<crate::foreign_lsp::ConfiguredExtensionSpec>,
) {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return;
    };
    for item in items {
        if let Some(spec) = parse_configured_extension_item(item, workspace_root) {
            out.push(spec);
        }
    }
}

fn parse_configured_extension_item(
    item: &serde_json::Value,
    workspace_root: Option<&std::path::Path>,
) -> Option<crate::foreign_lsp::ConfiguredExtensionSpec> {
    let (name, path, config) = if let Some(path) = item.as_str() {
        let path_buf = resolve_configured_extension_path(path, workspace_root);
        (
            configured_extension_name_from_path(&path_buf),
            path_buf,
            serde_json::json!({}),
        )
    } else if let Some(obj) = item.as_object() {
        let path_str = obj.get("path")?.as_str()?;
        let path_buf = resolve_configured_extension_path(path_str, workspace_root);
        let name = obj
            .get("name")
            .and_then(|value| value.as_str())
            .map(String::from)
            .unwrap_or_else(|| configured_extension_name_from_path(&path_buf));
        let config = obj
            .get("config")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        (name, path_buf, config)
    } else {
        return None;
    };

    Some(crate::foreign_lsp::ConfiguredExtensionSpec { name, path, config })
}

fn resolve_configured_extension_path(
    path: &str,
    workspace_root: Option<&std::path::Path>,
) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(path);
    if path.is_absolute() {
        return path;
    }
    workspace_root.map(|root| root.join(&path)).unwrap_or(path)
}

fn configured_extension_name_from_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(String::from)
        .unwrap_or_else(|| "configured-extension".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::parser_source;
    use tower_lsp_server::LspService;

    #[tokio::test]
    async fn test_server_creation() {
        let (service, _socket) = LspService::new(|client| ShapeLanguageServer::new(client));

        // Just verify we can create the service
        drop(service);
    }

    #[test]
    fn test_frontmatter_extension_path_ranges_points_to_path_line() {
        let source = r#"---
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
let x = 1
"#;

        let ranges = frontmatter_extension_path_ranges(source);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start.line, 3);
        assert_eq!(ranges[0].start.character, 0);
    }

    #[test]
    fn test_frontmatter_extension_path_ranges_handles_shebang() {
        let source = r#"#!/usr/bin/env shape
---
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
let x = 1
"#;

        let ranges = frontmatter_extension_path_ranges(source);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start.line, 4);
    }

    #[test]
    fn test_validate_imports_accepts_namespace_import_from_frontmatter_extension() {
        let source = r#"---
# shape.toml
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
use duckdb
let conn = duckdb.connect("duckdb://analytics.db")
"#;

        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("script.shape");
        std::fs::write(&file_path, source).unwrap();

        let parse_src = parser_source(source);
        let program = parse_program(parse_src.as_ref()).expect("program should parse");
        let module_cache = crate::module_cache::ModuleCache::new();
        let mut compiler = shape_vm::BytecodeCompiler::new();

        let diagnostics = crate::analysis::validate_imports_and_register_items(
            &program,
            source,
            &file_path,
            &module_cache,
            None,
            &mut compiler,
        );

        assert!(
            diagnostics.iter().all(|diag| {
                !diag.message.contains("Cannot resolve module ''")
                    && !diag.message.contains("Cannot resolve module 'duckdb'")
            }),
            "namespace import from frontmatter extension should not emit resolution errors: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_imports_reports_unknown_namespace_module_name() {
        let source = "use missingmod\nlet x = 1\n";
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("script.shape");
        std::fs::write(&file_path, source).unwrap();

        let parse_src = parser_source(source);
        let program = parse_program(parse_src.as_ref()).expect("program should parse");
        let module_cache = crate::module_cache::ModuleCache::new();
        let mut compiler = shape_vm::BytecodeCompiler::new();

        let diagnostics = crate::analysis::validate_imports_and_register_items(
            &program,
            source,
            &file_path,
            &module_cache,
            None,
            &mut compiler,
        );

        assert!(
            diagnostics
                .iter()
                .any(|diag| diag.message.contains("Cannot resolve module 'missingmod'")),
            "expected unknown namespace module diagnostic, got {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_is_position_in_frontmatter() {
        let source = r#"---
name = "script"
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
let x = 1
"#;

        assert!(is_position_in_frontmatter(
            source,
            Position {
                line: 1,
                character: 0
            }
        ));
        assert!(is_position_in_frontmatter(
            source,
            Position {
                line: 3,
                character: 2
            }
        ));
        assert!(!is_position_in_frontmatter(
            source,
            Position {
                line: 6,
                character: 0
            }
        ));
    }

    #[test]
    fn test_is_position_in_frontmatter_ignores_shebang_line() {
        let source = r#"#!/usr/bin/env shape
---
name = "script"
---
print("hello")
"#;

        assert!(!is_position_in_frontmatter(
            source,
            Position {
                line: 0,
                character: 5
            }
        ));
        assert!(is_position_in_frontmatter(
            source,
            Position {
                line: 2,
                character: 1
            }
        ));
        assert!(!is_position_in_frontmatter(
            source,
            Position {
                line: 4,
                character: 0
            }
        ));
    }

    #[test]
    fn test_configured_extensions_from_lsp_value_parses_top_level_array() {
        let value = serde_json::json!({
            "alwaysLoadExtensions": [
                "./extensions/libshape_ext_python.so",
                {
                    "name": "duckdb",
                    "path": "/tmp/libshape_ext_duckdb.so",
                    "config": { "mode": "readonly" }
                }
            ]
        });
        let workspace_root = std::path::Path::new("/workspace");

        let specs = collect_configured_extensions_from_options(Some(&value), Some(workspace_root));
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].path,
            std::path::PathBuf::from("/workspace").join("./extensions/libshape_ext_python.so")
        );
        assert_eq!(specs[0].config, serde_json::json!({}));
        assert_eq!(specs[1].name, "duckdb");
        assert_eq!(
            specs[1].path,
            std::path::PathBuf::from("/tmp/libshape_ext_duckdb.so")
        );
        assert_eq!(specs[1].config, serde_json::json!({ "mode": "readonly" }));
    }

    #[test]
    fn test_configured_extensions_from_lsp_value_parses_nested_shape_key_and_dedupes() {
        let value = serde_json::json!({
            "shape": {
                "always_load_extensions": [
                    "/tmp/libshape_ext_python.so",
                    "/tmp/libshape_ext_python.so"
                ]
            }
        });

        let specs = dedup_extension_specs(collect_configured_extensions_from_options(
            Some(&value),
            None,
        ));
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0].path,
            std::path::PathBuf::from("/tmp/libshape_ext_python.so")
        );
        assert_eq!(specs[0].name, "libshape_ext_python");
    }
}
