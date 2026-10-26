use verum_common::{Heap, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, UniverseLevel, infer, normalize,
};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}
fn universe(level: u32) -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(level))
}

fn context(face: &str) -> Context {
    Context::new()
        .extend(Text::from("A"), universe(0))
        .extend(Text::from("x"), var("A"))
        .extend(Text::from(face), universe(0))
}

fn hcomp(face: CoreTerm) -> CoreTerm {
    CoreTerm::HComp {
        phi: Heap::new(face),
        walls: Heap::new(CoreTerm::Lam {
            binder: Text::from("i"),
            domain: Heap::new(var("A")),
            body: Heap::new(universe(0)),
        }),
        base: Heap::new(var("x")),
    }
}
fn glue(face: CoreTerm) -> CoreTerm {
    CoreTerm::Glue {
        carrier: Heap::new(var("A")),
        phi: Heap::new(face),
        fiber: Heap::new(var("x")),
        equiv: Heap::new(var("x")),
    }
}

#[test]
fn individually_typable_children_do_not_establish_cubical_judgments() {
    for term in [hcomp(var("phi")), glue(var("phi"))] {
        assert!(infer(&context("phi"), &term, &AxiomRegistry::new()).is_err());
    }
}

#[test]
fn face_spelling_cannot_erase_composition_or_gluing() {
    for face in [
        "top", "true", "i1", "1", "⊤", "bot", "false", "i0", "0", "⊥", "phi",
    ] {
        for term in [hcomp(var(face)), glue(var(face))] {
            assert_eq!(
                normalize(&term),
                term,
                "untyped face marker {face} authorized reduction"
            );
        }
    }
}

#[test]
fn unsupported_cubes_cannot_enter_through_reflexivity_or_beta_erasure() {
    for term in [hcomp(var("phi")), glue(var("phi"))] {
        let refl = CoreTerm::Refl(Heap::new(term.clone()));
        assert!(infer(&context("phi"), &refl, &AxiomRegistry::new()).is_err());
        let discard = CoreTerm::Lam {
            binder: Text::from("ignored"),
            domain: Heap::new(var("A")),
            body: Heap::new(var("x")),
        };
        let application = CoreTerm::App(Heap::new(discard), Heap::new(term));
        assert!(infer(&context("phi"), &application, &AxiomRegistry::new()).is_err());
    }
}

#[test]
fn definitions_cannot_register_unchecked_composition() {
    let mut axioms = AxiomRegistry::new();
    let term = CoreTerm::HComp {
        phi: Heap::new(universe(0)),
        walls: Heap::new(universe(0)),
        base: Heap::new(universe(0)),
    };
    assert!(
        axioms
            .register_definition(
                Text::from("bad_cube"),
                term,
                universe(1),
                FrameworkId {
                    framework: Text::from("test"),
                    citation: Text::from("explicit fixture")
                }
            )
            .is_err()
    );
    assert!(axioms.get("bad_cube").is_none());
}
