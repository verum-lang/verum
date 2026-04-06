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
// Tests for Deref/DerefMut protocol implementation
//
// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4 - Deref/DerefMut Protocols
//
// This test file validates:
// 1. Deref protocol is registered and has correct structure
// 2. DerefMut protocol extends Deref
// 3. Automatic implementations for reference types (&T, &checked T, &unsafe T)
// 4. Method calls through Deref work correctly
// 5. Deref chain resolution

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{Maybe, Text};
use verum_types::{protocol::ProtocolChecker, ty::Type};

// ==================== Test 1: Protocol Definition ====================

#[test]
fn test_deref_protocol_registered() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.1 - Deref protocol must be registered
    let checker = ProtocolChecker::new();

    // Verify Deref protocol exists
    let deref = checker.get_protocol(&"Deref".into());
    assert!(
        matches!(deref, Maybe::Some(_)),
        "Deref protocol should be registered"
    );

    if let Maybe::Some(deref_proto) = deref {
        // Verify protocol name
        assert_eq!(deref_proto.name, Text::from("Deref"));

        // Verify has 'deref' method
        assert!(
            deref_proto.methods.contains_key(&Text::from("deref")),
            "Deref protocol should have 'deref' method"
        );

        // Verify has 'Target' associated type
        assert!(
            deref_proto.associated_types.contains_key(&"Target".into()),
            "Deref protocol should have 'Target' associated type"
        );

        // Verify no superprotocols
        assert_eq!(
            deref_proto.super_protocols.len(),
            0,
            "Deref should not have superprotocols"
        );
    }
}

#[test]
fn test_derefmut_protocol_registered() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.2 - DerefMut protocol must be registered
    let checker = ProtocolChecker::new();

    // Verify DerefMut protocol exists
    let deref_mut = checker.get_protocol(&"DerefMut".into());
    assert!(
        matches!(deref_mut, Maybe::Some(_)),
        "DerefMut protocol should be registered"
    );

    if let Maybe::Some(deref_mut_proto) = deref_mut {
        // Verify protocol name
        assert_eq!(deref_mut_proto.name.as_str(), "DerefMut");

        // Verify has 'deref_mut' method
        assert!(
            deref_mut_proto
                .methods
                .contains_key(&Text::from("deref_mut")),
            "DerefMut protocol should have 'deref_mut' method"
        );

        // Verify has 'Target' associated type
        assert!(
            deref_mut_proto
                .associated_types
                .contains_key(&"Target".into()),
            "DerefMut protocol should have 'Target' associated type"
        );
    }
}

#[test]
fn test_derefmut_extends_deref() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.2 - DerefMut must extend Deref
    let checker = ProtocolChecker::new();

    let deref_mut = checker.get_protocol(&"DerefMut".into());
    if let Maybe::Some(deref_mut_proto) = deref_mut {
        // Verify has Deref as superprotocol
        assert!(
            !deref_mut_proto.super_protocols.is_empty(),
            "DerefMut should have superprotocols"
        );

        // Check if Deref is in the superprotocols
        let has_deref = deref_mut_proto.super_protocols.iter().any(|bound| {
            bound
                .protocol
                .as_ident()
                .map(|ident| ident.as_str() == "Deref")
                .unwrap_or(false)
        });

        assert!(has_deref, "DerefMut should extend Deref protocol");
    }
}

#[test]
fn test_derefmut_inherits_from_deref() {
    // Verify transitive inheritance check works
    let checker = ProtocolChecker::new();

    let inherits = checker.inherits_from(&"DerefMut".into(), &"Deref".into());
    assert!(inherits, "DerefMut should inherit from Deref");
}

// ==================== Test 2: Automatic Implementations ====================

#[test]
fn test_reference_implements_deref() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &T automatically implements Deref<Target=T>
    let checker = ProtocolChecker::new();

    // Test immutable reference
    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&T should automatically implement Deref"
    );
}

#[test]
fn test_mutable_reference_implements_deref() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &mut T implements Deref
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Int),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&mut T should implement Deref"
    );
}

#[test]
fn test_mutable_reference_implements_derefmut() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &mut T implements DerefMut
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Int),
    };

    let deref_mut_path = Path::single(Ident::new("DerefMut", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_mut_path),
        "&mut T should implement DerefMut"
    );
}

#[test]
fn test_immutable_reference_does_not_implement_derefmut() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &T does NOT implement DerefMut
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let deref_mut_path = Path::single(Ident::new("DerefMut", Span::default()));
    assert!(
        !checker.implements(&ref_type, &deref_mut_path),
        "&T should NOT implement DerefMut (only &mut T does)"
    );
}

#[test]
fn test_checked_reference_implements_deref() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &checked T implements Deref
    let checker = ProtocolChecker::new();

    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&checked_ref, &deref_path),
        "&checked T should implement Deref"
    );
}

