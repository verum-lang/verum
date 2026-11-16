//! Both-polarity e2e for the row-based arch inference (T0848):
//! the transitive escalation is CAUGHT, and the clean twin passes —
//! a checker satisfied by screaming at everything fails the twin.
//!
//! (Design §8 discipline, applied to the tool itself.)

use verum_compiler::arch_query::PinComparison;

const ESCALATING: &str = r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    lifecycle: Lifecycle.Definition,
    requires: [Capability.Read(ResourceTag.Logger)],
)
module fixtures.escalating;

fn leaf_dials_out() {
    core.net.tcp.connect("evil.example", 443);
}

fn middle() {
    leaf_dials_out();
}

public fn entry() {
    middle();
}
"#;

const INNOCENT: &str = r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    lifecycle: Lifecycle.Definition,
    requires: [Capability.Network(NetProtocol.Tcp, NetDirection.Outbound)],
)
module fixtures.innocent;

fn dial_helper() {
    core.net.tcp.connect("peer.example", 443);
}

public fn entry() {
    dial_helper();
}
"#;

/// The escalation is TRANSITIVE: entry → middle → leaf_dials_out.
/// The flat walk this inference replaced saw the Network atom only
/// on the leaf; the row solver must surface it on the module AND
/// report it as an escalation against the read-only pin.
#[test]
fn transitive_escalation_is_caught() {
    let report = verum_compiler::arch_query::arch_query_source(ESCALATING)
        .expect("fixture parses");
    assert!(
        report
            .inferred
            .iter()
            .any(|a| a.atom.contains("Network")),
        "the leaf's Network atom must reach the module surface \
         transitively; inferred = {:?}",
        report.inferred
    );
    assert_eq!(report.comparison, PinComparison::GroundDifference);
    let esc = report.escalations.expect("pin present ⟹ judgment runs");
    assert!(
        esc.iter().any(|a| a.atom.contains("Network")),
        "Network exceeds the read-only pin and must be reported"
    );
    // The per-function detail places the atom on ALL THREE frames.
    for f in ["leaf_dials_out", "middle", "entry"] {
        let s = report
            .functions
            .iter()
            .find(|x| x.function == f)
            .unwrap_or_else(|| panic!("summary for {f}"));
        assert!(
            s.atoms.iter().any(|a| a.atom.contains("Network")),
            "{f} must carry the Network atom through the summary join"
        );
    }
}

/// Clean twin: matching pin judges clean — no escalations, and the
/// judgment does not manufacture dead rights out of the exercised pin.
#[test]
fn innocent_twin_judges_clean() {
    let report = verum_compiler::arch_query::arch_query_source(INNOCENT)
        .expect("fixture parses");
    assert_eq!(report.comparison, PinComparison::GroundMatch);
    let esc = report.escalations.expect("pin present");
    let dead = report.dead_rights.expect("pin present");
    assert!(
        esc.is_empty(),
        "no escalation on the clean twin; got {esc:?}"
    );
    assert!(
        dead.is_empty(),
        "the pinned Network capability IS exercised; got {dead:?}"
    );
}

/// The unpinned module still answers: derived-only surface, no
/// judgment — the machine contract for unannotated code.
#[test]
fn unpinned_module_reports_derived_surface() {
    let src = r#"
module fixtures.plain;
fn f() { core.net.tcp.connect("h", 1); }
"#;
    let report =
        verum_compiler::arch_query::arch_query_source(src).expect("parses");
    assert_eq!(report.comparison, PinComparison::Unpinned);
    assert!(report.pinned.is_none());
    assert!(report.escalations.is_none());
    assert!(report.inferred.iter().any(|a| a.atom.contains("Network")));
}

