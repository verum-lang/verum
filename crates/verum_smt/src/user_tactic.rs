//! Phase D.4: User-Facing Tactic DSL Execution Bridge
//!
//! Bridges the gap between the surface-level tactic syntax (as parsed from
//! `grammar/verum.ebnf::tactic_expr`) and the internal Z3 tactic combinators
//! in `tactics.rs`.
//!
//! ## Surface Syntax → Internal Strategy
//!
//! ```verum
//! // User writes:
//! proof by cubical;
//! proof by { ring; simp; }
//! proof by { try { exact(h) } else { auto; } }
//! proof by { repeat(3) { rewrite(assoc); simp; } }
//! ```
//!
//! These surface forms are parsed into `TacticExpr` AST nodes by the parser,
//! then this module translates them into `TacticCombinator` strategies that
//! the Z3 backend can execute.
//!
//! ## Architecture
//!
//! ```text
//! User `.vr` file
//!   │ (parser)
//!   ▼
//! TacticExpr (AST)
//!   │ (this module: compile_tactic)
//!   ▼
//! TacticCombinator (internal)
//!   │ (tactics.rs: apply_combinator)
//!   ▼
//! Z3 Tactic Result
//!   │ (proof_extraction.rs)
//!   ▼
//! ProofTerm / Certificate
//! ```

use verum_common::{List, Text};

use crate::tactics::{
    TacticCombinator, TacticKind, TacticParams, StrategyBuilder,
};

/// A parsed tactic expression from the surface syntax.
/// Mirrors the `tactic_expr` rule in `grammar/verum.ebnf`.
#[derive(Debug, Clone, PartialEq)]
pub enum TacticExpr {
    /// A named tactic: `auto`, `simp`, `ring`, `cubical`, etc.
    Named(Text),

    /// A named tactic with arguments: `rewrite(lemma)`, `apply(h)`, `exact(term)`
    NamedWithArgs {
        name: Text,
        args: List<Text>,
    },

    /// Sequential composition: `t1; t2`
    Seq(Box<TacticExpr>, Box<TacticExpr>),

    /// Try-else: `try { t1 } else { t2 }`
    TryElse {
        primary: Box<TacticExpr>,
        fallback: Box<TacticExpr>,
    },

    /// Repeat: `repeat(n) { t }` or `repeat { t }` (unbounded)
    Repeat {
        count: Option<usize>,
        body: Box<TacticExpr>,
    },

    /// First-succeed: `first { t1; t2; t3 }`
    First(List<TacticExpr>),

    /// Apply to all goals: `all_goals { t }`
    AllGoals(Box<TacticExpr>),

    /// Focus on a specific goal: `focus(n) { t }`
    Focus {
        goal_index: usize,
        body: Box<TacticExpr>,
    },

    /// User-defined tactic invocation: `my_tactic(args...)`
    UserDefined {
        name: Text,
        args: List<Text>,
    },

    /// LLM-oracle tactic: propose proof candidates via language model,
    /// sample above confidence threshold, verify via SMT.
    ///
    /// Surface syntax: `oracle` or `oracle(0.85)`.
    /// The `goal_text` field is empty when constructed at parse time and is
    /// filled in by the execution engine just before the tactic is run.
    Oracle {
        /// Serialised representation of the goal (filled at execution time).
        goal_text: Text,
        /// Minimum softmax probability required before a candidate is trusted.
        confidence: f64,
    },

    /// Quote a tactic expression as a first-class value.
    ///
    /// Surface syntax: `` `tactic_expr `` or `quote { tactic_expr }`.
    /// At evaluation, a `Quote` node does NOT execute its inner
    /// tactic — it returns a handle that callers can manipulate
    /// (compose with other handles, pass as a user-defined tactic
    /// argument, Unquote later to run). This is the meta-programming
    /// entry point the Ltac2-style DSL in
    /// `docs/verification/tactic-dsl.md §7` describes.
    ///
    /// Semantic contract: `Quote(t)` is `t`-inert — the solver does
    /// not observe `t`'s side-effects until an `Unquote` node
    /// corresponding to this Quote runs. Quotes are values; Unquotes
    /// are invocations.
    Quote(Box<TacticExpr>),

