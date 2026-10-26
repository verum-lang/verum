use verum_common::{Heap, Text};
use verum_kernel::{AxiomRegistry, Context, CoreTerm, UniverseLevel, infer, normalize};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn universe() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}

fn context() -> Context {
    Context::new()
        .extend(Text::from("A"), universe())
        .extend(Text::from("B"), universe())
        .extend(Text::from("x"), var("A"))
        .extend(
            Text::from("p"),
            CoreTerm::PathTy {
                carrier: Heap::new(universe()),
                lhs: Heap::new(var("A")),
                rhs: Heap::new(var("B")),
            },
        )
}

#[test]
fn variable_names_cannot_authorize_transport_identity() {
    for name in ["i1", "1", "true", "ordinary"] {
        let path_type = infer(&context(), &var("p"), &AxiomRegistry::new()).unwrap();
        let ctx = context().extend(Text::from(name), path_type);
        let term = CoreTerm::Transport {
            path: Heap::new(var(name)),
            value: Heap::new(var("x")),
        };
        let ty = infer(&ctx, &term, &AxiomRegistry::new()).unwrap();
        assert_eq!(ty, var("B"));
        let normal = normalize(&term);
        assert_eq!(
            infer(&ctx, &normal, &AxiomRegistry::new()).unwrap(),
            ty,
            "normalization changed the type for marker {name}"
        );
    }
}

fn transport(path: CoreTerm, value: CoreTerm) -> CoreTerm {
    CoreTerm::Transport {
        path: Heap::new(path),
        value: Heap::new(value),
    }
}

#[test]
fn reflexive_transport_preserves_the_source_type() {
    let term = transport(CoreTerm::Refl(Heap::new(var("A"))), var("x"));
    let ty = infer(&context(), &term, &AxiomRegistry::new()).unwrap();
    let normal = normalize(&term);
    assert_eq!(normal, var("x"));
    assert_eq!(
        infer(&context(), &normal, &AxiomRegistry::new()).unwrap(),
        ty
    );
}

#[test]
fn neutral_transport_requires_the_actual_source_type() {
    let ctx = context().extend(Text::from("y"), var("B"));
    assert!(infer(&ctx, &transport(var("p"), var("y")), &AxiomRegistry::new()).is_err());
    assert!(
        infer(
            &ctx,
            &transport(var("p"), universe()),
            &AxiomRegistry::new()
        )
        .is_err()
    );
}

#[test]
fn ordinary_paths_and_annotations_cannot_transport_types() {
    let axioms = AxiomRegistry::new();
    let annotation = infer(&context(), &var("p"), &axioms).unwrap();
    for path in [CoreTerm::Refl(Heap::new(var("x"))), annotation, universe()] {
        assert!(infer(&context(), &transport(path, var("x")), &axioms).is_err());
    }
}

#[test]
fn substitution_can_reveal_reflexivity_without_losing_lexical_scope() {
    use verum_kernel::{free_vars, substitute};
    let term = transport(var("p"), var("x"));
    let free = free_vars(&term);
    assert_eq!(free.len(), 2);
    assert!(free.contains(&Text::from("p")) && free.contains(&Text::from("x")));
    let reduced = substitute(&term, "p", &CoreTerm::Refl(Heap::new(var("A"))));
    assert_eq!(normalize(&reduced), var("x"));
    let function = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(var("A")),
        body: Heap::new(term),
    };
    let captured = substitute(&function, "p", &var("x"));
    assert!(free_vars(&captured).contains(&Text::from("x")));
}

#[test]
fn beta_reduced_path_evidence_computes() {
    let function = CoreTerm::Lam {
        binder: Text::from("T"),
        domain: Heap::new(universe()),
        body: Heap::new(CoreTerm::Refl(Heap::new(var("T")))),
    };
    let term = transport(
        CoreTerm::App(Heap::new(function), Heap::new(var("A"))),
        var("x"),
    );
    assert_eq!(
        infer(&context(), &term, &AxiomRegistry::new()).unwrap(),
        var("A")
    );
    assert_eq!(normalize(&term), var("x"));
}

#[test]
fn transport_round_trips_but_legacy_regularity_payloads_are_refused() {
    let term = transport(var("p"), var("x"));
    let wire = serde_json::to_value(&term).unwrap();
    assert_eq!(serde_json::from_value::<CoreTerm>(wire).unwrap(), term);
    for name in ["i1", "1", "true", "ordinary"] {
        let legacy = serde_json::json!({"Transp": {
            "path": var("p"), "regular": var(name), "value": var("x")
        }});
        assert!(serde_json::from_value::<CoreTerm>(legacy).is_err());
    }
}

#[test]
fn checked_in_schemas_match_the_production_exporter() {
    use verum_kernel::soundness::{
        SoundnessExporter, agda::AgdaBackend, coq::CoqBackend, isabelle::IsabelleBackend,
        lean::LeanBackend,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external");
    let exporter = SoundnessExporter::new();
    for (path, generated) in [
        (
            "lean/VerumExternalReplay/KernelSoundness.lean",
            exporter.emit(&LeanBackend::new()),
        ),
        ("coq/kernel_soundness.v", exporter.emit(&CoqBackend::new())),
        (
            "isabelle/KernelSoundness.thy",
            exporter.emit(&IsabelleBackend::new()),
        ),
        (
            "agda/KernelSoundness.agda",
            exporter.emit(&AgdaBackend::new()),
        ),
    ] {
        assert_eq!(
            std::fs::read_to_string(root.join(path)).unwrap(),
            generated,
            "checked-in schema is stale: {path}"
        );
    }
}

#[test]
#[ignore = "requires Lean and Coq executables to check the generated rule schemas"]
fn transport_export_schemas_typecheck() {
    use std::{fs, process::Command};
    use verum_kernel::soundness::{
        SoundnessExporter, agda::AgdaBackend, coq::CoqBackend, isabelle::IsabelleBackend,
        lean::LeanBackend,
    };

    let dir = std::env::temp_dir().join(format!("verum-transport-export-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let exporter = SoundnessExporter::new();
    if let Some(output) = std::env::var_os("VERUM_TRANSPORT_SCHEMA_OUTPUT") {
        let output = std::path::PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        for (name, source) in [
            ("KernelSoundness.lean", exporter.emit(&LeanBackend::new())),
            ("kernel_soundness.v", exporter.emit(&CoqBackend::new())),
            (
                "KernelSoundness.thy",
                exporter.emit(&IsabelleBackend::new()),
            ),
            ("KernelSoundness.agda", exporter.emit(&AgdaBackend::new())),
        ] {
            fs::write(output.join(name), source).unwrap();
        }
    }
    for (tool, name, source) in [
        (
            "lean",
            "TransportSchema.lean",
            exporter.emit(&LeanBackend::new()),
        ),
        (
            "coqc",
            "TransportSchema.v",
            exporter.emit(&CoqBackend::new()),
        ),
    ] {
        fs::write(dir.join(name), source).unwrap();
        let output = Command::new(tool)
            .arg(name)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{tool} rejected the generated schema at {}:\n{}\n{}",
            dir.display(),
            std::str::from_utf8(&output.stdout).unwrap_or("non-UTF-8 output"),
            std::str::from_utf8(&output.stderr).unwrap_or("non-UTF-8 output")
        );
    }
    fs::remove_dir_all(dir).unwrap();
}
