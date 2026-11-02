//! Replay field types synthesized from parsed expressions under real lexical binders.
use std::fmt::Write;
use verum_ast::{
    Expr, ExprKind, FileId, Span,
    visitor::{Visitor, walk_expr},
};
use verum_common::{Text, record_telescope::RecordTelescope};
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker};

#[path = "../../../verification/external/lean/record_telescope_replay.rs"]
mod replay;

#[derive(Default)]
struct Uses(Vec<(Text, Span)>);

impl Visitor for Uses {
    fn visit_expr(&mut self, expr: &Expr) {
        if let ExprKind::Field {
            expr: receiver,
            field,
        } = &expr.kind
            && matches!(field.name.as_str(), "witness" | "truncation")
            && let ExprKind::Path(path) = &receiver.kind
        {
            self.0.push((path.last_segment_name().into(), expr.span));
        }
        walk_expr(self, expr);
    }
}

// Synthesize the parsed tuple while the closure's parameter scope is live.
// Its return type contains the actual field types produced at each expression.
fn project_source(checker: &mut TypeChecker, expression: &str) -> Vec<(Text, Type)> {
    let source = FastParser::new()
        .parse_expr_str(expression, FileId::new(18))
        .unwrap();
    let mut uses = Uses::default();
    uses.visit_expr(&source);
    let result = checker.synth_expr(&source).unwrap();
    let Type::Function { return_type, .. } = result.ty else {
        panic!("expected a synthesized closure")
    };
    let Type::Tuple(components) = *return_type else {
        panic!("expected a tuple of projected fields")
    };
    assert_eq!(uses.0.len(), components.len());
    uses.0
        .into_iter()
        .map(|(name, _)| name)
        .zip(components)
        .collect()
}

// This observation language covers the fixture's nominal family applied to
// one type-valued slot. Reject any other type shape before encoding it.
fn nominal(ty: &Type) -> (String, &[Type]) {
    let (key, args) = match ty {
        Type::Named { path, args } => (
            path.segments
                .iter()
                .map(|segment| match segment {
                    verum_ast::ty::PathSegment::Name(name) => name.name.as_str(),
                    _ => panic!("unresolved witness family path"),
                })
                .collect::<Vec<_>>()
                .join("."),
            args,
        ),
        Type::Generic { name, args } => (name.to_string(), args),
        other => panic!("expected the nominal witness family, got {other:?}"),
    };
    (key, args.as_slice())
}

fn family_argument(ty: &Type) -> (&Type, String) {
    let (key, args) = nominal(ty);
    assert!(
        key == "Witness" || key == "source.Witness",
        "unexpected family {key}"
    );
    assert_eq!(args.len(), 1);
    (&args[0], key)
}