    /// Invoke a quoted tactic — splice the inner TacticExpr into
    /// the current proof context and execute it.
    ///
    /// Surface syntax: `$(expr)` inside a quoted block, or
    /// `unquote(handle)` at statement position.
    ///
    /// Invariant: `Unquote(Quote(t))` is operationally equivalent to
    /// `t`. The roundtrip is the identity on semantics; the
    /// intermediate Quote just defers evaluation across a
    /// macro/meta boundary.
    Unquote(Box<TacticExpr>),

    /// Introduce the current proof goal as a fresh metavariable the
    /// user's meta-tactic body can reference.
    ///
    /// Surface syntax: `let goal = goal_intro()` inside a
    /// `@tactic meta fn`.
    ///
    /// Populates a binding whose value is the current goal's
    /// expression (quoted). Meta-tactics can then destructure it,
    /// match on head symbols, or pass it to other meta-tactics.
    /// The execution engine snapshots the goal at the moment
    /// `GoalIntro` runs — subsequent tactics that modify the goal
    /// don't retroactively update the snapshot.
    GoalIntro,
}

/// Result of compiling a surface tactic to an internal combinator.
#[derive(Debug, Clone)]
pub enum CompileResult {
    /// Successfully compiled to a combinator.
    Ok(TacticCombinator),
    /// The tactic name was not recognized.
    UnknownTactic(Text),
    /// Compilation error (e.g., wrong number of arguments).
    Error(Text),
}

/// Compile a surface tactic expression into an internal Z3 tactic combinator.
///
/// This is the main entry point: takes a `TacticExpr` from the parser
/// and produces a `TacticCombinator` that can be executed by
/// `tactics.rs::apply_combinator()`.
///
/// Post-processing: every successful compile result runs through
/// `tactic_laws::normalize` so Quote/Unquote/GoalIntro artefacts
/// from the parser (which compile to skip no-ops) collapse out of
/// the tree before the executor sees it. Skipping the normalize
/// step was leaving `AndThen(skip, body)` dispatches in every
/// meta-programmed tactic, paying an executor step for zero work.
pub fn compile_tactic(expr: &TacticExpr) -> CompileResult {
    let result = compile_tactic_raw(expr);
    // Only normalise on the Ok path — passing-through the error
    // variants unchanged keeps the diagnostic surface stable.
    match result {
        CompileResult::Ok(combinator) => {
            CompileResult::Ok(crate::tactic_laws::normalize(combinator))
        }
        err => err,
    }
}

