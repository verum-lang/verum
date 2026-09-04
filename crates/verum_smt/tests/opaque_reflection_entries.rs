//! An OPAQUE reflected entry — a function whose body this translator
//! cannot express, kept so its NAME is declared (T0905).
//!
//! The distinction being pinned here is between two things that both
//! used to answer `None`:
//!
//!   DETERMINISM decline    nothing true can be said; emit nothing.
//!   TRANSLATABILITY decline
//!                          the function IS a function; `f(x) == f(x)`
//!                          is true and `(declare-fun f …)` says it.
//!
//! Without the declaration a goal mentioning the name fails on
//! WELL-FORMEDNESS rather than on its merits, which reads in the
//! diagnostics exactly like a failed proof.
//!
//! The empty `body_smtlib` is the carrier of "opaque". These tests
//! exist because that encoding is easy to break in a way nothing else
//! notices: emitting `(assert (= (f x) ))` is not a weaker axiom, it
//! is a syntax error that makes Z3 reject the WHOLE block — so the
//! failure would appear as every unrelated theorem in the module
//! regressing at once.

use verum_common::{List, Text};

use verum_smt::refinement_reflection::{ReflectedFunction, RefinementReflectionRegistry};

/// A normal, fully translated entry: `double(n) = 2n`.
fn definitional() -> ReflectedFunction {
    ReflectedFunction {
        name: Text::from("double"),
        parameters: List::from_iter([Text::from("n")]),
        body_smtlib: Text::from("(* 2 n)"),
        return_sort: Text::from("Int"),
        parameter_sorts: List::from_iter([Text::from("Int")]),
        aux_decls: List::new(),
    }
}

/// An opaque entry: signature known, body inexpressible. The EMPTY
/// `body_smtlib` is what says so.
fn opaque() -> ReflectedFunction {
    ReflectedFunction {
        name: Text::from("sum_all"),
        parameters: List::from_iter([Text::from("xs")]),
        body_smtlib: Text::from(""),
        return_sort: Text::from("Int"),
        parameter_sorts: List::from_iter([Text::from("Verum!List")]),
        aux_decls: List::from_iter([Text::from("(declare-sort Verum!List 0)")]),
    }
}

/// A definitional entry whose body CALLS the opaque one. Before
/// T0905 this was dropped by the closure pass, because `sum_all` was
/// registered nowhere and therefore counted as an undeclared symbol.
fn caller_of_opaque() -> ReflectedFunction {
    ReflectedFunction {
        name: Text::from("twice_sum"),
        parameters: List::from_iter([Text::from("xs")]),
        body_smtlib: Text::from("(* 2 (sum_all xs))"),
        return_sort: Text::from("Int"),
        parameter_sorts: List::from_iter([Text::from("Verum!List")]),
        aux_decls: List::from_iter([Text::from("(declare-sort Verum!List 0)")]),
    }
}

// ---------------------------------------------------------------
// POSITIVE CONTROL
// ---------------------------------------------------------------

/// Nothing below distinguishes "opaque works" from "the emitter emits
/// nothing at all", so the ordinary case is asserted first. If this
/// fails, every assertion after it is vacuous.
#[test]
fn a_definitional_entry_still_emits_both_a_declaration_and_an_axiom() {
    let f = definitional();
    let decl = f.to_smtlib_decl();
    let axiom = f.to_smtlib_axiom();

    assert_eq!(decl.as_str(), "(declare-fun double (Int) Int)");
    assert!(
        axiom.as_str().contains("(* 2 n)"),
        "the definitional axiom must carry the body; got {:?}",
        axiom.as_str()
    );
    assert!(axiom.as_str().starts_with("(assert "), "got {:?}", axiom.as_str());
}

// ---------------------------------------------------------------
// THE OPAQUE ENTRY ITSELF
// ---------------------------------------------------------------

#[test]
fn an_opaque_entry_declares_its_symbol() {
    assert_eq!(
        opaque().to_smtlib_decl().as_str(),
        "(declare-fun sum_all (Verum!List) Int)"
    );
}

#[test]
fn an_opaque_entry_states_no_axiom() {
    // Not "a weaker axiom" — NO axiom. The empty Text is the contract
    // `to_smtlib_block` reads to decide whether to emit a line at all.
    assert_eq!(
        opaque().to_smtlib_axiom().as_str(),
        "",
        "an empty body must yield the empty Text, never a malformed assert"
    );
}

