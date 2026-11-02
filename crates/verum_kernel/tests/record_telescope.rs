use std::fmt::Write;
#[path = "../../../verification/external/lean/record_telescope_replay.rs"]
mod replay;
use verum_common::{
    Heap, Text,
    record_telescope::{RecordTelescope, TelescopeError},
};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, InductiveRegistry, UniverseLevel,
    record_telescope::{RecordError, SigmaRecord},
    verify_with_inductives,
};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(name.into())
}
fn universe() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}
fn path(carrier: CoreTerm, point: CoreTerm) -> CoreTerm {
    CoreTerm::PathTy {
        carrier: Heap::new(carrier),
        lhs: Heap::new(point.clone()),
        rhs: Heap::new(point),
    }
}
fn fields() -> Vec<(Text, CoreTerm)> {
    vec![
        ("carrier".into(), universe()),
        ("point".into(), var("carrier")),
        ("witness".into(), path(var("carrier"), var("point"))),
    ]
}
fn environment() -> Context {
    Context::new()
        .extend("T".into(), universe())
        .extend("element".into(), var("T"))
        .extend("Other".into(), universe())
        .extend("wrong".into(), var("Other"))
}

#[test]
fn checks_formation_construction_receiver_projections_and_partial_writes() {
    let ctx = environment();
    let axioms = AxiomRegistry::new();
    let inductives = InductiveRegistry::new();
    let record = SigmaRecord::form(&ctx, fields(), &axioms, &inductives).unwrap();
    assert_eq!(record.plan().rows(), &[vec![], vec![0], vec![0, 1]]);
    let values = [
        var("T"),
        var("element"),
        CoreTerm::Refl(Heap::new(var("element"))),
    ];
    let introduction = record
        .construct(&ctx, &values, &axioms, &inductives)
        .unwrap();
    assert_eq!(introduction.fields[2].ty, path(var("T"), var("element")));
    verify_with_inductives(
        &ctx,
        &introduction.term,
        record.schema(),
        &axioms,
        &inductives,
    )
    .unwrap();
    let mut bad_values = values.clone();
    bad_values[1] = var("wrong");
    assert!(
        record
            .construct(&ctx, &bad_values, &axioms, &inductives)
            .is_err()
    );
    assert!(
        record
            .construct(&ctx, &values[..2], &axioms, &inductives)
            .is_err()
    );
    for malformed in [
        vec![("a".into(), var("b")), ("b".into(), universe())],
        vec![("a".into(), var("a"))],
        vec![("a".into(), universe()), ("a".into(), universe())],
    ] {
        assert!(SigmaRecord::form(&ctx, malformed, &axioms, &inductives).is_err());
    }
    let ctx = ctx
        .extend("r".into(), record.schema().clone())
        .extend("s".into(), record.schema().clone());
    let receiver: Text = "r".into();
    let local = record
        .project(&ctx, &receiver, 2, &axioms, &inductives)
        .unwrap();
    let foreign = record
        .project(&ctx, &"s".into(), 2, &axioms, &inductives)
        .unwrap();
    assert_ne!(local.field.ty, foreign.field.ty);
    assert!(
        record
            .check_mutation(&ctx, &receiver, 2, &local.term, &axioms, &inductives)
            .is_ok()
    );
    assert!(
        record
            .check_mutation(&ctx, &receiver, 2, &foreign.term, &axioms, &inductives)
            .is_err()
    );
    assert!(matches!(
        record.check_mutation(&ctx, &receiver, 0, &var("Other"), &axioms, &inductives),
        Err(RecordError::DependedOn(0))
    ));
    assert!(matches!(
        record.check_mutation(&ctx, &receiver, 1, &var("element"), &axioms, &inductives),
        Err(RecordError::DependedOn(1))
    ));
    assert!(
        record
            .project(&ctx, &"Other".into(), 1, &axioms, &inductives)
            .is_err()
    );
}

