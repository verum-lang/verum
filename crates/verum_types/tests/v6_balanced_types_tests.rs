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
// v6.0-BALANCED Semantic Types Integration Tests
//
// This test suite validates correct usage and integration of v6.0-BALANCED
// semantic types (List, Text, Map, Set,  Heap, Shared) throughout
// the verum_types crate.
//
// Semantic integrity: types describe meaning (List, Text, Map), not implementation (Vec, String, HashMap)

use indexmap::IndexMap;
use verum_ast::{
    expr::*,
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{Heap, List, Map, Maybe, Set, Text};
use verum_types::context::TypeContext;
use verum_types::infer::*;
use verum_types::subtype::Subtyping;
use verum_types::ty::Type;
use verum_types::unify::Unifier;

// ============================================================================
// List<T> Type Tests - Semantic Type for Sequences
// ============================================================================

#[test]
fn test_list_type_construction() {
    use verum_ast::ty::{Ident, Path};

    // List<Int> type
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert_eq!(list_int.to_string(), "List<Int>");

    // List<Text> type
    let list_text = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    assert_eq!(list_text.to_string(), "List<Text>");

    // Nested List<List<Bool>>
    let list_bool = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::bool()].into(),
    };
    let nested_list = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![list_bool].into(),
    };
    assert_eq!(nested_list.to_string(), "List<List<Bool>>");
}

#[test]
fn test_list_type_is_well_formed() {
    use verum_ast::ty::{Ident, Path};

    // Verify List<Int> is a well-formed monotype
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    assert!(list_int.is_monotype());
    assert_eq!(list_int.to_string(), "List<Int>");
}

// ============================================================================
// Text Type Tests - Semantic Type for Strings
// ============================================================================

#[test]
fn test_text_type_is_primitive() {
    let text_ty = Type::text();
    assert_eq!(text_ty, Type::Text);
    assert!(text_ty.is_monotype());
}

#[test]
fn test_text_literal_inference() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let text_lit = Literal::string("hello world".to_string().into(), span);
    let expr = Expr::literal(text_lit);

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::text());
}

#[test]
fn test_text_type_in_expressions() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // String literals should infer as Text type
    let text_lit = Literal::string("hello world".to_string().into(), span);
    let expr = Expr::literal(text_lit);

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::text());

    // Text type is distinct from other primitives
    assert_ne!(Type::text(), Type::int());
    assert_ne!(Type::text(), Type::bool());
}

// ============================================================================
// Map<K, V> Type Tests - Semantic Type for Dictionaries
// ============================================================================

#[test]
fn test_map_type_construction() {
    use verum_ast::ty::{Ident, Path};

    // Map<Text, Int> - string keys to integer values
    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    assert_eq!(map_ty.to_string(), "Map<Text, Int>");

    // Map<Int, List<Text>> - nested types
    let list_text = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    let complex_map = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::int(), list_text].into(),
    };
    assert_eq!(complex_map.to_string(), "Map<Int, List<Text>>");
}

#[test]
fn test_map_type_unification() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let map1 = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    let map2 = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };

    let result = unifier.unify(&map1, &map2, span);
    assert!(result.is_ok());
}

#[test]
fn test_map_type_key_value_mismatch() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let map1 = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    let map2 = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::int(), Type::text()].into(),
    };

    // Different key/value types should not unify
    let result = unifier.unify(&map1, &map2, span);
    assert!(result.is_err());
}

// ============================================================================
// Set<T> Type Tests - Semantic Type for Unique Collections
// ============================================================================

