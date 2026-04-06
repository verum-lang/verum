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
//! Tests for pattern matching AST nodes.
//!
//! This module tests all pattern types and their combinations,
//! including complex destructuring patterns.

use proptest::prelude::*;
use verum_ast::literal::Literal;
use verum_ast::pattern::*;
use verum_ast::span::{FileId, Span};
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_wildcard_pattern() {
    let span = test_span();
    let pattern = Pattern::wildcard(span);

    assert_eq!(pattern.kind, PatternKind::Wildcard);
    assert_eq!(pattern.span, span);
}

#[test]
fn test_rest_pattern() {
    let span = test_span();
    let pattern = Pattern::new(PatternKind::Rest, span);

    assert_eq!(pattern.kind, PatternKind::Rest);

    // Rest pattern in slice: [first, .., last]
    let slice_with_rest = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![Pattern::ident(test_ident("first"), false, span)]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![Pattern::ident(test_ident("last"), false, span)]),
        },
        span,
    );

    match &slice_with_rest.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert_eq!(before.len(), 1);
            assert!(rest.is_some());
            assert_eq!(after.len(), 1);
        }
        _ => panic!("Expected slice pattern"),
    }
}

#[test]
fn test_identifier_patterns() {
    let span = test_span();

    // Immutable binding
    let immut = Pattern::ident(test_ident("x"), false, span);
    match &immut.kind {
        PatternKind::Ident {
            mutable,
            name,
            subpattern,
            ..
        } => {
            assert!(!mutable);
            assert_eq!(name.name.as_str(), "x");
            assert!(subpattern.is_none());
        }
        _ => panic!("Expected identifier pattern"),
    }

    // Mutable binding
    let mutable = Pattern::ident(test_ident("y"), true, span);
    match &mutable.kind {
        PatternKind::Ident { mutable, name, .. } => {
            assert!(mutable);
            assert_eq!(name.name.as_str(), "y");
        }
        _ => panic!("Expected identifier pattern"),
    }

    // With subpattern: x @ Some(_)
    let with_subpattern = Pattern::new(
        PatternKind::Ident {
            by_ref: false,
            mutable: false,
            name: test_ident("x"),
            subpattern: Maybe::Some(Heap::new(Pattern::new(
                PatternKind::Variant {
                    path: Path::single(test_ident("Some")),
                    data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![
                        Pattern::wildcard(span),
                    ]))),
                },
                span,
            ))),
        },
        span,
    );

    match &with_subpattern.kind {
        PatternKind::Ident { subpattern, .. } => {
            assert!(subpattern.is_some());
        }
        _ => panic!("Expected identifier pattern with subpattern"),
    }
}

#[test]
fn test_literal_patterns() {
    let span = test_span();

    // Integer literal pattern
    let int_pat = Pattern::literal(Literal::int(42, span));
    match &int_pat.kind {
        PatternKind::Literal(lit) => {
            assert!(matches!(lit.kind, LiteralKind::Int(_)));
        }
        _ => panic!("Expected literal pattern"),
    }

    // String literal pattern
    let str_pat = Pattern::literal(Literal::string("hello".to_string().into(), span));
    match &str_pat.kind {
        PatternKind::Literal(lit) => {
            assert!(matches!(lit.kind, LiteralKind::Text(_)));
        }
        _ => panic!("Expected literal pattern"),
    }

    // Boolean literal pattern
    let bool_pat = Pattern::literal(Literal::bool(true, span));
    match &bool_pat.kind {
        PatternKind::Literal(lit) => {
            assert_eq!(lit.kind, LiteralKind::Bool(true));
        }
        _ => panic!("Expected literal pattern"),
    }
}

#[test]
fn test_tuple_patterns() {
    let span = test_span();

    // Empty tuple pattern: ()
    let empty = Pattern::new(PatternKind::Tuple(List::from(vec![])), span);
    match &empty.kind {
        PatternKind::Tuple(patterns) => {
            assert!(patterns.is_empty());
        }
        _ => panic!("Expected tuple pattern"),
    }

    // Single element: (x,)
    let single = Pattern::new(
        PatternKind::Tuple(List::from(vec![Pattern::ident(
            test_ident("x"),
            false,
            span,
        )])),
        span,
    );
    match &single.kind {
        PatternKind::Tuple(patterns) => {
            assert_eq!(patterns.len(), 1);
        }
        _ => panic!("Expected tuple pattern"),
    }

    // Multiple elements: (x, _, 42, mut y)
    let multi = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::ident(test_ident("x"), false, span),
            Pattern::wildcard(span),
            Pattern::literal(Literal::int(42, span)),
            Pattern::ident(test_ident("y"), true, span),
        ])),
        span,
    );
    match &multi.kind {
        PatternKind::Tuple(patterns) => {
            assert_eq!(patterns.len(), 4);
        }
        _ => panic!("Expected tuple pattern"),
    }
}

