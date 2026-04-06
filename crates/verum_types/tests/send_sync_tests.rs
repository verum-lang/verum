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
// Comprehensive Test Suite for Send and Sync Thread-Safety Protocols
//
// Basic protocols with simple associated types (initial release) — 4 - Thread-Safety Protocols
//
// This test suite validates the Send and Sync marker protocol implementation,
// ensuring thread-safety guarantees are correctly enforced at compile-time.
//
// Test Coverage:
// - Primitives are Send + Sync
// - Containers derive Send/Sync from elements
// - References require Sync for Send
// - Unsafe references bypass checks
// - Functions are !Send + !Sync
// - Shared<T> requires T: Send + Sync
// - Mutex<T> requires T: Send
// - Futures are Send but !Sync
// - Negative cases (compilation failures)

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{List, Maybe, Text};
use verum_types::refinement::RefinementPredicate;
use verum_types::{ProtocolChecker, SendSyncDerivation, Type, TypeVar};

// ==================== Helper Functions ====================

fn create_checker() -> ProtocolChecker {
    ProtocolChecker::new()
}

fn create_derivation(checker: &ProtocolChecker) -> SendSyncDerivation<'_> {
    SendSyncDerivation::new(checker)
}

// ==================== Primitive Types ====================

#[test]
fn test_primitives_are_send() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // All primitive types should be Send
    assert!(derivation.is_send(&Type::Unit), "Unit should be Send");
    assert!(derivation.is_send(&Type::Bool), "Bool should be Send");
    assert!(derivation.is_send(&Type::Int), "Int should be Send");
    assert!(derivation.is_send(&Type::Float), "Float should be Send");
    assert!(derivation.is_send(&Type::Char), "Char should be Send");
    assert!(derivation.is_send(&Type::Text), "Text should be Send");
}

#[test]
fn test_primitives_are_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // All primitive types should be Sync
    assert!(derivation.is_sync(&Type::Unit), "Unit should be Sync");
    assert!(derivation.is_sync(&Type::Bool), "Bool should be Sync");
    assert!(derivation.is_sync(&Type::Int), "Int should be Sync");
    assert!(derivation.is_sync(&Type::Float), "Float should be Sync");
    assert!(derivation.is_sync(&Type::Char), "Char should be Sync");
    assert!(derivation.is_sync(&Type::Text), "Text should be Sync");
}

#[test]
fn test_primitive_send_derivation() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Derive Send for Int
    let send_impl = derivation.derive_send(&Type::Int);
    assert!(send_impl.is_some(), "Should be able to derive Send for Int");

    if let Maybe::Some(impl_) = send_impl {
        assert_eq!(impl_.for_type, Type::Int);
        assert!(impl_.methods.is_empty(), "Marker protocol has no methods");
    }
}

#[test]
fn test_primitive_sync_derivation() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Derive Sync for Int
    let sync_impl = derivation.derive_sync(&Type::Int);
    assert!(sync_impl.is_some(), "Should be able to derive Sync for Int");

    if let Maybe::Some(impl_) = sync_impl {
        assert_eq!(impl_.for_type, Type::Int);
        assert!(impl_.methods.is_empty(), "Marker protocol has no methods");
    }
}

// ==================== Compound Types ====================

#[test]
fn test_tuple_send_sync_derivation() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // (Int, Bool) should be Send + Sync
    let tuple = Type::Tuple(vec![Type::Int, Type::Bool].into());
    assert!(derivation.is_send(&tuple), "(Int, Bool) should be Send");
    assert!(derivation.is_sync(&tuple), "(Int, Bool) should be Sync");

    // Nested tuples
    let nested =
        Type::Tuple(vec![Type::Tuple(vec![Type::Int, Type::Int].into()), Type::Bool].into());
    assert!(derivation.is_send(&nested), "Nested tuple should be Send");
    assert!(derivation.is_sync(&nested), "Nested tuple should be Sync");

    // Empty tuple
    let empty = Type::Tuple(vec![].into());
    assert!(derivation.is_send(&empty), "() should be Send");
    assert!(derivation.is_sync(&empty), "() should be Sync");
}

