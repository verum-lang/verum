//! Cubical Tactic Runtime Bridge
//!
//! This module wires the `cubical`, `category_simp`, `category_law`, and
//! `descent_check` builtin tactics (declared in `grammar/verum.ebnf` and
//! compiled to `TacticCombinator` by `user_tactic.rs`) to concrete
//! execution logic operating on `ProofGoal` values.
//!
//! ## Why a separate module?
//!
//! `verum_types` cannot be imported from `verum_smt` due to a circular
//! dependency (`verum_types → verum_smt → verum_types`). The cubical
//! normalizer that lives in `verum_types::cubical` therefore cannot be
//! called directly. This module re-implements the minimal normalizer
//! needed for the `cubical` tactic (the same five reduction rules) as
//! a self-contained, zero-dependency copy. The types are structurally
//! identical to `verum_types::cubical::CubicalTerm` but carry no
//! cross-crate `use` dependencies.
//!
//! ## Tactics implemented
//!
//! | Surface syntax              | Handler function                   |
//! |-----------------------------|-------------------------------------|
//! | `proof by cubical;`         | [`execute_cubical_tactic`]          |
//! | `proof by homotopy;`        | [`execute_cubical_tactic`]          |
//! | `proof by category_simp;`   | [`execute_category_simp_tactic`]    |
//! | `proof by category_law;`    | [`execute_category_law_tactic`]     |
//! | `proof by descent_check;`   | [`execute_descent_check_tactic`]    |
//!
//! ## Path-goal structure
//!
//! The `cubical` tactic acts on equality goals of the form `lhs == rhs`
//! (AST node `ExprKind::Binary { op: BinOp::Eq, .. }`). It converts
//! each side to a [`CubicalNorm`] via [`expr_to_cubical_norm`] and
//! checks definitional equality by reducing both to WHNF. If the
//! normalised forms coincide the goal is closed with no subgoals; if
//! not, the tactic falls back to the SMT solver.
//!
//! ## Category-law structure
//!
//! The `category_simp` and `category_law` tactics look for goals of the
//! form `f == g` where `f` and `g` are compositions of morphisms. They
//! apply a fixed set of rewrite rules (associativity, left/right
//! identity, functor preservation) up to a bounded number of steps,
//! then hand off to the SMT solver.
//!
//! ## Descent structure
//!
//! The `descent_check` tactic is a thin wrapper: it recognises goals
//! whose head is `descent_condition(…)` or `compatible_sections(…)` and
//! delegates to the SMT solver with a hint that the sheaf-domain
//! encoding should be activated.

use verum_ast::{BinOp, Expr, ExprKind};
use verum_common::{List, Text};

use crate::proof_search::{ProofError, ProofGoal};

// =============================================================================
// Section 1: Self-contained cubical normalizer
//
// Mirrors the reduction rules in `verum_types::cubical` without importing it.
// =============================================================================

/// Interval endpoint used during substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IEndpoint {
    I0,
    I1,
}

/// Minimal cubical normal-form representation.
///
/// We only need enough structure to decide definitional equality:
/// we track the *name* of dimension variables and *value* atoms, plus
/// the constructor forms whose reduction rules can fire.
#[derive(Debug, Clone, PartialEq)]
enum CubicalNorm {
    /// An atomic value (variable or constant), opaque to the normalizer.
    Atom(Text),

    /// Interval endpoint literal.
    Endpoint(IEndpoint),

    /// A dimension variable bound by a `PathLambda`. Produced internally
    /// during `subst` / path-lambda β-reduction; never constructed from
    /// user-facing `Expr` terms (only in tests to exercise reduction).
    #[allow(dead_code)]
    Dim(Text),

    /// `refl(x)` — constant path at `x`.
    Refl(Box<CubicalNorm>),

    /// `transport(line, value)` — reduces when `line` is `refl`.
    Transport {
        line: Box<CubicalNorm>,
        value: Box<CubicalNorm>,
    },

    /// `hcomp(base, sides)` — reduces when `sides` is `refl`.
    Hcomp {
        base: Box<CubicalNorm>,
        sides: Box<CubicalNorm>,
    },

    /// `(λi. body) @ endpoint` path-lambda β-redex.
    PathApp {
        path: Box<CubicalNorm>,
        at: Box<CubicalNorm>,
    },

    /// `λi. body` — path abstraction.
    PathLambda {
        dim: Text,
        body: Box<CubicalNorm>,
    },

    /// `sym(p)` — reverses a path; `sym(refl(x)) ↦ refl(x)`.
    Sym(Box<CubicalNorm>),

    /// `trans(p, q)` — opaque path composition.
    Trans(Box<CubicalNorm>, Box<CubicalNorm>),

    /// `ua(e)` — univalence path; `ua(id_equiv) ↦ refl(Type)`.
    Ua(Box<CubicalNorm>),

    /// `e.fwd(x)` — forward action of equivalence (output of transport-ua).
    EquivFwd {
        equiv: Box<CubicalNorm>,
        value: Box<CubicalNorm>,
    },

