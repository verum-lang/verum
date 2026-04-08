//! Integration tests for the differential testing runner
//!
//! These tests verify the end-to-end functionality of the differential
//! testing infrastructure.

use vcs_differential_runner::{
    EquivalenceConfig, NormalizationConfig, Normalizer, SemanticEquivalenceChecker,
};

#[test]
fn test_normalizer_basic() {
    let normalizer = Normalizer::new(NormalizationConfig::default());

    let input = "Object at 0x7fff1234abcd\nValue: 3.14159265\n";
    let output = normalizer.normalize(input);

    // Addresses should be normalized
    assert!(output.contains("<ADDR>"));
    // Floats should be normalized
    assert!(output.contains("3."));
}

#[test]
fn test_normalizer_timestamps() {
    let normalizer = Normalizer::new(NormalizationConfig::default());

    let input = "Event at 2024-01-15T10:30:45.123Z";
    let output = normalizer.normalize(input);

    assert!(output.contains("<TIME>"));
}

#[test]
fn test_normalizer_ansi_codes() {
    let normalizer = Normalizer::new(NormalizationConfig::default());

    let input = "\x1B[31mError\x1B[0m: Something went wrong";
    let output = normalizer.normalize(input);

    assert!(!output.contains("\x1B"));
    assert!(output.contains("Error"));
}

#[test]
fn test_semantic_equivalence_identical() {
    let checker = SemanticEquivalenceChecker::new(EquivalenceConfig::default());

    let a = "hello world\n42\n";
    let b = "hello world\n42\n";

    let result = checker.check(a, b);
    assert!(result.is_equivalent());
}

#[test]
fn test_semantic_equivalence_float_tolerance() {
    let checker = SemanticEquivalenceChecker::new(EquivalenceConfig::default());

    // Use values within the default epsilon (1e-10)
    let a = "1.00000000001";
    let b = "1.00000000002";

    let result = checker.check(a, b);
    // With default epsilon (1e-10), these should be equivalent
    assert!(result.is_equivalent());
}

#[test]
fn test_semantic_equivalence_different() {
    let checker = SemanticEquivalenceChecker::new(EquivalenceConfig::default());

    let a = "hello";
    let b = "world";

    let result = checker.check(a, b);
    assert!(!result.is_equivalent());
}

#[test]
fn test_normalizer_config_modes() {
    // Test exact mode (minimal normalization)
    let exact_normalizer = Normalizer::new(NormalizationConfig::exact());
    let input = "0x7fff1234abcd";
    let output = exact_normalizer.normalize(input);
    assert!(output.contains("0x7fff")); // Should not be normalized

    // Test aggressive mode (maximum normalization)
    let aggressive_normalizer = Normalizer::new(NormalizationConfig::aggressive());
    let output = aggressive_normalizer.normalize(input);
    assert!(output.contains("<ADDR>")); // Should be normalized
}

#[test]
fn test_normalizer_line_endings() {
    let normalizer = Normalizer::new(NormalizationConfig::default());

    let input = "line1\r\nline2\rline3";
    let output = normalizer.normalize(input);

    // All line endings should be normalized to \n
    assert!(!output.contains("\r"));
    assert_eq!(output.matches('\n').count(), 2);
}
