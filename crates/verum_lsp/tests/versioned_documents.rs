//! Document changes and diagnostic publication share one source revision.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::document::{DocumentState, DocumentStore, DocumentUpdateError};

fn change(start: (u32, u32), end: (u32, u32), text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range::new(
            Position::new(start.0, start.1),
            Position::new(end.0, end.1),
        )),
        range_length: None,
        text: text.into(),
    }
}

#[test]
fn utf16_edits_keep_non_bmp_neighbors_and_apply_in_order() {
    let mut doc = DocumentState::new(
        "// a😀я\nfn answer() -> Int { 42 }".into(),
        1,
        FileId::new(1),
    );
    doc.try_apply_changes(
        vec![change((0, 4), (0, 6), "𐐀!"), change((0, 7), (0, 8), "z")],
        2,
    )
    .unwrap();
    assert_eq!(doc.text, "// a𐐀!z\nfn answer() -> Int { 42 }");
    assert_eq!(doc.version, 2);
    assert!(doc.module.is_some());
    assert!(doc.diagnostics.is_empty());
}

#[test]
fn stale_and_duplicate_versions_preserve_the_analysis() {
    let mut doc = DocumentState::new("fn answer() -> Int { 42 }".into(), 3, FileId::new(1));
    for version in [2, 3] {
        assert_eq!(
            doc.try_update("broken".into(), version),
            Err(DocumentUpdateError::StaleVersion {
                current: 3,
                incoming: version
            })
        );
    }
    assert_eq!(doc.text, "fn answer() -> Int { 42 }");
    assert!(doc.module.is_some());
    assert!(doc.diagnostics.is_empty());
}

#[test]
fn failed_second_change_rolls_back_the_entire_batch() {
    let source = "// 😀\nfn answer() -> Int { 42 }";
    let mut doc = DocumentState::new(source.into(), 1, FileId::new(1));
    let error = doc.try_apply_changes(
        vec![change((0, 3), (0, 5), "😀!"), change((0, 4), (0, 5), "x")],
        2,
    );
    assert_eq!(error, Err(DocumentUpdateError::InvalidRange { index: 1 }));
    assert_eq!(doc.text, source);
    assert_eq!(doc.version, 1);
    assert!(doc.module.is_some());
}

#[test]
fn invalid_lines_reversed_ranges_and_wrong_lengths_are_refused() {
    let source = "// 😀";
    for invalid in [
        change((0, 5), (0, 3), ""),
        change((2, 0), (2, 1), ""),
        TextDocumentContentChangeEvent {
            range_length: Some(1),
            ..change((0, 3), (0, 5), "")
        },
    ] {
        let mut doc = DocumentState::new(source.into(), 1, FileId::new(1));
        assert!(doc.try_apply_changes(vec![invalid], 2).is_err());
        assert_eq!(doc.text, source);
        assert_eq!(doc.version, 1);
    }
}

#[test]
fn full_change_then_incremental_change_uses_intermediate_source() {
    let mut doc = DocumentState::new(String::new(), 1, FileId::new(1));
    doc.try_apply_changes(
        vec![
            TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "// 😀".into(),
            },
            change((0, 3), (0, 5), "ok"),
        ],
        2,
    )
    .unwrap();
    assert_eq!(doc.text, "// ok");
}

#[test]
fn snapshots_remain_immutable_and_reopen_has_new_identity() {
    let uri = Url::parse("file:///versioned.vr").unwrap();
    let store = DocumentStore::new();
    assert_eq!(
        store.try_update(&uri, String::new(), 1),
        Err(DocumentUpdateError::NotOpen)
    );
    store.open(uri.clone(), "fn answer() -> Int { 42 }".into(), 1);
    let original = store.diagnostic_snapshot(&uri).unwrap();
    assert!(store.is_current_snapshot(&uri, &original));
    store
        .try_update(&uri, "fn answer() -> Int {".into(), 2)
        .unwrap();
    let invalid = store.diagnostic_snapshot(&uri).unwrap();
    assert!(!invalid.diagnostics.is_empty());
    assert!(original.diagnostics.is_empty());
    assert!(!store.is_current_snapshot(&uri, &original));
    store.close(&uri);
    assert!(!store.is_current_snapshot(&uri, &invalid));
    store.open(uri.clone(), original.text.clone(), original.version);
    assert!(!store.is_current_snapshot(&uri, &original));
}