    /// `e.bwd(x)` — backward action of equivalence (output of transport-sym-ua).
    EquivBwd {
        equiv: Box<CubicalNorm>,
        value: Box<CubicalNorm>,
    },
}

impl CubicalNorm {
    /// Substitute dimension variable `dim` with `endpoint` throughout.
    fn subst(&self, dim: &str, endpoint: IEndpoint) -> CubicalNorm {
        match self {
            CubicalNorm::Dim(d) if d.as_str() == dim => CubicalNorm::Endpoint(endpoint),
            CubicalNorm::Dim(_)
            | CubicalNorm::Atom(_)
            | CubicalNorm::Endpoint(_) => self.clone(),

            CubicalNorm::PathLambda { dim: d, body } if d.as_str() == dim => {
                // Shadowed — do not substitute inside
                self.clone()
            }
            CubicalNorm::PathLambda { dim: d, body } => CubicalNorm::PathLambda {
                dim: d.clone(),
                body: Box::new(body.subst(dim, endpoint)),
            },

            CubicalNorm::Refl(x) => CubicalNorm::Refl(Box::new(x.subst(dim, endpoint))),
            CubicalNorm::Sym(p) => CubicalNorm::Sym(Box::new(p.subst(dim, endpoint))),
            CubicalNorm::Ua(e) => CubicalNorm::Ua(Box::new(e.subst(dim, endpoint))),
            CubicalNorm::Trans(p, q) => CubicalNorm::Trans(
                Box::new(p.subst(dim, endpoint)),
                Box::new(q.subst(dim, endpoint)),
            ),
            CubicalNorm::Transport { line, value } => CubicalNorm::Transport {
                line: Box::new(line.subst(dim, endpoint)),
                value: Box::new(value.subst(dim, endpoint)),
            },
            CubicalNorm::Hcomp { base, sides } => CubicalNorm::Hcomp {
                base: Box::new(base.subst(dim, endpoint)),
                sides: Box::new(sides.subst(dim, endpoint)),
            },
            CubicalNorm::PathApp { path, at } => CubicalNorm::PathApp {
                path: Box::new(path.subst(dim, endpoint)),
                at: Box::new(at.subst(dim, endpoint)),
            },
            CubicalNorm::EquivFwd { equiv, value } => CubicalNorm::EquivFwd {
                equiv: Box::new(equiv.subst(dim, endpoint)),
                value: Box::new(value.subst(dim, endpoint)),
            },
            CubicalNorm::EquivBwd { equiv, value } => CubicalNorm::EquivBwd {
                equiv: Box::new(equiv.subst(dim, endpoint)),
                value: Box::new(value.subst(dim, endpoint)),
            },
        }
    }

