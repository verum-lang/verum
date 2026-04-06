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
//! Comprehensive tests for type system features:
//! - Linear type tracking
//! - Implicit argument resolution
//! - GAT parsing and checking
//! - Specialization overlap detection

use verum_ast::span::{FileId, Span};
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{ConstValue, List, Map, Maybe, Set, Text};

use verum_types::Type;
use verum_types::affine::{AffineTracker, ResourceKind, check_linear_modifier, check_resource_modifier};
use verum_types::implicit::{
    ConstraintSource, ImplicitContext, ImplicitElaborator, ImplicitResolver,
};
use verum_types::specialization::{OverlapDetector, SpecializationValidationError, SpecializationValidator};
use verum_types::ty::{Substitution, SubstitutionExt, TypeVar, UniverseLevel};
use verum_types::unify::Unifier;
use verum_types::{TypeError, UniverseConstraint, UniverseContext, UniverseSubstitution};

fn dummy_span() -> Span {
    Span::new(0, 1, FileId::dummy())
}

fn named_type(name: &str) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, dummy_span())),
        args: List::new(),
    }
}

fn generic_type(name: &str, args: Vec<Type>) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, dummy_span())),
        args: args.into(),
    }
}

// ============================================================================
// LINEAR TYPE TRACKING TESTS
// ============================================================================

mod linear_types {
    use super::*;

    #[test]
    fn test_linear_type_registration() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        assert!(tracker.is_linear_type("MustClose"));
        assert!(!tracker.is_linear_type("RegularType"));
        assert!(!tracker.is_affine_type("MustClose")); // Linear is not affine
    }

    #[test]
    fn test_linear_resource_kind() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");
        tracker.register_affine_type("FileHandle");

        assert_eq!(tracker.get_resource_kind("MustClose"), ResourceKind::Linear);
        assert_eq!(tracker.get_resource_kind("FileHandle"), ResourceKind::Affine);
        assert_eq!(tracker.get_resource_kind("Int"), ResourceKind::Copy);
    }

    #[test]
    fn test_linear_value_consumed_ok() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("handle", ty, dummy_span());

        // Consume it
        tracker.use_value("handle", dummy_span()).unwrap();

        // Check should pass - value was consumed
        let errors = tracker.check_linear_consumed(dummy_span());
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_linear_value_not_consumed_error() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("handle", ty, dummy_span());

        // Don't consume - check should report error
        let errors = tracker.check_linear_consumed(dummy_span());
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TypeError::LinearNotConsumed { name, .. } if name.as_str() == "handle"));
    }

    #[test]
    fn test_linear_value_double_use_error() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("handle", ty, dummy_span());

        // First use - OK
        tracker.use_value("handle", dummy_span()).unwrap();

        // Second use - should fail (moved)
        let result = tracker.use_value("handle", dummy_span());
        assert!(result.is_err());
    }

    #[test]
    fn test_linear_binding_detection() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("handle", ty, dummy_span());

        assert!(tracker.is_binding_linear("handle"));
        assert_eq!(
            tracker.get_binding_resource_kind("handle"),
            Some(ResourceKind::Linear)
        );
    }

    #[test]
    fn test_linear_vs_affine_scope_check() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");
        tracker.register_affine_type("FileHandle");

        let linear_ty = named_type("MustClose");
        let affine_ty = named_type("FileHandle");

        tracker.bind("linear_val", linear_ty, dummy_span());
        tracker.bind("affine_val", affine_ty, dummy_span());

        // Neither consumed - linear should report error, affine should not
        let errors = tracker.check_linear_consumed(dummy_span());
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TypeError::LinearNotConsumed { name, .. } if name.as_str() == "linear_val"));
    }

    #[test]
    fn test_linear_in_loop_error() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("handle", ty, dummy_span());

        tracker.enter_loop();

        // Using linear value from outer scope in loop should error
        let result = tracker.use_value("handle", dummy_span());
        assert!(result.is_err());

        tracker.exit_loop();
    }

    #[test]
    fn test_resource_kind_properties() {
        assert!(ResourceKind::Copy.allows_multiple_use());
        assert!(!ResourceKind::Affine.allows_multiple_use());
        assert!(!ResourceKind::Linear.allows_multiple_use());

        assert!(!ResourceKind::Copy.is_at_most_once());
        assert!(ResourceKind::Affine.is_at_most_once());
        assert!(ResourceKind::Linear.is_at_most_once());

        assert!(!ResourceKind::Copy.is_exactly_once());
        assert!(!ResourceKind::Affine.is_exactly_once());
        assert!(ResourceKind::Linear.is_exactly_once());
    }

    #[test]
    fn test_check_resource_modifier_includes_linear() {
        use verum_ast::decl::ResourceModifier;

        assert!(check_resource_modifier(&Some(ResourceModifier::Affine)));
        assert!(check_resource_modifier(&Some(ResourceModifier::Linear)));
        assert!(!check_resource_modifier(&None));
    }

    #[test]
    fn test_check_linear_modifier() {
        use verum_ast::decl::ResourceModifier;

        assert!(!check_linear_modifier(&Some(ResourceModifier::Affine)));
        assert!(check_linear_modifier(&Some(ResourceModifier::Linear)));
        assert!(!check_linear_modifier(&None));
    }

    #[test]
    fn test_linear_scope_preservation() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");
        tracker.register_affine_type("FileHandle");

        // new_scope should preserve type registrations
        let scoped = tracker.new_scope();
        assert!(scoped.is_linear_type("MustClose"));
        assert!(scoped.is_affine_type("FileHandle"));
    }

    #[test]
    fn test_linear_type_via_type_query() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        assert!(tracker.is_type_linear(&ty));
        assert!(tracker.is_type_affine(&ty)); // linear is at-most-once too
    }

    #[test]
    fn test_multiple_linear_values_check() {
        let mut tracker = AffineTracker::new();
        tracker.register_linear_type("MustClose");

        let ty = named_type("MustClose");
        tracker.bind("a", ty.clone(), dummy_span());
        tracker.bind("b", ty.clone(), dummy_span());
        tracker.bind("c", ty, dummy_span());

        // Consume only 'b'
        tracker.use_value("b", dummy_span()).unwrap();

        let errors = tracker.check_linear_consumed(dummy_span());
        assert_eq!(errors.len(), 2); // 'a' and 'c' not consumed
    }

    #[test]
    fn test_bind_with_linear_kind() {
        let mut tracker = AffineTracker::new();
        // Don't register the type as linear - use explicit kind
        let ty = named_type("SomeType");
        tracker.bind_with_kind("val", ty, ResourceKind::Linear, dummy_span());

        assert!(tracker.is_binding_linear("val"));
        let errors = tracker.check_linear_consumed(dummy_span());
        assert_eq!(errors.len(), 1);
    }
}

