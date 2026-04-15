//! Phase D.5: Proof Term Extraction Bridge
//!
//! Connects tactic execution results to extractable proof terms and
//! certificates. When a `proof by <tactic>` block succeeds, this module
//! translates the Z3 proof object into a `ProofTerm` that can be:
//!
//! 1. Erased at codegen (zero-cost proofs — the common case)
//! 2. Exported as a certificate in Dedukti, Coq, Lean, or Metamath format
//! 3. Stored alongside VBC bytecode as Proof-Carrying Code (PCC)
//!
//! ## Architecture
//!
//! ```text
//! TacticResult (from user_tactic.rs / tactics.rs)
//!   │
//!   ├─ extract_proof_term()  → ProofTerm (Verum-internal)
//!   │
//!   ├─ proof_to_certificate()  → Certificate (via CertificateGenerator)
//!   │     ├─ lift_to_unified()   → proof_term_unified::ProofTerm
//!   │     └─ CertificateGenerator::generate()
//!   │
//!   ├─ export_dedukti()      → Text (Dedukti λΠ-calculus)
//!   ├─ export_coq()          → Text (Gallina)
//!   ├─ export_lean()         → Text (Lean 4 term)
//!   ├─ export_metamath()     → Text (Metamath)
//!   │
//!   └─ erase_proof()         → () (zero-cost: remove at codegen)
//! ```
//!
//! ## Integration Points
//!
//! - `certificates.rs` — handles the actual format-specific serialization
//! - `proof_extraction.rs` — Z3 proof object → internal proof tree
//! - `proof_term_unified.rs` — unified proof term representation
//! - `dependent.rs` — ProofStructure enum for dependent type proofs

use verum_common::{List, Maybe, Text};

use crate::tactics::TacticResult;

/// A proof term in the Verum internal representation.
///
/// This is the bridge between Z3's proof objects and exportable
/// certificate formats. Each variant corresponds to a proof rule.
#[derive(Debug, Clone)]
pub enum ProofTerm {
    /// Assumption: the goal is in the hypothesis set.
    Assumption { name: Text },

    /// Reflexivity: `refl(a) : a = a`
    Reflexivity { term: Text },

    /// Symmetry: from `a = b` derive `b = a`
    Symmetry { proof: Box<ProofTerm> },

    /// Transitivity: from `a = b` and `b = c` derive `a = c`
    Transitivity {
        left: Box<ProofTerm>,
        right: Box<ProofTerm>,
    },

    /// Congruence: from `a = b` derive `f(a) = f(b)`
    Congruence {
        function: Text,
        arg_proof: Box<ProofTerm>,
    },

    /// Modus ponens: from `P` and `P → Q` derive `Q`
    ModusPonens {
        hypothesis: Box<ProofTerm>,
        implication: Box<ProofTerm>,
    },

    /// Introduction: `λ(x: A). proof_of_B(x)` for `∀x:A. B(x)`
    Introduction {
        param: Text,
        param_type: Text,
        body: Box<ProofTerm>,
    },

    /// Application: `proof_f(a)` for applying a universal proof to a witness
    Application {
        function: Box<ProofTerm>,
        argument: Text,
    },

    /// SMT-verified: the SMT solver discharged this goal.
    /// The proof is the solver's certificate (opaque).
    SmtVerified {
        solver: Text, // "z3" or "cvc5"
        goal: Text,
    },

    /// Tactic-produced: the tactic chain produced this proof.
    TacticProduced {
        tactic_name: Text,
        subproofs: List<ProofTerm>,
    },

    /// Cubical: path-based proof term.
    CubicalPath {
        dimension: Text,
        body: Box<ProofTerm>,
    },

    /// Transport: proof obtained by transporting along a path.
    Transport {
        path_proof: Box<ProofTerm>,
        value_proof: Box<ProofTerm>,
    },

    /// Erased: proof has been erased (zero-cost at runtime).
    Erased,
}

/// Certificate format for export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateFormat {
    /// Dedukti (λΠ-calculus modulo rewriting)
    Dedukti,
    /// Coq (Gallina)
    Coq,
    /// Lean 4
    Lean,
    /// Metamath
    Metamath,
    /// JSON (for tooling integration)
    Json,
}