#[test]
fn test_array_send_sync_derivation() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // [Int; 10] should be Send + Sync
    let array = Type::Array {
        element: Box::new(Type::Int),
        size: Some(10),
    };
    assert!(derivation.is_send(&array), "[Int; 10] should be Send");
    assert!(derivation.is_sync(&array), "[Int; 10] should be Sync");

    // Nested arrays
    let nested_array = Type::Array {
        element: Box::new(Type::Array {
            element: Box::new(Type::Bool),
            size: Some(5),
        }),
        size: Some(3),
    };
    assert!(
        derivation.is_send(&nested_array),
        "[[Bool; 5]; 3] should be Send"
    );
    assert!(
        derivation.is_sync(&nested_array),
        "[[Bool; 5]; 3] should be Sync"
    );
}

#[test]
fn test_record_send_sync_derivation() {
    use indexmap::IndexMap;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // { x: Int, y: Bool } should be Send + Sync
    let mut fields = IndexMap::new();
    fields.insert(Text::from("x"), Type::Int);
    fields.insert(Text::from("y"), Type::Bool);
    let record = Type::Record(fields);

    assert!(
        derivation.is_send(&record),
        "Record with Send fields should be Send"
    );
    assert!(
        derivation.is_sync(&record),
        "Record with Sync fields should be Sync"
    );

    // Empty record
    let empty_record = Type::Record(IndexMap::new());
    assert!(
        derivation.is_send(&empty_record),
        "Empty record should be Send"
    );
    assert!(
        derivation.is_sync(&empty_record),
        "Empty record should be Sync"
    );
}

#[test]
fn test_variant_send_sync_derivation() {
    use indexmap::IndexMap;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Some(Int) | None should be Send + Sync
    let mut variants = IndexMap::new();
    variants.insert(Text::from("Some"), Type::Int);
    variants.insert(Text::from("None"), Type::Unit);
    let variant = Type::Variant(variants);

    assert!(
        derivation.is_send(&variant),
        "Variant with Send types should be Send"
    );
    assert!(
        derivation.is_sync(&variant),
        "Variant with Sync types should be Sync"
    );
}

// ==================== References ====================

#[test]
fn test_reference_send_requires_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // &Int is Send because Int is Sync
    let ref_int = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert!(
        derivation.is_send(&ref_int),
        "&Int should be Send (Int: Sync)"
    );
    assert!(derivation.is_sync(&ref_int), "&Int should be Sync");

    // &mut Int is Send because Int is Sync
    let mut_ref_int = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Int),
    };
    assert!(
        derivation.is_send(&mut_ref_int),
        "&mut Int should be Send (Int: Sync)"
    );
    // Note: &mut T is NOT Sync (mutable aliasing not allowed)
}

#[test]
fn test_checked_reference_send_requires_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // &checked Int is Send because Int is Sync
    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert!(
        derivation.is_send(&checked_ref),
        "&checked Int should be Send (Int: Sync)"
    );
    assert!(
        derivation.is_sync(&checked_ref),
        "&checked Int should be Sync"
    );
}

#[test]
fn test_unsafe_reference_is_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // &unsafe T is assumed Send/Sync (user responsibility)
    let unsafe_ref = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert!(
        derivation.is_send(&unsafe_ref),
        "&unsafe Int should be Send (unchecked)"
    );
    assert!(
        derivation.is_sync(&unsafe_ref),
        "&unsafe Int should be Sync (unchecked)"
    );

    // Even for functions (normally !Send + !Sync)
    let unsafe_fn_ref = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::Function {
            params: vec![Type::Int].into(),
            return_type: Box::new(Type::Bool),
            contexts: None,
            type_params: vec![].into(),
            properties: None,
        }),
    };
    assert!(
        derivation.is_send(&unsafe_fn_ref),
        "&unsafe Fn should be Send (unchecked)"
    );
    assert!(
        derivation.is_sync(&unsafe_fn_ref),
        "&unsafe Fn should be Sync (unchecked)"
    );
}

// ==================== Pointers ====================

#[test]
fn test_raw_pointers_not_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // *const Int is NOT Send
    let const_ptr = Type::Pointer {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert!(
        !derivation.is_send(&const_ptr),
        "*const Int should NOT be Send"
    );
    assert!(
        !derivation.is_sync(&const_ptr),
        "*const Int should NOT be Sync"
    );

    // *mut Int is NOT Send
    let mut_ptr = Type::Pointer {
        mutable: true,
        inner: Box::new(Type::Int),
    };
    assert!(!derivation.is_send(&mut_ptr), "*mut Int should NOT be Send");
    assert!(!derivation.is_sync(&mut_ptr), "*mut Int should NOT be Sync");
}

