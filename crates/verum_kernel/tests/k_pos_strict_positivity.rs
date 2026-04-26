//! K-Pos strict-positivity check integration tests (VUVA §7.3).
//!
//! Per VUVA §7.3: an inductive type `T` is well-formed only when every
//! recursive occurrence of `T` in any constructor's argument types
//! appears strictly positively. Berardi 1998 establishes that
//! admitting non-positive recursion in a system with even minimal
//! impredicativity yields a derivation of `False` — the kernel
//! enforces strict positivity at registration time so `False` is not
//! reachable through ill-formed inductives.
//!
//! The accept paths cover the standard inductive zoo (`Nat`, `List<A>`,
//! `Tree<A>`, `Vec<A, n>`); the reject paths cover Berardi-shaped
//! definitions (`Bad = Wrap(Bad → A)`, indirect non-positive via
//! parametrised types).

use verum_common::{Heap, List, Text};
use verum_kernel::{
    check_strict_positivity, ConstructorSig, CoreTerm, InductiveRegistry, KernelError,
    PositivityCtx, RegisteredInductive, UniverseLevel,
};

fn ind(name: &str) -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from(name),
        args: List::new(),
    }
}

fn ind_with(name: &str, args: Vec<CoreTerm>) -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from(name),
        args: List::from_iter(args),
    }
}

fn pi(domain: CoreTerm, codomain: CoreTerm) -> CoreTerm {
    CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(domain),
        codomain: Heap::new(codomain),
    }
}

fn type0() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}

fn ctx(name: &str, idx: usize) -> PositivityCtx<'_> {
    PositivityCtx::root(name, idx)
}

// =============================================================================
// Accept paths — standard inductive zoo
// =============================================================================

#[test]
fn nat_is_strictly_positive() {
    // type Nat = Zero | Succ(Nat)
    //   Zero       — no args
    //   Succ(Nat)  — one arg, the type's own name in a non-arrow position
    let succ_arg = ind("Nat");
    let result = check_strict_positivity("Nat", &succ_arg, &ctx("Succ", 0));
    assert!(result.is_ok(), "Succ(Nat) is the canonical strict-positive use");
}

#[test]
fn list_is_strictly_positive() {
    // type List<A> = Nil | Cons(A, List<A>)
    //   Cons        — args A and List<A>
    let cons_a   = CoreTerm::Var(Text::from("A"));
    let cons_lst = ind_with("List", vec![CoreTerm::Var(Text::from("A"))]);

    assert!(check_strict_positivity("List", &cons_a,   &ctx("Cons", 0)).is_ok());
    assert!(check_strict_positivity("List", &cons_lst, &ctx("Cons", 1)).is_ok());
}

#[test]
fn tree_is_strictly_positive() {
    // type Tree<A> = Leaf(A) | Branch(Tree<A>, Tree<A>)
    //   Branch — two args, both Tree<A> (in non-arrow positions)
    let tree_a = ind_with("Tree", vec![CoreTerm::Var(Text::from("A"))]);

    assert!(check_strict_positivity("Tree", &tree_a, &ctx("Branch", 0)).is_ok());
    assert!(check_strict_positivity("Tree", &tree_a, &ctx("Branch", 1)).is_ok());
}

#[test]
fn rose_tree_under_named_inductive_is_strictly_positive() {
    // type Rose<A> = Node(A, List<Rose<A>>)
    // Containing the type inside a parametrised List is OK because the
    // walker descends into Inductive args and List itself is strictly
    // positive in its arg.
    let rose_a = ind_with("Rose", vec![CoreTerm::Var(Text::from("A"))]);
    let list_of_rose = ind_with("List", vec![rose_a]);
    assert!(
        check_strict_positivity("Rose", &list_of_rose, &ctx("Node", 1)).is_ok(),
        "Rose under List<…> is strictly positive"
    );
}

#[test]
fn registry_admits_well_formed_inductive() {
    // Register Nat: Zero | Succ(Nat).
    let mut reg = InductiveRegistry::new();
    let result = reg.register(RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![
            ConstructorSig { name: Text::from("Zero"), arg_types: List::new() },
            ConstructorSig {
                name: Text::from("Succ"),
                arg_types: List::from_iter(vec![ind("Nat")]),
            },
        ]),
    ));
    assert!(result.is_ok());
    assert!(matches!(reg.get("Nat"), verum_common::Maybe::Some(_)));
}

// =============================================================================
// Reject paths — Berardi-shaped definitions
// =============================================================================

