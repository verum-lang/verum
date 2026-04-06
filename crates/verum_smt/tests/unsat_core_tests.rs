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
// Tests for unsat_core module
// Migrated from src/unsat_core.rs per CLAUDE.md standards
// FIXED (Session 23): Tests enabled
#![allow(unexpected_cfgs)]
// #![cfg(feature = "unsat_core_tests_disabled")]

use verum_common::{Maybe, Text};
use z3::ast::Bool;

use verum_smt::unsat_core::*;

#[test]
fn test_unsat_core_extraction() {
    let mut extractor = UnsatCoreExtractor::new(Default::default());

    // Create contradictory assertions using the z3 global context
    let x = Bool::new_const("x");
    let not_x = x.not();

    extractor.track(TrackedAssertion {
        id: Text::from("A1"),
        assertion: x.clone(),
        source: Maybe::Some(Text::from("test.rs:10")),
        category: AssertionCategory::Precondition,
        description: Maybe::Some(Text::from("x must be true")),
    });

    extractor.track(TrackedAssertion {
        id: Text::from("A2"),
        assertion: not_x,
        source: Maybe::Some(Text::from("test.rs:11")),
        category: AssertionCategory::Precondition,
        description: Maybe::Some(Text::from("x must be false")),
    });

    // Extract core should find both assertions
    let result = extractor.extract_core();
    assert!(result.is_ok());
    if let Ok(core) = result {
        assert_eq!(core.core.len(), 2);
    }
}