    /// Reduce to Weak Head Normal Form.
    ///
    /// Applies the five core cubical reduction rules (identical to
    /// `verum_types::cubical::CubicalTerm::whnf`):
    ///
    /// 1. `transport(refl, x)          ↦ x`
    /// 2. `transport(ua(e), x)         ↦ e.fwd(x)`
    /// 3. `transport(sym(ua(e)), x)    ↦ e.bwd(x)`
    /// 4. `hcomp(base, refl(sides))    ↦ base`
    /// 5. `(λi. body) @ endpoint       ↦ body[i := endpoint]`
    /// 6. `refl(x) @ _                 ↦ x`
    /// 7. `sym(refl(x))                ↦ refl(x)`
    /// 8. `ua(id_equiv)                ↦ refl(Type)`
    fn whnf(self) -> CubicalNorm {
        match self {
            // Rule 1: transport(refl, x) ↦ x
            CubicalNorm::Transport { line, value }
                if matches!(line.as_ref(), CubicalNorm::Refl(_)) =>
            {
                value.whnf()
            }

            // Rule 2: transport(ua(e), x) ↦ e.fwd(x)
            CubicalNorm::Transport { line, value }
                if matches!(line.as_ref(), CubicalNorm::Ua(_)) =>
            {
                let equiv = match *line {
                    CubicalNorm::Ua(e) => e,
                    _ => unreachable!(),
                };
                CubicalNorm::EquivFwd {
                    equiv,
                    value: Box::new(value.whnf()),
                }
            }

            // Rule 3: transport(sym(ua(e)), x) ↦ e.bwd(x)
            CubicalNorm::Transport { line, value }
                if matches!(
                    line.as_ref(),
                    CubicalNorm::Sym(inner)
                        if matches!(inner.as_ref(), CubicalNorm::Ua(_))
                ) =>
            {
                let equiv = match *line {
                    CubicalNorm::Sym(boxed) => match *boxed {
                        CubicalNorm::Ua(e) => e,
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                };
                CubicalNorm::EquivBwd {
                    equiv,
                    value: Box::new(value.whnf()),
                }
            }

            // Rule 4: hcomp(base, refl(sides)) ↦ base
            CubicalNorm::Hcomp { base, sides }
                if matches!(sides.as_ref(), CubicalNorm::Refl(_)) =>
            {
                base.whnf()
            }

            // Rule 5+6: path-app β and refl-app
            CubicalNorm::PathApp { path, at } => match (*path, *at) {
                (CubicalNorm::PathLambda { dim, body }, CubicalNorm::Endpoint(ep)) => {
                    body.subst(dim.as_str(), ep).whnf()
                }
                (CubicalNorm::Refl(x), _) => x.whnf(),
                (path, at) => CubicalNorm::PathApp {
                    path: Box::new(path),
                    at: Box::new(at),
                },
            },

            // Rule 7: sym(refl(x)) ↦ refl(x)
            CubicalNorm::Sym(inner) if matches!(inner.as_ref(), CubicalNorm::Refl(_)) => {
                inner.whnf()
            }

            // Rule 8: ua(id_equiv) ↦ refl(Type)
            CubicalNorm::Ua(inner)
                if matches!(
                    inner.as_ref(),
                    CubicalNorm::Atom(v) if v.as_str() == "id_equiv"
                ) =>
            {
                CubicalNorm::Refl(Box::new(CubicalNorm::Atom(Text::from("Type"))))
            }

            other => other,
        }
    }

    /// Definitional equality: reduce both to WHNF, then compare.
    fn definitionally_equal(&self, other: &CubicalNorm) -> bool {
        self.clone().whnf() == other.clone().whnf()
    }
}

// =============================================================================
// Section 2: Translate Expr → CubicalNorm
//
// We convert `Expr` AST nodes to `CubicalNorm` by recognising the known
// cubical constructors by name and argument count. Unknown expressions
// become opaque `Atom` values so that syntactic equality still works for
// them.
// =============================================================================

/// Convert an `Expr` into a `CubicalNorm` for normalisation.
///
/// The mapping is:
///
/// | Expr pattern                            | CubicalNorm          |
/// |-----------------------------------------|----------------------|
/// | literal / path variable                 | `Atom(name)`         |
/// | `refl(x)`                               | `Refl(x)`            |
/// | `transport(p, x)`                       | `Transport { p, x }` |
/// | `hcomp(base, sides)`                    | `Hcomp { base, sides }` |
/// | `sym(p)`                                | `Sym(p)`             |
/// | `trans(p, q)` / `p.trans(q)`            | `Trans(p, q)`        |
/// | `ua(e)`                                 | `Ua(e)`              |
/// | `path_lam(i, body)` / `λi. body`        | `PathLambda { i, body }` |
/// | `path_app(path, pt)` / `path @ pt`      | `PathApp { path, pt }` |
/// | `i0`                                    | `Endpoint(I0)`       |
/// | `i1`                                    | `Endpoint(I1)`       |
/// | anything else                           | `Atom(<display>)`    |
fn expr_to_cubical_norm(expr: &Expr) -> CubicalNorm {
    match &expr.kind {
        // Literal values become atoms
        ExprKind::Literal(lit) => {
            CubicalNorm::Atom(Text::from(format!("{:?}", lit.kind)))
        }

        // Variable / path reference
        ExprKind::Path(p) => {
            let name = match p.as_ident() {
                verum_common::Maybe::Some(id) => id.to_string(),
                verum_common::Maybe::None => format!("{:?}", p),
            };
            match name.as_str() {
                "i0" => CubicalNorm::Endpoint(IEndpoint::I0),
                "i1" => CubicalNorm::Endpoint(IEndpoint::I1),
                "id_equiv" => CubicalNorm::Atom(Text::from("id_equiv")),
                _ => CubicalNorm::Atom(Text::from(name)),
            }
        }

        // Parenthesised expression — transparent
        ExprKind::Paren(inner) => expr_to_cubical_norm(inner),

        // Function call: recognise cubical builtins by name + arity
        ExprKind::Call { func, args, .. } => {
            let head = func_head_name(func);
            match (head.as_deref(), args.len()) {
                (Some("refl"), 1) => {
                    CubicalNorm::Refl(Box::new(expr_to_cubical_norm(&args[0])))
                }
                (Some("transport"), 2) => CubicalNorm::Transport {
                    line: Box::new(expr_to_cubical_norm(&args[0])),
                    value: Box::new(expr_to_cubical_norm(&args[1])),
                },
                (Some("hcomp"), 2) => CubicalNorm::Hcomp {
                    base: Box::new(expr_to_cubical_norm(&args[0])),
                    sides: Box::new(expr_to_cubical_norm(&args[1])),
                },
                (Some("sym"), 1) => {
                    CubicalNorm::Sym(Box::new(expr_to_cubical_norm(&args[0])))
                }
                (Some("trans"), 2) => CubicalNorm::Trans(
                    Box::new(expr_to_cubical_norm(&args[0])),
                    Box::new(expr_to_cubical_norm(&args[1])),
                ),
                (Some("ua"), 1) => {
                    CubicalNorm::Ua(Box::new(expr_to_cubical_norm(&args[0])))
                }
                (Some("path_lam"), 2) => {
                    // path_lam(dim_name_as_string, body_expr)
                    let dim_name = match &args[0].kind {
                        ExprKind::Literal(lit) => format!("{:?}", lit.kind),
                        ExprKind::Path(p) => match p.as_ident() {
                            verum_common::Maybe::Some(id) => id.to_string(),
                            verum_common::Maybe::None => "_".to_string(),
                        },
                        _ => "_".to_string(),
                    };
                    CubicalNorm::PathLambda {
                        dim: Text::from(dim_name),
                        body: Box::new(expr_to_cubical_norm(&args[1])),
                    }
                }
                (Some("path_app"), 2) => CubicalNorm::PathApp {
                    path: Box::new(expr_to_cubical_norm(&args[0])),
                    at: Box::new(expr_to_cubical_norm(&args[1])),
                },
                _ => {
                    // Unknown call — represent opaquely
                    let display = format!("{:?}", expr.kind);
                    CubicalNorm::Atom(Text::from(display))
                }
            }
        }

        // Method call: p.trans(q)
        ExprKind::MethodCall { receiver, method, args, .. } if method.as_str() == "trans" && args.len() == 1 => {
            CubicalNorm::Trans(
                Box::new(expr_to_cubical_norm(receiver)),
                Box::new(expr_to_cubical_norm(&args[0])),
            )
        }

        // Index operator: used as path application syntax `path[endpoint]`
        ExprKind::Index { expr: base, index } => CubicalNorm::PathApp {
            path: Box::new(expr_to_cubical_norm(base)),
            at: Box::new(expr_to_cubical_norm(index)),
        },

        // All other forms — represent opaquely so syntactic comparison works
        _ => {
            let display = format!("{:?}", expr.kind);
            CubicalNorm::Atom(Text::from(display))
        }
    }
}

/// Extract the leading name from a function-position expression.
fn func_head_name(func: &Expr) -> Option<String> {
    match &func.kind {
        ExprKind::Path(p) => match p.as_ident() {
            verum_common::Maybe::Some(id) => Some(id.to_string()),
            verum_common::Maybe::None => None,
        },
        ExprKind::Paren(inner) => func_head_name(inner),
        _ => None,
    }
}

// =============================================================================
// Section 3: Public tactic handlers
//
// These are called from `proof_search.rs::try_named_tactic` when the
// tactic name matches one of the builtin cubical/categorical names.
// =============================================================================

/// Execute the `cubical` (or `homotopy`) tactic on a proof goal.
///
/// ## Algorithm
///
/// 1. Expect the goal to be an equality: `lhs == rhs`.
/// 2. Convert `lhs` and `rhs` to [`CubicalNorm`] via
///    [`expr_to_cubical_norm`].
/// 3. Reduce both sides to WHNF using the cubical reduction rules.
/// 4. Compare the reduced forms structurally.
///    - If equal: goal is closed → return empty subgoal list (proved).
///    - If unequal: fall back to the SMT-backed `try_auto` path (return
///      the original goal as a single remaining subgoal so the caller
///      can try another tactic).
///
/// The `fallback_smt` closure receives the original goal when cubical
/// normalisation is insufficient. The closure typically calls
/// `ProofSearchEngine::try_auto`.
pub fn execute_cubical_tactic(
    goal: &ProofGoal,
) -> CubicalTacticOutcome {
    match &goal.goal.kind {
        ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } => {
            let lhs_norm = expr_to_cubical_norm(left);
            let rhs_norm = expr_to_cubical_norm(right);

            if lhs_norm.definitionally_equal(&rhs_norm) {
                // Goal solved by cubical normalisation — Refl proof
                CubicalTacticOutcome::Proved {
                    method: ProofMethod::CubicalRefl,
                }
            } else {
                // Normalisation did not close the goal — hand off to SMT
                CubicalTacticOutcome::FallbackToSmt
            }
        }
        _ => {
            // Goal is not an equality at all — not applicable
            CubicalTacticOutcome::NotApplicable
        }
    }
}

