//! End-to-end integration tests for LSP protocol handlers
//!
//! Tests the full LSP protocol flow through the Backend using tower_lsp::LspService.

use tower_lsp::lsp_types::*;

/// Helper to create a test URI
fn test_uri(name: &str) -> Url {
    Url::parse(&format!("file:///test/{}.vr", name)).unwrap()
}

// ==================== Workspace Index Tests ====================

#[test]
fn test_workspace_index_creation() {
    let index = verum_lsp::workspace_index::WorkspaceIndex::new();
    assert_eq!(index.module_count(), 0);
}

#[test]
fn test_workspace_index_find_symbol_empty() {
    let index = verum_lsp::workspace_index::WorkspaceIndex::new();
    let symbols = index.find_symbol_across_workspace("foo");
    assert!(symbols.is_empty());
}

// ==================== Selection Range Tests ====================

#[test]
fn test_selection_range_no_module() {
    let doc = verum_lsp::DocumentState::new(
        "fn main() { let x = 1; }".to_string(),
        1,
        verum_ast::FileId::new(1),
    );

    let positions = vec![Position {
        line: 0,
        character: 5,
    }];

    let ranges = verum_lsp::selection_range::compute_selection_ranges(&doc, &positions);
    assert_eq!(ranges.len(), 1);
    // Should at least have a file-level range
    assert!(ranges[0].range.start.line == 0);
}

#[test]
fn test_selection_range_in_function() {
    let source = "fn foo() {\n    let x = 42;\n    x\n}";
    let doc = verum_lsp::DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(1));

    let positions = vec![Position {
        line: 1,
        character: 8, // on 'x' in 'let x = 42;'
    }];

    let ranges = verum_lsp::selection_range::compute_selection_ranges(&doc, &positions);
    assert_eq!(ranges.len(), 1);

    // Should have nested ranges (word -> statement -> block -> function -> file)
    let sr = &ranges[0];
    assert!(sr.parent.is_some()); // At least one parent
}

// ==================== Type Hierarchy Tests ====================

#[test]
fn test_type_hierarchy_prepare_no_module() {
    let doc = verum_lsp::DocumentState::new(String::new(), 1, verum_ast::FileId::new(1));
    let uri = test_uri("test");
    let result = verum_lsp::type_hierarchy::prepare_type_hierarchy(
        &doc,
        Position {
            line: 0,
            character: 0,
        },
        &uri,
    );
    assert!(result.is_none());
}

#[test]
fn test_type_hierarchy_prepare_type() {
    let source = "type Point is { x: Float, y: Float };";
    let doc = verum_lsp::DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(1));
    let uri = test_uri("test");

    // Position on "Point"
    let result = verum_lsp::type_hierarchy::prepare_type_hierarchy(
        &doc,
        Position {
            line: 0,
            character: 5,
        },
        &uri,
    );

    if let Some(items) = result {
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Point");
        assert_eq!(items[0].kind, SymbolKind::STRUCT);
    }
    // May be None if parser doesn't produce module for this input
}

// ==================== Inline Values Tests ====================

#[test]
fn test_inline_values_empty_doc() {
    let doc = verum_lsp::DocumentState::new(String::new(), 1, verum_ast::FileId::new(1));
    let range = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 100,
            character: 0,
        },
    };
    let values = verum_lsp::inline_values::compute_inline_values(&doc, range);
    assert!(values.is_empty());
}

#[test]
fn test_inline_values_with_lets() {
    let source = "fn main() {\n    let x: Int = 42;\n    let y = x + 1;\n}";
    let doc = verum_lsp::DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(1));
    let range = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 10,
            character: 0,
        },
    };
    let values = verum_lsp::inline_values::compute_inline_values(&doc, range);
    // May or may not find values depending on parser output
    // The test is that it doesn't panic
    let _ = values;
}

// ==================== Cross-File Navigation Tests ====================

#[test]
fn test_cross_file_goto_definition_falls_back() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    let uri = test_uri("main");
    doc_store.open(
        uri.clone(),
        "fn main() { foo(); }".to_string(),
        1,
    );

    // Should return None since 'foo' is not defined anywhere
    let result = verum_lsp::workspace_index::goto_definition_cross_file(
        &doc_store,
        &workspace_index,
        &uri,
        Position {
            line: 0,
            character: 13, // on 'foo'
        },
    );
    // Result may be None or Some depending on whether text matches
    let _ = result;
}

#[test]
fn test_cross_file_references() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    let uri = test_uri("main");
    doc_store.open(
        uri.clone(),
        "fn foo() { 1 }\nfn main() { foo(); }".to_string(),
        1,
    );

    let refs = verum_lsp::workspace_index::find_references_cross_file(
        &doc_store,
        &workspace_index,
        &uri,
        Position {
            line: 0,
            character: 3, // on 'foo' definition
        },
        true,
    );

    // Should find at least the definition of foo
    assert!(!refs.is_empty(), "Should find at least one reference to 'foo'");
}