// ============================================================================
// IMPLICIT ARGUMENT RESOLUTION TESTS
// ============================================================================

mod implicit_args {
    use super::*;

    #[test]
    fn test_implicit_resolver_basic_inference() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        // fn id{T}(x: T) -> T
        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        // Usage: id(42) => T should be Int
        resolver.add_constraint(t_meta, Type::Int, span, ConstraintSource::Argument { position: 0 });

        let subst = resolver.solve().unwrap();
        let inferred = resolver.get_inferred(t_meta, &subst);
        assert!(matches!(inferred, Maybe::Some(Type::Int)));
    }

    #[test]
    fn test_implicit_two_params() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        // fn pair{A, B}(a: A, b: B) -> (A, B)
        let a_meta = resolver.register_implicit(
            "A".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );
        let b_meta = resolver.register_implicit(
            "B".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        // Usage: pair(42, true) => A=Int, B=Bool
        resolver.add_constraint(a_meta, Type::Int, span, ConstraintSource::Argument { position: 0 });
        resolver.add_constraint(b_meta, Type::Bool, span, ConstraintSource::Argument { position: 1 });

        let subst = resolver.solve().unwrap();

        assert!(matches!(resolver.get_inferred(a_meta, &subst), Maybe::Some(Type::Int)));
        assert!(matches!(resolver.get_inferred(b_meta, &subst), Maybe::Some(Type::Bool)));
    }

    #[test]
    fn test_implicit_return_type_constraint() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        // fn default{T}() -> T
        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        // Usage: let x: Int = default()
        resolver.add_constraint(t_meta, Type::Int, span, ConstraintSource::ReturnType);

        let subst = resolver.solve().unwrap();
        assert!(matches!(resolver.get_inferred(t_meta, &subst), Maybe::Some(Type::Int)));
    }

    #[test]
    fn test_implicit_ambiguous_error() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        // No constraints => ambiguous
        let _t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        let result = resolver.solve();
        assert!(matches!(result, Err(TypeError::AmbiguousType { .. })));
    }

    #[test]
    fn test_implicit_consistent_constraints() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        // fn f{T}(a: T, b: T) -> T
        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        // Usage: f(42, 43) => T=Int from first, T=Int from second (consistent)
        resolver.add_constraint(t_meta, Type::Int, span, ConstraintSource::Argument { position: 0 });
        resolver.add_constraint(t_meta, Type::Int, span, ConstraintSource::Argument { position: 1 });

        let subst = resolver.solve().unwrap();
        assert!(matches!(resolver.get_inferred(t_meta, &subst), Maybe::Some(Type::Int)));
    }

    #[test]
    fn test_implicit_context_nested_scopes() {
        let mut ctx = ImplicitContext::new();
        assert!(!ctx.in_scope());

        // Outer function scope
        ctx.enter_scope();
        assert!(ctx.in_scope());

        {
            let scope = ctx.current_scope();
            assert!(matches!(scope, Maybe::Some(_)));
            if let Maybe::Some(resolver) = scope {
                assert_eq!(resolver.pending_count(), 0);
            }
        }

        // Inner function scope
        ctx.enter_scope();

        // Exit inner
        let inner = ctx.exit_scope();
        assert!(matches!(inner, Maybe::Some(_)));
        assert!(ctx.in_scope()); // Still in outer

        // Exit outer
        let outer = ctx.exit_scope();
        assert!(matches!(outer, Maybe::Some(_)));
        assert!(!ctx.in_scope());
    }

    #[test]
    fn test_elaborator_replaces_metavars() {
        let mut subst = Substitution::new();
        let tv1 = TypeVar::fresh();
        let tv2 = TypeVar::fresh();
        subst.insert(tv1, Type::Int);
        subst.insert(tv2, Type::Bool);

        let elaborator = ImplicitElaborator::new(subst);

        // Elaborate fn(T, U) -> T to fn(Int, Bool) -> Int
        let fn_ty = Type::Function {
            params: vec![Type::Var(tv1), Type::Var(tv2)].into(),
            return_type: Box::new(Type::Var(tv1)),
            type_params: List::new(),
            contexts: None,
            properties: None,
        };

        let elaborated = elaborator.elaborate_type(&fn_ty);
        match &elaborated {
            Type::Function { params, return_type, .. } => {
                assert_eq!(params[0], Type::Int);
                assert_eq!(params[1], Type::Bool);
                assert_eq!(**return_type, Type::Int);
            }
            _ => panic!("Expected Function type"),
        }
    }

    #[test]
    fn test_implicit_diagnostics_mode() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        resolver.add_constraint(t_meta, Type::Int, span, ConstraintSource::Argument { position: 0 });

        // solve_with_diagnostics should also work
        let subst = resolver.solve_with_diagnostics().unwrap();
        assert!(matches!(resolver.get_inferred(t_meta, &subst), Maybe::Some(Type::Int)));
    }

    #[test]
    fn test_implicit_resolver_clear() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        resolver.register_implicit("T".into(), Type::Universe { level: UniverseLevel::TYPE }, span);
        assert_eq!(resolver.pending_count(), 1);

        resolver.clear();
        assert_eq!(resolver.pending_count(), 0);
        assert_eq!(resolver.constraint_count(), 0);
    }

    #[test]
    fn test_implicit_field_access_constraint() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        // Constraint from field access: expr.field
        resolver.add_constraint(
            t_meta,
            Type::Float,
            span,
            ConstraintSource::FieldAccess { field: "x".into() },
        );

        let subst = resolver.solve().unwrap();
        assert!(matches!(resolver.get_inferred(t_meta, &subst), Maybe::Some(Type::Float)));
    }

    #[test]
    fn test_implicit_type_annotation_constraint() {
        let mut resolver = ImplicitResolver::new();
        let span = dummy_span();

        let t_meta = resolver.register_implicit(
            "T".into(),
            Type::Universe { level: UniverseLevel::TYPE },
            span,
        );

        resolver.add_constraint(t_meta, Type::Text, span, ConstraintSource::TypeAnnotation);

        let subst = resolver.solve().unwrap();
        assert!(matches!(resolver.get_inferred(t_meta, &subst), Maybe::Some(Type::Text)));
    }
}

