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
//! GAT Integration Example - Complete End-to-End Test
//!
//! This test demonstrates parsing a complete Verum program with multiple
//! GATs showcasing all supported syntax variations.

use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_complete_gat_program() {
    // Complete Verum program showcasing GAT syntax
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        // Example 1: Monad Protocol with GAT
        type Monad is protocol {
            type Wrapped<T>;

            fn wrap<T>(value: T) -> Self.Wrapped<T>;
            fn bind<A, B>(self: Self.Wrapped<A>, f: fn(A) -> Self.Wrapped<B>) -> Self.Wrapped<B>;
        };

        // Example 2: Container with Constrained GAT
        type Container is protocol {
            type Item<T> where type T: Clone + Debug;

            fn get<T: Clone + Debug>(self: &Self, index: usize) -> Maybe<Self.Item<T>>;
            fn set<T: Clone + Debug>(self: &mut Self, index: usize, value: Self.Item<T>);
        };

        // Example 3: Graph with Multiple GATs
        type Graph is protocol {
            type Node;
            type Edge<N>;
            type Path<N, E>;

            fn add_node(self: &mut Self, node: Self.Node);
            fn add_edge<N>(self: &mut Self, from: N, to: N) -> Self.Edge<N>;
        };

        // Example 4: Functor (Higher-Kinded Type)
        type Functor is protocol {
            type F<T>;

            fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>;
        };

        // Example 5: StreamingIterator with Regular Associated Type
        type StreamingIterator is protocol {
            type Item;

            fn get(&self) -> Maybe<&Self.Item>;
            fn advance(&mut self);
        };

        // Example 6: Complex GAT with Multiple Bounds
        type Serializable is protocol {
            type Encoded<T>: Clone + Debug where type T: Serialize + Clone;

            fn encode<T: Serialize + Clone>(value: T) -> Self.Encoded<T>;
            fn decode<T: Serialize + Clone>(encoded: Self.Encoded<T>) -> Maybe<T>;
        };

        // Example 7: BiDirectional with Multiple GATs
        type BiDirectionalIterator is protocol {
            type Item<T>;
            type Forward<T>;
            type Backward<T>;

            fn forward<T>(&self) -> Self.Forward<T>;
            fn backward<T>(&self) -> Self.Backward<T>;
        };

        // Example 8: Async with GAT
        type AsyncIterator is protocol {
            type Item<T>;

            async fn next<T>(&mut self) -> Maybe<Self.Item<T>>;
        };
    "#;

    // Parse the source
    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let result = parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join("\n")
    });

    // Verify parsing succeeded
    assert!(
        result.is_ok(),
        "Failed to parse complete GAT program: {:?}",
        result.err()
    );

    let module = result.unwrap();
    let items = module.items;

    // Verify we got all 8 protocol definitions
    assert_eq!(items.len(), 8, "Should have 8 protocol definitions");

    // Verify each protocol has the expected structure
    use verum_ast::{
        ItemKind,
        decl::{ProtocolItemKind, TypeDeclBody},
    };

    // Check Monad protocol
    match &items[0].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "Monad");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    assert_eq!(protocol_body.items.len(), 3); // 1 type + 2 functions

                    // First item should be the Wrapped GAT
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Wrapped");
                            assert_eq!(
                                type_params.len(),
                                1,
                                "Wrapped should be a GAT with 1 parameter"
                            );
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check Container protocol
    match &items[1].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "Container");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    // First item should be the Item GAT with where clause
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name,
                            type_params,
                            where_clause,
                            ..
                        } => {
                            assert_eq!(name.name.as_str(), "Item");
                            assert_eq!(type_params.len(), 1);
                            assert!(where_clause.is_some(), "Item should have where clause");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check Graph protocol (multiple GATs)
    match &items[2].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "Graph");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    // Should have 3 types + 2 functions = 5 items
                    assert_eq!(protocol_body.items.len(), 5);

                    // Check Node (regular associated type)
                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Node");
                            assert_eq!(type_params.len(), 0, "Node is not a GAT");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    // Check Edge (GAT with 1 param)
                    match &protocol_body.items[1].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Edge");
                            assert_eq!(type_params.len(), 1, "Edge is a GAT");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    // Check Path (GAT with 2 params)
                    match &protocol_body.items[2].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Path");
                            assert_eq!(type_params.len(), 2, "Path is a GAT with 2 params");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }
                }
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check Functor protocol (higher-kinded type)
    match &items[3].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "Functor");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                    ProtocolItemKind::Type {
                        name, type_params, ..
                    } => {
                        assert_eq!(name.name.as_str(), "F");
                        assert_eq!(type_params.len(), 1, "F is a type constructor");
                    }
                    _ => panic!("Expected Type protocol item"),
                },
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check StreamingIterator (regular associated type for comparison)
    match &items[4].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "StreamingIterator");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                    ProtocolItemKind::Type {
                        name, type_params, ..
                    } => {
                        assert_eq!(name.name.as_str(), "Item");
                        assert_eq!(type_params.len(), 0, "Item is NOT a GAT here");
                    }
                    _ => panic!("Expected Type protocol item"),
                },
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check Serializable (GAT with bounds and where clause)
    match &items[5].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "Serializable");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => match &protocol_body.items[0].kind {
                    ProtocolItemKind::Type {
                        name,
                        type_params,
                        bounds,
                        where_clause,
                        ..
                    } => {
                        assert_eq!(name.name.as_str(), "Encoded");
                        assert_eq!(type_params.len(), 1);
                        assert_eq!(bounds.len(), 2, "Should have Clone + Debug bounds");
                        assert!(where_clause.is_some(), "Should have where clause");
                    }
                    _ => panic!("Expected Type protocol item"),
                },
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check BiDirectionalIterator (multiple GATs)
    match &items[6].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "BiDirectionalIterator");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    // 3 types + 2 functions = 5 items
                    assert_eq!(protocol_body.items.len(), 5);

                    // All three should be GATs
                    for i in 0..3 {
                        match &protocol_body.items[i].kind {
                            ProtocolItemKind::Type { type_params, .. } => {
                                assert_eq!(type_params.len(), 1, "All should be GATs");
                            }
                            _ => panic!("Expected Type protocol item"),
                        }
                    }
                }
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    // Check AsyncIterator (async with GAT)
    match &items[7].kind {
        ItemKind::Type(type_decl) => {
            assert_eq!(type_decl.name.name.as_str(), "AsyncIterator");
            match &type_decl.body {
                TypeDeclBody::Protocol(protocol_body) => {
                    // 1 type + 1 async function = 2 items
                    assert_eq!(protocol_body.items.len(), 2);

                    match &protocol_body.items[0].kind {
                        ProtocolItemKind::Type {
                            name, type_params, ..
                        } => {
                            assert_eq!(name.name.as_str(), "Item");
                            assert_eq!(type_params.len(), 1, "Item is a GAT");
                        }
                        _ => panic!("Expected Type protocol item"),
                    }

                    match &protocol_body.items[1].kind {
                        ProtocolItemKind::Function {
                            decl,
                            default_impl: _,
                        } => {
                            assert!(decl.is_async, "next should be async");
                        }
                        _ => panic!("Expected Function protocol item"),
                    }
                }
                _ => panic!("Expected Protocol body"),
            }
        }
        _ => panic!("Expected Type item"),
    }

    println!("✅ All GAT syntax variations parsed successfully!");
}