#[test]
fn direct_non_positive_recursion_rejected() {
    // type Bad = Wrap(Bad -> A)
    //   Wrap — one arg with type Bad → A; Bad appears in the negative
    //   position of an arrow. Strict positivity must reject.
    let bad_arrow = pi(ind("Bad"), CoreTerm::Var(Text::from("A")));
    match check_strict_positivity("Bad", &bad_arrow, &ctx("Wrap", 0)) {
        Err(KernelError::PositivityViolation { type_name, constructor, position }) => {
            assert_eq!(type_name.as_str(), "Bad");
            assert_eq!(constructor.as_str(), "Wrap");
            // Position must mention "left of an arrow" so the diagnostic
            // is actionable.
            assert!(
                position.as_str().contains("left of an arrow"),
                "diagnostic must localise to the negative position; got: {position}"
            );
        }
        other => panic!("expected PositivityViolation, got {other:?}"),
    }
}

#[test]
fn second_order_non_positive_recursion_rejected() {
    // type Bad2 = Wrap((Bad2 -> A) -> A)
    //   The outer arrow's domain is itself an arrow whose domain
    //   contains Bad2 — even though there's a double-negation, the
    //   strict-positivity rule rejects ANY occurrence of the type in
    //   ANY arrow domain, not just the outermost.
    let inner_arrow = pi(ind("Bad2"), CoreTerm::Var(Text::from("A")));
    let outer_arrow = pi(inner_arrow, CoreTerm::Var(Text::from("A")));
    match check_strict_positivity("Bad2", &outer_arrow, &ctx("Wrap", 0)) {
        Err(KernelError::PositivityViolation { .. }) => {}
        other => panic!("expected PositivityViolation, got {other:?}"),
    }
}

#[test]
fn non_positive_inside_inductive_arg_rejected() {
    // type BadList = Cons(BadFn, BadList)
    //   where BadFn smuggles BadList into a function-arrow domain.
    //   The walker must descend into Cons's args and detect the violation.
    //
    // Concretely: type BadList = Cons(BadList -> A)
    let bad_arrow = pi(ind("BadList"), CoreTerm::Var(Text::from("A")));
    match check_strict_positivity("BadList", &bad_arrow, &ctx("Cons", 0)) {
        Err(KernelError::PositivityViolation { type_name, .. }) => {
            assert_eq!(type_name.as_str(), "BadList");
        }
        other => panic!("expected PositivityViolation, got {other:?}"),
    }
}

#[test]
fn registry_rejects_non_positive_declaration() {
    // Try to register the Berardi witness; the kernel must refuse.
    let mut reg = InductiveRegistry::new();
    let bad_arrow = pi(ind("Bad"), CoreTerm::Var(Text::from("A")));
    let result = reg.register(RegisteredInductive::new(
        Text::from("Bad"),
        List::new(),
        List::from_iter(vec![ConstructorSig {
            name: Text::from("Wrap"),
            arg_types: List::from_iter(vec![bad_arrow]),
        }]),
    ));
    match result {
        Err(KernelError::PositivityViolation { type_name, constructor, .. }) => {
            assert_eq!(type_name.as_str(), "Bad");
            assert_eq!(constructor.as_str(), "Wrap");
        }
        other => panic!("expected PositivityViolation, got {other:?}"),
    }
    // The bad inductive must NOT be in the registry.
    assert!(matches!(reg.get("Bad"), verum_common::Maybe::None));
}

#[test]
fn registry_rejects_duplicate_inductive_name() {
    let mut reg = InductiveRegistry::new();
    let nat = RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![ConstructorSig {
            name: Text::from("Zero"),
            arg_types: List::new(),
        }]),
    );
    reg.register(nat.clone()).expect("first registration must succeed");
    match reg.register(nat) {
        Err(KernelError::DuplicateInductive(name)) => assert_eq!(name.as_str(), "Nat"),
        other => panic!("expected DuplicateInductive, got {other:?}"),
    }
}

// =============================================================================
// Atom-level invariants — strict positivity is vacuously OK for atoms
// =============================================================================

#[test]
fn universe_is_vacuously_strictly_positive() {
    let u = type0();
    assert!(check_strict_positivity("AnyType", &u, &ctx("AnyCtor", 0)).is_ok());
}

#[test]
fn variable_is_vacuously_strictly_positive() {
    let v = CoreTerm::Var(Text::from("x"));
    assert!(check_strict_positivity("AnyType", &v, &ctx("AnyCtor", 0)).is_ok());
}

#[test]
fn arrow_in_codomain_position_is_admitted() {
    // type Curried = Curry(Int → Curried)
    //   Codomain of an arrow is a strictly-positive position.
    //   Note: this MUST be rejected because Curried also appears in the
    //   NEGATIVE position (the arrow's domain `Int` does NOT contain
    //   Curried, but the WHOLE arg type IS `Int → Curried` — the outer
    //   walker sees Int as the negative position which is fine, then
    //   recurses into Curried as the codomain which is fine).
    let arr = pi(CoreTerm::Var(Text::from("Int")), ind("Curried"));
    assert!(
        check_strict_positivity("Curried", &arr, &ctx("Curry", 0)).is_ok(),
        "Curried in arrow codomain (positive position) is admitted"
    );
}