/// §5b bounded polymorphism at the trait seam: a call through a
/// PROTOCOL-typed parameter with a declared @max_shape contributes
/// the CITED row and keeps the summary CLOSED — no row variable, no
/// silent widening, and the provenance names the protocol.
#[test]
fn protocol_max_shape_bounds_the_seam() {
    let src = r#"
module fixtures.seam;

@max_shape(requires: [Capability.Network(NetProtocol.Tcp, NetDirection.Outbound)])
type Dialer is protocol {
    fn dial(&self) -> Int;
};

public fn use_dialer(d: Dialer) -> Int {
    d(0)
}
"#;
    let report =
        verum_compiler::arch_query::arch_query_source(src).expect("parses");
    let f = report
        .functions
        .iter()
        .find(|f| f.function == "use_dialer")
        .expect("summary");
    assert!(
        f.open_over.is_empty(),
        "protocol-typed param must NOT open the row (bounded seam); got {:?}",
        f.open_over
    );
    assert!(
        f.atoms
            .iter()
            .any(|a| a.atom.contains("Network") && a.evidence.contains("Dialer")),
        "the seam contributes the protocol's CITED max-shape; got {:?}",
        f.atoms
    );
}

/// Unresolved MOUNTED callee (no session, single file) is SURFACED
/// as a qualified unresolved edge — never guessed, never dropped.
#[test]
fn mounted_callee_without_registry_is_surfaced() {
    let src = r#"
module fixtures.consumer;
mount fixtures.provider.{net_helper};

public fn entry() {
    net_helper();
}
"#;
    let report =
        verum_compiler::arch_query::arch_query_source(src).expect("parses");
    assert!(
        report
            .unresolved_calls
            .iter()
            .any(|e| e.contains("fixtures.provider.net_helper")),
        "the mounted callee must surface QUALIFIED in unresolved; got {:?}",
        report.unresolved_calls
    );
}

#[test]
fn unresolved_surface_cannot_establish_a_clean_pin_or_dead_right() {
    let source = r#"
@arch_module(requires: [Capability.Read(ResourceTag.Logger)])
module fixtures.partial;
mount fixtures.provider.{external};
public fn run() { external(); core.net.tcp.connect("host", 443); }
"#;
    let report = verum_compiler::arch_query::arch_query_source(source).unwrap();
    assert_eq!(report.comparison, PinComparison::Partial);
    assert!(!report.unresolved_calls.is_empty());
    assert!(report.dead_rights.is_none());
    assert!(report.escalations.unwrap().iter().any(|a| a.atom.contains("Network")));
}

/// Drive the actual parser, row extraction and kernel phase on one source.
/// This tests their declaration contract; it does not claim to typecheck or
/// execute the capability-relevant call used to exercise the extractor.
fn phase_for_source(source: &str) -> verum_kernel::arch_phase::ModuleArchResult {
    use verum_ast::decl::ItemKind;
    use verum_common::FileId;
    use verum_kernel::arch_phase::{PhaseInputs, run_arch_phase_one_with};

    let module = verum_fast_parser::FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("fixture parses");
    let attribute = module
        .attributes
        .iter()
        .chain(
            module
                .items
                .iter()
                .filter(|item| matches!(&item.kind, ItemKind::Module(_)))
                .flat_map(|item| item.attributes.iter()),
        )
        .find(|attr| attr.name.as_str() == "arch_module")
        .expect("module annotation");
    let args = attribute
        .args
        .as_ref()
        .map(|args| args.as_slice())
        .unwrap_or(&[]);
    let summaries = verum_compiler::pipeline::infer_summaries_for_query(&module);
    let inferred = summaries
        .module_surface()
        .facts()
        .map(|fact| fact.atom.clone())
        .collect();
    run_arch_phase_one_with(
        "fixture".into(),
        args,
        &PhaseInputs {
            inferred_used_capabilities: inferred,
            ..PhaseInputs::default()
        },
    )
}

fn capability_source(fields: &str, unresolved: bool) -> verum_common::Text {
    let call = if unresolved { "external();" } else { "" };
    format!(
        r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    stratum: MsfsStratum.LCls,
    lifecycle: Lifecycle.Definition,
    {fields}
)
module fixtures.constraint;
mount fixtures.provider.{{external}};
fn probe() {{ core.io.fs.read_file("x"); {call} }}
"#
    )
    .into()
}

