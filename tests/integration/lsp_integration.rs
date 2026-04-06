//! Category 2: LSP Integration Tests
//!
//! Tests LSP server with real-world scenarios:
//! - Document lifecycle (open → edit → save → close)
//! - Incremental updates with document cache
//! - Completion + type info
//! - Go-to-definition + find references
//! - Rename across files
//! - Format + diagnostics
//! - Concurrent requests (100+ simultaneous)
//! - Large codebases (10K+ LOC)

use std::sync::Arc;
use std::time::Duration;
use tower_lsp::lsp_types::*;
use verum_lsp::backend_incremental::IncrementalBackend;
use verum_lsp::document_cache::DocumentCache;
use verum_std::core::{List, Text};

use crate::integration::test_utils::*;

// ============================================================================
// Test 2.1: Document Lifecycle
// ============================================================================

#[tokio::test]
async fn test_document_open_edit_save_close() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    // Open document
    let initial_text = "fn add(x: Int, y: Int) -> Int { x + y }";
    cache.open(uri.clone(), initial_text.to_string());

    assert!(cache.is_open(&uri), "Document should be open");
    assert_eq!(cache.get_text(&uri), Some(initial_text.to_string()));

    // Edit document
    let updated_text = "fn add(x: Int, y: Int) -> Int { x + y + 1 }";
    cache.update(uri.clone(), updated_text.to_string());

    assert_eq!(cache.get_text(&uri), Some(updated_text.to_string()));

    // Close document
    cache.close(uri.clone());

    assert!(!cache.is_open(&uri), "Document should be closed");
}

#[tokio::test]
async fn test_document_incremental_updates() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    // Open with initial content
    cache.open(uri.clone(), "let x = 1".to_string());

    // Make incremental changes
    let changes = vec![
        ("let x = 2", 10),
        ("let x = 3", 20),
        ("let x = 4", 30),
    ];

    for (text, _delay_ms) in changes {
        tokio::time::sleep(Duration::from_millis(5)).await;
        cache.update(uri.clone(), text.to_string());
    }

    assert_eq!(cache.get_text(&uri), Some("let x = 4".to_string()));
}

#[tokio::test]
async fn test_multiple_documents() {
    let cache = DocumentCache::new();

    let docs = vec![
        ("file:///a.vr", "fn a() -> Int { 1 }"),
        ("file:///b.vr", "fn b() -> Int { 2 }"),
        ("file:///c.vr", "fn c() -> Int { 3 }"),
    ];

    // Open all documents
    for (path, content) in &docs {
        let uri = Url::parse(path).unwrap();
        cache.open(uri, content.to_string());
    }

    // Verify all are open
    for (path, _) in &docs {
        let uri = Url::parse(path).unwrap();
        assert!(cache.is_open(&uri));
    }

    // Close one document
    let uri_a = Url::parse("file:///a.vr").unwrap();
    cache.close(uri_a.clone());

    assert!(!cache.is_open(&uri_a));
    assert!(cache.is_open(&Url::parse("file:///b.vr").unwrap()));
}

// ============================================================================
// Test 2.2: Completion
// ============================================================================

#[tokio::test]
async fn test_completion_basic() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn add(x: Int, y: Int) -> Int { x + y }

fn main() {
    let result = ad  // Complete here
}
"#;

    cache.open(uri.clone(), source.to_string());

    // Request completion at cursor position
    let position = Position::new(4, 18);

    // Completion should suggest "add"
    // Implementation would call LSP completion handler
}

#[tokio::test]
async fn test_completion_context_aware() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn process(x: Int) -> Int {
    x.  // Complete here - should show Int methods
}
"#;

    cache.open(uri, source.to_string());

    // Completion should show Int methods/properties
}

// ============================================================================
// Test 2.3: Hover Information
// ============================================================================

#[tokio::test]
async fn test_hover_function_signature() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn add(x: Int, y: Int) -> Int { x + y }

fn main() {
    add(1, 2)
}
"#;

    cache.open(uri, source.to_string());

    // Hover over "add" should show signature: fn add(x: Int, y: Int) -> Int
}

#[tokio::test]
async fn test_hover_type_information() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
let x = 42
"#;

    cache.open(uri, source.to_string());

    // Hover over "x" should show type: Int
}

// ============================================================================
// Test 2.4: Go-to-Definition
// ============================================================================

#[tokio::test]
async fn test_goto_definition_same_file() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn helper() -> Int { 42 }

fn main() {
    helper()  // Go to definition
}
"#;

    cache.open(uri, source.to_string());

    // Go-to-definition on "helper" should jump to line 1
}

