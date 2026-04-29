//! Locks the protocol hierarchy traversal in
//! `SpecializationVerifier::is_subprotocol`.
//!
//! Pre-fix the function returned `false` for everything except the trivial
//! reflexive case (`sub == super`). The comment admitted "Full implementation
//! would traverse the protocol hierarchy graph" but used `false` as a
//! conservative default.
//!
//! Impact: `type_implements_protocol_local` (called from coherence checking)
//! uses `is_subprotocol` to decide whether a type implementing a subprotocol
//! also satisfies a superprotocol bound. Without traversal, every transitive
//! implication was lost — e.g., `Iterator : IntoIterator` did not propagate,
//! so a type with only an `Iterator` impl was wrongly judged not to implement
//! `IntoIterator`, narrowing valid specializations.
//!
//! Post-fix: BFS over `super_protocols` graph from `register_protocol`'s
//! data, with cycle guard.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{List, Map, Maybe};
use verum_protocol_types::protocol_base::{Protocol, ProtocolBound};
use verum_smt::specialization_coherence::SpecializationVerifier;

fn path_named(name: &str) -> Path {
    Path {
        segments: vec![PathSegment::Name(Ident {
            name: name.into(),
            span: Span::dummy(),
        })]
        .into(),
        span: Span::dummy(),
    }
}

fn protocol_with_supers(name: &str, supers: &[&str]) -> Protocol {
    let super_protocols: List<ProtocolBound> = supers
        .iter()
        .map(|s| ProtocolBound::positive(path_named(s), List::new()))
        .collect();
    Protocol {
        name: name.into(),
        type_params: List::new(),
        super_protocols,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        methods: Map::new(),
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

#[test]
fn reflexive_is_always_a_subprotocol() {
    let verifier = SpecializationVerifier::new().expect("verifier");
    assert!(verifier.is_subprotocol_by_name("Eq", "Eq"));
    assert!(verifier.is_subprotocol_by_name("Iterator", "Iterator"));
}

#[test]
fn empty_registry_means_no_subprotocols_except_reflexive() {
    let verifier = SpecializationVerifier::new().expect("verifier");
    // No protocols registered — the only true relationships are reflexive.
    assert!(!verifier.is_subprotocol_by_name("Iterator", "IntoIterator"));
    assert!(!verifier.is_subprotocol_by_name("Foo", "Bar"));
}

#[test]
fn direct_subprotocol_is_recognized() {
    let mut verifier = SpecializationVerifier::new().expect("verifier");
    verifier.register_protocol(protocol_with_supers("Iterator", &["IntoIterator"]));
    verifier.register_protocol(protocol_with_supers("IntoIterator", &[]));

    assert!(
        verifier.is_subprotocol_by_name("Iterator", "IntoIterator"),
        "Iterator declares IntoIterator as super — the walker must follow that edge"
    );
    // Inverse direction is not implied: IntoIterator is NOT a subprotocol
    // of Iterator (super-protocol relations are directed).
    assert!(!verifier.is_subprotocol_by_name("IntoIterator", "Iterator"));
}

#[test]
fn transitive_subprotocol_is_recognized() {
    let mut verifier = SpecializationVerifier::new().expect("verifier");
    // A : B : C — three-step chain.
    verifier.register_protocol(protocol_with_supers("A", &["B"]));
    verifier.register_protocol(protocol_with_supers("B", &["C"]));
    verifier.register_protocol(protocol_with_supers("C", &[]));

    assert!(verifier.is_subprotocol_by_name("A", "B"));
    assert!(verifier.is_subprotocol_by_name("B", "C"));
    assert!(
        verifier.is_subprotocol_by_name("A", "C"),
        "transitive A : C must be inferred from A : B : C — the BFS walk must \
         enqueue all parents discovered along the way"
    );
}

#[test]
fn diamond_inheritance_is_supported() {
    // A inherits from both B and C; B and C both inherit from D.
    //   D
    //  / \
    // B   C
    //  \ /
    //   A
    let mut verifier = SpecializationVerifier::new().expect("verifier");
    verifier.register_protocol(protocol_with_supers("A", &["B", "C"]));
    verifier.register_protocol(protocol_with_supers("B", &["D"]));
    verifier.register_protocol(protocol_with_supers("C", &["D"]));
    verifier.register_protocol(protocol_with_supers("D", &[]));

    assert!(verifier.is_subprotocol_by_name("A", "B"));
    assert!(verifier.is_subprotocol_by_name("A", "C"));
    assert!(verifier.is_subprotocol_by_name("A", "D"));
    // D is the root — nothing is a subprotocol of A from D's side.
    assert!(!verifier.is_subprotocol_by_name("D", "A"));
}

#[test]
fn cycle_terminates_safely() {
    // Malformed declaration: A : B and B : A. Real coherence checks reject
    // this elsewhere, but the walker must still terminate so the diagnostic
    // path itself doesn't hang the compiler.
    let mut verifier = SpecializationVerifier::new().expect("verifier");
    verifier.register_protocol(protocol_with_supers("A", &["B"]));
    verifier.register_protocol(protocol_with_supers("B", &["A"]));

    // Both directions should report true for transitive subprotocol because
    // each can reach the other through the cycle.
    assert!(verifier.is_subprotocol_by_name("A", "B"));
    assert!(verifier.is_subprotocol_by_name("B", "A"));
    // Neither is a subprotocol of an unrelated protocol.
    assert!(!verifier.is_subprotocol_by_name("A", "Unrelated"));
}

#[test]
fn unrelated_protocols_are_not_subprotocols() {
    let mut verifier = SpecializationVerifier::new().expect("verifier");
    verifier.register_protocol(protocol_with_supers("Eq", &[]));
    verifier.register_protocol(protocol_with_supers("Display", &[]));
    verifier.register_protocol(protocol_with_supers("Hash", &["Eq"]));

    assert!(!verifier.is_subprotocol_by_name("Display", "Eq"));
    assert!(!verifier.is_subprotocol_by_name("Eq", "Display"));
    // Hash : Eq, but Hash has no relationship with Display.
    assert!(verifier.is_subprotocol_by_name("Hash", "Eq"));
    assert!(!verifier.is_subprotocol_by_name("Hash", "Display"));
}
