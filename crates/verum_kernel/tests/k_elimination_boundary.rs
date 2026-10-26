//! Elimination must justify every branch at the kernel boundary.
use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, ConstructorSig, Context, CoreTerm, InductiveRegistry, RegisteredInductive,
    UniverseLevel, definitional_eq, infer_with_inductives, normalize_with_inductives, verify_full,
    verify_with_inductives,
};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn universe() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}
fn named(name: &str) -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from(name),
        args: List::new(),
    }
}
fn pi(name: &str, domain: CoreTerm, codomain: CoreTerm) -> CoreTerm {
    CoreTerm::Pi {
        binder: Text::from(name),
        domain: Heap::new(domain),
        codomain: Heap::new(codomain),
    }
}
fn app(f: CoreTerm, x: CoreTerm) -> CoreTerm {
    CoreTerm::App(Heap::new(f), Heap::new(x))
}

fn ctor(parent: &str, name: &str, args: List<CoreTerm>) -> CoreTerm {
    CoreTerm::Constructor {
        inductive: Text::from(parent),
        name: Text::from(name),
        params: List::new(),
        args,
    }
}
fn zero() -> CoreTerm {
    ctor("Nat", "Zero", List::new())
}
fn succ(value: CoreTerm) -> CoreTerm {
    ctor("Nat", "Succ", List::from_iter([value]))
}

fn natural_induction() -> (Context, InductiveRegistry, CoreTerm) {
    let nat = named("Nat");
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Nat"),
            List::new(),
            List::from_iter([
                ConstructorSig {
                    name: Text::from("Zero"),
                    arg_types: List::new(),
                },
                ConstructorSig {
                    name: Text::from("Succ"),
                    arg_types: List::from_iter([nat.clone()]),
                },
            ]),
        ))
        .unwrap();
    let context = Context::new()
        .extend(Text::from("Zero"), nat.clone())
        .extend(Text::from("Succ"), pi("value", nat.clone(), nat.clone()))
        .extend(Text::from("P"), pi("subject", nat.clone(), universe()))
        .extend(Text::from("base"), app(var("P"), zero()))
        .extend(
            Text::from("step"),
            pi(
                "predecessor",
                nat.clone(),
                pi(
                    "hypothesis",
                    app(var("P"), var("predecessor")),
                    app(var("P"), succ(var("predecessor"))),
                ),
            ),
        )
        .extend(Text::from("n"), nat);
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(var("n")),
        motive: Heap::new(var("P")),
        cases: List::from_iter([var("base"), var("step")]),
    };
    (context, registry, proof)
}

#[test]
fn an_empty_eliminator_cannot_inhabit_an_arbitrary_goal() {
    let universe = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let context = Context::new()
        .extend(Text::from("A"), universe.clone())
        .extend(Text::from("x"), var("A"))
        .extend(Text::from("Goal"), universe);
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(var("x")),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("unused"),
            domain: Heap::new(var("A")),
            body: Heap::new(var("Goal")),
        }),
        cases: List::new(),
    };
    assert!(verify_full(&context, &proof, &var("Goal"), &AxiomRegistry::new()).is_err());
}

#[test]
fn registered_induction_checks_renamed_recursive_hypotheses() {
    let (context, registry, proof) = natural_induction();
    let actual = infer_with_inductives(&context, &proof, &AxiomRegistry::new(), &registry).unwrap();
    assert!(definitional_eq(&actual, &app(var("P"), var("n"))));
    verify_with_inductives(
        &context,
        &proof,
        &app(var("P"), var("n")),
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
}

#[test]
fn every_declared_branch_is_required_and_checked() {
    let (context, registry, proof) = natural_induction();
    for cases in [
        List::new(),
        List::from_iter([var("base")]),
        List::from_iter([var("base"), var("step"), var("step")]),
        List::from_iter([zero(), var("step")]),
        List::from_iter([var("base"), zero()]),
    ] {
        let mut bad = proof.clone();
        let CoreTerm::Elim {
            cases: branches, ..
        } = &mut bad
        else {
            unreachable!()
        };
        *branches = cases;
        assert!(infer_with_inductives(&context, &bad, &AxiomRegistry::new(), &registry).is_err());
    }
}

#[test]
fn a_recursive_branch_cannot_omit_its_hypothesis() {
    let (context, registry, proof) = natural_induction();
    let context = context.extend(
        Text::from("step"),
        pi(
            "predecessor",
            named("Nat"),
            app(var("P"), succ(var("predecessor"))),
        ),
    );
    assert!(infer_with_inductives(&context, &proof, &AxiomRegistry::new(), &registry).is_err());
}

#[test]
fn genuinely_empty_types_have_valid_empty_elimination() {
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Empty"),
            List::new(),
            List::new(),
        ))
        .unwrap();
    let context = Context::new()
        .extend(Text::from("impossible"), named("Empty"))
        .extend(Text::from("Goal"), universe());
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(var("impossible")),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("ignored"),
            domain: Heap::new(named("Empty")),
            body: Heap::new(var("Goal")),
        }),
        cases: List::new(),
    };
    let actual = infer_with_inductives(&context, &proof, &AxiomRegistry::new(), &registry).unwrap();
    assert!(definitional_eq(&actual, &var("Goal")));
}

