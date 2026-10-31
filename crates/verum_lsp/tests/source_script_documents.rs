//! Script documents retain compiler parsing rules and source coordinates.
use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};
use verum_ast::FileId;
use verum_lsp::{ParsedDocument, document::DocumentState};

#[test]
fn source_headers_select_script_mode_without_rewriting_spans() {
    for prefix in ["", "\u{feff}"] {
        let source = format!("{prefix}#!/usr/bin/env verum\nlet value: Int = 42;\nprint(value);\n");
        let doc = DocumentState::new(source.clone(), 1, FileId::new(7));
        assert!(doc.module.as_ref().is_some_and(|module| module.is_script()));
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.text, source);
        let cached = ParsedDocument::new(source, 1, FileId::new(7));
        assert!(cached.ast.as_ref().is_some_and(|module| module.is_script()));
        assert!(cached.parse_errors.is_empty());
    }
}

#[test]
fn script_edits_replace_the_whole_entry_and_header_removal_restores_module_rules() {
    let source = format!(
        "#!/usr/bin/env verum\nlet value = 1;\n{}print(value);\n",
        "// padding\n".repeat(300)
    );
    let mut doc = ParsedDocument::new(source, 1, FileId::new(9));
    doc.apply_incremental_changes(
        &[TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(1, 12), Position::new(1, 13))),
            range_length: Some(1),
            text: "2".into(),
        }],
        2,
    )
    .unwrap();
    assert!(doc.source.contains("let value = 2;"));
    assert!(doc.ast.as_ref().is_some_and(|module| module.is_script()));
    assert!(doc.parse_errors.is_empty());
    doc.apply_incremental_changes(
        &[TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 0), Position::new(1, 0))),
            range_length: None,
            text: "".into(),
        }],
        3,
    )
    .unwrap();
    assert!(
        doc.ast.is_none(),
        "bare expressions cannot become module items"
    );
    assert!(
        !doc.parse_errors.is_empty(),
        "refusal must retain diagnostics"
    );
}

#[test]
fn malformed_scripts_are_refused_by_both_document_stores() {
    let source = "#!/usr/bin/env verum\nlet = ;\n";
    let doc = DocumentState::new(source.into(), 1, FileId::new(1));
    assert!(doc.module.is_none());
    assert!(!doc.diagnostics.is_empty());
    let cached = ParsedDocument::new(source.into(), 1, FileId::new(1));
    assert!(cached.ast.is_none());
    assert!(!cached.parse_errors.is_empty());
}