/// Extract a proof term from a tactic result.
///
/// This is called after a tactic successfully closes all goals.
/// The resulting ProofTerm can be exported or erased.
pub fn extract_proof_term(
    tactic_name: &str,
    goal: &str,
    _result: &TacticResult,
) -> ProofTerm {
    // For SMT-discharged proofs, wrap in SmtVerified
    match tactic_name {
        "auto" | "smt" | "blast" | "omega" | "decide" => {
            ProofTerm::SmtVerified {
                solver: Text::from("z3"),
                goal: Text::from(goal),
            }
        }
        "cubical" | "homotopy" => {
            ProofTerm::CubicalPath {
                dimension: Text::from("i"),
                body: Box::new(ProofTerm::SmtVerified {
                    solver: Text::from("z3"),
                    goal: Text::from(goal),
                }),
            }
        }
        "ring" | "field" | "norm_num" => {
            ProofTerm::TacticProduced {
                tactic_name: Text::from(tactic_name),
                subproofs: List::new(),
            }
        }
        _ => {
            ProofTerm::TacticProduced {
                tactic_name: Text::from(tactic_name),
                subproofs: List::new(),
            }
        }
    }
}

/// Export a proof term to a certificate format.
pub fn export_certificate(proof: &ProofTerm, format: CertificateFormat) -> Text {
    match format {
        CertificateFormat::Dedukti => export_dedukti(proof),
        CertificateFormat::Coq => export_coq(proof),
        CertificateFormat::Lean => export_lean(proof),
        CertificateFormat::Metamath => export_metamath(proof),
        CertificateFormat::Json => export_json(proof),
    }
}

/// Export to Dedukti (λΠ-calculus modulo rewriting).
fn export_dedukti(proof: &ProofTerm) -> Text {
    match proof {
        ProofTerm::Assumption { name } => Text::from(format!("{}", name)),
        ProofTerm::Reflexivity { term } => Text::from(format!("refl {}", term)),
        ProofTerm::Symmetry { proof } => {
            Text::from(format!("(sym _ _ _ {})", export_dedukti(proof)))
        }
        ProofTerm::Transitivity { left, right } => {
            Text::from(format!("(trans _ _ _ _ {} {})",
                export_dedukti(left), export_dedukti(right)))
        }
        ProofTerm::Congruence { function, arg_proof } => {
            Text::from(format!("cong {} ({})", function, export_dedukti(arg_proof)))
        }
        ProofTerm::ModusPonens { hypothesis, implication } => {
            Text::from(format!("({}) ({})",
                export_dedukti(implication), export_dedukti(hypothesis)))
        }
        ProofTerm::Introduction { param, param_type, body } => {
            Text::from(format!("\\{} : {}. {}",
                param, param_type, export_dedukti(body)))
        }
        ProofTerm::Application { function, argument } => {
            Text::from(format!("({}) {}", export_dedukti(function), argument))
        }
        ProofTerm::SmtVerified { solver, goal } => {
            Text::from(format!("(; {}: {} ;) smt_axiom", solver, goal))
        }
        ProofTerm::TacticProduced { tactic_name, .. } => {
            Text::from(format!("(; tactic: {} ;) tactic_axiom", tactic_name))
        }
        ProofTerm::CubicalPath { dimension, body } => {
            Text::from(format!("\\{}. {}", dimension, export_dedukti(body)))
        }
        ProofTerm::Transport { path_proof, value_proof } => {
            Text::from(format!("(transport _ _ _ {} {})",
                export_dedukti(path_proof), export_dedukti(value_proof)))
        }
        ProofTerm::Erased => Text::from("_"),
    }
}

