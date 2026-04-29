//! Industrial-tactic fast path — adapt the deterministic kernel
//! tactics from `verum_kernel::tactics_industrial` into the SMT
//! tactic dispatcher's call site.
//!
//! ## What this delivers
//!
//! The kernel's industrial tactics (`tactic_lia`, `tactic_decide`,
//! `tactic_induction`, `tactic_congruence`, `tactic_eauto`) are
//! deterministic decision procedures that close certain subgoal
//! shapes in milliseconds without needing Z3.  This module exposes
//! them as a fast-path callable from the existing SMT tactic
//! dispatcher (`tactics.rs`) — when the dispatcher selects e.g.
//! `TacticKind::LIA`, it tries the industrial fast-path first; on
//! a `Closed` outcome it returns immediately, on `Open` it falls
//! through to Z3's tactic.
//!
//! ## Integration contract
//!
//! [`try_industrial_fast_path`] takes a tactic-kind name + a parsed
//! argument bundle (already routed through the kernel's typed
//! tactic surface) and returns:
//!
//!   * `Some(closed_witness)` — the kernel decided the goal; the
//!     witness should be packaged as an SMT certificate so the
//!     Z3 dispatch can be skipped.
//!   * `None` — fall through to Z3.
//!
//! ## What this UNBLOCKS
//!
//!   - `apply lia` / `apply decide` / `apply induction` /
//!     `apply congruence` / `apply eauto` in MSFS proof bodies
//!     can now route through deterministic kernel tactics for the
//!     decidable subset, leaving Z3 for the genuinely-undecidable
//!     residue.
//!   - The dispatcher's CPU/latency budget shifts from "always Z3"
//!     to "Z3 only when industrial fails" — a measurable
//!     compile-time win on proof scripts dominated by trivial
//!     constraints.

use verum_common::Text;
use verum_kernel::tactics_industrial::{
    BoolFormula, CongruenceEquation, EautoHint, LinearConstraint,
    TacticOutcome, tactic_congruence, tactic_decide, tactic_eauto,
    tactic_induction, tactic_lia,
};

/// Kind of tactic the fast-path dispatcher recognises.  Mirrors the
/// names used in `apply X` in proof bodies — the dispatcher routes
/// via this enum rather than parsing a string per call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndustrialTactic {
    /// Linear Integer Arithmetic (`apply lia`).
    Lia,
    /// Boolean tautology decision (`apply decide`).
    Decide,
    /// Structural induction on ℕ (`apply induction`).
    Induction,
    /// Congruence closure on EUF (`apply congruence`).
    Congruence,
    /// Bounded back-chaining (`apply eauto`).
    Eauto,
}

impl IndustrialTactic {
    /// Diagnostic name (matches proof-DSL spelling).
    pub fn name(&self) -> &'static str {
        match self {
            IndustrialTactic::Lia => "lia",
            IndustrialTactic::Decide => "decide",
            IndustrialTactic::Induction => "induction",
            IndustrialTactic::Congruence => "congruence",
            IndustrialTactic::Eauto => "eauto",
        }
    }

    /// Parse a tactic name from the proof DSL.  Returns `None` for
    /// names not handled by the industrial fast-path.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "lia" => Some(IndustrialTactic::Lia),
            "decide" => Some(IndustrialTactic::Decide),
            "induction" => Some(IndustrialTactic::Induction),
            "congruence" => Some(IndustrialTactic::Congruence),
            "eauto" => Some(IndustrialTactic::Eauto),
            _ => None,
        }
    }
}

/// Argument bundle passed through to the fast-path.  The shape is
/// tactic-specific and the caller is responsible for parsing the
/// proof-DSL form into one of these variants.
#[derive(Debug, Clone)]
pub enum IndustrialArgs {
    Lia {
        constraints: Vec<LinearConstraint>,
    },
    Decide {
        formula: BoolFormula,
    },
    Induction {
        goal_name: Text,
        base_closed: bool,
        step_closed: bool,
    },
    Congruence {
        equations: Vec<CongruenceEquation>,
        target: CongruenceEquation,
        universe_size: u32,
    },
    Eauto {
        hints: Vec<EautoHint>,
        goal: u32,
        bound: u32,
    },
}

/// The closing witness emitted by the fast-path.  When `Some(_)`,
/// the SMT dispatcher should skip Z3 and instead encode the witness
/// as an `SmtCertificate` (V1 wiring; V0 returns the raw witness
/// for re-checking).
#[derive(Debug, Clone, PartialEq)]
pub struct FastPathClosure {
    /// Diagnostic name of the closing tactic.
    pub tactic_name: Text,
    /// The re-checkable witness.
    pub witness: Text,
}

/// Try to close the goal via the industrial fast-path.  Returns
/// `Some(FastPathClosure)` when the kernel's decision procedure
/// returns `Closed`; returns `None` otherwise (caller falls through
/// to Z3).
pub fn try_industrial_fast_path(
    tactic: IndustrialTactic,
    args: IndustrialArgs,
) -> Option<FastPathClosure> {
    let outcome = match (&tactic, args) {
        (IndustrialTactic::Lia, IndustrialArgs::Lia { constraints }) => {
            tactic_lia(&constraints)
        }
        (IndustrialTactic::Decide, IndustrialArgs::Decide { formula }) => {
            tactic_decide(&formula)
        }
        (
            IndustrialTactic::Induction,
            IndustrialArgs::Induction {
                goal_name,
                base_closed,
                step_closed,
            },
        ) => tactic_induction(goal_name, base_closed, step_closed),
        (
            IndustrialTactic::Congruence,
            IndustrialArgs::Congruence {
                equations,
                target,
                universe_size,
            },
        ) => tactic_congruence(&equations, &target, universe_size),
        (
            IndustrialTactic::Eauto,
            IndustrialArgs::Eauto {
                hints,
                goal,
                bound,
            },
        ) => tactic_eauto(&hints, goal, bound),
        _ => return None, // tactic / args mismatch
    };

    match outcome {
        TacticOutcome::Closed {
            tactic_name,
            witness,
        } => Some(FastPathClosure {
            tactic_name,
            witness,
        }),
        TacticOutcome::Open { .. } => None,
    }
}

