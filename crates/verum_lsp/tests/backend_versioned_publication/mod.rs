use super::*;

#[tokio::test]
async fn old_revision_and_reopened_document_cannot_publish() {
    let (service, _socket) = tower_lsp::LspService::new(Backend::new);
    let backend = service.inner();
    let uri = Url::parse("file:///late-diagnostics.vr").unwrap();
    backend.documents.open(uri.clone(), String::new(), 1);
    let old = backend.documents.diagnostic_snapshot(&uri).unwrap();
    backend
        .documents
        .try_update(&uri, "// changed".into(), 2)
        .unwrap();
    assert!(!backend.publish_snapshot(&uri, &old, vec![]).await);
    backend.documents.close(&uri);
    backend.documents.open(uri.clone(), String::new(), 1);
    assert!(!backend.publish_snapshot(&uri, &old, vec![]).await);
}

#[tokio::test]
async fn initialized_keeps_the_clients_workspace_root() {
    let root = tempfile::tempdir().unwrap();
    let (service, _socket) = tower_lsp::LspService::new(Backend::new);
    let backend = service.inner();
    #[allow(deprecated)]
    let params = InitializeParams {
        root_uri: Some(Url::from_directory_path(root.path()).unwrap()),
        ..Default::default()
    };
    backend.initialize(params).await.unwrap();
    // Created after initialize: resolution must use the selected root,
    // not an old entry retained by the initial directory scan.
    let path = root.path().join("late.vr");
    std::fs::write(&path, "fn answer() -> Int { 42 }").unwrap();
    backend.initialized(InitializedParams {}).await;
    let uri = Url::from_file_path(&path).unwrap();
    assert_eq!(
        backend.workspace_index.resolve_mount(&uri, "late"),
        Some(uri)
    );
}

#[tokio::test]
async fn invalid_and_closed_documents_remove_stale_exports() {
    let (service, _socket) = tower_lsp::LspService::new(Backend::new);
    let backend = service.inner();
    backend
        .config
        .apply_json(&serde_json::json!({"lintEnabled": false}));
    let uri = Url::parse("file:///stale-index.vr").unwrap();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "verum".into(),
                version: 1,
                text: "fn answer() -> Int { 42 }".into(),
            },
        })
        .await;
    assert_eq!(
        backend
            .workspace_index
            .find_symbol_across_workspace("answer")
            .len(),
        1
    );
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn answer() -> Int {".into(),
            }],
        })
        .await;
    assert!(
        backend
            .workspace_index
            .find_symbol_across_workspace("answer")
            .is_empty()
    );
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 3,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn answer() -> Int { 42 }".into(),
            }],
        })
        .await;
    assert_eq!(
        backend
            .workspace_index
            .find_symbol_across_workspace("answer")
            .len(),
        1
    );
    backend
        .did_close(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        })
        .await;
    assert!(
        backend
            .workspace_index
            .find_symbol_across_workspace("answer")
            .is_empty()
    );
}
