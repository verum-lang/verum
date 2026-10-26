use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, ConstructorSig, CoreTerm, FrameworkId, InductiveRegistry, RegisteredInductive,
    UniverseLevel,
};

fn universe(level: u32) -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(level))
}

fn framework() -> FrameworkId {
    FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("explicit test environment"),
    }
}

fn reference(name: &str, ty: CoreTerm) -> CoreTerm {
    CoreTerm::Axiom {
        name: Text::from(name),
        ty: Heap::new(ty),
        framework: framework(),
    }
}

#[test]
fn definitions_check_bodies_before_admission() {
    let mut registry = AxiomRegistry::new();
    assert!(
        registry
            .register_definition(Text::from("bad"), universe(0), universe(0), framework())
            .is_err()
    );
    assert!(registry.get("bad").is_none());
    registry
        .register_definition(Text::from("good"), universe(0), universe(1), framework())
        .unwrap();
    registry
        .register_definition(
            Text::from("alias"),
            reference("good", universe(1)),
            universe(1),
            framework(),
        )
        .unwrap();
    assert_eq!(
        verum_kernel::normalize_with_axioms(&reference("alias", universe(1)), &registry),
        universe(0)
    );
}

#[test]
fn definitions_reject_open_forward_and_recursive_bodies() {
    for (name, body) in [
        ("open", CoreTerm::Var(Text::from("free"))),
        ("forward", reference("later", universe(1))),
        ("self", reference("self", universe(1))),
    ] {
        let mut registry = AxiomRegistry::new();
        assert!(
            registry
                .register_definition(Text::from(name), body, universe(1), framework())
                .is_err()
        );
        assert!(registry.all().is_empty());
    }
}

#[test]
fn definitions_require_a_well_formed_declared_type() {
    let mut registry = AxiomRegistry::new();
    let bogus_type = CoreTerm::Lam {
        binder: Text::from("A"),
        domain: Heap::new(universe(1)),
        body: Heap::new(CoreTerm::Var(Text::from("A"))),
    };
    assert!(
        registry
            .register_definition(Text::from("bad"), universe(0), bogus_type, framework())
            .is_err()
    );
    assert!(registry.all().is_empty());
}

fn named(name: &str) -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from(name),
        args: List::new(),
    }
}

fn declaration(name: &str, fields: List<CoreTerm>) -> RegisteredInductive {
    RegisteredInductive::new(
        Text::from(name),
        List::new(),
        List::from_iter([ConstructorSig {
            name: Text::from(format!("Make{name}")),
            arg_types: fields,
        }]),
    )
}

#[test]
fn declaration_group_checks_mutual_positivity_and_is_atomic() {
    let mut registry = InductiveRegistry::new();
    let negative = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(named("Left")),
        codomain: Heap::new(named("Left")),
    };
    assert!(
        registry
            .register_group(
                List::from_iter([
                    declaration("Left", List::from_iter([named("Right")])),
                    declaration("Right", List::from_iter([negative])),
                ]),
                &AxiomRegistry::new()
            )
            .is_err()
    );
    assert!(registry.all().is_empty());
    registry
        .register_group(
            List::from_iter([
                declaration("Left", List::from_iter([named("Right")])),
                declaration("Right", List::from_iter([named("Left")])),
            ]),
            &AxiomRegistry::new(),
        )
        .unwrap();
    assert_eq!(registry.all().len(), 2);
}

#[test]
fn declaration_admission_rejects_unknown_dependencies_and_universe_drift() {
    let mut registry = InductiveRegistry::new();
    assert!(
        registry
            .register(declaration("Forward", List::from_iter([named("Missing")])))
            .is_err()
    );
    assert!(
        registry
            .register(declaration("TooSmall", List::from_iter([universe(0)])))
            .is_err()
    );
    assert!(registry.all().is_empty());
    registry
        .register(
            declaration("LargeEnough", List::from_iter([universe(0)]))
                .with_universe(UniverseLevel::Concrete(1)),
        )
        .unwrap();
}

#[test]
fn declarations_support_checked_generic_container_dependencies() {
    let parameter = CoreTerm::Var(Text::from("A"));
    let mut registry = InductiveRegistry::new();
    registry
        .register(RegisteredInductive::new(
            Text::from("Box"),
            List::from_iter([Text::from("A")]),
            List::from_iter([ConstructorSig {
                name: Text::from("Boxed"),
                arg_types: List::from_iter([parameter]),
            }]),
        ))
        .unwrap();
    registry
        .register(declaration(
            "Tree",
            List::from_iter([CoreTerm::Inductive {
                path: Text::from("Box"),
                args: List::from_iter([named("Tree")]),
            }]),
        ))
        .unwrap();
}

#[test]
fn unknown_heads_do_not_fabricate_a_universe() {
    let context = verum_kernel::Context::new();
    let axioms = AxiomRegistry::new();
    assert!(verum_kernel::infer(&context, &named("Missing"), &axioms).is_err());
    assert!(
        verum_kernel::infer_with_inductives(
            &context,
            &named("Missing"),
            &axioms,
            &InductiveRegistry::new()
        )
        .is_err()
    );
}