// ==================== Functions ====================

#[test]
fn test_functions_not_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // fn(Int) -> Bool is NOT Send + NOT Sync
    let func = Type::Function {
        params: vec![Type::Int].into(),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };
    assert!(
        !derivation.is_send(&func),
        "Function types should NOT be Send"
    );
    assert!(
        !derivation.is_sync(&func),
        "Function types should NOT be Sync"
    );

    // fn() -> () is NOT Send + NOT Sync
    let nullary_func = Type::Function {
        params: List::new(),
        return_type: Box::new(Type::Unit),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };
    assert!(
        !derivation.is_send(&nullary_func),
        "Nullary function should NOT be Send"
    );
    assert!(
        !derivation.is_sync(&nullary_func),
        "Nullary function should NOT be Sync"
    );
}

// ==================== Refinement Types ====================

#[test]
fn test_refined_types_inherit_send_sync() {
    use verum_ast::ty::Path;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    let span = Span::default();

    // Int{x > 0} should be Send + Sync (inherits from Int)
    let positive_path = Path::single(Ident::new("Positive", span));
    let positive = Type::Refined {
        base: Box::new(Type::Int),
        predicate: RefinementPredicate::named(positive_path, span),
    };
    assert!(derivation.is_send(&positive), "Refined Int should be Send");
    assert!(derivation.is_sync(&positive), "Refined Int should be Sync");

    // Refined tuple
    let valid_path = Path::single(Ident::new("Valid", span));
    let refined_tuple = Type::Refined {
        base: Box::new(Type::Tuple(vec![Type::Int, Type::Bool].into())),
        predicate: RefinementPredicate::named(valid_path, span),
    };
    assert!(
        derivation.is_send(&refined_tuple),
        "Refined tuple should be Send"
    );
    assert!(
        derivation.is_sync(&refined_tuple),
        "Refined tuple should be Sync"
    );
}

// ==================== Type Variables ====================

#[test]
fn test_type_vars_assumed_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Type variables are assumed Send/Sync (checked at instantiation)
    let tvar = Type::Var(TypeVar::with_id(0));
    assert!(
        derivation.is_send(&tvar),
        "Type variables should be assumed Send"
    );
    assert!(
        derivation.is_sync(&tvar),
        "Type variables should be assumed Sync"
    );
}

// ==================== Futures and Generators ====================

#[test]
fn test_future_send_not_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Future<Int> is Send (Int: Send) but NOT Sync (interior mutability)
    let future = Type::Future {
        output: Box::new(Type::Int),
    };
    assert!(
        derivation.is_send(&future),
        "Future<Int> should be Send (Int: Send)"
    );
    assert!(
        !derivation.is_sync(&future),
        "Future should NOT be Sync (interior mutability)"
    );

    // Future<()> is Send but NOT Sync
    let future_unit = Type::Future {
        output: Box::new(Type::Unit),
    };
    assert!(
        derivation.is_send(&future_unit),
        "Future<()> should be Send"
    );
    assert!(
        !derivation.is_sync(&future_unit),
        "Future<()> should NOT be Sync"
    );
}

#[test]
fn test_generator_send_not_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Generator<Int, Bool> is Send if Int: Send and Bool: Send
    let generator = Type::Generator {
        yield_ty: Box::new(Type::Int),
        return_ty: Box::new(Type::Bool),
    };
    assert!(
        derivation.is_send(&generator),
        "Generator<Int, Bool> should be Send"
    );
    assert!(
        !derivation.is_sync(&generator),
        "Generator should NOT be Sync (interior mutability)"
    );
}

// ==================== Meta Parameters ====================

#[test]
fn test_meta_parameters_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Meta parameters are compile-time only, so Send + Sync
    let meta = Type::Meta {
        name: Text::from("N"),
        ty: Box::new(Type::Int),
        refinement: None,
    };
    assert!(
        derivation.is_send(&meta),
        "Meta parameters should be Send (compile-time)"
    );
    assert!(
        derivation.is_sync(&meta),
        "Meta parameters should be Sync (compile-time)"
    );
}

// ==================== Tensors ====================

