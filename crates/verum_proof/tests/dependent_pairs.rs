use verum_proof::proof_checker::{Certificate, DataField, Level, Term};

fn witness_type() -> Term {
    // A : U, x : A |- Sigma(y : A). Id(A, x, y).
    Term::sigma(
        Term::var(1),
        Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
    )
}
fn witness() -> Term {
    Term::pair(Term::var(0), Term::refl(Term::var(0)))
}
fn close(term: Term, claimed: Term) -> Certificate {
    Certificate {
        term: Term::lam(Term::universe(0), Term::lam(Term::var(0), term)),
        claimed_type: Term::pi(Term::universe(0), Term::pi(Term::var(0), claimed)),
        metadata: Default::default(),
    }
}
fn battery() -> Vec<(&'static str, Certificate, bool)> {
    let bit = Term::Data {
        name: "Bit".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![], vec![]].into_iter().map(Into::into).collect(),
    };
    let zero = Term::Construct {
        data: Box::new(bit.clone()),
        constructor: 0,
        args: vec![],
    };
    let holder = Term::Data {
        name: "Evidence".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![DataField::Value(Box::new(witness_type()))]]
            .into_iter()
            .map(Into::into)
            .collect(),
    };
    let lifted = Term::sigma(
        Term::var(2),
        Term::id_ty(Term::var(3), Term::var(2), Term::var(0)),
    );
    let mut cases = vec![
        ("dependent_pair", close(witness(), witness_type()), true),
        (
            "let_body",
            close(
                Term::Let {
                    value: Box::new(Term::var(0)),
                    body: Box::new(witness()),
                },
                witness_type(),
            ),
            true,
        ),
        (
            "application_argument",
            close(
                Term::app(
                    Term::lam(witness_type(), Term::fst(Term::var(0))),
                    witness(),
                ),
                Term::var(1),
            ),
            true,
        ),
        (
            "constructor_field",
            close(
                Term::Construct {
                    data: Box::new(holder.clone()),
                    constructor: 0,
                    args: vec![witness()],
                },
                holder,
            ),
            true,
        ),
        (
            "induction_branch",
            close(
                Term::Induct {
                    scrutinee: Box::new(zero),
                    motive: Box::new(Term::lam(bit, lifted.clone())),
                    cases: vec![witness(), witness()],
                },
                witness_type(),
            ),
            true,
        ),
        (
            "identity_elimination_base",
            close(
                Term::j(
                    Term::lam(
                        Term::var(1),
                        Term::sigma(
                            Term::var(2),
                            Term::id_ty(Term::var(3), Term::var(1), Term::var(0)),
                        ),
                    ),
                    witness(),
                    Term::refl(Term::var(0)),
                ),
                witness_type(),
            ),
            true,
        ),
        (
            "reflexive_dependent_pair",
            close(
                Term::refl(witness()),
                Term::id_ty(witness_type(), witness(), witness()),
            ),
            true,
        ),
        (
            "malformed_family",
            close(witness(), Term::sigma(Term::var(1), Term::var(0))),
            false,
        ),
    ];
    let defined = |value: Term, body: Term| Term::Let {
        value: Box::new(value),
        body: Box::new(body),
    };
    cases.extend([
        (
            "transparent_value_retains_captured_binder",
            close(
                defined(
                    Term::var(0),
                    Term::lam(Term::var(2), Term::refl(Term::var(1))),
                ),
                Term::pi(
                    Term::var(1),
                    Term::id_ty(Term::var(2), Term::var(1), Term::var(1)),
                ),
            ),
            true,
        ),
        (
            "transparent_value_cannot_capture_later_binder",
            close(
                defined(
                    Term::var(0),
                    Term::lam(Term::var(2), Term::refl(Term::var(1))),
                ),
                Term::pi(
                    Term::var(1),
                    Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
                ),
            ),
            false,
        ),
        (
            "transparent_pair",
            close(defined(witness(), Term::var(0)), witness_type()),
            true,
        ),
        (
            "transparent_alias_chain",
            close(
                defined(witness(), defined(Term::var(0), Term::var(0))),
                witness_type(),
            ),
            true,
        ),
        (
            "transparent_under_binder",
            close(
                defined(witness(), Term::lam(Term::var(2), Term::var(1))),
                Term::pi(Term::var(1), lifted.clone()),
            ),
            true,
        ),
        (
            "transparent_argument",
            close(
                defined(
                    witness(),
                    Term::app(Term::lam(lifted, Term::fst(Term::var(0))), Term::var(0)),
                ),
                Term::var(1),
            ),
            true,
        ),
        // Normalizing a definition before checking would erase this application
        // and incorrectly refine its explicitly nondependent result type.
        (
            "application_is_not_transparent_alias",
            close(
                defined(
                    Term::app(
                        Term::lam(
                            Term::sigma(
                                Term::var(1),
                                Term::id_ty(Term::var(2), Term::var(1), Term::var(1)),
                            ),
                            Term::var(0),
                        ),
                        witness(),
                    ),
                    Term::var(0),
                ),
                witness_type(),
            ),
            false,
        ),
        // A discarded definition still must be checked before any evaluation.
        (
            "ill_typed_unused_definition",
            close(
                defined(
                    Term::app(Term::lam(Term::var(1), Term::var(0)), Term::universe(0)),
                    Term::pair(Term::var(1), Term::refl(Term::var(1))),
                ),
                witness_type(),
            ),
            false,
        ),
        // Assumptions retain their stated types; only local definitions unfold.
        (
            "nondependent_assumption",
            close(
                Term::lam(
                    Term::sigma(
                        Term::var(1),
                        Term::id_ty(Term::var(2), Term::var(1), Term::var(1)),
                    ),
                    Term::var(0),
                ),
                Term::pi(
                    Term::sigma(
                        Term::var(1),
                        Term::id_ty(Term::var(2), Term::var(1), Term::var(1)),
                    ),
                    Term::sigma(
                        Term::var(2),
                        Term::id_ty(Term::var(3), Term::var(2), Term::var(0)),
                    ),
                ),
            ),
            false,
        ),
    ]);
    let mut wrong_annotation = close(witness(), witness_type());
    if let Term::Lam(_, inner) = &mut wrong_annotation.term {
        if let Term::Lam(domain, _) = inner.as_mut() {
            *domain = Box::new(Term::universe(0));
        }
    }
    cases.push(("wrong_domain_annotation", wrong_annotation, false));
    // A, x:A, y:A |- (y,refl x) cannot prove Id(A,x,y) for arbitrary x,y.
    cases.push((
        "substituted_witness",
        Certificate {
            term: Term::lam(
                Term::universe(0),
                Term::lam(
                    Term::var(0),
                    Term::lam(
                        Term::var(1),
                        Term::pair(Term::var(0), Term::refl(Term::var(1))),
                    ),
                ),
            ),
            claimed_type: Term::pi(
                Term::universe(0),
                Term::pi(
                    Term::var(0),
                    Term::pi(
                        Term::var(1),
                        Term::sigma(
                            Term::var(2),
                            Term::id_ty(Term::var(3), Term::var(2), Term::var(0)),
                        ),
                    ),
                ),
            ),
            metadata: Default::default(),
        },
        false,
    ));
    cases.push((
        "literal_universe_ceiling",
        Certificate {
            term: Term::universe(u32::MAX - 1),
            claimed_type: Term::universe(u32::MAX),
            metadata: Default::default(),
        },
        true,
    ));
    cases.push((
        "overflowing_term",
        Certificate {
            term: Term::universe(u32::MAX),
            claimed_type: Term::universe(0),
            metadata: Default::default(),
        },
        false,
    ));
    // A : U, B : A -> U, p : Sigma(A,B). Eta is dependent and
    // symmetric; changing a component is not eta conversion.
    let family = Term::pi(Term::var(0), Term::universe(0));
    let product = Term::sigma(Term::var(1), Term::app(Term::var(1), Term::var(0)));
    let product_under_p = Term::sigma(Term::var(2), Term::app(Term::var(2), Term::var(0)));
    let rebuilt = Term::pair(Term::fst(Term::var(0)), Term::snd(Term::var(0)));
    for (name, left, right) in [
        ("dependent_product_eta", rebuilt.clone(), Term::var(0)),
        ("dependent_product_eta_reverse", Term::var(0), rebuilt),
    ] {
        cases.push((
            name,
            Certificate {
                term: Term::lam(
                    Term::universe(0),
                    Term::lam(
                        family.clone(),
                        Term::lam(product.clone(), Term::refl(Term::var(0))),
                    ),
                ),
                claimed_type: Term::pi(
                    Term::universe(0),
                    Term::pi(
                        family.clone(),
                        Term::pi(
                            product.clone(),
                            Term::id_ty(product_under_p.clone(), left, right),
                        ),
                    ),
                ),
                metadata: Default::default(),
            },
            true,
        ));
    }
    // For a nondependent pair of arbitrary A values, swapping projections
    // and dropping the second projection must both be rejected.
    for (name, rebuilt) in [
        (
            "product_eta_cannot_swap",
            Term::pair(Term::snd(Term::var(0)), Term::fst(Term::var(0))),
        ),
        (
            "product_eta_cannot_duplicate",
            Term::pair(Term::fst(Term::var(0)), Term::fst(Term::var(0))),
        ),
    ] {
        let product = Term::sigma(Term::var(0), Term::var(1));
        let product_under_p = Term::sigma(Term::var(1), Term::var(2));
        cases.push((
            name,
            Certificate {
                term: Term::lam(
                    Term::universe(0),
                    Term::lam(product.clone(), Term::refl(Term::var(0))),
                ),
                claimed_type: Term::pi(
                    Term::universe(0),
                    Term::pi(product, Term::id_ty(product_under_p, rebuilt, Term::var(0))),
                ),
                metadata: Default::default(),
            },
            false,
        ));
    }
    cases
}

