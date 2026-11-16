//! T2293: authored absence and authored emptiness are different constraints.

use verum_ast::expr::{ArrayExpr, Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{Heap, List};
use verum_kernel::arch::{Capability, Foundation, Lifecycle, MsfsStratum, ResourceTag, Shape};
use verum_kernel::arch_anti_pattern::{AntiPatternCode, check_capability_escalation};
use verum_kernel::arch_parse::parse_arch_module;
use verum_kernel::arch_phase::{PhaseInputs, run_arch_phase_one_with};

fn path(parts: &[&str]) -> Expr {
    let span = Span::dummy();
    Expr::new(
        ExprKind::Path(Path::new(
            parts
                .iter()
                .map(|part| PathSegment::Name(Ident::new(*part, span)))
                .collect(),
            span,
        )),
        span,
    )
}

fn named(name: &str, value: Expr) -> Expr {
    Expr::new(
        ExprKind::NamedArg {
            name: Ident::new(name, Span::dummy()),
            value: Heap::new(value),
        },
        Span::dummy(),
    )
}

fn empty_list(name: &str) -> Expr {
    named(
        name,
        Expr::new(ExprKind::Array(ArrayExpr::List(List::new())), Span::dummy()),
    )
}

fn metadata() -> List<Expr> {
    [
        named("foundation", path(&["Foundation", "ZfcTwoInacc"])),
        named("stratum", path(&["MsfsStratum", "LCls"])),
        named("lifecycle", path(&["Lifecycle", "Definition"])),
    ]
    .into_iter()
    .collect()
}

fn read_logger() -> Capability {
    Capability::Read {
        resource: ResourceTag::Logger,
    }
}

#[test]
fn metadata_preserves_values_without_inventing_capability_constraints() {
    let args = metadata();
    let shape = parse_arch_module(&args).unwrap();
    assert_eq!(shape.foundation, Foundation::ZfcTwoInacc);
    assert_eq!(shape.stratum, MsfsStratum::LCls);
    assert_eq!(shape.lifecycle, Lifecycle::Definition);
    assert!(shape.required_capability_constraint().is_none());
    assert!(shape.exposed_capability_constraint().is_none());

    let result = run_arch_phase_one_with(
        "fixture".into(),
        &args,
        &PhaseInputs {
            inferred_used_capabilities: [read_logger()].into(),
            ..PhaseInputs::default()
        },
    );
    assert!(result.parse_errors.is_empty());
    assert!(result.violations.is_empty(), "{:?}", result.violations);
}

#[test]
fn explicit_empty_requirement_rejects_known_usage() {
    let mut args = metadata();
    args.push(empty_list("requires"));
    let result = run_arch_phase_one_with(
        "fixture".into(),
        &args,
        &PhaseInputs {
            inferred_used_capabilities: [read_logger()].into(),
            ..PhaseInputs::default()
        },
    );
    let shape = result.shape.as_ref().unwrap();
    assert_eq!(shape.required_capability_constraint(), Some([].as_slice()));
    assert!(shape.exposed_capability_constraint().is_none());
    assert_eq!(result.violations.len(), 1);
    assert_eq!(
        result.violations[0].code,
        AntiPatternCode::CapabilityEscalation
    );
}

#[test]
fn explicit_empty_exposure_does_not_pin_requirements() {
    let args = [empty_list("exposes")];
    let shape = parse_arch_module(&args).unwrap();
    assert!(shape.required_capability_constraint().is_none());
    assert_eq!(shape.exposed_capability_constraint(), Some([].as_slice()));
    assert!(check_capability_escalation(&shape, &[read_logger()]).is_none());
}

#[test]
fn matching_requirement_passes_but_exposure_does_not_grant_usage() {
    let cap = Expr::new(
        ExprKind::Call {
            func: Heap::new(path(&["Capability", "Read"])),
            type_args: List::new(),
            args: [path(&["ResourceTag", "Logger"])].into_iter().collect(),
        },
        Span::dummy(),
    );
    let list = Expr::new(
        ExprKind::Array(ArrayExpr::List([cap].into_iter().collect())),
        Span::dummy(),
    );
    let matching = parse_arch_module(&[named("requires", list.clone())]).unwrap();
    assert!(check_capability_escalation(&matching, &[read_logger()]).is_none());
    let exposed = parse_arch_module(&[empty_list("requires"), named("exposes", list)]).unwrap();
    assert!(check_capability_escalation(&exposed, &[read_logger()]).is_some());
}

#[test]
fn serialization_preserves_independent_presence_and_rejects_missing_provenance() {
    for requires in [false, true] {
        for exposes in [false, true] {
            let mut args = metadata();
            if requires {
                args.push(empty_list("requires"));
            }
            if exposes {
                args.push(empty_list("exposes"));
            }
            let shape = parse_arch_module(&args).unwrap();
            let serialized = serde_json::to_value(&shape).unwrap();
            let restored: Shape = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(restored.capability_fields, shape.capability_fields);
            assert_eq!(
                restored.required_capability_constraint().is_some(),
                requires
            );
            assert_eq!(restored.exposed_capability_constraint().is_some(), exposes);

            let mut missing = serialized;
            missing.as_object_mut().unwrap().remove("capability_fields");
            assert!(serde_json::from_value::<Shape>(missing).is_err());
        }
    }
}

#[test]
fn a_computed_shape_does_not_author_a_requirement() {
    let mut inferred = Shape::default_for_unannotated();
    inferred.requires = [read_logger()].into();
    assert!(inferred.required_capability_constraint().is_none());
    let composed = verum_kernel::arch_composition::compose(
        &Shape::default_for_unannotated(),
        &Shape::default_for_unannotated(),
    )
    .unwrap_composed();
    assert!(composed.required_capability_constraint().is_none());
    assert!(composed.exposed_capability_constraint().is_none());
}
