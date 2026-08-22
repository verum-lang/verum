//! The round-trip law of the capability vocabulary: a rendered
//! capability, written into an `@arch_module` pin, parses back to the
//! SAME capability — `parse(render(cap)) == cap` at the spelling
//! level. Display is injective over the vocabulary, so string
//! equality of the re-rendered pin IS the law.
//!
//! This is the pin against the T0834 class: a parser that stamps
//! placeholders (the pre-fix parser turned `Http2` into `Tcp`,
//! `Deadlined(500)` into `Detached`, `Syscall(93)` into `Syscall(0)`,
//! and dropped every FFI symbol) breaks one of these controls
//! immediately, because the judgment would compare against fiction.

use verum_kernel::arch::{
    Capability, ExecTarget, ExpirationPolicy, NetDirection, NetProtocol,
    PersistenceMedium, PrivilegeRealm, ResourceTag, TaskLifetime,
};

/// Every renderable capability form, payloads included. Extending the
/// vocabulary means extending THIS list — the test fails honest when a
/// new variant lacks a round-trip.
fn vocabulary() -> Vec<Capability> {
    vec![
        Capability::Read {
            resource: ResourceTag::Database {
                name: "ledger".to_string(),
            },
        },
        Capability::Read {
            resource: ResourceTag::File {
                path_pattern: "/tmp/*".to_string(),
            },
        },
        Capability::Write {
            resource: ResourceTag::Memory {
                region: "cache".to_string(),
            },
        },
        Capability::Write {
            resource: ResourceTag::Config {
                namespace: "app.net".to_string(),
            },
        },
        Capability::Read {
            resource: ResourceTag::Logger,
        },
        Capability::Read {
            resource: ResourceTag::Random,
        },
        Capability::Write {
            resource: ResourceTag::Custom("gpu-doorbell".to_string()),
        },
        Capability::Exec {
            target: ExecTarget::Ffi {
                library: "libz".to_string(),
                symbol: "inflate".to_string(),
            },
        },
        Capability::Exec {
            target: ExecTarget::Syscall { number: 93 },
        },
        Capability::Exec {
            target: ExecTarget::Program {
                path: "/bin/sh".to_string(),
            },
        },
        Capability::Escalate {
            realm: PrivilegeRealm::Root,
        },
        Capability::Escalate {
            realm: PrivilegeRealm::Custom("hypervisor".to_string()),
        },
        Capability::Spawn {
            lifetime: TaskLifetime::ScopedToParent,
        },
        Capability::Spawn {
            lifetime: TaskLifetime::Deadlined { milliseconds: 500 },
        },
        Capability::TimeBound {
            until: ExpirationPolicy::AfterDuration { milliseconds: 1000 },
        },
        Capability::TimeBound {
            until: ExpirationPolicy::OnEvent {
                event_tag: "shutdown".to_string(),
            },
        },
        Capability::Persist {
            medium: PersistenceMedium::Disk {
                path: "/var/db".to_string(),
            },
        },
        Capability::Persist {
            medium: PersistenceMedium::DistributedLog {
                topic: "audit".to_string(),
            },
        },
        // The exact spellings the old parser silently rewrote.
        Capability::Network {
            protocol: NetProtocol::Http2,
            direction: NetDirection::Outbound,
        },
        Capability::Network {
            protocol: NetProtocol::Grpc,
            direction: NetDirection::Inbound,
        },
        Capability::Network {
            protocol: NetProtocol::Tcp,
            direction: NetDirection::Bidirectional,
        },
    ]
}

fn pin_module_source(caps: &[String]) -> String {
    format!(
        "@arch_module(\n    requires: [{}],\n)\nmodule fixtures.roundtrip;\n\nfn noop() -> Int {{ 0 }}\n",
        caps.join(", ")
    )
}

