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
//! Tests for associated type equality constraints in where clauses.
//!
//! This test suite verifies that the parser correctly handles associated type equality
//! constraints of the form: `where T.Item = Int`

use verum_ast::ty::TypeBoundKind;
use verum_ast::{FileId, TypeKind, WhereClause, WherePredicateKind};
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

fn parse_where_clause(input: &str) -> WhereClause {
    let file_id = FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);
    parser
        .parse_where_clause()
        .expect("failed to parse where clause")
}

#[test]
fn test_simple_associated_type_equality() {
    // Test: where T.Item = Int
    let where_clause = parse_where_clause("where T.Item = Int");

    assert_eq!(where_clause.predicates.len(), 1);

    let predicate = &where_clause.predicates[0];
    match &predicate.kind {
        WherePredicateKind::Type { ty, bounds } => {
            // Check that ty is a Qualified type (T.Item)
            match &ty.kind {
                TypeKind::Qualified {
                    self_ty,
                    assoc_name,
                    ..
                } => {
                    // self_ty should be T
                    match &self_ty.kind {
                        TypeKind::Path(path) => {
                            assert_eq!(path.segments.len(), 1);
                            if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                assert_eq!(ident.name.as_str(), "T");
                            } else {
                                panic!("Expected Name segment");
                            }
                        }
                        _ => panic!("Expected Path for self_ty"),
                    }

                    // assoc_name should be Item
                    assert_eq!(assoc_name.name.as_str(), "Item");
                }
                _ => panic!("Expected Qualified type, got: {:?}", ty.kind),
            }

            // Check that bounds contains an Equality bound to Int
            assert_eq!(bounds.len(), 1);
            match &bounds[0].kind {
                TypeBoundKind::Equality(concrete_ty) => {
                    // Debug: print what we actually got
                    eprintln!("concrete_ty.kind = {:?}", concrete_ty.kind);
                    match &concrete_ty.kind {
                        TypeKind::Path(path) => {
                            assert_eq!(path.segments.len(), 1);
                            if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                assert_eq!(ident.name.as_str(), "Int");
                            } else {
                                panic!("Expected Name segment for Int");
                            }
                        }
                        TypeKind::Int => {
                            // Int is parsed as a primitive type (TypeKind::Int), not a Path
                            // This is actually correct!
                        }
                        _ => panic!(
                            "Expected Path type or primitive Int, got: {:?}",
                            concrete_ty.kind
                        ),
                    }
                }
                _ => panic!("Expected Equality bound"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }
}

#[test]
fn test_multiple_associated_type_equalities() {
    // Test: where T.Item = Int, U.Output = String
    let where_clause = parse_where_clause("where T.Item = Int, U.Output = String");

    assert_eq!(where_clause.predicates.len(), 2);

    // First predicate: T.Item = Int
    let pred1 = &where_clause.predicates[0];
    match &pred1.kind {
        WherePredicateKind::Type { ty, bounds } => {
            match &ty.kind {
                TypeKind::Qualified { assoc_name, .. } => {
                    assert_eq!(assoc_name.name.as_str(), "Item");
                }
                _ => panic!("Expected Qualified type"),
            }

            assert_eq!(bounds.len(), 1);
            match &bounds[0].kind {
                TypeBoundKind::Equality(concrete_ty) => {
                    match &concrete_ty.kind {
                        TypeKind::Path(path) => {
                            if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                assert_eq!(ident.name.as_str(), "Int");
                            }
                        }
                        TypeKind::Int => {
                            // Int is a primitive type
                        }
                        _ => panic!("Expected Path type or primitive Int"),
                    }
                }
                _ => panic!("Expected Equality bound"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }

    // Second predicate: U.Output = String
    let pred2 = &where_clause.predicates[1];
    match &pred2.kind {
        WherePredicateKind::Type { ty, bounds } => {
            match &ty.kind {
                TypeKind::Qualified { assoc_name, .. } => {
                    assert_eq!(assoc_name.name.as_str(), "Output");
                }
                _ => panic!("Expected Qualified type"),
            }

            assert_eq!(bounds.len(), 1);
            match &bounds[0].kind {
                TypeBoundKind::Equality(concrete_ty) => {
                    match &concrete_ty.kind {
                        TypeKind::Path(path) => {
                            if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                assert_eq!(ident.name.as_str(), "String");
                            }
                        }
                        TypeKind::Text => {
                            // String could be parsed as Text primitive - but we use String in this test
                            // so it should be Path
                            panic!("Expected Path type String, got Text primitive");
                        }
                        _ => panic!("Expected Path type for String"),
                    }
                }
                _ => panic!("Expected Equality bound"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }
}

#[test]
fn test_mixed_where_predicates() {
    // Test: where T: Iterator, T.Item = Int
    let where_clause = parse_where_clause("where T: Iterator, T.Item = Int");

    assert_eq!(where_clause.predicates.len(), 2);

    // First predicate: T: Iterator (protocol bound)
    let pred1 = &where_clause.predicates[0];
    match &pred1.kind {
        WherePredicateKind::Type { ty, bounds } => {
            // ty should be just T
            match &ty.kind {
                TypeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "T");
                    }
                }
                _ => panic!("Expected Path type"),
            }

            // bounds should be Protocol(Iterator)
            assert_eq!(bounds.len(), 1);
            match &bounds[0].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Iterator");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }

    // Second predicate: T.Item = Int (associated type equality)
    let pred2 = &where_clause.predicates[1];
    match &pred2.kind {
        WherePredicateKind::Type { ty, bounds: _ } => match &ty.kind {
            TypeKind::Qualified { assoc_name, .. } => {
                assert_eq!(assoc_name.name.as_str(), "Item");
            }
            _ => panic!("Expected Qualified type"),
        },
        _ => panic!("Expected Type predicate"),
    }
}

#[test]
fn test_associated_type_with_generic() {
    // Test: where T.Item = List<Int>
    let where_clause = parse_where_clause("where T.Item = List<Int>");

    assert_eq!(where_clause.predicates.len(), 1);

    let predicate = &where_clause.predicates[0];
    match &predicate.kind {
        WherePredicateKind::Type { ty, bounds } => {
            // Check ty is T.Item
            match &ty.kind {
                TypeKind::Qualified { assoc_name, .. } => {
                    assert_eq!(assoc_name.name.as_str(), "Item");
                }
                _ => panic!("Expected Qualified type"),
            }

            // Check bounds contains List<Int>
            assert_eq!(bounds.len(), 1);
            match &bounds[0].kind {
                TypeBoundKind::Equality(concrete_ty) => {
                    match &concrete_ty.kind {
                        TypeKind::Generic { base, args } => {
                            // Base should be List
                            match &base.kind {
                                TypeKind::Path(path) => {
                                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                        assert_eq!(ident.name.as_str(), "List");
                                    }
                                }
                                _ => panic!("Expected Path for List"),
                            }

                            // Args should contain Int
                            assert_eq!(args.len(), 1);
                        }
                        _ => panic!("Expected Generic type for List<Int>"),
                    }
                }
                _ => panic!("Expected Equality bound"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }
}

#[test]
fn test_self_associated_type() {
    // Test: where Self.Item = Int
    let where_clause = parse_where_clause("where Self.Item = Int");

    assert_eq!(where_clause.predicates.len(), 1);

    let predicate = &where_clause.predicates[0];
    match &predicate.kind {
        WherePredicateKind::Type { ty, bounds } => {
            match &ty.kind {
                TypeKind::Qualified {
                    self_ty,
                    assoc_name,
                    ..
                } => {
                    // self_ty should be Self (SelfType token maps to SelfValue in PathSegment)
                    match &self_ty.kind {
                        TypeKind::Path(path) => {
                            assert_eq!(path.segments.len(), 1);
                            // PathSegment::SelfValue is used for both self and Self
                            match &path.segments[0] {
                                verum_ast::PathSegment::SelfValue => {
                                    // Success
                                }
                                _ => panic!("Expected SelfValue segment"),
                            }
                        }
                        _ => panic!("Expected Path for Self"),
                    }

                    assert_eq!(assoc_name.name.as_str(), "Item");
                }
                _ => panic!("Expected Qualified type"),
            }
        }
        _ => panic!("Expected Type predicate"),
    }
}
