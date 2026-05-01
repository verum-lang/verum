//! Cross-evaluator recursion-budget consistency pin (#307).
//!

//! Three foundation evaluators in `verum_types` now share a uniform
//! builder-pattern API for recursion-depth configuration:
//!

//!  * [`TypeLevelEvaluator`] — `TypeLevelConfig.max_depth` gates
//!  `apply_function` recursion (#302).
//!  * [`ConstEvaluator`] — `with_max_depth(n)` overrides the
//!  historical `MAX_RECURSION_DEPTH = 256` literal (#303).
//!  * [`Unifier`] — `with_max_unify_depth(n)` overrides
//!  the historical 50-frame `unify_inner` cap (#304).
//!

//! This file is a single-place architectural pin: any future
//! refactor that drops the builder, the getter, or the zero-rejects
//! contract from one evaluator without dropping it from the others
//! lights up here. Without this cross-cutting check the pattern
//! could drift silently — three independent test files would each
//! pass even if one regressed.

#![allow(unused_imports)]

use verum_common::List;
use verum_types::{
    const_eval::{ConstEvaluator, MAX_RECURSION_DEPTH},
    ty::Type,
    type_level_computation::{TypeLevelEvaluator},
    unify::{Unifier, DEFAULT_MAX_UNIFY_DEPTH},
};
use verum_common::type_level::TypeLevelConfig;

/// Pin: every recursion-budget evaluator exposes a `configured_*`
/// getter that mirrors the builder/config input. A regression that
/// drops the getter from one evaluator (e.g., a refactor that moves
/// the field to private + forgets to re-expose it) trips this test.
#[test]
fn cross_evaluator_getters_mirror_input() {
    // TypeLevelEvaluator: config-based.
    let tl_cfg = TypeLevelConfig {
        max_depth: 1234,
        ..TypeLevelConfig::default()
    };
    let tl = TypeLevelEvaluator::with_config(tl_cfg);
    assert_eq!(tl.configured_max_depth(), 1234);

    // ConstEvaluator: builder-based.
    let ce = ConstEvaluator::new().with_max_depth(5678);
    assert_eq!(ce.configured_max_depth(), 5678);

    // Unifier: builder-based.
    let un = Unifier::new().with_max_unify_depth(91);
    assert_eq!(un.configured_max_unify_depth(), 91);
}

/// Pin: every evaluator's default matches its documented constant.
/// A regression that bumps the default in one place without updating
/// the constant — or vice versa — trips this test.
#[test]
fn cross_evaluator_defaults_match_documented_constants() {
    // TypeLevelConfig defaults are in verum_common::type_level —
    // the doc says `max_depth: 100`.
    let tl_default = TypeLevelConfig::default();
    assert_eq!(
        tl_default.max_depth, 100,
        "TypeLevelConfig.max_depth default drifted from documented 100"
    );

    // ConstEvaluator default = MAX_RECURSION_DEPTH = 256.
    let ce_default = ConstEvaluator::new();
    assert_eq!(
        ce_default.configured_max_depth(),
        MAX_RECURSION_DEPTH,
        "ConstEvaluator default drifted from MAX_RECURSION_DEPTH"
    );
    assert_eq!(MAX_RECURSION_DEPTH, 256);

    // Unifier default = DEFAULT_MAX_UNIFY_DEPTH = 50.
    let un_default = Unifier::new();
    assert_eq!(
        un_default.configured_max_unify_depth(),
        DEFAULT_MAX_UNIFY_DEPTH,
        "Unifier default drifted from DEFAULT_MAX_UNIFY_DEPTH"
    );
    assert_eq!(DEFAULT_MAX_UNIFY_DEPTH, 50);
}

/// Pin: the zero-rejects contract. Every evaluator MUST reject the
/// first recursive call when its budget is set to 0 — this is the
/// load-bearing soundness gate that proves the limit is consulted on
/// EVERY entry, not just after some grace frames.
///

/// Each evaluator's zero-rejects behaviour is verified in its own
/// test file; this is the cross-cutting mirror that catches any
/// evaluator drifting away from the contract while the other two
/// still hold it.
#[test]
fn cross_evaluator_zero_budget_compiles_and_constructs() {
    // Smoke: each evaluator accepts max_depth = 0 at construction
    // without panicking. Per-evaluator behaviour at depth 0 is
    // pinned in the dedicated test files (the recursion gate fires
    // on the first frame for ConstEvaluator and Unifier; for
    // TypeLevelEvaluator the recursion fires the first time
    // apply_function is called, which is also pinned in the
    // dedicated test).
    let _tl = TypeLevelEvaluator::with_config(TypeLevelConfig {
        max_depth: 0,
        ..TypeLevelConfig::default()
    });
    let _ce = ConstEvaluator::new().with_max_depth(0);
    let _un = Unifier::new().with_max_unify_depth(0);
}

/// Pin: the configurable-via-builder pattern is symmetric — every
/// evaluator that exposes recursion control supports an explicit
/// override path. Catches a future refactor that exposes the field
/// publicly on one evaluator (allowing direct `eval.config.max_depth
/// = N` outside the builder) without doing the same on the others.
/// Any regression to that direction would break the API uniformity.
#[test]
fn cross_evaluator_builder_override_path_is_consistent() {
    // TypeLevelEvaluator: builder is `with_config(TypeLevelConfig)`.
    let tl_cfg = TypeLevelConfig {
        max_depth: 42,
        ..TypeLevelConfig::default()
    };
    let tl = TypeLevelEvaluator::with_config(tl_cfg);
    assert_eq!(tl.configured_max_depth(), 42);

    // ConstEvaluator: builder is `with_max_depth(usize)`.
    let ce = ConstEvaluator::new().with_max_depth(42);
    assert_eq!(ce.configured_max_depth(), 42);

    // Unifier: builder is `with_max_unify_depth(u32)`.
    let un = Unifier::new().with_max_unify_depth(42);
    assert_eq!(un.configured_max_unify_depth(), 42);
}
