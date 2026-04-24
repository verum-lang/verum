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
// Tests for type parsing
//
// Tests for type system syntax: named, generic, function, refinement, dependent types
// This module tests parsing of all Verum type forms including:
// - Primitive types (Int, Float, Bool, Text, Char)
// - Generic types (List<T>, Map<K,V>)
// - Function types (T -> U, fn(A, B) -> C)
// - Refinement types (Int{> 0})
// - Reference types (&T, &checked T, &unsafe T)
// - Tuple types ((A, B, C))
// - Array types ([T], [T; N])
// - Complex nested types

use verum_ast::{FileId, Type, TypeKind};
use verum_parser::VerumParser;

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

// === PRIMITIVE TYPE TESTS ===

#[test]
fn test_parse_type_int() {
    let ty = parse_type("Int");
    match ty.kind {
        TypeKind::Int => {
            // Correct
        }
        _ => panic!("Expected Int type"),
    }
}

#[test]
fn test_parse_type_float() {
    let ty = parse_type("Float");
    match ty.kind {
        TypeKind::Float => {
            // Correct
        }
        _ => panic!("Expected Float type"),
    }
}

#[test]
fn test_parse_type_bool() {
    let ty = parse_type("Bool");
    match ty.kind {
        TypeKind::Bool => {
            // Correct
        }
        _ => panic!("Expected Bool type"),
    }
}

#[test]
fn test_parse_type_text() {
    let ty = parse_type("Text");
    match ty.kind {
        TypeKind::Text => {
            // Correct
        }
        _ => panic!("Expected Text type"),
    }
}

#[test]
fn test_parse_type_char() {
    let ty = parse_type("Char");
    match ty.kind {
        TypeKind::Char => {
            // Correct
        }
        _ => panic!("Expected Char type"),
    }
}

#[test]
fn test_parse_type_unit() {
    let ty = parse_type("()");
    match ty.kind {
        TypeKind::Unit => {
            // Correct
        }
        _ => panic!("Expected unit type"),
    }
}

// === GENERIC TYPE TESTS ===

#[test]
fn test_parse_type_list() {
    let ty = parse_type("List<Int>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should be a path-based type with generics
        }
        _ => panic!("Expected generic List type"),
    }
}

#[test]
fn test_parse_type_map() {
    let ty = parse_type("Map<Text, Int>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should be a path-based type with generics
        }
        _ => panic!("Expected generic Map type"),
    }
}

#[test]
fn test_parse_type_option() {
    let ty = parse_type("Option<Text>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should be a path-based type with generics
        }
        _ => panic!("Expected generic Option type"),
    }
}

#[test]
fn test_parse_type_result() {
    let ty = parse_type("Result<Int, Text>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should be a path-based type with generics
        }
        _ => panic!("Expected generic Result type"),
    }
}

#[test]
fn test_parse_type_nested_generic() {
    let ty = parse_type("List<List<Int>>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should be a nested generic type
        }
        _ => panic!("Expected nested generic type"),
    }
}

// === FUNCTION TYPE TESTS ===

