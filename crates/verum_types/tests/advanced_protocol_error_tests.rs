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
//! Comprehensive Test Suite for Advanced Protocol Error Messages
//!
//! Higher-kinded type (HKT) kind inference: infers kinds for type constructors
//! (e.g., List has kind Type -> Type, Map has kind Type -> Type -> Type).
//! Uses constraint-based kind inference with unification.
//!
//! This test suite validates:
//! - Error message formatting and clarity
//! - Diagnostic generation with labels and notes
//! - Help message generation
//! - Code example suggestions
//! - Integration with the diagnostic system

use verum_ast::span::Span;
use verum_common::span::FileId;
use verum_common::{List, Map};
use verum_types::{
    CandidateInfo, GATArityError, GATWhereClauseError, GenerationMismatchError,
    NegativeSpecializationError, ProtocolBound, SpecializationAmbiguityError, Type, WhereClause,
};

// ==================== Test Utilities ====================

fn dummy_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

fn dummy_span_at(start: u32, end: u32) -> Span {
    Span::new(start, end, FileId::new(0))
}

fn make_named_type(name: &str) -> Type {
    use verum_ast::ty::{Ident, Path};
    let ident = Ident::new(name, Span::default());
    Type::Named {
        path: Path::single(ident),
        args: List::new(),
    }
}

// ==================== GAT Arity Error Tests ====================

#[test]
fn test_gat_arity_error_zero_to_one() {
    let error = GATArityError::new("Item", 1, 0, dummy_span());

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert_eq!(diag.code(), Some("E0307"));
    assert!(diag.message().contains("Item"));
    assert!(diag.message().contains("1 type parameter"));
    assert!(diag.message().contains("0 were provided"));
}

#[test]
fn test_gat_arity_error_one_to_zero() {
    let error = GATArityError::new("Simple", 0, 1, dummy_span());

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert!(diag.message().contains("0 type parameter"));
    assert!(diag.message().contains("1 were provided"));
}

#[test]
fn test_gat_arity_error_with_protocol() {
    let error = GATArityError::new("Item", 1, 0, dummy_span()).with_protocol("Iterator");

    let diag = error.to_diagnostic();

    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();
    assert!(notes.contains("Iterator"));
}

#[test]
fn test_gat_arity_error_with_params() {
    let params = List::from_iter(vec!["T".into(), "E".into()]);
    let error = GATArityError::new("Result", 2, 0, dummy_span()).with_expected_params(params);

    let diag = error.to_diagnostic();
    let helps: String = diag.helps().iter().map(|h| h.to_string()).collect();

    assert!(helps.contains("Result<T, E>"));
}

#[test]
fn test_gat_arity_error_has_help_messages() {
    let error = GATArityError::new("Item", 1, 0, dummy_span());
    let diag = error.to_diagnostic();

    assert!(!diag.helps().is_empty());
    let help_text: String = diag.helps().iter().map(|h| h.to_string()).collect();
    assert!(help_text.contains("Add") || help_text.contains("Change"));
}

#[test]
fn test_gat_arity_error_too_many_args() {
    let error = GATArityError::new("Simple", 1, 3, dummy_span());
    let diag = error.to_diagnostic();

    let help_text: String = diag.helps().iter().map(|h| h.to_string()).collect();
    assert!(help_text.contains("Remove"));
}

#[test]
fn test_gat_arity_correct_usage_generation() {
    let error1 = GATArityError::new("Item", 0, 1, dummy_span());
    assert_eq!(error1.correct_usage(), "Item");

    let error2 = GATArityError::new("Item", 1, 0, dummy_span());
    assert!(error2.correct_usage().contains("<"));

    let error3 = GATArityError::new("Result", 2, 0, dummy_span())
        .with_expected_params(List::from_iter(vec!["T".into(), "E".into()]));
    assert_eq!(error3.correct_usage(), "Result<T, E>");
}

// ==================== Specialization Ambiguity Error Tests ====================

#[test]
fn test_specialization_ambiguity_basic() {
    let candidates = List::from_iter(vec![1, 2]);
    let error = SpecializationAmbiguityError::new(
        "Display",
        make_named_type("List<Int>"),
        candidates,
        dummy_span(),
    );

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert_eq!(diag.code(), Some("E0308"));
    assert!(diag.message().contains("Ambiguous specialization"));
    assert!(diag.message().contains("Display"));
}

