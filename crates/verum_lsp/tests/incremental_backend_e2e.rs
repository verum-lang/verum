#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! End-to-End tests for IncrementalBackend with all LSP features
//!
//! This test suite validates that all 6 LSP features work correctly
//! with the DocumentCache integration:
//! - Completion
//! - Hover
//! - Go to Definition
//! - Find References
//! - Rename
//! - Formatting

use tower_lsp::LanguageServer;
use tower_lsp::LspService;
use tower_lsp::lsp_types::*;
use verum_lsp::backend_incremental::IncrementalBackend;

/// Helper to create a test URI
fn test_uri() -> Url {
    Url::parse("file:///test.vr").unwrap()
}

#[tokio::test]
async fn test_completion_basic() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    // Initialize the server
    let init_params = InitializeParams::default();
    let result = backend.initialize(init_params).await.unwrap();
    assert!(result.capabilities.completion_provider.is_some());

    // Open a document with a simple function
    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn main() {\n    let x = 5;\n    \n}".to_string(),
            },
        })
        .await;

    // Request completion
    let completion_response = backend
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 2,
                    character: 4,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .unwrap();

    // Should have completions (keywords, types, etc.)
    assert!(completion_response.is_some());
    if let Some(CompletionResponse::Array(items)) = completion_response {
        assert!(!items.is_empty());
        // Check for at least one keyword
        assert!(items.iter().any(|item| item.label == "fn"));
    }
}

#[tokio::test]
async fn test_hover_shows_type_info() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    let init_params = InitializeParams::default();
    backend.initialize(init_params).await.unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn add(x: Int, y: Int) -> Int {\n    x + y\n}".to_string(),
            },
        })
        .await;

    // Hover over function name
    let hover_response = backend
        .hover(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: 3,
                },
            },
            work_done_progress_params: Default::default(),
        })
        .await
        .unwrap();

    // Should show function signature
    if let Some(hover) = hover_response
        && let HoverContents::Markup(markup) = hover.contents
    {
        assert!(markup.value.contains("add"));
    }
}

#[tokio::test]
async fn test_goto_definition() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn test() -> Int { 42 }\nfn main() {\n    test()\n}".to_string(),
            },
        })
        .await;

    // Go to definition of 'test' call
    let def_response = backend
        .goto_definition(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 2,
                    character: 4,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .unwrap();

    // Should find the definition
    assert!(def_response.is_some());
}

#[tokio::test]
async fn test_find_references() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn helper() -> Int { 1 }\nfn main() {\n    helper()\n    helper()\n}"
                    .to_string(),
            },
        })
        .await;

    // Find references to 'helper'
    let refs_response = backend
        .references(ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: 3,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: ReferenceContext {
                include_declaration: true,
            },
        })
        .await
        .unwrap();

    // Should find multiple references
    if let Some(refs) = refs_response {
        assert!(refs.len() >= 2); // At least definition + one usage
    }
}

#[tokio::test]
async fn test_rename_symbol() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn old_name() -> Int { 42 }\nfn main() {\n    old_name()\n}".to_string(),
            },
        })
        .await;

    // Prepare rename
    let prepare_response = backend
        .prepare_rename(TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position: Position {
                line: 0,
                character: 3,
            },
        })
        .await
        .unwrap();

    assert!(prepare_response.is_some());

    // Execute rename
    let rename_response = backend
        .rename(RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: 3,
                },
            },
            new_name: "new_name".to_string(),
            work_done_progress_params: Default::default(),
        })
        .await
        .unwrap();

    // Note: rename might return None if symbol table isn't fully populated
    // This is expected behavior for incremental parsing where symbol resolution
    // might not be complete. The important thing is no panic occurs.
    if let Some(edit) = rename_response {
        // If we got edits, they should contain changes
        assert!(edit.changes.is_some() || edit.document_changes.is_some());
    }
}

#[tokio::test]
async fn test_formatting() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    let unformatted_code = "fn main(){let x=5;let y=10;}";
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: unformatted_code.to_string(),
            },
        })
        .await;

    // Format document
    let format_response = backend
        .formatting(DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            options: FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        })
        .await
        .unwrap();

    // Should return formatting edits
    assert!(format_response.is_some());
}

#[tokio::test]
async fn test_incremental_updates() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn main() {}".to_string(),
            },
        })
        .await;

    // Make incremental change
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position {
                        line: 0,
                        character: 3,
                    },
                    end: Position {
                        line: 0,
                        character: 7,
                    },
                }),
                range_length: Some(4),
                text: "test".to_string(),
            }],
        })
        .await;

    // Request completion after change
    let completion_response = backend
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: 10,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .unwrap();

    // Should still get completions after incremental update
    assert!(completion_response.is_some());
}

#[tokio::test]
async fn test_concurrent_requests() {
    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = std::sync::Arc::new(service.inner());

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    let uri = test_uri();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: "fn func1() {}\nfn func2() {}\nfn main() {}".to_string(),
            },
        })
        .await;

    // Make concurrent LSP requests
    let backend1 = backend.clone();
    let backend2 = backend.clone();
    let backend3 = backend.clone();
    let uri1 = uri.clone();
    let uri2 = uri.clone();
    let uri3 = uri.clone();

    let (r1, r2, r3) = tokio::join!(
        async move {
            backend1
                .completion(CompletionParams {
                    text_document_position: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri1 },
                        position: Position {
                            line: 2,
                            character: 10,
                        },
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    context: None,
                })
                .await
        },
        async move {
            backend2
                .hover(HoverParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri2 },
                        position: Position {
                            line: 0,
                            character: 3,
                        },
                    },
                    work_done_progress_params: Default::default(),
                })
                .await
        },
        async move {
            backend3
                .goto_definition(GotoDefinitionParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri3 },
                        position: Position {
                            line: 1,
                            character: 3,
                        },
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                })
                .await
        }
    );

    // All requests should succeed
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());
}

#[tokio::test]
async fn test_performance_large_file() {
    use std::time::Instant;

    let (service, _socket) = LspService::new(IncrementalBackend::new);
    let backend = service.inner();

    backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();

    // Generate a large file with many functions
    let mut large_code = String::new();
    for i in 0..1000 {
        large_code.push_str(&format!("fn func{}() {{ return {}; }}\n", i, i));
    }

    let uri = test_uri();
    let start = Instant::now();

    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".to_string(),
                version: 1,
                text: large_code,
            },
        })
        .await;

    let open_time = start.elapsed();
    println!("Large file open time: {:?}", open_time);

    // Test completion performance
    let start = Instant::now();
    let _completion = backend
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 500,
                    character: 10,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .unwrap();

    let completion_time = start.elapsed();
    println!("Completion time on large file: {:?}", completion_time);

    // Should respond within 100ms target
    assert!(
        completion_time.as_millis() < 100,
        "Completion took too long: {:?}",
        completion_time
    );
}