#[test]
#[ignore = "requires pinned Lean; run explicitly with --ignored"]
fn lean_checks_the_real_hott_ntype_declaration_and_specialized_truncation_indices() {
    let parser = FastParser::new();
    // Preserve the actual parsed field, protocol signatures, generic kinds,
    // attributes, source spans, and visibility of the standard declarations.
    let mut module = parser
        .parse_module_str(include_str!("../../../core/math/hott.vr"), FileId::new(17))
        .unwrap();
    module.items.retain(|item| matches!(&item.kind, verum_ast::ItemKind::Type(decl) if matches!(decl.name.name.as_str(), "NType" | "IsTrunc")));
    assert_eq!(module.items.len(), 2);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("core.math.hott");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for item in &module.items {
        checker.check_item(item).unwrap();
    }
    let projected = project_source(
        &mut checker,
        "|x: &NType<0>, y: &NType<0>, z: &NType<1>| (x.truncation, y.truncation, z.truncation)",
    );
    let deferred = checker.take_deferred_errors();
    assert!(deferred.is_empty(), "{deferred:?}");
    assert!(
        !checker.diagnostics().iter().any(|d| d.is_error()),
        "{:?}",
        checker.diagnostics()
    );

    let Type::Record(fields) = checker
        .resolved_type("__struct_fields_core.math.hott.NType")
        .unwrap()
    else {
        panic!("missing NType fields")
    };
    let (truncation, _, declared) = fields.get_full(&Text::from("truncation")).unwrap();
    let (head, args) = nominal(declared);
    assert!(
        head == "IsTrunc" || head == "core.math.hott.IsTrunc",
        "{head}"
    );
    assert_eq!(args.len(), 2);
    let Type::RecordField {
        slot,
        receiver: None,
        level,
    } = &args[0]
    else {
        panic!("not an open NType carrier: {:?}", args[0])
    };
    assert_eq!(slot.owner.as_str(), "core.math.hott.NType");
    assert_eq!(
        fields.get_index(slot.ordinal).unwrap().0.as_str(),
        "carrier"
    );
    match &args[1] {
        Type::Meta {
            name, value: None, ..
        } => assert_eq!(name.as_str(), "N"),
        Type::Named { path, args } if args.is_empty() => assert_eq!(path.last_segment_name(), "N"),
        other => panic!("NType lost its declared index N: {other:?}"),
    }
    let plan = RecordTelescope::from_dependencies(
        fields
            .values()
            .map(|field| field.record_field_slots().into_iter().map(|s| s.ordinal)),
    )
    .unwrap();
    assert_eq!(projected.len(), 3);
    let mut claims =
        String::from("\nnamespace Verum.NTypeSourceEvidence\nopen Verum.RecordTelescope\n");
    writeln!(
        claims,
        "def rows : List (List Nat) := {:?}\nexample : form rows = true := by decide",
        plan.rows()
    )
    .unwrap();
    let mut receivers = Vec::new();
    let carrier = slot.ordinal;
    // The generic N uses a separate substitution key outside all field slots.
    let generic = fields.len();
    for (i, ((_, ty), expected_n)) in projected.iter().zip([0i128, 0, 1]).enumerate() {
        let (actual_head, actual) = nominal(ty);
        assert_eq!(actual_head, head);
        assert_eq!(actual.len(), 2);
        let Type::RecordField {
            slot: actual_slot,
            receiver: Some(root),
            level: actual_level,
        } = &actual[0]
        else {
            panic!("missing projected carrier: {ty:?}")
        };
        assert_eq!(actual_slot, slot);
        assert_eq!(actual_level, level);
        assert!(root.fields.is_empty());
        let Type::Meta {
            value: Some(verum_common::ConstValue::Int(n)),
            ..
        } = &actual[1]
        else {
            panic!("missing signed NType index: {ty:?}")
        };
        assert_eq!(*n, expected_n);
        let receiver = root.binding;
        receivers.push(receiver);
        writeln!(claims, "example : projection rows {truncation} {receiver} = some [({carrier}, ({receiver}, {}))] := by decide", actual_slot.ordinal).unwrap();
        writeln!(claims, "def args{i} : List (Nat × FieldTerm) := [({carrier}, .project {receiver} {carrier}), ({generic}, .root {expected_n})]").unwrap();
        let source = format!("(.pair (.root 70) (.pair (.slot {carrier}) (.slot {generic})))");
        let result = format!(
            "(.pair (.root 70) (.pair (.project {receiver} {}) (.root {n})))",
            actual_slot.ordinal
        );
        writeln!(claims, "theorem truncation{i} : specialize args{i} {source} = some {result} := by decide\ntheorem graph{i} : Substitutes args{i} {source} {result} := (specialize_sound truncation{i}).1\n#print axioms graph{i}").unwrap();
    }
    let (first, second) = (receivers[0], receivers[1]);
    writeln!(claims, "theorem receiverSeparation : ({first} : Nat) ≠ {second} := by decide\n#print axioms receiverSeparation\nend Verum.NTypeSourceEvidence").unwrap();
    let expected = format!("(.project {first} {carrier}) (.root 0))) := by decide");
    let swapped = format!("(.project {second} {carrier}) (.root 0))) := by decide");
    let wrong_index = format!("(.project {first} {carrier}) (.root 1))) := by decide");
    replay::check(
        &claims,
        &["graph0", "graph1", "graph2", "receiverSeparation"],
        &[(&expected, &swapped), (&expected, &wrong_index)],
    );
}