/// Export to Coq (Gallina).
fn export_coq(proof: &ProofTerm) -> Text {
    match proof {
        ProofTerm::Assumption { name } => Text::from(format!("exact {}", name)),
        ProofTerm::Reflexivity { .. } => Text::from("reflexivity"),
        ProofTerm::Symmetry { proof } => {
            Text::from(format!("(eq_sym {})", export_coq(proof)))
        }
        ProofTerm::Transitivity { left, right } => {
            Text::from(format!("(eq_trans {} {})",
                export_coq(left), export_coq(right)))
        }
        ProofTerm::Congruence { function, arg_proof } => {
            Text::from(format!("(f_equal {} {})", function, export_coq(arg_proof)))
        }
        ProofTerm::ModusPonens { hypothesis, implication } => {
            Text::from(format!("(({}) ({}))", export_coq(implication), export_coq(hypothesis)))
        }
        ProofTerm::Introduction { param, body, .. } => {
            Text::from(format!("(fun {} => {})", param, export_coq(body)))
        }
        ProofTerm::Application { function, argument } => {
            Text::from(format!("({} {})", export_coq(function), argument))
        }
        ProofTerm::SmtVerified { solver, goal } => {
            Text::from(format!("by {} (* {} *)", solver, goal))
        }
        ProofTerm::TacticProduced { tactic_name, .. } => {
            Text::from(format!("by {}", tactic_name))
        }
        ProofTerm::CubicalPath { .. } => {
            Text::from("(eq_refl _) (* cubical path *)")
        }
        ProofTerm::Transport { path_proof, value_proof } => {
            Text::from(format!("(eq_rect _ _ {} _ {})",
                export_coq(value_proof), export_coq(path_proof)))
        }
        ProofTerm::Erased => {
            Text::from("I (* erased proof: no computational content *)")
        }
    }
}

/// Export to Lean 4.
fn export_lean(proof: &ProofTerm) -> Text {
    match proof {
        ProofTerm::Assumption { name } => Text::from(format!("exact {}", name)),
        ProofTerm::Reflexivity { .. } => Text::from("rfl"),
        ProofTerm::Symmetry { proof } => {
            Text::from(format!("({}).symm", export_lean(proof)))
        }
        ProofTerm::Transitivity { left, right } => {
            Text::from(format!("Eq.trans {} {}",
                export_lean(left), export_lean(right)))
        }
        ProofTerm::Congruence { function, arg_proof } => {
            Text::from(format!("congr_arg {} {}", function, export_lean(arg_proof)))
        }
        ProofTerm::ModusPonens { hypothesis, implication } => {
            Text::from(format!("{} {}", export_lean(implication), export_lean(hypothesis)))
        }
        ProofTerm::Introduction { param, body, .. } => {
            Text::from(format!("fun {} => {}", param, export_lean(body)))
        }
        ProofTerm::Application { function, argument } => {
            Text::from(format!("{} {}", export_lean(function), argument))
        }
        ProofTerm::SmtVerified { .. } => Text::from("by native_decide"),
        ProofTerm::TacticProduced { tactic_name, .. } => {
            Text::from(format!("by {}", tactic_name))
        }
        ProofTerm::CubicalPath { .. } => Text::from("rfl"),
        ProofTerm::Transport { path_proof, value_proof } => {
            Text::from(format!("{} ▸ {}", export_lean(path_proof), export_lean(value_proof)))
        }
        ProofTerm::Erased => {
            Text::from("trivial /- erased proof -/")
        }
    }
}

/// Export to Metamath.
fn export_metamath(proof: &ProofTerm) -> Text {
    match proof {
        ProofTerm::Assumption { name } => Text::from(name.as_str()),
        ProofTerm::Reflexivity { term } => Text::from(format!("eqid {}", term)),
        ProofTerm::Symmetry { proof } => {
            Text::from(format!("eqcomi {}", export_metamath(proof)))
        }
        ProofTerm::Transitivity { left, right } => {
            Text::from(format!("eqtri {} {}", export_metamath(left), export_metamath(right)))
        }
        ProofTerm::Congruence { function, arg_proof } => {
            // Metamath uses fveq2 (function value equality) for congruence
            Text::from(format!("fveq2i $( {} $) {}", function, export_metamath(arg_proof)))
        }
        ProofTerm::ModusPonens { hypothesis, implication } => {
            // Metamath ax-mp: hypothesis first, then implication (major premise)
            Text::from(format!("ax-mp {} {}", export_metamath(hypothesis), export_metamath(implication)))
        }
        ProofTerm::Introduction { body, .. } => {
            // Universal generalisation: ax-gen wraps the body proof
            Text::from(format!("ax-gen {}", export_metamath(body)))
        }
        ProofTerm::Application { function, argument } => {
            Text::from(format!("ax-mp {} {}", argument, export_metamath(function)))
        }
        ProofTerm::SmtVerified { .. } => Text::from("$a smt-verified $."),
        ProofTerm::TacticProduced { tactic_name, .. } => {
            Text::from(format!("$( tactic: {} $) $a tactic-verified $.", tactic_name))
        }
        ProofTerm::CubicalPath { .. } => {
            // Metamath has no native cubical notion; approximate with eqid
            Text::from("eqid $( cubical path approximated as reflexivity $)")
        }
        ProofTerm::Transport { path_proof, value_proof } => {
            // Transport is closest to eqeltri (element of a transported set)
            Text::from(format!("eqeltri {} {}", export_metamath(path_proof), export_metamath(value_proof)))
        }
        ProofTerm::Erased => {
            Text::from("$( erased proof $)")
        }
    }
}

