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
//! Comprehensive tests for CBGR predicates
//!
//! Tests cover:
//! - Generation counter operations
//! - Epoch extraction
//! - Reference validity checking
//! - Same allocation checking
//! - Monotonicity verification
//! - Epoch increment verification
//! - Integration with refinement verification

// FIXED (Session 24): Use GenerationPredicate from verum_smt, not verum_protocol_types

use verum_ast::span::Span;
use verum_ast::ty::Type;
use verum_smt::cbgr_predicates::{
    CBGRPredicateEncoder, GenerationPredicate, encode_generation_counter, extract_epoch,
    extract_generation_value, is_valid_reference, verify_generation_property,
    verify_generation_refinement,
};
use verum_smt::{CBGRCounterexample, ReferenceValue};

#[test]
fn test_encoder_creation() {
    let encoder = CBGRPredicateEncoder::new();
    // Should create successfully
}

#[test]
fn test_extract_generation_value() {
    // Generation value is lower 48 bits
    let generation = 0x0000_0000_0000_002A; // 42
    assert_eq!(extract_generation_value(generation), 42);

    let gen_with_epoch = 0x0001_0000_0000_002A; // epoch=1, gen=42
    assert_eq!(extract_generation_value(gen_with_epoch), 42);
}

#[test]
fn test_extract_epoch() {
    // Epoch is bits 48-63
    let gen_with_epoch = 0x0001_0000_0000_0000; // epoch=1
    assert_eq!(extract_epoch(gen_with_epoch), 1);

    let gen_with_epoch_max = 0xFFFF_0000_0000_0000; // epoch=65535
    assert_eq!(extract_epoch(gen_with_epoch_max), 65535);
}

#[test]
fn test_encode_generation_counter() {
    let gen_value = 42;
    let epoch = 1;

    let encoded = encode_generation_counter(gen_value, epoch);

    assert_eq!(extract_generation_value(encoded), 42);
    assert_eq!(extract_epoch(encoded), 1);
}

#[test]
fn test_generation_counter_roundtrip() {
    let gen_value = 12345;
    let epoch = 99;

    let encoded = encode_generation_counter(gen_value, epoch);
    let decoded_gen = extract_generation_value(encoded);
    let decoded_epoch = extract_epoch(encoded);

    assert_eq!(decoded_gen, gen_value);
    assert_eq!(decoded_epoch, epoch);
}

#[test]
fn test_is_valid_reference_valid() {
    assert!(is_valid_reference(42, 100));
    assert!(is_valid_reference(0, 0));
    assert!(is_valid_reference(100, 100));
}

#[test]
fn test_is_valid_reference_invalid() {
    assert!(!is_valid_reference(100, 42));
    assert!(!is_valid_reference(1, 0));
}

#[test]
fn test_verify_generation_property() {
    let property = GenerationPredicate::Generation {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);
    // Should complete verification
}

#[test]
fn test_verify_epoch_property() {
    let property = GenerationPredicate::Epoch {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);
    assert_eq!(result.stats.epoch_checks, 1);
}

#[test]
fn test_verify_valid_property() {
    let property = GenerationPredicate::Valid {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);
    assert_eq!(result.stats.validity_checks, 1);
}

#[test]
fn test_verify_same_allocation_property() {
    let property = GenerationPredicate::SameAllocation {
        ref_a: Box::new(Type::int(Span::dummy())),
        ref_b: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);
    assert_eq!(result.stats.allocation_checks, 1);
}

#[test]
fn test_generation_refinement() {
    let result = verify_generation_refinement("generation(ref) >= 0");
    assert!(result.is_valid);
}

#[test]
fn test_default_encoder() {
    let encoder = CBGRPredicateEncoder::default();
    // Should create successfully
}

#[test]
fn test_generation_bounds() {
    // Max generation value (48 bits)
    let max_gen = 0x0000_FFFF_FFFF_FFFF;
    let encoded = encode_generation_counter(max_gen, 0);
    assert_eq!(extract_generation_value(encoded), max_gen);
}