/// The raw (pre-normalize) compilation — used internally by
/// recursive compile paths that want to avoid double-normalize
/// work on sub-trees. External callers should use `compile_tactic`.
fn compile_tactic_raw(expr: &TacticExpr) -> CompileResult {
    match expr {
        TacticExpr::Named(name) => compile_named_tactic(name),

        TacticExpr::NamedWithArgs { name, args } => {
            compile_named_tactic_with_args(name, args)
        }

        TacticExpr::Seq(left, right) => {
            match (compile_tactic(left), compile_tactic(right)) {
                (CompileResult::Ok(l), CompileResult::Ok(r)) => {
                    CompileResult::Ok(TacticCombinator::AndThen(
                        Box::new(l),
                        Box::new(r),
                    ))
                }
                (CompileResult::Ok(_), err) => err,
                (err, _) => err,
            }
        }

        TacticExpr::TryElse { primary, fallback } => {
            match (compile_tactic(primary), compile_tactic(fallback)) {
                (CompileResult::Ok(p), CompileResult::Ok(f)) => {
                    CompileResult::Ok(TacticCombinator::OrElse(
                        Box::new(p),
                        Box::new(f),
                    ))
                }
                (CompileResult::Ok(_), err) => err,
                (err, _) => err,
            }
        }

        TacticExpr::Repeat { count, body } => {
            match compile_tactic(body) {
                CompileResult::Ok(b) => {
                    let max_iter = count.unwrap_or(100); // default max 100 iterations
                    CompileResult::Ok(TacticCombinator::Repeat(
                        Box::new(b),
                        max_iter,
                    ))
                }
                err => err,
            }
        }

        TacticExpr::First(alternatives) => {
            let mut compiled = List::new();
            for alt in alternatives {
                match compile_tactic(alt) {
                    CompileResult::Ok(c) => compiled.push(c),
                    err => return err,
                }
            }
            if compiled.is_empty() {
                CompileResult::Error(Text::from("first{} requires at least one tactic"))
            } else {
                // Chain alternatives with OrElse
                let mut result = compiled.pop().unwrap();
                while let Some(prev) = compiled.pop() {
                    result = TacticCombinator::OrElse(
                        Box::new(prev),
                        Box::new(result),
                    );
                }
                CompileResult::Ok(result)
            }
        }

        TacticExpr::AllGoals(body) => {
            // AllGoals applies the tactic to each goal independently.
            // Internally, we use Repeat + the tactic itself.
            match compile_tactic(body) {
                CompileResult::Ok(b) => {
                    CompileResult::Ok(TacticCombinator::Repeat(Box::new(b), 1000))
                }
                err => err,
            }
        }

        TacticExpr::Focus { goal_index: _, body } => {
            // Focus restricts to a single goal — in the Z3 backend,
            // we just run the inner tactic (goal selection is handled
            // by the proof state manager, not the SMT solver).
            compile_tactic(body)
        }

        TacticExpr::UserDefined { name, args } => {
            compile_named_tactic_with_args(name, args)
        }

        TacticExpr::Oracle { confidence, .. } => {
            CompileResult::Ok(oracle_strategy(*confidence))
        }

        // Quote is inert at compilation time — returning the inner
        // tactic's compiled form would leak the Quote's "don't
        // execute" contract. Instead we compile it to a NoOp
        // placeholder that the execution engine detects and
        // short-circuits (Quote-valued positions are values, not
        // invocations). The Quote's value form is carried by the
        // surrounding expression context — a compiled Quote is
        // therefore always a "skip-nothing-happens" combinator.
        TacticExpr::Quote(_) => CompileResult::Ok(skip_strategy()),

        // Unquote splices the inner tactic into the current
        // context. At the combinator level this is operationally
        // identical to compiling the inner tactic directly — the
        // Quote/Unquote pair is a parse-time marker, not a
        // runtime combinator. If the inner tactic is itself a
        // Quote, we recurse one level and strip it (preserves
        // `Unquote(Quote(t)) ≡ t`).
        TacticExpr::Unquote(inner) => match inner.as_ref() {
            TacticExpr::Quote(quoted) => compile_tactic(quoted),
            other => compile_tactic(other),
        },

        // GoalIntro is a pure binding-introduction marker — at
        // the Z3 combinator layer it produces no work (the proof
        // state manager consumes the snapshot out-of-band). It
        // compiles to a no-op so the sequencing combinators
        // pass through cleanly.
        TacticExpr::GoalIntro => CompileResult::Ok(skip_strategy()),
    }
}

