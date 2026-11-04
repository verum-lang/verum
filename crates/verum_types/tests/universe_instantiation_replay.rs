//! Check native instantiation maps and resolved expressions with independent
//! semantics. A successful native unification is input to replay, never proof.

use std::{fmt::Write, path::Path, process::Command};

use verum_common::{List, Map, Text};
use verum_types::{
    universe_inference::UniverseInference,
    universe_level::{UniverseLevel as Level, UniverseMeta},
};

enum Work<'a> {
    Visit(&'a Level),
    Successor,
    Maximum,
}

fn emit_formula(output: &mut Text, level: &Level, next: &mut usize) -> usize {
    let mut work = List::from_single(Work::Visit(level));
    let mut values = List::new();
    while let Some(item) = work.pop() {
        let expression = match item {
            Work::Visit(Level::Concrete(value)) => format!(".concrete {value}"),
            Work::Visit(Level::Variable(id)) => format!(".parameter {id}"),
            Work::Visit(Level::Meta(id)) => format!(".metavar {}", id.id()),
            Work::Visit(Level::Succ(inner)) => {
                work.push(Work::Successor);
                work.push(Work::Visit(inner));
                continue;
            }
            Work::Visit(Level::Max(left, right)) => {
                work.push(Work::Maximum);
                work.push(Work::Visit(right));
                work.push(Work::Visit(left));
                continue;
            }
            Work::Successor => format!(".succ e{}", values.pop().unwrap()),
            Work::Maximum => {
                let right = values.pop().unwrap();
                let left = values.pop().unwrap();
                format!(".max e{left} e{right}")
            }
        };
        writeln!(output, "def e{next} : Formula := {expression}").unwrap();
        values.push(*next);
        *next += 1;
    }
    values.pop().unwrap()
}

fn emit_bindings<'a>(
    output: &mut Text,
    name: &str,
    entries: impl Iterator<Item = (u64, &'a Level)>,
    next: &mut usize,
) {
    let mut entries: List<_> = entries.collect();
    entries.sort_by_key(|entry| entry.0);
    let mut emitted = List::new();
    for (id, level) in entries {
        emitted.push((id, emit_formula(output, level, next)));
    }
    write!(output, "def {name} : Bindings := [").unwrap();
    for (index, (id, expression)) in emitted.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "({id}, e{expression})").unwrap();
    }
    output.push_str("]\n");
}

fn emit_assignment(output: &mut Text, inference: &UniverseInference) {
    let mut assignments = List::new();
    for id in inference.meta_bindings().keys() {
        let value = inference
            .apply(&Level::Meta(*id))
            .unwrap()
            .evaluate(&Map::new())
            .expect("this replay case must have a ground model");
        assignments.push((id.id(), value));
    }
    assignments.sort_by_key(|entry| entry.0);
    output.push_str("def assignment : Assignment := [");
    for (index, (id, value)) in assignments.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "({id}, {value})").unwrap();
    }
    output.push_str("]\n");
}

fn emit_case(
    output: &mut Text,
    name: &str,
    declared: &[u32],
    inference: &UniverseInference,
    source: &Level,
    equation: Option<(&Level, &Level)>,
) {
    emit_observed_case(
        output,
        name,
        declared,
        inference,
        source,
        &inference.apply(source).unwrap(),
        equation,
    );
}

