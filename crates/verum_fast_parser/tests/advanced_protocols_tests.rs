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
//! Comprehensive tests for advanced protocol parsing
//!
//! Tests for advanced protocol features: GATs, higher-rank bounds, specialization
//!
//! Tests:
//! - Generic Associated Types (GATs)
//! - Higher-kinded types
//! - GenRef wrapper types
//! - Specialization with @specialize
//! - Negative protocol bounds
//! - Refinement types in protocol methods

use verum_ast::{
    FileId, TypeKind,
    decl::{ImplKind, ProtocolItemKind},
    ty::TypeBoundKind,
};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).expect("Parse failed")
}

// ==================== GAT Tests ====================

#[test]
fn test_simple_gat_declaration() {
    let source = r#"
        type Iterator is protocol {
            type Item<T>;
            fn next(&mut self) -> Maybe<Self.Item>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind {
        if let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            assert_eq!(protocol_body.items.len(), 2);

            // Check GAT declaration
            if let ProtocolItemKind::Type {
                name, type_params, ..
            } = &protocol_body.items[0].kind
            {
                assert_eq!(name.name.as_str(), "Item");
                assert_eq!(type_params.len(), 1); // Has one type parameter
            } else {
                panic!("Expected Type protocol item");
            }
        } else {
            panic!("Expected Protocol body");
        }
    } else {
        panic!("Expected Type declaration");
    }
}

#[test]
fn test_gat_with_bounds() {
    let source = r#"
        type Container is protocol {
            type Item<T>: Clone + Debug;
            fn get(&self, index: usize) -> Maybe<Self.Item<T>>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind {
        if let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            if let ProtocolItemKind::Type {
                name,
                type_params,
                bounds,
                ..
            } = &protocol_body.items[0].kind
            {
                assert_eq!(name.name.as_str(), "Item");
                assert_eq!(type_params.len(), 1);
                assert_eq!(bounds.len(), 2); // Clone + Debug
            } else {
                panic!("Expected Type protocol item");
            }
        } else {
            panic!("Expected Protocol body");
        }
    }
}

#[test]
fn test_gat_with_where_clause() {
    let source = r#"
        type Container is protocol {
            type Item<T> where type T: Clone;
            fn get(&self) -> Self.Item<T>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            if let ProtocolItemKind::Type {
                name,
                type_params,
                where_clause,
                ..
            } = &protocol_body.items[0].kind
            {
                assert_eq!(name.name.as_str(), "Item");
                assert_eq!(type_params.len(), 1);
                assert!(where_clause.is_some());
            } else {
                panic!("Expected Type protocol item");
            }
        }
}

#[test]
fn test_monad_protocol_with_gat() {
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        type Monad is protocol {
            type Wrapped<T>;

            fn wrap<T>(value: T) -> Self.Wrapped<T>;
            fn bind<T, U>(
                self: Self.Wrapped<T>,
                f: fn(T) -> Self.Wrapped<U>
            ) -> Self.Wrapped<U>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind {
        assert_eq!(type_decl.name.name.as_str(), "Monad");

        if let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            assert_eq!(protocol_body.items.len(), 3); // 1 GAT + 2 methods

            // Check GAT
            if let ProtocolItemKind::Type {
                name, type_params, ..
            } = &protocol_body.items[0].kind
            {
                assert_eq!(name.name.as_str(), "Wrapped");
                assert_eq!(type_params.len(), 1);
            }
        }
    }
}

// ==================== GenRef Tests ====================