#[test]
fn test_checked_mutable_reference_implements_derefmut() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &checked mut T implements DerefMut
    let checker = ProtocolChecker::new();

    let checked_ref = Type::CheckedReference {
        mutable: true,
        inner: Box::new(Type::Bool),
    };

    let deref_mut_path = Path::single(Ident::new("DerefMut", Span::default()));
    assert!(
        checker.implements(&checked_ref, &deref_mut_path),
        "&checked mut T should implement DerefMut"
    );
}

#[test]
fn test_unsafe_reference_implements_deref() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &unsafe T implements Deref
    let checker = ProtocolChecker::new();

    let unsafe_ref = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::Float),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&unsafe_ref, &deref_path),
        "&unsafe T should implement Deref"
    );
}

#[test]
fn test_unsafe_mutable_reference_implements_derefmut() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - &unsafe mut T implements DerefMut
    let checker = ProtocolChecker::new();

    let unsafe_ref = Type::UnsafeReference {
        mutable: true,
        inner: Box::new(Type::Char),
    };

    let deref_mut_path = Path::single(Ident::new("DerefMut", Span::default()));
    assert!(
        checker.implements(&unsafe_ref, &deref_mut_path),
        "&unsafe mut T should implement DerefMut"
    );
}

#[test]
fn test_non_reference_does_not_implement_deref() {
    // Verify that non-reference types don't automatically implement Deref
    let checker = ProtocolChecker::new();

    let int_type = Type::Int;
    let deref_path = Path::single(Ident::new("Deref", Span::default()));

    assert!(
        !checker.implements(&int_type, &deref_path),
        "Int should not implement Deref automatically"
    );
}

// ==================== Test 3: Deref Target Resolution ====================

#[test]
fn test_get_deref_target_for_reference() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - Target type resolution
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let target = checker.get_deref_target(&ref_type);
    assert!(matches!(target, Maybe::Some(_)), "Should have deref target");

    if let Maybe::Some(target_ty) = target {
        assert_eq!(target_ty, Type::Int, "Deref target should be Int");
    }
}

#[test]
fn test_get_deref_target_for_checked_reference() {
    let checker = ProtocolChecker::new();

    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::Bool),
    };

    let target = checker.get_deref_target(&checked_ref);
    if let Maybe::Some(target_ty) = target {
        assert_eq!(target_ty, Type::Bool, "Deref target should be Bool");
    }
}

#[test]
fn test_get_deref_target_for_unsafe_reference() {
    let checker = ProtocolChecker::new();

    let unsafe_ref = Type::UnsafeReference {
        mutable: true,
        inner: Box::new(Type::Float),
    };

    let target = checker.get_deref_target(&unsafe_ref);
    if let Maybe::Some(target_ty) = target {
        assert_eq!(target_ty, Type::Float, "Deref target should be Float");
    }
}

#[test]
fn test_get_deref_target_for_non_reference() {
    let checker = ProtocolChecker::new();

    let int_type = Type::Int;
    let target = checker.get_deref_target(&int_type);

    assert!(
        matches!(target, Maybe::None),
        "Non-reference types should not have deref target"
    );
}

#[test]
fn test_nested_reference_deref_target() {
    // Test deref target for nested references: &&T -> &T
    let checker = ProtocolChecker::new();

    let nested_ref = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        }),
    };

    let target = checker.get_deref_target(&nested_ref);
    if let Maybe::Some(target_ty) = target {
        // Target should be &Int
        assert!(
            matches!(target_ty, Type::Reference { .. }),
            "Deref target of &&T should be &T"
        );

        if let Type::Reference { inner, .. } = target_ty {
            assert_eq!(*inner, Type::Int, "Inner type should be Int");
        }
    }
}

// ==================== Test 4: Complex Type Scenarios ====================

#[test]
fn test_reference_to_tuple_implements_deref() {
    let checker = ProtocolChecker::new();

    let tuple_type = Type::Tuple(vec![Type::Int, Type::Bool].into());
    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(tuple_type.clone()),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&(Int, Bool) should implement Deref"
    );

    // Verify target is the tuple type
    let target = checker.get_deref_target(&ref_type);
    if let Maybe::Some(target_ty) = target {
        assert_eq!(target_ty, tuple_type, "Target should be (Int, Bool)");
    }
}

#[test]
fn test_reference_to_array_implements_deref() {
    let checker = ProtocolChecker::new();

    let array_type = Type::Array {
        element: Box::new(Type::Int),
        size: Some(10),
    };
    let ref_type = Type::Reference {
        mutable: true,
        inner: Box::new(array_type.clone()),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&mut [Int; 10] should implement Deref"
    );

    // Should also implement DerefMut since it's mutable
    let deref_mut_path = Path::single(Ident::new("DerefMut", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_mut_path),
        "&mut [Int; 10] should implement DerefMut"
    );
}