#[test]
fn coordinate_policy_survives_recursive_inference() {
    use verum_kernel::{Context, KernelCoord, KernelError, OrdinalDepth};
    let high = KernelCoord::canonical(Text::from("high"), OrdinalDepth::finite(2));
    let low = KernelCoord::canonical(Text::from("low"), OrdinalDepth::finite(0));
    let mut axioms = AxiomRegistry::new();
    axioms
        .register_with_coord(
            Text::from("external_type"),
            universe(0),
            framework(),
            high.clone(),
        )
        .unwrap();
    let axiom = reference("external_type", universe(0));
    let terms = [
        CoreTerm::Refl(Heap::new(axiom.clone())),
        CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(axiom.clone()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        },
        CoreTerm::Pair(Heap::new(universe(0)), Heap::new(axiom)),
    ];
    for term in terms {
        assert!(matches!(
            verum_kernel::infer_with_full_context(
                &Context::new(),
                &term,
                &axioms,
                &InductiveRegistry::new(),
                &low,
                false
            ),
            Err(KernelError::CoordViolation { .. })
        ));
        assert!(
            verum_kernel::infer_with_full_context(
                &Context::new(),
                &term,
                &axioms,
                &InductiveRegistry::new(),
                &high,
                false
            )
            .is_ok()
        );
        assert!(
            verum_kernel::infer_with_full_context(
                &Context::new(),
                &term,
                &axioms,
                &InductiveRegistry::new(),
                &low,
                true
            )
            .is_ok()
        );
    }
}

#[test]
fn transport_checks_source_value_and_rejects_nonpaths() {
    use verum_kernel::{Context, infer};
    let a = CoreTerm::Var(Text::from("A"));
    let x = CoreTerm::Var(Text::from("x"));
    let context = Context::new()
        .extend(Text::from("A"), universe(0))
        .extend(Text::from("x"), a.clone());
    let transport = |path, value| CoreTerm::Transport {
        path: Heap::new(path),
        value: Heap::new(value),
    };
    let path = CoreTerm::Refl(Heap::new(a.clone()));
    assert_eq!(
        infer(
            &context,
            &transport(path.clone(), x.clone()),
            &AxiomRegistry::new()
        )
        .unwrap(),
        a
    );
    assert!(
        infer(
            &context,
            &transport(path, universe(0)),
            &AxiomRegistry::new()
        )
        .is_err()
    );
    assert!(
        infer(
            &context,
            &transport(universe(0), x.clone()),
            &AxiomRegistry::new()
        )
        .is_err()
    );
    let malformed_lambda = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(x),
        body: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    assert!(infer(&context, &malformed_lambda, &AxiomRegistry::new()).is_err());
}

#[test]
fn transparent_aliases_preserve_citation_requirements() {
    use verum_kernel::{Context, KernelCoord, KernelError, OrdinalDepth};
    let high = KernelCoord::canonical(Text::from("high"), OrdinalDepth::finite(2));
    let low = KernelCoord::canonical(Text::from("low"), OrdinalDepth::finite(0));
    let mut axioms = AxiomRegistry::new();
    axioms
        .register_with_coord(Text::from("origin"), universe(0), framework(), high.clone())
        .unwrap();
    axioms
        .register_definition(
            Text::from("alias"),
            reference("origin", universe(0)),
            universe(0),
            framework(),
        )
        .unwrap();
    axioms
        .register_definition(
            Text::from("second_alias"),
            reference("alias", universe(0)),
            universe(0),
            framework(),
        )
        .unwrap();
    assert_eq!(axioms.get("second_alias").unwrap().coord, Some(high));
    assert!(matches!(
        verum_kernel::infer_with_full_context(
            &Context::new(),
            &reference("second_alias", universe(0)),
            &axioms,
            &InductiveRegistry::new(),
            &low,
            false
        ),
        Err(KernelError::CoordViolation { .. })
    ));
}

#[test]
fn definition_dependencies_cannot_be_rebound_to_another_universe() {
    let mut small = InductiveRegistry::new();
    small.register(declaration("Carrier", List::new())).unwrap();
    let mut axioms = AxiomRegistry::new();
    axioms
        .register_definition_with_inductives(
            Text::from("alias"),
            named("Carrier"),
            universe(0),
            framework(),
            &small,
        )
        .unwrap();
    let mut large = InductiveRegistry::new();
    large
        .register(declaration("Carrier", List::new()).with_universe(UniverseLevel::Concrete(1)))
        .unwrap();
    let term = reference("alias", universe(0));
    assert!(
        verum_kernel::infer_with_inductives(&verum_kernel::Context::new(), &term, &axioms, &small)
            .is_ok()
    );
    assert!(
        verum_kernel::infer_with_inductives(&verum_kernel::Context::new(), &term, &axioms, &large)
            .is_err()
    );
    assert!(verum_kernel::infer(&verum_kernel::Context::new(), &term, &axioms).is_err());
}

#[test]
fn inductive_dependencies_cannot_rebind_a_transparent_field_type() {
    let mut axioms = AxiomRegistry::new();
    axioms
        .register_definition(Text::from("Field"), universe(0), universe(1), framework())
        .unwrap();
    let mut registry = InductiveRegistry::new();
    registry
        .register_group(
            List::from_iter([declaration(
                "Box",
                List::from_iter([reference("Field", universe(1))]),
            )
            .with_universe(UniverseLevel::Concrete(1))]),
            &axioms,
        )
        .unwrap();
    assert!(
        verum_kernel::infer_with_inductives(
            &verum_kernel::Context::new(),
            &named("Box"),
            &axioms,
            &registry
        )
        .is_ok()
    );
    let mut replacement = AxiomRegistry::new();
    replacement
        .register_definition(Text::from("Field"), universe(1), universe(2), framework())
        .unwrap();
    assert!(
        verum_kernel::infer_with_inductives(
            &verum_kernel::Context::new(),
            &named("Box"),
            &replacement,
            &registry
        )
        .is_err()
    );
}