#[test]
fn test_parse_type_function_simple() {
    let ty = parse_type("fn(Int) -> Int");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_parse_type_function_multiple_params() {
    let ty = parse_type("fn(Int, Text) -> Bool");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_parse_type_function_no_params() {
    let ty = parse_type("fn() -> Int");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_parse_type_function_no_return() {
    let ty = parse_type("fn(Int)");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct, should default to unit return
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_parse_type_function_returning_function() {
    let ty = parse_type("fn(Int) -> fn(Int) -> Bool");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct - higher-order function type
        }
        _ => panic!("Expected function type"),
    }
}

// === REFINEMENT TYPE TESTS ===

#[test]
fn test_parse_type_refinement_simple() {
    let ty = parse_type("Int{> 0}");
    match ty.kind {
        TypeKind::Refined { .. } => {
            // Correct
        }
        _ => panic!("Expected refinement type"),
    }
}

#[test]
fn test_parse_type_refinement_comparison() {
    let ty = parse_type("Int{< 100}");
    match ty.kind {
        TypeKind::Refined { .. } => {
            // Correct
        }
        _ => panic!("Expected refinement type"),
    }
}

#[test]
fn test_parse_type_refinement_function() {
    let ty = parse_type("Text{is_email(it)}");
    match ty.kind {
        TypeKind::Refined { .. } => {
            // Correct
        }
        _ => panic!("Expected refinement type"),
    }
}

#[test]
fn test_parse_type_refinement_generic() {
    let ty = parse_type("List<Int>{is_sorted(it)}");
    match ty.kind {
        TypeKind::Refined { .. } => {
            // Correct
        }
        _ => panic!("Expected refinement type"),
    }
}

// === MANAGED REFERENCE TYPE TESTS (CBGR - &T) ===

#[test]
fn test_parse_type_reference() {
    let ty = parse_type("&Int");
    match ty.kind {
        TypeKind::Reference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable reference");
            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int as inner type"),
            }
        }
        _ => panic!("Expected reference type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_mutable_reference() {
    let ty = parse_type("&mut Int");
    match ty.kind {
        TypeKind::Reference { mutable, ref inner } => {
            assert!(mutable, "Expected mutable reference");
            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int as inner type"),
            }
        }
        _ => panic!("Expected mutable reference type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_ownership() {
    let ty = parse_type("%Int");
    match ty.kind {
        TypeKind::Ownership { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable ownership");
            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int as inner type"),
            }
        }
        _ => panic!("Expected ownership type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_reference_to_generic() {
    let ty = parse_type("&List<Int>");
    match ty.kind {
        TypeKind::Reference { .. } => {
            // Correct
        }
        _ => panic!("Expected reference type"),
    }
}

// === CHECKED REFERENCE TESTS (&checked T) ===

#[test]
fn test_parse_type_checked_reference() {
    let ty = parse_type("&checked Int");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_mutable_reference() {
    let ty = parse_type("&checked mut Int");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(mutable, "Expected mutable checked reference");

            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected mutable CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_reference_generic() {
    let ty = parse_type("&checked List<Int>");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - generic type
                }
                _ => panic!("Expected generic type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_reference_slice() {
    let ty = parse_type("&checked [Int]");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Slice(ref element) => {
                    match element.kind {
                        TypeKind::Int => {
                            // Correct
                        }
                        _ => panic!("Expected Int slice element"),
                    }
                }
                _ => panic!("Expected slice type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_reference_nested() {
    let ty = parse_type("&checked Map<Text, List<Int>>");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - complex generic type
                }
                _ => panic!("Expected generic type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_reference_tuple() {
    let ty = parse_type("&checked (Int, Text)");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Tuple(ref elements) => {
                    assert_eq!(elements.len(), 2, "Expected 2-element tuple");
                }
                _ => panic!("Expected tuple type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_checked_reference_with_refinement() {
    // NOTE: &checked Int{> 0} parses as Refined { base: CheckedReference { Int } }
    // This is because refinements on references apply to the REFERENCE type itself,
    // not the inner type. This behavior also prevents HRTB parsing issues where
    // function body braces would be incorrectly consumed as refinements.
    // To get a reference to a refined type, use explicit parens: &checked (Int{> 0})
    let ty = parse_type("&checked Int{> 0}");

    match ty.kind {
        TypeKind::Refined { ref base, .. } => {
            match base.kind {
                TypeKind::CheckedReference { mutable, ref inner } => {
                    assert!(!mutable, "Expected immutable checked reference");
                    match inner.kind {
                        TypeKind::Int => {
                            // Correct - the inner type is plain Int
                        }
                        _ => panic!("Expected Int inner type, got {:?}", inner.kind),
                    }
                }
                _ => panic!("Expected CheckedReference as base, got {:?}", base.kind),
            }
        }
        _ => panic!("Expected Refined type, got {:?}", ty.kind),
    }
}

// === UNSAFE REFERENCE TESTS (&unsafe T) ===

#[test]
fn test_parse_type_unsafe_reference() {
    let ty = parse_type("&unsafe Int");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_mutable_reference() {
    let ty = parse_type("&unsafe mut Int");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(mutable, "Expected mutable unsafe reference");

            match inner.kind {
                TypeKind::Int => {
                    // Correct
                }
                _ => panic!("Expected Int type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected mutable UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_reference_generic() {
    let ty = parse_type("&unsafe List<Text>");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct - generic type
                }
                _ => panic!("Expected generic type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_reference_slice() {
    let ty = parse_type("&unsafe [Float]");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Slice(ref element) => {
                    match element.kind {
                        TypeKind::Float => {
                            // Correct
                        }
                        _ => panic!("Expected Float slice element"),
                    }
                }
                _ => panic!("Expected slice type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_reference_raw_pointer() {
    let ty = parse_type("&unsafe *const Int");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Pointer {
                    mutable: ptr_mut, ..
                } => {
                    assert!(!ptr_mut, "Expected const pointer");
                }
                _ => panic!("Expected pointer type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_reference_array() {
    let ty = parse_type("&unsafe [Int; 10]");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Array { .. } => {
                    // Correct
                }
                _ => panic!("Expected array type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_unsafe_reference_ffi() {
    // Test with path type that could be FFI type
    let ty = parse_type("&unsafe CString");

    match ty.kind {
        TypeKind::UnsafeReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable unsafe reference");

            match inner.kind {
                TypeKind::Path { .. } => {
                    // Correct - path type (could be FFI)
                }
                _ => panic!("Expected path type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected UnsafeReference, got {:?}", ty.kind),
    }
}

// === COMPLEX COMBINATIONS ===

#[test]
fn test_parse_type_nested_references() {
    // &checked reference to another reference
    let ty = parse_type("&checked &Int");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(!mutable, "Expected immutable checked reference");

            match inner.kind {
                TypeKind::Reference { .. } => {
                    // Correct - nested reference
                }
                _ => panic!("Expected inner reference type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_mixed_references() {
    // &checked mut reference to &unsafe reference
    let ty = parse_type("&checked mut &unsafe Int");

    match ty.kind {
        TypeKind::CheckedReference { mutable, ref inner } => {
            assert!(mutable, "Expected mutable checked reference");

            match inner.kind {
                TypeKind::UnsafeReference { .. } => {
                    // Correct - nested unsafe reference
                }
                _ => panic!("Expected inner unsafe reference type, got {:?}", inner.kind),
            }
        }
        _ => panic!("Expected CheckedReference, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_reference_in_function() {
    let ty = parse_type("fn(&checked Int) -> &unsafe Text");

    match ty.kind {
        TypeKind::Function {
            ref params,
            ref return_type,
            ..
        } => {
            // Check parameter is &checked Int
            assert_eq!(params.len(), 1, "Expected 1 parameter");
            match params[0].kind {
                TypeKind::CheckedReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected checked reference parameter"),
            }

            // Check return type is &unsafe Text
            match return_type.kind {
                TypeKind::UnsafeReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected unsafe reference return type"),
            }
        }
        _ => panic!("Expected function type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_reference_tuple() {
    let ty = parse_type("(&checked Int, &unsafe Text)");

    match ty.kind {
        TypeKind::Tuple(ref elements) => {
            assert_eq!(elements.len(), 2, "Expected 2-element tuple");

            // First element should be &checked Int
            match elements[0].kind {
                TypeKind::CheckedReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected checked reference in first tuple element"),
            }

            // Second element should be &unsafe Text
            match elements[1].kind {
                TypeKind::UnsafeReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected unsafe reference in second tuple element"),
            }
        }
        _ => panic!("Expected tuple type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_reference_array() {
    let ty = parse_type("[&checked Int; 10]");

    match ty.kind {
        TypeKind::Array { ref element, .. } => {
            match element.kind {
                TypeKind::CheckedReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected checked reference array element"),
            }
        }
        _ => panic!("Expected array type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_reference_slice() {
    let ty = parse_type("[&unsafe Int]");

    match ty.kind {
        TypeKind::Slice(ref element) => {
            match element.kind {
                TypeKind::UnsafeReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected unsafe reference slice element"),
            }
        }
        _ => panic!("Expected slice type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_complex_nested() {
    // fn(List<&checked Int>) -> &unsafe Map<Text, &Int>
    let ty = parse_type("fn(List<&checked Int>) -> &unsafe Map<Text, &Int>");

    match ty.kind {
        TypeKind::Function {
            ref params,
            ref return_type,
            ..
        } => {
            assert_eq!(params.len(), 1, "Expected 1 parameter");

            // Parameter is List<&checked Int>
            match params[0].kind {
                TypeKind::Generic { .. } | TypeKind::Path { .. } => {
                    // Correct
                }
                _ => panic!("Expected generic type parameter"),
            }

            // Return type is &unsafe Map<Text, &Int>
            match return_type.kind {
                TypeKind::UnsafeReference { .. } => {
                    // Correct
                }
                _ => panic!("Expected unsafe reference return type"),
            }
        }
        _ => panic!("Expected function type, got {:?}", ty.kind),
    }
}

// === ERROR CASES ===

#[test]
fn test_parse_type_checked_without_ampersand() {
    // "checked Int" should fail - must have & prefix
    assert!(
        expect_parse_error("checked Int"),
        "Expected parse error for 'checked Int' without ampersand"
    );
}

#[test]
fn test_parse_type_unsafe_without_ampersand() {
    // "unsafe Int" should fail - must have & prefix
    assert!(
        expect_parse_error("unsafe Int"),
        "Expected parse error for 'unsafe Int' without ampersand"
    );
}

#[test]
fn test_parse_type_mut_without_ampersand() {
    // "mut Int" should fail - must have & prefix
    assert!(
        expect_parse_error("mut Int"),
        "Expected parse error for 'mut Int' without ampersand"
    );
}

// === TUPLE TYPE TESTS ===

#[test]
fn test_parse_type_tuple_two_elements() {
    let ty = parse_type("(Int, Text)");
    match ty.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2, "Expected two elements");
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_parse_type_tuple_three_elements() {
    let ty = parse_type("(Int, Text, Bool)");
    match ty.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 3, "Expected three elements");
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_parse_type_tuple_nested() {
    let ty = parse_type("((Int, Text), Bool)");
    match ty.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2, "Expected two elements");
        }
        _ => panic!("Expected tuple type"),
    }
}

// === ARRAY/SLICE TYPE TESTS ===

#[test]
fn test_parse_type_slice() {
    let ty = parse_type("[Int]");
    match ty.kind {
        TypeKind::Slice { .. } => {
            // Correct
        }
        _ => panic!("Expected slice type"),
    }
}

#[test]
fn test_parse_type_array_fixed() {
    let ty = parse_type("[Int; 10]");
    match ty.kind {
        TypeKind::Array { .. } => {
            // Correct
        }
        _ => panic!("Expected array type"),
    }
}

#[test]
fn test_parse_type_array_generic() {
    let ty = parse_type("[List<Int>]");
    match ty.kind {
        TypeKind::Slice { .. } => {
            // Correct
        }
        _ => panic!("Expected slice type"),
    }
}

// === COMPLEX NESTED TYPE TESTS ===

#[test]
fn test_parse_type_reference_to_function() {
    let ty = parse_type("&fn(Int) -> Int");
    match ty.kind {
        TypeKind::Reference { .. } => {
            // Correct
        }
        _ => panic!("Expected reference type"),
    }
}

#[test]
fn test_parse_type_tuple_of_references() {
    let ty = parse_type("(&Int, &Text)");
    match ty.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2, "Expected two elements");
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_parse_type_function_with_generic_params() {
    let ty = parse_type("fn(List<Int>, Map<Text, Bool>) -> Int");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_parse_type_generic_with_refinement() {
    let ty = parse_type("List<Int{> 0}>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Should parse generic with refined type parameter
        }
        _ => panic!("Expected generic type with refinement"),
    }
}

// === SIGMA TYPE TESTS (Five Binding Rules - Rule 3) ===

#[test]
fn test_parse_type_sigma() {
    // Per VUVA §5 the sigma surface form parses to `TypeKind::Refined`
    // with `predicate.binding = Some(name)`.
    let ty = parse_type("x: Int where x > 0");
    match ty.kind {
        TypeKind::Refined { ref predicate, .. } => {
            assert!(
                matches!(predicate.binding, verum_common::Maybe::Some(_)),
                "Expected sigma refinement with explicit binder"
            );
        }
        _ => panic!("Expected refined type (sigma surface form)"),
    }
}

#[test]
fn test_parse_type_sigma_with_complex_predicate() {
    let ty = parse_type("x: List<Int> where is_sorted(x)");
    match ty.kind {
        TypeKind::Refined { ref predicate, .. } => {
            assert!(
                matches!(predicate.binding, verum_common::Maybe::Some(_)),
                "Expected sigma refinement with explicit binder"
            );
        }
        _ => panic!("Expected refined type (sigma surface form)"),
    }
}

// === INFERRED TYPE TESTS ===

#[test]
fn test_parse_type_inferred() {
    let ty = parse_type("_");
    match ty.kind {
        TypeKind::Inferred => {
            // Correct
        }
        _ => panic!("Expected inferred type"),
    }
}

// === PATH-BASED TYPE TESTS ===

#[test]
fn test_parse_type_simple_path() {
    let ty = parse_type("MyType");
    match ty.kind {
        TypeKind::Path { .. } => {
            // Correct
        }
        _ => panic!("Expected path type"),
    }
}

#[test]
fn test_parse_type_qualified_path() {
    let ty = parse_type("module.Type");
    match ty.kind {
        TypeKind::Path { .. } => {
            // Ideally correct - simple multi-segment path
            // However, the parser doesn't currently reach this case
        }
        TypeKind::Qualified { .. } => {
            // Current behavior - parser treats module.Type as qualified syntax
            // This is a known limitation: multi-segment paths are treated as associated types
            // The parser cannot distinguish module.Type from T.Item without type context
        }
        _ => panic!("Expected path or qualified type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_path_with_generics() {
    let ty = parse_type("Container<A, B, C>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Correct
        }
        _ => panic!("Expected path type with generics"),
    }
}

// === EDGE CASES ===

#[test]
fn test_parse_type_deeply_nested_generics() {
    let ty = parse_type("Map<List<Set<Int>>, Option<Text>>");
    match ty.kind {
        TypeKind::Generic { .. } | TypeKind::Path { .. } => {
            // Correct
        }
        _ => panic!("Expected deeply nested generic type"),
    }
}

#[test]
fn test_parse_type_reference_to_refinement() {
    // NOTE: &Int{> 0} parses as Refined { base: Reference { Int } }
    // Refinements on references apply to the reference type itself.
    // For a reference to a refined type, use parens: &(Int{> 0})
    let ty = parse_type("&Int{> 0}");
    match ty.kind {
        TypeKind::Refined { ref base, .. } => {
            match base.kind {
                TypeKind::Reference { .. } => {
                    // Correct - the base is a reference to Int
                }
                _ => panic!("Expected Reference as base, got {:?}", base.kind),
            }
        }
        _ => panic!("Expected Refined type, got {:?}", ty.kind),
    }
}

#[test]
fn test_parse_type_function_returning_tuple() {
    let ty = parse_type("fn(Int) -> (Int, Text)");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function returning tuple"),
    }
}

#[test]
fn test_parse_type_function_with_refinement_params() {
    let ty = parse_type("fn(Int{> 0}, Text{is_email(it)}) -> Bool");
    match ty.kind {
        TypeKind::Function { .. } => {
            // Correct
        }
        _ => panic!("Expected function with refinement parameters"),
    }
}
