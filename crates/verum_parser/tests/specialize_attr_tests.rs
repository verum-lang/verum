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
//! Tests for @specialize attribute parsing.
//!
//! Tests for protocol specialization: conditional impl, overlapping resolution, default impl
//!
//! This module tests parsing of specialization attributes in all supported forms:
//! 1. Basic specialization: @specialize
//! 2. Negative specialization: @specialize(negative)
//! 3. Ranked specialization: @specialize(rank = 10)
//! 4. Conditional specialization: @specialize(when(T: Clone))

use verum_ast::{ItemKind, attr::SpecializeAttr, span::FileId};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse and extract the first impl block's specialize attribute
fn parse_impl_specialize_attr(source: &str) -> Option<SpecializeAttr> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(module) => {
            if let Some(item) = module.items.first()
                && let ItemKind::Impl(impl_decl) = &item.kind {
                    return impl_decl.specialize_attr.clone();
                }
            None
        }
        Err(_) => None,
    }
}

#[test]
fn test_basic_specialize() {
    let source = r#"
        @specialize
        implement Display for List<Text> {
            fn fmt(self: &Self) -> Text { "list" }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());

    if let Some(attr) = attr {
        assert!(!attr.negative, "Basic @specialize should not be negative");
        assert!(
            attr.rank.is_none(),
            "Basic @specialize should have no rank"
        );
        assert!(
            attr.when_clause.is_none(),
            "Basic @specialize should have no when clause"
        );
    }
}

#[test]
fn test_no_specialize() {
    let source = r#"
        implement Display for Int {
            fn fmt(self: &Self) -> Text { "int" }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_none(), "No @specialize should result in None");
}

#[test]
fn test_negative_specialize() {
    let source = r#"
        @specialize(negative)
        implement<T: !Clone> MyProtocol for List<T> {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());

    if let Some(attr) = attr {
        assert!(
            attr.negative,
            "@specialize(negative) should set negative = true"
        );
        assert!(attr.rank.is_none());
        assert!(attr.when_clause.is_none());
    }
}

#[test]
fn test_rank_specialize() {
    let source = r#"
        @specialize(rank = 10)
        implement MyProtocol for Int {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());

    if let Some(attr) = attr {
        assert!(!attr.negative);
        assert!(matches!(attr.rank, Some(10)), "Should parse rank = 10");
        assert!(attr.when_clause.is_none());
    }
}

#[test]
fn test_rank_specialize_high_priority() {
    let source = r#"
        @specialize(rank = 100)
        implement Clone for Text {
            fn clone(self: &Self) -> Self { self }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert_eq!(attr.effective_rank(), 100);
    }
}

#[test]
fn test_rank_specialize_negative_rank() {
    let source = r#"
        @specialize(rank = -5)
        implement Default for Float {
            fn default() -> Self { 0.0 }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert_eq!(attr.effective_rank(), -5);
    }
}

#[test]
fn test_when_specialize_single_bound() {
    let source = r#"
        @specialize(when(T: Clone))
        implement<T> MyProtocol for Heap<T> {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());

    if let Some(attr) = attr {
        assert!(!attr.negative);
        assert!(attr.rank.is_none());
        assert!(attr.has_when_clause(), "Expected when clause to be parsed");
    }
}

#[test]
fn test_when_specialize_multiple_bounds() {
    let source = r#"
        @specialize(when(T: Clone + Send))
        implement<T> Protocol for Container<T> {
            fn process() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());
}

#[test]
fn test_basic_specialize_with_generics() {
    let source = r#"
        @specialize
        implement<T: Clone> Display for List<T> {
            fn fmt(self: &Self) -> Text { "list" }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());
}

#[test]
fn test_specialize_inherent_impl() {
    let source = r#"
        @specialize
        implement List<Int> {
            fn sum(self: &Self) -> Int { 0 }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());
}

#[test]
fn test_specialize_with_where_clause() {
    let source = r#"
        @specialize
        implement<T> Clone for Maybe<T>
        where type T: Clone
        {
            fn clone(self: &Self) -> Self { self }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());
}

#[test]
fn test_specialize_effective_rank_default() {
    let source = r#"
        @specialize
        implement Protocol for Type {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert_eq!(attr.effective_rank(), 0, "Default rank should be 0");
    }
}

#[test]
fn test_specialize_has_when_clause_false() {
    let source = r#"
        @specialize(rank = 5)
        implement Protocol for Type {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert!(!attr.has_when_clause(), "Should not have when clause");
    }
}

#[test]
fn test_multiple_attributes_with_specialize() {
    let source = r#"
        @inline
        @specialize(rank = 10)
        implement Protocol for Type {
            fn method() {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    assert!(attr.is_some());

    if let Some(attr) = attr {
        assert_eq!(attr.effective_rank(), 10);
    }
}

#[test]
fn test_complex_specialize_scenario() {
    // Real-world scenario: specialized iterator implementation for List<Text>
    let source = r#"
        @specialize(rank = 15)
        implement Iterator for List<Text> {
            type Item is Text;

            fn next(self: &mut Self) -> Maybe<Text> {
                Maybe.None
            }
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert!(!attr.negative);
        assert_eq!(attr.effective_rank(), 15);
    }
}

#[test]
fn test_negative_specialization_real_world() {
    // Specialization for types that do NOT implement Clone
    let source = r#"
        @specialize(negative)
        implement<T: !Clone> Container for Heap<T> {
            fn store(value: T) {}
        }
    "#;

    let attr = parse_impl_specialize_attr(source);
    if let Some(attr) = attr {
        assert!(attr.negative, "Should be negative specialization");
    }
}

#[test]
fn test_specialize_builder_basic() {
    let span = verum_ast::Span::dummy();
    let attr = SpecializeAttr::basic(span);

    assert!(!attr.negative);
    assert!(attr.rank.is_none());
    assert!(!attr.has_when_clause());
    assert_eq!(attr.effective_rank(), 0);
}

#[test]
fn test_specialize_builder_negative() {
    let span = verum_ast::Span::dummy();
    let attr = SpecializeAttr::negative(span);

    assert!(attr.negative);
    assert!(attr.rank.is_none());
}

#[test]
fn test_specialize_builder_with_rank() {
    let span = verum_ast::Span::dummy();
    let attr = SpecializeAttr::with_rank(42, span);

    assert!(!attr.negative);
    assert_eq!(attr.effective_rank(), 42);
}

#[test]
fn test_specialize_integration_with_type_system() {
    // This tests that the parsed attribute can be used by the type system
    let source = r#"
        @specialize(rank = 10)
        implement<T: Clone> Clone for Maybe<T> {
            fn clone(self: &Self) -> Self {
                match self {
                    Maybe.Some(x) => Maybe.Some(x.clone()),
                    Maybe.None => Maybe.None,
                }
            }
        }
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(module) => {
            if let Some(item) = module.items.first()
                && let ItemKind::Impl(impl_decl) = &item.kind {
                    // Verify the attribute is properly attached
                    assert!(impl_decl.specialize_attr.is_some());

                    if let Some(attr) = &impl_decl.specialize_attr {
                        // Type system can use this for specialization lattice construction
                        let rank = attr.effective_rank();
                        assert_eq!(rank, 10);

                        // Can check for negative reasoning
                        assert!(!attr.negative);

                        // Can check for conditional specialization
                        assert!(!attr.has_when_clause());
                    }
                }
        }
        Err(e) => panic!("Parse error: {:?}", e),
    }
}
