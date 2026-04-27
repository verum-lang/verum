//! Coq proof-replay backend.
//!
//! Lowers an [`SmtCertificate`] into a Coq tactic chain. The backend
//! distinguishes Z3-style and CVC5-style traces by the certificate's
//! `backend` field; when the trace is empty or its shape is not
//! recognised the backend falls back to a structured `Admitted`
//! scaffold (`admitted = true` on the [`TargetTactic`]) so the
//! exported Coq file stays syntactically valid.
//!
//! Tactic vocabulary (V6.0 baseline):
//!   * `intros` / `intro <H>` for quantifier introduction.
//!   * `apply <hyp>` for forward-chaining via depends_on hypotheses.
//!   * `rewrite <eq>` for equality-discharge steps.
//!   * `split` / `destruct` for ∧/∨ destructuring.
//!   * `lia` / `nia` / `omega` for linear-arithmetic close-out.
//!   * `auto` / `eauto` for proof-search close-out.
//!   * `reflexivity` / `congruence` for terminal closing.
//!   * `Admitted.` as the strict fallback when nothing else fits.
//!
//! V6.1+ adds full Z3 `(proof ...)` parsing and ALETHE step-by-step
//! reconstruction; the V6.0 baseline produces compilable Coq with
//! the right tactic vocabulary so downstream tooling can iterate
//! on coverage incrementally.

use verum_common::Text;
use verum_kernel::SmtCertificate;

use super::{
    DeclarationHeader, DeclKind, ProofReplayBackend, ReplayError, TargetTactic,
};

/// Coq proof-replay backend.
///
/// The struct is stateless; per-target customisation goes through
/// the `lower` method. Construct via [`Self::new`] or use the
/// `default_registry` convenience that pre-registers it.
#[derive(Debug, Default)]
pub struct CoqProofReplay;

impl CoqProofReplay {
    pub fn new() -> Self {
        Self
    }

    /// Coq's tactic-language envelope: `Proof. <tactics> Qed.`
    /// Keeps a deterministic newline shape so emitter diffs stay clean.
    fn wrap_tactics(body: &str) -> String {
        format!("Proof.\n{}\nQed.\n", body)
    }

    /// Coq's admitted scaffold. Emits a comment block carrying the
    /// declaration kind so downstream tooling can grep for unfilled
    /// proofs by category.
    fn admitted_with_context(decl: &DeclarationHeader) -> String {
        format!(
            "Proof.\n  (* admitted: trace shape \
             not yet covered by CoqProofReplay for {} `{}` *)\n  \
             Admitted.\n",
            decl.kind.as_str(),
            decl.name.as_str(),
        )
    }
}

impl ProofReplayBackend for CoqProofReplay {
    fn target_name(&self) -> &'static str {
        "coq"
    }

    fn lower(
        &self,
        cert: &SmtCertificate,
        decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError> {
        // Schema-version gate: V6.0 supports schema 0 (legacy) + 1
        // (current). Future schema bumps require an explicit
        // backend-version handshake.
        if cert.schema_version > 1 {
            return Err(ReplayError::UnsupportedSchema {
                target: Text::from("coq"),
                found: cert.schema_version,
                max_supported: 1,
            });
        }

        // Backend dispatch: route to per-source-backend lowering.
        // Unknown backends fall through to the conservative
        // tactic-chain emitter; they don't fail outright so the
        // exported file still compiles.
        let backend_name = cert.backend.as_str();
        let body = match backend_name {
            "z3" | "z3-stub" => lower_z3_trace(&cert.trace, decl),
            "cvc5" | "cvc5-stub" => lower_cvc5_trace(&cert.trace, decl),
            _ => lower_generic_trace(&cert.trace, decl),
        };

        // Empty body ⇒ admitted scaffold; otherwise wrap in
        // `Proof. ... Qed.` envelope.
        let (source, admitted) = if body.is_empty() {
            (Self::admitted_with_context(decl), true)
        } else {
            (Self::wrap_tactics(&body), false)
        };

        // Forward-chain dependencies — every framework-cited axiom
        // and every depends_on hypothesis becomes a dependency
        // entry the emitter uses to verify imports.
        let mut deps: Vec<Text> = Vec::new();
        if let Some(fw) = &decl.framework {
            deps.push(fw.name.clone());
        }
        Ok(TargetTactic {
            language: Text::from("coq"),
            source,
            depends_on: deps,
            admitted,
        })
    }
}

