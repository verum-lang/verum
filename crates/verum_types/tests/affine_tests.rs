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
// Tests for affine type checking
//
// Type system extensions: advanced features beyond core HM inference
//
// These tests verify that affine types are properly tracked and
// usage violations are detected at compile time.
#![allow(unexpected_cfgs)]

use verum_ast::span::Span;
use verum_common::Text;
use verum_types::{Type, TypeError, TypeVar};

/// Test helper to create a dummy span
fn dummy_span() -> Span {
    Span::dummy()
}

#[test]
fn test_affine_type_single_use_ok() {
    // This test is a placeholder for future affine tracking implementation
    // Currently the Type enum doesn't track affine status directly
    // (it's tracked in the type context by type name)

    // Once affine tracking is implemented in the type checker:
    // 1. Declare an affine type
    // 2. Use it once in an expression
    // 3. Verify no error is raised

    // Example (pseudo-code):
    // type affine Handle is { fd: Int }
    // fn use_handle(h: Handle) -> Int { h.fd }
    // let handle = Handle { fd: 42 };
    // let result = use_handle(handle);  // OK - used once
}

#[test]
fn test_affine_type_zero_uses_ok() {
    // Affine types allow zero uses (value dropped with cleanup)
    //
    // Example (pseudo-code):
    // type affine Handle is { fd: Int }
    // let handle = Handle { fd: 42 };
    // // handle dropped here with cleanup - OK
}

#[test]
fn test_type_var_operations() {
    // Test basic type variable operations
    let var1 = TypeVar::fresh();
    let var2 = TypeVar::fresh();

    assert_ne!(var1, var2);
    assert_ne!(var1.id(), var2.id());

    let var3 = TypeVar::with_id(100);
    assert_eq!(var3.id(), 100);
}

#[test]
fn test_type_constructors() {
    // Test basic type construction
    let unit = Type::unit();
    let bool_ty = Type::bool();
    let int_ty = Type::int();
    let float_ty = Type::float();
    let text_ty = Type::text();

    assert!(matches!(unit, Type::Unit));
    assert!(matches!(bool_ty, Type::Bool));
    assert!(matches!(int_ty, Type::Int));
    assert!(matches!(float_ty, Type::Float));
    assert!(matches!(text_ty, Type::Text));
}

