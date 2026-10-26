//! Explicit declarations for tests exercising named standard types.
//! Unknown names are never inferred from test terms or assigned a default universe.
#![allow(dead_code)]
use verum_common::{List, Text};
use verum_kernel::*;

pub fn declarations() -> InductiveRegistry {
    fn named(name: &str) -> CoreTerm {
        CoreTerm::Inductive {
            path: Text::from(name),
            args: List::new(),
        }
    }
    fn ctor(name: &str, fields: List<CoreTerm>) -> ConstructorSig {
        ConstructorSig {
            name: Text::from(name),
            arg_types: fields,
        }
    }
    let mut registry = InductiveRegistry::new();
    for declaration in [
        RegisteredInductive::new(
            Text::from("Unit"),
            List::new(),
            List::from_iter([ctor("tt", List::new())]),
        ),
        RegisteredInductive::new(
            Text::from("Bool"),
            List::new(),
            List::from_iter([ctor("False", List::new()), ctor("True", List::new())]),
        ),
        RegisteredInductive::new(
            Text::from("Nat"),
            List::new(),
            List::from_iter([
                ctor("Zero", List::new()),
                ctor("Succ", List::from_iter([named("Nat")])),
            ]),
        ),
        RegisteredInductive::new(
            Text::from("Int"),
            List::new(),
            List::from_iter([
                ctor("Positive", List::from_iter([named("Nat")])),
                ctor("Negative", List::from_iter([named("Nat")])),
            ]),
        ),
    ] {
        registry.register(declaration).unwrap();
    }
    let a = CoreTerm::Var(Text::from("A"));
    registry
        .register(RegisteredInductive::new(
            Text::from("List"),
            List::from_iter([Text::from("A")]),
            List::from_iter([
                ctor("Nil", List::new()),
                ctor(
                    "Cons",
                    List::from_iter([
                        a.clone(),
                        CoreTerm::Inductive {
                            path: Text::from("List"),
                            args: List::from_iter([a]),
                        },
                    ]),
                ),
            ]),
        ))
        .unwrap();
    registry
}

pub fn infer_fixture(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreTerm, KernelError> {
    infer_with_inductives(ctx, term, axioms, &declarations())
}
pub fn check_fixture(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreType, KernelError> {
    infer_fixture(ctx, term, axioms).map(|ty| verum_kernel::support::shape_of(&ty))
}
pub fn verify_fixture(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    verify_with_inductives(ctx, term, expected, axioms, &declarations())
}

pub trait FixtureAxioms {
    fn register_fixture(
        &mut self,
        name: Text,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError>;
    fn register_fixture_definition(
        &mut self,
        name: Text,
        body: CoreTerm,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError>;
}
impl FixtureAxioms for AxiomRegistry {
    fn register_fixture(
        &mut self,
        name: Text,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError> {
        self.register_with_regime_and_inductives(
            name,
            ty,
            framework,
            SubsingletonRegime::ClosedPropositionOnly,
            &declarations(),
        )
    }
    fn register_fixture_definition(
        &mut self,
        name: Text,
        body: CoreTerm,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError> {
        self.register_definition_with_inductives(name, body, ty, framework, &declarations())
    }
}
