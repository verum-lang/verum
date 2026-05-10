//! Direct SMT-LIB 2 file check — the library-layer entry point for
//! `verum verify --check-smt-formula <FILE>`.
//!

//! Accepts a raw SMT-LIB 2 string and dispatches it to the
//! configured solver (currently Z3 only; CVC5 support waits on
//! parser-library linking). The handler is deliberately thin:
//! no AST / type-checker / VC-generator involvement — the input
//! is raw SMT-LIB, the output is the solver's verdict verbatim.
//!

//! Closes task #67's `--check-smt-formula` surface.

/// Verdict returned by [`check_smtlib_string`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckVerdict {
    /// The formula is satisfiable.
    Sat,
    /// The formula is unsatisfiable.
    Unsat,
    /// The solver could not decide within the budget.
    Unknown,
}

/// Per-variant projection for [`CheckVerdict`].
///
/// `name` matches the SMT-LIB2 `(check-sat)` output token verbatim,
/// so a solver-supplied string round-trips through `from_str`/
/// `as_str`. `is_definitive` flags `Sat` and `Unsat` (the two
/// terminating verdicts); `Unknown` indicates timeout / resource
/// exhaustion / undecidable fragment. Same shape as
/// `verum_smt::backend_trait::SatResult` (consolidated in
/// 3d6c96590) — the two enums are structural duplicates that
/// long-term unification can collapse.
#[derive(Debug, Clone, Copy)]
pub struct CheckVerdictMeta {
    pub name: &'static str,
    pub is_definitive: bool,
}

impl CheckVerdict {
    pub const ALL: &'static [Self] = &[Self::Sat, Self::Unsat, Self::Unknown];

    pub const fn meta(self) -> CheckVerdictMeta {
        match self {
            Self::Sat => CheckVerdictMeta {
                name: "sat",
                is_definitive: true,
            },
            Self::Unsat => CheckVerdictMeta {
                name: "unsat",
                is_definitive: true,
            },
            Self::Unknown => CheckVerdictMeta {
                name: "unknown",
                is_definitive: false,
            },
        }
    }

    /// Canonical SMT-LIB verdict string.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Parse the SMT-LIB verdict token back to the typed verdict.
    /// Closes a drift defect: previously `as_str` was present but
    /// no inverse mapping existed, so callers receiving a
    /// `(check-sat)` output line had to re-implement the lookup.
    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// True for `Sat` / `Unsat` (terminating verdicts); false for
    /// `Unknown` (timeout / resource exhaustion / undecidable
    /// fragment).
    #[inline]
    pub const fn is_definitive(&self) -> bool {
        self.meta().is_definitive
    }
}

/// Error reported by [`check_smtlib_string`].
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// Raw SMT-LIB string contained no `(check-sat)` directive —
    /// under-specified.
    #[error("SMT-LIB input contains no `(check-sat)` directive")]
    NoCheckSat,

    /// Requested solver is not supported by this binary.
    #[error("unsupported solver `{0}` for --check-smt-formula")]
    UnsupportedSolver(String),
}

/// Dispatch a raw SMT-LIB 2 string to the configured solver.
///

/// * `content` — the SMT-LIB source (must include
///  `(check-sat)`).
/// * `solver` — `z3` | `auto` | `portfolio` | `capability`
///  currently dispatch through Z3; `cvc5` returns
///  `UnsupportedSolver` because CVC5 parser linking is
///  optional. Unknown values surface as `UnsupportedSolver`.
/// * `timeout_s` — per-query timeout in seconds. Forwarded to
///  Z3 via `set_timeout`.
///

/// Returns the solver verdict on success.
pub fn check_smtlib_string(
    content: &str,
    solver: &str,
    timeout_s: u64,
) -> Result<CheckVerdict, CheckError> {
    if !content.contains("check-sat") {
        return Err(CheckError::NoCheckSat);
    }

    match solver {
        "z3" | "auto" | "portfolio" | "capability" => Ok(check_via_z3(content, timeout_s)),
        "cvc5" => Err(CheckError::UnsupportedSolver(
            "cvc5 (parser library not linked; use --solver=z3)".to_string(),
        )),
        other => Err(CheckError::UnsupportedSolver(other.to_string())),
    }
}

