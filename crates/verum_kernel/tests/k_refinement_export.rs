use std::{fs, process::Command};

use verum_common::{Heap, Text};
use verum_kernel::soundness::{SoundnessExporter, lean::LeanBackend};
use verum_kernel::{AxiomRegistry, Context, CoreTerm, KernelError, UniverseLevel, infer};

const ADMISSION: &str = include_str!("fixtures/refinement_admission.lean");

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}
fn refinement(base: CoreTerm, predicate: CoreTerm) -> CoreTerm {
    CoreTerm::Refine {
        base: Heap::new(base),
        binder: Text::from("x"),
        predicate: Heap::new(predicate),
    }
}

#[test]
fn rust_admission_matches_replay_boundary_controls() {
    let axioms = AxiomRegistry::new();
    for level in [
        UniverseLevel::Concrete(0),
        UniverseLevel::Concrete(7),
        UniverseLevel::Variable(Text::from("u")),
    ] {
        let ctx = Context::new()
            .extend(Text::from("A"), CoreTerm::Universe(level.clone()))
            .extend(Text::from("P"), CoreTerm::Universe(UniverseLevel::Prop));
        assert_eq!(
            infer(&ctx, &refinement(var("A"), var("P")), &axioms).unwrap(),
            CoreTerm::Universe(level)
        );
        assert!(matches!(
            infer(&ctx, &refinement(var("A"), var("x")), &axioms),
            Err(KernelError::InvalidRefinementPredicate { .. })
        ));
        assert!(matches!(
            infer(
                &ctx,
                &refinement(var("A"), CoreTerm::Universe(UniverseLevel::Concrete(1))),
                &axioms
            ),
            Err(KernelError::DepthViolation { .. })
        ));
        assert!(infer(&ctx, &refinement(var("missing"), var("P")), &axioms).is_err());
    }
}

#[test]
#[ignore = "requires an installed Lean to independently check the admission model and mutation refusals"]
fn refinement_boundary_and_schema_replay_in_lean() {
    let dir = std::env::var_os("VERUM_REFINEMENT_REPLAY_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("verum-refinement-replay-{}", std::process::id()))
        });
    fs::create_dir_all(&dir).unwrap();
    let lean = std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into());
    let version = Command::new(&lean)
        .arg("--version")
        .output()
        .expect("Lean must be installed");
    assert!(version.status.success());
    fs::write(dir.join("lean-version.txt"), &version.stdout).unwrap();
    let run = |name: &str, source: &str, accept: bool| {
        fs::write(dir.join(name), source).unwrap();
        let result = Command::new(&lean)
            .arg(name)
            .current_dir(&dir)
            .output()
            .unwrap();
        fs::write(dir.join(format!("{name}.stdout")), &result.stdout).unwrap();
        fs::write(dir.join(format!("{name}.stderr")), &result.stderr).unwrap();
        assert_eq!(
            result.status.success(),
            accept,
            "Lean verdict for {name}:\n{}\n{}",
            std::str::from_utf8(&result.stdout).unwrap_or("non-UTF8 output"),
            std::str::from_utf8(&result.stderr).unwrap_or("non-UTF8 output")
        );
        result
    };
    let accepted = run("RefinementAdmission.lean", ADMISSION, true);
    assert!(
        std::str::from_utf8(&accepted.stdout).unwrap_or("non-UTF8 output")
            .contains("'VerumRefinementAdmission.accepted_iff' does not depend on any axioms")
    );
    let missing_sort_gate = ADMISSION.replace(
        "| .other => .invalidPredicate",
        "| .other => .accepted level",
    );
    assert_ne!(missing_sort_gate, ADMISSION);
    run("MissingSortGate.lean", &missing_sort_gate, false);
    let wrong_depth_boundary = ADMISSION.replacen(
        "if predicateDepth > baseDepth",
        "if predicateDepth >= baseDepth",
        1,
    );
    assert_ne!(wrong_depth_boundary, ADMISSION);
    run("WrongDepthBoundary.lean", &wrong_depth_boundary, false);

    // The whole exported Typing relation has separate, declared trust gaps.
    // These controls check the corrected rule interface, not Rust soundness.
    let schema = SoundnessExporter::new().emit(&LeanBackend::new());
    run(
        "RefinementSchema.lean",
        &format!("{schema}\n{FORMATION_CONTROL}"),
        true,
    );
    run(
        "MissingWitness.lean",
        &format!("{schema}\n{MISSING_WITNESS}"),
        false,
    );
    run(
        "WrongWitness.lean",
        &format!("{schema}\n{WRONG_WITNESS}"),
        false,
    );
}

const FORMATION_CONTROL: &str = r#"
namespace KernelSoundness
example : Typing [] CoreTerm.PropSort (CoreTerm.Universe 0) := Typing.t_prop []
example : Typing [("P", CoreTerm.PropSort)]
    (CoreTerm.Refine (CoreTerm.Universe 0) "x" (CoreTerm.Var "P"))
    (CoreTerm.Universe 1) := by
  apply K_Refine_sound
  · exact Typing.t_univ _ 0
  · apply Typing.t_var
    simp
  · simp [mDepth]
end KernelSoundness
"#;

const MISSING_WITNESS: &str = r#"
namespace KernelSoundness
example {Γ : Ctx} {a base predicate : CoreTerm} {x : String} {i : Nat}
    (formed : Typing Γ (CoreTerm.Refine base x predicate) (CoreTerm.Universe i))
    (value : Typing Γ a base) : Typing Γ a (CoreTerm.Refine base x predicate) := by
  exact K_Refine_Intro_sound formed value
end KernelSoundness
"#;

const WRONG_WITNESS: &str = r#"
namespace KernelSoundness
example {Γ : Ctx} {a base predicate witness other : CoreTerm} {x : String} {i : Nat}
    (formed : Typing Γ (CoreTerm.Refine base x predicate) (CoreTerm.Universe i))
    (value : Typing Γ a base) (proof : Typing Γ witness (subst x other predicate)) :
    Typing Γ a (CoreTerm.Refine base x predicate) := by
  exact K_Refine_Intro_sound formed value proof
end KernelSoundness
"#;
