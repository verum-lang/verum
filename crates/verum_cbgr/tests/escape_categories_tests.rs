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
// Tests for escape_categories module
// Migrated from src/escape_categories.rs per CLAUDE.md standards

use verum_cbgr::analysis::{EscapeResult, RefId};
use verum_cbgr::escape_categories::*;

#[test]
fn test_escape_categories() {
    assert_eq!(EscapeCategory::NoEscape.cbgr_cost_ns(), 0);
    assert_eq!(EscapeCategory::LocalEscape.cbgr_cost_ns(), 15);
    assert_eq!(EscapeCategory::HeapEscape.cbgr_cost_ns(), 15);
    assert_eq!(EscapeCategory::ThreadEscape.cbgr_cost_ns(), 15);
}

#[test]
fn test_sbgl_applicability() {
    assert!(EscapeCategory::NoEscape.sbgl_applicable());
    assert!(!EscapeCategory::LocalEscape.sbgl_applicable());
    assert!(!EscapeCategory::HeapEscape.sbgl_applicable());
    assert!(!EscapeCategory::ThreadEscape.sbgl_applicable());
}

#[test]
fn test_categorize_escape() {
    assert_eq!(
        categorize_escape(EscapeResult::DoesNotEscape),
        EscapeCategory::NoEscape
    );
    assert_eq!(
        categorize_escape(EscapeResult::EscapesViaReturn),
        EscapeCategory::LocalEscape
    );
    assert_eq!(
        categorize_escape(EscapeResult::EscapesViaHeap),
        EscapeCategory::HeapEscape
    );
    assert_eq!(
        categorize_escape(EscapeResult::EscapesViaThread),
        EscapeCategory::ThreadEscape
    );
}

#[test]
fn test_optimization_decision() {
    let decision = OptimizationDecision::for_category(EscapeCategory::NoEscape);
    assert_eq!(decision, OptimizationDecision::ApplySbgl);
    assert_eq!(decision.expected_cost_ns(), 0);

    let decision = OptimizationDecision::for_category(EscapeCategory::LocalEscape);
    assert_eq!(decision, OptimizationDecision::UseCbgr);
    assert_eq!(decision.expected_cost_ns(), 15);
}

#[test]
fn test_diagnostic() {
    let ref_id = RefId(42);
    let diagnostic = SbglDiagnostic::new(ref_id, EscapeResult::DoesNotEscape);

    assert_eq!(diagnostic.category, EscapeCategory::NoEscape);
    assert!(diagnostic.sbgl_applicable());
    assert!(diagnostic.warning_message().is_none());

    let diagnostic = SbglDiagnostic::new(ref_id, EscapeResult::EscapesViaReturn);
    assert_eq!(diagnostic.category, EscapeCategory::LocalEscape);
    assert!(!diagnostic.sbgl_applicable());
    assert!(diagnostic.warning_message().is_some());
}