#[test]
fn expected_families_reach_every_introduction_boundary() {
    for (name, cert, expected) in battery() {
        let substitution = cert.verify();
        assert_eq!(
            substitution.is_ok(),
            expected,
            "substitution {name}: {substitution:?}"
        );
        let nbe = verum_proof::proof_checker_nbe::verify_certificate(&cert);
        assert_eq!(nbe.is_ok(), expected, "NbE {name}: {nbe:?}");
    }
}

#[test]
fn public_check_does_not_admit_malformed_expected_families() {
    let context = verum_proof::proof_checker::Context::new()
        .extend(Term::universe(0))
        .extend(Term::var(0));
    let malformed = Term::sigma(Term::var(1), Term::var(0));
    assert!(verum_proof::proof_checker::check(&context, &witness(), &malformed).is_err());
    let nbe_context = verum_proof::proof_checker_nbe::NbeContext::new().extend(
        verum_proof::proof_checker_nbe::Value::VUniverse(Level::Concrete(0)),
    );
    let a = verum_proof::proof_checker_nbe::eval(&Term::var(0), nbe_context.env());
    let nbe_context = nbe_context.extend(a);
    assert!(verum_proof::proof_checker_nbe::check(&nbe_context, &witness(), &malformed).is_err());
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the built independent checker"]
fn independent_lean_replays_dependent_pairs_and_causal_rejections() {
    let replay =
        std::env::var_os("VERUM_LEAN_REPLAY").expect("build the independent checker first");
    let cases = battery();
    let rows: Vec<_> = cases
        .iter()
        .map(|(id, cert, _)| {
            serde_json::json!({
                "id": id, "term": cert.term, "claimed_type": cert.claimed_type,
            })
        })
        .collect();
    let directory =
        std::env::temp_dir().join(format!("verum-dependent-pairs-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("battery.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(replay)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdicts: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = verdicts["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (verdict, (name, _, expected)) in verdicts.iter().zip(cases) {
        assert_eq!(verdict["id"], name);
        assert_eq!(verdict["ok"], expected, "{name}: {verdict}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}