#[test]
fn test_slice_patterns() {
    let span = test_span();

    // Empty slice: []
    let empty = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![]),
            rest: Maybe::None,
            after: List::from(vec![]),
        },
        span,
    );
    match &empty.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert!(before.is_empty());
            assert!(rest.is_none());
            assert!(after.is_empty());
        }
        _ => panic!("Expected slice pattern"),
    }

    // Fixed size: [a, b, c]
    let fixed = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![
                Pattern::ident(test_ident("a"), false, span),
                Pattern::ident(test_ident("b"), false, span),
                Pattern::ident(test_ident("c"), false, span),
            ]),
            rest: Maybe::None,
            after: List::from(vec![]),
        },
        span,
    );
    match &fixed.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert_eq!(before.len(), 3);
            assert!(rest.is_none());
            assert!(after.is_empty());
        }
        _ => panic!("Expected slice pattern"),
    }

    // With rest at beginning: [.., last]
    let rest_beginning = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![Pattern::ident(test_ident("last"), false, span)]),
        },
        span,
    );
    match &rest_beginning.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert!(before.is_empty());
            assert!(rest.is_some());
            assert_eq!(after.len(), 1);
        }
        _ => panic!("Expected slice pattern"),
    }

    // With rest in middle: [first, .., last]
    let rest_middle = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![Pattern::ident(test_ident("first"), false, span)]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![Pattern::ident(test_ident("last"), false, span)]),
        },
        span,
    );
    match &rest_middle.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert_eq!(before.len(), 1);
            assert!(rest.is_some());
            assert_eq!(after.len(), 1);
        }
        _ => panic!("Expected slice pattern"),
    }

    // With rest at end: [first, ..]
    let rest_end = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![Pattern::ident(test_ident("first"), false, span)]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![]),
        },
        span,
    );
    match &rest_end.kind {
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            assert_eq!(before.len(), 1);
            assert!(rest.is_some());
            assert!(after.is_empty());
        }
        _ => panic!("Expected slice pattern"),
    }
}

#[test]
fn test_variant_patterns() {
    let span = test_span();

    // Unit variant: None
    let unit_variant = Pattern::new(
        PatternKind::Variant {
            path: Path::single(test_ident("None")),
            data: Maybe::None,
        },
        span,
    );
    match &unit_variant.kind {
        PatternKind::Variant { path, data } => {
            assert!(
                matches!(&path.segments[0], PathSegment::Name(ident) if ident.name.as_str() == "None")
            );
            assert!(data.is_none());
        }
        _ => panic!("Expected variant pattern"),
    }

    // Tuple variant: Some(x)
    let tuple_variant = Pattern::new(
        PatternKind::Variant {
            path: Path::single(test_ident("Some")),
            data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::ident(
                test_ident("x"),
                false,
                span,
            )]))),
        },
        span,
    );
    match &tuple_variant.kind {
        PatternKind::Variant { path, data } => {
            assert!(
                matches!(&path.segments[0], PathSegment::Name(ident) if ident.name.as_str() == "Some")
            );
            if let Maybe::Some(VariantPatternData::Tuple(patterns)) = data {
                assert_eq!(patterns.len(), 1);
            } else {
                panic!("Expected tuple variant data");
            }
        }
        _ => panic!("Expected variant pattern"),
    }

    // Record variant: Point { x, y }
    let struct_variant = Pattern::new(
        PatternKind::Variant {
            path: Path::single(test_ident("Point")),
            data: Maybe::Some(VariantPatternData::Record {
                fields: List::from(vec![
                    FieldPattern {
                        name: test_ident("x"),
                        pattern: Maybe::Some(Pattern::ident(test_ident("x"), false, span)),
                        span,
                    },
                    FieldPattern {
                        name: test_ident("y"),
                        pattern: Maybe::Some(Pattern::ident(test_ident("y"), false, span)),
                        span,
                    },
                ]),
                rest: false,
            }),
        },
        span,
    );
    match &struct_variant.kind {
        PatternKind::Variant { path, data } => {
            assert!(
                matches!(&path.segments[0], PathSegment::Name(ident) if ident.name.as_str() == "Point")
            );
            if let Maybe::Some(VariantPatternData::Record { fields, rest }) = data {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields.first().unwrap().name.name.as_str(), "x");
                assert_eq!(fields.get(1).unwrap().name.name.as_str(), "y");
                assert!(!rest);
            } else {
                panic!("Expected record variant data");
            }
        }
        _ => panic!("Expected variant pattern"),
    }
}