#[test]
fn the_block_declares_the_opaque_symbol_and_asserts_nothing_about_it() {
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(opaque()).unwrap();
    let block = reg.to_smtlib_block();
    let s = block.as_str();

    assert!(
        s.contains("(declare-fun sum_all (Verum!List) Int)"),
        "block must declare the opaque symbol; got:\n{}",
        s
    );
    assert!(
        s.contains("(declare-sort Verum!List 0)"),
        "the opaque parameter sort travels with the entry; got:\n{}",
        s
    );
    // The whole point: the name exists, and NOTHING is claimed of it.
    assert!(
        !s.contains("(assert"),
        "an opaque entry must contribute no assertion; got:\n{}",
        s
    );
}

#[test]
fn an_opaque_entry_leaves_no_blank_assert_line() {
    // The malformed form this guards against is `(assert (= (f x) ))`,
    // which Z3 rejects for the entire block — so the symptom would be
    // every unrelated theorem in the module failing at once, with
    // nothing pointing here.
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(opaque()).unwrap();
    reg.register(definitional()).unwrap();
    let block = reg.to_smtlib_block();

    for line in block.as_str().lines() {
        assert!(
            !line.contains("(= (sum_all"),
            "opaque entry produced an equation line: {:?}",
            line
        );
    }
    // …and the healthy neighbour in the same block is unaffected.
    assert!(
        block.as_str().contains("(* 2 n)"),
        "the definitional entry must still emit its axiom alongside\n{}",
        block.as_str()
    );
}

// ---------------------------------------------------------------
// THE SECOND EFFECT: THE CLOSURE PASS STOPS DROPPING THE CALLER
// ---------------------------------------------------------------

#[test]
fn an_entry_calling_an_unregistered_helper_is_dropped() {
    // The red pole, stated as a measurement rather than remembered:
    // this is what happened to EVERY caller of an un-translatable
    // helper before T0905, and it is why the fix has a second effect
    // beyond declaring the helper itself.
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(caller_of_opaque()).unwrap();

    let drops = reg.open_entry_drops();
    assert_eq!(
        drops.len(),
        1,
        "expected exactly one drop, got {:?}",
        drops.iter().map(|d| d.name.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(drops[0].name.as_str(), "twice_sum");
    assert_eq!(drops[0].missing_symbol.as_str(), "sum_all");
}

#[test]
fn registering_the_opaque_helper_keeps_its_caller_alive() {
    // Same registry, one entry added — the helper's DECLARATION is
    // enough to close the call graph, even though it carries no
    // axiom. This is the effect that reaches real modules: a
    // predicate written over a loop-bodied helper stops vanishing.
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(caller_of_opaque()).unwrap();
    reg.register(opaque()).unwrap();

    assert!(
        reg.open_entry_drops().is_empty(),
        "an opaque declaration must close the call graph; still dropped: {:?}",
        reg.open_entry_drops()
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
    );

    let block = reg.to_smtlib_block();
    let s = block.as_str();
    assert!(
        s.contains("(* 2 (sum_all xs))"),
        "the caller's axiom must survive; got:\n{}",
        s
    );
    assert!(
        s.contains("(declare-fun sum_all (Verum!List) Int)"),
        "…and the helper it names must be declared above it; got:\n{}",
        s
    );
}

#[test]
fn the_helpers_declaration_precedes_the_axiom_that_uses_it() {
    // Z3 reads the block in order. A declaration after its use is
    // rejected exactly as loudly as no declaration at all, so the
    // ordering is part of the contract, not a stylistic preference.
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(caller_of_opaque()).unwrap();
    reg.register(opaque()).unwrap();
    let block = reg.to_smtlib_block();
    let s = block.as_str();

    let decl_at = s
        .find("(declare-fun sum_all")
        .expect("helper declaration missing");
    let use_at = s
        .find("(* 2 (sum_all xs))")
        .expect("caller axiom missing");
    assert!(
        decl_at < use_at,
        "declaration at {} must precede its use at {};\n{}",
        decl_at,
        use_at,
        s
    );
}
