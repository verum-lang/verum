use verum_proof::proof_checker::{Certificate, DataField, Level, Term};

pub fn nat() -> Term {
    Term::Data {
        name: "Nat".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![], vec![DataField::Recursive]],
    }
}
pub fn ctor(data: Term, constructor: usize, args: Vec<Term>) -> Term {
    Term::Construct {
        data: Box::new(data),
        constructor,
        args,
    }
}
pub fn zero() -> Term {
    ctor(nat(), 0, vec![])
}
pub fn succ(n: Term) -> Term {
    ctor(nat(), 1, vec![n])
}
pub fn list(element: Term) -> Term {
    Term::Data {
        name: "List".into(),
        level: Level::Concrete(0),
        constructors: vec![
            vec![],
            vec![DataField::Value(Box::new(element)), DataField::Recursive],
        ],
    }
}
pub fn induct(value: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Induct {
        scrutinee: Box::new(value),
        motive: Box::new(motive),
        cases,
    }
}
pub fn certificate(term: Term, claimed_type: Term) -> Certificate {
    Certificate {
        term,
        claimed_type,
        metadata: Default::default(),
    }
}
pub fn copy() -> Term {
    Term::lam(
        nat(),
        induct(
            Term::var(0),
            Term::lam(nat(), nat()),
            vec![
                zero(),
                Term::lam(nat(), Term::lam(nat(), succ(Term::var(0)))),
            ],
        ),
    )
}
pub fn copy_identity() -> Certificate {
    let motive = Term::lam(
        nat(),
        Term::id_ty(nat(), Term::app(copy(), Term::var(0)), Term::var(0)),
    );
    let proof_step = Term::lam(
        nat(),
        Term::lam(
            Term::id_ty(nat(), Term::app(copy(), Term::var(0)), Term::var(0)),
            Term::j(
                Term::lam(
                    nat(),
                    Term::id_ty(
                        nat(),
                        succ(Term::app(copy(), Term::var(2))),
                        succ(Term::var(0)),
                    ),
                ),
                Term::refl(succ(Term::app(copy(), Term::var(1)))),
                Term::var(0),
            ),
        ),
    );
    certificate(
        Term::lam(
            nat(),
            induct(Term::var(0), motive, vec![Term::refl(zero()), proof_step]),
        ),
        Term::pi(
            nat(),
            Term::id_ty(nat(), Term::app(copy(), Term::var(0)), Term::var(0)),
        ),
    )
}
pub fn length() -> Term {
    Term::lam(
        Term::universe(0),
        Term::lam(
            list(Term::var(0)),
            induct(
                Term::var(0),
                Term::lam(list(Term::var(1)), nat()),
                vec![
                    zero(),
                    Term::lam(
                        Term::var(1),
                        Term::lam(list(Term::var(2)), Term::lam(nat(), succ(Term::var(0)))),
                    ),
                ],
            ),
        ),
    )
}
pub fn outcome() -> Term {
    Term::Data {
        name: "Outcome".into(),
        level: Level::Concrete(0),
        constructors: vec![
            vec![DataField::Value(Box::new(nat()))],
            vec![
                DataField::Value(Box::new(nat())),
                DataField::Value(Box::new(nat())),
            ],
            vec![DataField::Value(Box::new(nat()))],
        ],
    }
}
pub fn outcome_payload() -> Term {
    Term::lam(
        outcome(),
        induct(
            Term::var(0),
            Term::lam(outcome(), nat()),
            vec![
                Term::lam(nat(), Term::var(0)),
                Term::lam(nat(), Term::lam(nat(), Term::var(0))),
                Term::lam(nat(), Term::var(0)),
            ],
        ),
    )
}
pub fn battery() -> Vec<(&'static str, Certificate, bool)> {
    let mut rows = vec![
        ("nat-constructor", certificate(succ(zero()), nat()), true),
        ("general-copy-identity", copy_identity(), true),
        (
            "polymorphic-length",
            certificate(
                length(),
                Term::pi(Term::universe(0), Term::pi(list(Term::var(0)), nat())),
            ),
            true,
        ),
    ];
    let two = ctor(
        list(nat()),
        1,
        vec![
            zero(),
            ctor(
                list(nat()),
                1,
                vec![succ(zero()), ctor(list(nat()), 0, vec![])],
            ),
        ],
    );
    let computed_length = Term::app(Term::app(length(), nat()), two);
    rows.push((
        "length-computation",
        certificate(
            Term::refl(computed_length.clone()),
            Term::id_ty(nat(), computed_length, succ(succ(zero()))),
        ),
        true,
    ));
    for (name, tag, args, payload) in [
        ("continue-payload", 0, vec![zero()], zero()),
        ("stop-payload", 1, vec![zero(), succ(zero())], succ(zero())),
        (
            "refuse-payload",
            2,
            vec![succ(succ(zero()))],
            succ(succ(zero())),
        ),
    ] {
        let computed = Term::app(outcome_payload(), ctor(outcome(), tag, args));
        rows.push((
            name,
            certificate(
                Term::refl(computed.clone()),
                Term::id_ty(nat(), computed, payload),
            ),
            true,
        ));
    }
    let tree = Term::Data {
        name: "Tree".into(),
        level: Level::Concrete(0),
        constructors: vec![
            vec![DataField::Value(Box::new(nat()))],
            vec![DataField::Recursive, DataField::Recursive],
        ],
    };
    let leaf = |value| ctor(tree.clone(), 0, vec![value]);
    let value = ctor(tree.clone(), 1, vec![leaf(zero()), leaf(succ(zero()))]);
    let fold = induct(
        value,
        Term::lam(tree.clone(), nat()),
        vec![
            Term::lam(nat(), Term::var(0)),
            Term::lam(
                tree.clone(),
                Term::lam(nat(), Term::lam(tree, Term::lam(nat(), Term::var(0)))),
            ),
        ],
    );
    rows.push((
        "multiple-recursive-hypotheses",
        certificate(
            Term::refl(fold.clone()),
            Term::id_ty(nat(), fold, succ(zero())),
        ),
        true,
    ));
    let local_body = induct(
        zero(),
        Term::lam(
            nat(),
            Term::id_ty(nat(), Term::app(Term::var(1), Term::var(0)), zero()),
        ),
        vec![
            Term::refl(zero()),
            Term::lam(
                nat(),
                Term::lam(
                    Term::id_ty(nat(), Term::app(Term::var(1), Term::var(0)), zero()),
                    Term::refl(zero()),
                ),
            ),
        ],
    );
    let local = Term::Let {
        value: Box::new(Term::lam(nat(), zero())),
        body: Box::new(local_body.clone()),
    };
    rows.push((
        "transparent-definition-in-induction",
        certificate(local, Term::id_ty(nat(), zero(), zero())),
        true,
    ));
    rows.push((
        "assumed-function-does-not-unfold",
        certificate(
            Term::lam(Term::pi(nat(), nat()), local_body),
            Term::pi(
                Term::pi(nat(), nat()),
                Term::id_ty(nat(), Term::app(Term::var(0), zero()), zero()),
            ),
        ),
        false,
    ));
    let local_capture = Term::lam(
        Term::universe(0),
        Term::lam(
            Term::var(0),
            Term::Let {
                value: Box::new(Term::var(0)),
                body: Box::new(Term::lam(Term::var(2), Term::var(1))),
            },
        ),
    );
    rows.push((
        "let-binder-capture",
        certificate(
            local_capture,
            Term::pi(
                Term::universe(0),
                Term::pi(Term::var(0), Term::pi(Term::var(1), Term::var(2))),
            ),
        ),
        true,
    ));
    rows.push((
        "unused-invalid-let-value",
        certificate(
            Term::Let {
                value: Box::new(Term::var(99)),
                body: Box::new(zero()),
            },
            nat(),
        ),
        false,
    ));
    rows.push((
        "let-does-not-demote-universe",
        certificate(
            Term::Let {
                value: Box::new(Term::universe(0)),
                body: Box::new(Term::var(0)),
            },
            Term::universe(0),
        ),
        false,
    ));
    let mut changed_owner = nat();
    if let Term::Data { constructors, .. } = &mut changed_owner {
        constructors[1] = vec![];
    }
    rows.push((
        "owner-shape-forgery",
        certificate(ctor(changed_owner, 1, vec![]), nat()),
        false,
    ));
    rows.push((
        "missing-recursive-field",
        certificate(ctor(nat(), 1, vec![]), nat()),
        false,
    ));
    rows.push((
        "unknown-constructor",
        certificate(ctor(nat(), 4, vec![]), nat()),
        false,
    ));
    rows.push((
        "wrong-field-type",
        certificate(ctor(nat(), 1, vec![Term::universe(0)]), nat()),
        false,
    ));
    let bad_data = Term::Data {
        name: "Bad".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![DataField::Value(Box::new(Term::universe(0)))]],
    };
    rows.push((
        "universe-demotion",
        certificate(bad_data, Term::universe(0)),
        false,
    ));
    let open = Term::Data {
        name: "Open".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![DataField::Value(Box::new(Term::var(0)))]],
    };
    rows.push((
        "unbound-field-dependency",
        certificate(open, Term::universe(0)),
        false,
    ));
    rows.push((
        "missing-induction-case",
        certificate(induct(zero(), Term::lam(nat(), nat()), vec![zero()]), nat()),
        false,
    ));
    rows.push((
        "missing-induction-hypothesis",
        certificate(
            induct(
                zero(),
                Term::lam(nat(), nat()),
                vec![zero(), Term::lam(nat(), zero())],
            ),
            nat(),
        ),
        false,
    ));
    rows.push((
        "unrelated-induction-case",
        certificate(
            induct(
                zero(),
                Term::lam(nat(), nat()),
                vec![
                    Term::universe(0),
                    Term::lam(nat(), Term::lam(nat(), zero())),
                ],
            ),
            nat(),
        ),
        false,
    ));
    let wrong_motive = Term::lam(Term::universe(0), nat());
    rows.push((
        "wrong-motive-domain",
        certificate(
            induct(
                zero(),
                wrong_motive,
                vec![zero(), Term::lam(nat(), Term::lam(nat(), zero()))],
            ),
            nat(),
        ),
        false,
    ));
    let changed_result = Term::app(outcome_payload(), ctor(outcome(), 2, vec![succ(zero())]));
    rows.push((
        "changed-refusal-payload",
        certificate(
            Term::refl(changed_result.clone()),
            Term::id_ty(nat(), changed_result, zero()),
        ),
        false,
    ));
    let mut wrong_claim = copy_identity();
    wrong_claim.claimed_type = Term::pi(
        nat(),
        Term::id_ty(nat(), Term::app(copy(), Term::var(0)), zero()),
    );
    rows.push(("forged-general-theorem", wrong_claim, false));
    rows.extend(dependent_fields_battery());
    rows
}