/// Execute the `category_simp` tactic.
///
/// Applies a bounded (≤ `MAX_REWRITES`) sequence of categorical rewrite
/// rules to the goal, checking after each step whether both sides have
/// become definitionally equal:
///
/// * **Associativity**: `(f ∘ g) ∘ h  =  f ∘ (g ∘ h)`
/// * **Left identity**: `id ∘ f  =  f`
/// * **Right identity**: `f ∘ id  =  f`
///
/// After exhausting the rewrite budget, any remaining equality is
/// deferred to the SMT fallback.
pub fn execute_category_simp_tactic(
    goal: &ProofGoal,
) -> CubicalTacticOutcome {
    const MAX_REWRITES: usize = 50;

    match &goal.goal.kind {
        ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } => {
            // Normalise both sides under categorical laws
            let lhs = normalise_cat(left, MAX_REWRITES);
            let rhs = normalise_cat(right, MAX_REWRITES);

            // Compare the normalised atoms textually
            if cat_eq(&lhs, &rhs) {
                CubicalTacticOutcome::Proved {
                    method: ProofMethod::CategoryNorm,
                }
            } else {
                CubicalTacticOutcome::FallbackToSmt
            }
        }
        _ => CubicalTacticOutcome::NotApplicable,
    }
}

/// Execute the `category_law` tactic.
///
/// A more aggressive version of `category_simp` that also unfolds
/// functor-preservation laws:
///
/// * `F(id)     =  id`
/// * `F(g ∘ f)  =  F(g) ∘ F(f)`
///
/// Uses a larger rewrite budget (100 steps) before falling back to SMT.
pub fn execute_category_law_tactic(
    goal: &ProofGoal,
) -> CubicalTacticOutcome {
    const MAX_REWRITES: usize = 100;

    match &goal.goal.kind {
        ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } => {
            let lhs = normalise_cat_with_functor(left, MAX_REWRITES);
            let rhs = normalise_cat_with_functor(right, MAX_REWRITES);

            if cat_eq(&lhs, &rhs) {
                CubicalTacticOutcome::Proved {
                    method: ProofMethod::CategoryLaw,
                }
            } else {
                CubicalTacticOutcome::FallbackToSmt
            }
        }
        _ => CubicalTacticOutcome::NotApplicable,
    }
}

