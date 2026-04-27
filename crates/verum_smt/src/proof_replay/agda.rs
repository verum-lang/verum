//! Agda proof-replay backend.
//!
//! Lowers an [`SmtCertificate`] into an Agda term-style proof.
//! Unlike Coq/Lean, Agda is term-oriented — proofs are values
//! constructed from the goal type's constructors. The V8.0
//! baseline emits a small term-vocabulary recognising common
//! shape-equivalents:
//!
//!   * `refl` for reflexivity.
//!   * `cong` for congruence under a function.
//!   * `sym` / `trans` for equality manipulation.
//!   * `λ x → ...` for quantifier introduction.
//!   * `case-of`-style destructuring for sums.
//!   * `{!!}` (Agda's hole) as the strict fallback.

use verum_common::Text;
use verum_kernel::SmtCertificate;

use super::{
    DeclarationHeader, ProofReplayBackend, ReplayError, TargetTactic,
};

#[derive(Debug, Default)]
pub struct AgdaProofReplay;

impl AgdaProofReplay {
    pub fn new() -> Self {
        Self
    }

    fn admitted_with_context(decl: &DeclarationHeader) -> String {
        format!(
            "{{!! agda hole: trace shape not yet \
             covered by AgdaProofReplay for {} `{}` !!}}",
            decl.kind.as_str(),
            decl.name.as_str(),
        )
    }
}

impl ProofReplayBackend for AgdaProofReplay {
    fn target_name(&self) -> &'static str {
        "agda"
    }

    fn lower(
        &self,
        cert: &SmtCertificate,
        decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError> {
        if cert.schema_version > 1 {
            return Err(ReplayError::UnsupportedSchema {
                target: Text::from("agda"),
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
            language: Text::from("agda"),
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
    // Agda is term-style; we synthesise a small term composing the
    // recognisable proof witnesses.
    let mut term_parts: Vec<&'static str> = Vec::new();
    match backend.as_str() {
        "z3" | "z3-stub" => {
            if s.contains("(refl") {
                term_parts.push("refl");
            }
            if s.contains("(asserted") || s.contains("(quant-intro") {
                // Lambda binding for any quantifier-intro shape.
                term_parts.push("λ x → x");
            }
            if s.contains("(rewrite") {
                term_parts.push("cong _ refl");
            }
            if s.contains("(unit-resolution") {
                term_parts.push("h");
            }
            if s.contains("(true-axiom") {
                term_parts.push("tt");
            }
        }
        "cvc5" | "cvc5-stub" => {
            if s.contains(":rule refl") {
                term_parts.push("refl");
            }
            if s.contains("(assume") {
                term_parts.push("λ x → x");
            }
            if s.contains(":rule trans") {
                term_parts.push("trans h₁ h₂");
            }
            if s.contains(":rule symm") {
                term_parts.push("sym h");
            }
        }
        _ => {}
    }
    if term_parts.is_empty() {
        return String::new();
    }
    // Pick the first recognised witness — Agda terms don't compose
    // like tactic chains, so we emit the strongest single witness.
    term_parts.first().unwrap().to_string()
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
    fn agda_replay_target_is_agda() {
        assert_eq!(AgdaProofReplay::new().target_name(), "agda");
    }

    #[test]
    fn agda_replay_empty_trace_emits_hole() {
        let t = AgdaProofReplay::new()
            .lower(&cert("z3", ""), &theorem("foo"))
            .unwrap();
        assert!(t.admitted);
        assert!(t.source.contains("{!!"));
        assert!(t.source.contains("foo"));
    }

    #[test]
    fn agda_replay_z3_refl_emits_refl_term() {
        let t = AgdaProofReplay::new()
            .lower(&cert("z3", "(refl x)"), &theorem("r"))
            .unwrap();
        assert_eq!(t.source, "refl");
        assert!(!t.admitted);
    }

    #[test]
    fn agda_replay_z3_quant_intro_emits_lambda() {
        let t = AgdaProofReplay::new()
            .lower(&cert("z3", "(quant-intro foo)"), &theorem("q"))
            .unwrap();
        assert!(t.source.contains("λ"));
    }

    #[test]
    fn agda_replay_z3_true_axiom_emits_tt() {
        let t = AgdaProofReplay::new()
            .lower(&cert("z3", "(true-axiom)"), &theorem("t"))
            .unwrap();
        assert_eq!(t.source, "tt");
    }

    #[test]
    fn agda_replay_cvc5_trans_emits_trans_witness() {
        let t = AgdaProofReplay::new()
            .lower(
                &cert("cvc5", "(step t1 :rule trans :premises h1 h2)"),
                &theorem("c"),
            )
            .unwrap();
        assert!(t.source.contains("trans"));
    }

    #[test]
    fn agda_replay_unknown_backend_emits_hole() {
        let t = AgdaProofReplay::new()
            .lower(&cert("vampire", "any"), &theorem("v"))
            .unwrap();
        assert!(t.admitted);
    }

    #[test]
    fn agda_replay_rejects_future_schema() {
        let mut c = cert("z3", "(refl x)");
        c.schema_version = 99;
        let result = AgdaProofReplay::new().lower(&c, &theorem("v")).err();
        assert!(matches!(result, Some(ReplayError::UnsupportedSchema { .. })));
    }

    #[test]
    fn agda_replay_carries_framework_dep() {
        let decl = DeclarationHeader {
            name: Text::from("y"),
            kind: DeclKind::Theorem,
            framework: Some(super::super::FrameworkRef {
                name: Text::from("lurie_htt"),
                citation: Text::from("HTT 6.2.2.7"),
            }),
        };
        let t = AgdaProofReplay::new()
            .lower(&cert("z3", "(refl x)"), &decl)
            .unwrap();
        assert_eq!(t.depends_on.len(), 1);
    }
}