#[test]
fn every_vocabulary_capability_round_trips_through_a_pin() {
    let rendered: Vec<String> =
        vocabulary().iter().map(|c| c.to_string()).collect();
    let source = pin_module_source(&rendered);
    let report = verum_compiler::arch_query::arch_query_source(&source)
        .expect("pin module parses");
    let parsed = report.pinned.expect("pin present");

    let want: std::collections::BTreeSet<&str> =
        rendered.iter().map(String::as_str).collect();
    let got: std::collections::BTreeSet<&str> =
        parsed.iter().map(String::as_str).collect();
    assert_eq!(
        want, got,
        "parse∘render must be the identity over the vocabulary; \
         a difference means the parser stamped a placeholder or the \
         renderer drifted from the pin syntax"
    );
}

/// A pin OUTSIDE the vocabulary must surface verbatim — arguments
/// included — as a Custom tag, never lose its payload, and never
/// collapse into a stamped builtin.
#[test]
fn unknown_pin_spelling_is_preserved_whole() {
    let source = pin_module_source(&[
        "Capability.FileRead(\"/tmp/*\")".to_string()
    ]);
    let report = verum_compiler::arch_query::arch_query_source(&source)
        .expect("pin module parses");
    let parsed = report.pinned.expect("pin present");
    assert_eq!(
        parsed,
        vec![r#"Capability.Custom("Capability.FileRead(\"/tmp/*\")")"#
            .to_string()],
        "the fallback must keep the WHOLE spelling — a dropped \
         argument is a lost right"
    );
}

/// Explicit `Capability.Custom("tag")` round-trips with the tag alone
/// (not wrapped into `Custom("Capability.Custom(...)")`).
#[test]
fn explicit_custom_round_trips_by_tag() {
    let source =
        pin_module_source(&["Capability.Custom(\"gpu-submit\")".to_string()]);
    let report = verum_compiler::arch_query::arch_query_source(&source)
        .expect("pin module parses");
    let parsed = report.pinned.expect("pin present");
    assert_eq!(parsed, vec!["Capability.Custom(\"gpu-submit\")".to_string()]);
}

/// Dotted calls the ontology does not know are SURFACED, not
/// swallowed: a stdlib-rooted call and a mount-expanded call land in
/// `unresolved_calls`; a value-method call (`x.push(1)`) does not —
/// it is typed dispatch at the carry(T) seam, not a module edge.
#[test]
fn dotted_calls_surface_and_value_methods_do_not() {
    let source = r#"
module fixtures.dotted;

mount core.fs;

fn touches_fs() {
    fs.open("/etc/passwd");
}

fn touches_stdlib_directly() {
    core.env.args();
}

fn value_method_only(x: List<Int>) -> Int {
    x.push(1);
    0
}
"#;
    let report = verum_compiler::arch_query::arch_query_source(source)
        .expect("fixture parses");
    let unresolved = report.unresolved_calls.join("\n");
    assert!(
        unresolved.contains("touches_fs -> core.fs.open"),
        "mount-expanded dotted callee must surface; got:\n{unresolved}"
    );
    assert!(
        unresolved.contains("touches_stdlib_directly -> core.env.args"),
        "stdlib-rooted dotted callee must surface; got:\n{unresolved}"
    );
    assert!(
        !unresolved.contains("x.push"),
        "a value-method call is not a module edge; got:\n{unresolved}"
    );
}

/// A qualified LOCAL static call (`Point.origin()`) is a call-graph
/// edge to the same summary the bare name carries: the callee's atom
/// reaches the caller transitively through the qualified spelling.
#[test]
fn qualified_local_static_call_is_a_graph_edge() {
    let source = r#"
module fixtures.qualified_local;

type Point is { x: Int, y: Int };

implement Point {
    fn origin() -> Point {
        core.net.tcp.connect("telemetry.example", 443);
        Point { x: 0, y: 0 }
    }
}

public fn entry() -> Point {
    Point.origin()
}
"#;
    let report = verum_compiler::arch_query::arch_query_source(source)
        .expect("fixture parses");
    let entry = report
        .functions
        .iter()
        .find(|f| f.function == "entry")
        .expect("entry summary");
    assert!(
        entry.atoms.iter().any(|a| a.atom.contains("Network")),
        "the impl method's Network atom must reach `entry` through \
         the qualified call edge; entry atoms = {:?}, unresolved = {:?}",
        entry.atoms,
        report.unresolved_calls
    );
}