fn emit_observed_case(
    output: &mut Text,
    name: &str,
    declared: &[u32],
    inference: &UniverseInference,
    source: &Level,
    observed: &Level,
    equation: Option<(&Level, &Level)>,
) {
    writeln!(
        output,
        "\nnamespace Verum.UniverseInstantiationReplay.{name}"
    )
    .unwrap();
    output.push_str("open Verum.UniverseInstantiation\n");
    let mut next = 0;
    let source_id = emit_formula(output, source, &mut next);
    let output_id = emit_formula(output, observed, &mut next);
    writeln!(
        output,
        "def source : Formula := e{source_id}\ndef output : Formula := e{output_id}"
    )
    .unwrap();
    emit_bindings(
        output,
        "parameters",
        inference
            .parameter_bindings()
            .iter()
            .map(|(id, level)| (u64::from(*id), level)),
        &mut next,
    );
    emit_bindings(
        output,
        "metas",
        inference
            .meta_bindings()
            .iter()
            .map(|(id, level)| (id.id(), level)),
        &mut next,
    );
    // Bind the witness to the declaration supplied to the actual native call,
    // rather than deriving the declaration back from the solver's output map.
    let mut declared: List<_> = declared.iter().copied().collect();
    declared.sort();
    write!(output, "def declared : List Nat := [").unwrap();
    for (index, id) in declared.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "{id}").unwrap();
    }
    output.push_str("]\n");
    output.push_str(
        r"theorem rosterChecked : RosterValid declared parameters = true := by decide
theorem outputChecked : resolve parameters metas 64 true source = some output := by decide
theorem nativeSemantics : ∀ p m, Model metas p m →
    denote p m output = denote (parameterValuation parameters p m) m source := by
  intro p m agreement
  exact resolve_sound agreement outputChecked
#print axioms rosterChecked
#print axioms outputChecked
#print axioms nativeSemantics
",
    );
    if let Some((left, right)) = equation {
        emit_assignment(output, inference);
        let left_source = emit_formula(output, left, &mut next);
        let right_source = emit_formula(output, right, &mut next);
        let left_output = emit_formula(output, &inference.apply(left).unwrap(), &mut next);
        let right_output = emit_formula(output, &inference.apply(right).unwrap(), &mut next);
        // The selected equation resolves to exactly equal syntax. Its original
        // sides need not be equal, and both outputs are independently replayed.
        writeln!(output,
            r"theorem modelChecked : GroundModelValid metas assignment = true := by decide
theorem leftChecked : resolve parameters metas 64 true e{left_source} = some e{left_output} := by decide
theorem rightChecked : resolve parameters metas 64 true e{right_source} = some e{right_output} := by decide
theorem outputsAgree : ∀ p, denote p (assignmentValuation assignment) e{left_output} =
    denote p (assignmentValuation assignment) e{right_output} := by intro p; rfl
theorem witnessedSemantics : ∀ p, denote p (assignmentValuation assignment) output =
    denote (parameterValuation parameters p (assignmentValuation assignment))
      (assignmentValuation assignment) source :=
  checked_resolution modelChecked outputChecked
theorem verifiedEquation : ∀ p,
    denote (parameterValuation parameters p (assignmentValuation assignment))
      (assignmentValuation assignment) e{left_source} =
    denote (parameterValuation parameters p (assignmentValuation assignment))
      (assignmentValuation assignment) e{right_source} :=
  checked_equation modelChecked leftChecked rightChecked outputsAgree
#print axioms modelChecked
#print axioms witnessedSemantics
#print axioms verifiedEquation"
        ).unwrap();
    } else {
        // No assumptions about unbound metavariables: this statement quantifies
        // over every valuation of their actual opaque native identities.
        output.push_str(
            r"theorem unrestrictedSemantics : ∀ p m,
    denote p m output = denote (parameterValuation parameters p m) m source := by
  intro p m
  apply nativeSemantics p m
  simp [Model, metas, lookup]
#print axioms unrestrictedSemantics
",
        );
    }
    writeln!(output, "end Verum.UniverseInstantiationReplay.{name}").unwrap();
}

fn native_replay() -> Text {
    let declared = [7, 9];
    let mut inference = UniverseInference::instantiate(&declared).unwrap();
    let a = inference.apply(&Level::Variable(7)).unwrap();
    let b = inference.apply(&Level::Variable(9)).unwrap();
    let opaque = Level::Meta(UniverseMeta::fresh());
    let source = Level::Max(
        Box::new(Level::Variable(7).succ()),
        Box::new(Level::Max(
            Box::new(Level::Variable(9)),
            Box::new(Level::Variable(4_000_000_000)),
        )),
    );
    let source_with_opaque = Level::Max(Box::new(source.clone()), Box::new(opaque));
    let mut output = Text::from(include_str!(
        "../../../verification/external/lean/VerumExternalReplay/UniverseInstantiation.lean"
    ));
    output.push_str(
        "\n#print axioms Verum.UniverseInstantiation.resolve_sound\n\
         #print axioms Verum.UniverseInstantiation.resolve_preserves_agreed_valuation\n\
         #print axioms Verum.UniverseInstantiation.groundModel_sound\n\
         #print axioms Verum.UniverseInstantiation.checked_equation\n",
    );
    use verum_types::{
        context::{GenericBinder, TypeScheme},
        ty::{Type, TypeVar},
    };
    let owner = TypeVar::fresh();
    let source_level = Level::Max(
        Box::new(Level::Variable(7)),
        Box::new(Level::Variable(9).succ()),
    );
    let source_type = Type::universe(source_level.clone());
    let mut bounds = Map::new();
    bounds.insert(
        owner,
        List::from_single(Type::function(List::new(), source_type.clone())),
    );
    let mut scheme = TypeScheme::poly(List::from_single(owner), source_type.clone())
        .with_type_bounds(bounds)
        .with_universe_parameters(declared.into_iter().collect())
        .unwrap();
    scheme.impl_self_type = Some(source_type);
    scheme.explicit_method_vars =
        Some(List::from_iter(declared.into_iter().map(|id| {
            Some(GenericBinder::Universe(Level::Variable(id)))
        })));
    let instance = scheme.try_instantiate().unwrap();
    let Type::Function { return_type, .. } = &instance.type_bounds[&instance.fresh_vars[0]][0]
    else {
        panic!("callable bound")
    };
    for (name, ty) in [
        ("SchemeBody", &instance.ty),
        ("SchemeBound", return_type.as_ref()),
        ("SchemeReceiver", instance.impl_self_type.as_ref().unwrap()),
    ] {
        let Type::Universe { level } = ty else {
            panic!("universe field")
        };
        emit_observed_case(
            &mut output,
            name,
            &declared,
            instance.universe_instantiation(),
            &source_level,
            level,
            None,
        );
    }
    for (index, parameter) in declared.iter().enumerate() {
        let Some(GenericBinder::Universe(observed)) =
            &instance.explicit_method_vars.as_ref().unwrap()[index]
        else {
            panic!("native explicit universe slot")
        };
        emit_observed_case(
            &mut output,
            &format!("SchemeSlot{index}"),
            &declared,
            instance.universe_instantiation(),
            &Level::Variable(*parameter),
            observed,
            None,
        );
    }
    emit_case(
        &mut output,
        "Unsolved",
        &declared,
        &inference,
        &source_with_opaque,
        None,
    );

    let independent = UniverseInference::instantiate(&declared).unwrap();
    emit_case(
        &mut output,
        "Independent",
        &declared,
        &independent,
        &source,
        None,
    );
    // Equal numeric IDs in the two namespaces must remain distinct. The opaque
    // meta is not a declaration occurrence and must not be renamed with it.
    let opaque = UniverseMeta::fresh();
    let parameter = u32::try_from(opaque.id()).expect("test identity fits a declaration ID");
    let separated = UniverseInference::instantiate(&[parameter]).unwrap();
    let collision = Level::Max(
        Box::new(Level::Variable(parameter)),
        Box::new(Level::Meta(opaque)),
    );
    emit_case(
        &mut output,
        "NamespaceSeparation",
        &[parameter],
        &separated,
        &collision,
        None,
    );
    output.push_str(
        r"namespace Verum.UniverseInstantiationReplay
open Verum.UniverseInstantiation
theorem nativeInstancesAreDisjoint :
    (metaIds Unsolved.parameters ++ metaIds Independent.parameters).Nodup := by decide
#print axioms nativeInstancesAreDisjoint
end Verum.UniverseInstantiationReplay
",
    );

    // Two links exercise recursive meta resolution. Remaining rigid parameters
    // are universally quantified; their numeric identity never becomes a level.
    inference.unify(&a, &b.succ()).unwrap();
    inference.unify(&b, &Level::Concrete(3)).unwrap();
    let equation = (Level::Variable(7), Level::Variable(9).succ());
    let wide_source = Level::Max(Box::new(source), Box::new(Level::Concrete(u32::MAX).succ()));
    emit_case(
        &mut output,
        "Resolved",
        &declared,
        &inference,
        &wide_source,
        Some((&equation.0, &equation.1)),
    );
    output
}

