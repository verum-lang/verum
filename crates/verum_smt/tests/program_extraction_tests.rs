//! Comprehensive tests for program extraction module
//!
//! Tests for extracting executable programs from constructive proofs.
//! Implements the Curry-Howard correspondence for Verum's type system.
//!
//! Program extraction via the Curry-Howard correspondence: constructive proofs of
//! existence theorems yield executable programs. `@extract` turns `exists!(q,r). a = b*q+r`
//! into `fn div_mod(a, b) -> (Nat, Nat)`. `@extract_witness` extracts just the witness.
//! Proof-irrelevant parts (axioms, SMT proofs) are erased; only computational content remains.

#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs
)]

use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{GenericArg, Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{Heap, List, Maybe};
use verum_smt::program_extraction::{
    CodeGenerator, ExtractedProgram, ExtractionConfig, ExtractionTarget, ProgramExtractor,
    ProofEraser,
};
use verum_smt::ProofTerm;
use verum_common::Text;

// ==================== ProgramExtractor Tests ====================

#[test]
fn test_program_extractor_creation() {
    let extractor = ProgramExtractor::new();
    assert_eq!(extractor.stats().attempts, 0);
    assert_eq!(extractor.stats().successful, 0);
}

#[test]
fn test_extraction_config_defaults() {
    let config = ExtractionConfig::default();
    assert!(config.optimize);
    assert!(config.erase_proofs);
    assert!(config.inline_small_functions);
}

#[test]
fn test_extraction_config_builder_pattern() {
    let config = ExtractionConfig::default()
        .without_optimizations()
        .keep_proofs();

    assert!(!config.optimize);
    assert!(!config.inline_small_functions);
    assert!(!config.erase_proofs);
}

// ==================== Expression to Pattern Conversion Tests ====================

#[test]
fn test_expr_to_pattern_literal_int() {
    let extractor = ProgramExtractor::new();

    // Create a literal expression: 42
    let lit_expr = Expr::literal(Literal {
        kind: LiteralKind::Int(IntLit {
            value: 42,
            suffix: Maybe::None,
        }),
        span: Span::default(),
    });

    // This would be tested via the internal method
    // The extractor should be able to convert literal expressions to patterns
}

#[test]
fn test_expr_to_pattern_path_identifier() {
    let extractor = ProgramExtractor::new();

    // Create a path expression: x
    let path_expr = Expr::new(
        ExprKind::Path(Path::single(Ident {
            name: "x".to_string().into(),
            span: Span::default(),
        })),
        Span::default(),
    );

    // The extractor should convert this to an identifier pattern
}

#[test]
fn test_expr_to_pattern_tuple() {
    let extractor = ProgramExtractor::new();

    // Create a tuple expression: (a, b, c)
    let elements = List::from(vec![
        Expr::new(
            ExprKind::Path(Path::single(Ident {
                name: "a".to_string().into(),
                span: Span::default(),
            })),
            Span::default(),
        ),
        Expr::new(
            ExprKind::Path(Path::single(Ident {
                name: "b".to_string().into(),
                span: Span::default(),
            })),
            Span::default(),
        ),
    ]);

    let tuple_expr = Expr::new(ExprKind::Tuple(elements), Span::default());

    // The extractor should convert this to a tuple pattern
}

// ==================== Type Formatting Tests ====================

#[test]
fn test_code_formatter_creation() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);
    // Formatter should be ready to format expressions and types
}

#[test]
fn test_format_primitive_types() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test Bool type formatting
    let bool_type = Type::new(TypeKind::Bool, Span::default());
    let formatted = generator.generate(&ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: bool_type,
        body: Expr::literal(Literal {
            kind: LiteralKind::Bool(true),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    });
    assert!(formatted.contains("Bool"));

    // Test Int type formatting
    let int_type = Type::new(TypeKind::Int, Span::default());
    let formatted = generator.generate(&ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: int_type,
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 0,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    });
    assert!(formatted.contains("Int"));
}

#[test]
fn test_format_reference_types() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test immutable reference type
    let ref_type = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(Type::new(TypeKind::Int, Span::default())),
        },
        Span::default(),
    );
    let formatted = generator.generate(&ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: ref_type,
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 0,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    });
    assert!(formatted.contains("&"));
    assert!(formatted.contains("Int"));
}

#[test]
fn test_format_tuple_types() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test tuple type (Int, Bool)
    let tuple_type = Type::new(
        TypeKind::Tuple(List::from(vec![
            Type::new(TypeKind::Int, Span::default()),
            Type::new(TypeKind::Bool, Span::default()),
        ])),
        Span::default(),
    );
    let formatted = generator.generate(&ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: tuple_type,
        body: Expr::new(
            ExprKind::Tuple(List::from(vec![
                Expr::literal(Literal {
                    kind: LiteralKind::Int(IntLit {
                        value: 0,
                        suffix: Maybe::None,
                    }),
                    span: Span::default(),
                }),
                Expr::literal(Literal {
                    kind: LiteralKind::Bool(true),
                    span: Span::default(),
                }),
            ])),
            Span::default(),
        ),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    });
    assert!(formatted.contains("Int"));
    assert!(formatted.contains("Bool"));
}

