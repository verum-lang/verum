//! Regression: `get_type_constructors` must read constructor data
//! from the `variant_map` + `variant_recursive_args` registries —
//! NOT a hardcoded match on stdlib names.
//!
//! Pre-fix the function had a hardcoded match arm for "Nat" / "List" /
//! "Tree" / "BinaryTree" / "Bool" with their constructors, and
//! returned "Unknown inductive type" for everything else. User-defined
//! variants couldn't drive `induction`.
//!
//! Post-fix: registered types resolve via metadata; non-recursive
//! variants like `Color = Red | Green | Blue` work without recursion
//! info; recursive types use the new
//! `register_variant_recursion(name, recursive_args)` API.

use verum_common::Text;
use verum_smt::proof_search::ProofSearchEngine;

#[test]
fn registered_non_recursive_variant_yields_constructors_with_no_recursive_args() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Color"),
        vec![
            Text::from("Red"),
            Text::from("Green"),
            Text::from("Blue"),
        ],
    );

    let ctors = engine
        .get_type_constructors_for_test(&Text::from("Color"))
        .expect("Color must resolve");
    assert_eq!(ctors.len(), 3, "Color has 3 constructors");
    let names: Vec<&str> = ctors.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["Red", "Green", "Blue"]);
    // No recursion info registered → all empty.
    for (_, args) in &ctors {
        assert!(args.is_empty(), "Color is non-recursive — no recursive args expected");
    }
}

#[test]
fn registered_recursive_variant_with_explicit_recursion_carries_arg_positions() {
    // `List<T> = Nil | Cons(T, List<T>)`: Cons recurses on arg index 1.
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("List"),
        vec![Text::from("Nil"), Text::from("Cons")],
    );
    engine.register_variant_recursion(
        Text::from("List"),
        vec![vec![], vec![1]],
    );

    let ctors = engine
        .get_type_constructors_for_test(&Text::from("List"))
        .expect("List must resolve");
    assert_eq!(ctors.len(), 2);
    assert_eq!(ctors[0].0.as_str(), "Nil");
    assert!(ctors[0].1.is_empty());
    assert_eq!(ctors[1].0.as_str(), "Cons");
    assert_eq!(ctors[1].1, vec![1usize]);
}

#[test]
fn unregistered_type_errors_with_pointer_to_register_variant_type() {
    let engine = ProofSearchEngine::new();
    let result = engine.get_type_constructors_for_test(&Text::from("MadeUpType"));
    assert!(
        result.is_err(),
        "unregistered type must fail — pre-fix this fell through to a hardcoded match"
    );
    let msg = format!("{}", result.err().unwrap());
    assert!(
        msg.contains("MadeUpType") && msg.contains("register_variant_type"),
        "error must name the type and point at the remediation API. got: {}",
        msg
    );
}

#[test]
fn no_hardcoded_nat_or_list_or_tree_or_bool() {
    // The exact case the hardcoded match shielded: a fresh engine
    // with NO types registered must NOT magically know about Nat,
    // List, Tree, BinaryTree, or Bool. They have to come from
    // metadata like every other type — no stdlib-name knowledge in
    // the compiler.
    let engine = ProofSearchEngine::new();
    for name in &["Nat", "Natural", "List", "Tree", "BinaryTree", "Bool"] {
        let result = engine.get_type_constructors_for_test(&Text::from(*name));
        assert!(
            result.is_err(),
            "engine must NOT carry hardcoded knowledge of {} — got {:?}",
            name,
            result
        );
    }
}

#[test]
fn recursion_info_shorter_than_ctor_list_defaults_remaining_to_empty() {
    // Defensive: if the user registers ctors first then partial
    // recursion info, the missing tail entries must default to
    // empty (no recursive args) rather than panic on out-of-bounds
    // access.
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("PartiallyKnown"),
        vec![
            Text::from("A"),
            Text::from("B"),
            Text::from("C"),
        ],
    );
    engine.register_variant_recursion(
        Text::from("PartiallyKnown"),
        vec![vec![]], // only A's info; B and C should default to empty
    );

    let ctors = engine
        .get_type_constructors_for_test(&Text::from("PartiallyKnown"))
        .expect("PartiallyKnown must resolve");
    assert_eq!(ctors.len(), 3);
    assert!(ctors.iter().all(|(_, args)| args.is_empty()));
}