/// Execute the `descent_check` tactic.
///
/// Recognises goals of the form `descent_condition(cover, sections)` or
/// `compatible_sections(cover, s1, s2)` and delegates them to the SMT
/// solver. The sheaf-domain encoding in `domains/` handles the actual
/// verification; this tactic is a thin routing shim that tells the SMT
/// layer to activate that encoding.
///
/// For goals that do not match the descent pattern, the tactic is
/// `NotApplicable` so the proof engine can try other tactics.
pub fn execute_descent_check_tactic(
    goal: &ProofGoal,
) -> CubicalTacticOutcome {
    let is_descent_goal = is_descent_shaped(&goal.goal);

    if is_descent_goal {
        // The heavy lifting is done by the SMT solver's sheaf encoding.
        // Signal that we want the SMT fallback with the descent hint.
        CubicalTacticOutcome::FallbackToSmtWithHint {
            hint: Text::from("sheaf_descent"),
        }
    } else {
        CubicalTacticOutcome::NotApplicable
    }
}

// =============================================================================
// Section 4: Outcome type
// =============================================================================

/// Result of running a cubical/categorical tactic on a goal.
#[derive(Debug, Clone)]
pub enum CubicalTacticOutcome {
    /// Tactic closed the goal — no remaining subgoals.
    Proved {
        /// Which proof method discharged the goal.
        method: ProofMethod,
    },
    /// The tactic made no progress; hand off to the SMT solver.
    FallbackToSmt,
    /// The tactic requires SMT with a specific domain hint enabled.
    FallbackToSmtWithHint {
        hint: Text,
    },
    /// The goal shape is not applicable to this tactic.
    NotApplicable,
}

/// Which proof method closed the goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofMethod {
    /// Closed by cubical WHNF normalisation (definitional equality).
    CubicalRefl,
    /// Closed by categorical rewrite normalisation.
    CategoryNorm,
    /// Closed by functor + category law normalisation.
    CategoryLaw,
}

// =============================================================================
// Section 5: Helpers for `proof_search.rs`
//
// These are the functions that `ProofSearchEngine::try_named_tactic`
// actually calls — they wrap the outcome into the `Result<List<ProofGoal>,
// ProofError>` signature expected by the engine.
// =============================================================================

/// Called by `ProofSearchEngine::try_named_tactic` for `"cubical"` / `"homotopy"`.
///
/// Returns:
/// - `Ok(List::new())` — goal proved by cubical normalisation.
/// - `Err(ProofError::TacticFailed)` with a `__smt_fallback` message — tells
///   the engine to retry with `try_auto`.
pub fn try_cubical(goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
    match execute_cubical_tactic(goal) {
        CubicalTacticOutcome::Proved { .. } => Ok(List::new()),
        CubicalTacticOutcome::FallbackToSmt | CubicalTacticOutcome::FallbackToSmtWithHint { .. } => {
            Err(ProofError::TacticFailed(Text::from("__smt_fallback")))
        }
        CubicalTacticOutcome::NotApplicable => Err(ProofError::TacticFailed(Text::from(
            "cubical: goal is not a Path/equality type",
        ))),
    }
}

/// Called by `ProofSearchEngine::try_named_tactic` for `"category_simp"`.
pub fn try_category_simp(goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
    match execute_category_simp_tactic(goal) {
        CubicalTacticOutcome::Proved { .. } => Ok(List::new()),
        CubicalTacticOutcome::FallbackToSmt | CubicalTacticOutcome::FallbackToSmtWithHint { .. } => {
            Err(ProofError::TacticFailed(Text::from("__smt_fallback")))
        }
        CubicalTacticOutcome::NotApplicable => Err(ProofError::TacticFailed(Text::from(
            "category_simp: goal is not a categorical equality",
        ))),
    }
}