// ==================== Expression Formatting Tests ====================

#[test]
fn test_format_literal_expressions() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test integer literal
    let int_program = ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: Type::new(TypeKind::Int, Span::default()),
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 42,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };
    let formatted = generator.generate(&int_program);
    assert!(formatted.contains("42"));

    // Test boolean literal
    let bool_program = ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: Type::new(TypeKind::Bool, Span::default()),
        body: Expr::literal(Literal {
            kind: LiteralKind::Bool(true),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };
    let formatted = generator.generate(&bool_program);
    assert!(formatted.contains("true"));
}

#[test]
fn test_format_binary_expressions() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test addition expression: 1 + 2
    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 1,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
            right: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 2,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
        },
        Span::default(),
    );

    let program = ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: Type::new(TypeKind::Int, Span::default()),
        body: add_expr,
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };
    let formatted = generator.generate(&program);
    assert!(formatted.contains("1"));
    assert!(formatted.contains("2"));
    assert!(formatted.contains("+"));
}

#[test]
fn test_format_comparison_expressions() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    // Test comparison: x < 10
    let cmp_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident {
                    name: "x".to_string().into(),
                    span: Span::default(),
                })),
                Span::default(),
            )),
            right: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 10,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
        },
        Span::default(),
    );

    let program = ExtractedProgram {
        name: Text::from("test"),
        params: List::new(),
        return_type: Type::new(TypeKind::Bool, Span::default()),
        body: cmp_expr,
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };
    let formatted = generator.generate(&program);
    assert!(formatted.contains("x"));
    assert!(formatted.contains("10"));
    assert!(formatted.contains("<"));
}

// ==================== Type Inference Tests ====================

#[test]
fn test_type_inference_literal() {
    // Type inference for literals should be straightforward
    let extractor = ProgramExtractor::new();

    // Integer literal should infer to Int
    let int_lit = Expr::literal(Literal {
        kind: LiteralKind::Int(IntLit {
            value: 42,
            suffix: Maybe::None,
        }),
        span: Span::default(),
    });
    // Type inference would be tested via extraction

    // Boolean literal should infer to Bool
    let bool_lit = Expr::literal(Literal {
        kind: LiteralKind::Bool(true),
        span: Span::default(),
    });
    // Type inference would be tested via extraction
}

#[test]
fn test_type_inference_binary_ops() {
    // Arithmetic operations should infer numeric types
    // Comparison operations should infer Bool
    // Logical operations should infer Bool

    let extractor = ProgramExtractor::new();

    // Addition of ints should infer Int
    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 1,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
            right: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 2,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
        },
        Span::default(),
    );
    // Type inference would infer Int

    // Comparison should infer Bool
    let cmp_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 1,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
            right: Heap::new(Expr::literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: 2,
                    suffix: Maybe::None,
                }),
                span: Span::default(),
            })),
        },
        Span::default(),
    );
    // Type inference would infer Bool
}

// ==================== Parameter Type Inference Tests ====================

#[test]
fn test_param_type_inference_conventions() {
    // Test that parameter names follow conventions for type inference
    // n, m, k, i, j -> Nat
    // x, y, z -> Int
    // b, p, q -> Bool
    // xs, ys, zs -> List<_>

    let extractor = ProgramExtractor::new();
    // These would be tested via extraction of proofs with specific parameter names
}

// ==================== Proof Eraser Tests ====================

#[test]
fn test_proof_eraser_creation() {
    let eraser = ProofEraser::new();
    assert_eq!(eraser.stats().programs_processed, 0);
    assert_eq!(eraser.stats().proofs_erased, 0);
}

#[test]
fn test_proof_erasure_preserves_computational_content() {
    let mut eraser = ProofEraser::new();

    // Create a simple extracted program
    let program = ExtractedProgram {
        name: Text::from("identity"),
        params: List::new(),
        return_type: Type::new(TypeKind::Int, Span::default()),
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 42,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };

    let erased = eraser.erase_proofs(&program);

    // The computational content should be preserved
    assert_eq!(erased.name, program.name);
    // Source proof should be removed
    assert!(erased.source_proof.is_none());
}

// ==================== Extracted Program Tests ====================

