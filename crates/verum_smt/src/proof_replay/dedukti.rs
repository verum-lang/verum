//! Dedukti proof-replay backend.
//!
//! Lowers an [`SmtCertificate`] into a Dedukti `def ... :=` body
//! using λΠ-modulo rewrite-rule style. Dedukti is the most
//! minimal of the five targets: every proof is a λ-term in the
//! `λΠ-calculus modulo rewriting`. The V9.0 baseline emits a
//! small term vocabulary and falls back to `(; admitted ;)`
//! comments when the trace shape isn't recognised.

use verum_common::Text;
use verum_kernel::SmtCertificate;

use super::{
    DeclarationHeader, ProofReplayBackend, ReplayError, TargetTactic,
};

#[derive(Debug, Default)]
pub struct DeduktiProofReplay;

impl DeduktiProofReplay {
    pub fn new() -> Self {
        Self
    }

    fn admitted_with_context(decl: &DeclarationHeader) -> String {
        format!(
            "(; admitted: trace shape not yet \
             covered by DeduktiProofReplay for {} `{}` ;)",
            decl.kind.as_str(),
            decl.name.as_str(),
        )
    }
}

impl ProofReplayBackend for DeduktiProofReplay {
    fn target_name(&self) -> &'static str {
        "dedukti"
    }

    fn lower(
        &self,
        cert: &SmtCertificate,
        decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError> {
        if cert.schema_version > 1 {
            return Err(ReplayError::UnsupportedSchema {
                target: Text::from("dedukti"),
                found: cert.schema_version,
                max_supported: 1,
            });
        }
        let body = lower_trace(&cert.backend, &cert.trace);
        let (source, admitted) = if body.is_empty() {
            (Self::admitted_with_context(decl), true)
        } else {
            (body, false)
        };
        let mut deps: Vec<Text> = Vec::new();
        if let Some(fw) = &decl.framework {
            deps.push(fw.name.clone());
        }
        Ok(TargetTactic {
            language: Text::from("dedukti"),
            source,
            depends_on: deps,
            admitted,
        })
    }
}

fn lower_trace(backend: &Text, trace: &verum_common::List<u8>) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let s = bytes_to_string(trace);
    // Dedukti term-style: emit the strongest single λ-term witness
    // we can recognise. Each proof rule maps to a λΠ-modulo idiom.
    match backend.as_str() {
        "z3" | "z3-stub" => {
            if s.contains("(refl") {
                return "logic.refl _ _".to_string();
            }
            if s.contains("(asserted") {
                return "h0".to_string(); // asserted hypothesis
            }
            if s.contains("(quant-intro") {
                return "x => h x".to_string();
            }
            if s.contains("(rewrite") {
                return "logic.eq_rewrite _ _ _ h refl".to_string();
            }
            if s.contains("(unit-resolution") {
                return "h".to_string();
            }
            if s.contains("(true-axiom") {
                return "logic.True_intro".to_string();
            }
            if s.contains("(th-lemma") {
                // Theory lemma — we don't have a generic Dedukti
                // tactic for arithmetic, fall through to admitted.
                return String::new();
            }
        }
        "cvc5" | "cvc5-stub" => {
            if s.contains(":rule refl") {
                return "logic.refl _ _".to_string();
            }
            if s.contains("(assume") {
                return "h0".to_string();
            }
            if s.contains(":rule trans") {
                return "logic.eq_trans _ _ _ _ h1 h2".to_string();
            }
            if s.contains(":rule symm") {
                return "logic.eq_sym _ _ _ h".to_string();
            }
            if s.contains(":rule modus_ponens") {
                return "h2 h1".to_string();
            }
        }
        _ => {}
    }
    String::new()
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
    use super::super::DeclKind;
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
    fn dedukti_replay_target_is_dedukti() {
        assert_eq!(DeduktiProofReplay::new().target_name(), "dedukti");
    }

    #[test]
    fn dedukti_replay_empty_trace_emits_admitted_comment() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("z3", ""), &theorem("foo"))
            .unwrap();
        assert!(t.admitted);
        assert!(t.source.contains("admitted"));
    }

    #[test]
    fn dedukti_replay_z3_refl_emits_logic_refl() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("z3", "(refl x)"), &theorem("r"))
            .unwrap();
        assert!(t.source.contains("logic.refl"));
    }

    #[test]
    fn dedukti_replay_z3_quant_intro_emits_lambda_arrow() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("z3", "(quant-intro foo)"), &theorem("q"))
            .unwrap();
        assert!(t.source.contains("=>"));
    }

    #[test]
    fn dedukti_replay_z3_rewrite_emits_eq_rewrite() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("z3", "(rewrite (= a b))"), &theorem("rw"))
            .unwrap();
        assert!(t.source.contains("eq_rewrite"));
    }

    #[test]
    fn dedukti_replay_z3_th_lemma_falls_back_to_admitted() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("z3", "(th-lemma arith)"), &theorem("a"))
            .unwrap();
        assert!(t.admitted);
    }

    #[test]
    fn dedukti_replay_cvc5_trans_emits_eq_trans() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("cvc5", "(step :rule trans)"), &theorem("c"))
            .unwrap();
        assert!(t.source.contains("eq_trans"));
    }

    #[test]
    fn dedukti_replay_cvc5_symm_emits_eq_sym() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("cvc5", "(step :rule symm)"), &theorem("s"))
            .unwrap();
        assert!(t.source.contains("eq_sym"));
    }

    #[test]
    fn dedukti_replay_unknown_backend_emits_admitted() {
        let t = DeduktiProofReplay::new()
            .lower(&cert("vampire", "any"), &theorem("v"))
            .unwrap();
        assert!(t.admitted);
    }

    #[test]
    fn dedukti_replay_rejects_future_schema() {
        let mut c = cert("z3", "(refl x)");
        c.schema_version = 99;
        let result = DeduktiProofReplay::new().lower(&c, &theorem("v")).err();
        assert!(matches!(result, Some(ReplayError::UnsupportedSchema { .. })));
    }
}