// ============================================================================
// GAT (GENERIC ASSOCIATED TYPES) TESTS
// ============================================================================

mod gats {
    use super::*;
    use verum_types::advanced_protocols::{AssociatedTypeKind, GATTypeParam, GATWhereClause};
    use verum_types::protocol::{AssociatedType, Protocol, ProtocolBound, ProtocolKind};
    use verum_types::variance::Variance;

    #[test]
    fn test_gat_associated_type_creation() {
        // type Item<T> in a protocol
        let gat = AssociatedType::generic(
            "Item".into(),
            List::from_iter([GATTypeParam {
                name: "T".into(),
                bounds: List::new(),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            List::new(),
            List::new(),
        );

        assert_eq!(gat.type_params.len(), 1);
        assert_eq!(gat.type_params[0].name.as_str(), "T");
        assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 1 }));
        assert!(gat.is_gat());
    }

    #[test]
    fn test_gat_with_bounds() {
        // type Item<T: Clone + Debug>
        let clone_bound = ProtocolBound::simple(Path::single(Ident::new("Clone", dummy_span())));
        let debug_bound = ProtocolBound::simple(Path::single(Ident::new("Debug", dummy_span())));

        let gat = AssociatedType::generic(
            "Item".into(),
            List::from_iter([GATTypeParam {
                name: "T".into(),
                bounds: List::from_iter([clone_bound, debug_bound]),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            List::new(),
            List::new(),
        );

        assert_eq!(gat.type_params[0].bounds.len(), 2);
    }

    #[test]
    fn test_gat_with_where_clauses() {
        // type Item<T> where T: Ord
        let where_clause = GATWhereClause {
            param: "T".into(),
            constraints: List::from_iter([ProtocolBound::simple(
                Path::single(Ident::new("Ord", dummy_span())),
            )]),
            span: dummy_span(),
        };

        let gat = AssociatedType::generic(
            "Item".into(),
            List::from_iter([GATTypeParam {
                name: "T".into(),
                bounds: List::new(),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            List::new(),
            List::from_iter([where_clause]),
        );

        assert_eq!(gat.where_clauses.len(), 1);
        assert_eq!(gat.where_clauses[0].param.as_str(), "T");
    }

    #[test]
    fn test_gat_multi_param() {
        // type Transform<A, B>
        let gat = AssociatedType::generic(
            "Transform".into(),
            List::from_iter([
                GATTypeParam {
                    name: "A".into(),
                    bounds: List::new(),
                    default: Maybe::None,
                    variance: Variance::Covariant,
                },
                GATTypeParam {
                    name: "B".into(),
                    bounds: List::new(),
                    default: Maybe::None,
                    variance: Variance::Contravariant,
                },
            ]),
            List::new(),
            List::new(),
        );

        assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 2 }));
        assert_eq!(gat.type_params[1].variance, Variance::Contravariant);
        assert_eq!(gat.arity(), 2);
    }

    #[test]
    fn test_regular_associated_type() {
        // type Item (non-GAT)
        let assoc = AssociatedType::simple("Item".into(), List::new());

        assert!(matches!(assoc.kind, AssociatedTypeKind::Regular));
        assert!(assoc.type_params.is_empty());
        assert!(assoc.where_clauses.is_empty());
        assert!(!assoc.is_gat());
    }

    #[test]
    fn test_protocol_with_gat() {
        // protocol Container {
        //     type Item<T>;
        //     fn get{T}(self: &Self, key: T) -> Self.Item<T>;
        // }
        let mut associated_types = Map::new();
        associated_types.insert(
            "Item".into(),
            AssociatedType::generic(
                "Item".into(),
                List::from_iter([GATTypeParam {
                    name: "T".into(),
                    bounds: List::new(),
                    default: Maybe::None,
                    variance: Variance::Covariant,
                }]),
                List::new(),
                List::new(),
            ),
        );

        let protocol = Protocol {
            kind: ProtocolKind::Constraint,
            name: "Container".into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: dummy_span(),
        };

        assert!(protocol.associated_types.contains_key(&Text::from("Item")));
        let item_gat = &protocol.associated_types[&Text::from("Item")];
        assert!(matches!(item_gat.kind, AssociatedTypeKind::Generic { arity: 1 }));
    }

    #[test]
    fn test_gat_with_default_type() {
        // type Item<T> = Maybe<T>
        let mut gat = AssociatedType::generic(
            "Item".into(),
            List::from_iter([GATTypeParam {
                name: "T".into(),
                bounds: List::new(),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            List::new(),
            List::new(),
        );

        // GAT can have a default type
        gat.default = Maybe::Some(generic_type("Maybe", vec![Type::Var(TypeVar::fresh())]));
        assert!(matches!(gat.default, Maybe::Some(_)));
    }

    #[test]
    fn test_type_context_protocol_bounds() {
        let mut ctx = verum_types::TypeContext::new();
        let tv = TypeVar::fresh();

        let bound = ProtocolBound::simple(Path::single(Ident::new("Clone", dummy_span())));

        ctx.add_protocol_bound(tv, bound);

        assert!(ctx.has_protocol_bound(&tv, &"Clone".into()));
        assert!(!ctx.has_protocol_bound(&tv, &"Debug".into()));

        // Get bounds
        let bounds = ctx.get_protocol_bounds(&tv);
        assert!(matches!(bounds, Maybe::Some(b) if b.len() == 1));

        // Clear bounds
        ctx.clear_protocol_bounds(&tv);
        assert!(!ctx.has_protocol_bound(&tv, &"Clone".into()));
    }

    #[test]
    fn test_covariant_associated_type() {
        let assoc = AssociatedType::covariant("Output".into(), List::new());
        assert_eq!(assoc.expected_variance, Variance::Covariant);
        assert!(!assoc.is_gat());
    }

    #[test]
    fn test_contravariant_associated_type() {
        let assoc = AssociatedType::contravariant("Input".into(), List::new());
        assert_eq!(assoc.expected_variance, Variance::Contravariant);
        assert!(!assoc.is_gat());
    }
}

// ============================================================================
// SPECIALIZATION OVERLAP DETECTION TESTS
// ============================================================================

mod specialization_overlap {
    use super::*;
    use verum_types::advanced_protocols::SpecializationInfo;
    use verum_types::protocol::{Protocol, ProtocolBound, ProtocolImpl, ProtocolKind, WhereClause};

    fn make_protocol(name: &str) -> Protocol {
        Protocol {
            kind: ProtocolKind::Constraint,
            name: name.into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: dummy_span(),
        }
    }

    fn make_impl(protocol_name: &str, for_type: Type) -> ProtocolImpl {
        ProtocolImpl {
            protocol: Path::single(Ident::new(protocol_name, dummy_span())),
            protocol_args: List::new(),
            for_type,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: dummy_span(),
            type_param_fn_bounds: Map::new(),
        }
    }

    fn make_specialized_impl(protocol_name: &str, for_type: Type) -> ProtocolImpl {
        let mut imp = make_impl(protocol_name, for_type);
        imp.specialization = Maybe::Some(SpecializationInfo {
            is_specialized: true,
            specializes: Maybe::None,
            specificity_rank: 10,
            is_default: false,
            span: dummy_span(),
        });
        imp
    }

    #[test]
    fn test_no_overlap_different_types() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Int),
            make_impl("Display", Type::Bool),
        ];

        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_ok());
    }

    #[test]
    fn test_overlap_same_concrete_type() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Int),
            make_impl("Display", Type::Int),
        ];

        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_err());
    }

    #[test]
    fn test_overlap_type_var_with_concrete() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Var(TypeVar::fresh())), // implement<T> Display for T
            make_impl("Display", Type::Int),                    // implement Display for Int
        ];

        // Should detect overlap since T can be Int
        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_err());
    }

    #[test]
    fn test_overlap_with_specialization_ok() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Var(TypeVar::fresh())), // Base: implement<T> Display for T
            make_specialized_impl("Display", Type::Int),        // @specialize: implement Display for Int
        ];

        // Should be OK because specialization relationship exists
        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_ok());
    }

    #[test]
    fn test_overlap_generic_types() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Clone");

        let impls = vec![
            make_impl("Clone", generic_type("List", vec![Type::Var(TypeVar::fresh())])),
            make_impl("Clone", generic_type("List", vec![Type::Int])),
        ];

        // List<T> overlaps with List<Int>
        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_overlap_different_generics() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Clone");

        let impls = vec![
            make_impl("Clone", generic_type("List", vec![Type::Int])),
            make_impl("Clone", generic_type("Map", vec![Type::Int])),
        ];

        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validator_full_pipeline() {
        let mut validator = SpecializationValidator::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Var(TypeVar::fresh())), // Base
            make_specialized_impl("Display", Type::Int),        // Specialized
        ];

        let result = validator.validate_specializations(&protocol, &impls);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validator_overlap_without_specialization() {
        let mut validator = SpecializationValidator::new();
        let protocol = make_protocol("Display");

        let impls = vec![
            make_impl("Display", Type::Var(TypeVar::fresh())),
            make_impl("Display", Type::Int), // NOT @specialize
        ];

        let result = validator.validate_specializations(&protocol, &impls);
        assert!(result.is_err());
    }

    #[test]
    fn test_overlap_detection_tuple_types() {
        let detector = OverlapDetector::new();

        let ty1 = Type::Tuple(vec![Type::Int, Type::Bool].into());
        let ty2 = Type::Tuple(vec![Type::Int, Type::Bool].into());
        assert!(detector.check_overlap(&ty1, &ty2).is_some());

        let ty3 = Type::Tuple(vec![Type::Int, Type::Float].into());
        assert!(detector.check_overlap(&ty1, &ty3).is_none());
    }

    #[test]
    fn test_overlap_detection_function_types() {
        let detector = OverlapDetector::new();

        let fn1 = Type::function(vec![Type::Int].into(), Type::Bool);
        let fn2 = Type::function(vec![Type::Int].into(), Type::Bool);
        assert!(detector.check_overlap(&fn1, &fn2).is_some());

        let fn3 = Type::function(vec![Type::Float].into(), Type::Bool);
        assert!(detector.check_overlap(&fn1, &fn3).is_none());
    }

    #[test]
    fn test_overlap_detection_array_types() {
        let detector = OverlapDetector::new();

        let arr1 = Type::Array { element: Box::new(Type::Int), size: Some(5) };
        let arr2 = Type::Array { element: Box::new(Type::Int), size: Some(5) };
        assert!(detector.check_overlap(&arr1, &arr2).is_some());

        let arr3 = Type::Array { element: Box::new(Type::Int), size: Some(10) };
        assert!(detector.check_overlap(&arr1, &arr3).is_none()); // Different sizes

        let arr4 = Type::Array { element: Box::new(Type::Int), size: None };
        assert!(detector.check_overlap(&arr1, &arr4).is_some()); // Unknown size overlaps
    }

    #[test]
    fn test_overlap_reference_types() {
        let detector = OverlapDetector::new();

        let ref1 = Type::Reference { inner: Box::new(Type::Int), mutable: false };
        let ref2 = Type::Reference { inner: Box::new(Type::Int), mutable: false };
        assert!(detector.check_overlap(&ref1, &ref2).is_some());

        // Mut ref does not overlap with immut ref (when mut is first arg)
        let ref_mut = Type::Reference { inner: Box::new(Type::Int), mutable: true };
        let ref_immut = Type::Reference { inner: Box::new(Type::Int), mutable: false };
        assert!(detector.check_overlap(&ref_mut, &ref_immut).is_none());

        // Different inner types don't overlap
        let ref3 = Type::Reference { inner: Box::new(Type::Bool), mutable: false };
        assert!(detector.check_overlap(&ref1, &ref3).is_none());
    }

    #[test]
    fn test_three_way_overlap() {
        let mut detector = OverlapDetector::new();
        let protocol = make_protocol("Debug");

        let t1 = TypeVar::fresh();
        let t2 = TypeVar::fresh();

        let impls = vec![
            make_impl("Debug", Type::Var(t1)),            // implement<T> Debug for T
            make_impl("Debug", Type::Int),                  // implement Debug for Int
            make_impl("Debug", generic_type("List", vec![Type::Var(t2)])), // implement<T> Debug for List<T>
        ];

        // All three overlap with the first (blanket impl)
        let result = detector.detect_overlaps(&protocol, &impls);
        assert!(result.is_err());
        // Should detect at least 2 overlaps (blanket+Int, blanket+List<T>)
        if let Err(errors) = result {
            assert!(errors.len() >= 2);
        }
    }

    #[test]
    fn test_overlap_with_where_clause_impl() {
        let mut validator = SpecializationValidator::new();
        let protocol = make_protocol("Clone");

        let t_var = TypeVar::fresh();

        // Base impl: implement<T> Clone for List<T> where T: Clone
        let mut base = make_impl("Clone", generic_type("List", vec![Type::Var(t_var)]));
        base.where_clauses.push(WhereClause {
            ty: Type::Var(t_var),
            bounds: List::from_iter([ProtocolBound::simple(
                Path::single(Ident::new("Clone", dummy_span())),
            )]),
        });

        // Specialized: implement Clone for List<Int> where Int: Clone
        // (more specific type + where clauses cover base bounds)
        let mut specialized = make_specialized_impl(
            "Clone",
            generic_type("List", vec![Type::Int]),
        );
        specialized.where_clauses.push(WhereClause {
            ty: Type::Int,
            bounds: List::from_iter([ProtocolBound::simple(
                Path::single(Ident::new("Clone", dummy_span())),
            )]),
        });

        let impls = vec![base, specialized];
        let result = validator.validate_specializations(&protocol, &impls);
        // Should succeed: List<Int> is more specific than List<T>, and Int: Clone satisfies T: Clone
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_overlap_different_arity() {
        let detector = OverlapDetector::new();

        let ty1 = Type::Tuple(vec![Type::Int].into());
        let ty2 = Type::Tuple(vec![Type::Int, Type::Bool].into());
        assert!(detector.check_overlap(&ty1, &ty2).is_none());
    }

    #[test]
    fn test_overlap_slice_types() {
        let detector = OverlapDetector::new();

        let slice1 = Type::Slice { element: Box::new(Type::Int) };
        let slice2 = Type::Slice { element: Box::new(Type::Int) };
        assert!(detector.check_overlap(&slice1, &slice2).is_some());

        let slice3 = Type::Slice { element: Box::new(Type::Bool) };
        assert!(detector.check_overlap(&slice1, &slice3).is_none());
    }
}

