//! Capability ontology — primitive-call → Capability resolver.
//!
//! ## Architectural role
//!
//! ATS-V's AP-001 CapabilityEscalation requires knowing which
//! capabilities a cog actually exercises in its body, not just
//! which capabilities its `@arch_module(requires = [...])`
//! declaration claims.  The compiler walks the cog's AST,
//! resolves every primitive call site against the canonical
//! ontology shipped here, and aggregates the result into
//! `PhaseInputs.inferred_used_capabilities`.
//!
//! ## What this module provides
//!
//! 1. [`OntologyEntry`] — one row of the canonical map: path
//!    (`core.io.fs.read_file`) → `Capability` factory closure.
//! 2. [`canonical_ontology`] — the static roster of recognised
//!    primitive paths.  Production-grade entries cover I/O,
//!    network, system, randomness, observability primitives.
//! 3. [`lookup_capability`] — single-call resolver.
//! 4. [`lookup_capability_by_segments`] — convenience for
//!    AST callers that already split the path into segments.
//!
//! ## Architectural pin
//!
//! The roster is the SINGLE source of truth for capability
//! attribution.  Adding a new capability-relevant primitive to
//! `core/` requires extending this roster AND adding a matching
//! `core.architecture.capability_ontology` registry entry.  The
//! cross-side pin
//! `pin_capability_inference_ontology_present` enforces both
//! sides agree.

use crate::arch::{Capability, ExecTarget, NetDirection, NetProtocol, PrivilegeRealm, ResourceTag};

/// One canonical mapping from a primitive function path to the
/// `Capability` it implies.  The factory pattern (`fn() ->
/// Capability`) lets each entry build its capability with the
/// appropriate `ResourceTag` / `NetProtocol` / etc. without
/// hard-coding a fully-resolved instance at registry load time.
#[derive(Clone, Copy)]
pub struct OntologyEntry {
    /// Fully-qualified function path (e.g. `core.io.fs.read_file`).
    pub path: &'static str,
    /// Constructs the implied capability for this primitive.
    pub capability_factory: fn() -> Capability,
}

