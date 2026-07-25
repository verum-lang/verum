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
//! Higher-Kinded Type (HKT) Parsing Tests
//!
//! Tests that HKT syntax is properly parsed in protocols, type parameters, and constraints.
//!
//! Tests for higher-kinded type parsing: type constructors with kind parameters
//! Grammar: grammar/verum.ebnf lines 543-546

use verum_ast::decl::ProtocolItemKind;
use verum_ast::{FileId, ItemKind, TypeKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_hkt_placeholder_in_generic_args() {
    let source = r#"
protocol Functor<F<_>> {
    fn map<A, B>(self: F<A>, f: fn(A) -> B) -> F<B>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse HKT placeholder: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Functor");
            assert_eq!(
                protocol_def.generics.len(),
                1,
                "Expected one generic parameter"
            );

            // The generic parameter should be F<_> - a higher-kinded type with arity 1
            match &protocol_def.generics[0].kind {
                verum_ast::ty::GenericParamKind::HigherKinded { name, arity, .. } => {
                    assert_eq!(name.as_str(), "F", "Expected generic parameter named F");
                    assert_eq!(*arity, 1, "Expected arity 1 for F<_>");
                }
                other => panic!("Expected HigherKinded generic parameter, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_functor_protocol_with_hkt() {
    let source = r#"
protocol Functor<F<_>> {
    fn map<A, B>(self: F<A>, f: fn(A) -> B) -> F<B>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse Functor protocol: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Functor");
            assert_eq!(
                protocol_def.generics.len(),
                1,
                "Expected one generic parameter"
            );

            // Verify the protocol has a map method
            assert_eq!(protocol_def.items.len(), 1, "Expected one protocol item");

            match &protocol_def.items[0].kind {
                ProtocolItemKind::Function { decl, .. } => {
                    assert_eq!(decl.name.as_str(), "map");
                    assert_eq!(
                        decl.generics.len(),
                        2,
                        "Expected two generic parameters (A, B)"
                    );
                }
                other => panic!("Expected Function protocol item, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_monad_protocol_with_hkt() {
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
protocol Monad<M<_>> {
    fn wrap<A>(value: A) -> M<A>;
    fn flat_map<A, B>(self: M<A>, f: fn(A) -> M<B>) -> M<B>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse Monad protocol: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Monad");
            assert_eq!(
                protocol_def.generics.len(),
                1,
                "Expected one generic parameter"
            );

            // Verify the protocol has wrap and flat_map methods
            assert_eq!(protocol_def.items.len(), 2, "Expected two protocol items");

            // Check first method (wrap)
            match &protocol_def.items[0].kind {
                ProtocolItemKind::Function { decl, .. } => {
                    assert_eq!(decl.name.as_str(), "wrap");
                }
                other => panic!("Expected Function protocol item, got {:?}", other),
            }

            // Check second method (flat_map)
            match &protocol_def.items[1].kind {
                ProtocolItemKind::Function { decl, .. } => {
                    assert_eq!(decl.name.as_str(), "flat_map");
                }
                other => panic!("Expected Function protocol item, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_applicative_with_hkt_extends() {
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
protocol Applicative<F<_>> extends Functor {
    fn wrap<A>(value: A) -> F<A>;
    fn ap<A, B>(self: F<fn(A) -> B>, fa: F<A>) -> F<B>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse Applicative protocol: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Applicative");

            // Verify protocol extends Functor
            assert!(!protocol_def.bounds.is_empty(), "Expected extends clause");
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_hkt_in_associated_type() {
    let source = r#"
protocol Container {
    type Item<_>;
    fn get<T>(self: &Self, index: Int) -> Self.Item<T>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse Container protocol with HKT associated type: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.items.len(), 2, "Expected two protocol items");

            // Check associated type with HKT
            match &protocol_def.items[0].kind {
                ProtocolItemKind::Type {
                    name, type_params, ..
                } => {
                    assert_eq!(name.as_str(), "Item");
                    assert_eq!(type_params.len(), 1, "Expected one type parameter for HKT");
                }
                other => panic!("Expected Type protocol item, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_hkt_with_multiple_parameters() {
    let source = r#"
protocol Bifunctor<F<_, _>> {
    fn bimap<A, B, C, D>(
        self: F<A, B>,
        f: fn(A) -> C,
        g: fn(B) -> D
    ) -> F<C, D>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse Bifunctor with multiple HKT parameters: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Bifunctor");

            assert_eq!(protocol_def.items.len(), 1, "Expected one protocol item");

            match &protocol_def.items[0].kind {
                ProtocolItemKind::Function { decl, .. } => {
                    assert_eq!(decl.name.as_str(), "bimap");
                    assert_eq!(
                        decl.generics.len(),
                        4,
                        "Expected four generic parameters (A, B, C, D)"
                    );
                }
                other => panic!("Expected Function protocol item, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_hkt_in_impl_block() {
    let source = r#"
implement<T> Functor<List> for List<T> {
    fn map<A, B>(self: List<A>, f: fn(A) -> B) -> List<B> {
        let mut result = List.new();
        for item in self {
            result.push(f(item));
        }
        result
    }
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse impl with HKT: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Impl(impl_def) => {
            // Verify the impl block was parsed
            assert!(
                !impl_def.generics.is_empty(),
                "Expected at least one generic parameter"
            );
        }
        other => panic!("Expected Impl item, got {:?}", other),
    }
}

#[test]
fn test_hkt_constraint_in_where_clause() {
    let source = r#"
fn transform<F<_>, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>
where type F: Functor {
    F.map(fa, f)
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse function with HKT constraint: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Function(func_def) => {
            assert_eq!(func_def.name.as_str(), "transform");
            assert!(
                func_def.generics.len() >= 3,
                "Expected at least three generic parameters"
            );

            // Verify where clause exists
            assert!(
                func_def.generic_where_clause.is_some(),
                "Expected where clause"
            );
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

#[test]
fn test_nested_hkt() {
    let source = r#"
protocol Nested<F<_>, G<_>> {
    fn wrap<A>(value: A) -> F<G<A>>;
    fn unwrap<A>(nested: F<G<A>>) -> A;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse nested HKT: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Nested");
            assert_eq!(
                protocol_def.generics.len(),
                2,
                "Expected two generic parameters"
            );

            assert_eq!(protocol_def.items.len(), 2, "Expected two protocol items");
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_hkt_with_refinement() {
    let source = r#"
protocol SafeContainer<F<_>> {
    type Size: Int{> 0};
    fn create<A>(size: Size, default: A) -> F<A>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse HKT with refinement: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "SafeContainer");

            assert_eq!(protocol_def.items.len(), 2, "Expected two protocol items");

            // Check Size associated type with refinement
            match &protocol_def.items[0].kind {
                ProtocolItemKind::Type { name, bounds, .. } => {
                    assert_eq!(name.as_str(), "Size");
                    // The bounds should contain the Int{> 0} type with refinement
                    assert!(!bounds.is_empty(), "Expected type bounds");
                }
                other => panic!("Expected Type protocol item, got {:?}", other),
            }
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}

#[test]
fn test_hkt_placeholder_only() {
    // Test that _ alone in type position is parsed as inferred type
    let source = r#"
protocol Wrapper<F<_>> {
    fn identity<A>(value: F<A>) -> F<A>;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse _ placeholder: {:?}",
        result.err()
    );
}

#[test]
fn test_simple_protocol_generic() {
    // Test that regular generics still work
    let source = r#"
protocol Iterator<T> {
    fn next(self: &mut Self) -> T;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse simple protocol: {:?}",
        result.err()
    );
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::Protocol(protocol_def) => {
            assert_eq!(protocol_def.name.as_str(), "Iterator");
            assert_eq!(
                protocol_def.generics.len(),
                1,
                "Expected one generic parameter"
            );
        }
        other => panic!("Expected Protocol item, got {:?}", other),
    }
}