fn dependent_fields_battery() -> Vec<(&'static str, Certificate, bool)> {
    let package = Term::Data {
        name: "Package".into(),
        level: Level::Concrete(1),
        constructors: vec![vec![
            DataField::Value(Box::new(Term::universe(0))),
            DataField::Dependent(Box::new(Term::var(0))),
            DataField::Dependent(Box::new(Term::id_ty(
                Term::var(1),
                Term::var(0),
                Term::var(0),
            ))),
        ]],
    };
    let packed = ctor(package.clone(), 0, vec![nat(), zero(), Term::refl(zero())]);
    let project = Term::lam(
        package.clone(),
        induct(
            Term::var(0),
            Term::lam(package.clone(), Term::universe(0)),
            vec![Term::lam(
                Term::universe(0),
                Term::lam(
                    Term::var(0),
                    Term::lam(
                        Term::id_ty(Term::var(1), Term::var(0), Term::var(0)),
                        Term::var(2),
                    ),
                ),
            )],
        ),
    );
    let tree = Term::Data {
        name: "EvidenceTree".into(),
        level: Level::Concrete(1),
        constructors: vec![
            vec![],
            vec![
                DataField::Recursive,
                DataField::Value(Box::new(Term::universe(0))),
                DataField::Dependent(Box::new(Term::var(0))),
                DataField::Dependent(Box::new(Term::id_ty(
                    Term::var(1),
                    Term::var(0),
                    Term::var(0),
                ))),
            ],
        ],
    };
    let leaf = ctor(tree.clone(), 0, vec![]);
    let tree_value = ctor(
        tree.clone(),
        1,
        vec![leaf, nat(), zero(), Term::refl(zero())],
    );
    let size = Term::lam(
        tree.clone(),
        induct(
            Term::var(0),
            Term::lam(tree.clone(), nat()),
            vec![
                zero(),
                Term::lam(
                    tree.clone(),
                    Term::lam(
                        nat(),
                        Term::lam(
                            Term::universe(0),
                            Term::lam(
                                Term::var(0),
                                Term::lam(
                                    Term::id_ty(Term::var(1), Term::var(0), Term::var(0)),
                                    succ(Term::var(3)),
                                ),
                            ),
                        ),
                    ),
                ),
            ],
        ),
    );
    let reduced = Term::app(size.clone(), tree_value);
    let mut rows = vec![
        (
            "dependent-telescope-formation",
            certificate(package.clone(), Term::universe(1)),
            true,
        ),
        (
            "dependent-telescope-construction",
            certificate(packed.clone(), package.clone()),
            true,
        ),
        (
            "dependent-telescope-general-elimination",
            certificate(
                project.clone(),
                Term::pi(package.clone(), Term::universe(0)),
            ),
            true,
        ),
        (
            "dependent-telescope-computation",
            certificate(
                Term::refl(Term::app(project.clone(), packed.clone())),
                Term::id_ty(Term::universe(0), Term::app(project, packed), nat()),
            ),
            true,
        ),
        (
            "dependent-field-after-recursion",
            certificate(size, Term::pi(tree, nat())),
            true,
        ),
        (
            "dependent-recursive-computation",
            certificate(
                Term::refl(reduced.clone()),
                Term::id_ty(nat(), reduced, succ(zero())),
            ),
            true,
        ),
        (
            "dependent-field-wrong-argument",
            certificate(
                ctor(
                    package.clone(),
                    0,
                    vec![nat(), Term::universe(0), Term::refl(zero())],
                ),
                package.clone(),
            ),
            false,
        ),
        (
            "dependent-field-wrong-evidence",
            certificate(
                ctor(
                    package.clone(),
                    0,
                    vec![nat(), zero(), Term::refl(succ(zero()))],
                ),
                package.clone(),
            ),
            false,
        ),
    ];
    for (name, index) in [
        ("dependent-field-self-reference", 1),
        ("dependent-field-forward-reference", 2),
    ] {
        let malformed = Term::Data {
            name: "Malformed".into(),
            level: Level::Concrete(1),
            constructors: vec![vec![
                DataField::Value(Box::new(Term::universe(0))),
                DataField::Dependent(Box::new(Term::var(index))),
            ]],
        };
        rows.push((name, certificate(malformed, Term::universe(1)), false));
    }
    for (name, fields) in [
        (
            "dependent-field-not-a-type",
            vec![
                DataField::Value(Box::new(nat())),
                DataField::Dependent(Box::new(Term::var(0))),
            ],
        ),
        (
            "dependent-field-universe-demotion",
            vec![DataField::Dependent(Box::new(Term::universe(0)))],
        ),
    ] {
        rows.push((
            name,
            certificate(
                Term::Data {
                    name: "InvalidField".into(),
                    level: Level::Concrete(0),
                    constructors: vec![fields],
                },
                Term::universe(0),
            ),
            false,
        ));
    }
    let captured = Term::Data {
        name: "CapturedEvidence".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![
            DataField::Value(Box::new(Term::var(1))),
            DataField::Dependent(Box::new(Term::id_ty(
                Term::var(2),
                Term::var(0),
                Term::var(1),
            ))),
        ]],
    };
    rows.push((
        "dependent-field-captured-parameters",
        certificate(
            Term::lam(
                Term::universe(0),
                Term::lam(
                    Term::var(0),
                    ctor(
                        captured.clone(),
                        0,
                        vec![Term::var(0), Term::refl(Term::var(0))],
                    ),
                ),
            ),
            Term::pi(Term::universe(0), Term::pi(Term::var(0), captured.clone())),
        ),
        true,
    ));
    let concrete = Term::Data {
        name: "CapturedEvidence".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![
            DataField::Value(Box::new(nat())),
            DataField::Dependent(Box::new(Term::id_ty(nat(), Term::var(0), zero()))),
        ]],
    };
    let abstraction = Term::lam(
        Term::universe(0),
        Term::lam(
            Term::var(0),
            ctor(captured, 0, vec![Term::var(0), Term::refl(Term::var(0))]),
        ),
    );
    rows.push((
        "dependent-field-beta-preserves-capture",
        certificate(Term::app(Term::app(abstraction, nat()), zero()), concrete),
        true,
    ));
    rows
}