#[test]
fn canonical_rows_ordered_substitution_and_missing_witnesses() {
    let plan = RecordTelescope::from_dependencies([vec![], vec![], vec![1, 0, 1]]).unwrap();
    assert_eq!(plan.rows(), &[vec![], vec![], vec![0, 1]]);
    assert_eq!(
        plan.construction_substitution(2, &[Some(8), Some(3)])
            .unwrap(),
        vec![(0, 8), (1, 3)]
    );
    assert_eq!(
        plan.construction_substitution(2, &[None, Some(3)]),
        Err(TelescopeError::MissingWitness(0))
    );
    assert!(plan.construction_substitution(2, &[Some(8)]).is_err());
    assert!(
        plan.construction_substitution(2, &[Some(8), Some(3), Some(4)])
            .is_err()
    );
    for rows in [
        vec![vec![0]],
        vec![vec![1], vec![]],
        vec![vec![], vec![0, 0]],
        vec![vec![], vec![], vec![1, 0]],
    ] {
        assert!(RecordTelescope::new(rows).is_err());
    }
    assert!(RecordTelescope::new(vec![vec![]; 4097]).is_err());
    assert!(RecordTelescope::from_dependencies([vec![], vec![0; 16385]]).is_err());
    assert!(plan.selector(3).is_err());
    assert_eq!(plan.may_mutate(0), Ok(false));
    assert_eq!(plan.may_mutate(2), Ok(true));
}

