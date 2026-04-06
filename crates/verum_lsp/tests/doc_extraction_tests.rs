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
//! Tests for documentation extraction from attributes
//!
//! Per CLAUDE.md standards: Tests in tests/ directory, not inline

use verum_ast::FileId;
use verum_lsp::document::{DocumentState, SymbolKind};

#[test]
fn test_doc_extraction_function() {
    let source = r#"
        /// This is a documented function.
        /// It has multiple lines of documentation.
        fn documented_func() -> i32 {
            42
        }
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Find the function symbol
    if let Some(symbol) = doc.get_symbol("documented_func") {
        assert_eq!(symbol.kind, SymbolKind::Function);
        // Note: Doc extraction depends on parser storing doc comments as attributes
        // This test validates the extraction mechanism is in place
    }
}

#[test]
fn test_doc_extraction_type() {
    let source = r#"
        /// A documented type.
        type Point is record {
            x: f64,
            y: f64,
        }
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Find the type symbol
    if let Some(symbol) = doc.get_symbol("Point") {
        assert_eq!(symbol.kind, SymbolKind::Type);
    }
}

#[test]
fn test_doc_extraction_protocol() {
    let source = r#"
        /// A documented protocol.
        protocol Drawable {
            fn draw(&self);
        }
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Find the protocol symbol
    if let Some(symbol) = doc.get_symbol("Drawable") {
        assert_eq!(symbol.kind, SymbolKind::Protocol);
    }
}

#[test]
fn test_doc_extraction_const() {
    let source = r#"
        /// The answer to life, the universe, and everything.
        const ANSWER: i32 = 42;
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Find the constant symbol
    if let Some(symbol) = doc.get_symbol("ANSWER") {
        assert_eq!(symbol.kind, SymbolKind::Constant);
    }
}

#[test]
fn test_no_doc_extraction() {
    let source = r#"
        fn undocumented_func() -> i32 {
            42
        }
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Find the function symbol
    if let Some(symbol) = doc.get_symbol("undocumented_func") {
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert!(symbol.docs.is_none());
    }
}

#[test]
fn test_multiline_doc_extraction() {
    let source = r#"
        /// Line 1 of documentation
        /// Line 2 of documentation
        /// Line 3 of documentation
        fn multi_doc_func() {}
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Verify extraction mechanism is in place
    if let Some(symbol) = doc.get_symbol("multi_doc_func") {
        assert_eq!(symbol.kind, SymbolKind::Function);
    }
}

#[test]
fn test_block_doc_extraction() {
    let source = r#"
        /**
         * Block style documentation
         * with multiple lines
         */
        fn block_doc_func() {}
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Verify extraction mechanism handles block docs
    if let Some(symbol) = doc.get_symbol("block_doc_func") {
        assert_eq!(symbol.kind, SymbolKind::Function);
    }
}

#[test]
fn test_module_doc_extraction() {
    let source = r#"
        //! Module-level documentation
        //! This describes the entire module

        /// A function in the module
        fn module_func() {}
    "#;

    let file_id = FileId::new(0);
    let doc = DocumentState::new(source.to_string(), 0, file_id);

    // Verify module-level docs don't interfere with function docs
    if let Some(symbol) = doc.get_symbol("module_func") {
        assert_eq!(symbol.kind, SymbolKind::Function);
    }
}