#[test]
fn test_tensor_send_sync_derivation() {
    use verum_common::ConstValue;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Tensor<f32, [4]> should be Send + Sync if f32: Send + Sync
    let tensor = Type::Tensor {
        element: Box::new(Type::Float),
        shape: vec![ConstValue::Int(4)].into(),
        strides: vec![1].into(),
        span: Span::default(),
    };
    assert!(
        derivation.is_send(&tensor),
        "Tensor<Float, [4]> should be Send"
    );
    assert!(
        derivation.is_sync(&tensor),
        "Tensor<Float, [4]> should be Sync"
    );

    // 2D tensor
    let tensor_2d = Type::Tensor {
        element: Box::new(Type::Int),
        shape: vec![ConstValue::Int(2), ConstValue::Int(3)].into(),
        strides: vec![3, 1].into(),
        span: Span::default(),
    };
    assert!(
        derivation.is_send(&tensor_2d),
        "Tensor<Int, [2, 3]> should be Send"
    );
    assert!(
        derivation.is_sync(&tensor_2d),
        "Tensor<Int, [2, 3]> should be Sync"
    );
}

// ==================== Existential and Universal Types ====================

#[test]
fn test_existential_types_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // ∃α. α should inherit Send/Sync from body
    let exists = Type::Exists {
        var: TypeVar::with_id(0),
        body: Box::new(Type::Int),
    };
    assert!(derivation.is_send(&exists), "∃α. Int should be Send");
    assert!(derivation.is_sync(&exists), "∃α. Int should be Sync");
}

#[test]
fn test_universal_types_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // ∀α. α should inherit Send/Sync from body
    let forall = Type::Forall {
        vars: vec![TypeVar::with_id(0)].into(),
        body: Box::new(Type::Bool),
    };
    assert!(derivation.is_send(&forall), "∀α. Bool should be Send");
    assert!(derivation.is_sync(&forall), "∀α. Bool should be Sync");
}

// ==================== Standard Library Types ====================

#[test]
fn test_list_send_sync_registered() {
    let checker = create_checker();

    // List<Int> should implement Send + Sync
    let list_ty = Type::Named {
        path: Path::single(Ident::new("List", Span::default())),
        args: vec![].into(),
    };

    let send_path = Path::single(Ident::new("Send", Span::default()));
    let sync_path = Path::single(Ident::new("Sync", Span::default()));

    assert!(
        checker.implements(&list_ty, &send_path),
        "List should implement Send"
    );
    assert!(
        checker.implements(&list_ty, &sync_path),
        "List should implement Sync"
    );
}

#[test]
fn test_map_send_sync_registered() {
    let checker = create_checker();

    let map_ty = Type::Named {
        path: Path::single(Ident::new("Map", Span::default())),
        args: vec![].into(),
    };

    let send_path = Path::single(Ident::new("Send", Span::default()));
    let sync_path = Path::single(Ident::new("Sync", Span::default()));

    assert!(
        checker.implements(&map_ty, &send_path),
        "Map should implement Send"
    );
    assert!(
        checker.implements(&map_ty, &sync_path),
        "Map should implement Sync"
    );
}

#[test]
fn test_maybe_send_sync_registered() {
    let checker = create_checker();

    let maybe_ty = Type::Named {
        path: Path::single(Ident::new("Maybe", Span::default())),
        args: vec![].into(),
    };

    let send_path = Path::single(Ident::new("Send", Span::default()));
    let sync_path = Path::single(Ident::new("Sync", Span::default()));

    assert!(
        checker.implements(&maybe_ty, &send_path),
        "Maybe should implement Send"
    );
    assert!(
        checker.implements(&maybe_ty, &sync_path),
        "Maybe should implement Sync"
    );
}

// ==================== Synchronization Primitives ====================

#[test]
fn test_shared_send_sync_registered() {
    let checker = create_checker();

    // Shared<T> where T: Send + Sync
    let shared_ty = Type::Named {
        path: Path::single(Ident::new("Shared", Span::default())),
        args: vec![Type::Var(TypeVar::with_id(0))].into(),
    };

    let send_path = Path::single(Ident::new("Send", Span::default()));
    let sync_path = Path::single(Ident::new("Sync", Span::default()));

    // Note: The actual Send/Sync implementation depends on where clauses
    // being satisfied, but the implementation should be registered
    assert!(
        checker.find_impl(&shared_ty, &send_path).is_some(),
        "Shared<T> should have Send implementation"
    );
    assert!(
        checker.find_impl(&shared_ty, &sync_path).is_some(),
        "Shared<T> should have Sync implementation"
    );
}