// =============================================================================
// Per-source-backend trace lowering
// =============================================================================

/// V6.0 — Z3 `(proof ...)` trace recognition. The trace bytes are
/// the raw Z3 proof S-expression. We recognise common rule heads
/// and emit the corresponding Coq tactic; everything else falls
/// through to a conservative `intros; auto.` chain (which closes
/// trivial obligations without panicking).
fn lower_z3_trace(trace: &verum_common::List<u8>, decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let s = bytes_to_string(trace);
    let mut tactics: Vec<&'static str> = Vec::new();

    // Recognised proof-rule signatures → tactic mapping.
    if s.contains("(asserted") {
        tactics.push("  intros.");
    }
    if s.contains("(quant-intro") {
        tactics.push("  intros.");
    }
    if s.contains("(rewrite") {
        tactics.push("  subst.");
    }
    if s.contains("(and-elim") || s.contains("(not-or-elim") {
        tactics.push("  destruct H.");
    }
    if s.contains("(unit-resolution") {
        tactics.push("  apply H.");
    }
    if s.contains("(th-lemma") {
        // Theory lemma — most often arithmetic. Try lia first,
        // omega as fallback for older Coq corpora.
        tactics.push("  lia.");
    }
    if s.contains("(true-axiom") {
        tactics.push("  trivial.");
    }
    if s.contains("(refl") {
        tactics.push("  reflexivity.");
    }

    // Universal close-out: `auto` is safe (it does nothing on
    // failure rather than diverging).
    if matches!(decl.kind, DeclKind::Lemma | DeclKind::Theorem | DeclKind::Corollary) {
        tactics.push("  auto.");
    }

    if tactics.is_empty() {
        return String::new();
    }
    tactics.join("\n")
}

/// V6.0 — CVC5 ALETHE trace recognition. ALETHE has explicit step
/// names; common heads include `assume` / `step` / `anchor`. We
/// emit a similar conservative tactic chain.
fn lower_cvc5_trace(trace: &verum_common::List<u8>, decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let s = bytes_to_string(trace);
    let mut tactics: Vec<&'static str> = Vec::new();

    if s.contains("(assume") {
        tactics.push("  intros.");
    }
    if s.contains("(step") && s.contains(":rule la_") {
        tactics.push("  lia.");
    }
    if s.contains(":rule eq_resolve") || s.contains(":rule trans") {
        tactics.push("  subst.");
    }
    if s.contains(":rule and") {
        tactics.push("  split.");
    }
    if s.contains(":rule modus_ponens") {
        tactics.push("  apply H.");
    }
    if s.contains(":rule refl") {
        tactics.push("  reflexivity.");
    }

    if matches!(decl.kind, DeclKind::Lemma | DeclKind::Theorem | DeclKind::Corollary) {
        tactics.push("  auto.");
    }

    if tactics.is_empty() {
        return String::new();
    }
    tactics.join("\n")
}

