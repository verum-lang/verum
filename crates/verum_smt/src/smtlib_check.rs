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
///
/// Type alias for the canonical `backend_trait::SatResult` — three
/// SAT verdicts (`Sat` / `Unsat` / `Unknown`) with SMT-LIB2 lowercase
/// wire form and a definitive partition. Before the 5-way
/// unification this was a separate `enum CheckVerdict` with an
/// identical shape; the alias preserves the public surface
/// (`CheckVerdict::Sat`, `CheckVerdict::ALL`, `as_str`, `from_str`,
/// `is_definitive`) while collapsing the structural duplication.
pub type CheckVerdict = crate::backend_trait::SatResult;

/// Type-alias projection name. The canonical projection lives in
/// `backend_trait::SatResultMeta`; this alias keeps the
/// `CheckVerdictMeta` spelling that older callers reference.
pub type CheckVerdictMeta = crate::backend_trait::SatResultMeta;

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

    // Wire-form / round-trip / definitive-partition pins live with
    // the canonical `backend_trait::SatResult`. After the 5-way
    // unification `CheckVerdict` is a thin alias of that type, so
    // pinning the same shape here would only test the alias
    // boundary — covered by `alias_contract` below.

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

    /// Pin the `CheckVerdict ≡ backend_trait::SatResult` alias
    /// contract. If a future refactor accidentally re-introduces a
    /// distinct nominal type the cross-assignments below stop
    /// compiling, so the public `CheckVerdict` surface and the
    /// canonical SatResult cannot drift apart silently.
    #[test]
    fn alias_contract() {
        let canonical: crate::backend_trait::SatResult = CheckVerdict::Sat;
        let _: CheckVerdict = canonical;
        // Method surface preserved through the alias.
        assert_eq!(CheckVerdict::Sat.as_str(), "sat");
        assert_eq!(CheckVerdict::from_str("unsat"), Some(CheckVerdict::Unsat));
    }
}