/// Canonical capability-inference ontology.  Each entry maps
/// a primitive function path to its implied capability.
///
/// ## Coverage
///
///   * **File I/O** (`core.io.fs.*`): Read / Write resource tags.
///   * **Network** (`core.net.{tcp, udp, http, http2, http3, quic}.*`):
///     Network protocol + inbound/outbound direction.
///   * **System** (`core.sys.process.*`, `core.sys.exec.*`):
///     Spawn lifetime, Exec target.
///   * **Randomness** (`core.security.random.*`,
///     `core.math.random.*`): Read from `Random` resource.
///   * **Observability** (`core.tracing.*`, `core.metrics.*`,
///     `core.diagnostics.*`): `Custom("logger")` /
///     `Custom("metrics")` / `Custom("tracing")`.
///   * **Privilege escalation** (`core.security.escalate.*`):
///     `Escalate` to the named realm.
///   * **Persistence** (`core.database.*.commit`,
///     `core.io.fs.fsync`): `Persist` to medium.
///
/// Adding a new entry requires:
///   1. A primitive function shipped in `core/` whose runtime
///      effect maps cleanly to one of the eight Capability arms.
///   2. A matching Verum-side registry entry in
///      `core/architecture/capability_ontology.vr`
///      (`ATS_V_CANONICAL_CAPABILITIES` for `Custom` capabilities,
///      or built-in arms for the kind-typed surface).
///   3. Cross-side pin extension so CI catches any drift.
pub fn canonical_ontology() -> Vec<OntologyEntry> {
    vec![
        // ----- File I/O ----------------------------------------------------
        OntologyEntry {
            path: "core.io.fs.read_file",
            capability_factory: || Capability::Read {
                resource: ResourceTag::File {
                    path_pattern: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.io.fs.read_to_string",
            capability_factory: || Capability::Read {
                resource: ResourceTag::File {
                    path_pattern: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.io.fs.write_file",
            capability_factory: || Capability::Write {
                resource: ResourceTag::File {
                    path_pattern: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.io.fs.append_file",
            capability_factory: || Capability::Write {
                resource: ResourceTag::File {
                    path_pattern: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.io.fs.fsync",
            capability_factory: || Capability::Persist {
                medium: crate::arch::PersistenceMedium::Disk {
                    path: "<inferred>".into(),
                },
            },
        },
        // ----- Network ----------------------------------------------------
        OntologyEntry {
            path: "core.net.tcp.connect",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Tcp,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.tcp.listen",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Tcp,
                direction: NetDirection::Inbound,
            },
        },
        OntologyEntry {
            path: "core.net.udp.bind",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Udp,
                direction: NetDirection::Bidirectional,
            },
        },
        OntologyEntry {
            path: "core.net.http.get",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.http.post",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.http.put",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.http.delete",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.http2.request",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.quic.connect",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Quic,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.tls.connect",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Tls,
                direction: NetDirection::Outbound,
            },
        },
        OntologyEntry {
            path: "core.net.unix.connect",
            capability_factory: || Capability::Network {
                protocol: NetProtocol::Unix,
                direction: NetDirection::Bidirectional,
            },
        },
        // ----- System / Process ------------------------------------------
        OntologyEntry {
            path: "core.sys.process.spawn",
            capability_factory: || Capability::Spawn {
                lifetime: crate::arch::TaskLifetime::ScopedToParent,
            },
        },
        OntologyEntry {
            path: "core.shell.exec",
            capability_factory: || Capability::Exec {
                target: ExecTarget::Program {
                    path: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.shell.spawn",
            capability_factory: || Capability::Exec {
                target: ExecTarget::Program {
                    path: "<inferred>".into(),
                },
            },
        },
        OntologyEntry {
            path: "core.security.escalate.to_admin",
            capability_factory: || Capability::Escalate {
                realm: PrivilegeRealm::Admin,
            },
        },
        OntologyEntry {
            path: "core.security.escalate.to_root",
            capability_factory: || Capability::Escalate {
                realm: PrivilegeRealm::Root,
            },
        },
        // ----- Randomness ------------------------------------------------
        OntologyEntry {
            path: "core.security.random.bytes",
            capability_factory: || Capability::Read {
                resource: ResourceTag::Random,
            },
        },
        OntologyEntry {
            path: "core.security.random.u64",
            capability_factory: || Capability::Read {
                resource: ResourceTag::Random,
            },
        },
        OntologyEntry {
            path: "core.math.random.next",
            capability_factory: || Capability::Read {
                resource: ResourceTag::Random,
            },
        },
        // ----- Observability ---------------------------------------------
        OntologyEntry {
            path: "core.tracing.span",
            capability_factory: || Capability::Custom {
                tag: "tracing".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Distributed-tracing span emitter".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        OntologyEntry {
            path: "core.tracing.event",
            capability_factory: || Capability::Custom {
                tag: "tracing".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Distributed-tracing event emitter".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        OntologyEntry {
            path: "core.metrics.counter",
            capability_factory: || Capability::Custom {
                tag: "metrics".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Metrics counter emitter".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        OntologyEntry {
            path: "core.metrics.gauge",
            capability_factory: || Capability::Custom {
                tag: "metrics".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Metrics gauge emitter".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        OntologyEntry {
            path: "core.metrics.histogram",
            capability_factory: || Capability::Custom {
                tag: "metrics".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Metrics histogram emitter".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        OntologyEntry {
            path: "core.diagnostics.log",
            capability_factory: || Capability::Custom {
                tag: "logger".into(),
                schema: crate::arch::CapabilitySchema {
                    description: "Structured logging sink".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        },
        // ----- Database / persistence -----------------------------------
        OntologyEntry {
            path: "core.database.commit",
            capability_factory: || Capability::Persist {
                medium: crate::arch::PersistenceMedium::Database {
                    connection_tag: "<inferred>".into(),
                },
            },
        },
        // ----- Time-bound rights ----------------------------------------
        OntologyEntry {
            path: "core.security.token.issue_with_ttl",
            capability_factory: || Capability::TimeBound {
                until: crate::arch::ExpirationPolicy::AfterDuration {
                    milliseconds: 0,
                },
            },
        },
    ]
}

/// Look up the capability implied by a primitive call path.
/// Returns `None` for paths not in the canonical ontology
/// (the call is treated as capability-neutral).
pub fn lookup_capability(path: &str) -> Option<Capability> {
    canonical_ontology()
        .into_iter()
        .find(|e| e.path == path)
        .map(|e| (e.capability_factory)())
}

/// Look up by pre-split path segments.  AST callers already have
/// the path as a `&[&str]`; joining once at the call site is
/// cheaper than re-allocating a `String` per lookup.
pub fn lookup_capability_by_segments(segments: &[&str]) -> Option<Capability> {
    let path = segments.join(".");
    lookup_capability(&path)
}

/// Stable count of canonical entries.  Pin tests assert the
/// roster size so accidental deletion fails CI.
pub fn ontology_size() -> usize {
    canonical_ontology().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ontology_is_non_empty() {
        assert!(ontology_size() >= 30);
    }

    #[test]
    fn fs_read_file_resolves_to_read_capability() {
        let cap = lookup_capability("core.io.fs.read_file").unwrap();
        match cap {
            Capability::Read {
                resource: ResourceTag::File { .. },
            } => {}
            other => panic!("expected Read{{File}}, got {:?}", other),
        }
    }

    #[test]
    fn http_get_resolves_to_outbound_http() {
        let cap = lookup_capability("core.net.http.get").unwrap();
        match cap {
            Capability::Network {
                protocol: NetProtocol::Http,
                direction: NetDirection::Outbound,
            } => {}
            other => panic!("expected Network{{Http, Outbound}}, got {:?}", other),
        }
    }

    #[test]
    fn unknown_path_returns_none() {
        assert!(lookup_capability("core.unknown.fn").is_none());
        assert!(lookup_capability("not.a.real.path").is_none());
    }

    #[test]
    fn segments_lookup_matches_string_lookup() {
        let by_str = lookup_capability("core.net.tcp.connect");
        let by_seg = lookup_capability_by_segments(&["core", "net", "tcp", "connect"]);
        assert_eq!(by_str.is_some(), by_seg.is_some());
    }

    #[test]
    fn ontology_paths_are_unique() {
        let entries = canonical_ontology();
        let mut paths: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for e in &entries {
            assert!(
                paths.insert(e.path),
                "duplicate ontology path: {}",
                e.path
            );
        }
    }

    #[test]
    fn metrics_paths_resolve_to_custom_metrics_tag() {
        for path in &[
            "core.metrics.counter",
            "core.metrics.gauge",
            "core.metrics.histogram",
        ] {
            let cap = lookup_capability(path).expect(path);
            match cap {
                Capability::Custom { tag, .. } => assert_eq!(tag, "metrics"),
                other => panic!("expected Custom{{tag:metrics}}, got {:?}", other),
            }
        }
    }
}