#[test]
fn test_specialization_ambiguity_with_details() {
    let details = List::from_iter(vec![
        CandidateInfo {
            impl_id: 1,
            span: None,
            signature: "implement<T: Clone> Display for List<T>".into(),
            bounds: List::from_iter(vec!["T: Clone".into()]),
        },
        CandidateInfo {
            impl_id: 2,
            span: None,
            signature: "implement<T: Send> Display for List<T>".into(),
            bounds: List::from_iter(vec!["T: Send".into()]),
        },
    ]);

    let error = SpecializationAmbiguityError::new(
        "Display",
        make_named_type("List<SomeType>"),
        List::from_iter(vec![1, 2]),
        dummy_span(),
    )
    .with_candidate_details(details);

    let diag = error.to_diagnostic();

    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();
    assert!(notes.contains("Clone"));
    assert!(notes.contains("Send"));
}

#[test]
fn test_specialization_ambiguity_help_has_rank() {
    let error = SpecializationAmbiguityError::new(
        "Protocol",
        make_named_type("Type"),
        List::from_iter(vec![1, 2]),
        dummy_span(),
    );

    let diag = error.to_diagnostic();
    let helps: String = diag.helps().iter().map(|h| h.to_string()).collect();

    assert!(helps.contains("rank"));
    assert!(helps.contains("@specialize"));
}

#[test]
fn test_specialization_ambiguity_candidate_count() {
    let error = SpecializationAmbiguityError::new(
        "Protocol",
        make_named_type("Type"),
        List::from_iter(vec![1, 2, 3]),
        dummy_span(),
    );

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("3"));
}

// ==================== GenRef Generation Mismatch Error Tests ====================

#[test]
fn test_genref_generation_mismatch_basic() {
    let error = GenerationMismatchError::new(42, 43, 0x7ffee4c0a000, dummy_span());

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert_eq!(diag.code(), Some("E0309"));
    assert!(diag.message().contains("generation mismatch"));
}

#[test]
fn test_genref_generation_values_in_notes() {
    let error = GenerationMismatchError::new(100, 105, 0x1234, dummy_span());

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("100"));
    assert!(notes.contains("105"));
    assert!(notes.contains("0x1234"));
}

#[test]
fn test_genref_with_operation() {
    let error =
        GenerationMismatchError::new(1, 2, 0x1000, dummy_span()).with_operation("dereference");

    let diag = error.to_diagnostic();

    assert!(diag.message().contains("dereference"));
}

#[test]
fn test_genref_has_validation_help() {
    let error = GenerationMismatchError::new(1, 2, 0x1000, dummy_span());

    let diag = error.to_diagnostic();
    let helps: String = diag.helps().iter().map(|h| h.to_string()).collect();

    assert!(helps.contains("is_valid") || helps.contains("valid"));
}

#[test]
fn test_genref_explains_use_after_free() {
    let error = GenerationMismatchError::new(1, 2, 0x1000, dummy_span());

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("freed") || notes.contains("reallocated"));
}

// ==================== GAT Where Clause Error Tests ====================

#[test]
fn test_where_clause_violation_basic() {
    let clause = WhereClause {
        ty: make_named_type("K"),
        bounds: List::from_iter(vec![ProtocolBound {
            protocol: {
                use verum_ast::ty::{Ident, Path};
                Path::single(Ident::new("Hash", Span::default()))
            },
            args: List::new(),
            is_negative: false,
        }]),
    };

    let mut instantiation = Map::new();
    instantiation.insert("K".into(), make_named_type("Text"));

    let error = GATWhereClauseError::new("Item", clause, instantiation, dummy_span());

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert_eq!(diag.code(), Some("E0310"));
    assert!(diag.message().contains("where clause not satisfied"));
}

#[test]
fn test_where_clause_with_protocol() {
    let clause = WhereClause {
        ty: make_named_type("T"),
        bounds: List::new(),
    };

    let error = GATWhereClauseError::new("Item", clause, Map::new(), dummy_span())
        .with_protocol("Collection");

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("Collection"));
}

#[test]
fn test_where_clause_shows_instantiation() {
    let clause = WhereClause {
        ty: make_named_type("T"),
        bounds: List::new(),
    };

    let mut instantiation = Map::new();
    instantiation.insert("K".into(), make_named_type("Text"));
    instantiation.insert("V".into(), make_named_type("Int"));

    let error = GATWhereClauseError::new("Item", clause, instantiation, dummy_span());

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    // Should show the type mappings
    assert!(notes.contains("K =") || notes.contains("V =") || notes.contains("instantiation"));
}