#[test]
fn parser_and_type_diagnostics_keep_source_locations() {
    for source in [
        "// 😀\nfn answer() -> Int {",
        "// 😀\nfn answer() -> Int { true }",
    ] {
        let doc = DocumentState::new(source.into(), 1, FileId::new(1));
        assert!(!doc.diagnostics.is_empty(), "fixture must fail: {source}");
        assert!(
            doc.diagnostics
                .iter()
                .any(|diagnostic| diagnostic.primary_span().is_some_and(|span| span.line == 2))
        );
    }
}

#[test]
fn recovered_statement_errors_and_direct_errors_are_all_reported_once() {
    let source = "fn broken() -> Int {\nlet first: Text = 42;\nlet second: Bool = 7;\n0\n}\nfn direct() -> Int { false }";
    let doc = DocumentState::new(source.into(), 1, FileId::new(1));
    assert!(doc.module.is_some(), "fixture must parse");
    assert_eq!(
        doc.diagnostics.len(),
        3,
        "both recovered errors and the direct return error must be retained once: {:?}",
        doc.diagnostics
    );
    let mut lines: Vec<_> = doc
        .diagnostics
        .iter()
        .map(|error| error.primary_span().expect("type error location").line)
        .collect();
    lines.sort_unstable();
    assert_eq!(lines, vec![2, 3, 6]);
}

async fn send(writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, message: Value) {
    let body = serde_json::to_vec(&message).unwrap();
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await
        .unwrap();
    writer.write_all(&body).await.unwrap();
    writer.flush().await.unwrap();
}

async fn receive(reader: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Value {
    let mut length = None;
    loop {
        let mut header = String::new();
        assert!(
            reader.read_line(&mut header).await.unwrap() > 0,
            "server closed before response"
        );
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length: ") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("content length")];
    reader.read_exact(&mut body).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn actual_lsp_notifications_have_exact_versions_and_ignore_stale_changes() {
    let (service, socket) = tower_lsp::LspService::new(verum_lsp::Backend::new);
    let (client, server) = tokio::io::duplex(65536);
    let (server_read, server_write) = tokio::io::split(server);
    let server = tower_lsp::Server::new(server_read, server_write, socket).serve(service);
    let (read, mut write) = tokio::io::split(client);
    let mut read = BufReader::new(read);
    let client = async {
        send(&mut write, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"initializationOptions":{"lintEnabled":false}}})).await;
        assert_eq!(receive(&mut read).await["id"], 1);
        let uri = "file:///versioned-notification.vr";
        send(&mut write, json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"verum","version":7,"text":"fn answer() -> Int { 42 }"}}})).await;
        let opened = receive(&mut read).await;
        assert_eq!(opened["method"], "textDocument/publishDiagnostics");
        assert_eq!(opened["params"]["version"], 7);
        assert_eq!(opened["params"]["diagnostics"], json!([]));
        send(&mut write, json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":8},"contentChanges":[{"text":"fn answer() -> Int {"}]}})).await;
        let changed = receive(&mut read).await;
        assert_eq!(changed["params"]["version"], 8);
        assert!(
            !changed["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        send(&mut write, json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":7},"contentChanges":[{"text":"fn stale() -> Int { 1 }"}]}})).await;
        send(&mut write, json!({"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":uri}}})).await;
        let barrier = receive(&mut read).await;
        assert_eq!(
            barrier["id"], 2,
            "a stale change must not publish diagnostics"
        );
        assert!(barrier["result"].is_null());
        send(&mut write, json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}})).await;
        let closed = receive(&mut read).await;
        assert_eq!(closed["params"]["version"], 8);
        assert_eq!(closed["params"]["diagnostics"], json!([]));
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! { _ = server => panic!("server exited early"), _ = client => {} }
    })
    .await
    .expect("LSP request loop completed");
}