// ============================================================================
// UNIVERSE HIERARCHY TESTS
// ============================================================================

mod universe_hierarchy {
    use super::*;

    #[test]
    fn test_universe_level_constants() {
        assert_eq!(UniverseLevel::TYPE, UniverseLevel::Concrete(0));
        assert_eq!(UniverseLevel::TYPE1, UniverseLevel::Concrete(1));
        assert_eq!(UniverseLevel::TYPE2, UniverseLevel::Concrete(2));
    }

    #[test]
    fn test_universe_succ() {
        assert_eq!(UniverseLevel::TYPE.succ(), UniverseLevel::TYPE1);
        assert_eq!(UniverseLevel::TYPE1.succ(), UniverseLevel::TYPE2);
    }

    #[test]
    fn test_universe_max_concrete() {
        let max = UniverseLevel::TYPE.max(UniverseLevel::TYPE1);
        assert_eq!(max, UniverseLevel::TYPE1);

        let max2 = UniverseLevel::TYPE2.max(UniverseLevel::TYPE);
        assert_eq!(max2, UniverseLevel::TYPE2);
    }

    #[test]
    fn test_universe_constraint_satisfaction() {
        let subst = UniverseSubstitution::new();

        // 0 <= 1
        let c = UniverseConstraint::LessOrEqual(UniverseLevel::TYPE, UniverseLevel::TYPE1);
        assert!(c.is_satisfied(&subst));

        // NOT 1 < 0
        let c2 = UniverseConstraint::StrictlyLess(UniverseLevel::TYPE1, UniverseLevel::TYPE);
        assert!(!c2.is_satisfied(&subst));

        // 0 = 0
        let c3 = UniverseConstraint::Equal(UniverseLevel::TYPE, UniverseLevel::TYPE);
        assert!(c3.is_satisfied(&subst));

        // 1 = 0 + 1 (successor)
        let c4 = UniverseConstraint::Successor(UniverseLevel::TYPE1, UniverseLevel::TYPE);
        assert!(c4.is_satisfied(&subst));
    }

