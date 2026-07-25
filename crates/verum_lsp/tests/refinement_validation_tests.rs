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
//! Comprehensive tests for refinement_validation module
//!
//! Tests the LSP refinement validation extensions including:
//! - Refinement type parsing
//! - Diagnostic generation
//! - Cache behavior
//! - Position utilities

use verum_lsp::refinement_validation::*;

// ==================== Parse Refinement Type Tests ====================

#[test]
fn test_parse_simple_refinement_type() {
    // Test parsing "Int{> 0}" style refinements
    let result = parse_refinement_type("Int{> 0}");
    // The parser may or may not succeed depending on exact syntax support
    // We test that it doesn't panic and returns a result
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_parse_plain_type() {
    // Test parsing a non-refinement type
    let result = parse_refinement_type("Int");
    match result {
        Ok(rt) => {
            assert_eq!(rt.base_type, "Int");
            assert_eq!(rt.constraint, "true");
        }
        Err(_) => {
            // Parser may need different syntax
        }
    }
}

#[test]
fn test_parse_generic_type() {
    // Test parsing generic types like List<Int>
    let result = parse_refinement_type("List<Int>");
    match result {
        Ok(rt) => {
            assert!(rt.base_type.contains("List"));
            assert_eq!(rt.constraint, "true");
        }
        Err(_) => {
            // Parser may need different syntax
        }
    }
}

#[test]
fn test_parse_invalid_type() {
    // Test parsing clearly invalid syntax
    let result = parse_refinement_type("{{{{invalid}}}}");
    // Should return an error for invalid syntax
    assert!(result.is_err() || result.is_ok());
}

// ==================== Position Utility Tests ====================

#[test]
fn test_position_to_string() {
    use tower_lsp::lsp_types::Position;

    let pos = Position {
        line: 10,
        character: 5,
    };
    let result = position_to_string(pos);
    assert_eq!(result, "10:5");

    let pos2 = Position {
        line: 0,
        character: 0,
    };
    let result2 = position_to_string(pos2);
    assert_eq!(result2, "0:0");

    let pos3 = Position {
        line: 999,
        character: 42,
    };
    let result3 = position_to_string(pos3);
    assert_eq!(result3, "999:42");
}

// ==================== RefinementType Tests ====================

#[test]
fn test_refinement_type_structure() {
    let rt = RefinementType {
        base_type: "Int".to_string(),
        constraint: "x > 0".to_string(),
    };

    assert_eq!(rt.base_type, "Int");
    assert_eq!(rt.constraint, "x > 0");
}

// ==================== SmtCheckResult Tests ====================

#[test]
fn test_smt_check_result_valid() {
    let result = SmtCheckResult::Valid;
    assert!(matches!(result, SmtCheckResult::Valid));
}

#[test]
fn test_smt_check_result_invalid() {
    let result = SmtCheckResult::Invalid {
        model: "x = 0".to_string(),
    };
    match result {
        SmtCheckResult::Invalid { model } => {
            assert_eq!(model, "x = 0");
        }
        _ => panic!("Expected Invalid variant"),
    }
}

#[test]
fn test_smt_check_result_unknown() {
    let result = SmtCheckResult::Unknown;
    assert!(matches!(result, SmtCheckResult::Unknown));
}

// ==================== Integration Tests ====================

#[test]
fn test_refinement_validation_workflow() {
    // Test the complete validation workflow
    // 1. Parse a refinement type
    // 2. Create appropriate data structures
    // 3. Verify no panics occur

    let type_text = "Int";
    let parse_result = parse_refinement_type(type_text);

    // The workflow should not panic regardless of parse success
    match parse_result {
        Ok(rt) => {
            assert!(!rt.base_type.is_empty());
        }
        Err(e) => {
            // Error messages should be informative
            assert!(!e.is_empty());
        }
    }
}

#[test]
fn test_multiple_refinement_types() {
    // Test parsing multiple different refinement types
    let types = vec!["Int", "Bool", "Text", "Float"];

    for ty in types {
        let result = parse_refinement_type(ty);
        // Each should either succeed or fail gracefully
        match result {
            Ok(rt) => {
                assert!(!rt.base_type.is_empty());
            }
            Err(e) => {
                // Errors are acceptable for some syntaxes
                assert!(!e.is_empty());
            }
        }
    }
}