fn lean(path: &Path) -> std::process::Output {
    Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .expect("Lean must be installed for this explicitly requested independent replay")
}

#[test]
fn replay_preserves_native_parameters_metas_and_full_width_levels() {
    let replay = native_replay();
    assert!(replay.contains(".parameter 4000000000"));
    assert!(replay.contains(".metavar "));
    assert!(replay.contains(".concrete 4294967295"));
    assert!(replay.contains(".succ e"));
    assert!(replay.contains("nativeInstancesAreDisjoint"));
    assert!(replay.contains("checked_equation modelChecked leftChecked rightChecked outputsAgree"));
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn independent_lean_checks_native_instantiation_and_resolved_equations() {
    let text = native_replay();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("UniverseInstantiationReplay.lean");
    // Inline the current model: an old .olean cannot validate a new witness.
    std::fs::write(&path, text.as_bytes()).unwrap();
    let output = lean(&path);
    let stdout = std::string::String::from_utf8_lossy(&output.stdout);
    let stderr = std::string::String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    for theorem in [
        "SchemeBody.unrestrictedSemantics",
        "SchemeBound.unrestrictedSemantics",
        "SchemeReceiver.unrestrictedSemantics",
        "SchemeSlot0.unrestrictedSemantics",
        "SchemeSlot1.unrestrictedSemantics",
        "Unsolved.unrestrictedSemantics",
        "Independent.unrestrictedSemantics",
        "NamespaceSeparation.unrestrictedSemantics",
        "nativeInstancesAreDisjoint",
        "Resolved.witnessedSemantics",
        "Resolved.verifiedEquation",
    ] {
        assert!(
            stdout.contains(theorem),
            "missing independently checked theorem {theorem}"
        );
    }
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(
            !stdout.contains(forbidden) && !stderr.contains(forbidden),
            "{stdout}\n{stderr}"
        );
    }

    let changed_binder = text.replacen(
        "def declared : List Nat := [7, 9]",
        "def declared : List Nat := [7, 10]",
        1,
    );
    assert_ne!(changed_binder, text);
    std::fs::write(&path, changed_binder.as_bytes()).unwrap();
    assert!(
        !lean(&path).status.success(),
        "Lean accepted a different declaration roster"
    );

    let changed_output = text.replacen(
        "resolve parameters metas 64 true source = some output",
        "resolve parameters metas 64 true source = some (.parameter 4000000001)",
        1,
    );
    assert_ne!(changed_output, text);
    std::fs::write(&path, changed_output.as_bytes()).unwrap();
    assert!(
        !lean(&path).status.success(),
        "Lean accepted an unrelated resolved output"
    );
}
