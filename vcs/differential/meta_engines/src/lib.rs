//! META-EXEC-CONVERGENCE-1 — differential harness for Verum's two
//! compile-time meta-execution engines.
//!

//! Verum has **two** engines that evaluate meta functions at compile time,
//! and they must converge on the semantics of the shared language surface:
//!

//! 1. **VBC executor** (engine "vbc") — the real Tier-0 bytecode
//!    interpreter behind
//!    `verum_compiler::meta::vbc_executor::VbcExecutor`. Meta functions are
//!    compiled to VBC by `VbcCodegen` and run on the production
//!    `verum_vbc::interpreter::Interpreter` (NaN-boxed `Value` model,
//!    i64 wrapping integer arithmetic).
//! 2. **Tree-walk evaluator** (engine "tree") — the large AST evaluator in
//!    `verum_compiler::meta::evaluator` (`MetaContext::execute_user_meta_fn`
//!    / `eval_meta_expr`), producing `ConstValue` (= `verum_ast::MetaValue`,
//!    i128 integer arithmetic). It serves `@const`, tagged literals, and
//!    interpolation handlers.
//!

//! The harness runs the *same* meta function with the *same* arguments
//! through both engines — each run under `std::panic::catch_unwind`, so an
//! engine panic (e.g. the tree-walk's raw `i128` arithmetic overflowing
//! under `overflow-checks`) is **observed as a divergence** instead of
//! crashing the harness — then classifies the pair of outcomes.
//!

//! ## Fixture policy
//!

//! * **Agreement surface** — in-range integer/float arithmetic, comparisons,
//!   bool logic, `if`/`else` control flow, `let` bindings, and the Text
//!   overlap subset — must produce equal results (`Verdict::Agree`).
//! * **Known divergences** are *pinned*: each pin asserts the divergence's
//!   current shape (`Outcome` / `Type` / `Value` mismatch). A pin fires when
//!   the divergence **drifts** (shape changed) *or* **disappears** (engines
//!   now converge) — either way a human must look and update the pin.
//!

//! ## Extractor contract (see [`extractor`])
//!

//! VBC results are decoded from the interpreter heap into a small
//! [`extractor::Comparable`] domain: Int / Float / Bool / Text are decoded
//! exactly; **collections and every other heap shape are `Opaque`** — the
//! harness never value-compares what it cannot faithfully decode. Structural
//! collection decoding is the designated step-(ii) donor
//! (`verum_vbc::interpreter::script_engine::extract_owned` already marshals
//! List/Map structurally and can donate that logic when the harness grows
//! collection fixtures).
//!

//! ## Relation to `vcs/differential/runner`
//!

//! A larger differential-testing crate exists at `vcs/differential/runner`
//! (`vcs-differential-runner`, bin `vcs-diff`). It is **dormant and not a
//! workspace member**, so this harness deliberately does *not* link it; the
//! tiny float-epsilon comparator it would provide is vendored here in
//! [`compare`] instead. If that crate is ever revived, the comparator can be
//! reconciled.
//!

//! ## Entry points
//!

//! * `cargo run -p meta_engines --bin meta-engines-diff` — run every fixture,
//!   print per-fixture verdicts and honest totals
//!   (agree / known-diverge / NEW-diverge), exit non-zero on NEW divergences.
//! * `cargo test -p meta_engines` — the same fixtures as tests, plus unit
//!   tests for the extractor and comparator.
//! * `make -C vcs diff-meta-engines` — the Makefile wrapper (clears
//!   `~/.verum/script-cache` first).

pub mod compare;
pub mod engines;
pub mod extractor;
pub mod fixtures;
pub mod report;

pub use compare::{compare_outcomes, EngineOutcome, OutcomeKind, Verdict};
pub use engines::{build_meta_function, run_tree_walk, run_vbc, with_quiet_panics, Arg};
pub use extractor::Comparable;
pub use fixtures::{all_fixtures, overflow_checks_enabled, Expectation, Fixture, PinShape};
pub use report::{classify, run_all, FixtureReport, Status, Totals};