#[test]
fn test_where_clause_has_implementation_suggestion() {
    let clause = WhereClause {
        ty: make_named_type("T"),
        bounds: List::from_iter(vec![ProtocolBound {
            protocol: {
                use verum_ast::ty::{Ident, Path};
                Path::single(Ident::new("Clone", Span::default()))
            },
            args: List::new(),
            is_negative: false,
        }]),
    };

    let error = GATWhereClauseError::new("Item", clause, Map::new(), dummy_span());

    let diag = error.to_diagnostic();
    let helps: String = diag.helps().iter().map(|h| h.to_string()).collect();

    // Should suggest implementing the missing protocol
    assert!(!helps.is_empty() || !diag.notes().is_empty());
}

// ==================== Negative Specialization Error Tests ====================

#[test]
fn test_negative_specialization_basic() {
    let bound = ProtocolBound {
        protocol: {
            use verum_ast::ty::{Ident, Path};
            Path::single(Ident::new("Clone", Span::default()))
        },
        args: List::new(),
        is_negative: true,
    };

    let error = NegativeSpecializationError::new(
        "Default",
        make_named_type("Wrapper<T>"),
        bound,
        dummy_span(),
    );

    let diag = error.to_diagnostic();

    assert!(diag.is_error());
    assert_eq!(diag.code(), Some("E0311"));
    assert!(diag.message().contains("Negative specialization"));
    assert!(diag.message().contains("!Clone"));
}

#[test]
fn test_negative_specialization_with_reason() {
    let bound = ProtocolBound {
        protocol: {
            use verum_ast::ty::{Ident, Path};
            Path::single(Ident::new("Send", Span::default()))
        },
        args: List::new(),
        is_negative: true,
    };

    let error = NegativeSpecializationError::new(
        "MyProtocol",
        make_named_type("MyType"),
        bound,
        dummy_span(),
    )
    .with_reason("Type implements Send via derive");

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("derive"));
}

#[test]
fn test_negative_specialization_explains_requirement() {
    let bound = ProtocolBound {
        protocol: {
            use verum_ast::ty::{Ident, Path};
            Path::single(Ident::new("Sync", Span::default()))
        },
        args: List::new(),
        is_negative: true,
    };

    let error =
        NegativeSpecializationError::new("Protocol", make_named_type("Type"), bound, dummy_span());

    let diag = error.to_diagnostic();
    let notes: String = diag.notes().iter().map(|n| n.to_string()).collect();

    assert!(notes.contains("NOT") || notes.contains("not"));
}

#[test]
fn test_negative_specialization_has_alternatives() {
    let bound = ProtocolBound {
        protocol: {
            use verum_ast::ty::{Ident, Path};
            Path::single(Ident::new("Protocol", Span::default()))
        },
        args: List::new(),
        is_negative: true,
    };

    let error =
        NegativeSpecializationError::new("Protocol", make_named_type("Type"), bound, dummy_span());

    let diag = error.to_diagnostic();
    let helps: String = diag.helps().iter().map(|h| h.to_string()).collect();

    // Should suggest alternatives like removing the impl or using positive specialization
    assert!(!helps.is_empty());
}

// ==================== Integration Tests ====================

#[test]
fn test_all_error_codes_are_unique() {
    let gat_error = GATArityError::new("T", 1, 0, dummy_span());
    let spec_error = SpecializationAmbiguityError::new(
        "P",
        make_named_type("T"),
        List::from_iter(vec![1]),
        dummy_span(),
    );
    let gen_error = GenerationMismatchError::new(1, 2, 0, dummy_span());
    let where_error = GATWhereClauseError::new(
        "T",
        WhereClause {
            ty: make_named_type("T"),
            bounds: List::new(),
        },
        Map::new(),
        dummy_span(),
    );
    let neg_error = NegativeSpecializationError::new(
        "P",
        make_named_type("T"),
        ProtocolBound {
            protocol: {
                use verum_ast::ty::{Ident, Path};
                Path::single(Ident::new("P", Span::default()))
            },
            args: List::new(),
            is_negative: true,
        },
        dummy_span(),
    );

    let gat_diag = gat_error.to_diagnostic();
    let spec_diag = spec_error.to_diagnostic();
    let gen_diag = gen_error.to_diagnostic();
    let where_diag = where_error.to_diagnostic();
    let neg_diag = neg_error.to_diagnostic();

    let codes = vec![
        gat_diag.code(),
        spec_diag.code(),
        gen_diag.code(),
        where_diag.code(),
        neg_diag.code(),
    ];

    // All codes should be present
    for code in &codes {
        assert!(code.is_some(), "All errors must have error codes");
    }

    // All codes should be unique
    for i in 0..codes.len() {
        for j in (i + 1)..codes.len() {
            assert_ne!(
                codes[i], codes[j],
                "Error codes must be unique: {:?} == {:?}",
                codes[i], codes[j]
            );
        }
    }
}