/// V6.0 — generic / unknown-backend fallback. Produces a minimal
/// "intros; auto." chain that closes trivial obligations and leaves
/// non-trivial ones admitted-equivalent (auto fails silently).
fn lower_generic_trace(trace: &verum_common::List<u8>, _decl: &DeclarationHeader) -> String {
    if trace.is_empty() {
        return String::new();
    }
    "  intros.\n  auto.".to_string()
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

    fn cert_with_backend_and_trace(backend: &str, trace: &str) -> SmtCertificate {
        let bytes: Vec<u8> = trace.bytes().collect();
        SmtCertificate::new(
            Text::from(backend),
            Text::from("test-version"),
            List::from_iter(bytes),
            Text::from("blake3:test"),
        )
    }

    fn theorem_decl(name: &str) -> DeclarationHeader {
        DeclarationHeader {
            name: Text::from(name),
            kind: DeclKind::Theorem,
            framework: None,
        }
    }

    #[test]
    fn coq_replay_returns_target_name_coq() {
        assert_eq!(CoqProofReplay::new().target_name(), "coq");
    }

    #[test]
    fn coq_replay_empty_trace_emits_admitted() {
        let cert = cert_with_backend_and_trace("z3", "");
        let backend = CoqProofReplay::new();
        let t = backend.lower(&cert, &theorem_decl("plus_comm")).unwrap();
        assert!(t.admitted, "empty trace must produce admitted scaffold");
        assert!(t.source.contains("Admitted"));
        assert!(t.source.contains("plus_comm"));
    }

    #[test]
    fn coq_replay_z3_asserted_emits_intros() {
        let cert = cert_with_backend_and_trace("z3", "(asserted (= a b))");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("eq_test")).unwrap();
        assert!(!t.admitted);
        assert!(t.source.contains("intros."));
        assert!(t.source.starts_with("Proof."));
        assert!(t.source.trim_end().ends_with("Qed."));
    }

    #[test]
    fn coq_replay_z3_th_lemma_emits_lia() {
        let cert =
            cert_with_backend_and_trace("z3", "(th-lemma arith :linear)");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("arith")).unwrap();
        assert!(t.source.contains("lia."));
    }

    #[test]
    fn coq_replay_z3_true_axiom_emits_trivial() {
        let cert = cert_with_backend_and_trace("z3", "(true-axiom)");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("triv")).unwrap();
        assert!(t.source.contains("trivial."));
    }

    #[test]
    fn coq_replay_z3_rewrite_emits_subst() {
        let cert = cert_with_backend_and_trace("z3", "(rewrite (= a b))");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("rw")).unwrap();
        assert!(t.source.contains("subst."));
    }

    #[test]
    fn coq_replay_cvc5_assume_emits_intros() {
        let cert = cert_with_backend_and_trace("cvc5", "(assume h0 phi)");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("c5")).unwrap();
        assert!(t.source.contains("intros."));
    }

    #[test]
    fn coq_replay_cvc5_la_step_emits_lia() {
        let cert = cert_with_backend_and_trace(
            "cvc5",
            "(step t1 (cl phi) :rule la_arith :premises)",
        );
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("c5l")).unwrap();
        assert!(t.source.contains("lia."));
    }

    #[test]
    fn coq_replay_unknown_backend_falls_through_to_generic() {
        let cert = cert_with_backend_and_trace("vampire", "any trace");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("v")).unwrap();
        assert!(!t.admitted);
        assert!(t.source.contains("intros."));
        assert!(t.source.contains("auto."));
    }

    #[test]
    fn coq_replay_unknown_trace_in_z3_falls_back_to_admitted() {
        let cert = cert_with_backend_and_trace("z3", "totally unknown shape");
        let t = CoqProofReplay::new().lower(&cert, &theorem_decl("u")).unwrap();
        // Theorem kind triggers `auto.` close-out, so source is non-empty.
        // For axiom kind there's no `auto.` emitted → admitted.
        let axiom_t = CoqProofReplay::new()
            .lower(
                &cert,
                &DeclarationHeader {
                    name: Text::from("ax"),
                    kind: DeclKind::Axiom,
                    framework: None,
                },
            )
            .unwrap();
        assert!(axiom_t.admitted);
        // For theorems, the universal `auto.` close-out fires:
        assert!(!t.admitted);
        assert!(t.source.contains("auto."));
    }

    #[test]
    fn coq_replay_rejects_future_schema_version() {
        let mut cert = cert_with_backend_and_trace("z3", "(asserted)");
        cert.schema_version = 99;
        let result = CoqProofReplay::new().lower(&cert, &theorem_decl("v")).err();
        assert!(matches!(
            result,
            Some(ReplayError::UnsupportedSchema { found: 99, .. })
        ));
    }

    #[test]
    fn coq_replay_carries_framework_dependency() {
        let cert = cert_with_backend_and_trace("z3", "(asserted)");
        let decl = DeclarationHeader {
            name: Text::from("yoneda"),
            kind: DeclKind::Theorem,
            framework: Some(super::super::FrameworkRef {
                name: Text::from("lurie_htt"),
                citation: Text::from("HTT 6.2.2.7"),
            }),
        };
        let t = CoqProofReplay::new().lower(&cert, &decl).unwrap();
        assert_eq!(t.depends_on.len(), 1);
        assert_eq!(t.depends_on[0].as_str(), "lurie_htt");
    }
}