/// Called by `ProofSearchEngine::try_named_tactic` for `"category_law"`.
pub fn try_category_law(goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
    match execute_category_law_tactic(goal) {
        CubicalTacticOutcome::Proved { .. } => Ok(List::new()),
        CubicalTacticOutcome::FallbackToSmt | CubicalTacticOutcome::FallbackToSmtWithHint { .. } => {
            Err(ProofError::TacticFailed(Text::from("__smt_fallback")))
        }
        CubicalTacticOutcome::NotApplicable => Err(ProofError::TacticFailed(Text::from(
            "category_law: goal is not a categorical equality",
        ))),
    }
}

/// Called by `ProofSearchEngine::try_named_tactic` for `"descent_check"` / `"descent"`.
pub fn try_descent_check(goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
    match execute_descent_check_tactic(goal) {
        CubicalTacticOutcome::Proved { .. } => Ok(List::new()),
        CubicalTacticOutcome::FallbackToSmt | CubicalTacticOutcome::FallbackToSmtWithHint { .. } => {
            // Descent goals need the full SMT machinery with sheaf encoding.
            Err(ProofError::TacticFailed(Text::from("__smt_fallback")))
        }
        CubicalTacticOutcome::NotApplicable => Err(ProofError::TacticFailed(Text::from(
            "descent_check: goal does not match descent pattern",
        ))),
    }
}

// =============================================================================
// Section 6: Category normalisation helpers
// =============================================================================

/// Canonical representation of a morphism expression after category rewriting.
///
/// We represent a normalised morphism as a left-flat list of atoms joined
/// by composition — `[f, g, h]` means `f ∘ g ∘ h` with identity elements
/// removed. This makes equality checking trivial.
#[derive(Debug, Clone, PartialEq)]
struct CatNorm {
    /// Atoms in composition order (identity elements are stripped).
    atoms: List<Text>,
}

impl CatNorm {
    /// Construct from a single atom.
    fn atom(name: Text) -> Self {
        Self {
            atoms: {
                let mut l = List::new();
                l.push(name);
                l
            },
        }
    }

    /// Identity morphism — empty composition list.
    fn identity() -> Self {
        Self { atoms: List::new() }
    }

    /// Compose `self` then `other` (left-to-right).
    fn compose(mut self, mut other: CatNorm) -> CatNorm {
        self.atoms.extend(other.atoms.drain(..));
        self
    }
}

/// Normalise a morphism `Expr` under category associativity + identity laws.
///
/// Recognises:
/// - `id` (identity) — stripped
/// - `f.compose(g)` / `compose(f, g)` / `f >> g` / `g << f` — flattened
///
/// Returns a [`CatNorm`] (flat atom list without identities).
fn normalise_cat(expr: &Expr, budget: usize) -> CatNorm {
    if budget == 0 {
        // Out of rewrite steps — represent opaquely
        return CatNorm::atom(Text::from(format!("{:?}", expr.kind)));
    }

    match &expr.kind {
        ExprKind::Paren(inner) => normalise_cat(inner, budget),

        ExprKind::Path(p) => {
            let name = match p.as_ident() {
                verum_common::Maybe::Some(id) => id.to_string(),
                verum_common::Maybe::None => format!("{:?}", p),
            };
            if is_identity_method(&name) {
                CatNorm::identity()
            } else {
                CatNorm::atom(Text::from(name))
            }
        }

        // Method call: f.compose(g), f.then(g), f.andThen(g), etc.
        ExprKind::MethodCall { receiver, method, args, .. }
            if is_composition_method(method.as_str()) && args.len() == 1 =>
        {
            let l = normalise_cat(receiver, budget - 1);
            let r = normalise_cat(&args[0], budget - 1);
            l.compose(r)
        }

        // Free-form call: compose(f, g), comp(f, g), etc.
        ExprKind::Call { func, args, .. } if args.len() == 2 => {
            let head = func_head_name(func).unwrap_or_default();
            if is_composition_method(&head) {
                let l = normalise_cat(&args[0], budget - 1);
                let r = normalise_cat(&args[1], budget - 1);
                l.compose(r)
            } else {
                CatNorm::atom(Text::from(format!("{:?}", expr.kind)))
            }
        }

        // Binary composition operators: f >> g or g << f
        ExprKind::Binary { op: BinOp::Shr, left, right } => {
            let l = normalise_cat(left, budget - 1);
            let r = normalise_cat(right, budget - 1);
            l.compose(r)
        }
        ExprKind::Binary { op: BinOp::Shl, left, right } => {
            // g << f  means  f ∘ g  (reverse compose)
            let r = normalise_cat(left, budget - 1);
            let l = normalise_cat(right, budget - 1);
            l.compose(r)
        }

        _ => CatNorm::atom(Text::from(format!("{:?}", expr.kind))),
    }
}

