use verum_source_proof::{PROFILE, SourceReport, SourceRequest, replay};

fn check(source: &str, name: &str) -> SourceReport {
    let request = SourceRequest {
        profile: PROFILE.into(),
        source: source.into(),
        declaration: name.into(),
    };
    replay(&serde_json::to_vec(&request).unwrap())
}

#[test]
fn derived_local_lemmas_replay_the_derivability_argument() {
    let source = include_str!("fixtures/second_incompleteness.vr");
    let report = check(source, "second_incompleteness");
    assert_eq!(report.status, "verified", "{:?}", report.diagnostic);
    assert!(report.native.unwrap().accepted);
    for changed in [
        source.replace("diagonal_in, neg_prov", "diagonal_out, neg_prov"),
        source.replace(
            "conditional, introspection(fixed)",
            "conditional, introspection(falsum)",
        ),
        source.replace(
            "prov_bottom, internal_consistency",
            "prov_bottom, diagonal_in",
        ),
    ] {
        let report = check(&changed, "second_incompleteness");
        assert_eq!(report.status, "rejected", "{:?}", report.diagnostic);
        assert!(report.judgment.is_none());
    }
}

#[test]
fn diagonal_witness_uses_its_own_code_in_both_directions() {
    let source = include_str!("fixtures/diagonal_substitution.vr");
    let name = "diagonal_substitution";
    assert_eq!(check(source, name).status, "verified");
    for changed in [
        source.replace("coding(compose_diag(phi))", "coding(phi)"),
        source.replace("exact(backward(phi,", "exact(forward(phi,"),
        source.replace("out_proof, in_proof", "out_proof, out_proof"),
    ] {
        assert_eq!(check(&changed, name).status, "rejected");
    }
}

#[test]
fn colimit_comparison_uses_factorization_and_uniqueness() {
    let source = include_str!("fixtures/colimit_comparison.vr");
    let name = "colimit_comparison_roundtrip";
    for syntax in ["rewrite <", "rw <"] {
        let checked = check(&source.replace("rewrite <", syntax), name);
        assert_eq!(checked.status, "verified", "{:?}", checked.diagnostic);
    }
    for changed in [
        source.replace("exact(factor_y(x,cx))", "exact(factor_x(y,cy))"),
        source.replace("exact(unique_x(x,identity_x))", "reflexivity"),
    ] {
        assert_eq!(check(&changed, name).status, "rejected");
    }
}

#[test]
fn quotient_descent_requires_generator_agreement_and_checked_local_evidence() {
    let source = include_str!("fixtures/quotient_descent.vr");
    let name = "quotient_descent";
    assert_eq!(check(source, name).status, "verified");
    for changed in [
        source.replace("exact(factors(x))", "reflexivity"),
        source.replace("on_points, q));", "factors, q));"),
        source.replace("exact(factors(x))", "exact(on_points(x))"),
        source.replace("intro(mediator, factors, q);", "intro(mediator, factors);"),
    ] {
        let report = check(&changed, name);
        assert_ne!(report.status, "verified");
        assert!(report.judgment.is_none(), "{:?}", report.diagnostic);
    }
}

#[test]
fn an_identity_section_conflicts_with_the_strict_stage_bound() {
    let source = include_str!("fixtures/bounded_stage.vr");
    let name = "bounded_stage_has_no_section";
    assert_eq!(check(source, name).status, "verified");
    for changed in [
        source.replacen("below(include_at(n,x),n)", "below(n,include_at(n,x))", 1),
        source.replacen("include_at(cutoff,section_at(x)) == x", "x == x", 1),
        source.replace("exact(irreflexive(cutoff,r,impossible))", "exact(impossible)"),
    ] {
        assert_eq!(check(&changed, name).status, "rejected");
    }
}

#[test]
fn constructive_implication_checks_both_propositions_and_their_direction() {
    let source = "theorem t(a:Type, b:Type, x:a): a implies b implies a proof { intro(first, second); exact(first); }";
    assert_eq!(check(source, "t").status, "verified");
    for changed in [
        source.replace("exact(first)", "exact(second)"),
        source.replace(": a implies", ": x implies"),
        source.replace("b implies a proof", "b implies x proof"),
    ] {
        assert_eq!(check(&changed, "t").status, "rejected");
    }
}

#[test]
fn local_facts_are_checked_even_when_unused() {
    let source = "theorem t(a:Type,b:Type,x:a):a proof { have h:a by exact(x); exact(x); }";
    assert_eq!(check(source, "t").status, "verified");
    for source in [
        source.replace("h:a", "h:b"),
        source.replace("exact(x); exact", "exact(h); exact"),
        source.replace("exact(x); }", "exact(x); exact(x); }"),
    ] {
        let report = check(&source, "t");
        assert_eq!(report.status, "rejected", "{:?}", report.diagnostic);
        assert!(report.judgment.is_none());
    }
}

#[test]
fn singleton_colimit_requires_both_collapse_and_cocone_coherence() {
    let source = include_str!("fixtures/collapsing_chain.vr");
    for name in [
        "collapsing_chain_factorization",
        "collapsing_chain_factor_unique",
    ] {
        let result = check(source, name);
        assert_eq!(result.status, "verified", "{:?}", result.diagnostic);
    }
    for changed in [
        source.replacen(
            "advance(n, x) == advance(n, y)",
            "advance(n, x) == advance(n, x)",
            1,
        ),
        source.replacen(
            "leg(n, x) == leg(succ(n), advance(n, x))",
            "leg(n, x) == leg(n, x)",
            1,
        ),
    ] {
        let result = check(&changed, "collapsing_chain_factorization");
        assert_eq!(result.status, "rejected", "{:?}", result.diagnostic);
        assert!(result.judgment.is_none());
    }
    let changed = source.replace("leg(n, x) == candidate", "leg(n, x) == leg(n, x)");
    assert_eq!(
        check(&changed, "collapsing_chain_factor_unique").status,
        "rejected"
    );
}

#[test]
fn grouped_local_justifications_preserve_their_own_sequence() {
    for proof in [
        "(intro(v); exact(v))",
        "{ intro(v); exact(v); }",
        "intro(v), exact(v)",
    ] {
        let source = format!(
            "theorem t(a:Type,x:a):a proof {{ have h:forall y:a. a by {proof}; exact(h(x)); }}"
        );
        let result = check(&source, "t");
        assert_eq!(result.status, "verified", "{:?}", result.diagnostic);
    }
}