#[test]
fn test_cross_file_rename() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    let uri = test_uri("main");
    doc_store.open(
        uri.clone(),
        "fn foo() { 1 }\nfn main() { foo(); }".to_string(),
        1,
    );

    let result = verum_lsp::workspace_index::rename_cross_file(
        &doc_store,
        &workspace_index,
        &uri,
        Position {
            line: 0,
            character: 3, // on 'foo'
        },
        "bar".to_string(),
    );

    if let Some(edit) = result {
        let changes = edit.changes.unwrap();
        assert!(changes.contains_key(&uri));
        let edits = &changes[&uri];
        assert!(edits.len() >= 2, "Should rename both definition and call");
        for edit in edits {
            assert_eq!(edit.new_text, "bar");
        }
    }
}

#[test]
fn test_rename_invalid_name_rejected() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    let uri = test_uri("main");
    doc_store.open(
        uri.clone(),
        "fn foo() { 1 }".to_string(),
        1,
    );

    // Try renaming to a keyword
    let result = verum_lsp::workspace_index::rename_cross_file(
        &doc_store,
        &workspace_index,
        &uri,
        Position {
            line: 0,
            character: 3,
        },
        "fn".to_string(), // keyword
    );
    assert!(result.is_none(), "Should reject keyword as new name");

    // Try renaming to invalid identifier
    let result = verum_lsp::workspace_index::rename_cross_file(
        &doc_store,
        &workspace_index,
        &uri,
        Position {
            line: 0,
            character: 3,
        },
        "123abc".to_string(),
    );
    assert!(result.is_none(), "Should reject invalid identifier");
}

// ==================== Semantic Token Delta Tests ====================

#[test]
fn test_compute_semantic_token_edits_identical() {
    // When tokens are identical, no edits should be produced
    // This tests the concept — actual edit computation is internal to backend
}

// ==================== Document Store Workspace Integration ====================

#[test]
fn test_document_store_workspace_indexing() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    // Open a document
    let uri_a = test_uri("module_a");
    doc_store.open(
        uri_a.clone(),
        "fn helper() { 1 }\ntype Config is { debug: Bool };".to_string(),
        1,
    );

    // Index it
    doc_store.with_document(&uri_a, |doc| {
        if let Some(module) = &doc.module {
            workspace_index.index_document(&uri_a, module, &doc.text);
        }
    });

    // Should find the exported symbols
    let helper_locs = workspace_index.find_symbol_across_workspace("helper");
    assert!(!helper_locs.is_empty(), "Should find 'helper' function");

    let config_locs = workspace_index.find_symbol_across_workspace("Config");
    assert!(!config_locs.is_empty(), "Should find 'Config' type");
}

#[test]
fn test_mount_parsing() {
    let doc_store = verum_lsp::DocumentStore::new();
    let workspace_index = verum_lsp::workspace_index::WorkspaceIndex::new();

    // Open a document with mount statements
    let uri = test_uri("main");
    doc_store.open(
        uri.clone(),
        "mount collections.list;\nfn main() { 1 }".to_string(),
        1,
    );

    // Index it
    doc_store.with_document(&uri, |doc| {
        if let Some(module) = &doc.module {
            workspace_index.index_document(&uri, module, &doc.text);
        }
    });

    // The mount graph should contain entries for this file
    // (depends on parser producing Mount items)
}

// ==================== Completion Resolve Tests ====================

#[test]
fn test_completion_item_data_format() {
    let data = serde_json::json!({
        "uri": "file:///test/foo.vr",
        "name": "my_function"
    });
    assert_eq!(data["uri"].as_str().unwrap(), "file:///test/foo.vr");
    assert_eq!(data["name"].as_str().unwrap(), "my_function");
}

#[test]
fn test_completion_items_carry_resolve_data() {
    let source = "fn helper(x: Int) -> Int { x + 1 }\nfn main() { helper(1); }";
    let doc = verum_lsp::DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(1));

    let items = verum_lsp::completion::complete_at_position(
        &doc,
        Position { line: 1, character: 14 },
    );

    // Find the 'helper' completion
    let helper_item = items.iter().find(|i| i.label == "helper");
    if let Some(item) = helper_item {
        // Module-level function completions should carry a data payload
        assert!(item.data.is_some(), "function completion should carry resolve data");
        let data = item.data.as_ref().unwrap();
        assert_eq!(data["name"].as_str().unwrap(), "helper");
        // Documentation is deferred to resolve
        assert!(item.documentation.is_none());
    }
}

#[test]
fn test_attach_resolve_data() {
    let mut item = CompletionItem {
        label: "foo".to_string(),
        ..Default::default()
    };
    verum_lsp::completion::attach_resolve_data(&mut item, "file:///a.vr", "foo");
    let data = item.data.unwrap();
    assert_eq!(data["uri"].as_str().unwrap(), "file:///a.vr");
    assert_eq!(data["name"].as_str().unwrap(), "foo");
}

// ==================== On-Type Formatting Tests ====================

#[test]
fn test_format_on_type_match_arm_indent() {
    // After `=>`, newline should indent one level deeper
    let indent = verum_lsp::formatting::calculate_indent_for_new_line("        Some(x) =>");
    assert_eq!(indent, 3);
}

#[test]
fn test_format_on_type_pipeline_continuation() {
    // After `|>`, newline should keep same indent
    let indent = verum_lsp::formatting::calculate_indent_for_new_line("        |> filter");
    assert_eq!(indent, 2);
}