/// The public parse entry calls `on_module_parsed`, which invokes the same
/// ATS phase used by check/build/run. No stdlib loading or execution is needed.
fn compiler_phase_diagnostics(
    source: &str,
) -> (anyhow::Result<()>, verum_common::List<verum_common::Text>) {
    use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("arch_presence_fixture.vr");
    let mut session = Session::new(CompilerOptions {
        input: path.clone(),
        ..CompilerOptions::default()
    });
    let file = session.load_source_string(source, path).unwrap();
    let parsed = {
        let mut pipeline = CompilationPipeline::new_check(&mut session);
        pipeline.phase_parse(file).map(|_| ())
    };
    let diagnostics = session
        .diagnostics()
        .iter()
        .map(|diagnostic| format!("{diagnostic:?}").into())
        .collect();
    (parsed, diagnostics)
}

#[test]
fn metadata_only_keeps_inference_without_inventing_a_pin() {
    use verum_kernel::arch::{Foundation, Lifecycle, MsfsStratum};
    let source = capability_source("", true);
    let report = verum_compiler::arch_query::arch_query_source(&source).unwrap();
    assert_eq!(report.comparison, PinComparison::Unpinned);
    assert!(report.pinned.is_none());
    assert!(report.pinned_exposes.is_none());
    assert!(report.escalations.is_none());
    assert!(report.dead_rights.is_none());
    assert_eq!(report.inferred.len(), 1);
    assert_eq!(report.inferred[0].evidence, "computed");
    assert!(
        report
            .unresolved_calls
            .iter()
            .any(|call| call.contains("fixtures.provider.external"))
    );

    let phase = phase_for_source(&source);
    assert!(phase.parse_errors.is_empty());
    assert!(phase.violations.is_empty(), "{:?}", phase.violations);
    let shape = phase.shape.unwrap();
    assert_eq!(shape.foundation, Foundation::ZfcTwoInacc);
    assert_eq!(shape.stratum, MsfsStratum::LCls);
    assert_eq!(shape.lifecycle, Lifecycle::Definition);
    assert!(shape.required_capability_constraint().is_none());
    let (parsed, diagnostics) = compiler_phase_diagnostics(&source);
    parsed.expect("metadata-only annotation permits parsing");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.contains("ATS-V-AP-001")),
        "{diagnostics:?}"
    );
}

#[test]
fn explicit_empty_matching_and_exposed_constraints_keep_both_polarities() {
    use verum_kernel::arch_anti_pattern::AntiPatternCode;
    let read = r#"Capability.Read(ResourceTag.File("<inferred>"))"#;
    for (fields, comparison, escalation, exposed) in [
        (
            "requires: [],".into(),
            PinComparison::GroundDifference,
            true,
            false,
        ),
        (
            format!("requires: [{read}],"),
            PinComparison::GroundMatch,
            false,
            false,
        ),
        ("exposes: [],".into(), PinComparison::Unpinned, false, true),
        (
            format!("requires: [], exposes: [{read}],"),
            PinComparison::GroundDifference,
            true,
            true,
        ),
        (
            format!("requires: [{read}], exposes: [],"),
            PinComparison::GroundMatch,
            false,
            true,
        ),
    ] {
        let source = capability_source(&fields, false);
        let report = verum_compiler::arch_query::arch_query_source(&source).unwrap();
        assert_eq!(report.comparison, comparison, "{fields}");
        assert_eq!(report.pinned_exposes.is_some(), exposed, "{fields}");
        assert_eq!(
            report
                .escalations
                .as_ref()
                .is_some_and(|atoms| !atoms.is_empty()),
            escalation,
            "{fields}"
        );
        if comparison == PinComparison::GroundMatch {
            assert!(report.dead_rights.as_ref().unwrap().is_empty());
        }
        let phase = phase_for_source(&source);
        assert!(phase.parse_errors.is_empty(), "{fields}");
        assert_eq!(
            phase
                .violations
                .iter()
                .any(|v| v.code == AntiPatternCode::CapabilityEscalation),
            escalation,
            "{fields}: {:?}",
            phase.violations
        );
        let (parsed, diagnostics) = compiler_phase_diagnostics(&source);
        parsed.expect("valid capability constraints permit parsing");
        assert_eq!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("ATS-V-AP-001")),
            escalation,
            "{fields}: {diagnostics:?}"
        );
    }
}