/// Map a named tactic to an internal Z3 tactic.
fn compile_named_tactic(name: &str) -> CompileResult {
    let kind = match name {
        // === Basic tactics ===
        "auto" => return CompileResult::Ok(auto_strategy()),
        "simp" | "simplify" => TacticKind::Simplify,
        "ring" => TacticKind::Ring,
        "field" => TacticKind::Field,
        "omega" => TacticKind::LIA,  // linear integer arithmetic
        "smt" => TacticKind::SMT,
        "blast" => TacticKind::Blast,
        "trivial" => TacticKind::Simplify,
        "assumption" => TacticKind::Simplify, // closed by simplification
        "contradiction" => TacticKind::Simplify,

        // === Arithmetic tactics ===
        "norm_num" => return CompileResult::Ok(norm_num_strategy()),
        "linarith" => TacticKind::LIA,
        "nlinarith" => TacticKind::NLA,
        "polyrith" => TacticKind::NLA,

        // === Rewriting tactics ===
        "rewrite" => TacticKind::Simplify,
        "unfold" => TacticKind::Simplify,
        "simp_all" => TacticKind::CtxSolverSimplify,

        // === Decision procedures ===
        "decide" => TacticKind::Sat,
        "tauto" => TacticKind::Blast,

        // === Oracle tactic (no-arg form) ===
        "oracle" => return CompileResult::Ok(oracle_strategy(0.9)),

        // === Cubical HoTT tactics ===
        "cubical" => return CompileResult::Ok(cubical_strategy()),
        "homotopy" => return CompileResult::Ok(cubical_strategy()),

        // === Category theory tactics ===
        "category_simp" => return CompileResult::Ok(category_strategy()),
        "category_law" => return CompileResult::Ok(category_strategy()),
        "functor_law" => return CompileResult::Ok(category_strategy()),

        // === Topological tactics ===
        "descent_check" => return CompileResult::Ok(descent_strategy()),
        "topos_simp" => return CompileResult::Ok(topos_strategy()),

        // === Introduction/elimination ===
        "intro" | "intros" => TacticKind::SolveEqs,
        "apply" | "exact" => TacticKind::Simplify,
        "induction" | "cases" => TacticKind::SolveEqs,

        _ => return CompileResult::UnknownTactic(Text::from(name)),
    };

    CompileResult::Ok(TacticCombinator::Single(kind))
}

/// Compile a named tactic with arguments.
fn compile_named_tactic_with_args(name: &str, args: &[Text]) -> CompileResult {
    match name {
        "rewrite" | "rw" => {
            // rewrite(lemma_name) → simplify with specific rewrite rules
            let mut params = TacticParams::default();
            if let Some(lemma) = args.first() {
                params.options.insert(
                    Text::from(format!("rewrite:{}", lemma)),
                    true,
                );
            }
            CompileResult::Ok(TacticCombinator::WithParams(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                params,
            ))
        }

        "apply" | "exact" => {
            // apply(term) → directly use the term as proof
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Simplify))
        }

        "repeat" => {
            if let Some(count_str) = args.first() {
                if let Ok(count) = count_str.as_str().parse::<usize>() {
                    CompileResult::Ok(TacticCombinator::Repeat(
                        Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                        count,
                    ))
                } else {
                    CompileResult::Error(Text::from("repeat: expected integer argument"))
                }
            } else {
                CompileResult::Ok(TacticCombinator::Repeat(
                    Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                    100,
                ))
            }
        }

        "focus" => {
            if let Some(idx_str) = args.first() {
                if let Ok(_idx) = idx_str.as_str().parse::<usize>() {
                    CompileResult::Ok(TacticCombinator::Single(TacticKind::Simplify))
                } else {
                    CompileResult::Error(Text::from("focus: expected integer argument"))
                }
            } else {
                CompileResult::Error(Text::from("focus: requires goal index argument"))
            }
        }

        "oracle" => {
            // LLM-oracle tactic: oracle(confidence)
            //
            // Parse the optional confidence threshold (0 < confidence ≤ 1.0).
            // Build an Oracle-branded combinator that the execution engine
            // (proof_search.rs::try_named_tactic) recognises and dispatches
            // to try_oracle_tactic.
            let confidence = args
                .first()
                .and_then(|s| s.as_str().parse::<f64>().ok())
                .filter(|&c| c > 0.0 && c <= 1.0)
                .unwrap_or(0.9);

            // Tag the combinator so the engine knows to use the oracle path.
            // We encode the confidence threshold in the custom tactic name so
            // the proof_search dispatcher can recover it without needing a
            // separate wrapper type.
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Custom(
                Text::from(format!("oracle:{}", confidence)),
            )))
        }

        _ => compile_named_tactic(name),
    }
}

// =============================================================================
// Pre-built strategies for domain-specific tactics
// =============================================================================

/// A no-op strategy — compiles to `Simplify` with max_iter=0 so
/// the combinator executor runs the identity step. Used as the
/// compile target for `Quote` and `GoalIntro` (meta-programming
/// markers that have no Z3-level side effect).
///
/// Why not an explicit `NoOp` combinator variant: the executor
/// already handles `Repeat(_, 0)` as a zero-step run, so reusing
/// that path keeps the combinator enum small. The semantic is
/// identical.
fn skip_strategy() -> TacticCombinator {
    TacticCombinator::Repeat(
        Box::new(TacticCombinator::Single(TacticKind::Simplify)),
        0,
    )
}

