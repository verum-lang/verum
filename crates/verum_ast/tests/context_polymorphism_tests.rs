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
//! Comprehensive tests for Context Polymorphism AST support.
//!
//! Tests for context polymorphism: higher-order functions propagating contexts.
//! Verifies context parameter generic params and context-aware function types.
//!
//! Tests the `GenericParamKind::Context` variant for context polymorphism
//! which enables higher-order functions to propagate contexts from callbacks.

use verum_ast::span::{FileId, Span};
use verum_ast::ty::{GenericParam, GenericParamKind, Ident, Lifetime};
use verum_common::{List, Maybe};

// ============================================================================
// GenericParamKind::Context Construction Tests
// ============================================================================

#[test]
fn test_context_param_construction() {
    let name = Ident::new("C", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name: name.clone() },
        is_implicit: false,
        span: Span::dummy(),
    };

    match &param.kind {
        GenericParamKind::Context { name: ctx_name } => {
            assert_eq!(ctx_name.name.as_str(), "C");
        }
        _ => panic!("Expected Context param kind"),
    }
}

#[test]
fn test_context_param_with_various_names() {
    let names = vec!["C", "Ctx", "Context", "MyContext", "CtxA", "C1"];

    for name_str in names {
        let name = Ident::new(name_str, Span::dummy());
        let param = GenericParam {
            kind: GenericParamKind::Context { name },
            is_implicit: false,
            span: Span::dummy(),
        };

        match &param.kind {
            GenericParamKind::Context { name: ctx_name } => {
                assert_eq!(ctx_name.name.as_str(), name_str);
            }
            _ => panic!("Expected Context param kind for name: {}", name_str),
        }
    }
}

#[test]
fn test_context_param_preserves_span() {
    let span = Span::new(10, 20, FileId::new(0));
    let name = Ident::new("C", span);
    let param = GenericParam {
        kind: GenericParamKind::Context { name: name.clone() },
        is_implicit: false,
        span,
    };

    assert_eq!(param.span.start, 10);
    assert_eq!(param.span.end, 20);

    match &param.kind {
        GenericParamKind::Context { name: ctx_name } => {
            assert_eq!(ctx_name.span.start, 10);
            assert_eq!(ctx_name.span.end, 20);
        }
        _ => panic!("Expected Context param kind"),
    }
}

// ============================================================================
// GenericParamKind::Context Equality Tests
// ============================================================================

#[test]
fn test_context_param_equality() {
    let name1 = Ident::new("C", Span::dummy());
    let name2 = Ident::new("C", Span::dummy());
    let name3 = Ident::new("D", Span::dummy());

    let param1 = GenericParam {
        kind: GenericParamKind::Context { name: name1 },
        is_implicit: false,
        span: Span::dummy(),
    };
    let param2 = GenericParam {
        kind: GenericParamKind::Context { name: name2 },
        is_implicit: false,
        span: Span::dummy(),
    };
    let param3 = GenericParam {
        kind: GenericParamKind::Context { name: name3 },
        is_implicit: false,
        span: Span::dummy(),
    };

    assert_eq!(param1, param2, "Same name should be equal");
    assert_ne!(param1, param3, "Different names should not be equal");
}

#[test]
fn test_context_param_not_equal_to_type_param() {
    let name = Ident::new("C", Span::dummy());

    let context_param = GenericParam {
        kind: GenericParamKind::Context { name: name.clone() },
        is_implicit: false,
        span: Span::dummy(),
    };

    let type_param = GenericParam {
        kind: GenericParamKind::Type {
            name: name.clone(),
            bounds: List::new(),
            default: Maybe::None,
        },
        is_implicit: false,
        span: Span::dummy(),
    };

    assert_ne!(
        context_param, type_param,
        "Context param should not equal Type param even with same name"
    );
}

// ============================================================================
// GenericParamKind::Context with Other Param Types Tests
// ============================================================================

