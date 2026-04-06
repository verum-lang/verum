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
// Unit tests for session.rs
//
// Migrated from src/session.rs to comply with CLAUDE.md test organization.

use std::path::PathBuf;
use verum_compiler::{CompilerOptions, Session};
use verum_diagnostics::DiagnosticBuilder;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

#[test]
fn test_session_creation() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    assert_eq!(session.error_count(), 0);
    assert_eq!(session.warning_count(), 0);
    assert!(!session.has_errors());
}

#[test]
fn test_file_loading() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Load a source string (simulating file loading)
    let source = "fn main() { 42 }";
    let file_id = session
        .load_source_string(source, PathBuf::from("test.vr"))
        .expect("Failed to load source");

    // Verify source was loaded
    let loaded = session.get_source(file_id).expect("Source not found");
    assert_eq!(loaded.source, source);
}

#[test]
fn test_diagnostics() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Create a dummy file for diagnostic
    let file_id = session
        .load_source_string("test", PathBuf::from("test.vr"))
        .expect("Failed to load source");

    // Emit a diagnostic
    let span = verum_ast::Span::new(0, 4, file_id);
    let diagnostic = DiagnosticBuilder::error()
        .message("test error")
        .span(session.convert_span(span))
        .build();

    session.emit_diagnostic(diagnostic);

    assert_eq!(session.error_count(), 1);
    assert!(session.has_errors());
}

#[test]
fn test_module_caching() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Load and parse a module
    let source = "fn main() { 42 }";
    let file_id = session
        .load_source_string(source, PathBuf::from("test.vr"))
        .expect("Failed to load source");

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let module = parser
        .parse_module(lexer, file_id)
        .expect("Failed to parse module");

    // Cache the module
    session.cache_module(file_id, module);

    // Verify it's cached
    let cached = session.get_module(file_id);
    assert!(cached.is_some());
}

// =============================================================================
// Tier Analysis Cache Tests
// =============================================================================

use verum_cbgr::analysis::RefId;
use verum_cbgr::tier_analysis::TierAnalysisResult;
use verum_cbgr::tier_types::{ReferenceTier, TierStatistics};
use verum_compiler::session::FunctionId;
use verum_common::Map;

#[test]
fn test_tier_cache_store_and_retrieve() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Create a function ID and analysis result
    let func_id = FunctionId(42);
    let mut decisions: Map<RefId, ReferenceTier> = Map::new();
    decisions.insert(RefId(0), ReferenceTier::Tier1);

    let result = TierAnalysisResult {
        decisions,
        stats: TierStatistics {
            total_refs: 1,
            tier0_count: 0,
            tier1_count: 1,
            tier2_count: 0,
            ..TierStatistics::default()
        },
        ref_to_span: Map::new(),
        ownership: None,
        concurrency: None,
        lifetime: None,
        nll: None,
    };

    // Cache the result
    session.cache_tier_analysis(func_id, result.clone());

    // Retrieve and verify
    let cached = session.get_tier_analysis(func_id);
    assert!(cached.is_some());
    let cached = cached.unwrap();
    assert_eq!(cached.stats.total_refs, 1);
    assert_eq!(cached.stats.tier1_count, 1);
}

#[test]
fn test_tier_cache_has_analysis() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    let func_id = FunctionId(100);

    // Initially should not have analysis
    assert!(!session.has_tier_analysis(func_id));

    // Add analysis
    let result = TierAnalysisResult::empty();
    session.cache_tier_analysis(func_id, result);

    // Now should have analysis
    assert!(session.has_tier_analysis(func_id));
}

#[test]
fn test_tier_statistics_update() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Initial stats should be empty
    let stats = session.tier_statistics();
    assert_eq!(stats.total_refs, 0);
    assert_eq!(stats.tier0_count, 0);

    // Update with first batch
    let batch1 = TierStatistics {
        total_refs: 20,
        tier0_count: 5,
        tier1_count: 15,
        tier2_count: 0,
        ..TierStatistics::default()
    };
    session.merge_tier_statistics(&batch1);

    let stats = session.tier_statistics();
    assert_eq!(stats.total_refs, 20);
    assert_eq!(stats.tier0_count, 5);
    assert_eq!(stats.tier1_count, 15);

    // Update with second batch (should accumulate)
    let batch2 = TierStatistics {
        total_refs: 10,
        tier0_count: 2,
        tier1_count: 8,
        tier2_count: 0,
        ..TierStatistics::default()
    };
    session.merge_tier_statistics(&batch2);

    let stats = session.tier_statistics();
    assert_eq!(stats.total_refs, 30);
    assert_eq!(stats.tier0_count, 7);
    assert_eq!(stats.tier1_count, 23);
}

#[test]
fn test_tier_cache_size() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Initially empty
    assert_eq!(session.tier_cache_size(), 0);

    // Add multiple analyses
    for i in 0..5u64 {
        let func_id = FunctionId(i);
        let result = TierAnalysisResult {
            decisions: Map::new(),
            stats: TierStatistics {
                total_refs: i,
                ..TierStatistics::default()
            },
            ref_to_span: Map::new(),
            ownership: None,
            concurrency: None,
            lifetime: None,
            nll: None,
        };
        session.cache_tier_analysis(func_id, result);
    }

    assert_eq!(session.tier_cache_size(), 5);
}

#[test]
fn test_tier_cache_clear() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Add analysis
    let func_id = FunctionId(1);
    let result = TierAnalysisResult {
        decisions: Map::new(),
        stats: TierStatistics {
            total_refs: 1,
            tier1_count: 1,
            ..TierStatistics::default()
        },
        ref_to_span: Map::new(),
        ownership: None,
        concurrency: None,
        lifetime: None,
        nll: None,
    };
    session.cache_tier_analysis(func_id, result);

    // Add statistics
    let stats = TierStatistics {
        total_refs: 5,
        tier0_count: 2,
        tier1_count: 3,
        tier2_count: 0,
        ..TierStatistics::default()
    };
    session.merge_tier_statistics(&stats);

    // Verify they exist
    assert!(session.has_tier_analysis(func_id));
    assert_eq!(session.tier_statistics().total_refs, 5);

    // Clear the cache
    session.clear_tier_cache();

    // Verify everything is cleared
    assert!(!session.has_tier_analysis(func_id));
    assert_eq!(session.tier_cache_size(), 0);
    assert_eq!(session.tier_statistics().total_refs, 0);
}

#[test]
fn test_tier_all_analyses() {
    let options = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"));
    let session = Session::new(options);

    // Add multiple analyses
    for i in 0..3u64 {
        let func_id = FunctionId(i);
        let result = TierAnalysisResult {
            decisions: Map::new(),
            stats: TierStatistics {
                total_refs: i,
                ..TierStatistics::default()
            },
            ref_to_span: Map::new(),
            ownership: None,
            concurrency: None,
            lifetime: None,
            nll: None,
        };
        session.cache_tier_analysis(func_id, result);
    }

    // Get all analyses
    let all = session.all_tier_analyses();
    assert_eq!(all.len(), 3);

    // Verify each is present
    for i in 0..3u64 {
        let func_id = FunctionId(i);
        assert!(all.contains_key(&func_id));
        assert_eq!(all.get(&func_id).unwrap().stats.total_refs, i);
    }
}
