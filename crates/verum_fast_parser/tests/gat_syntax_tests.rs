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
//! Comprehensive GAT (Generic Associated Type) syntax parser tests
//!
//! Tests for advanced protocol features: GATs, higher-rank bounds, specialization Section 1.1-1.4
//!
//! Tests cover:
//! - Simple associated types (baseline)
//! - GAT with single type parameter
//! - GAT with multiple type parameters
//! - GAT with where clauses
//! - GAT with protocol bounds
//! - GAT with multiple bounds
//! - GAT instantiation syntax
//! - Higher-kinded types
//! - Complex nested GATs
//! - Error cases

use verum_ast::{
    ItemKind,
    decl::{ProtocolItemKind, TypeDeclBody},
    span::FileId,
};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse Verum source code
fn parse(source: &str) -> Result<List<verum_ast::Item>, String> {
    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    parser
        .parse_module(lexer, file_id)
        .map(|module| module.items)
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| format!("{:?}", e))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

#[test]
fn test_simple_associated_type() {
    // Baseline: Regular associated type (not a GAT)
    let source = r#"
        type Iterator is protocol {
            type Item;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => {
                assert_eq!(protocol_body.items.len(), 1);
                match &protocol_body.items[0].kind {
                    ProtocolItemKind::Type {
                        name,
                        type_params,
                        bounds,
                        where_clause,
                        ..
                    } => {
                        assert_eq!(name.name.as_str(), "Item");
                        assert_eq!(
                            type_params.len(),
                            0,
                            "Simple associated type should have no type params"
                        );
                        assert_eq!(bounds.len(), 0);
                        assert!(where_clause.is_none());
                    }
                    _ => panic!("Expected Type protocol item"),
                }
            }
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_single_type_parameter() {
    // GAT with one type parameter: type Item<T>;
    let source = r#"
        type LendingIterator is protocol {
            type Item<T>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1, "GAT should have 1 type parameter");
                    assert_eq!(bounds.len(), 0);
                    assert!(where_clause.is_none());
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_multiple_type_parameters() {
    // GAT with multiple type parameters: type Assoc<K, V>;
    let source = r#"
        type Collection is protocol {
            type Assoc<K, V>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Assoc");
                    assert_eq!(type_params.len(), 2, "GAT should have 2 type parameters");
                    assert_eq!(bounds.len(), 0);
                    assert!(where_clause.is_none());
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_simple_where_clause() {
    // GAT with where clause: type Item<T> where type T: Clone;
    let source = r#"
        type Container is protocol {
            type Item<T> where type T: Clone;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1);
                    assert_eq!(bounds.len(), 0);
                    assert!(where_clause.is_some(), "Should have where clause");

                    let where_clause = where_clause.as_ref().unwrap();
                    assert_eq!(where_clause.predicates.len(), 1);
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_multiple_where_clauses() {
    // GAT with multiple where clauses: type Item<T, U> where type T: Clone, type U: Debug;
    let source = r#"
        type Container is protocol {
            type Item<T, U> where type T: Clone, type U: Debug;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 2);
                    assert!(where_clause.is_some());

                    let where_clause = where_clause.as_ref().unwrap();
                    assert_eq!(
                        where_clause.predicates.len(),
                        2,
                        "Should have 2 where predicates"
                    );
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_protocol_bounds() {
    // GAT with protocol bounds: type Item<T>: Clone;
    let source = r#"
        type Container is protocol {
            type Item<T>: Clone;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1);
                    assert_eq!(bounds.len(), 1, "Should have 1 protocol bound");
                    assert!(where_clause.is_none());
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_multiple_protocol_bounds() {
    // GAT with multiple protocol bounds: type Item<T>: Clone + Debug;
    let source = r#"
        type Container is protocol {
            type Item<T>: Clone + Debug;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1);
                    assert_eq!(bounds.len(), 2, "Should have 2 protocol bounds");
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_bounds_and_where_clause() {
    // GAT with both bounds and where clause
    let source = r#"
        type Container is protocol {
            type Item<T>: Clone where type T: Debug;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1);
                    assert_eq!(bounds.len(), 1, "Should have 1 protocol bound");
                    assert!(where_clause.is_some(), "Should have where clause");
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_monad_example() {
    // Real-world example: Monad protocol with Wrapped<T> GAT
    // GAT with type parameter: `type Item<T>` in protocol definition
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        type Monad is protocol {
            type Wrapped<T>;

            fn wrap<T>(value: T) -> Self.Wrapped<T>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(protocol_body.items.len(), 2, "Should have 2 protocol items");

                    // Check the GAT
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Wrapped");
                            assert_eq!(type_params.len(), 1);
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    // Check the function using the GAT
                    match &protocol_body.items[1].kind {
                        ProtocolItemKind::Function { .. } => {
                            // Function parsed successfully
                        }
                        _ => panic!("Expected Function protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_container_example() {
    // Real-world example: Container with constrained GAT
    // GAT Monad pattern: `type Wrapped<T>` with bind/pure methods
    let source = r#"
        type Container is protocol {
            type Item<T> where type T: Clone + Debug;

            fn get<T: Clone + Debug>(self: &Self, index: usize) -> Self.Item<T>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    where_clause,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1);
                    assert!(where_clause.is_some());
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_bidirectional_iterator() {
    // Complex example: BiDirectionalIterator with multiple GATs
    // GAT Iterator pattern: `type Item<T>` with generation tracking
    let source = r#"
        type BiDirectionalIterator is protocol {
            type Item<T>;
            type Forward<T>;
            type Backward<T>;

            fn forward(&self) -> Self.Forward;
            fn backward(&self) -> Self.Backward;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(
                        protocol_body.items.len(),
                        5,
                        "Should have 3 types + 2 functions"
                    );

                    // Check all three GATs
                    for (i, expected_name) in ["Item", "Forward", "Backward"].iter().enumerate() {
                        match &protocol_body.items[i].kind {
                            ProtocolItemKind::Type {
                                name, type_params, ..
                            } => {
                                assert_eq!(name.name.as_str(), *expected_name);
                                assert_eq!(
                                    type_params.len(),
                                    1,
                                    "Each GAT should have 1 type param"
                                );
                            }
                            _ => panic!("Expected Type protocol item at index {}", i),
                        }
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_three_type_parameters() {
    // GAT with three type parameters
    let source = r#"
        type TripleAssoc is protocol {
            type Triplet<A, B, C>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                ProtocolItemKind::Type {
                    name, type_params, ..
                } => {
                    assert_eq!(name.name.as_str(), "Triplet");
                    assert_eq!(type_params.len(), 3, "Should have 3 type parameters");
                }
                _ => panic!("Expected Type protocol item"),
            },
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_bounded_type_parameter() {
    // GAT where the type parameter itself has bounds
    let source = r#"
        type Container is protocol {
            type Item<T: Clone>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Item");
                            assert_eq!(type_params.len(), 1);

                            // Check that the type parameter has bounds
                            use verum_ast::ty::GenericParamKind;
                            match &type_params[0].kind {
                                GenericParamKind::Type { name, bounds, .. } => {
                                    assert_eq!(name.name.as_str(), "T");
                                    assert_eq!(
                                        bounds.len(),
                                        1,
                                        "Type parameter should have 1 bound"
                                    );
                                }
                                _ => panic!("Expected Type generic param"),
                            }
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_with_multiple_bounded_type_parameters() {
    // GAT where multiple type parameters have bounds
    let source = r#"
        type Graph is protocol {
            type Edge<N: Node, E: Edge>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 2);

                            // Both type parameters should have bounds
                            use verum_ast::ty::GenericParamKind;
                            for param in type_params.iter() {
                                match &param.kind {
                                    GenericParamKind::Type { bounds, .. } => {
                                        assert_eq!(bounds.len(), 1);
                                    }
                                    _ => panic!("Expected Type generic param"),
                                }
                            }
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_multiple_gats_in_one_protocol() {
    // Protocol with multiple GATs
    let source = r#"
        type MultiGAT is protocol {
            type First<T>;
            type Second<U>;
            type Third<V, W>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(protocol_body.items.len(), 3, "Should have 3 GATs");

                    // Check each GAT
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 1);
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    match &protocol_body.items[1].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 1);
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    match &protocol_body.items[2].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 2);
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_mixed_with_regular_associated_type() {
    // Protocol with both regular associated types and GATs
    let source = r#"
        type Mixed is protocol {
            type Regular;
            type Generic<T>;
        };
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(protocol_body.items.len(), 2);

                    // First should be regular (no type params)
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 0, "Regular associated type");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    // Second should be GAT (with type params)
                    match &protocol_body.items[1].kind {
                        ProtocolItemKind::Type { type_params, .. } => {
                            assert_eq!(type_params.len(), 1, "GAT with one parameter");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

// Error case tests

#[test]
fn test_gat_error_missing_semicolon() {
    // Missing semicolon should fail
    let source = r#"
        type Container is protocol {
            type Item<T>
        }
    "#;

    let result = parse(source);
    assert!(result.is_err(), "Should fail without semicolon");
}

#[test]
fn test_gat_error_empty_type_params() {
    // Empty type parameter list should fail
    let source = r#"
        type Container is protocol {
            type Item<>;
        }
    "#;

    let result = parse(source);
    assert!(
        result.is_err(),
        "Should fail with empty type parameter list"
    );
}

#[test]
fn test_gat_complex_real_world_example() {
    // Complex real-world example combining multiple features
    // Advanced GAT: higher-kinded type parameter with kind constraint
    let source = r#"
        type LinearIterator is protocol {
            type Item<T> where type T: Linear;

            fn next_linear<T: Linear>(self: Self) -> (Self, Maybe<Self.Item<T>>);
        };
    "#;

    let result = parse(source);
    assert!(
        result.is_ok(),
        "Failed to parse complex example: {:?}",
        result.err()
    );

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(protocol_body.items.len(), 2);

                    // Check the GAT with where clause
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name,
                            type_params,
                            where_clause,
                            ..
                        } => {
                            assert_eq!(name.name.as_str(), "Item");
                            assert_eq!(type_params.len(), 1);
                            assert!(where_clause.is_some(), "Should have where clause");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol type body"),
            }
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_gat_functor_example() {
    // Higher-kinded type example: Functor
    // GAT lending iterator: borrowing from container via generation refs
    let source = r#"
        type Functor is protocol {
            type F<T>;

            fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>;
        };
    "#;

    let result = parse(source);
    assert!(
        result.is_ok(),
        "Failed to parse Functor: {:?}",
        result.err()
    );

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Type(type_decl) => match &type_decl.body {
            TypeDeclBody::Protocol(protocol_body) => {
                assert_eq!(protocol_body.items.len(), 2);

                match &protocol_body.items[0].kind {
                    ProtocolItemKind::Type {
                        name, type_params, ..
                    } => {
                        assert_eq!(name.name.as_str(), "F");
                        assert_eq!(type_params.len(), 1);
                    }
                    _ => panic!("Expected Type protocol item"),
                }
            }
            _ => panic!("Expected Protocol type body"),
        },
        _ => panic!("Expected Type item"),
    }
}