    #[test]
    fn test_universe_substitution_resolve() {
        let mut subst = UniverseSubstitution::new();
        subst.insert(0, UniverseLevel::Concrete(1));

        let resolved = subst.resolve(&UniverseLevel::Variable(0));
        assert_eq!(resolved, UniverseLevel::Concrete(1));

        // Unresolved variable stays as-is
        let unresolved = subst.resolve(&UniverseLevel::Variable(99));
        assert_eq!(unresolved, UniverseLevel::Variable(99));
    }

    #[test]
    fn test_universe_context_fresh_vars() {
        let mut ctx = UniverseContext::new();

        let v1 = ctx.fresh_universe_var();
        let v2 = ctx.fresh_universe_var();

        // Fresh variables should be different
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_universe_context_solve_basic() {
        let mut ctx = UniverseContext::new();

        // u0 = Type_0
        let u0 = ctx.fresh_universe_var();
        ctx.add_constraint(UniverseConstraint::Equal(u0, UniverseLevel::TYPE));

        // Should solve successfully
        ctx.solve().unwrap();
    }

    #[test]
    fn test_universe_context_solve_strict_ordering() {
        let mut ctx = UniverseContext::new();

        // Type_0 < Type_1 should be satisfiable
        ctx.add_cumulative(UniverseLevel::TYPE, UniverseLevel::TYPE1);

        ctx.solve().unwrap();
    }

    #[test]
    fn test_universe_context_contradiction() {
        let mut ctx = UniverseContext::new();

        // Type_1 < Type_0 should fail
        ctx.add_constraint(UniverseConstraint::StrictlyLess(
            UniverseLevel::TYPE1,
            UniverseLevel::TYPE,
        ));

        assert!(ctx.solve().is_err());
    }

    #[test]
    fn test_universe_max_constraint() {
        let mut ctx = UniverseContext::new();

        // result = max(Type_0, Type_1) should be Type_1
        let result = ctx.fresh_universe_var();
        ctx.add_max_constraint(
            result,
            List::from_iter([UniverseLevel::TYPE, UniverseLevel::TYPE1]),
        );

        ctx.solve().unwrap();
    }

    #[test]
    fn test_universe_constraint_variables() {
        let c = UniverseConstraint::LessOrEqual(
            UniverseLevel::Variable(0),
            UniverseLevel::Variable(1),
        );
        let vars = c.variables();
        assert!(vars.contains(&0));
        assert!(vars.contains(&1));
    }

    #[test]
    fn test_universe_substitution_merge() {
        let mut subst1 = UniverseSubstitution::new();
        subst1.insert(0, UniverseLevel::Concrete(1));

        let mut subst2 = UniverseSubstitution::new();
        subst2.insert(1, UniverseLevel::Concrete(2));

        subst1.merge(&subst2);

        assert_eq!(subst1.resolve(&UniverseLevel::Variable(0)), UniverseLevel::Concrete(1));
        assert_eq!(subst1.resolve(&UniverseLevel::Variable(1)), UniverseLevel::Concrete(2));
    }

    #[test]
    fn test_universe_successor_constraint_check() {
        let subst = UniverseSubstitution::new();

        // Succ(0) = 0 + 1: true
        let c = UniverseConstraint::Successor(
            UniverseLevel::Concrete(1),
            UniverseLevel::Concrete(0),
        );
        assert!(c.is_satisfied(&subst));

        // Succ(1) = 0 + 1: false (2 != 1)
        let c2 = UniverseConstraint::Successor(
            UniverseLevel::Concrete(2),
            UniverseLevel::Concrete(0),
        );
        assert!(!c2.is_satisfied(&subst));
    }

    #[test]
    fn test_universe_equal_concrete_violation() {
        let subst = UniverseSubstitution::new();

        let c = UniverseConstraint::Equal(
            UniverseLevel::Concrete(0),
            UniverseLevel::Concrete(1),
        );
        assert!(!c.is_satisfied(&subst));
    }
}
