//! Lean 4 proof-replay backend.
//!
//! Lowers an [`SmtCertificate`] into a Lean 4 tactic block. Z3-shape
//! and CVC5-shape proof traces are recognised; unsupported shapes
//! and empty traces fall back to `sorry`.
//!
//! Tactic vocabulary (V7.0 baseline):
//!   * `intros` / `intro` for quantifier introduction.
//!   * `rfl` / `Eq.refl` for reflexivity close-out.
//!   * `simp` / `simp_all` for rewrite normalisation.
//!   * `linarith` / `omega` for linear-integer arithmetic.
//!   * `constructor` / `cases` for ∧/∨ destructuring.
//!   * `apply <hyp>` / `exact <hyp>` for forward-chaining.
//!   * `trivial` / `tauto` for proof-search close-out.
//!   * `sorry` as the strict fallback.

use verum_common::Text;
use verum_kernel::SmtCertificate;

use super::{
    DeclarationHeader, DeclKind, ProofReplayBackend, ReplayError, TargetTactic,
};

#[derive(Debug, Default)]
pub struct LeanProofReplay;

impl LeanProofReplay {
    pub fn new() -> Self {
        Self
    }

    /// Lean 4 tactic-block envelope: `by\n  <tactics>` (term-style)
    /// or `:= by <tactics>`. Returns the tactic-block body (no
    /// outer `theorem ... :=` framing — that's the emitter's job).
    fn wrap_tactics(body: &str) -> String {
        format!("by\n{}\n", body)
    }

    fn admitted_with_context(decl: &DeclarationHeader) -> String {
        format!(
            "by\n  -- sorry: trace shape not yet \
             covered by LeanProofReplay for {} `{}`\n  sorry\n",
            decl.kind.as_str(),
            decl.name.as_str(),
        )
    }
}

impl ProofReplayBackend for LeanProofReplay {
    fn target_name(&self) -> &'static str {
        "lean"
    }

    fn lower(
        &self,
        cert: &SmtCertificate,
        decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError> {
        if cert.schema_version > 1 {
            return Err(ReplayError::UnsupportedSchema {
                target: Text::from("lean"),
                found: cert.schema_version,
                max_supported: 1,
            });
        }
        let body = match cert.backend.as_str() {
            "z3" | "z3-stub" => lower_z3_trace(&cert.trace, decl),
            "cvc5" | "cvc5-stub" => lower_cvc5_trace(&cert.trace, decl),
            _ => lower_generic_trace(&cert.trace, decl),
        };
        let (source, admitted) = if body.is_empty() {
            (Self::admitted_with_context(decl), true)
        } else {
            (Self::wrap_tactics(&body), false)
        };
        let mut deps: Vec<Text> = Vec::new();
        if let Some(fw) = &decl.framework {
            deps.push(fw.name.clone());
        }
        Ok(TargetTactic {
            language: Text::from("lean"),
            source,
            depends_on: deps,
            admitted,
        })
    }
}

fn lower_z3_trace(trace: &verum_common::List<u8>, decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let s = bytes_to_string(trace);
    let mut tactics: Vec<&'static str> = Vec::new();
    if s.contains("(asserted") || s.contains("(quant-intro") {
        tactics.push("  intros");
    }
    if s.contains("(rewrite") {
        tactics.push("  simp_all");
    }
    if s.contains("(and-elim") || s.contains("(not-or-elim") {
        tactics.push("  cases h");
    }
    if s.contains("(unit-resolution") {
        tactics.push("  exact h");
    }
    if s.contains("(th-lemma") {
        tactics.push("  linarith");
    }
    if s.contains("(true-axiom") {
        tactics.push("  trivial");
    }
    if s.contains("(refl") {
        tactics.push("  rfl");
    }
    if matches!(decl.kind, DeclKind::Lemma | DeclKind::Theorem | DeclKind::Corollary) {
        tactics.push("  tauto");
    }
    if tactics.is_empty() {
        return String::new();
    }
    tactics.join("\n")
}