#[test]
fn closed_natural_induction_checks_and_computes_without_axioms() {
    let (_, registry, _) = natural_induction();
    let equality = |value: CoreTerm| CoreTerm::PathTy {
        carrier: Heap::new(named("Nat")),
        lhs: Heap::new(value.clone()),
        rhs: Heap::new(value),
    };
    let motive = CoreTerm::Lam {
        binder: Text::from("subject"),
        domain: Heap::new(named("Nat")),
        body: Heap::new(equality(var("subject"))),
    };
    let branch = CoreTerm::Lam {
        binder: Text::from("predecessor"),
        domain: Heap::new(named("Nat")),
        body: Heap::new(CoreTerm::Lam {
            binder: Text::from("ih"),
            domain: Heap::new(equality(var("predecessor"))),
            body: Heap::new(CoreTerm::Refl(Heap::new(succ(var("predecessor"))))),
        }),
    };
    let proof = CoreTerm::Lam {
        binder: Text::from("n"),
        domain: Heap::new(named("Nat")),
        body: Heap::new(CoreTerm::Elim {
            scrutinee: Heap::new(var("n")),
            motive: Heap::new(motive),
            cases: List::from_iter([CoreTerm::Refl(Heap::new(zero())), branch]),
        }),
    };
    let goal = pi("value", named("Nat"), equality(var("value")));
    verify_with_inductives(
        &Context::new(),
        &proof,
        &goal,
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
    let two = succ(succ(zero()));
    let instance = app(proof, two.clone());
    verify_with_inductives(
        &Context::new(),
        &instance,
        &equality(two.clone()),
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
    assert_eq!(
        normalize_with_inductives(&instance, &registry),
        CoreTerm::Refl(Heap::new(two))
    );
}

#[test]
fn constructors_are_owned_by_a_declaration_and_cannot_be_shadowed() {
    let (context, mut registry, proof) = natural_induction();
    registry
        .register(RegisteredInductive::new(
            Text::from("Other"),
            List::new(),
            List::from_iter([ConstructorSig {
                name: Text::from("Zero"),
                arg_types: List::new(),
            }]),
        ))
        .unwrap();
    let context = context
        .extend(Text::from("Zero"), universe())
        .extend(Text::from("Succ"), universe());
    verify_with_inductives(
        &context,
        &proof,
        &app(var("P"), var("n")),
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
    let malformed = [
        ctor("Missing", "Zero", List::new()),
        ctor("Other", "Succ", List::new()),
        ctor("Nat", "Succ", List::new()),
        ctor("Nat", "Zero", List::from_iter([zero()])),
        succ(ctor("Other", "Zero", List::new())),
    ];
    for term in malformed {
        assert!(infer_with_inductives(&context, &term, &AxiomRegistry::new(), &registry).is_err());
    }
    let local = CoreTerm::Elim {
        scrutinee: Heap::new(var("Zero")),
        motive: Heap::new(var("P")),
        cases: List::from_iter([var("base"), var("step")]),
    };
    assert_eq!(normalize_with_inductives(&local, &registry), local);
}

#[test]
fn generic_branches_instantiate_parameters_simultaneously() {
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Product"),
            List::from_iter([Text::from("A"), Text::from("B")]),
            List::from_iter([ConstructorSig {
                name: Text::from("Pair"),
                arg_types: List::from_iter([var("A"), var("B")]),
            }]),
        ))
        .unwrap();
    let product = CoreTerm::Inductive {
        path: Text::from("Product"),
        args: List::from_iter([var("B"), var("A")]),
    };
    let context = Context::new()
        .extend(Text::from("A"), universe())
        .extend(Text::from("B"), universe())
        .extend(Text::from("Goal"), universe())
        .extend(Text::from("answer"), var("Goal"))
        .extend(Text::from("product"), product.clone());
    let branch = CoreTerm::Lam {
        binder: Text::from("first"),
        domain: Heap::new(var("B")),
        body: Heap::new(CoreTerm::Lam {
            binder: Text::from("second"),
            domain: Heap::new(var("A")),
            body: Heap::new(var("answer")),
        }),
    };
    let mut proof = CoreTerm::Elim {
        scrutinee: Heap::new(var("product")),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("p"),
            domain: Heap::new(product),
            body: Heap::new(var("Goal")),
        }),
        cases: List::from_iter([branch]),
    };
    verify_with_inductives(
        &context,
        &proof,
        &var("Goal"),
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
    let CoreTerm::Elim { cases, .. } = &mut proof else {
        unreachable!()
    };
    let CoreTerm::Lam { domain, .. } = &mut cases[0] else {
        unreachable!()
    };
    *domain = Heap::new(var("A"));
    assert!(
        verify_with_inductives(
            &context,
            &proof,
            &var("Goal"),
            &AxiomRegistry::new(),
            &registry
        )
        .is_err()
    );
}