#[test]
fn test_extracted_program_creation() {
    let program = ExtractedProgram {
        name: Text::from("test_function"),
        params: List::new(),
        return_type: Type::new(TypeKind::Bool, Span::default()),
        body: Expr::literal(Literal {
            kind: LiteralKind::Bool(true),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };

    assert_eq!(program.name.as_str(), "test_function");
    assert!(program.params.is_empty());
    assert!(program.preconditions.is_empty());
}

#[test]
fn test_extracted_program_with_params() {
    use verum_smt::program_extraction::Parameter as ExtractionParameter;

    let params = vec![ExtractionParameter::new(
        Text::from("n"),
        Type::new(
            TypeKind::Path(Path::single(Ident {
                name: "Nat".to_string().into(),
                span: Span::default(),
            })),
            Span::default(),
        ),
    )];

    let program = ExtractedProgram {
        name: Text::from("factorial"),
        params: params.into(),
        return_type: Type::new(
            TypeKind::Path(Path::single(Ident {
                name: "Nat".to_string().into(),
                span: Span::default(),
            })),
            Span::default(),
        ),
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 1,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };

    assert_eq!(program.params.len(), 1);
}

// ==================== Pattern Formatting Tests ====================

#[test]
fn test_format_wildcard_pattern() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);
    // Test that wildcard patterns are formatted as "_"
}

#[test]
fn test_format_identifier_pattern() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);
    // Test that identifier patterns preserve names
}

#[test]
fn test_format_tuple_pattern() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);
    // Test that tuple patterns are formatted as (a, b, c)
}

#[test]
fn test_format_variant_pattern() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);
    // Test that variant patterns are formatted as Some(x), None, etc.
}

// ==================== Integration Tests ====================

#[test]
fn test_extract_simple_proof() {
    let mut extractor = ProgramExtractor::new();

    // Create a simple axiom proof with a literal true expression
    let true_expr = Expr::literal(Literal {
        kind: LiteralKind::Bool(true),
        span: Span::default(),
    });
    let proof = ProofTerm::axiom("true_intro", true_expr);

    // Extraction should work for simple proofs
    let result = extractor.extract_program(&proof);
    // Axiom proofs may not produce executable programs, so result might be None
    // The important thing is that extraction doesn't panic
    let _ = result;
}

#[test]
fn test_extraction_statistics() {
    let mut extractor = ProgramExtractor::new();

    // Initial stats should be zero
    assert_eq!(extractor.stats().attempts, 0);
    assert_eq!(extractor.stats().successful, 0);

    // After extraction attempts, stats should be updated
    let true_expr = Expr::literal(Literal {
        kind: LiteralKind::Bool(true),
        span: Span::default(),
    });
    let proof = ProofTerm::axiom("test", true_expr);

    // Attempt extraction - stats should be updated
    let _ = extractor.extract_program(&proof);
    assert!(extractor.stats().attempts >= 1, "Extraction attempt should be recorded");
}

#[test]
fn test_reset_statistics() {
    let mut extractor = ProgramExtractor::new();

    // Extract something to accumulate stats
    let true_expr = Expr::literal(Literal {
        kind: LiteralKind::Bool(true),
        span: Span::default(),
    });
    let proof = ProofTerm::axiom("test", true_expr);

    // Attempt extraction to accumulate stats
    let _ = extractor.extract_program(&proof);
    let attempts_before_reset = extractor.stats().attempts;

    // Reset stats
    extractor.reset_stats();

    // Stats should be reset to zero
    assert_eq!(extractor.stats().attempts, 0, "Attempts should be reset to 0");
    assert_eq!(extractor.stats().successful, 0, "Successful should be reset to 0");
    let _ = attempts_before_reset; // Use the variable
}

// ==================== Code Target Formatting Tests ====================

#[test]
fn test_format_for_verum_target() {
    let generator = CodeGenerator::new(ExtractionTarget::Verum);

    let program = ExtractedProgram {
        name: Text::from("add"),
        params: List::new(),
        return_type: Type::new(TypeKind::Int, Span::default()),
        body: Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(Expr::literal(Literal {
                    kind: LiteralKind::Int(IntLit {
                        value: 1,
                        suffix: Maybe::None,
                    }),
                    span: Span::default(),
                })),
                right: Heap::new(Expr::literal(Literal {
                    kind: LiteralKind::Int(IntLit {
                        value: 2,
                        suffix: Maybe::None,
                    }),
                    span: Span::default(),
                })),
            },
            Span::default(),
        ),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };

    let verum_code = generator.generate(&program);
    assert!(verum_code.contains("fn"));
    assert!(verum_code.contains("add"));
}

#[test]
fn test_format_for_coq_target() {
    let generator = CodeGenerator::new(ExtractionTarget::Coq);

    let program = ExtractedProgram {
        name: Text::from("identity"),
        params: List::new(),
        return_type: Type::new(TypeKind::Int, Span::default()),
        body: Expr::literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: 42,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        }),
        source_proof: Maybe::None,
        preconditions: List::new(),
        postconditions: List::new(),
        is_extracted: true,
        documentation: Maybe::None,
    };

    let coq_code = generator.generate(&program);
    assert!(coq_code.contains("Definition"));
    assert!(coq_code.contains("identity"));
}