#[test]
fn test_epoch_bounds() {
    // Max epoch (16 bits)
    let max_epoch: u16 = 0xFFFF;
    let encoded = encode_generation_counter(0, max_epoch);
    assert_eq!(extract_epoch(encoded), max_epoch);
}

#[test]
fn test_generation_overflow_protection() {
    // Generation values beyond 48 bits should be masked
    let overflow_value = 0x0001_0000_0000_0000; // Bit 48 set
    let encoded = encode_generation_counter(overflow_value, 0);

    // Should mask to 48 bits
    assert_eq!(extract_generation_value(encoded), 0);
}

#[test]
fn test_epoch_increment() {
    let gen1 = encode_generation_counter(100, 1);
    let gen2 = encode_generation_counter(100, 2);

    assert_eq!(
        extract_generation_value(gen1),
        extract_generation_value(gen2)
    );
    assert!(extract_epoch(gen2) > extract_epoch(gen1));
}

#[test]
fn test_generation_monotonicity_concept() {
    // Later generations should have higher values
    let gen1 = encode_generation_counter(100, 0);
    let gen2 = encode_generation_counter(101, 0);

    assert!(extract_generation_value(gen2) > extract_generation_value(gen1));
}

#[test]
fn test_reference_value_creation() {
    // ReferenceValue is re-exported at crate level, not from cbgr_predicates module
    let ref_val = ReferenceValue {
        ptr: 0x1000,
        generation: 42,
        epoch: 1,
        is_valid: true,
    };

    assert_eq!(ref_val.ptr, 0x1000);
    assert_eq!(ref_val.generation, 42);
    assert_eq!(ref_val.epoch, 1);
    assert!(ref_val.is_valid);
}

#[test]
fn test_verification_statistics() {
    let property = GenerationPredicate::Generation {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);

    assert_eq!(result.stats.generation_checks, 1);
    assert_eq!(result.stats.epoch_checks, 0);
    assert_eq!(result.stats.validity_checks, 0);
    assert_eq!(result.stats.allocation_checks, 0);
}

#[test]
fn test_multiple_property_verification() {
    let encoder = CBGRPredicateEncoder::new();

    let prop1 = GenerationPredicate::Generation {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let prop2 = GenerationPredicate::Epoch {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result1 = encoder.verify_property(&prop1);
    let result2 = encoder.verify_property(&prop2);

    assert_eq!(result1.stats.generation_checks, 1);
    assert_eq!(result2.stats.epoch_checks, 1);
}

#[test]
fn test_zero_generation() {
    let generation = encode_generation_counter(0, 0);
    assert_eq!(extract_generation_value(generation), 0);
    assert_eq!(extract_epoch(generation), 0);
}

#[test]
fn test_verification_timing() {
    let property = GenerationPredicate::Valid {
        ref_expr: Box::new(Type::int(Span::dummy())),
    };

    let result = verify_generation_property(&property);

    // Should be fast (<50ms target)
    assert!(result.duration.as_millis() < 100);
}

#[test]
fn test_counterexample_fields() {
    use verum_common::Map;

    let ce = CBGRCounterexample {
        ref_values: Map::new(),
        violated_property: "generation(ref) == 42".into(),
        explanation: "Test explanation".into(),
    };

    assert_eq!(ce.violated_property.as_str(), "generation(ref) == 42");
}

#[test]
fn test_all_predicate_types() {
    let encoder = CBGRPredicateEncoder::new();

    let predicates = vec![
        GenerationPredicate::Generation {
            ref_expr: Box::new(Type::int(Span::dummy())),
        },
        GenerationPredicate::Epoch {
            ref_expr: Box::new(Type::int(Span::dummy())),
        },
        GenerationPredicate::Valid {
            ref_expr: Box::new(Type::int(Span::dummy())),
        },
        GenerationPredicate::SameAllocation {
            ref_a: Box::new(Type::int(Span::dummy())),
            ref_b: Box::new(Type::int(Span::dummy())),
        },
    ];

    for predicate in predicates {
        let _ = encoder.verify_property(&predicate);
    }
}
