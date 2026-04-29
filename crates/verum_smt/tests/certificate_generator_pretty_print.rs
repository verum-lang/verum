//! Pinpoint test for the wiring of `GeneratorConfig.pretty_print` into
//! the JSON certificate path. The flag was previously an inert config
//! field — set in `Default::default` but never read by `to_json`.
//!
//! Spec: closes 13th instance of the inert-defense anti-pattern series.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::Span;
use verum_smt::certificates::{
    CertificateFormat, CertificateGenerator, GeneratorConfig, Theorem,
};
use verum_smt::proof_term_unified::ProofTerm;

fn dummy_proof() -> ProofTerm {
    let formula = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit { value: 1, suffix: None }),
            Span::dummy(),
        )),
        Span::dummy(),
    );
    ProofTerm::Axiom {
        name: "ax_one_eq_one".into(),
        formula,
    }
}

#[test]
fn json_certificate_pretty_print_inserts_newlines() {
    let proof = dummy_proof();
    let theorem = Theorem::new("t".into(), "1 = 1".into());

    let mut config = GeneratorConfig::default();
    config.pretty_print = true;
    let generator = CertificateGenerator::with_config(CertificateFormat::Json, config);

    let cert = generator.generate(&proof, theorem).unwrap();
    let body = cert.content.as_str();
    assert!(
        body.contains('\n'),
        "pretty_print=true must produce multi-line JSON, got: {body}"
    );
}

#[test]
fn json_certificate_compact_when_pretty_print_disabled() {
    let proof = dummy_proof();
    let theorem = Theorem::new("t".into(), "1 = 1".into());

    let mut config = GeneratorConfig::default();
    config.pretty_print = false;
    let generator = CertificateGenerator::with_config(CertificateFormat::Json, config);

    let cert = generator.generate(&proof, theorem).unwrap();
    let body = cert.content.as_str();
    assert!(
        !body.contains('\n'),
        "pretty_print=false must produce single-line JSON, got: {body}"
    );
}
