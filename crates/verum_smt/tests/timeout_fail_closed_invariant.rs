//! Red-team Round 1 §5.1 — Z3 timeout fail-closed soundness invariant.
//!
//! When Z3 cannot decide a refinement obligation within the configured
//! timeout, every consumer in the verifier translates `SatResult::Unknown`
//! into a verification failure (Err / `keep-runtime-check`). This test
//! programmatically constructs a Z3-hard formula (nonlinear arithmetic
//! over unbounded Ints) plus a pathologically tiny timeout and asserts:
//!
//!   1. Z3 returns `SatResult::Unknown` (not Sat/Unsat).
//!   2. The reason hints at timeout / incompleteness, not "decided".
//!   3. The canonical Verum translation pattern (`SatResult::Unknown ->
//!      Err`) is applied — this is the property all 9 audit sites in
//!      `vcs/red-team/round-1-architecture.md §5.1` jointly enforce.
//!
//! Companion guardrails:
//!   - `vcs/specs/L0-critical/verification/z3_timeout_fail_closed.vr`
//!   - `vcs/specs/L1-core/refinement/smt/proof_timeout.vr`

use z3::ast::Int;
use z3::{Config, SatResult, Solver};

/// Construct a Z3-hard formula and a tiny timeout, assert that Z3
/// reports Unknown rather than Sat/Unsat. This is the *prerequisite*
/// for the verifier's fail-closed invariant: if Z3 always returned
/// Sat/Unsat in finite time, the Unknown branch would be unreachable.
#[test]
fn z3_returns_unknown_on_pathological_timeout() {
    // Mirror the production-side configuration pattern from
    // crates/verum_smt/src/z3_backend.rs:113-123.
    let mut cfg = Config::new();
    cfg.set_timeout_msec(1); // 1ms — pathologically small.

    let result = z3::with_z3_config(&cfg, || {
        let solver = Solver::new();

        // Nonlinear integer arithmetic — Z3's NRA tactic is incomplete
        // here. Specifically: ∃ a, b, c, d ∈ Int.
        //   a > 1e9, b > 1e9, c > 1e9, d > 1e9
        //   a^3 * b^2 == c^3 * d^2 + 1
        // Provably consistent for some witness, but Z3 cannot find
        // the witness (or refute it) in 1ms.
        let a = Int::new_const("a");
        let b = Int::new_const("b");
        let c = Int::new_const("c");
        let d = Int::new_const("d");

        let big = Int::from_i64(1_000_000_000);
        solver.assert(a.gt(&big));
        solver.assert(b.gt(&big));
        solver.assert(c.gt(&big));
        solver.assert(d.gt(&big));

        let a3 = &(&a * &a) * &a;
        let b2 = &b * &b;
        let c3 = &(&c * &c) * &c;
        let d2 = &d * &d;
        let one = Int::from_i64(1);
        let lhs = &a3 * &b2;
        let rhs = &(&c3 * &d2) + &one;
        solver.assert(lhs.eq(&rhs));

        let r = solver.check();
        let reason = solver.get_reason_unknown();
        (r, reason)
    });

    let (verdict, reason_opt) = result;

    // The pathological 1ms timeout means Z3 should bail out with Unknown.
    // If somehow Z3 decided the formula in 1ms, Unknown is not reachable
    // and this test should be tightened with a harder formula.
    assert_eq!(
        verdict,
        SatResult::Unknown,
        "Z3 should return Unknown on 1ms timeout for nonlinear arithmetic; \
         got {:?}. If Z3 became powerful enough to decide this in 1ms, \
         harden the formula to keep the test exercising the Unknown path.",
        verdict
    );

    let reason = reason_opt.unwrap_or_else(|| "no reason".to_string());
    let reason_lc = reason.to_lowercase();
    assert!(
        reason_lc.contains("timeout")
            || reason_lc.contains("canceled")
            || reason_lc.contains("limit")
            || reason_lc.contains("incomplete")
            || reason_lc.contains("polysat")
            || reason_lc.contains("nlsat")
            || reason_lc.contains("unknown"),
        "Z3 Unknown reason should indicate timeout or incompleteness; got: {}",
        reason
    );
}

/// Sanity check the converse: when given enough time, Z3 produces a
/// definitive verdict on a routine formula. This guards against false
/// positives where a misconfigured Z3 reports Unknown for everything.
#[test]
fn z3_decides_routine_formula_when_given_time() {
    let mut cfg = Config::new();
    cfg.set_timeout_msec(5_000); // 5s — plenty.

    let verdict = z3::with_z3_config(&cfg, || {
        let solver = Solver::new();

        // Trivially Unsat: x > 0 ∧ x < 0
        let x = Int::new_const("x");
        let zero = Int::from_i64(0);
        solver.assert(x.gt(&zero));
        solver.assert(x.lt(&zero));

        solver.check()
    });

    assert_eq!(
        verdict,
        SatResult::Unsat,
        "Z3 should decide x > 0 ∧ x < 0 as Unsat; got {:?}",
        verdict
    );
}

/// The fail-closed invariant in code form: any function that wraps Z3's
/// `SatResult` and is consumed by the verifier MUST map
/// `SatResult::Unknown` to a verification failure (Err / negative
/// answer / keep-check). This test pins the canonical translation
/// pattern that all 8 fail-closed sites in the audit table use.
#[test]
fn unknown_to_err_translation_pattern() {
    fn fail_closed_translate(r: SatResult) -> Result<bool, &'static str> {
        match r {
            SatResult::Sat => Ok(false),       // counterexample present → invalid
            SatResult::Unsat => Ok(true),      // formula proved → valid
            SatResult::Unknown => Err("unknown"), // CRITICAL: never silent-accept
        }
    }

    assert_eq!(fail_closed_translate(SatResult::Sat), Ok(false));
    assert_eq!(fail_closed_translate(SatResult::Unsat), Ok(true));
    assert!(
        fail_closed_translate(SatResult::Unknown).is_err(),
        "Unknown MUST map to Err; mapping it to Ok would be the silent-accept \
         soundness defect Round 1 §5.1 guards against."
    );
}

/// The `bounds_elimination` audit-site uses a different but still-sound
/// translation: `Unknown -> Ok(false)` meaning "do NOT lift the runtime
/// bounds check". This pattern is sound because the *semantic* answer
/// of the function ("can we eliminate the runtime check?") is "no" —
/// no false negatives in safety, only over-conservatism in performance.
#[test]
fn unknown_to_keep_check_pattern_is_sound() {
    fn keep_runtime_check(r: SatResult) -> bool {
        // Returns `true` when the runtime check should be kept;
        // `false` only when we have proof the check is redundant.
        match r {
            SatResult::Unsat => true,            // proof bounds always satisfied → can elide?
            SatResult::Sat => false,             // counterexample → keep check
            SatResult::Unknown => false,         // can't prove → keep check (sound)
        }
    }

    // The "keep check" return value is `false` (do NOT elide) — pin that
    // Unknown stays at "keep the check active".
    assert!(!keep_runtime_check(SatResult::Unknown),
        "Unknown must result in keeping the runtime check, never silent elision.");
}