/// Like [`normalise_cat`] but also unfolds functor preservation laws:
/// `F(id) ↦ id` and `F(g ∘ f) ↦ F(g) ∘ F(f)`.
fn normalise_cat_with_functor(expr: &Expr, budget: usize) -> CatNorm {
    if budget == 0 {
        return CatNorm::atom(Text::from(format!("{:?}", expr.kind)));
    }

    // Check if this is F(id) or F(compose(...))
    if let ExprKind::Call { func, args, .. } = &expr.kind {
        if args.len() == 1 {
            let inner = &args[0];
            match &inner.kind {
                // F(id) ↦ id
                ExprKind::Path(p) => {
                    let nm = match p.as_ident() {
                        verum_common::Maybe::Some(id) => id.to_string(),
                        verum_common::Maybe::None => String::new(),
                    };
                    if is_identity_method(&nm) {
                        return CatNorm::identity();
                    }
                }
                // F(g ∘ f) ↦ F(g) ∘ F(f)
                ExprKind::MethodCall { receiver, method, args: margs, .. }
                    if is_composition_method(method.as_str()) && margs.len() == 1 =>
                {
                    // Build F(g) and F(f) synthetic exprs and normalise
                    let fng = make_call_expr(func.as_ref().clone(), receiver.as_ref().clone());
                    let fnf = make_call_expr(func.as_ref().clone(), margs[0].clone());
                    let l = normalise_cat_with_functor(&fng, budget - 1);
                    let r = normalise_cat_with_functor(&fnf, budget - 1);
                    return l.compose(r);
                }
                _ => {}
            }
        }
    }

    // Delegate to base category normaliser
    normalise_cat(expr, budget)
}

/// Check whether two [`CatNorm`] values are equal.
fn cat_eq(a: &CatNorm, b: &CatNorm) -> bool {
    a == b
}

/// Known composition-like method names from the Verum category ecosystem.
/// Includes stdlib Category.compose, operator aliases, and common variants.
fn is_composition_method(name: &str) -> bool {
    matches!(
        name,
        "compose"
            | "then"
            | "after"
            | "comp"
            | "andThen"
            | "∘"
            | "·"
            | "<<<"
            | ">>>"
            | "compose_morphisms"
            | "compose_functors"
    )
}

/// Known identity-morphism method/variable names.
fn is_identity_method(name: &str) -> bool {
    matches!(
        name,
        "id" | "identity" | "id_morphism" | "identity_morphism"
    )
}

/// Check whether an expression is shaped like a descent condition.
///
/// Matches:
/// - A call to `descent_condition(…)` or `compatible_sections(…)`
/// - A method call with descent-related names
/// - An equality whose sides recursively look like descent expressions
fn is_descent_shaped(expr: &Expr) -> bool {
    match &expr.kind {
        // Direct call to known descent operations
        ExprKind::Call { func, .. } => {
            if let Some(name) = func_head_name(func) {
                matches!(
                    name.as_str(),
                    "descent_condition"
                        | "compatible_sections"
                        | "gluing_condition"
                        | "sheaf_condition"
                        | "check_descent"
                        | "verify_descent"
                        | "cech_condition"
                        | "cosimplicial_limit"
                )
            } else {
                false
            }
        }
        // Method call with descent-related names
        ExprKind::MethodCall { method, .. } => {
            matches!(
                method.as_str(),
                "restrict"
                    | "restriction"
                    | "sections"
                    | "descent"
                    | "glue"
                    | "patch"
                    | "pullback"
                    | "base_change"
            )
        }
        // Equality involving descent structures
        ExprKind::Binary { op: BinOp::Eq, left, right } => {
            is_descent_shaped(left) || is_descent_shaped(right)
        }
        ExprKind::Paren(inner) => is_descent_shaped(inner),
        _ => false,
    }
}

/// Construct a synthetic `F(arg)` call expression for functor law unfolding.
///
/// Uses the real span from `func` so that any diagnostics produced during
/// normalisation point to the original source location rather than a dummy
/// span.
fn make_call_expr(func: Expr, arg: Expr) -> Expr {
    let span = func.span; // Use the REAL span from the function, not dummy
    let mut args = List::new();
    args.push(arg);
    Expr::new(
        ExprKind::Call {
            func: Box::new(func),
            type_args: List::new(),
            args,
        },
        span,
    )
}