#[test]
fn test_function_type() {
    let fn_ty = Type::function(vec![Type::int(), Type::float()].into(), Type::bool());

    assert!(fn_ty.is_function());

    match fn_ty {
        Type::Function {
            params,
            return_type,
            contexts,
            type_params,
            properties,
        } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(params[0], Type::Int));
            assert!(matches!(params[1], Type::Float));
            assert!(matches!(*return_type, Type::Bool));
            assert!(contexts.is_none());
            assert_eq!(type_params.len(), 0);
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_reference_types() {
    let ref_ty = Type::reference(false, Type::int());
    let mut_ref_ty = Type::reference(true, Type::int());

    match ref_ty {
        Type::Reference { mutable, inner } => {
            assert!(!mutable);
            assert!(matches!(*inner, Type::Int));
        }
        _ => panic!("Expected reference type"),
    }

    match mut_ref_ty {
        Type::Reference { mutable, inner } => {
            assert!(mutable);
            assert!(matches!(*inner, Type::Int));
        }
        _ => panic!("Expected mutable reference type"),
    }
}

#[test]
fn test_checked_reference() {
    let checked = Type::checked_reference(false, Type::int());

    match checked {
        Type::CheckedReference { mutable, inner } => {
            assert!(!mutable);
            assert!(matches!(*inner, Type::Int));
        }
        _ => panic!("Expected checked reference type"),
    }
}

#[test]
fn test_unsafe_reference() {
    let unsafe_ref = Type::unsafe_reference(false, Type::int());

    match unsafe_ref {
        Type::UnsafeReference { mutable, inner } => {
            assert!(!mutable);
            assert!(matches!(*inner, Type::Int));
        }
        _ => panic!("Expected unsafe reference type"),
    }
}

#[test]
fn test_tuple_type() {
    let tuple = Type::tuple(vec![Type::int(), Type::float(), Type::bool()].into());

    match tuple {
        Type::Tuple(types) => {
            assert_eq!(types.len(), 3);
            assert!(matches!(types[0], Type::Int));
            assert!(matches!(types[1], Type::Float));
            assert!(matches!(types[2], Type::Bool));
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_array_type() {
    let array = Type::array(Type::int(), Some(10));

    match array {
        Type::Array { element, size } => {
            assert!(matches!(*element, Type::Int));
            assert_eq!(size, Some(10));
        }
        _ => panic!("Expected array type"),
    }
}

#[test]
fn test_type_equality() {
    assert_eq!(Type::int(), Type::int());
    assert_eq!(Type::bool(), Type::bool());
    assert_ne!(Type::int(), Type::float());

    let tuple1 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let tuple2 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let tuple3 = Type::tuple(vec![Type::bool(), Type::int()].into());

    assert_eq!(tuple1, tuple2);
    assert_ne!(tuple1, tuple3);
}

#[test]
fn test_type_display() {
    assert_eq!(format!("{}", Type::unit()), "Unit");
    assert_eq!(format!("{}", Type::bool()), "Bool");
    assert_eq!(format!("{}", Type::int()), "Int");
    assert_eq!(format!("{}", Type::float()), "Float");
    assert_eq!(format!("{}", Type::text()), "Text");

    let fn_ty = Type::function(vec![Type::int()].into(), Type::bool());
    assert_eq!(format!("{}", fn_ty), "fn(Int) -> Bool");

    let tuple = Type::tuple(vec![Type::int(), Type::bool()].into());
    assert_eq!(format!("{}", tuple), "(Int, Bool)");
}

#[test]
fn test_error_types() {
    // Test affine violation error
    let affine_err = TypeError::AffineViolation {
        ty: Text::from("FileHandle"),
        first_use: dummy_span(),
        second_use: dummy_span(),
    };

    assert!(format!("{}", affine_err).contains("FileHandle"));
    assert!(format!("{}", affine_err).contains("more than once"));

    // Test linear violation error
    let linear_err = TypeError::LinearViolation {
        ty: Text::from("MustUse"),
        usage_count: 0,
        span: dummy_span(),
    };

    assert!(format!("{}", linear_err).contains("MustUse"));
    assert!(format!("{}", linear_err).contains("not used exactly once"));

    // Test moved value error
    let moved_err = TypeError::MovedValueUsed {
        name: Text::from("handle"),
        moved_at: dummy_span(),
        used_at: dummy_span(),
    };

    let msg = format!("{}", moved_err);
    assert!(msg.contains("handle"));
    assert!(msg.contains("used after move"));
    // Should also contain location information
    assert!(msg.contains("moved at"));
    assert!(msg.contains("used at"));
}

#[test]
fn test_type_free_vars() {
    // Type with no variables
    let int_ty = Type::int();
    assert!(int_ty.free_vars().is_empty());

    // Type with variables
    let var = TypeVar::fresh();
    let var_ty = Type::Var(var);
    let free = var_ty.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&var));
}

#[test]
fn test_type_is_monotype() {
    // Monotype (no free variables)
    assert!(Type::int().is_monotype());
    assert!(Type::bool().is_monotype());
    assert!(Type::function(vec![Type::int()].into(), Type::bool()).is_monotype());

    // Not monotype (has type variable)
    let var_ty = Type::Var(TypeVar::fresh());
    assert!(!var_ty.is_monotype());
}

#[test]
fn test_type_base() {
    // Base of non-refined type is itself
    let int_ty = Type::int();
    assert_eq!(int_ty.base(), &Type::Int);

    // NOTE: Refined type base extraction is tested in refinement_tests.rs
    // which has comprehensive tests for refinement type construction and operations
}

// Integration tests for affine checking (to be implemented)
// These require a full type checker implementation

#[cfg(feature = "integration_tests")]
mod integration {
    use super::*;

    #[test]
    fn test_affine_double_use_error() {
        // This would be implemented once the full type checker supports affine tracking
        //
        // let code = r#"
        //     type affine Handle is { fd: Int }
        //
        //     fn use_twice(h: Handle) -> Int {
        //         let x = h.fd;  // First use
        //         let y = h.fd;  // ERROR: Second use
        //         x + y
        //     }
        // "#;
        //
        // let err = type_check(code).unwrap_err();
        // assert!(matches!(err, TypeError::AffineViolation { .. }));
    }

    #[test]
    fn test_affine_moved_value() {
        // let code = r#"
        //     type affine Handle is { fd: Int }
        //
        //     fn consume(h: Handle) -> Int { h.fd }
        //
        //     let handle = Handle { fd: 42 };
        //     let x = consume(handle);  // handle moved
        //     let y = consume(handle);  // ERROR: handle already moved
        // "#;
        //
        // let err = type_check(code).unwrap_err();
        // assert!(matches!(err, TypeError::MovedValueUsed { .. }));
    }

    #[test]
    fn test_linear_not_used() {
        // let code = r#"
        //     type linear MustUse is { value: Int }
        //
        //     fn test() {
        //         let x = MustUse { value: 42 };
        //         // ERROR: linear value not used
        //     }
        // "#;
        //
        // let err = type_check(code).unwrap_err();
        // assert!(matches!(err, TypeError::LinearViolation { .. }));
    }
}

// ============================================================================
// Implicit Affine Stdlib Types Tests
// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 2 (Implicit Affine для stdlib)
// ============================================================================

mod stdlib_affine_tests {
    use verum_types::affine::AffineTracker;

    #[test]
    fn test_with_core_starts_empty() {
        // with_core() no longer hardcodes stdlib types — affine types are
        // discovered from `type affine` declarations in source code.
        let tracker = AffineTracker::with_core();

        // No types registered until parsed from source
        assert!(!tracker.is_affine_type("Text"));
        assert!(!tracker.is_affine_type("List"));
        assert!(!tracker.is_affine_type("Map"));
    }

    #[test]
    fn test_register_affine_types() {
        let mut tracker = AffineTracker::with_core();

        // Types are registered when parsed from source declarations
        tracker.register_affine_type("Text");
        tracker.register_affine_type("List");
        tracker.register_affine_type("Heap");
        tracker.register_affine_type("Shared");

        assert!(tracker.is_affine_type("Text"));
        assert!(tracker.is_affine_type("List"));
        assert!(tracker.is_affine_type("Heap"));
        assert!(tracker.is_affine_type("Shared"));
    }

    #[test]
    fn test_io_types_registered_as_affine() {
        let mut tracker = AffineTracker::with_core();

        tracker.register_affine_type("File");
        tracker.register_affine_type("TcpStream");
        tracker.register_affine_type("UdpSocket");
        tracker.register_affine_type("TcpListener");

        assert!(tracker.is_affine_type("File"));
        assert!(tracker.is_affine_type("TcpStream"));
        assert!(tracker.is_affine_type("UdpSocket"));
        assert!(tracker.is_affine_type("TcpListener"));
    }

    #[test]
    fn test_concurrency_types_registered_as_affine() {
        let mut tracker = AffineTracker::with_core();

        for ty in &["Channel", "Sender", "Receiver", "Mutex", "RwLock", "Condvar"] {
            tracker.register_affine_type(*ty);
        }

        assert!(tracker.is_affine_type("Channel"));
        assert!(tracker.is_affine_type("Mutex"));
        assert!(tracker.is_affine_type("Condvar"));
    }

    #[test]
    fn test_async_types_registered_as_affine() {
        let mut tracker = AffineTracker::with_core();

        for ty in &["Future", "Promise", "Task", "JoinHandle"] {
            tracker.register_affine_type(*ty);
        }

        assert!(tracker.is_affine_type("Future"));
        assert!(tracker.is_affine_type("Task"));
    }

    #[test]
    fn test_primitive_types_are_not_affine() {
        let tracker = AffineTracker::with_core();

        // Primitive types should NOT be affine (they are Copy)
        assert!(!tracker.is_affine_type("Int"));
        assert!(!tracker.is_affine_type("Float"));
        assert!(!tracker.is_affine_type("Bool"));
        assert!(!tracker.is_affine_type("Char"));
        assert!(!tracker.is_affine_type("Unit"));
    }

    #[test]
    fn test_new_scope_preserves_affine_types() {
        let mut tracker = AffineTracker::with_core();
        tracker.register_affine_type("Text");
        tracker.register_affine_type("List");
        tracker.register_affine_type("Mutex");

        let scoped = tracker.new_scope();

        // Scoped tracker should preserve affine type registrations
        assert!(scoped.is_affine_type("Text"));
        assert!(scoped.is_affine_type("List"));
        assert!(scoped.is_affine_type("Mutex"));
    }

    #[test]
    fn test_register_additional_affine_type() {
        let mut tracker = AffineTracker::with_core();

        // User can register additional affine types
        tracker.register_affine_type("MyCustomResource");
        assert!(tracker.is_affine_type("MyCustomResource"));

        // Unregistered types are not affine
        assert!(!tracker.is_affine_type("Text"));
    }
}

// ============================================================================
// Linear Types Tests
// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
// ============================================================================

mod linear_type_tests {
    use verum_types::affine::{AffineTracker, ResourceKind};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path};
    use verum_types::Type;

    /// Helper to create a simple named type
    fn named_type(name: &str) -> Type {
        let ident = Ident::new(name, Span::dummy());
        let path = Path::single(ident);
        Type::Named {
            path,
            args: vec![].into(),
        }
    }

    #[test]
    fn test_resource_kind_copy() {
        let kind = ResourceKind::Copy;
        assert!(kind.allows_multiple_use());
        assert!(!kind.is_at_most_once());
        assert!(!kind.is_exactly_once());
    }

    #[test]
    fn test_resource_kind_affine() {
        let kind = ResourceKind::Affine;
        assert!(!kind.allows_multiple_use());
        assert!(kind.is_at_most_once());
        assert!(!kind.is_exactly_once());
    }

    #[test]
    fn test_resource_kind_linear() {
        let kind = ResourceKind::Linear;
        assert!(!kind.allows_multiple_use());
        assert!(kind.is_at_most_once());
        assert!(kind.is_exactly_once());
    }

    #[test]
    fn test_register_linear_type() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        assert!(tracker.is_linear_type("MustClose"));
        assert!(!tracker.is_affine_type("MustClose"));
        assert_eq!(tracker.get_resource_kind("MustClose"), ResourceKind::Linear);
    }

    #[test]
    fn test_linear_type_priority() {
        // If a type is registered as both linear and affine, linear takes priority
        let mut tracker = AffineTracker::new();
        tracker.register_affine_type("Resource");
        tracker.register_linear_type("Resource");

        // Linear should take priority
        assert_eq!(tracker.get_resource_kind("Resource"), ResourceKind::Linear);
    }

    #[test]
    fn test_get_resource_kind_unregistered() {
        let tracker = AffineTracker::new();
        // Unregistered types are Copy
        assert_eq!(tracker.get_resource_kind("UnknownType"), ResourceKind::Copy);
    }

    #[test]
    fn test_bind_linear_type() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let span = Span::dummy();
        let ty = named_type("MustClose");
        tracker.bind("f", ty, span);

        assert!(tracker.is_binding_linear("f"));
        assert_eq!(tracker.get_binding_resource_kind("f"), Some(ResourceKind::Linear));
    }

    #[test]
    fn test_check_linear_consumed_empty() {
        let tracker = AffineTracker::new();
        let scope_end = Span::dummy();

        let errors = tracker.check_linear_consumed(scope_end);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_check_linear_consumed_unconsumed() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let span = Span::dummy();
        let ty = named_type("MustClose");
        tracker.bind("f", ty, span);

        // Don't consume the linear value
        let scope_end = Span::dummy();
        let errors = tracker.check_linear_consumed(scope_end);

        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_check_linear_consumed_when_consumed() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let span = Span::dummy();
        let ty = named_type("MustClose");
        tracker.bind("f", ty, span);

        // Consume the linear value
        let use_span = Span::dummy();
        tracker.use_value("f", use_span).unwrap();

        // Now there should be no errors
        let scope_end = Span::dummy();
        let errors = tracker.check_linear_consumed(scope_end);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_affine_not_in_linear_check() {
        let mut tracker = AffineTracker::new();
        tracker.register_affine_type("Handle");

        let span = Span::dummy();
        let ty = named_type("Handle");
        tracker.bind("h", ty, span);

        // Affine values don't need to be consumed (at most once)
        let scope_end = Span::dummy();
        let errors = tracker.check_linear_consumed(scope_end);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_linear_multiple_bindings() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("Connection");

        let span = Span::dummy();
        let ty1 = named_type("Connection");
        let ty2 = named_type("Connection");
        tracker.bind("c1", ty1, span);
        tracker.bind("c2", ty2, span);

        // Neither consumed - should get 2 errors
        let scope_end = Span::dummy();
        let errors = tracker.check_linear_consumed(scope_end);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_linear_partial_consumption() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("Transaction");

        let span = Span::dummy();
        let ty1 = named_type("Transaction");
        let ty2 = named_type("Transaction");
        tracker.bind("t1", ty1, span);
        tracker.bind("t2", ty2, span);

        // Only consume t1
        let use_span = Span::dummy();
        tracker.use_value("t1", use_span).unwrap();

        // Should get 1 error for t2
        let scope_end = Span::dummy();
        let errors = tracker.check_linear_consumed(scope_end);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_bind_with_explicit_kind() {
        let mut tracker = AffineTracker::new();

        let span = Span::dummy();
        let ty = named_type("CustomResource");
        tracker.bind_with_kind("r", ty, ResourceKind::Linear, span);

        assert!(tracker.is_binding_linear("r"));
    }
}