#[test]
fn invalid_parameter_rosters_and_universe_lowering_are_rejected() {
    let mut registry = InductiveRegistry::new();
    assert!(
        registry
            .register(RegisteredInductive::new(
                Text::from("Duplicate"),
                List::from_iter([Text::from("A"), Text::from("A")]),
                List::new()
            ))
            .is_err()
    );
    let constructor = ConstructorSig {
        name: Text::from("Only"),
        arg_types: List::new(),
    };
    assert!(
        registry
            .register(RegisteredInductive::new(
                Text::from("DuplicateCtor"),
                List::new(),
                List::from_iter([constructor.clone(), constructor])
            ))
            .is_err()
    );
    registry
        .register(RegisteredInductive::new(
            Text::from("Box"),
            List::from_iter([Text::from("T")]),
            List::from_iter([ConstructorSig {
                name: Text::from("Value"),
                arg_types: List::from_iter([var("T")]),
            }]),
        ))
        .unwrap();
    for args in [List::new(), List::from_iter([universe()])] {
        let invalid = CoreTerm::Inductive {
            path: Text::from("Box"),
            args,
        };
        assert!(
            infer_with_inductives(&Context::new(), &invalid, &AxiomRegistry::new(), &registry)
                .is_err()
        );
    }
    registry
        .register(RegisteredInductive::new(
            Text::from("Small"),
            List::new(),
            List::from_iter([ConstructorSig {
                name: Text::from("Large"),
                arg_types: List::from_iter([universe()]),
            }]),
        ))
        .unwrap();
    let context = Context::new().extend(Text::from("T"), universe());
    assert!(
        infer_with_inductives(
            &context,
            &ctor("Small", "Large", List::from_iter([var("T")])),
            &AxiomRegistry::new(),
            &registry
        )
        .is_err()
    );
}

#[test]
fn missing_path_coherence_is_not_accepted_as_a_higher_inductive_proof() {
    let mut registry = InductiveRegistry::new();
    registry
        .register(
            RegisteredInductive::new(
                Text::from("Circle"),
                List::new(),
                List::from_iter([ConstructorSig {
                    name: Text::from("Base"),
                    arg_types: List::new(),
                }]),
            )
            .with_path_constructor(verum_kernel::PathCtorSig::one_cell(
                Text::from("Loop"),
                ctor("Circle", "Base", List::new()),
                ctor("Circle", "Base", List::new()),
            )),
        )
        .unwrap();
    let context = Context::new()
        .extend(Text::from("Goal"), universe())
        .extend(Text::from("answer"), var("Goal"));
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(ctor("Circle", "Base", List::new())),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("c"),
            domain: Heap::new(named("Circle")),
            body: Heap::new(var("Goal")),
        }),
        cases: List::from_iter([var("answer"), var("answer")]),
    };
    assert!(
        verify_with_inductives(
            &context,
            &proof,
            &var("Goal"),
            &AxiomRegistry::new(),
            &registry
        )
        .is_err()
    );
}