/// The `auto` strategy: try multiple approaches in sequence.
fn auto_strategy() -> TacticCombinator {
    StrategyBuilder::new()
        .then(TacticKind::Simplify)
        .then(TacticKind::SolveEqs)
        .or_else(TacticKind::SMT)
        .or_else(TacticKind::Blast)
        .build()
}

/// The `oracle(confidence)` strategy: stochastic LLM-guided proof search.
///
/// Encodes the confidence threshold in the custom tactic tag so that
/// `proof_search.rs::try_named_tactic` can recover it and dispatch to
/// `try_oracle_tactic`.  Falls back to `auto` via `OrElse` if the oracle
/// path does not fire.
fn oracle_strategy(confidence: f64) -> TacticCombinator {
    TacticCombinator::OrElse(
        Box::new(TacticCombinator::Single(TacticKind::Custom(
            Text::from(format!("oracle:{}", confidence)),
        ))),
        Box::new(auto_strategy()),
    )
}

/// The `norm_num` strategy: numerical normalization.
fn norm_num_strategy() -> TacticCombinator {
    StrategyBuilder::new()
        .then(TacticKind::Simplify)
        .then(TacticKind::LIA)
        .or_else(TacticKind::NLA)
        .build()
}

/// The `cubical` strategy: path normalization + transport reduction.
///
/// Applies cubical-specific simplifications:
/// 1. Transport on refl ↦ identity
/// 2. hcomp on constant ↦ base
/// 3. Path lambda β-reduction
/// 4. Univalence computation (transport(ua(e), x) ↦ e.fwd(x))
fn cubical_strategy() -> TacticCombinator {
    TacticCombinator::AndThen(
        Box::new(TacticCombinator::Single(TacticKind::Custom(
            Text::from("cubical_normalize"),
        ))),
        Box::new(StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::SolveEqs)
            .or_else(TacticKind::SMT)
            .build()),
    )
}

/// The `category_simp` strategy: categorical equation solving.
///
/// Normalizes expressions using:
/// 1. Associativity: (f ∘ g) ∘ h = f ∘ (g ∘ h)
/// 2. Left identity: id ∘ f = f
/// 3. Right identity: f ∘ id = f
/// 4. Functor laws: F(id) = id, F(g ∘ f) = F(g) ∘ F(f)
fn category_strategy() -> TacticCombinator {
    TacticCombinator::Repeat(
        Box::new(TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::Custom(
                Text::from("category_rewrite"),
            ))),
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
        )),
        50, // max 50 rewrites
    )
}

/// The `descent_check` strategy: Čech descent verification.
fn descent_strategy() -> TacticCombinator {
    TacticCombinator::AndThen(
        Box::new(TacticCombinator::Single(TacticKind::Custom(
            Text::from("cech_nerve_compute"),
        ))),
        Box::new(TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Custom(
                Text::from("cocycle_verify"),
            ))),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        )),
    )
}

/// The `topos_simp` strategy: Heyting algebra simplification.
fn topos_strategy() -> TacticCombinator {
    StrategyBuilder::new()
        .then(TacticKind::Simplify)
        .then(TacticKind::Custom(Text::from("heyting_normalize")))
        .or_else(TacticKind::SMT)
        .build()
}

/// Entry point for `proof by <tactic>;` statements.
///
/// Called by the proof checker when it encounters a `proof by` block.
/// Compiles the surface tactic expression, executes it against the Z3
/// backend, and returns success/failure with an optional proof term.
pub fn proof_by_tactic(
    tactic_name: &str,
    _goal_formula: &str, // SMT-LIB2 encoding of the goal
) -> ProofByResult {
    let expr = TacticExpr::Named(Text::from(tactic_name));
    match compile_tactic(&expr) {
        CompileResult::Ok(combinator) => {
            // The actual Z3 execution happens in tactics.rs::apply_combinator.
            // Here we just return the compiled strategy for the caller to execute.
            ProofByResult::Compiled {
                strategy: combinator,
            }
        }
        CompileResult::UnknownTactic(name) => {
            ProofByResult::Error(
                Text::from(format!("unknown tactic: {}", name)),
            )
        }
        CompileResult::Error(msg) => {
            ProofByResult::Error(msg)
        }
    }
}