fn lower_cvc5_trace(trace: &verum_common::List<u8>, decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let s = bytes_to_string(trace);
    let mut tactics: Vec<&'static str> = Vec::new();
    if s.contains("(assume") {
        tactics.push("  intros");
    }
    if s.contains(":rule la_") {
        tactics.push("  linarith");
    }
    if s.contains(":rule eq_resolve") || s.contains(":rule trans") {
        tactics.push("  simp_all");
    }
    if s.contains(":rule and") {
        tactics.push("  constructor");
    }
    if s.contains(":rule modus_ponens") {
        tactics.push("  apply h");
    }
    if s.contains(":rule refl") {
        tactics.push("  rfl");
    }
    if matches!(decl.kind, DeclKind::Lemma | DeclKind::Theorem | DeclKind::Corollary) {
        tactics.push("  tauto");
    }
    if tactics.is_empty() {
        return String::new();
    }
    tactics.join("\n")
}

fn lower_generic_trace(trace: &verum_common::List<u8>, _decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    "  intros\n  tauto".to_string()
}

fn bytes_to_string(bytes: &verum_common::List<u8>) -> String {
    let mut out = String::with_capacity(bytes.iter().count());
    for b in bytes.iter() {
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::List;

    fn cert(backend: &str, trace: &str) -> SmtCertificate {
        SmtCertificate::new(
            Text::from(backend),
            Text::from("test"),
            List::from_iter(trace.bytes().collect::<Vec<_>>()),
            Text::from("blake3:t"),
        )
    }

    fn theorem(name: &str) -> DeclarationHeader {
        DeclarationHeader {
            name: Text::from(name),
            kind: DeclKind::Theorem,
            framework: None,
        }
    }

    #[test]
    fn lean_replay_target_is_lean() {
        assert_eq!(LeanProofReplay::new().target_name(), "lean");
    }

    #[test]
    fn lean_replay_empty_trace_emits_sorry() {
        let t = LeanProofReplay::new()
            .lower(&cert("z3", ""), &theorem("foo"))
            .unwrap();
        assert!(t.admitted);
        assert!(t.source.contains("sorry"));
    }

    #[test]
    fn lean_replay_z3_th_lemma_emits_linarith() {
        let t = LeanProofReplay::new()
            .lower(&cert("z3", "(th-lemma arith)"), &theorem("a"))
            .unwrap();
        assert!(t.source.contains("linarith"));
    }

    #[test]
    fn lean_replay_z3_refl_emits_rfl() {
        let t = LeanProofReplay::new()
            .lower(&cert("z3", "(refl x)"), &theorem("r"))
            .unwrap();
        assert!(t.source.contains("rfl"));
    }

    #[test]
    fn lean_replay_z3_rewrite_emits_simp_all() {
        let t = LeanProofReplay::new()
            .lower(&cert("z3", "(rewrite (= a b))"), &theorem("rw"))
            .unwrap();
        assert!(t.source.contains("simp_all"));
    }

    #[test]
    fn lean_replay_cvc5_la_emits_linarith() {
        let t = LeanProofReplay::new()
            .lower(
                &cert("cvc5", "(step t1 (cl phi) :rule la_arith)"),
                &theorem("c"),
            )
            .unwrap();
        assert!(t.source.contains("linarith"));
    }

    #[test]
    fn lean_replay_cvc5_and_emits_constructor() {
        let t = LeanProofReplay::new()
            .lower(&cert("cvc5", "(step :rule and-intro)"), &theorem("a"))
            .unwrap();
        assert!(t.source.contains("constructor"));
    }

    #[test]
    fn lean_replay_unknown_backend_falls_through_to_generic() {
        let t = LeanProofReplay::new()
            .lower(&cert("vampire", "any"), &theorem("v"))
            .unwrap();
        assert!(!t.admitted);
        assert!(t.source.contains("intros"));
        assert!(t.source.contains("tauto"));
    }

    #[test]
    fn lean_replay_rejects_future_schema() {
        let mut c = cert("z3", "(asserted)");
        c.schema_version = 99;
        let result = LeanProofReplay::new().lower(&c, &theorem("v")).err();
        assert!(matches!(result, Some(ReplayError::UnsupportedSchema { .. })));
    }

    #[test]
    fn lean_replay_carries_framework_dep() {
        let decl = DeclarationHeader {
            name: Text::from("y"),
            kind: DeclKind::Theorem,
            framework: Some(super::super::FrameworkRef {
                name: Text::from("lurie_htt"),
                citation: Text::from("HTT 6.2.2.7"),
            }),
        };
        let t = LeanProofReplay::new()
            .lower(&cert("z3", "(asserted)"), &decl)
            .unwrap();
        assert_eq!(t.depends_on.len(), 1);
        assert_eq!(t.depends_on[0].as_str(), "lurie_htt");
    }
}