/// Lookup-by-name dispatch.  Returns `Some(FastPathClosure)` when
/// `name` matches a fast-path tactic AND the kernel decides the
/// goal.  Used by the proof-DSL resolver as a single entry point.
pub fn dispatch_by_name(
    name: &str,
    args: IndustrialArgs,
) -> Option<FastPathClosure> {
    let tactic = IndustrialTactic::from_name(name)?;
    try_industrial_fast_path(tactic, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_kernel::tactics_industrial::LinearRelation;

    // ----- IndustrialTactic name parsing -----

    #[test]
    fn from_name_recognises_all_five() {
        assert_eq!(IndustrialTactic::from_name("lia"), Some(IndustrialTactic::Lia));
        assert_eq!(IndustrialTactic::from_name("decide"), Some(IndustrialTactic::Decide));
        assert_eq!(IndustrialTactic::from_name("induction"), Some(IndustrialTactic::Induction));
        assert_eq!(IndustrialTactic::from_name("congruence"), Some(IndustrialTactic::Congruence));
        assert_eq!(IndustrialTactic::from_name("eauto"), Some(IndustrialTactic::Eauto));
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(IndustrialTactic::from_name("auto"), None);
        assert_eq!(IndustrialTactic::from_name(""), None);
        assert_eq!(IndustrialTactic::from_name("LIA"), None);  // case-sensitive
    }

    // ----- LIA fast-path -----

    #[test]
    fn lia_fast_path_closes_trivial_unsat() {
        let outcome = dispatch_by_name(
            "lia",
            IndustrialArgs::Lia {
                constraints: vec![LinearConstraint {
                    coeffs: vec![],
                    rel: LinearRelation::Eq,
                    k: 1,
                }],
            },
        );
        assert!(outcome.is_some());
        assert_eq!(outcome.as_ref().unwrap().tactic_name.as_str(), "lia");
    }

    #[test]
    fn lia_fast_path_falls_through_on_non_trivial() {
        let outcome = dispatch_by_name(
            "lia",
            IndustrialArgs::Lia {
                constraints: vec![LinearConstraint {
                    coeffs: vec![1, 2],
                    rel: LinearRelation::Le,
                    k: 5,
                }],
            },
        );
        assert!(outcome.is_none(),
            "Non-trivial LIA falls through to Z3");
    }

    // ----- Decide fast-path -----

    #[test]
    fn decide_fast_path_closes_tautology() {
        // A ∨ ¬A
        let formula = BoolFormula::Or(
            Box::new(BoolFormula::Atom(0)),
            Box::new(BoolFormula::Not(Box::new(BoolFormula::Atom(0)))),
        );
        let outcome = dispatch_by_name(
            "decide",
            IndustrialArgs::Decide { formula },
        );
        assert!(outcome.is_some());
    }

    #[test]
    fn decide_fast_path_falls_through_on_non_tautology() {
        let formula = BoolFormula::And(
            Box::new(BoolFormula::Atom(0)),
            Box::new(BoolFormula::Atom(1)),
        );
        let outcome = dispatch_by_name(
            "decide",
            IndustrialArgs::Decide { formula },
        );
        assert!(outcome.is_none());
    }

    // ----- Induction fast-path -----

    #[test]
    fn induction_fast_path_closes_when_both_subgoals_closed() {
        let outcome = dispatch_by_name(
            "induction",
            IndustrialArgs::Induction {
                goal_name: Text::from("∀n. P(n)"),
                base_closed: true,
                step_closed: true,
            },
        );
        assert!(outcome.is_some());
    }

    // ----- Congruence fast-path -----

    #[test]
    fn congruence_fast_path_closes_via_transitivity() {
        let outcome = dispatch_by_name(
            "congruence",
            IndustrialArgs::Congruence {
                equations: vec![
                    CongruenceEquation { lhs: 0, rhs: 1 },
                    CongruenceEquation { lhs: 1, rhs: 2 },
                ],
                target: CongruenceEquation { lhs: 0, rhs: 2 },
                universe_size: 3,
            },
        );
        assert!(outcome.is_some());
    }

    // ----- Eauto fast-path -----

    #[test]
    fn eauto_fast_path_closes_via_axiom() {
        let outcome = dispatch_by_name(
            "eauto",
            IndustrialArgs::Eauto {
                hints: vec![EautoHint { head: 0, body: vec![] }],
                goal: 0,
                bound: 0,
            },
        );
        assert!(outcome.is_some());
    }

    // ----- Mismatched args -----

    #[test]
    fn dispatch_returns_none_on_tactic_args_mismatch() {
        // LIA tactic with Decide args.
        let outcome = try_industrial_fast_path(
            IndustrialTactic::Lia,
            IndustrialArgs::Decide {
                formula: BoolFormula::True,
            },
        );
        assert!(outcome.is_none());
    }

    #[test]
    fn dispatch_returns_none_for_unknown_name() {
        let outcome = dispatch_by_name(
            "auto",  // not in fast-path set
            IndustrialArgs::Lia {
                constraints: vec![],
            },
        );
        assert!(outcome.is_none());
    }
}