#[test]
fn invalid_constraint_is_an_error_in_query_and_phase() {
    let source = capability_source("requires: 7,", false);
    let error = verum_compiler::arch_query::arch_query_source(&source).unwrap_err();
    assert!(error.to_string().contains("requires"));
    let phase = phase_for_source(&source);
    assert_eq!(phase.parse_errors.len(), 1);
    let (parsed, diagnostics) = compiler_phase_diagnostics(&source);
    let error = parsed.expect_err("the actual compiler must reject malformed capability intent");
    assert!(error.to_string().contains("compilation failed"));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("@arch_module parse error")
                && diagnostic.contains("requires")),
        "{diagnostics:?}"
    );
}

#[test]
fn a_function_constraint_does_not_pin_the_enclosing_module() {
    let source = r#"
module fixtures.scoped;
@arch_module(requires: [])
fn pure_function() {}
fn other_function() { core.io.fs.read_file("x"); }
"#;
    let report = verum_compiler::arch_query::arch_query_source(source).unwrap();
    assert_eq!(report.comparison, PinComparison::Unpinned);
    assert!(report.pinned.is_none());
    assert_eq!(report.inferred.len(), 1);
}

#[test]
fn a_child_module_constraint_does_not_pin_the_enclosing_source() {
    let source = r#"
module fixtures.parent;
@arch_module(requires: [])
module child { fn pure_function() {} }
fn other_function() { core.io.fs.read_file("x"); }
"#;
    let report = verum_compiler::arch_query::arch_query_source(source).unwrap();
    assert_eq!(report.comparison, PinComparison::Unpinned);
    assert!(report.pinned.is_none());
    assert_eq!(report.inferred.len(), 1);
}

#[test]
fn verum_shape_mirror_retains_capability_field_presence() {
    use verum_ast::decl::{FunctionBody, ItemKind, TypeDeclBody};
    use verum_ast::{ExprKind, LiteralKind, TypeKind};
    use verum_common::FileId;
    let source = include_str!("../../../core/architecture/types.vr");
    let module = verum_fast_parser::FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("source mirror parses");
    let declaration = |name: &str| {
        module
            .items
            .iter()
            .find_map(|item| match &item.kind {
                ItemKind::Type(decl) if decl.name.name.as_str() == name => Some(decl),
                _ => None,
            })
            .expect("source mirror type")
    };
    let TypeDeclBody::Record(presence) = &declaration("CapabilityFieldPresence").body else {
        panic!("capability presence must be a record");
    };
    assert_eq!(presence.len(), 2);
    for name in ["requires", "exposes"] {
        let field = presence
            .iter()
            .find(|field| field.name.name.as_str() == name)
            .unwrap();
        assert!(matches!(&field.ty.kind, TypeKind::Bool));
    }
    let TypeDeclBody::Record(shape) = &declaration("ModuleShape").body else {
        panic!("ModuleShape must be a record");
    };
    assert!(
        shape
            .iter()
            .any(|field| field.name.name.as_str() == "capability_fields")
    );

    let constructor = module
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Function(function)
                if function.name.name.as_str() == "shape_default_for_unannotated" =>
            {
                Some(function)
            }
            _ => None,
        })
        .unwrap();
    let Some(FunctionBody::Block(body)) = &constructor.body else {
        panic!("default constructor body")
    };
    let ExprKind::Record { fields, .. } = &body.expr.as_ref().unwrap().kind else {
        panic!("default shape value")
    };
    let field = fields
        .iter()
        .find(|field| field.name.name.as_str() == "capability_fields")
        .unwrap();
    let ExprKind::Record { fields, .. } = &field.value.as_ref().unwrap().kind else {
        panic!("presence value")
    };
    assert_eq!(fields.len(), 2);
    for name in ["requires", "exposes"] {
        let value = fields
            .iter()
            .find(|field| field.name.name.as_str() == name)
            .unwrap()
            .value
            .as_ref()
            .unwrap();
        assert!(
            matches!(&value.kind, ExprKind::Literal(literal) if literal.kind == LiteralKind::Bool(false))
        );
    }
}
