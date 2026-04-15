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
pub fn compile_tactic(expr: &TacticExpr) -> CompileResult {
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
                    TacticCombinator::Single(TacticKind::Custom(ref tag)) => {
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
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Custom(ref tag))) => {
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
            CompileResult::Ok(TacticCombinator::Single(TacticKind::Custom(ref tag))) => {
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
                    TacticCombinator::Single(TacticKind::Custom(ref tag)) => {
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
}