#[test]
fn test_mutex_send_sync_registered() {
    let checker = create_checker();

    // Mutex<T> where T: Send
    let mutex_ty = Type::Named {
        path: Path::single(Ident::new("Mutex", Span::default())),
        args: vec![Type::Var(TypeVar::with_id(0))].into(),
    };

    let send_path = Path::single(Ident::new("Send", Span::default()));
    let sync_path = Path::single(Ident::new("Sync", Span::default()));

    assert!(
        checker.find_impl(&mutex_ty, &send_path).is_some(),
        "Mutex<T> should have Send implementation"
    );
    assert!(
        checker.find_impl(&mutex_ty, &sync_path).is_some(),
        "Mutex<T> should have Sync implementation"
    );
}

// ==================== Protocol Registration ====================

#[test]
fn test_send_protocol_registered() {
    let checker = create_checker();
    let send_protocol = checker.get_protocol(&"Send".into());

    assert!(
        send_protocol.is_some(),
        "Send protocol should be registered"
    );

    if let Maybe::Some(proto) = send_protocol {
        assert_eq!(proto.name.as_str(), "Send");
        assert!(
            proto.methods.is_empty(),
            "Send is a marker protocol (no methods)"
        );
        assert!(
            proto.associated_types.is_empty(),
            "Send has no associated types"
        );
        assert!(
            proto.super_protocols.is_empty(),
            "Send has no superprotocols"
        );
    }
}

#[test]
fn test_sync_protocol_registered() {
    let checker = create_checker();
    let sync_protocol = checker.get_protocol(&"Sync".into());

    assert!(
        sync_protocol.is_some(),
        "Sync protocol should be registered"
    );

    if let Maybe::Some(proto) = sync_protocol {
        assert_eq!(proto.name.as_str(), "Sync");
        assert!(
            proto.methods.is_empty(),
            "Sync is a marker protocol (no methods)"
        );
        assert!(
            proto.associated_types.is_empty(),
            "Sync has no associated types"
        );
        assert!(
            proto.super_protocols.is_empty(),
            "Sync has no superprotocols"
        );
    }
}

// ==================== Complex Scenarios ====================

#[test]
fn test_nested_containers_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Vec<Vec<Int>> should be Send + Sync
    let nested_tuple = Type::Tuple(
        vec![
            Type::Tuple(vec![Type::Int, Type::Bool].into()),
            Type::Tuple(vec![Type::Float, Type::Char].into()),
        ]
        .into(),
    );

    assert!(
        derivation.is_send(&nested_tuple),
        "Nested containers should be Send"
    );
    assert!(
        derivation.is_sync(&nested_tuple),
        "Nested containers should be Sync"
    );
}

#[test]
fn test_mixed_send_not_sync_container() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // (Int, Future<Bool>) should be Send but NOT Sync
    let mixed = Type::Tuple(
        vec![
            Type::Int,
            Type::Future {
                output: Box::new(Type::Bool),
            },
        ]
        .into(),
    );

    assert!(
        derivation.is_send(&mixed),
        "Container with Send types should be Send"
    );
    assert!(
        !derivation.is_sync(&mixed),
        "Container with !Sync type should NOT be Sync"
    );
}

#[test]
fn test_not_send_container() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // (Int, fn() -> Bool) should NOT be Send
    let with_fn = Type::Tuple(
        vec![
            Type::Int,
            Type::Function {
                params: vec![].into(),
                return_type: Box::new(Type::Bool),
                contexts: None,
                type_params: vec![].into(),
                properties: None,
            },
        ]
        .into(),
    );

    assert!(
        !derivation.is_send(&with_fn),
        "Container with !Send type should NOT be Send"
    );
    assert!(
        !derivation.is_sync(&with_fn),
        "Container with !Sync type should NOT be Sync"
    );
}

// ==================== Edge Cases ====================

#[test]
fn test_ownership_references_send_sync() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // %Int should be Send + Sync (ownership transfer)
    let ownership = Type::Ownership {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert!(derivation.is_send(&ownership), "%Int should be Send");
    assert!(derivation.is_sync(&ownership), "%Int should be Sync");
}

#[test]
fn test_deeply_nested_types() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Build: ((((Int))))
    let mut nested = Type::Int;
    for _ in 0..10 {
        nested = Type::Tuple(vec![nested].into());
    }

    assert!(
        derivation.is_send(&nested),
        "Deeply nested Send type should be Send"
    );
    assert!(
        derivation.is_sync(&nested),
        "Deeply nested Sync type should be Sync"
    );
}