struct Observation {
    rows: Vec<Vec<usize>>,
    field: usize,
    dependency: usize,
    uses: Vec<(Text, u64, usize)>,
}

fn native() -> Observation {
    let module = FastParser::new().parse_module_str(
        "public type Witness<A> is { public value: A };\npublic type Pack is { public carrier: Type, public witness: Witness<carrier> };\nfn forward<T>(x: &T) -> &T { x }",
        FileId::new(1),
    ).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("source");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for item in &module.items {
        checker.check_item(item).unwrap();
    }
    let projected = project_source(
        &mut checker,
        "|x: &Pack, y: &Pack| { let a = forward(x); let b = a; (x.witness, y.witness, x.witness, b.witness) }",
    );
    assert!(checker.take_deferred_errors().is_empty());
    assert!(!checker.diagnostics().iter().any(|d| d.is_error()));
    let Type::Record(fields) = checker
        .resolved_type("__struct_fields_source.Pack")
        .unwrap()
    else {
        panic!("missing Pack field template")
    };
    let authority: Text = "source.Pack".into();
    let rows = fields.values().map(|ty| {
        ty.record_field_slots().into_iter().map(|slot| {
            assert_eq!(slot.owner, authority);
            slot.ordinal
        })
    });
    let plan = RecordTelescope::from_dependencies(rows).unwrap();
    let (field, _, declared) = fields.get_full(&Text::from("witness")).unwrap();
    let (declared_arg, family) = family_argument(declared);
    let Type::RecordField {
        slot,
        receiver: None,
        level,
    } = declared_arg
    else {
        panic!("expected bound field slot, found {declared_arg:?}")
    };
    let dependency = slot.ordinal;
    assert_eq!(fields.get_index(dependency).unwrap().0.as_str(), "carrier");
    let mut observations = Vec::new();
    for (root, ty) in projected {
        let (argument, actual_family) = family_argument(&ty);
        assert_eq!(family, actual_family);
        let Type::RecordField {
            slot: projected,
            receiver: Some(receiver),
            level: observed_level,
        } = argument
        else {
            panic!("unresolved projection in {ty:?}")
        };
        assert_eq!(&**projected, &**slot);
        assert_eq!(observed_level, level);
        assert!(
            receiver.fields.is_empty(),
            "fixture expects immediate local roots"
        );
        observations.push((root, receiver.binding, projected.ordinal));
    }
    Observation {
        rows: plan.rows().to_vec(),
        field,
        dependency,
        uses: observations,
    }
}

#[test]
fn source_projection_types_retain_binding_and_slot_identity() {
    let observation = native();
    assert_eq!(observation.rows, vec![vec![], vec![0]]);
    assert_eq!(
        observation
            .uses
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["x", "y", "x", "b"]
    );
    assert_eq!(observation.uses[0], observation.uses[2]);
    assert_eq!(observation.uses[0].1, observation.uses[3].1);
    assert_ne!(observation.uses[0].1, observation.uses[1].1);
}