#[test]
fn test_or_patterns() {
    let span = test_span();

    // Simple or: 1 | 2 | 3
    let simple_or = Pattern::new(
        PatternKind::Or(List::from(vec![
            Pattern::literal(Literal::int(1, span)),
            Pattern::literal(Literal::int(2, span)),
            Pattern::literal(Literal::int(3, span)),
        ])),
        span,
    );
    match &simple_or.kind {
        PatternKind::Or(patterns) => {
            assert_eq!(patterns.len(), 3);
        }
        _ => panic!("Expected or pattern"),
    }

    // Complex or: Some(x) | None
    let complex_or =
        Pattern::new(
            PatternKind::Or(List::from(vec![
                Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(test_ident("Some")),
                        data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![
                            Pattern::ident(test_ident("x"), false, span),
                        ]))),
                    },
                    span,
                ),
                Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(test_ident("None")),
                        data: Maybe::None,
                    },
                    span,
                ),
            ])),
            span,
        );
    match &complex_or.kind {
        PatternKind::Or(patterns) => {
            assert_eq!(patterns.len(), 2);
        }
        _ => panic!("Expected or pattern"),
    }
}

#[test]
fn test_range_patterns() {
    let span = test_span();

    // Inclusive range: 1..=10
    let inclusive = Pattern::new(
        PatternKind::Range {
            start: Maybe::Some(Heap::new(Literal::int(1, span))),
            end: Maybe::Some(Heap::new(Literal::int(10, span))),
            inclusive: true,
        },
        span,
    );
    match &inclusive.kind {
        PatternKind::Range {
            start,
            end,
            inclusive,
        } => {
            assert!(start.is_some());
            assert!(end.is_some());
            assert!(*inclusive);
        }
        _ => panic!("Expected range pattern"),
    }

    // Exclusive range: 'a'..'z'
    let exclusive = Pattern::new(
        PatternKind::Range {
            start: Maybe::Some(Heap::new(Literal::char('a', span))),
            end: Maybe::Some(Heap::new(Literal::char('z', span))),
            inclusive: false,
        },
        span,
    );
    match &exclusive.kind {
        PatternKind::Range { inclusive, .. } => {
            assert!(!inclusive);
        }
        _ => panic!("Expected range pattern"),
    }

    // Half-open range: 5..
    let half_open = Pattern::new(
        PatternKind::Range {
            start: Maybe::Some(Heap::new(Literal::int(5, span))),
            end: Maybe::None,
            inclusive: false,
        },
        span,
    );
    match &half_open.kind {
        PatternKind::Range { start, end, .. } => {
            assert!(start.is_some());
            assert!(end.is_none());
        }
        _ => panic!("Expected range pattern"),
    }
}

#[test]
fn test_nested_patterns() {
    let span = test_span();

    // Nested tuple in variant: Some((x, y))
    let nested = Pattern::new(
        PatternKind::Variant {
            path: Path::single(test_ident("Some")),
            data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::new(
                PatternKind::Tuple(List::from(vec![
                    Pattern::ident(test_ident("x"), false, span),
                    Pattern::ident(test_ident("y"), false, span),
                ])),
                span,
            )]))),
        },
        span,
    );

    match &nested.kind {
        PatternKind::Variant { data, .. } => match data {
            Maybe::Some(VariantPatternData::Tuple(patterns)) => {
                assert_eq!(patterns.len(), 1);
                assert!(matches!(patterns[0].kind, PatternKind::Tuple(_)));
            }
            _ => panic!("Expected tuple variant data"),
        },
        _ => panic!("Expected variant pattern"),
    }

    // Deeply nested: Ok(Some([first, .., last]))
    let deeply_nested = Pattern::new(
        PatternKind::Variant {
            path: Path::single(test_ident("Ok")),
            data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::new(
                PatternKind::Variant {
                    path: Path::single(test_ident("Some")),
                    data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::new(
                        PatternKind::Slice {
                            before: List::from(vec![Pattern::ident(
                                test_ident("first"),
                                false,
                                span,
                            )]),
                            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
                            after: List::from(vec![Pattern::ident(
                                test_ident("last"),
                                false,
                                span,
                            )]),
                        },
                        span,
                    )]))),
                },
                span,
            )]))),
        },
        span,
    );

    // Just verify it can be constructed
    assert!(matches!(deeply_nested.kind, PatternKind::Variant { .. }));
}