// =============================================================================
// Section 7: Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::{Expr, ExprKind, Ident, Span};
    use verum_common::Text;

    // ---- CubicalNorm helpers ----

    fn atom(s: &str) -> CubicalNorm {
        CubicalNorm::Atom(Text::from(s))
    }

    fn refl_norm(x: CubicalNorm) -> CubicalNorm {
        CubicalNorm::Refl(Box::new(x))
    }

    fn transport_norm(line: CubicalNorm, value: CubicalNorm) -> CubicalNorm {
        CubicalNorm::Transport {
            line: Box::new(line),
            value: Box::new(value),
        }
    }

    // ---- CubicalNorm reduction tests ----

    #[test]
    fn test_transport_refl_reduces() {
        let t = transport_norm(refl_norm(atom("A")), atom("x"));
        assert_eq!(t.whnf(), atom("x"));
    }

    #[test]
    fn test_sym_refl_reduces() {
        let t = CubicalNorm::Sym(Box::new(refl_norm(atom("x"))));
        assert_eq!(t.whnf(), refl_norm(atom("x")));
    }

    #[test]
    fn test_hcomp_refl_sides_reduces() {
        let t = CubicalNorm::Hcomp {
            base: Box::new(atom("base")),
            sides: Box::new(refl_norm(atom("s"))),
        };
        assert_eq!(t.whnf(), atom("base"));
    }

    #[test]
    fn test_transport_ua_reduces_to_fwd() {
        let t = transport_norm(
            CubicalNorm::Ua(Box::new(atom("my_equiv"))),
            atom("x"),
        );
        assert_eq!(
            t.whnf(),
            CubicalNorm::EquivFwd {
                equiv: Box::new(atom("my_equiv")),
                value: Box::new(atom("x")),
            }
        );
    }

    #[test]
    fn test_ua_id_equiv_reduces_to_refl_type() {
        let t = CubicalNorm::Ua(Box::new(atom("id_equiv")));
        assert_eq!(t.whnf(), refl_norm(atom("Type")));
    }

    #[test]
    fn test_path_app_lambda_beta() {
        let t = CubicalNorm::PathApp {
            path: Box::new(CubicalNorm::PathLambda {
                dim: Text::from("i"),
                body: Box::new(CubicalNorm::Dim(Text::from("i"))),
            }),
            at: Box::new(CubicalNorm::Endpoint(IEndpoint::I0)),
        };
        assert_eq!(t.whnf(), CubicalNorm::Endpoint(IEndpoint::I0));
    }

    #[test]
    fn test_definitionally_equal_after_transport_refl() {
        let lhs = transport_norm(refl_norm(atom("A")), atom("x"));
        let rhs = atom("x");
        assert!(lhs.definitionally_equal(&rhs));
    }

    #[test]
    fn test_not_definitionally_equal_different_atoms() {
        let lhs = atom("x");
        let rhs = atom("y");
        assert!(!lhs.definitionally_equal(&rhs));
    }

    // ---- ProofGoal-level tactic tests ----

    fn make_eq_goal(lhs: &str, rhs: &str) -> ProofGoal {
        let span = Span::dummy();
        let make_path = |s: &str| -> Expr {
            let ident = Ident::new(s, span);
            Expr::new(ExprKind::Path(verum_ast::Path::from_ident(ident)), span)
        };
        let goal_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(make_path(lhs)),
                right: Box::new(make_path(rhs)),
            },
            span,
        );
        ProofGoal::new(goal_expr)
    }

    #[test]
    fn test_try_cubical_same_atom_proves() {
        let goal = make_eq_goal("x", "x");
        assert!(try_cubical(&goal).is_ok());
        let result = try_cubical(&goal).unwrap();
        assert!(result.is_empty(), "proved goal should have no subgoals");
    }

    #[test]
    fn test_try_cubical_different_atoms_falls_back() {
        let goal = make_eq_goal("x", "y");
        let result = try_cubical(&goal);
        assert!(result.is_err(), "different atoms should produce smt fallback error");
    }

    #[test]
    fn test_try_category_simp_id_proves() {
        // id == id  should be proved by category_simp
        let goal = make_eq_goal("id", "id");
        assert!(try_category_simp(&goal).is_ok());
    }

    #[test]
    fn test_execute_cubical_not_applicable_non_eq() {
        let span = Span::dummy();
        let ident = Ident::new("foo", span);
        let non_eq_expr = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(ident)),
            span,
        );
        let goal = ProofGoal::new(non_eq_expr);
        assert!(matches!(
            execute_cubical_tactic(&goal),
            CubicalTacticOutcome::NotApplicable
        ));
    }

    #[test]
    fn test_descent_check_not_applicable_plain_eq() {
        let goal = make_eq_goal("a", "b");
        assert!(matches!(
            execute_descent_check_tactic(&goal),
            CubicalTacticOutcome::NotApplicable
        ));
    }

    #[test]
    fn test_cat_norm_identity_stripped() {
        // CatNorm::identity should have no atoms
        let id = CatNorm::identity();
        assert!(id.atoms.is_empty());
    }

    #[test]
    fn test_cat_norm_compose_flattens() {
        let f = CatNorm::atom(Text::from("f"));
        let g = CatNorm::atom(Text::from("g"));
        let composed = f.compose(g);
        assert_eq!(composed.atoms.len(), 2);
        assert_eq!(composed.atoms[0].as_str(), "f");
        assert_eq!(composed.atoms[1].as_str(), "g");
    }
}