#[test]
fn test_gat_implementation_example() {
    // Test GAT implementation syntax
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        type Monad is protocol {
            type Wrapped<T>;

            fn wrap<T>(value: T) -> Self.Wrapped<T>;
        };

        type Maybe<T> is Some(T) | None;

        implement Monad for Maybe {
            type Wrapped<T> is Maybe<T>;

            fn wrap<T>(value: T) -> Maybe<T> {
                Some(value)
            }
        }
    "#;

    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let result = parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join("\n")
    });

    assert!(
        result.is_ok(),
        "Failed to parse GAT implementation: {:?}",
        result.err()
    );

    let module = result.unwrap();
    let items = module.items;
    assert_eq!(items.len(), 3); // Protocol + Type + Impl

    println!("✅ GAT implementation syntax parsed successfully!");
}

#[test]
fn test_gat_usage_in_functions() {
    // Test using GATs in function signatures
    let source = r#"
        type Container is protocol {
            type Item<T>;
        };

        fn process<C: Container>(container: C, index: usize) -> C.Item<Int> {
            // Function body would go here
        }

        fn transform<C: Container, T, U>(
            container: C,
            from: C.Item<T>,
            mapper: fn(T) -> U
        ) -> C.Item<U> {
            // Function body would go here
        }
    "#;

    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let result = parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join("\n")
    });

    assert!(
        result.is_ok(),
        "Failed to parse GAT usage in functions: {:?}",
        result.err()
    );

    println!("✅ GAT usage in function signatures parsed successfully!");
}