#[test]
fn test_mixed_generic_params_with_context() {
    let params: List<GenericParam> = List::from_iter([
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("T", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("U", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    assert_eq!(params.len(), 3);

    // Verify the context param is at the end
    match &params[2].kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "C");
        }
        _ => panic!("Expected Context param at index 2"),
    }
}

#[test]
fn test_multiple_context_params() {
    // While unusual, multiple context params should be syntactically valid
    let params: List<GenericParam> = List::from_iter([
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C1", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C2", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    assert_eq!(params.len(), 2);

    let names: Vec<&str> = params
        .iter()
        .filter_map(|p| match &p.kind {
            GenericParamKind::Context { name } => Some(name.name.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(names, vec!["C1", "C2"]);
}

#[test]
fn test_context_param_with_hkt_and_meta() {
    use verum_ast::ty::Type;

    let params: List<GenericParam> = List::from_iter([
        // HKT param: F<_>
        GenericParam {
            kind: GenericParamKind::HigherKinded {
                name: Ident::new("F", Span::dummy()),
                arity: 1,
                bounds: List::new(),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        // Meta param: N: meta Int
        GenericParam {
            kind: GenericParamKind::Meta {
                name: Ident::new("N", Span::dummy()),
                ty: Type::int(Span::dummy()),
                refinement: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        // Context param: using C
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    assert_eq!(params.len(), 3);

    // Count each type
    let mut hkt_count = 0;
    let mut meta_count = 0;
    let mut context_count = 0;

    for param in params.iter() {
        match &param.kind {
            GenericParamKind::HigherKinded { .. } => hkt_count += 1,
            GenericParamKind::Meta { .. } => meta_count += 1,
            GenericParamKind::Context { .. } => context_count += 1,
            _ => {}
        }
    }

    assert_eq!(hkt_count, 1);
    assert_eq!(meta_count, 1);
    assert_eq!(context_count, 1);
}

// ============================================================================
// GenericParamKind::Context Clone Tests
// ============================================================================

#[test]
fn test_context_param_clone() {
    let name = Ident::new("C", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    let cloned = param.clone();

    assert_eq!(param, cloned);

    // Verify it's a deep clone
    match (&param.kind, &cloned.kind) {
        (
            GenericParamKind::Context { name: n1 },
            GenericParamKind::Context { name: n2 },
        ) => {
            assert_eq!(n1.name, n2.name);
        }
        _ => panic!("Expected both to be Context"),
    }
}

// ============================================================================
// GenericParamKind::Context Debug/Display Tests
// ============================================================================

#[test]
fn test_context_param_debug() {
    let name = Ident::new("C", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    let debug_str = format!("{:?}", param);
    assert!(debug_str.contains("Context"));
    assert!(debug_str.contains("C"));
}

// ============================================================================
// GenericParamKind::Context Serialization Tests
// ============================================================================

#[test]
fn test_context_param_serialization() {
    let name = Ident::new("C", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    // Test JSON serialization
    let json = serde_json::to_string(&param).expect("Should serialize");
    assert!(json.contains("Context"));

    // Test deserialization
    let deserialized: GenericParam =
        serde_json::from_str(&json).expect("Should deserialize");
    assert_eq!(param, deserialized);
}

// ============================================================================
// GenericParamKind Pattern Matching Tests
// ============================================================================

#[test]
fn test_context_param_pattern_matching() {
    use verum_common::Text;

    let params = [
        GenericParamKind::Type {
            name: Ident::new("T", Span::dummy()),
            bounds: List::new(),
            default: Maybe::None,
        },
        GenericParamKind::Context {
            name: Ident::new("C", Span::dummy()),
        },
        GenericParamKind::Lifetime {
            name: Lifetime {
                name: Text::from("a"),
                span: Span::dummy(),
            },
        },
    ];

    for (i, kind) in params.iter().enumerate() {
        match kind {
            GenericParamKind::Type { .. } => assert_eq!(i, 0),
            GenericParamKind::Context { name } => {
                assert_eq!(i, 1);
                assert_eq!(name.name.as_str(), "C");
            }
            GenericParamKind::Lifetime { .. } => assert_eq!(i, 2),
            _ => panic!("Unexpected param kind at index {}", i),
        }
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_context_param_empty_name() {
    // Edge case: empty name (should still work syntactically)
    let name = Ident::new("", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    match &param.kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "");
        }
        _ => panic!("Expected Context"),
    }
}

#[test]
fn test_context_param_unicode_name() {
    // Edge case: unicode name
    let name = Ident::new("Контекст", Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    match &param.kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "Контекст");
        }
        _ => panic!("Expected Context"),
    }
}

#[test]
fn test_context_param_long_name() {
    use verum_common::Text;

    // Edge case: very long name
    let long_name = "A".repeat(1000);
    let name = Ident::new(Text::from(long_name.as_str()), Span::dummy());
    let param = GenericParam {
        kind: GenericParamKind::Context { name },
        is_implicit: false,
        span: Span::dummy(),
    };

    match &param.kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.len(), 1000);
        }
        _ => panic!("Expected Context"),
    }
}

// ============================================================================
// GenericParamKind::Context Helper Function Tests
// ============================================================================

#[test]
fn test_context_param_filter_from_list() {
    let params: List<GenericParam> = List::from_iter([
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("T", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("Ctx1", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("U", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("Ctx2", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    // Filter only context params
    let context_params: Vec<&GenericParam> = params
        .iter()
        .filter(|p| matches!(&p.kind, GenericParamKind::Context { .. }))
        .collect();

    assert_eq!(context_params.len(), 2);

    let names: Vec<&str> = context_params
        .iter()
        .filter_map(|p| match &p.kind {
            GenericParamKind::Context { name } => Some(name.name.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(names, vec!["Ctx1", "Ctx2"]);
}

#[test]
fn test_context_param_position_in_generic_list() {
    // Context params typically come after type params (convention)
    let params: List<GenericParam> = List::from_iter([
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("T", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("U", Span::dummy()),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    // Find the first context param position
    let context_pos = params
        .iter()
        .position(|p| matches!(&p.kind, GenericParamKind::Context { .. }));

    assert_eq!(context_pos, Some(2), "Context param should be at position 2");
}

// ============================================================================
// GenericParamKind::Context Interop Tests
// ============================================================================

#[test]
fn test_context_param_with_type_bounds() {
    use verum_ast::ty::{Path, TypeBound, TypeBoundKind};

    // Type param with bounds alongside context param
    let clone_bound = TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(Ident::new("Clone", Span::dummy()))),
        span: Span::dummy(),
    };

    let params: List<GenericParam> = List::from_iter([
        GenericParam {
            kind: GenericParamKind::Type {
                name: Ident::new("T", Span::dummy()),
                bounds: List::from_iter([clone_bound]),
                default: Maybe::None,
            },
            is_implicit: false,
            span: Span::dummy(),
        },
        GenericParam {
            kind: GenericParamKind::Context {
                name: Ident::new("C", Span::dummy()),
            },
            is_implicit: false,
            span: Span::dummy(),
        },
    ]);

    assert_eq!(params.len(), 2);

    // Verify type param has bounds
    match &params[0].kind {
        GenericParamKind::Type { bounds, .. } => {
            assert_eq!(bounds.len(), 1);
        }
        _ => panic!("Expected Type param at index 0"),
    }

    // Verify context param
    match &params[1].kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "C");
        }
        _ => panic!("Expected Context param at index 1"),
    }
}

#[test]
fn test_context_param_name_extraction() {
    let param = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("MyContext", Span::dummy()),
        },
        is_implicit: false,
        span: Span::dummy(),
    };

    // Extract name from context param
    let extracted_name = match &param.kind {
        GenericParamKind::Context { name } => name.name.as_str(),
        _ => "",
    };

    assert_eq!(extracted_name, "MyContext");
}

// ============================================================================
// GenericParamKind::Context Vec Collection Tests
// ============================================================================

#[test]
fn test_context_param_in_vec_collection() {
    let param1 = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C1", Span::dummy()),
        },
        is_implicit: false,
        span: Span::dummy(),
    };
    let param2 = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C2", Span::dummy()),
        },
        is_implicit: false,
        span: Span::dummy(),
    };
    let param3 = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C1", Span::dummy()),
        },
        is_implicit: false,
        span: Span::dummy(),
    };

    let arr = [param1.clone(), param2.clone(), param3.clone()];
    assert_eq!(arr.len(), 3);

    // Count unique by name
    let unique_names: std::collections::HashSet<&str> = arr
        .iter()
        .filter_map(|p| match &p.kind {
            GenericParamKind::Context { name } => Some(name.name.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(unique_names.len(), 2); // C1 and C2
}

// ============================================================================
// GenericParamKind::Context Default Value Tests
// ============================================================================

#[test]
fn test_context_param_with_span_default() {
    use verum_ast::span::Span;

    let param = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C", Span::default()),
        },
        is_implicit: false,
        span: Span::default(),
    };

    // Default span should be valid (likely dummy)
    match &param.kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "C");
        }
        _ => panic!("Expected Context"),
    }
}