#[test]
fn strict_kernel_agreement_rejects_erasure_wrong_arity_and_foreign_nominal_heads() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for (id, owner) in [(7, "left"), (8, "right")] {
        checker.set_current_module_path(owner);
        let module = FastParser::new()
            .parse_module_str(
                "public type Family<A, B> is { public fst: A, public snd: B };",
                FileId::new(id),
            )
            .unwrap();
        assert!(checker.prepare_module_declarations(&module).is_empty());
        for item in &module.items {
            checker.check_item(item).unwrap();
        }
    }
    let family = |owner: &str, args: Vec<Type>| Type::Generic {
        name: format!("{owner}.Family").into(),
        args: args.into(),
    };
    let expected = family("left", vec![Type::Int, Type::Bool]);
    let agrees = |actual: &Type, expected: &Type| {
        checker.check_record_family_agreement(actual, expected, Span::dummy())
    };
    agrees(&expected, &expected).unwrap();
    for invalid in [
        Type::Unknown,
        family("left", vec![]),
        family("left", vec![Type::Int]),
        family("left", vec![Type::Int, Type::Unknown]),
        family(
            "left",
            vec![Type::Int, Type::Var(verum_types::ty::TypeVar::fresh())],
        ),
        family("missing", vec![Type::Int, Type::Bool]),
    ] {
        // Identity of two malformed codes is not evidence that either is valid.
        assert!(
            agrees(&invalid, &invalid).is_err(),
            "accepted malformed {invalid:?}"
        );
        assert!(agrees(&invalid, &expected).is_err());
    }
    assert!(agrees(&family("right", vec![Type::Int, Type::Bool]), &expected).is_err());
    assert!(agrees(&family("left", vec![Type::Bool, Type::Int]), &expected).is_err());
    assert!(
        agrees(
            &Type::reference(true, Type::Int),
            &Type::reference(false, Type::Int)
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires pinned Lean; run explicitly with --ignored"]
fn lean_checks_source_record_projection_ownership() {
    let observation = native();
    let x = observation.uses[0].1;
    let y = observation.uses[1].1;
    let field = observation.field;
    let slot = observation.dependency;
    let mut claims =
        String::from("\nnamespace Verum.RecordSourceEvidence\nopen Verum.RecordTelescope\n");
    writeln!(
        claims,
        "def rows : List (List Nat) := {:?}",
        observation.rows
    )
    .unwrap();
    writeln!(claims, "theorem differentReceivers : ({x} : Nat) ≠ {y} := by decide\n#print axioms differentReceivers").unwrap();
    for (i, (name, observed_receiver, observed_slot)) in observation.uses.iter().enumerate() {
        let receiver = match name.as_str() {
            "x" | "b" => x,
            "y" => y,
            _ => panic!("unexpected source root"),
        };
        writeln!(claims, "example : projection rows {field} {receiver} = some [({slot}, ({observed_receiver}, {observed_slot}))] := by decide").unwrap();
        // 7 uniquely denotes the one validated nominal head in this fixture.
        writeln!(
            claims,
            "def bounds{i} : List (Nat × FieldTerm) := [({slot}, .project {receiver} {slot})]"
        )
        .unwrap();
        let source = format!("(.pair (.root 7) (.slot {slot}))");
        let result = format!("(.pair (.root 7) (.project {observed_receiver} {observed_slot}))");
        writeln!(
            claims,
            "theorem field{i} : specialize bounds{i} {source} = some {result} := by decide"
        )
        .unwrap();
        writeln!(claims, "theorem graph{i} : Substitutes bounds{i} {source} {result} := (specialize_sound field{i}).1\n#print axioms graph{i}").unwrap();
    }
    claims.push_str("end Verum.RecordSourceEvidence\n");
    let expected = format!("projection rows {field} {x} = some [({slot}, ({x}, {slot}))]");
    let foreign = format!("projection rows {field} {x} = some [({slot}, ({y}, {slot}))]");
    let original = format!("(.pair (.root 7) (.project {x} {slot})) := by decide");
    let changed = format!("(.pair (.root 7) (.project {x} {})) := by decide", slot + 1);
    replay::check(
        &claims,
        &["differentReceivers", "graph0", "graph1", "graph2", "graph3"],
        &[(&expected, &foreign), (&original, &changed)],
    );
}