#[test]
fn parameter_instantiation_does_not_invent_recursive_fields() {
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Box"),
            List::from_iter([Text::from("T")]),
            List::from_iter([ConstructorSig {
                name: Text::from("Value"),
                arg_types: List::from_iter([var("T")]),
            }]),
        ))
        .unwrap();
    let box_of = |ty| CoreTerm::Inductive {
        path: Text::from("Box"),
        args: List::from_iter([ty]),
    };
    let value = |ty, field| CoreTerm::Constructor {
        inductive: Text::from("Box"),
        name: Text::from("Value"),
        params: List::from_iter([ty]),
        args: List::from_iter([field]),
    };
    let inner = box_of(var("T"));
    let outer = box_of(inner.clone());
    let context = Context::new()
        .extend(Text::from("T"), universe())
        .extend(Text::from("element"), var("T"))
        .extend(Text::from("Goal"), universe())
        .extend(Text::from("answer"), var("Goal"));
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(value(inner.clone(), value(var("T"), var("element")))),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("box"),
            domain: Heap::new(outer),
            body: Heap::new(var("Goal")),
        }),
        cases: List::from_iter([CoreTerm::Lam {
            binder: Text::from("field"),
            domain: Heap::new(inner),
            body: Heap::new(var("answer")),
        }]),
    };
    verify_with_inductives(
        &context,
        &proof,
        &var("Goal"),
        &AxiomRegistry::new(),
        &registry,
    )
    .unwrap();
    assert_eq!(normalize_with_inductives(&proof, &registry), var("answer"));
}

#[test]
fn reduction_uses_the_owner_even_when_constructor_names_collide() {
    let (_, mut registry, _) = natural_induction();
    registry
        .register(RegisteredInductive::new(
            Text::from("Other"),
            List::new(),
            List::from_iter([
                ConstructorSig {
                    name: Text::from("First"),
                    arg_types: List::new(),
                },
                ConstructorSig {
                    name: Text::from("Zero"),
                    arg_types: List::new(),
                },
            ]),
        ))
        .unwrap();
    let proof = CoreTerm::Elim {
        scrutinee: Heap::new(ctor("Other", "Zero", List::new())),
        motive: Heap::new(var("P")),
        cases: List::from_iter([var("first"), var("second")]),
    };
    assert_eq!(normalize_with_inductives(&proof, &registry), var("second"));
}

#[test]
fn malformed_constructor_spines_stay_neutral_without_fabricated_arguments() {
    let (_, registry, _) = natural_induction();
    for bad in [
        ctor("Nat", "Succ", List::new()),
        ctor("Nat", "Zero", List::from_iter([zero()])),
    ] {
        let proof = CoreTerm::Elim {
            scrutinee: Heap::new(bad),
            motive: Heap::new(var("P")),
            cases: List::from_iter([var("base"), var("step")]),
        };
        assert_eq!(normalize_with_inductives(&proof, &registry), proof);
    }
}

#[test]
fn registry_free_trace_does_not_claim_constructor_replay() {
    assert!(
        verum_kernel::record_inference(&Context::new(), &zero(), &AxiomRegistry::new()).is_none()
    );
}

#[test]
fn transparent_definitions_cannot_hide_negative_recursion() {
    let framework = verum_kernel::FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    let mut axioms = AxiomRegistry::new();
    axioms
        .register_definition(
            Text::from("HiddenNegative"),
            pi("bad", named("Bad"), named("Result")),
            universe(),
            framework.clone(),
        )
        .unwrap();
    let alias = CoreTerm::Axiom {
        name: Text::from("HiddenNegative"),
        ty: Heap::new(universe()),
        framework,
    };
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Bad"),
            List::new(),
            List::from_iter([ConstructorSig {
                name: Text::from("Wrap"),
                arg_types: List::from_iter([alias.clone()]),
            }]),
        ))
        .unwrap();
    let context = Context::new().extend(Text::from("function"), alias);
    let proof = ctor("Bad", "Wrap", List::from_iter([var("function")]));
    assert!(matches!(
        infer_with_inductives(&context, &proof, &axioms, &registry),
        Err(verum_kernel::KernelError::PositivityViolation { .. })
    ));
}
