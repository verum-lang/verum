//! Both-polarity e2e for the row-based arch inference (T0848):
//! the transitive escalation is CAUGHT, and the clean twin passes —
//! a checker satisfied by screaming at everything fails the twin.
//!
//! (Design §8 discipline, applied to the tool itself.)

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
