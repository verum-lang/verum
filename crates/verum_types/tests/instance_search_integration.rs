//! Integration tests for the protocol instance search + coherence
//! checker (Phase D.4).
//!
//! These tests exercise realistic scenarios that mirror what the
//! type-checker encounters when resolving `implement P for T`
//! blocks across a multi-module project.

use verum_common::Text;
use verum_types::instance_search::{InstanceCandidate, InstanceRegistry, SearchResult};

fn cand(p: &str, t: &str, loc: &str) -> InstanceCandidate {
    InstanceCandidate::new(p, t).at(loc)
}

// ==================== Basic search ====================

#[test]
fn search_empty_registry_returns_not_found() {
    let reg = InstanceRegistry::new();
    assert_eq!(reg.search("Monoid", "Int"), SearchResult::NotFound);
}

#[test]
fn search_single_match_returns_unique() {
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "Int", "core/math/algebra.vr:120"));
    match reg.search("Monoid", "Int") {
        SearchResult::Unique(c) => {
            assert_eq!(c.protocol.as_str(), "Monoid");
            assert_eq!(c.target_type.as_str(), "Int");
            assert!(c.source_location.as_str().contains("algebra.vr"));
        }
        other => panic!("expected Unique, got {:?}", other),
    }
}

#[test]
fn search_orthogonal_implementations() {
    // Monoid for Z3 and Monoid for Nat4 are orthogonal — both should
    // resolve uniquely.
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "Z3", "a"));
    reg.register(cand("Monoid", "Nat4", "b"));
    assert!(matches!(reg.search("Monoid", "Z3"), SearchResult::Unique(_)));
    assert!(matches!(reg.search("Monoid", "Nat4"), SearchResult::Unique(_)));
    assert_eq!(reg.search("Monoid", "F2"), SearchResult::NotFound);
}

// ==================== Coherence violations ====================

#[test]
fn duplicate_implementation_is_ambiguous() {
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "Int", "mod1.vr:10"));
    reg.register(cand("Monoid", "Int", "mod2.vr:20"));
    match reg.search("Monoid", "Int") {
        SearchResult::Ambiguous(cs) => {
            assert_eq!(cs.len(), 2);
            assert!(cs
                .iter()
                .any(|c| c.source_location.as_str().contains("mod1")));
            assert!(cs
                .iter()
                .any(|c| c.source_location.as_str().contains("mod2")));
        }
        other => panic!("expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn check_coherence_reports_all_violations() {
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "Int", "a"));
    reg.register(cand("Monoid", "Int", "b")); // violation 1
    reg.register(cand("Group", "Nat", "c"));
    reg.register(cand("Group", "Nat", "d")); // violation 2
    reg.register(cand("Ring", "Float", "e")); // coherent
    let report = reg.check_coherence();
    assert!(!report.is_coherent());
    assert_eq!(report.violations.len(), 2);
    assert_eq!(report.total_instances, 5);
}

#[test]
fn check_coherence_clean_registry() {
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "Int", "a"));
    reg.register(cand("Monoid", "Float", "b"));
    reg.register(cand("Group", "Int", "c"));
    reg.register(cand("Ring", "Int", "d"));
    let report = reg.check_coherence();
    assert!(report.is_coherent());
    assert_eq!(report.violations.len(), 0);
    assert_eq!(report.total_instances, 4);
}

// ==================== Algebraic hierarchy scenarios ====================

#[test]
fn algebraic_hierarchy_is_coherent() {
    // Mirror the actual Verum stdlib: Z3 implements Magma + Semigroup
    // + Monoid + Group + AbelianGroup, all coherent.
    let mut reg = InstanceRegistry::new();
    for protocol in ["Magma", "Semigroup", "Monoid", "Group", "AbelianGroup"] {
        reg.register(cand(protocol, "Z3", "core/math/examples.vr"));
    }
    let report = reg.check_coherence();
    assert!(report.is_coherent());
    assert_eq!(report.total_instances, 5);
}

#[test]
fn multiple_concrete_types_per_protocol_is_coherent() {
    // Mirror: Monoid is implemented for Z3, Nat4, TwoFreeMonoid, F2.
    let mut reg = InstanceRegistry::new();
    for target in ["Z3", "Nat4", "TwoFreeMonoid", "F2"] {
        reg.register(cand("Monoid", target, "core/math/examples.vr"));
    }
    let report = reg.check_coherence();
    assert!(report.is_coherent());
    assert_eq!(report.total_instances, 4);
}

// ==================== Generic protocols ====================

#[test]
fn candidate_with_protocol_args() {
    let c = InstanceCandidate::new("Category", "IntegerPathCategory")
        .with_args([Text::from("Int"), Text::from("PathInt")])
        .at("core/math/examples.vr:112");
    assert_eq!(c.protocol_args.len(), 2);
    assert_eq!(c.protocol_args[0].as_str(), "Int");
    assert_eq!(c.protocol_args[1].as_str(), "PathInt");
}

#[test]
fn search_ignores_protocol_args_variance() {
    // Current simple keying: (protocol, target) — protocol_args are
    // stored on the candidate but not in the lookup key. Two different
    // arg instantiations are considered duplicates for coherence.
    let mut reg = InstanceRegistry::new();
    let c1 = InstanceCandidate::new("Category", "IPC")
        .with_args([Text::from("Int"), Text::from("PathInt")])
        .at("a");
    let c2 = InstanceCandidate::new("Category", "IPC")
        .with_args([Text::from("Nat"), Text::from("PathNat")])
        .at("b");
    reg.register(c1);
    reg.register(c2);
    match reg.search("Category", "IPC") {
        SearchResult::Ambiguous(cs) => assert_eq!(cs.len(), 2),
        other => panic!("expected Ambiguous, got {:?}", other),
    }
}

// ==================== Scale ====================

#[test]
fn registry_handles_large_scale() {
    let mut reg = InstanceRegistry::new();
    // Register 100 unique (protocol, type) pairs
    for i in 0..100 {
        reg.register(cand(
            &format!("Protocol{}", i / 10),
            &format!("Type{}", i % 10),
            &format!("loc{}", i),
        ));
    }
    assert_eq!(reg.len(), 100);
    let report = reg.check_coherence();
    assert!(report.is_coherent());
    assert_eq!(report.total_instances, 100);
}

#[test]
fn registry_empty_defaults() {
    let reg = InstanceRegistry::new();
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
    let report = reg.check_coherence();
    assert!(report.is_coherent());
    assert_eq!(report.total_instances, 0);
}

#[test]
fn coherence_report_describes_violation() {
    let mut reg = InstanceRegistry::new();
    reg.register(cand("Monoid", "T", "file_a.vr:10"));
    reg.register(cand("Monoid", "T", "file_b.vr:20"));
    let report = reg.check_coherence();
    let v = &report.violations[0];
    assert_eq!(v.protocol.as_str(), "Monoid");
    assert_eq!(v.target_type.as_str(), "T");
    assert_eq!(v.conflicting_locations.len(), 2);
}