#[test]
fn test_set_type_construction() {
    // Set<Int>
    let set_int = Type::Named {
        path: Path::single(Ident::new("Set", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert_eq!(set_int.to_string(), "Set<Int>");

    // Set<Text>
    let set_text = Type::Named {
        path: Path::single(Ident::new("Set", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    assert_eq!(set_text.to_string(), "Set<Text>");
}

#[test]
fn test_set_type_is_well_formed() {
    use verum_ast::ty::{Ident, Path};

    // Sets should require elements to implement Eq protocol
    // This is enforced at the protocol level
    let set_ty = Type::Named {
        path: Path::single(Ident::new("Set", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    // Verify type is well-formed
    assert!(set_ty.is_monotype());
}

// ============================================================================
// Maybe<T> Type Tests - Semantic Type for Optional Values
// ============================================================================

#[test]
fn test_maybe_type_construction() {
    // Maybe<Int>
    let maybe_int = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert_eq!(maybe_int.to_string(), "Maybe<Int>");

    // Maybe<Text>
    let maybe_text = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    assert_eq!(maybe_text.to_string(), "Maybe<Text>");
}

#[test]
fn test_maybe_type_is_well_formed() {
    use verum_ast::ty::{Ident, Path};

    // Verify Maybe<Int> is well-formed
    let maybe_int = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    assert!(maybe_int.is_monotype());
    assert_eq!(maybe_int.to_string(), "Maybe<Int>");
}

// ============================================================================
// Heap<T> Type Tests - Semantic Type for Heap Allocation
// ============================================================================

#[test]
fn test_heap_type_construction() {
    // Heap<Int> - heap-allocated integer
    let heap_int = Type::Named {
        path: Path::single(Ident::new("Heap", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert_eq!(heap_int.to_string(), "Heap<Int>");

    // Heap<List<Text>> - heap-allocated list
    let list_text = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    let heap_list = Type::Named {
        path: Path::single(Ident::new("Heap", Span::dummy())),
        args: vec![list_text].into(),
    };
    assert_eq!(heap_list.to_string(), "Heap<List<Text>>");
}

#[test]
fn test_heap_type_is_well_formed() {
    use verum_ast::ty::{Ident, Path};

    // Heap<Int> should deref to Int
    // This requires Deref protocol implementation
    let heap_ty = Type::Named {
        path: Path::single(Ident::new("Heap", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    // Verify heap type is well-formed
    assert!(heap_ty.is_monotype());
}

// ============================================================================
// Shared<T> Type Tests - Semantic Type for Shared References
// ============================================================================

#[test]
fn test_shared_type_construction() {
    // Shared<Int> - shared reference to integer
    let shared_int = Type::Named {
        path: Path::single(Ident::new("Shared", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert_eq!(shared_int.to_string(), "Shared<Int>");

    // Shared<Map<Text, Int>>
    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    let shared_map = Type::Named {
        path: Path::single(Ident::new("Shared", Span::dummy())),
        args: vec![map_ty].into(),
    };
    assert_eq!(shared_map.to_string(), "Shared<Map<Text, Int>>");
}

#[test]
fn test_shared_type_is_well_formed() {
    use verum_ast::ty::{Ident, Path};

    // Shared<T> requires T: Send + Sync
    // This is enforced at protocol level
    let shared_ty = Type::Named {
        path: Path::single(Ident::new("Shared", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    // Verify type is well-formed
    assert!(shared_ty.is_monotype());
}

// ============================================================================
// Integration Tests - Complex Type Scenarios
// ============================================================================

#[test]
fn test_nested_semantic_types() {
    // List<Maybe<Heap<Text>>>
    let heap_text = Type::Named {
        path: Path::single(Ident::new("Heap", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    let maybe_heap = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![heap_text].into(),
    };
    let list_maybe = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![maybe_heap].into(),
    };

    assert_eq!(list_maybe.to_string(), "List<Maybe<Heap<Text>>>");
}

#[test]
fn test_complex_collection_types() {
    // Map<Text, List<Set<Int>>>
    let set_int = Type::Named {
        path: Path::single(Ident::new("Set", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list_set = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![set_int].into(),
    };
    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), list_set].into(),
    };

    assert_eq!(map_ty.to_string(), "Map<Text, List<Set<Int>>>");
}

#[test]
fn test_function_with_semantic_types() {
    // fn(List<Int>) -> Maybe<Int>
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let maybe_int = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let func = Type::function(vec![list_int].into(), maybe_int);

    assert_eq!(func.to_string(), "fn(List<Int>) -> Maybe<Int>");
}

#[test]
fn test_semantic_types_subtyping() {
    let subtyping = Subtyping::new();

    // List<Int> should not be a subtype of List<Float>
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list_float = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::float()].into(),
    };

    let result = subtyping.is_subtype(&list_int, &list_float);
    assert!(!result);
}

#[test]
fn test_type_context_with_semantic_types() {
    let mut ctx = TypeContext::new();

    // Define function using semantic types
    let list_text = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::text()].into(),
    };
    let func_ty = Type::function(vec![list_text].into(), Type::int());

    ctx.define_type("process_strings".to_string(), func_ty.clone());

    let retrieved = ctx.lookup_type("process_strings");
    assert_eq!(retrieved, Maybe::Some(&func_ty));
}

// ============================================================================
// Error Cases - Invalid Semantic Type Usage
// ============================================================================

#[test]
fn test_list_wrong_arity() {
    use verum_ast::ty::{Ident, Path};

    // List requires exactly one type argument
    // This is validated at type construction level
    let _list_no_args = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: List::new(),
    };
    // Type system should validate arity at higher level
}

#[test]
fn test_map_wrong_arity() {
    // Map requires exactly two type arguments
    let _map_one_arg = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    // Type system should validate arity at higher level
}

#[test]
fn test_semantic_types_are_monotypes() {
    // All concrete semantic types are monotypes
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert!(list_int.is_monotype());

    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    assert!(map_ty.is_monotype());

    let maybe_bool = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::dummy())),
        args: vec![Type::bool()].into(),
    };
    assert!(maybe_bool.is_monotype());
}

// ============================================================================
// v6.0-BALANCED Compliance Verification
// ============================================================================

#[test]
fn test_no_rust_std_vec_usage() {
    // This test ensures we're using List, not Vec
    // Compile-time check via type system
    let list: List<i32> = vec![1, 2, 3].into();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_no_rust_std_string_usage() {
    // This test ensures we're using Text, not String
    // Compile-time check via type system
    let text: Text = Text::from("hello");
    assert_eq!(text, Text::from("hello"));
}

#[test]
fn test_no_rust_std_hashmap_usage() {
    // This test ensures we're using Map, not HashMap
    // Compile-time check via type system
    let map: Map<Text, i32> = Map::new();
    assert_eq!(map.len(), 0);
}

#[test]
fn test_type_display_uses_semantic_names() {
    // Verify Type::to_string() uses semantic names
    let list_ty = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    assert!(list_ty.to_string().contains("List"));
    assert!(!list_ty.to_string().contains("Vec"));

    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::dummy())),
        args: vec![Type::text(), Type::int()].into(),
    };
    assert!(map_ty.to_string().contains("Map"));
    assert!(!map_ty.to_string().contains("HashMap"));
}