#[test]
fn test_all_errors_are_error_severity() {
    let errors = vec![
        GATArityError::new("T", 1, 0, dummy_span()).to_diagnostic(),
        SpecializationAmbiguityError::new(
            "P",
            make_named_type("T"),
            List::from_iter(vec![1]),
            dummy_span(),
        )
        .to_diagnostic(),
        GenerationMismatchError::new(1, 2, 0, dummy_span()).to_diagnostic(),
        GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: make_named_type("T"),
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        )
        .to_diagnostic(),
        NegativeSpecializationError::new(
            "P",
            make_named_type("T"),
            ProtocolBound {
                protocol: {
                    use verum_ast::ty::{Ident, Path};
                    Path::single(Ident::new("P", Span::default()))
                },
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        )
        .to_diagnostic(),
    ];

    for diag in errors {
        assert!(
            diag.is_error(),
            "All advanced protocol diagnostics should be errors"
        );
    }
}

#[test]
fn test_all_errors_have_help_messages() {
    let errors = vec![
        GATArityError::new("T", 1, 0, dummy_span()).to_diagnostic(),
        SpecializationAmbiguityError::new(
            "P",
            make_named_type("T"),
            List::from_iter(vec![1]),
            dummy_span(),
        )
        .to_diagnostic(),
        GenerationMismatchError::new(1, 2, 0, dummy_span()).to_diagnostic(),
        GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: make_named_type("T"),
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        )
        .to_diagnostic(),
        NegativeSpecializationError::new(
            "P",
            make_named_type("T"),
            ProtocolBound {
                protocol: {
                    use verum_ast::ty::{Ident, Path};
                    Path::single(Ident::new("P", Span::default()))
                },
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        )
        .to_diagnostic(),
    ];

    for diag in &errors {
        assert!(
            !diag.helps().is_empty(),
            "All errors should have help messages: {}",
            diag.message()
        );
    }
}

#[test]
fn test_all_errors_have_primary_labels() {
    let errors = vec![
        GATArityError::new("T", 1, 0, dummy_span()).to_diagnostic(),
        SpecializationAmbiguityError::new(
            "P",
            make_named_type("T"),
            List::from_iter(vec![1]),
            dummy_span(),
        )
        .to_diagnostic(),
        GenerationMismatchError::new(1, 2, 0, dummy_span()).to_diagnostic(),
        GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: make_named_type("T"),
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        )
        .to_diagnostic(),
        NegativeSpecializationError::new(
            "P",
            make_named_type("T"),
            ProtocolBound {
                protocol: {
                    use verum_ast::ty::{Ident, Path};
                    Path::single(Ident::new("P", Span::default()))
                },
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        )
        .to_diagnostic(),
    ];

    for diag in &errors {
        assert!(
            !diag.primary_labels().is_empty(),
            "All errors should have at least one primary label: {}",
            diag.message()
        );
    }
}

#[test]
fn test_error_messages_are_descriptive() {
    let errors = vec![
        (
            "GAT",
            GATArityError::new("Item", 1, 0, dummy_span()).to_diagnostic(),
        ),
        (
            "Specialization",
            SpecializationAmbiguityError::new(
                "Display",
                make_named_type("T"),
                List::from_iter(vec![1, 2]),
                dummy_span(),
            )
            .to_diagnostic(),
        ),
        (
            "GenRef",
            GenerationMismatchError::new(1, 2, 0, dummy_span()).to_diagnostic(),
        ),
        (
            "WhereClause",
            GATWhereClauseError::new(
                "Item",
                WhereClause {
                    ty: make_named_type("T"),
                    bounds: List::new(),
                },
                Map::new(),
                dummy_span(),
            )
            .to_diagnostic(),
        ),
        (
            "Negative",
            NegativeSpecializationError::new(
                "Protocol",
                make_named_type("T"),
                ProtocolBound {
                    protocol: {
                        use verum_ast::ty::{Ident, Path};
                        Path::single(Ident::new("Clone", Span::default()))
                    },
                    args: List::new(),
                    is_negative: true,
                },
                dummy_span(),
            )
            .to_diagnostic(),
        ),
    ];

    for (name, diag) in &errors {
        let msg = diag.message();
        assert!(
            msg.len() > 20,
            "{} error message should be descriptive (length: {}): {}",
            name,
            msg.len(),
            msg
        );
    }
}