fn check_via_z3(content: &str, timeout_s: u64) -> CheckVerdict {
    use crate::solver_diagnostics;
    use z3::{Config, Solver, with_z3_config};

    // Protocol trace + query dump on the VERUM_SOLVER_PROTOCOL /
    // VERUM_DUMP_SMT_DIR side channels. Both are no-ops when
    // disabled so the pay-for-only-what-you-use contract holds.
    solver_diagnostics::log_send(content);
    solver_diagnostics::dump_smt_query("smtlib-check", content);

    let mut cfg = Config::new();
    cfg.set_timeout_msec(timeout_s.saturating_mul(1000));

    let verdict = with_z3_config(&cfg, || {
        let solver = Solver::new();
        // `from_string` consumes raw SMT-LIB 2 and applies
        // declarations + assertions to the current solver state.
        solver.from_string(content.to_string());

        match solver.check() {
            z3::SatResult::Sat => CheckVerdict::Sat,
            z3::SatResult::Unsat => CheckVerdict::Unsat,
            z3::SatResult::Unknown => CheckVerdict::Unknown,
        }
    });

    solver_diagnostics::log_recv(verdict.as_str());
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_check_sat_is_rejected() {
        let r = check_smtlib_string("(assert true)", "z3", 5);
        assert!(matches!(r, Err(CheckError::NoCheckSat)));
    }

    #[test]
    fn unsupported_solver_reports_name() {
        let r = check_smtlib_string("(check-sat)", "yices", 5);
        match r {
            Err(CheckError::UnsupportedSolver(name)) => {
                assert!(name.contains("yices"));
            }
            other => panic!("expected UnsupportedSolver, got {:?}", other),
        }
    }

    #[test]
    fn cvc5_returns_unsupported_with_guidance() {
        let r = check_smtlib_string("(check-sat)", "cvc5", 5);
        match r {
            Err(CheckError::UnsupportedSolver(msg)) => {
                assert!(msg.contains("cvc5"));
                assert!(msg.contains("use --solver=z3"));
            }
            other => panic!("expected UnsupportedSolver, got {:?}", other),
        }
    }

    #[test]
    fn verdict_as_str_is_canonical() {
        assert_eq!(CheckVerdict::Sat.as_str(), "sat");
        assert_eq!(CheckVerdict::Unsat.as_str(), "unsat");
        assert_eq!(CheckVerdict::Unknown.as_str(), "unknown");
    }

    #[test]
    fn z3_solves_trivial_sat_formula() {
        let content = "(declare-const x Int) (assert (= x 1)) (check-sat)";
        let r = check_smtlib_string(content, "z3", 5);
        assert_eq!(r.unwrap(), CheckVerdict::Sat);
    }

    #[test]
    fn z3_refutes_trivial_unsat_formula() {
        let content = "(assert false) (check-sat)";
        let r = check_smtlib_string(content, "z3", 5);
        assert_eq!(r.unwrap(), CheckVerdict::Unsat);
    }

    #[test]
    fn meta_pin_check_verdict_round_trip_and_definitive_partition() {
        assert_eq!(CheckVerdict::ALL.len(), 3);
        for v in CheckVerdict::ALL {
            let s = v.as_str();
            assert_eq!(CheckVerdict::from_str(s), Some(*v));
        }
        // Wire form matches SMT-LIB2 (check-sat) output tokens.
        assert_eq!(CheckVerdict::Sat.as_str(), "sat");
        assert_eq!(CheckVerdict::Unsat.as_str(), "unsat");
        assert_eq!(CheckVerdict::Unknown.as_str(), "unknown");
        // is_definitive partition: Sat/Unsat true; Unknown false.
        assert!(CheckVerdict::Sat.is_definitive());
        assert!(CheckVerdict::Unsat.is_definitive());
        assert!(!CheckVerdict::Unknown.is_definitive());
        assert!(CheckVerdict::from_str("__bogus__").is_none());
    }
}