#[test]
fn test_genref_type_parsing() {
    let source = r#"
        type WindowIterator<T> is {
            data: GenRef<List<T>>,
            window_size: usize,
            position: usize
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Record(fields) = &type_decl.body {
            // Check first field is GenRef
            assert_eq!(fields[0].name.name.as_str(), "data");

            if let TypeKind::GenRef { inner } = &fields[0].ty.kind {
                // Inner should be List<T>
                if let TypeKind::Generic { .. } = &inner.kind {
                    // Success - we have GenRef<List<T>>
                } else {
                    panic!("Expected Generic type inside GenRef");
                }
            } else {
                panic!("Expected GenRef type for data field");
            }
        }
}

#[test]
fn test_genref_in_protocol_return_type() {
    let source = r#"
        type LendingIterator is protocol {
            type Item;

            fn next(&mut self) -> Maybe<GenRef<Self.Item>>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            // Check method return type contains GenRef
            if let ProtocolItemKind::Function { decl, .. } = &protocol_body.items[1].kind
                && let Some(ret_ty) = &decl.return_type {
                    // Should have Maybe<GenRef<...>>
                    if let TypeKind::Generic { base, .. } = &ret_ty.kind {
                        // Success
                    }
                }
        }
}

// ==================== Higher-Kinded Types Tests ====================

#[test]
fn test_higher_kinded_type_placeholder() {
    let source = r#"
        type Functor is protocol {
            type F<_>;

            fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            // Check F<_> type constructor
            if let ProtocolItemKind::Type {
                name, type_params, ..
            } = &protocol_body.items[0].kind
            {
                assert_eq!(name.name.as_str(), "F");
                assert_eq!(type_params.len(), 1); // One placeholder
            }
        }
}

#[test]
fn test_applicative_with_hkt() {
    // Use 'X' as placeholder type parameter instead of '_' which isn't valid
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        type Applicative is protocol {
            type F<X>;

            fn wrap<T>(value: T) -> Self.F<T>;
            fn apply<A, B>(
                self: Self.F<fn(A) -> B>,
                arg: Self.F<A>
            ) -> Self.F<B>;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind {
        assert_eq!(type_decl.name.name.as_str(), "Applicative");

        if let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            assert_eq!(protocol_body.items.len(), 3); // F<_> + 2 methods
        }
    }
}

// ==================== Specialization Tests ====================

#[test]
fn test_specialize_attribute() {
    let source = r#"
        @specialize
        implement Display for List<Text> {
            fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
                // Optimized implementation
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[0].kind {
        assert!(
            impl_decl.specialize_attr.is_some(),
            "Should be marked as specialized"
        );
    } else {
        panic!("Expected Impl declaration");
    }
}

#[test]
fn test_non_specialized_impl() {
    let source = r#"
        implement<T> Display for List<T> where type T: Display {
            fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
                // Generic implementation
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[0].kind {
        assert!(
            impl_decl.specialize_attr.is_none(),
            "Should not be specialized"
        );
        assert_eq!(impl_decl.generics.len(), 1); // Has <T>
    } else {
        panic!("Expected Impl declaration");
    }
}

#[test]
fn test_specialized_with_constraints() {
    let source = r#"
        @specialize
        implement<T> Clone for Maybe<T> where type T: Copy {
            fn clone(self: &Self) -> Self {
                // Can just copy bits for Copy types
                *self
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[0].kind {
        assert!(impl_decl.specialize_attr.is_some());
        assert!(impl_decl.generic_where_clause.is_some());
    }
}

// ==================== Negative Bounds Tests ====================

#[test]
fn test_negative_bound_parsing() {
    let source = r#"
        implement<T> MyProtocol for T where type T: Send + !Sync {
            fn method(self: &Self) {
                // Implementation for Send but not Sync
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[0].kind
        && let Some(where_clause) = &impl_decl.generic_where_clause {
            // Check for negative bound
            for predicate in where_clause.predicates.iter() {
                if let verum_ast::WherePredicateKind::Type { bounds, .. } = &predicate.kind {
                    // Should have both Send and !Sync
                    let has_negative = bounds
                        .iter()
                        .any(|b| matches!(b.kind, TypeBoundKind::NegativeProtocol(_)));
                    assert!(has_negative, "Should have negative bound");
                }
            }
        }
}

#[test]
fn test_multiple_negative_bounds() {
    let source = r#"
        fn process<T>(value: T) where type T: Clone + !Copy + !Send {
            // T can be cloned but not copied or sent across threads
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Function(func_decl) = &module.items[0].kind
        && let Some(where_clause) = &func_decl.generic_where_clause {
            for predicate in where_clause.predicates.iter() {
                if let verum_ast::WherePredicateKind::Type { bounds, .. } = &predicate.kind {
                    let negative_count = bounds
                        .iter()
                        .filter(|b| matches!(b.kind, TypeBoundKind::NegativeProtocol(_)))
                        .count();
                    assert_eq!(
                        negative_count, 2,
                        "Should have 2 negative bounds (!Copy, !Send)"
                    );
                }
            }
        }
}

// ==================== Refinement in Protocol Methods ====================

#[test]
fn test_protocol_method_with_inline_refinement() {
    let source = r#"
        type NumericOps is protocol {
            fn divide(self: &Self, x: Int, y: Int{!= 0}) -> Float;
            fn abs(self: &Self, x: Int) -> Int{>= 0};
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            assert_eq!(protocol_body.items.len(), 2);

            // Check divide has refinement on y parameter
            if let ProtocolItemKind::Function { decl, .. } = &protocol_body.items[0].kind {
                assert_eq!(decl.params.len(), 3); // self, x, y
            }
        }
}

#[test]
fn test_protocol_method_with_sigma_refinement() {
    let source = r#"
        type RangeOps is protocol {
            fn get_bounded(self: &Self, idx: usize) -> result: Int where result >= 0 && result < 100;
        };
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 1);

    if let verum_ast::ItemKind::Type(type_decl) = &module.items[0].kind
        && let verum_ast::TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            // Method should parse successfully with sigma-type refinement
            if let ProtocolItemKind::Function { decl, .. } = &protocol_body.items[0].kind {
                assert_eq!(decl.name.name.as_str(), "get_bounded");
            }
        }
}

// ==================== Complex Integration Tests ====================

#[test]
fn test_lending_iterator_full_protocol() {
    let source = r#"
        type LendingIterator is protocol {
            type Item;

            fn next(&mut self) -> Maybe<GenRef<Self.Item>>;
            fn get(&self) -> Maybe<&Self.Item>;
            fn advance(&mut self);
        };

        implement<T> LendingIterator for WindowIterator<T> {
            type Item is [T];

            fn next(&mut self) -> Maybe<GenRef<&[T]>> {
                let data = self.data.deref();
                if self.position + self.window_size <= data.len() {
                    let slice = &data[self.position..self.position + self.window_size];
                    self.position += 1;
                    Some(GenRef.borrow(slice))
                } else {
                    None
                }
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 2); // Protocol + Impl

    // Check protocol
    if let verum_ast::ItemKind::Type(protocol) = &module.items[0].kind {
        assert_eq!(protocol.name.name.as_str(), "LendingIterator");
    }

    // Check implementation
    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[1].kind
        && let ImplKind::Protocol { protocol, .. } = &impl_decl.kind {
            assert_eq!(protocol.segments.len(), 1);
        }
}

#[test]
fn test_functor_with_maybe_impl() {
    let source = r#"
        type Functor is protocol {
            type F<_>;

            fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>;
        };

        implement Functor for MaybeFunctor {
            type F<T> is Maybe<T>;

            fn map<A, B>(self: Maybe<A>, f: fn(A) -> B) -> Maybe<B> {
                match self {
                    Some(x) => Some(f(x)),
                    None => None
                }
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 2);
}

#[test]
fn test_specialization_lattice_example() {
    let source = r#"
        // General implementation
        implement<T> Display for List<T> where type T: Display {
            fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
                // Generic
            }
        }

        // More specific
        @specialize
        implement<T> Display for List<T> where type T: Copy + Display {
            fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
                // Optimized for Copy types
            }
        }

        // Most specific
        @specialize
        implement Display for List<Text> {
            fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
                // Highly optimized for Text
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 3);

    // Check specialization markers
    let specialized_count = module
        .items
        .iter()
        .filter(|item| {
            if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                impl_decl.specialize_attr.is_some()
            } else {
                false
            }
        })
        .count();

    assert_eq!(specialized_count, 2, "Should have 2 specialized impls");
}

// ==================== HKT Protocol Arguments Tests ====================

#[test]
fn test_hkt_protocol_args_preserved() {
    // Test that HKT protocol arguments like Functor<List> are preserved in the AST
    let source = r#"
        type Functor<F> is protocol {
            fn map<A, B>(self: F<A>, f: fn(A) -> B) -> F<B>;
        };

        implement Functor<List> for ListFunctor {
            fn map<A, B>(self: List<A>, f: fn(A) -> B) -> List<B> {
                // Implementation
                self
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 2); // Protocol + Impl

    // Check implementation preserves HKT arguments
    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[1].kind {
        if let ImplKind::Protocol {
            protocol,
            protocol_args,
            for_type,
        } = &impl_decl.kind
        {
            // Verify protocol name
            assert_eq!(protocol.segments.len(), 1);
            if let verum_ast::ty::PathSegment::Name(name) = &protocol.segments[0] {
                assert_eq!(name.name.as_str(), "Functor");
            }

            // Verify protocol_args contains List
            assert_eq!(
                protocol_args.len(),
                1,
                "Should have one type constructor argument"
            );
            if let verum_ast::ty::GenericArg::Type(ty) = &protocol_args[0] {
                if let TypeKind::Path(path) = &ty.kind
                    && let verum_ast::ty::PathSegment::Name(name) = &path.segments[0] {
                        assert_eq!(name.name.as_str(), "List", "Protocol arg should be List");
                    }
            } else {
                panic!("Expected Type argument, got {:?}", protocol_args[0]);
            }

            // Verify for_type
            if let TypeKind::Path(path) = &for_type.kind
                && let verum_ast::ty::PathSegment::Name(name) = &path.segments[0] {
                    assert_eq!(name.name.as_str(), "ListFunctor");
                }
        } else {
            panic!("Expected Protocol impl kind");
        }
    } else {
        panic!("Expected Impl item");
    }
}

#[test]
fn test_simple_protocol_no_args() {
    // Test that simple protocols without HKT arguments have empty protocol_args
    let source = r#"
        type Show is protocol {
            fn show(self: Self) -> Text;
        };

        implement Show for Int {
            fn show(self: Int) -> Text {
                "42"
            }
        }
    "#;

    let module = parse_module(source);
    assert_eq!(module.items.len(), 2); // Protocol + Impl

    // Check implementation has empty protocol_args
    if let verum_ast::ItemKind::Impl(impl_decl) = &module.items[1].kind {
        if let ImplKind::Protocol {
            protocol,
            protocol_args,
            ..
        } = &impl_decl.kind
        {
            assert_eq!(
                protocol_args.len(),
                0,
                "Simple protocol should have no type args"
            );
        } else {
            panic!("Expected Protocol impl kind");
        }
    }
}