fn numbers(row: &[usize]) -> String {
    format!(
        "[{}]",
        row.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn list(parts: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", parts.into_iter().collect::<Vec<_>>().join(", "))
}

// Decode the kernel term independently of the production selector. Only the
// declared receiver and the canonical Fst/Snd elimination chain are accepted.
fn projection_id(term: &CoreTerm) -> Option<(usize, usize)> {
    let (mut tail, first) = match term {
        CoreTerm::Fst(tail) => (&**tail, true),
        CoreTerm::Snd(_) => (term, false),
        _ => return None,
    };
    let mut count = 0;
    while let CoreTerm::Snd(inner) = tail {
        count += 1;
        tail = inner;
    }
    let CoreTerm::Var(name) = tail else {
        return None;
    };
    let receiver = match name.as_str() {
        "r" => 0,
        "s" => 1,
        _ => return None,
    };
    if first == (count < 2) && count <= 2 {
        Some((receiver, count))
    } else {
        None
    }
}
fn term(term: &CoreTerm) -> String {
    if let Some((receiver, field)) = projection_id(term) {
        return format!("(.project {receiver} {field})");
    }
    match term {
        CoreTerm::Universe(UniverseLevel::Concrete(0)) => "(.root 100)".into(),
        CoreTerm::Var(name) => match name.as_str() {
            "T" => "(.root 10)",
            "element" => "(.root 11)",
            "carrier" => "(.slot 0)",
            "point" => "(.slot 1)",
            _ => panic!("unexpected free variable {name}"),
        }
        .into(),
        CoreTerm::PathTy { carrier, lhs, rhs } => format!(
            "(.pair (.root 102) (.pair {} (.pair {} {})))",
            self::term(carrier),
            self::term(lhs),
            self::term(rhs)
        ),
        _ => panic!("unexpected kernel observation {term:?}"),
    }
}
fn witness_id(term: &CoreTerm) -> usize {
    match term {
        CoreTerm::Var(name) if name.as_str() == "T" => 10,
        CoreTerm::Var(name) if name.as_str() == "element" => 11,
        _ => panic!("unexpected construction witness {term:?}"),
    }
}

fn emit_specialization(
    output: &mut String,
    name: &str,
    bindings: &str,
    source: &CoreTerm,
    result: &CoreTerm,
) {
    let source = term(source);
    let result = term(result);
    writeln!(
        output,
        "theorem {name} : specialize {bindings} {source} = some {result} := by decide"
    )
    .unwrap();
    writeln!(output, "theorem {name}Graph : Substitutes {bindings} {source} {result} := (specialize_sound {name}).1\n#print axioms {name}Graph").unwrap();
}

fn observations() -> String {
    let ctx = environment();
    let axioms = AxiomRegistry::new();
    let inductives = InductiveRegistry::new();
    let source = fields();
    let record = SigmaRecord::form(&ctx, source.clone(), &axioms, &inductives).unwrap();
    let values = [
        var("T"),
        var("element"),
        CoreTerm::Refl(Heap::new(var("element"))),
    ];
    let introduction = record
        .construct(&ctx, &values, &axioms, &inductives)
        .unwrap();
    let ctx = ctx
        .extend("r".into(), record.schema().clone())
        .extend("s".into(), record.schema().clone());
    let mut output =
        String::from("\nnamespace KernelTelescopeReplay\nopen Verum.RecordTelescope\n");
    writeln!(
        output,
        "def rows : List (List Nat) := {}",
        list(record.plan().rows().iter().map(|r| numbers(r)))
    )
    .unwrap();
    output.push_str("example : form rows = true := by decide\n");
    for (i, field) in introduction.fields.iter().enumerate() {
        let prefix = list(
            values[..i]
                .iter()
                .map(|value| format!("some {}", witness_id(value))),
        );
        let actual = list(
            field
                .substitutions
                .iter()
                .map(|(slot, value)| format!("({slot}, {})", witness_id(value))),
        );
        writeln!(
            output,
            "example : construction rows {i} {prefix} = some {actual} := by decide"
        )
        .unwrap();
        let bindings = list(
            field
                .substitutions
                .iter()
                .map(|(slot, value)| format!("({slot}, {})", term(value))),
        );
        emit_specialization(
            &mut output,
            &format!("introduced{i}"),
            &bindings,
            &source[i].1,
            &field.ty,
        );
        let mutable = record.plan().may_mutate(i).unwrap();
        writeln!(
            output,
            "example : mayMutate rows {i} = some {mutable} := by decide"
        )
        .unwrap();
    }
    for (id, name) in ["r", "s"].into_iter().enumerate() {
        let receiver: Text = name.into();
        for (i, (_, source_ty)) in source.iter().enumerate() {
            let observed = record
                .project(&ctx, &receiver, i, &axioms, &inductives)
                .unwrap();
            assert_eq!(projection_id(&observed.term), Some((id, i)));
            let actual = list(observed.field.substitutions.iter().map(|(slot, value)| {
                let (r, j) = projection_id(value).expect("typed kernel field projection");
                format!("({slot}, ({r}, {j}))")
            }));
            writeln!(
                output,
                "example : projection rows {i} {id} = some {actual} := by decide"
            )
            .unwrap();
            let bindings = list(
                observed
                    .field
                    .substitutions
                    .iter()
                    .map(|(slot, value)| format!("({slot}, {})", term(value))),
            );
            emit_specialization(
                &mut output,
                &format!("projected{id}Field{i}"),
                &bindings,
                source_ty,
                &observed.field.ty,
            );
        }
    }
    let r = Text::from("r");
    let expected = record.project(&ctx, &r, 2, &axioms, &inductives).unwrap();
    for source_receiver in ["r", "s"] {
        let supplied = record
            .project(&ctx, &source_receiver.into(), 2, &axioms, &inductives)
            .unwrap();
        let accepted = record
            .check_mutation(&ctx, &r, 2, &supplied.term, &axioms, &inductives)
            .is_ok();
        writeln!(
            output,
            "example : decide (({} : FieldTerm) = {}) = {accepted} := by decide",
            term(&expected.field.ty),
            term(&supplied.field.ty)
        )
        .unwrap();
    }
    output.push_str("end KernelTelescopeReplay\n");
    output
}

#[test]
#[ignore = "requires pinned Lean; run explicitly with --ignored"]
fn lean_replays_checked_sigma_fields_and_rejects_changed_observations() {
    replay::check(
        &observations(),
        &[
            "introduced2Graph",
            "projected0Field2Graph",
            "projected1Field2Graph",
        ],
        &[
            (
                "construction rows 2 [some 10, some 11] = some [(0, 10), (1, 11)]",
                "construction rows 2 [some 10, some 11] = some [(0, 11), (1, 10)]",
            ),
            (
                "projection rows 2 0 = some [(0, (0, 0)), (1, (0, 1))]",
                "projection rows 2 0 = some [(0, (1, 0)), (1, (0, 1))]",
            ),
            (
                "mayMutate rows 0 = some false",
                "mayMutate rows 0 = some true",
            ),
            (
                "some (.project 0 0) := by decide",
                "some (.project 1 0) := by decide",
            ),
        ],
    );
}