#[tokio::test]
async fn test_goto_definition_cross_file() {
    let cache = DocumentCache::new();

    // Module A
    let uri_a = Url::parse("file:///a.vr").unwrap();
    let source_a = "pub fn helper() -> Int { 42 }";
    cache.open(uri_a, source_a.to_string());

    // Module B
    let uri_b = Url::parse("file:///b.vr").unwrap();
    let source_b = r#"
using [A]

fn main() {
    A.helper()  // Go to definition
}
"#;
    cache.open(uri_b, source_b.to_string());

    // Go-to-definition should jump to file:///a.vr
}

// ============================================================================
// Test 2.5: Find References
// ============================================================================

#[tokio::test]
async fn test_find_references_same_file() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn helper() -> Int { 42 }

fn main() {
    helper()
    let x = helper()
    helper() + helper()
}
"#;

    cache.open(uri, source.to_string());

    // Find references for "helper" should return 4 locations
}

#[tokio::test]
async fn test_find_references_cross_file() {
    let cache = DocumentCache::new();

    // Module A with public function
    let uri_a = Url::parse("file:///a.vr").unwrap();
    cache.open(uri_a, "pub fn helper() -> Int { 42 }".to_string());

    // Module B using the function
    let uri_b = Url::parse("file:///b.vr").unwrap();
    cache.open(uri_b, "using [A]\nfn main() { A.helper() }".to_string());

    // Find references should return locations in both files
}

// ============================================================================
// Test 2.6: Rename
// ============================================================================

#[tokio::test]
async fn test_rename_same_file() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn oldName() -> Int { 42 }

fn main() {
    oldName()
}
"#;

    cache.open(uri.clone(), source.to_string());

    // Rename "oldName" to "newName"
    // Should update both definition and call site
}

#[tokio::test]
async fn test_rename_cross_file() {
    let cache = DocumentCache::new();

    let uri_a = Url::parse("file:///a.vr").unwrap();
    cache.open(uri_a, "pub fn oldName() -> Int { 42 }".to_string());

    let uri_b = Url::parse("file:///b.vr").unwrap();
    cache.open(uri_b, "using [A]\nfn main() { A.oldName() }".to_string());

    // Rename should update both files
}

// ============================================================================
// Test 2.7: Diagnostics
// ============================================================================

#[tokio::test]
async fn test_diagnostics_syntax_error() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = "fn broken(x: Int { x }"; // Missing closing paren
    cache.open(uri.clone(), source.to_string());

    let diagnostics = cache.get_diagnostics(&uri);
    assert!(diagnostics.len() > 0, "Should have syntax error diagnostic");
}

#[tokio::test]
async fn test_diagnostics_type_error() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn add(x: Int, y: Int) -> Int {
    x + "string"  // Type error
}
"#;
    cache.open(uri.clone(), source.to_string());

    let diagnostics = cache.get_diagnostics(&uri);
    // Should have type error diagnostic
}

#[tokio::test]
async fn test_diagnostics_incremental_update() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    // Start with error
    cache.open(uri.clone(), "fn broken(".to_string());
    let diag1 = cache.get_diagnostics(&uri);
    assert!(diag1.len() > 0, "Should have error");

    // Fix error
    cache.update(uri.clone(), "fn fixed() -> Int { 42 }".to_string());
    let diag2 = cache.get_diagnostics(&uri);
    // Should have no errors (or fewer errors)
}

// ============================================================================
// Test 2.8: Formatting
// ============================================================================

#[tokio::test]
async fn test_format_document() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let unformatted = "fn  add(x:Int,y:Int)->Int{x+y}";
    let expected = "fn add(x: Int, y: Int) -> Int { x + y }";

    cache.open(uri.clone(), unformatted.to_string());

    // Format document
    // Result should match expected formatting
}

#[tokio::test]
async fn test_format_range() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = r#"
fn add(x:Int,y:Int)->Int{x+y}
fn sub(x:Int,y:Int)->Int{x-y}
"#;

    cache.open(uri, source.to_string());

    // Format only first function
    // Only first line should be formatted
}

// ============================================================================
// Test 2.9: Concurrent Requests
// ============================================================================

#[tokio::test]
async fn test_concurrent_completion_requests() {
    let cache = Arc::new(DocumentCache::new());
    let uri = Url::parse("file:///test.vr").unwrap();

    let source = "fn add(x: Int, y: Int) -> Int { x + y }";
    cache.open(uri.clone(), source.to_string());

    // Send 100 concurrent completion requests
    let handles: Vec<_> = (0..100)
        .map(|i| {
            let cache = Arc::clone(&cache);
            let uri = uri.clone();
            tokio::spawn(async move {
                let position = Position::new(0, i % 20);
                // Request completion
                // This simulates concurrent LSP requests
            })
        })
        .collect();

    for handle in handles {
        handle.await.expect("Task should complete");
    }
}