#[test]
fn test_struct_patterns() {
    let span = test_span();

    // Struct with all fields: Person { name, age, address }
    let all_fields = Pattern::new(
        PatternKind::Record {
            path: Path::single(test_ident("Person")),
            fields: List::from(vec![
                FieldPattern {
                    name: test_ident("name"),
                    pattern: Maybe::Some(Pattern::ident(test_ident("n"), false, span)),
                    span,
                },
                FieldPattern {
                    name: test_ident("age"),
                    pattern: Maybe::Some(Pattern::ident(test_ident("a"), false, span)),
                    span,
                },
                FieldPattern {
                    name: test_ident("address"),
                    pattern: Maybe::Some(Pattern::wildcard(span)),
                    span,
                },
            ]),
            rest: false,
        },
        span,
    );

    match &all_fields.kind {
        PatternKind::Record { fields, rest, .. } => {
            assert_eq!(fields.len(), 3);
            assert!(!rest);
        }
        _ => panic!("Expected record pattern"),
    }

    // Struct with rest: Config { debug, .. }
    let with_rest = Pattern::new(
        PatternKind::Record {
            path: Path::single(test_ident("Config")),
            fields: List::from(vec![FieldPattern {
                name: test_ident("debug"),
                pattern: Maybe::Some(Pattern::ident(test_ident("debug"), false, span)),
                span,
            }]),
            rest: true,
        },
        span,
    );

    match &with_rest.kind {
        PatternKind::Record { fields, rest, .. } => {
            assert_eq!(fields.len(), 1);
            assert!(rest);
        }
        _ => panic!("Expected record pattern"),
    }

    // Shorthand field patterns: Point { x, y }
    let shorthand = Pattern::new(
        PatternKind::Record {
            path: Path::single(test_ident("Point")),
            fields: List::from(vec![
                FieldPattern {
                    name: test_ident("x"),
                    pattern: Maybe::None, // None means shorthand
                    span,
                },
                FieldPattern {
                    name: test_ident("y"),
                    pattern: Maybe::None, // None means shorthand
                    span,
                },
            ]),
            rest: false,
        },
        span,
    );

    match &shorthand.kind {
        PatternKind::Record { fields, .. } => {
            assert_eq!(fields.len(), 2);
            // Check that fields use shorthand syntax
            assert!(fields[0].pattern.is_none());
            assert!(fields[1].pattern.is_none());
        }
        _ => panic!("Expected record pattern"),
    }
}

#[test]
fn test_reference_patterns() {
    let span = test_span();

    // Immutable reference: &x
    let ref_pat = Pattern::new(
        PatternKind::Reference {
            mutable: false,
            inner: Heap::new(Pattern::ident(test_ident("x"), false, span)),
        },
        span,
    );

    match &ref_pat.kind {
        PatternKind::Reference { mutable, inner } => {
            assert!(!mutable);
            assert!(matches!(inner.kind, PatternKind::Ident { .. }));
        }
        _ => panic!("Expected reference pattern"),
    }

    // Mutable reference: &mut x
    let mut_ref_pat = Pattern::new(
        PatternKind::Reference {
            mutable: true,
            inner: Heap::new(Pattern::ident(test_ident("x"), false, span)),
        },
        span,
    );

    match &mut_ref_pat.kind {
        PatternKind::Reference { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected reference pattern"),
    }

    // Nested reference: &Some(x)
    let nested_ref =
        Pattern::new(
            PatternKind::Reference {
                mutable: false,
                inner: Heap::new(Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(test_ident("Some")),
                        data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![
                            Pattern::ident(test_ident("x"), false, span),
                        ]))),
                    },
                    span,
                )),
            },
            span,
        );

    match &nested_ref.kind {
        PatternKind::Reference { inner, .. } => {
            assert!(matches!(inner.kind, PatternKind::Variant { .. }));
        }
        _ => panic!("Expected reference pattern"),
    }
}