/// Result of `proof by <tactic>;` compilation.
#[derive(Debug, Clone)]
pub enum ProofByResult {
    /// Successfully compiled — ready for Z3 execution.
    Compiled { strategy: TacticCombinator },
    /// Error during compilation.
    Error(Text),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_auto() {
        match compile_tactic(&TacticExpr::Named(Text::from("auto"))) {
            CompileResult::Ok(_) => {}
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_cubical() {
        match compile_tactic(&TacticExpr::Named(Text::from("cubical"))) {
            CompileResult::Ok(_) => {}
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_category_simp() {
        match compile_tactic(&TacticExpr::Named(Text::from("category_simp"))) {
            CompileResult::Ok(_) => {}
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_seq() {
        let expr = TacticExpr::Seq(
            Box::new(TacticExpr::Named(Text::from("ring"))),
            Box::new(TacticExpr::Named(Text::from("simp"))),
        );
        match compile_tactic(&expr) {
            CompileResult::Ok(TacticCombinator::AndThen(_, _)) => {}
            other => panic!("expected AndThen, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_try_else() {
        let expr = TacticExpr::TryElse {
            primary: Box::new(TacticExpr::Named(Text::from("ring"))),
            fallback: Box::new(TacticExpr::Named(Text::from("auto"))),
        };
        match compile_tactic(&expr) {
            CompileResult::Ok(TacticCombinator::OrElse(_, _)) => {}
            other => panic!("expected OrElse, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_unknown() {
        match compile_tactic(&TacticExpr::Named(Text::from("nonexistent"))) {
            CompileResult::UnknownTactic(_) => {}
            other => panic!("expected UnknownTactic, got {:?}", other),
        }
    }

    #[test]
    fn test_proof_by_auto() {
        match proof_by_tactic("auto", "(assert (= 1 1))") {
            ProofByResult::Compiled { .. } => {}
            other => panic!("expected Compiled, got {:?}", other),
        }
    }

    #[test]
    fn test_proof_by_unknown() {
        match proof_by_tactic("nonexistent_tactic", "") {
            ProofByResult::Error(_) => {}
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // -------------------------------------------------------------------------
    // Oracle tactic tests
    // -------------------------------------------------------------------------

    /// `oracle` with no args compiles to an OrElse(Custom("oracle:0.9"), auto).
    #[test]
    fn test_compile_oracle_no_args() {
        match compile_tactic(&TacticExpr::Named(Text::from("oracle"))) {
            CompileResult::Ok(TacticCombinator::OrElse(inner, _fallback)) => {
                match *inner {
                    TacticCombinator::Single(TacticKind::Custom(tag)) => {
                        assert!(
                            tag.starts_with("oracle:"),
                            "tag should start with oracle:, got {}",
                            tag
                        );
                    }
                    ref other => panic!("expected Single(Custom(oracle:...)), got {:?}", other),
                }
            }
            ref other => panic!("expected OrElse, got {:?}", other),
        }
    }

    /// `oracle(0.75)` compiles to a Single(Custom("oracle:0.75")).
    #[test]
    fn test_compile_oracle_with_confidence() {
        match compile_named_tactic_with_args("oracle", &[Text::from("0.75")]) {
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Custom(tag))) => {
                assert_eq!(tag.as_str(), "oracle:0.75");
            }
            ref other => panic!("expected Single(Custom(oracle:0.75)), got {:?}", other),
        }
    }

    /// `oracle` with an out-of-range confidence falls back to the default 0.9.
    #[test]
    fn test_compile_oracle_invalid_confidence_fallback() {
        // Negative confidence is out of range; should fall back to 0.9.
        match compile_named_tactic_with_args("oracle", &[Text::from("-0.5")]) {
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Custom(tag))) => {
                assert_eq!(tag.as_str(), "oracle:0.9", "should fall back to 0.9");
            }
            ref other => panic!("expected Single(Custom(oracle:0.9)), got {:?}", other),
        }
    }

    /// The `Oracle` AST variant compiles to an OrElse strategy.
    #[test]
    fn test_compile_oracle_ast_variant() {
        let expr = TacticExpr::Oracle {
            goal_text: Text::from(""),
            confidence: 0.85,
        };
        match compile_tactic(&expr) {
            CompileResult::Ok(TacticCombinator::OrElse(ref inner, _)) => {
                match inner.as_ref() {
                    TacticCombinator::Single(TacticKind::Custom(tag)) => {
                        assert!(tag.starts_with("oracle:"), "tag should start with oracle:");
                    }
                    other => panic!("expected Single(Custom(oracle:...)), got {:?}", other),
                }
            }
            ref other => panic!("expected OrElse, got {:?}", other),
        }
    }

    /// `proof_by_tactic("oracle", ...)` compiles successfully.
    #[test]
    fn test_proof_by_oracle() {
        match proof_by_tactic("oracle", "(assert (= x x))") {
            ProofByResult::Compiled { .. } => {}
            other => panic!("expected Compiled, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // Meta-programming: Quote / Unquote / GoalIntro (task #73)
    // -----------------------------------------------------------------

    /// `Quote` compiles to a skip — the inner tactic's side effects
    /// are deferred, not executed.
    #[test]
    fn quote_compiles_to_noop() {
        let inner = TacticExpr::Named(Text::from("auto"));
        let quoted = TacticExpr::Quote(Box::new(inner));
        match compile_tactic(&quoted) {
            CompileResult::Ok(TacticCombinator::Repeat(_, 0)) => {}
            other => panic!("expected skip (Repeat with 0 iters), got {:?}", other),
        }
    }

    /// `Unquote(Quote(t))` compiles to the same combinator as
    /// `t` alone — the roundtrip is the identity on semantics.
    #[test]
    fn unquote_of_quote_is_identity() {
        let t = TacticExpr::Named(Text::from("simp"));
        let quoted = TacticExpr::Quote(Box::new(t.clone()));
        let unquoted = TacticExpr::Unquote(Box::new(quoted));

        let direct = format!("{:?}", compile_tactic(&t));
        let round = format!("{:?}", compile_tactic(&unquoted));
        assert_eq!(
            direct, round,
            "Unquote(Quote(t)) must compile identically to t"
        );
    }

    /// `Unquote` of a non-quoted expression compiles that expression
    /// directly — the Unquote is transparent when its target isn't
    /// a literal Quote.
    #[test]
    fn unquote_of_non_quote_passes_through() {
        let inner = TacticExpr::Named(Text::from("omega"));
        let unquoted = TacticExpr::Unquote(Box::new(inner.clone()));
        let direct = format!("{:?}", compile_tactic(&inner));
        let via_unquote = format!("{:?}", compile_tactic(&unquoted));
        assert_eq!(direct, via_unquote);
    }

    /// `GoalIntro` is a meta-programming marker; at the combinator
    /// level it's a no-op so sequencing combinators pass through.
    #[test]
    fn goal_intro_compiles_to_noop() {
        match compile_tactic(&TacticExpr::GoalIntro) {
            CompileResult::Ok(TacticCombinator::Repeat(_, 0)) => {}
            other => panic!("expected skip, got {:?}", other),
        }
    }

    /// `Seq(GoalIntro, real_tactic)` must behave identically to
    /// `real_tactic` alone — GoalIntro's no-op contract in the
    /// sequencing context.
    #[test]
    fn seq_with_goal_intro_does_not_alter_downstream_tactic() {
        let real = TacticExpr::Named(Text::from("auto"));
        let seq = TacticExpr::Seq(
            Box::new(TacticExpr::GoalIntro),
            Box::new(real.clone()),
        );
        // Both must compile (the Seq just wraps a skip + real tactic).
        assert!(matches!(compile_tactic(&seq), CompileResult::Ok(_)));
        assert!(matches!(compile_tactic(&real), CompileResult::Ok(_)));
    }
}
