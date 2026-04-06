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
//! Tests for Higher-Kinded Type (HKT) parsing.
//!
//! Tests for higher-kinded type syntax in grammar
//! higher_kinded_type = path , '<' , '_' , '>' ;
//!
//! Examples:
//! - List<_>
//! - Maybe<_>
//! - Result<_, E>

use verum_ast::ty::GenericArg;
use verum_ast::{FileId, TypeKind};
use verum_parser::VerumParser;

fn parse_type(source: &str) -> verum_ast::Type {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_type_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse type: {}", source))
}

#[test]
fn test_parse_list_underscore() {
    let ty = parse_type("List<_>");

    match &ty.kind {
        TypeKind::Generic { base, args } => {
            // Check base is List
            match &base.kind {
                TypeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                }
                _ => panic!("Expected Path type for base, got {:?}", base.kind),
            }

            // Check we have one argument that is Inferred
            assert_eq!(args.len(), 1, "Expected 1 generic argument");
            match &args[0] {
                GenericArg::Type(arg_ty) => {
                    assert!(
                        matches!(arg_ty.kind, TypeKind::Inferred),
                        "Expected Inferred type for _, got {:?}",
                        arg_ty.kind
                    );
                }
                _ => panic!("Expected Type generic argument, got {:?}", args[0]),
            }
        }
        _ => panic!("Expected Generic type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_maybe_underscore() {
    let ty = parse_type("Maybe<_>");
    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 1);
            match &args[0] {
                GenericArg::Type(arg_ty) => {
                    assert!(matches!(arg_ty.kind, TypeKind::Inferred));
                }
                _ => panic!("Expected Type generic argument"),
            }
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_parse_result_underscore_e() {
    let ty = parse_type("Result<_, E>");
    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 2, "Expected 2 generic arguments");

            // First arg should be Inferred (_)
            match &args[0] {
                GenericArg::Type(arg_ty) => {
                    assert!(
                        matches!(arg_ty.kind, TypeKind::Inferred),
                        "First arg should be Inferred"
                    );
                }
                _ => panic!("Expected Type generic argument"),
            }

            // Second arg should be Path (E)
            match &args[1] {
                GenericArg::Type(arg_ty) => match &arg_ty.kind {
                    TypeKind::Path(path) => {
                        assert_eq!(path.segments.len(), 1);
                    }
                    _ => panic!("Second arg should be Path"),
                },
                _ => panic!("Expected Type generic argument"),
            }
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_parse_type_param_with_underscore() {
    // Test parsing generic parameter with underscore placeholder
    // This is used in protocol definitions like:
    // protocol Functor {
    //     type F<_>
    // }
    let ty = parse_type("F<_>");

    match &ty.kind {
        TypeKind::Generic { base, args } => {
            assert_eq!(args.len(), 1);
            match &args[0] {
                GenericArg::Type(arg_ty) => {
                    assert!(
                        matches!(arg_ty.kind, TypeKind::Inferred),
                        "Expected Inferred type for _, got {:?}",
                        arg_ty.kind
                    );
                }
                _ => panic!("Expected Type generic argument"),
            }
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_parse_multiple_underscores() {
    // Test parsing multiple placeholders: Map<_, _>
    let ty = parse_type("Map<_, _>");

    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 2, "Expected 2 generic arguments");

            // Both args should be Inferred
            for (i, arg) in args.iter().enumerate() {
                match arg {
                    GenericArg::Type(arg_ty) => {
                        assert!(
                            matches!(arg_ty.kind, TypeKind::Inferred),
                            "Arg {} should be Inferred, got {:?}",
                            i,
                            arg_ty.kind
                        );
                    }
                    _ => panic!("Expected Type generic argument at position {}", i),
                }
            }
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_parse_nested_hkt() {
    // Test parsing nested HKT: List<Maybe<_>>
    let ty = parse_type("List<Maybe<_>>");

    match &ty.kind {
        TypeKind::Generic { base, args } => {
            // Check outer is List
            assert!(matches!(base.kind, TypeKind::Path(_)));
            assert_eq!(args.len(), 1);

            // Check inner is Maybe<_>
            match &args[0] {
                GenericArg::Type(inner_ty) => match &inner_ty.kind {
                    TypeKind::Generic {
                        args: inner_args, ..
                    } => {
                        assert_eq!(inner_args.len(), 1);
                        match &inner_args[0] {
                            GenericArg::Type(innermost_ty) => {
                                assert!(
                                    matches!(innermost_ty.kind, TypeKind::Inferred),
                                    "Innermost type should be Inferred"
                                );
                            }
                            _ => panic!("Expected Type generic argument"),
                        }
                    }
                    _ => panic!("Expected inner Generic type"),
                },
                _ => panic!("Expected Type generic argument"),
            }
        }
        _ => panic!("Expected Generic type"),
    }
}