#[test]
fn test_empty_containers_send_sync() {
    use indexmap::IndexMap;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Empty tuple
    assert!(derivation.is_send(&Type::Tuple(vec![].into())));
    assert!(derivation.is_sync(&Type::Tuple(vec![].into())));

    // Empty record
    assert!(derivation.is_send(&Type::Record(IndexMap::new())));
    assert!(derivation.is_sync(&Type::Record(IndexMap::new())));

    // Empty variant (should be Send + Sync)
    assert!(derivation.is_send(&Type::Variant(IndexMap::new())));
    assert!(derivation.is_sync(&Type::Variant(IndexMap::new())));
}

// ==================== Regression Tests ====================

#[test]
fn test_regression_reference_to_future() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // &Future<Int> should be Send (Future<Int>: Sync? No!)
    // Future is !Sync, so &Future should NOT be Send
    let ref_future = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Future {
            output: Box::new(Type::Int),
        }),
    };

    // &Future<Int> is NOT Send because Future<Int> is NOT Sync
    assert!(
        !derivation.is_send(&ref_future),
        "&Future should NOT be Send (Future: !Sync)"
    );
}

#[test]
fn test_regression_refined_reference() {
    use verum_ast::ty::Path;

    let checker = create_checker();
    let derivation = create_derivation(&checker);

    let span = Span::default();
    let positive_path = Path::single(Ident::new("Positive", span));

    // &(Int{x > 0}) should be Send (refined Int is Sync)
    let refined_ref = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Refined {
            base: Box::new(Type::Int),
            predicate: RefinementPredicate::named(positive_path, span),
        }),
    };

    assert!(derivation.is_send(&refined_ref), "&Positive should be Send");
    assert!(derivation.is_sync(&refined_ref), "&Positive should be Sync");
}

// ==================== Performance Tests ====================

#[test]
fn test_large_tuple_performance() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Create tuple with 100 elements
    let large_tuple = Type::Tuple(vec![Type::Int; 100].into());

    let start = std::time::Instant::now();
    let is_send = derivation.is_send(&large_tuple);
    let duration = start.elapsed();

    assert!(is_send, "Large tuple should be Send");
    assert!(duration.as_millis() < 100, "Should check Send in < 100ms");
}

#[test]
fn test_deeply_nested_performance() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    // Create 50 levels of nesting
    let mut nested = Type::Int;
    for _ in 0..50 {
        nested = Type::Tuple(vec![nested].into());
    }

    let start = std::time::Instant::now();
    let is_send = derivation.is_send(&nested);
    let duration = start.elapsed();

    assert!(is_send, "Deeply nested type should be Send");
    assert!(duration.as_millis() < 100, "Should check Send in < 100ms");
}

// ==================== Summary Test ====================

#[test]
fn test_comprehensive_coverage() {
    let checker = create_checker();
    let derivation = create_derivation(&checker);

    let test_cases = vec![
        // (Type, Expected Send, Expected Sync, Description)
        (Type::Int, true, true, "Int"),
        (Type::Bool, true, true, "Bool"),
        (Type::Unit, true, true, "Unit"),
        (Type::Float, true, true, "Float"),
        (
            Type::Tuple(vec![Type::Int, Type::Bool].into()),
            true,
            true,
            "Tuple(Int, Bool)",
        ),
        (
            Type::Function {
                params: vec![Type::Int].into(),
                return_type: Box::new(Type::Bool),
                contexts: None,
                type_params: List::new(),
                properties: None,
            },
            false,
            false,
            "Function",
        ),
        (
            Type::Future {
                output: Box::new(Type::Int),
            },
            true,
            false,
            "Future<Int>",
        ),
        (
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Int),
            },
            true,
            true,
            "&Int",
        ),
        (
            Type::Pointer {
                mutable: false,
                inner: Box::new(Type::Int),
            },
            false,
            false,
            "*const Int",
        ),
    ];

    for (ty, expected_send, expected_sync, desc) in test_cases {
        assert_eq!(
            derivation.is_send(&ty),
            expected_send,
            "{} Send mismatch",
            desc
        );
        assert_eq!(
            derivation.is_sync(&ty),
            expected_sync,
            "{} Sync mismatch",
            desc
        );
    }
}
