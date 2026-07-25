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
//! Tests for GenRef<T> type syntax parsing
//!
//! Tests for advanced protocol features: GATs, higher-rank bounds, specialization Section 1.2 lines 143-193
//! Also covers streaming iterator generation-aware patterns
//!
//! This module tests parsing of GenRef types used for generation-aware
//! references in CBGR lending iterators and self-referential types.
//!
//! GenRef<T> provides explicit generation tracking for references that
//! need to outlive their origin in lending iterator patterns.

use verum_ast::{Type, TypeKind, span::FileId};
use verum_fast_parser::VerumParser;

fn parse_type(source: &str) -> Type {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_type_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse type: {}", source))
}

fn expect_parse_error(source: &str) -> bool {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_type_str(source, file_id).is_err()
}

// ============================================================================
// BASIC GenRef SYNTAX TESTS
// ============================================================================

#[test]
fn test_genref_basic_int() {
    let ty = parse_type("GenRef<Int>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int as GenRef inner type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_basic_text() {
    let ty = parse_type("GenRef<Text>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Text => {
                    // Correct
                }
                _ => panic!("Expected Text as GenRef inner type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_basic_bool() {
    let ty = parse_type("GenRef<Bool>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Bool => {
                    // Correct
                }
                _ => panic!("Expected Bool as GenRef inner type"),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

// ============================================================================
// GenRef WITH GENERIC TYPES
// ============================================================================

#[test]
fn test_genref_list() {
    let ty = parse_type("GenRef<List<Int>>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - List<Int> is a generic type
                }
                _ => panic!("Expected generic type inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_map() {
    let ty = parse_type("GenRef<Map<Text, Int>>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - Map<Text, Int> is a generic type
                }
                _ => panic!("Expected generic type inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_option() {
    let ty = parse_type("GenRef<Maybe<Text>>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - Maybe<Text> is a generic type
                }
                _ => panic!("Expected generic type inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

// ============================================================================
// GenRef WITH REFERENCES
// ============================================================================

#[test]
fn test_genref_reference() {
    // GenRef wrapping a managed reference - common in lending iterators
    let ty = parse_type("GenRef<&Int>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match &inner.kind {
                TypeKind::Reference { mutable, inner } => {
                    assert!(!mutable, "Expected immutable reference");
                    match &inner.kind {
                        TypeKind::Int => {
                            // Correct
                        }
                        _ => panic!("Expected Int inside reference"),
                    }
                }
                _ => panic!("Expected reference inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_mutable_reference() {
    let ty = parse_type("GenRef<&mut Text>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match &inner.kind {
                TypeKind::Reference { mutable, inner } => {
                    assert!(*mutable, "Expected mutable reference");
                    match &inner.kind {
                        TypeKind::Text => {
                            // Correct
                        }
                        _ => panic!("Expected Text inside reference"),
                    }
                }
                _ => panic!("Expected reference inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_slice() {
    // Common pattern: GenRef<&[T]> for lending iterator windows
    let ty = parse_type("GenRef<&[Int]>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match &inner.kind {
                TypeKind::Reference {
                    mutable,
                    inner: ref_inner,
                } => {
                    assert!(!mutable, "Expected immutable reference");
                    match &ref_inner.kind {
                        TypeKind::Slice(_) => {
                            // Correct - slice type
                        }
                        _ => panic!("Expected slice inside reference, got {:?}", ref_inner.kind),
                    }
                }
                _ => panic!("Expected reference inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

// ============================================================================
// NESTED GenRef TESTS
// ============================================================================

#[test]
fn test_genref_nested() {
    // Nested GenRef - unusual but syntactically valid
    let ty = parse_type("GenRef<GenRef<Int>>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match &inner.kind {
                TypeKind::GenRef { inner: inner2 } => {
                    match &inner2.kind {
                        TypeKind::Int => {
                            // Correct
                        }
                        _ => panic!("Expected Int in nested GenRef"),
                    }
                }
                _ => panic!("Expected nested GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_complex_nested() {
    let ty = parse_type("GenRef<List<GenRef<Int>>>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - List<GenRef<Int>>
                }
                _ => panic!("Expected generic type inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

// ============================================================================
// GenRef WITH REFINEMENT TYPES
// ============================================================================

#[test]
fn test_genref_with_inline_refinement() {
    // GenRef<Int> with inline refinement
    let ty = parse_type("GenRef<Int{> 0}>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::Refined { ref base, .. } => {
                    match base.kind {
                        TypeKind::Int => {
                            // Correct - refined Int inside GenRef
                        }
                        _ => panic!("Expected Int as base of refinement"),
                    }
                }
                _ => panic!("Expected refined type inside GenRef, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_with_lambda_refinement() {
    // GenRef with lambda-style refinement (Rule 2)
    let ty = parse_type("GenRef<Int> where |x| x > 0");
    match ty.kind {
        TypeKind::Refined { ref base, .. } => {
            match base.kind {
                TypeKind::GenRef { ref inner } => {
                    match inner.kind {
                        TypeKind::Int => {
                            // Correct - refinement on GenRef
                        }
                        _ => panic!("Expected Int inside GenRef"),
                    }
                }
                _ => panic!("Expected GenRef as base of refinement, got {:?}", base.kind),
            }
        }
        _ => panic!("Expected refined GenRef type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_with_bare_refinement() {
    // GenRef with bare where refinement (Rule 5 - legacy)
    let ty = parse_type("GenRef<Text> where is_email(it)");
    match ty.kind {
        TypeKind::Refined { ref base, .. } => {
            match base.kind {
                TypeKind::GenRef { ref inner } => {
                    match inner.kind {
                        TypeKind::Text => {
                            // Correct - refinement on GenRef
                        }
                        _ => panic!("Expected Text inside GenRef"),
                    }
                }
                _ => panic!("Expected GenRef as base of refinement, got {:?}", base.kind),
            }
        }
        _ => panic!("Expected refined GenRef type, got {:?}", ty.kind),
    }
}

// ============================================================================
// GenRef IN FUNCTION SIGNATURES
// ============================================================================

#[test]
fn test_genref_function_parameter() {
    let ty = parse_type("fn(GenRef<Int>) -> Bool");
    match ty.kind {
        TypeKind::Function {
            ref params,
            ref return_type,
            ..
        } => {
            assert_eq!(params.len(), 1, "Expected 1 parameter");
            match params[0].kind {
                TypeKind::GenRef { .. } => {
                    // Correct - GenRef parameter
                }
                _ => panic!("Expected GenRef parameter, got {:?}", params[0].kind),
            }
            match return_type.kind {
                TypeKind::Bool => {
                    // Correct
                }
                _ => panic!("Expected Bool return type"),
            }
        }
        _ => panic!("Expected function type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_function_return() {
    // Common pattern: Iterator::next returns Maybe<GenRef<T>>
    let ty = parse_type("fn(&mut self) -> Maybe<GenRef<Int>>");
    match ty.kind {
        TypeKind::Function {
            ref params,
            ref return_type,
            ..
        } => {
            assert_eq!(params.len(), 1, "Expected 1 parameter");
            match return_type.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - Maybe<GenRef<Int>>
                }
                _ => panic!("Expected generic return type, got {:?}", return_type.kind),
            }
        }
        _ => panic!("Expected function type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_complex_function() {
    let ty = parse_type("fn(List<Int>, usize) -> Maybe<GenRef<&[Int]>>");
    match ty.kind {
        TypeKind::Function {
            ref params,
            ref return_type,
            ..
        } => {
            assert_eq!(params.len(), 2, "Expected 2 parameters");
            match return_type.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - Maybe<GenRef<&[Int]>>
                }
                _ => panic!("Expected generic return type"),
            }
        }
        _ => panic!("Expected function type, got {:?}", ty.kind),
    }
}

// ============================================================================
// GenRef IN TUPLES AND ARRAYS
// ============================================================================

#[test]
fn test_genref_in_tuple() {
    let ty = parse_type("(GenRef<Int>, Text)");
    match ty.kind {
        TypeKind::Tuple(ref elements) => {
            assert_eq!(elements.len(), 2, "Expected 2-element tuple");
            match elements[0].kind {
                TypeKind::GenRef { .. } => {
                    // Correct - GenRef in tuple
                }
                _ => panic!("Expected GenRef in first tuple element"),
            }
        }
        _ => panic!("Expected tuple type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_array() {
    let ty = parse_type("[GenRef<Int>; 10]");
    match ty.kind {
        TypeKind::Array { ref element, .. } => {
            match element.kind {
                TypeKind::GenRef { .. } => {
                    // Correct - array of GenRef
                }
                _ => panic!("Expected GenRef array element"),
            }
        }
        _ => panic!("Expected array type, got {:?}", ty.kind),
    }
}

#[test]
fn test_genref_slice_type() {
    let ty = parse_type("[GenRef<Text>]");
    match ty.kind {
        TypeKind::Slice(ref element) => {
            match element.kind {
                TypeKind::GenRef { .. } => {
                    // Correct - slice of GenRef
                }
                _ => panic!("Expected GenRef slice element"),
            }
        }
        _ => panic!("Expected slice type, got {:?}", ty.kind),
    }
}

// ============================================================================
// ERROR CASES
// ============================================================================

#[test]
fn test_genref_missing_type_parameter() {
    // GenRef without type parameter should fail
    assert!(
        expect_parse_error("GenRef"),
        "Expected parse error for 'GenRef' without type parameter"
    );
}

#[test]
fn test_genref_missing_closing_bracket() {
    // Unclosed generic bracket
    assert!(
        expect_parse_error("GenRef<Int"),
        "Expected parse error for unclosed generic bracket"
    );
}

#[test]
fn test_genref_empty_type_parameter() {
    // Empty type parameter
    assert!(
        expect_parse_error("GenRef<>"),
        "Expected parse error for empty type parameter"
    );
}

#[test]
fn test_genref_multiple_type_parameters() {
    // GenRef only takes one type parameter
    assert!(
        expect_parse_error("GenRef<Int, Text>"),
        "Expected parse error for multiple type parameters"
    );
}

// ============================================================================
// COMPLEX REAL-WORLD PATTERNS
// ============================================================================

#[test]
fn test_genref_lending_iterator_pattern() {
    // protocol Iterator { fn next(&mut self) -> Maybe<GenRef<Self::Item>> }
    let ty = parse_type("fn(&mut self) -> Maybe<GenRef<Item>>");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct - standard lending iterator signature
        }
        _ => panic!("Expected function type for lending iterator"),
    }
}

#[test]
fn test_genref_window_iterator() {
    // WindowIterator returns GenRef<&[T]>
    let ty = parse_type("GenRef<&[Text]>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match &inner.kind {
                TypeKind::Reference {
                    inner: ref_inner, ..
                } => {
                    match &ref_inner.kind {
                        TypeKind::Slice(_) => {
                            // Correct - window iterator pattern
                        }
                        _ => panic!("Expected slice type"),
                    }
                }
                _ => panic!("Expected reference type"),
            }
        }
        _ => panic!("Expected GenRef type"),
    }
}

#[test]
fn test_genref_self_referential_struct() {
    // Common pattern: struct field containing GenRef
    let ty = parse_type("(GenRef<List<Int>>, usize)");
    match ty.kind {
        TypeKind::Tuple(ref elements) => {
            assert_eq!(elements.len(), 2, "Expected 2-element tuple");
            match elements[0].kind {
                TypeKind::GenRef { .. } => {
                    // Correct - self-referential struct pattern
                }
                _ => panic!("Expected GenRef in tuple"),
            }
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_genref_with_checked_reference() {
    // GenRef<&checked T> - combining generation tracking with static checking
    let ty = parse_type("GenRef<&checked Int>");
    match ty.kind {
        TypeKind::GenRef { ref inner } => {
            match inner.kind {
                TypeKind::CheckedReference { .. } => {
                    // Correct - GenRef wrapping checked reference
                }
                _ => panic!("Expected checked reference inside GenRef"),
            }
        }
        _ => panic!("Expected GenRef type"),
    }
}

#[test]
fn test_genref_result_type() {
    // Result<GenRef<T>, Error> - common in fallible lending iterators
    let ty = parse_type("Result<GenRef<&[u8]>, Error>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Correct - Result containing GenRef
        }
        _ => panic!("Expected generic Result type"),
    }
}