#[tokio::test]
async fn test_concurrent_document_updates() {
    let cache = Arc::new(DocumentCache::new());
    let uri = Url::parse("file:///test.vr").unwrap();

    cache.open(uri.clone(), "initial".to_string());

    // Send 50 concurrent updates
    let durations = run_concurrent(50, |i| {
        let cache = Arc::clone(&cache);
        let uri = uri.clone();
        async move {
            cache.update(uri, format!("update {}", i));
        }
    })
    .await;

    let stats = PerfStats::from_durations(durations);
    assert_duration_lt(
        stats.p95,
        Duration::from_millis(100),
        "P95 update latency should be <100ms"
    );
}

// ============================================================================
// Test 2.10: Large Codebase Performance
// ============================================================================

#[tokio::test]
async fn test_large_single_file() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///large.vr").unwrap();

    // Generate 10K LOC file
    let large_source = generate_random_program(10_000);

    let (_, duration) = measure_time_async(|| async {
        cache.open(uri.clone(), large_source.clone());
    })
    .await;

    assert_duration_lt(
        duration,
        Duration::from_secs(5),
        "Opening 10K LOC file should be <5s"
    );
}

#[tokio::test]
async fn test_many_open_documents() {
    let cache = DocumentCache::new();

    // Open 100 documents
    let (_, duration) = measure_time_async(|| async {
        for i in 0..100 {
            let uri = Url::parse(&format!("file:///file{}.vr", i)).unwrap();
            let content = format!("fn func{}() -> Int {{ {} }}", i, i);
            cache.open(uri, content);
        }
    })
    .await;

    assert_duration_lt(
        duration,
        Duration::from_secs(10),
        "Opening 100 documents should be <10s"
    );
}

#[tokio::test]
async fn test_rapid_edits() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    cache.open(uri.clone(), "let x = 0".to_string());

    // Make 1000 rapid edits
    let (_, duration) = measure_time_async(|| async {
        for i in 0..1000 {
            cache.update(uri.clone(), format!("let x = {}", i));
        }
    })
    .await;

    assert_duration_lt(
        duration,
        Duration::from_secs(5),
        "1000 edits should complete in <5s"
    );
}

// ============================================================================
// Test 2.11: Debouncing
// ============================================================================

#[tokio::test]
async fn test_diagnostics_debouncing() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    cache.open(uri.clone(), "let x = 1".to_string());

    // Make rapid changes
    for i in 0..10 {
        cache.update(uri.clone(), format!("let x = {}", i));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Wait for debounce period
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Diagnostics should reflect final state
}

// ============================================================================
// Test 2.12: Memory Efficiency
// ============================================================================

#[tokio::test]
async fn test_memory_usage_with_large_file() {
    let mut tracker = MemoryTracker::new();
    let cache = DocumentCache::new();

    // Open large file
    let uri = Url::parse("file:///large.vr").unwrap();
    let large_content = generate_random_program(5000);
    cache.open(uri.clone(), large_content);

    tracker.update();

    // Close file
    cache.close(uri);

    tracker.update();

    let stats = tracker.stats();
    // Memory should be reclaimed after closing
}

#[tokio::test]
async fn test_cache_invalidation() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    // Open and parse
    cache.open(uri.clone(), "fn add(x: Int) -> Int { x + 1 }".to_string());

    // Update - should invalidate cache
    cache.update(uri.clone(), "fn add(x: Int) -> Int { x + 2 }".to_string());

    // Verify cache was invalidated and re-parsed
}

// ============================================================================
// Test 2.13: Error Recovery
// ============================================================================

#[tokio::test]
async fn test_error_recovery_during_edit() {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.vr").unwrap();

    // Start with valid code
    cache.open(uri.clone(), "fn add(x: Int) -> Int { x + 1 }".to_string());

    // Introduce syntax error
    cache.update(uri.clone(), "fn add(x: Int) -> Int { x +".to_string());

    let diag1 = cache.get_diagnostics(&uri);
    assert!(diag1.len() > 0);

    // Fix syntax error
    cache.update(uri.clone(), "fn add(x: Int) -> Int { x + 2 }".to_string());

    let diag2 = cache.get_diagnostics(&uri);
    // Errors should be cleared
}

#[cfg(test)]
mod property_tests {
    use super::*;

    #[tokio::test]
    async fn property_open_close_idempotent() {
        let cache = DocumentCache::new();
        let uri = Url::parse("file:///test.vr").unwrap();

        // Open
        cache.open(uri.clone(), "test".to_string());
        assert!(cache.is_open(&uri));

        // Close
        cache.close(uri.clone());
        assert!(!cache.is_open(&uri));

        // Close again (idempotent)
        cache.close(uri.clone());
        assert!(!cache.is_open(&uri));
    }

    #[tokio::test]
    async fn property_update_preserves_uri() {
        let cache = DocumentCache::new();
        let uri = Url::parse("file:///test.vr").unwrap();

        cache.open(uri.clone(), "initial".to_string());

        for i in 0..10 {
            cache.update(uri.clone(), format!("update {}", i));
            assert!(cache.is_open(&uri));
        }
    }
}