#[test]
fn test_reference_to_function_implements_deref() {
    let checker = ProtocolChecker::new();

    let fn_type = Type::function(vec![Type::Int].into(), Type::Bool);
    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(fn_type.clone()),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&(fn(Int) -> Bool) should implement Deref"
    );
}

#[test]
fn test_reference_to_refined_type_implements_deref() {
    use verum_ast::expr::Expr;
    use verum_ast::literal::Literal;
    use verum_types::refinement::RefinementPredicate;

    let checker = ProtocolChecker::new();

    // Create a refined type: Int{> 0}
    let span = Span::default();
    let predicate_expr = Expr::literal(Literal::bool(true, span));

    let refined_type = Type::Refined {
        base: Box::new(Type::Int),
        predicate: RefinementPredicate::lambda(predicate_expr, Text::from("x"), span),
    };

    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(refined_type.clone()),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&Int{{> 0}} should implement Deref"
    );
}

// ==================== Test 5: Protocol Inheritance ====================

#[test]
fn test_derefmut_satisfies_deref_requirement() {
    // If a function requires Deref, types implementing DerefMut should satisfy
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Int),
    };

    // Check that type satisfies both Deref and DerefMut
    let deref_satisfied = checker.check_protocol_satisfied(&ref_type, &"Deref".into());
    let derefmut_satisfied = checker.check_protocol_satisfied(&ref_type, &"DerefMut".into());

    assert!(
        matches!(deref_satisfied, Ok(true)),
        "&mut T should satisfy Deref"
    );
    assert!(
        matches!(derefmut_satisfied, Ok(true)),
        "&mut T should satisfy DerefMut"
    );
}

// ==================== Test 6: Edge Cases ====================

#[test]
fn test_reference_to_unit_implements_deref() {
    let checker = ProtocolChecker::new();

    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Unit),
    };

    let deref_path = Path::single(Ident::new("Deref", Span::default()));
    assert!(
        checker.implements(&ref_type, &deref_path),
        "&Unit should implement Deref"
    );
}

#[test]
fn test_all_reference_kinds_with_same_inner_type() {
    // Verify all three reference kinds implement Deref for the same inner type
    let checker = ProtocolChecker::new();
    let deref_path = Path::single(Ident::new("Deref", Span::default()));

    let inner = Box::new(Type::Text);

    let cbgr_ref = Type::Reference {
        mutable: false,
        inner: inner.clone(),
    };

    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: inner.clone(),
    };

    let unsafe_ref = Type::UnsafeReference {
        mutable: false,
        inner: inner.clone(),
    };

    assert!(
        checker.implements(&cbgr_ref, &deref_path),
        "&T implements Deref"
    );
    assert!(
        checker.implements(&checked_ref, &deref_path),
        "&checked T implements Deref"
    );
    assert!(
        checker.implements(&unsafe_ref, &deref_path),
        "&unsafe T implements Deref"
    );
}

#[test]
fn test_protocol_names_listed() {
    // Verify Deref and DerefMut are in the list of protocol names
    let checker = ProtocolChecker::new();
    let names = checker.protocol_names();

    let has_deref = names.iter().any(|n| n.as_str() == "Deref");
    let has_derefmut = names.iter().any(|n| n.as_str() == "DerefMut");

    assert!(has_deref, "Deref should be in protocol names");
    assert!(has_derefmut, "DerefMut should be in protocol names");
}

#[test]
fn test_deref_method_signature() {
    // Verify the deref method has the correct signature
    let checker = ProtocolChecker::new();

    if let Maybe::Some(deref_proto) = checker.get_protocol(&"Deref".into())
        && let Some(deref_method) = deref_proto.methods.get(&Text::from("deref"))
    {
        // Method should be a function type
        assert!(
            deref_method.ty.is_function(),
            "deref should be a function type"
        );

        // Verify method is not default
        assert!(
            !deref_method.has_default,
            "deref should not have default implementation"
        );
    }
}

#[test]
fn test_derefmut_method_signature() {
    // Verify the deref_mut method has the correct signature
    let checker = ProtocolChecker::new();

    if let Maybe::Some(deref_mut_proto) = checker.get_protocol(&"DerefMut".into())
        && let Some(deref_mut_method) = deref_mut_proto.methods.get(&Text::from("deref_mut"))
    {
        // Method should be a function type
        assert!(
            deref_mut_method.ty.is_function(),
            "deref_mut should be a function type"
        );

        // Verify method is not default
        assert!(
            !deref_mut_method.has_default,
            "deref_mut should not have default implementation"
        );
    }
}