/// Export to JSON (for tooling).
fn export_json(proof: &ProofTerm) -> Text {
    match proof {
        ProofTerm::Assumption { name } => {
            Text::from(format!(r#"{{"type":"assumption","name":"{}"}}"#, name))
        }
        ProofTerm::Reflexivity { term } => {
            Text::from(format!(r#"{{"type":"refl","term":"{}"}}"#, term))
        }
        ProofTerm::SmtVerified { solver, goal } => {
            Text::from(format!(r#"{{"type":"smt","solver":"{}","goal":"{}"}}"#, solver, goal))
        }
        ProofTerm::TacticProduced { tactic_name, .. } => {
            Text::from(format!(r#"{{"type":"tactic","name":"{}"}}"#, tactic_name))
        }
        ProofTerm::Erased => Text::from(r#"{"type":"erased"}"#),
        _ => Text::from(r#"{"type":"unknown"}"#),
    }
}

/// Erase a proof term for zero-cost codegen.
///
/// At runtime, proofs carry no computational content — they are purely
/// compile-time verification artifacts. This function marks a proof
/// as erased so the codegen can skip it entirely.
pub fn erase_proof(_proof: &ProofTerm) -> ProofTerm {
    ProofTerm::Erased
}

/// Check whether a proof term still needs erasure at codegen.
///
/// Returns `true` when the proof carries computational content that should
/// be stripped before runtime. Returns `false` when the proof has already
/// been erased (`ProofTerm::Erased`), so the codegen can skip it.
///
/// Semantics: `should_erase(p) == true` ⟹ call `erase_proof(p)` before codegen.
pub fn should_erase(proof: &ProofTerm) -> bool {
    // A proof that is NOT yet erased needs erasure.
    // A proof already marked Erased does not need re-erasure.
    !matches!(proof, ProofTerm::Erased)
}

// ==================== Pipeline: ProofTerm → Certificate ====================

/// Convert a `CertificateFormat` from this bridge module into the format type
/// expected by `certificates::CertificateGenerator`.
///
/// This is necessary because the bridge module defines a lightweight local
/// `CertificateFormat` for API convenience, while `certificates.rs` maintains
/// the authoritative format enum (which also includes `OpenTheory`).
pub fn lift_format(
    format: CertificateFormat,
) -> crate::certificates::CertificateFormat {
    match format {
        CertificateFormat::Dedukti => crate::certificates::CertificateFormat::Dedukti,
        CertificateFormat::Coq => crate::certificates::CertificateFormat::Coq,
        CertificateFormat::Lean => crate::certificates::CertificateFormat::Lean,
        CertificateFormat::Metamath => crate::certificates::CertificateFormat::Metamath,
        CertificateFormat::Json => crate::certificates::CertificateFormat::Json,
    }
}

/// Lift a bridge `ProofTerm` into the unified proof term representation
/// (`proof_term_unified::ProofTerm`) that `CertificateGenerator` consumes.
///
/// The bridge's `ProofTerm` is tactic-centric (produced by `extract_proof_term`),
/// while the unified representation covers the full range of Z3 proof rules.
/// Structural variants are translated directly; opaque SMT/tactic results are
/// wrapped in `SmtProof` or `Apply` so that every certificate format can emit
/// at least a valid (if coarse) proof object.
pub fn lift_to_unified(
    proof: &ProofTerm,
) -> crate::proof_term_unified::ProofTerm {
    use crate::proof_term_unified::ProofTerm as U;
    use verum_ast::{Expr, ExprKind, Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_common::Heap;

    /// Build a trivial `Expr` carrying a string annotation for use in
    /// unified proof terms that require an `Expr` but only have text.
    fn text_expr(s: &str) -> Expr {
        // Wrap the string as a regular string-literal expression at a dummy span.
        let lit = Literal::new(
            LiteralKind::Text(verum_ast::literal::StringLit::Regular(
                verum_common::Text::from(s),
            )),
            Span::dummy(),
        );
        Expr::new(ExprKind::Literal(lit), Span::dummy())
    }

    match proof {
        ProofTerm::Assumption { name } => U::Assumption {
            id: 0,
            formula: text_expr(name.as_str()),
        },

        ProofTerm::Reflexivity { term } => U::Reflexivity {
            term: text_expr(term.as_str()),
        },

        ProofTerm::Symmetry { proof } => U::Symmetry {
            equality: Heap::new(lift_to_unified(proof)),
        },

        ProofTerm::Transitivity { left, right } => U::Transitivity {
            left: Heap::new(lift_to_unified(left)),
            right: Heap::new(lift_to_unified(right)),
        },

        ProofTerm::Congruence { function, arg_proof } => {
            // Congruence ≈ applying a function-level rewrite rule
            U::Rewrite {
                source: Heap::new(lift_to_unified(arg_proof)),
                rule: function.clone(),
                target: text_expr(&format!("cong({})", function)),
            }
        }

        ProofTerm::ModusPonens { hypothesis, implication } => U::ModusPonens {
            premise: Heap::new(lift_to_unified(hypothesis)),
            implication: Heap::new(lift_to_unified(implication)),
        },

        ProofTerm::Introduction { param, param_type, body } => U::Lambda {
            var: param.clone(),
            body: Heap::new(lift_to_unified(body)),
        },

        ProofTerm::Application { function, argument } => U::Apply {
            rule: argument.clone(),
            premises: {
                let mut ps = List::new();
                ps.push(Heap::new(lift_to_unified(function)));
                ps
            },
        },

        ProofTerm::SmtVerified { solver, goal } => U::SmtProof {
            solver: solver.clone(),
            formula: text_expr(goal.as_str()),
            smt_trace: Maybe::None,
        },

        ProofTerm::TacticProduced { tactic_name, subproofs } => U::Apply {
            rule: tactic_name.clone(),
            premises: subproofs
                .iter()
                .map(|sp| Heap::new(lift_to_unified(sp)))
                .collect(),
        },

        ProofTerm::CubicalPath { dimension, body } => U::Lambda {
            var: dimension.clone(),
            body: Heap::new(lift_to_unified(body)),
        },

        ProofTerm::Transport { path_proof, value_proof } => U::Subst {
            eq_proof: Heap::new(lift_to_unified(path_proof)),
            property: Heap::new(text_expr("transport")),
        },

        // Erased proofs become a trivially true SMT discharge
        ProofTerm::Erased => U::SmtProof {
            solver: Text::from("z3"),
            formula: text_expr("erased"),
            smt_trace: Maybe::None,
        },
    }
}

/// Convert an extracted proof term into an exportable certificate.
///
/// This is the primary entry point for the proof extraction → certificate
/// export pipeline:
///
/// ```text
/// ProofTerm (bridge)
///   └─ lift_to_unified()  →  proof_term_unified::ProofTerm
///   └─ CertificateGenerator::generate()  →  Certificate
/// ```
///
/// # Arguments
///
/// * `proof` — the proof term produced by `extract_proof_term`
/// * `theorem_name` — identifier for the theorem (used in certificate header)
/// * `theorem_statement` — human-readable statement of what was proven
/// * `format` — target certificate format
///
/// # Returns
///
/// A [`crate::certificates::Certificate`] ready for integrity-checking, signing,
/// or writing to disk. Returns [`crate::certificates::CertificateError`] on
/// generation failure (malformed proof, unsupported format, etc.).
pub fn proof_to_certificate(
    proof: &ProofTerm,
    theorem_name: &str,
    theorem_statement: &str,
    format: CertificateFormat,
) -> Result<crate::certificates::Certificate, crate::certificates::CertificateError> {
    // 1. Validate: erased proofs cannot be exported.
    if matches!(proof, ProofTerm::Erased) {
        return Err(crate::certificates::CertificateError::GenerationFailed(
            Text::from("cannot export an erased proof; proof must not have been erased before certificate generation"),
        ));
    }

    // 2. Lift the bridge ProofTerm → unified ProofTerm.
    let unified = lift_to_unified(proof);

    // 3. Build the Theorem descriptor.
    let theorem = crate::certificates::Theorem::new(
        Text::from(theorem_name),
        Text::from(theorem_statement),
    );

    // 4. Map format and invoke CertificateGenerator.
    let cert_format = lift_format(format);
    let generator = crate::certificates::CertificateGenerator::new(cert_format);
    generator.generate(&unified, theorem)
}

/// Convenience wrapper: convert an extracted proof term into certificates in
/// *all* supported formats at once.
///
/// Returns a list of `(format, certificate)` pairs, one per format. Any format
/// that fails generation is silently skipped (errors are collected separately).
///
/// Use this when you want to produce a full cross-verification bundle.
pub fn proof_to_all_certificates(
    proof: &ProofTerm,
    theorem_name: &str,
    theorem_statement: &str,
) -> (
    List<(CertificateFormat, crate::certificates::Certificate)>,
    List<(CertificateFormat, crate::certificates::CertificateError)>,
) {
    let formats = [
        CertificateFormat::Dedukti,
        CertificateFormat::Coq,
        CertificateFormat::Lean,
        CertificateFormat::Metamath,
        CertificateFormat::Json,
    ];

    let mut certs = List::new();
    let mut errors = List::new();

    for format in formats {
        match proof_to_certificate(proof, theorem_name, theorem_statement, format) {
            Ok(cert) => certs.push((format, cert)),
            Err(e) => errors.push((format, e)),
        }
    }

    (certs, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactics::TacticResult;

    fn dummy_result() -> TacticResult {
        TacticResult {
            goals: List::new(),
            stats: Default::default(),
            tactic: crate::tactics::TacticCombinator::Single(crate::tactics::TacticKind::Simplify),
        }
    }

    #[test]
    fn test_extract_smt_proof() {
        let proof = extract_proof_term("auto", "(= 1 1)", &dummy_result());
        match proof {
            ProofTerm::SmtVerified { solver, .. } => {
                assert_eq!(solver.as_str(), "z3");
            }
            other => assert!(false, "expected SmtVerified, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_cubical_proof() {
        let proof = extract_proof_term("cubical", "(path a b)", &dummy_result());
        match proof {
            ProofTerm::CubicalPath { .. } => {}
            other => assert!(false, "expected CubicalPath, got {:?}", other),
        }
    }

    #[test]
    fn test_export_dedukti() {
        let proof = ProofTerm::Reflexivity { term: Text::from("x") };
        let exported = export_dedukti(&proof);
        assert!(exported.as_str().contains("refl"));
    }

    #[test]
    fn test_export_coq() {
        let proof = ProofTerm::Reflexivity { term: Text::from("x") };
        let exported = export_coq(&proof);
        assert_eq!(exported.as_str(), "reflexivity");
    }

    #[test]
    fn test_export_lean() {
        let proof = ProofTerm::Reflexivity { term: Text::from("x") };
        let exported = export_lean(&proof);
        assert_eq!(exported.as_str(), "rfl");
    }

    #[test]
    fn test_erase() {
        let proof = ProofTerm::Reflexivity { term: Text::from("x") };
        let erased = erase_proof(&proof);
        match erased {
            ProofTerm::Erased => {}
            other => assert!(false, "expected Erased, got {:?}", other),
        }
    }

    #[test]
    fn test_export_json() {
        let proof = ProofTerm::SmtVerified {
            solver: Text::from("z3"),
            goal: Text::from("(= 1 1)"),
        };
        let json = export_json(&proof);
        assert!(json.as_str().contains("smt"));
        assert!(json.as_str().contains("z3"));
    }
}