#[test]
fn test_complex_match_patterns() {
    let span = test_span();

    // Complex pattern from a real match expression
    // Result::Ok(Some((first, rest @ [_, ..]))) | Result::Err(_)
    let ok_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(
                List::from(vec![
                    PathSegment::Name(test_ident("Result")),
                    PathSegment::Name(test_ident("Ok")),
                ]),
                span,
            ),
            data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::new(
                PatternKind::Variant {
                    path: Path::single(test_ident("Some")),
                    data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![Pattern::new(
                        PatternKind::Tuple(List::from(vec![
                            Pattern::ident(test_ident("first"), false, span),
                            Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    mutable: false,
                                    name: test_ident("rest"),
                                    subpattern: Maybe::Some(Heap::new(Pattern::new(
                                        PatternKind::Slice {
                                            before: List::from(vec![Pattern::wildcard(span)]),
                                            rest: Maybe::Some(Heap::new(Pattern::new(
                                                PatternKind::Rest,
                                                span,
                                            ))),
                                            after: List::from(vec![]),
                                        },
                                        span,
                                    ))),
                                },
                                span,
                            ),
                        ])),
                        span,
                    )]))),
                },
                span,
            )]))),
        },
        span,
    );

    let err_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(
                List::from(vec![
                    PathSegment::Name(test_ident("Result")),
                    PathSegment::Name(test_ident("Err")),
                ]),
                span,
            ),
            data: Maybe::Some(VariantPatternData::Tuple(List::from(vec![
                Pattern::wildcard(span),
            ]))),
        },
        span,
    );

    let combined = Pattern::new(
        PatternKind::Or(List::from(vec![ok_pattern, err_pattern])),
        span,
    );

    match &combined.kind {
        PatternKind::Or(patterns) => {
            assert_eq!(patterns.len(), 2);
        }
        _ => panic!("Expected or pattern"),
    }
}

// Property-based tests
proptest! {
    #[test]
    fn prop_tuple_pattern_size(size in 0usize..20) {
        let span = test_span();
        let patterns: Vec<Pattern> = (0..size)
            .map(|i| Pattern::ident(test_ident(&format!("x{}", i)), false, span))
            .collect();

        let tuple_pat = Pattern::new(PatternKind::Tuple(List::from(patterns.clone())), span);

        match &tuple_pat.kind {
            PatternKind::Tuple(pats) => {
                assert_eq!(pats.len(), size);
            }
            _ => panic!("Expected tuple pattern"),
        }
    }

    #[test]
    fn prop_slice_pattern_with_rest(
        before in 0usize..5,
        after in 0usize..5,
    ) {
        let span = test_span();
        let mut patterns = Vec::new();

        // Add patterns before rest
        for i in 0..before {
            patterns.push(Pattern::ident(test_ident(&format!("b{}", i)), false, span));
        }

        // Add rest
        patterns.push(Pattern::new(PatternKind::Rest, span));

        // Add patterns after rest
        for i in 0..after {
            patterns.push(Pattern::ident(test_ident(&format!("a{}", i)), false, span));
        }

        // Convert the flat patterns vector to the structured Slice format
        let before_patterns: Vec<Pattern> = patterns[..before].to_vec();
        let after_patterns: Vec<Pattern> = patterns[before + 1..].to_vec();
        let rest_pattern = if patterns.len() > before {
            Some(Heap::new(patterns[before].clone()))
        } else {
            None
        };

        let slice_pat = Pattern::new(
            PatternKind::Slice {
                before: List::from(before_patterns.clone()),
                rest: rest_pattern.map(Maybe::Some).unwrap_or(Maybe::None),
                after: List::from(after_patterns.clone()),
            },
            span,
        );

        match &slice_pat.kind {
            PatternKind::Slice { before: b, rest: r, after: a } => {
                assert_eq!(b.len(), before);
                assert!(r.is_some());
                assert_eq!(a.len(), after);
            }
            _ => panic!("Expected slice pattern"),
        }
    }

    #[test]
    fn prop_or_pattern_alternatives(count in 2usize..10) {
        let span = test_span();
        let patterns: Vec<Pattern> = (0..count)
            .map(|i| Pattern::literal(Literal::int(i as i128, span)))
            .collect();

        let or_pat = Pattern::new(PatternKind::Or(List::from(patterns.clone())), span);

        match &or_pat.kind {
            PatternKind::Or(pats) => {
                assert_eq!(pats.len(), count);
            }
            _ => panic!("Expected or pattern"),
        }
    }
}

#[test]
fn test_pattern_span_tracking() {
    // Test that patterns correctly track their spans
    let span1 = Span::new(0, 5, FileId::new(0));
    let span2 = Span::new(5, 10, FileId::new(0));
    let span3 = Span::new(10, 15, FileId::new(0));

    let pattern = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::ident(Ident::new("a".to_string(), span1), false, span1),
            Pattern::wildcard(span2),
            Pattern::literal(Literal::int(42, span3)),
        ])),
        span3,
    );

    assert_eq!(pattern.span, span3);
    match &pattern.kind {
        PatternKind::Tuple(patterns) => {
            assert_eq!(patterns.first().unwrap().span, span1);
            assert_eq!(patterns.get(1).unwrap().span, span2);
            assert_eq!(patterns.get(2).unwrap().span, span3);
        }
        _ => panic!("Expected tuple pattern"),
    }
}
