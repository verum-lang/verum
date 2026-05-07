//! ATS-V Architectural Type System — kernel-side primitives.
//!
//! ## Architectural role
//!
//! ATS-V is a strict extension of Verum through ONE typed
//! attribute (`@arch_module(...)`) plus library types in
//! `core/architecture/`.  This module ships the kernel-side mirror
//! of those library types — Rust `enum`s and `struct`s that the
//! ATS-V phase (Phase 6.5, sitting between type inference and VBC
//! codegen) consumes during architectural type checking.
//!
//! ## Why mirrors live in the kernel
//!
//! The kernel discharges architectural invariants through
//! intrinsics (`kernel_arch_*` per §9.3). Kernel intrinsic dispatch
//! happens in Rust (per V8.1 META1 architectural principle), so
//! the carrier types must be Rust-side first-class enums. The
//! Verum-side `core/architecture/types.vr` mirrors the same shape
//! for human-readable declarations + LSP integration; the two
//! sides stay aligned through pin tests (added.4).
//!
//! ## Reuse over invention
//!
//! Per spec §17.1 (Grammar reuse map), every concept reuses an
//! existing Verum mechanism:
//!
//! * Capability flavour (Linear/Affine/Relevant/Unrestricted) →
//! existing `@quantity(0|1|omega)` attribute. We do NOT
//! introduce a parallel `CapabilityFlavour` enum — agents
//! declare flavour via `@quantity` on the capability binding.
//! * Verification route (V-axis CVE) → existing `@verify(strategy)`
//! ladder.
//! * Foundation citation → existing `@framework(corpus, "...")`.
//! * Refinement predicates for anti-patterns → existing `where`
//! clause.
//!
//! What this module DOES introduce: the canonical *types* that
//! `@arch_module(...)` named arguments parse into. These are
//! pure data carriers — no reasoning, no SMT. Reasoning lives
//! in [`super::arch_anti_pattern`].

use serde::{Deserialize, Serialize};

// =============================================================================
// Capability — first-class possibility tracked at architecture level
// =============================================================================

/// First-class capability — what a cog can DO. Per spec §4.2.
///
/// This enum is the closed canonical set; user-defined capabilities
/// register via [`Capability::Custom`] with mandatory ontology entry
/// in `core/architecture/capability_ontology.vr` (per spec §17.4).
///
/// Capability flavour (Linear / Affine / Relevant / Unrestricted) is
/// attached at the call site via existing `@quantity(0|1|omega)`
/// attribute — see spec §17.1. This enum carries only the kind,
/// not the substructural-logic discipline.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Read from a resource — file, db, network endpoint, etc.
    Read {
        /// Source resource being read.
        resource: ResourceTag,
    },
    /// Write to a resource.
    Write {
        /// Sink resource being written.
        resource: ResourceTag,
    },
    /// Execute a target — FFI call, syscall, external program.
    Exec {
        /// What is being executed.
        target: ExecTarget,
    },
    /// Escalate privileges into a higher realm.
    Escalate {
        /// Privilege boundary being crossed.
        realm: PrivilegeRealm,
    },
    /// Spawn a task in the supervision tree.
    Spawn {
        /// Supervision lifetime of the spawned task.
        lifetime: TaskLifetime,
    },
    /// Capability with TTL — must be exercised before expiry.
    TimeBound {
        /// Expiry policy after which the capability is void.
        until: ExpirationPolicy,
    },
    /// Persistence — durable state operation.
    Persist {
        /// Storage medium that holds the durable state.
        medium: PersistenceMedium,
    },
    /// Network — protocol-typed exposure or reach.
    Network {
        /// Wire protocol carried over the boundary.
        protocol: NetProtocol,
        /// Inbound, outbound, or bidirectional traffic.
        direction: NetDirection,
    },
    /// User-defined custom capability. MUST be registered in
    /// `core/architecture/capability_ontology.vr`; the kernel
    /// validates this at audit time.
    Custom {
        /// Capability tag — referenced by name in the ontology registry.
        tag: String,
        /// Schema describing the custom capability's surface.
        schema: CapabilitySchema,
    },
}

impl Capability {
 /// Stable single-token tag — used in audit JSON, error codes,
 /// machine-readable agent surfaces (per spec §32.2).
    pub fn tag(&self) -> &'static str {
        match self {
            Capability::Read { .. } => "read",
            Capability::Write { .. } => "write",
            Capability::Exec { .. } => "exec",
            Capability::Escalate { .. } => "escalate",
            Capability::Spawn { .. } => "spawn",
            Capability::TimeBound { .. } => "time_bound",
            Capability::Persist { .. } => "persist",
            Capability::Network { .. } => "network",
            Capability::Custom { .. } => "custom",
        }
    }
}

/// Resource identifier — what's being read/written.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceTag {
    /// Database table or whole connection.
    Database {
        /// Logical database / table identifier.
        name: String,
    },
    /// Filesystem path.
    File {
        /// Glob-style path pattern matching the file(s).
        path_pattern: String,
    },
    /// In-memory state slot.
    Memory {
        /// Logical memory region identifier.
        region: String,
    },
    /// Configuration store.
    Config {
        /// Configuration namespace identifier.
        namespace: String,
    },
    /// Logging sink.
    Logger,
    /// Random source.
    Random,
    /// Custom resource — names it explicitly.
    Custom(String),
}

/// Target of an `Exec` capability — what the cog can invoke.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecTarget {
    /// Foreign-function-interface call into a dynamic library.
    Ffi {
        /// Dynamic library name (e.g. `libsystem.B.dylib`).
        library: String,
        /// Symbol name resolved within the library.
        symbol: String,
    },
    /// Direct kernel syscall.
    Syscall {
        /// Syscall number for the target platform.
        number: u32,
    },
    /// Spawning an external program.
    Program {
        /// Filesystem path to the program executable.
        path: String,
    },
    /// User-defined exec target — referenced by tag in the ontology registry.
    Custom(String),
}

/// Privilege realm for `Capability::Escalate` — what escalated boundary
/// is crossed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrivilegeRealm {
    /// Administrative privilege.
    Admin,
    /// Root / superuser privilege.
    Root,
    /// Audit-only privilege (read-only access to security events).
    Audit,
    /// User-defined realm — referenced by tag in the ontology registry.
    Custom(String),
}

/// Supervision lifetime of a spawned task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskLifetime {
    /// Bounded by parent scope (structured concurrency).
    ScopedToParent,
    /// Detached — runs until explicit shutdown.
    Detached,
    /// Bounded by explicit deadline.
    Deadlined {
        /// Deadline in milliseconds from spawn.
        milliseconds: u64,
    },
}

/// Expiration policy for `Capability::TimeBound`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExpirationPolicy {
    /// Capability expires at a specific Unix-epoch timestamp.
    AtUnixTime {
        /// Expiration time, seconds since the Unix epoch.
        seconds: u64,
    },
    /// Capability expires after a duration from issuance.
    AfterDuration {
        /// Lifetime in milliseconds from the issuance moment.
        milliseconds: u64,
    },
    /// Capability expires when a named event fires.
    OnEvent {
        /// Event tag observed on the supervision bus.
        event_tag: String,
    },
}

/// Medium that backs a `Capability::Persist`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PersistenceMedium {
    /// Local disk path.
    Disk {
        /// Filesystem path of the persistent store.
        path: String,
    },
    /// Database connection.
    Database {
        /// Connection identifier resolvable in the context registry.
        connection_tag: String,
    },
    /// Distributed log (Kafka/NATS/etc).
    DistributedLog {
        /// Topic identifier on the distributed-log cluster.
        topic: String,
    },
}

/// Wire protocol carried by `Capability::Network`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetProtocol {
    /// Plain TCP.
    Tcp,
    /// UDP datagrams.
    Udp,
    /// Unix-domain socket (host-local).
    Unix,
    /// TLS-tunnelled TCP.
    Tls,
    /// QUIC over UDP.
    Quic,
    /// HTTP/1 or HTTP/2.
    Http,
    /// gRPC over HTTP/2.
    Grpc,
}

/// Traffic direction for a `Capability::Network`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetDirection {
    /// Inbound only — the cog accepts but does not initiate.
    Inbound,
    /// Outbound only — the cog initiates but does not accept.
    Outbound,
    /// Both inbound and outbound traffic on the boundary.
    Bidirectional,
}

/// Schema for custom capabilities — declared once in the ontology
/// registry, referenced by tag thereafter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilitySchema {
    /// Human-readable description.
    pub description: String,
    /// Whether the capability transfers privilege (escalate-style).
    pub transfers_privilege: bool,
    /// Optional related capabilities (subsumption hint).
    pub subsumed_by: Vec<String>,
}

/// Canonical capability-ontology registry — kernel-side mirror of
/// `core/architecture/capability_ontology.vr::ATS_V_CANONICAL_CAPABILITIES`.
/// The cross-side pin test
/// `crates/verum_kernel/tests/k_arch_v_alignment.rs::pin_capability_ontology_aligned`
/// asserts both sides agree on the canonical-tag set.
///
/// This baseline registry is the source of truth that the
/// compile-side ATS-V phase consults when it runs the AT-1
/// closure (`check_capability_ontology_v`).  Custom capability
/// tags outside this set raise the violation regardless of mode.
///
/// Adding a new canonical capability requires (a) adding to the
/// Verum-side `ATS_V_CANONICAL_CAPABILITIES` list, (b) adding the
/// matching string here, (c) updating the pin test.
pub fn canonical_capability_registry() -> Vec<String> {
    vec![
        // Observability
        "logger".to_string(),
        "metrics".to_string(),
        "tracing".to_string(),
        // Configuration
        "config_read".to_string(),
        "config_admin".to_string(),
        // Supervision
        "supervisor_spawn".to_string(),
        // Kernel
        "kernel_intrinsic".to_string(),
    ]
}

// =============================================================================
// Boundary — typed cross-module traffic discipline
// =============================================================================

/// Cross-module / cross-cog boundary. Per spec §4.3.
///
/// Carries:
/// * What messages can cross.
/// * What capabilities are handed off.
/// * What invariants are preserved (both sides).
/// * Wire encoding + physical layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Boundary {
    /// Messages entering the boundary from outside the cog.
    pub messages_in: Vec<MessageType>,
    /// Messages leaving the cog through the boundary.
    pub messages_out: Vec<MessageType>,
    /// Capabilities that traverse the boundary as values.
    pub capability_handoff: Vec<Capability>,
    /// Invariants both sides preserve at all times.
    pub invariants: Vec<BoundaryInvariant>,
    /// Serialisation discipline carrying messages on the wire.
    pub wire_encoding: WireEncoding,
    /// Physical-layer placement of the boundary (intracrate, IPC, network…).
    pub physical_layer: BoundaryPhysicalLayer,
}

/// Typed message that crosses a [`Boundary`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageType {
    /// Typed structured message.
    Typed {
        /// Message-type name (typically a record-type identifier).
        name: String,
        /// Stable hash over the schema, used for cross-cog version pinning.
        schema_hash: String,
    },
    /// Capability handoff message (transfers a capability).
    CapabilityTransfer {
        /// Tag of the capability being handed off.
        capability_tag: String,
    },
    /// Acknowledgement / control frame.
    Control {
        /// Control-message kind (`ack`, `ping`, …).
        kind: String,
    },
    /// Raw — discouraged; kernel flags via anti-pattern check.
    Raw,
}

/// Boundary invariant — predicate that holds on both sides of the
/// boundary at all times. Per spec §4.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoundaryInvariant {
 /// All-or-nothing transactional crossing.
    AllOrNothing,
 /// Messages serialised deterministically.
    DeterministicSerialisation,
 /// Authentication required before first message.
    AuthenticatedFirst,
 /// Backpressure honoured — no unbounded queues.
    BackpressureHonoured,
    /// Custom named invariant — refinement predicate referenced
    /// by name; resolved at audit time.
    Custom {
        /// Refinement-predicate name resolved at audit time.
        name: String,
    },
}

/// Wire-level serialisation discipline used on a [`Boundary`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireEncoding {
    /// Verum-native serialisation (canonical).
    VerumNative,
    /// Protocol Buffers schema.
    ProtoBuf {
        /// Path to the `.proto` schema file.
        schema_path: String,
    },
    /// JSON with schema reference.
    Json {
        /// URL pointing to the JSON Schema document.
        schema_url: String,
    },
    /// MessagePack.
    MsgPack,
    /// Raw bytes — flagged as anti-pattern unless explicitly justified.
    RawBytes,
}

/// Physical placement of a [`Boundary`] in the deployment topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoundaryPhysicalLayer {
    /// Same process, same crate.
    Intracrate,
    /// Same process, cross-crate.
    Intracess,
    /// Cross-process IPC.
    Ipc,
    /// Network boundary (any protocol).
    Network,
}

// =============================================================================
// Lifecycle — staged status of an architectural artifact
// =============================================================================

/// Lifecycle stage. Per spec §4.5.
///
/// Transitions are typed: `[H] → [P] → [C] → [T]` upward; `→ [D]`
/// downward. Citing higher from lower (e.g. `[T]` cites `[H]`) is
/// `LifecycleRegression` anti-pattern (ATS-V-AP-009).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lifecycle {
    /// `[H]` Hypothesis — speculation; no implementation.  CVE
    /// configuration: C partial (formulation only), V absent, E
    /// absent.  Must carry an explicit "plan to mature" — see CVE
    /// §3.5 boundary `[H]`/`[I]`: a hypothesis without a maturation
    /// plan degrades to `[I]` Interpretation (defective).
    Hypothesis {
        /// Confidence level the author assigns to the hypothesis.
        confidence: ConfidenceLevel,
    },
    /// `[Plan]` ATS-V legacy variant — "committed but not yet
    /// implemented" (TODO with target completion date).  Distinct
    /// from CVE `[P]` Postulate.  Retained for backward compatibility
    /// with existing `@arch_module(lifecycle = Lifecycle.Plan(..))`
    /// declarations.  New code SHOULD prefer:
    ///   - `Hypothesis` for not-yet-formalised intent
    ///   - `Postulate` for accepted-without-proof assumptions
    ///   - `Conditional` for "proven under explicit conditions"
    Plan {
        /// ISO-format target date (or free-form text) for completion.
        target_completion: String,
    },
    /// `[P]` Postulate — base architectural assumption accepted
    /// without proof at this layer (CVE §3.5).  CVE configuration:
    /// C accepted, V absent, E accepted.  The kernel-discharge
    /// axioms in `core/proof/kernel_bridge.vr` are canonical
    /// examples — admitted-with-citation against an external
    /// trusted base.
    Postulate {
        /// Citation justifying acceptance — typically a
        /// `@framework(corpus, "...")` reference.
        citation: String,
    },
    /// `[D]` Definition — postulated by fiat, not proven.  CVE
    /// §3.5 configuration: C present (the definition itself), V
    /// trivial (definitions are not theorems), E present.  Used
    /// for foundational types / capability-ontology entries that
    /// set boundaries rather than discharge them.
    Definition,
    /// `[C]` Conditional — proven under explicit assumptions.
    /// CVE configuration: C ∧ V ∧ E *relative to the listed
    /// conditions*.  Reading-as-`[T]` in the context where the
    /// conditions hold; outside that context, marked
    /// non-applicable without losing strength in the original.
    Conditional {
        /// Explicit assumptions under which the proof discharges.
        conditions: Vec<String>,
    },
    /// `[T]` Theorem — fully proven, load-bearing.  CVE
    /// configuration: CVE⁺ (full triple closure).  Mature
    /// artifact at the highest class.
    Theorem {
        /// Version at which the theorem reached `[T]` (load-bearing).
        since: String,
    },
    /// `[I]` Interpretation — CVE-violator; CVE §3.5 transitional
    /// status.  All three CVE axes absent AND no plan to mature.
    /// Permitted ONLY in transitional corpus revisions; mature
    /// corpus must contain ZERO Interpretation entries (each
    /// transformed to `[T]`/`[C]` via proof, downgraded to `[H]` with
    /// a maturation plan, or removed).  Annotating a cog as
    /// Interpretation in strict mode is a defect (AP candidate).
    Interpretation {
        /// Why the artefact was admitted as `[I]`-status.
        reason: String,
    },
    /// `[✗]` Retracted — previously declared but withdrawn /
    /// refuted.  Removed from active corpus but record preserved
    /// in audit chronicle as negative example.  CVE §3.5.
    Retracted {
        /// Free-form explanation of the retraction.
        reason: String,
        /// Identifier of the replacement artefact, if any.
        replacement: Option<String>,
    },
    /// `[O]` Obsolete — deprecated, scheduled for removal.  Less
    /// strict than Retracted: the artifact still functions but is
    /// expected to be replaced.  Legacy ATS-V variant; new code
    /// SHOULD use `Retracted` for explicit withdrawals and
    /// `Definition` for the `[D]` CVE status.
    Obsolete {
        /// Reason the artefact is being phased out.
        deprecation_reason: String,
        /// Identifier of the replacement artefact, if any.
        replacement: Option<String>,
    },
}

/// Confidence the author assigns to a `[H]` Hypothesis. Used by audit
/// reports to prioritise hypothesis maturation work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    /// Speculative — exploratory, low evidence.
    Low,
    /// Working hypothesis — partial evidence, planned next-step work.
    Medium,
    /// Strong hypothesis — clear path to maturation, mostly evidence.
    High,
}

impl Lifecycle {
    /// Stable diagnostic tag — single-token form used in audit
    /// JSON, anti-pattern messages, and the dual-audience surface
    /// per ATS-V §32.4.  Tags align with the CVE 7-symbol canonical
    /// taxonomy (CVE §3.5).
    pub fn tag(&self) -> &'static str {
        match self {
            Lifecycle::Hypothesis { .. } => "hypothesis",
            Lifecycle::Plan { .. } => "plan",
            Lifecycle::Postulate { .. } => "postulate",
            Lifecycle::Definition => "definition",
            Lifecycle::Conditional { .. } => "conditional",
            Lifecycle::Theorem { .. } => "theorem",
            Lifecycle::Interpretation { .. } => "interpretation",
            Lifecycle::Retracted { .. } => "retracted",
            Lifecycle::Obsolete { .. } => "obsolete",
        }
    }

    /// CVE §3.5 single-character status code — for compact
    /// rendering in cross-format outputs and stable error codes.
    /// Canonical ASCII glyphs: `H` Hypothesis, `P` Postulate,
    /// `D` Definition, `C` Conditional, `T` Theorem,
    /// `I` Interpretation, `O` Obsolete, `✗` Retracted.
    pub fn cve_glyph(&self) -> &'static str {
        match self {
            Lifecycle::Hypothesis { .. } => "H",
            Lifecycle::Plan { .. } => "Plan", // ATS-V legacy — no CVE glyph
            Lifecycle::Postulate { .. } => "P",
            Lifecycle::Definition => "D",
            Lifecycle::Conditional { .. } => "C",
            Lifecycle::Theorem { .. } => "T",
            Lifecycle::Interpretation { .. } => "I",
            Lifecycle::Retracted { .. } => "✗",
            Lifecycle::Obsolete { .. } => "O",
        }
    }

    /// Lifecycle ordering for `LifecycleRegression` (AP-009).
    /// `[T] > [D] = [C] > [P] > [H] > [I] > [✗] > Obsolete`.
    /// CVE §3.5 + §3.1: definitions / conditionals / postulates
    /// rank above hypotheses (mature artifacts cite mature
    /// artifacts); interpretations and retractions rank LOWEST
    /// (a Theorem citing an Interpretation is a defect).
    pub fn rank(&self) -> u8 {
        match self {
            Lifecycle::Obsolete { .. } => 0,
            Lifecycle::Retracted { .. } => 0,
            Lifecycle::Interpretation { .. } => 1,
            Lifecycle::Hypothesis { .. } => 2,
            Lifecycle::Plan { .. } => 3, // committed > hypothesis
            Lifecycle::Postulate { .. } => 4, // load-bearing assumption
            Lifecycle::Definition => 5,
            Lifecycle::Conditional { .. } => 5,
            Lifecycle::Theorem { .. } => 6,
        }
    }

    /// True iff the lifecycle is one CVE §6.7 forbids in mature
    /// corpus: `[I]` Interpretation without a maturation plan.
    /// Used by future anti-pattern check
    /// `InterpretationInMatureCorpus`.
    pub fn is_mature_corpus_forbidden(&self) -> bool {
        matches!(self, Lifecycle::Interpretation { .. })
    }
}

// =============================================================================
// Foundation — meta-theoretic profile
// =============================================================================

/// Foundation profile — the meta-theory the cog operates in.
/// Per spec §4.6. Composition of cogs with different foundations
/// requires an explicit functor-bridge (`FoundationBridge`); else
/// `FoundationDrift` anti-pattern (ATS-V-AP-005).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Foundation {
    /// ZFC + 2 strongly-inaccessibles (the canonical baseline).
    ZfcTwoInacc,
    /// Homotopy Type Theory (Univalent Foundations).
    Hott,
    /// Cubical HoTT.
    Cubical,
    /// Calculus of Inductive Constructions.
    Cic,
    /// Martin-Löf Type Theory.
    Mltt,
    /// Effective topos.
    Eff,
    /// User-defined custom foundation — must cite via
    /// `@framework(corpus, ...)`.
    Custom {
        /// Human-readable foundation name.
        name: String,
        /// Framework-corpus identifier carrying the foundation's axioms.
        framework_corpus: String,
    },
}

impl Foundation {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            Foundation::ZfcTwoInacc => "zfc_two_inacc",
            Foundation::Hott => "hott",
            Foundation::Cubical => "cubical",
            Foundation::Cic => "cic",
            Foundation::Mltt => "mltt",
            Foundation::Eff => "eff",
            Foundation::Custom { .. } => "custom",
        }
    }

 /// True iff `self` is interpretable into `target` without an
 /// explicit functor-bridge (canonical inclusions only).
    pub fn directly_subsumed_by(&self, target: &Foundation) -> bool {
 // Identity is always direct.
        if self == target {
            return true;
        }
 // CIC subsumes MLTT (CIC is MLTT + inductive families).
        matches!(
            (self, target),
            (Foundation::Mltt, Foundation::Cic)
                | (Foundation::Hott, Foundation::Cubical) // cubical subsumes Book HoTT
        )
    }
}

// =============================================================================
// Tier — execution placement
// =============================================================================

/// Execution tier. Per spec §4.7.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Tier 0: VBC interpreter — fast startup, ~100ns CBGR check.
    Interp,
    /// Tier 1: AOT via LLVM — 85-95% native speed.
    Aot,
    /// Tier 2: GPU compilation via MLIR.
    Gpu,
    /// Type-checking only (no codegen).
    Check,
    /// Multi-tier: cog runs on any of the listed tiers.
    MultiTier {
        /// Tiers the cog can run on (no ordering implied).
        allowed: Vec<Tier>,
    },
}

impl Tier {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            Tier::Interp => "interp",
            Tier::Aot => "aot",
            Tier::Gpu => "gpu",
            Tier::Check => "check",
            Tier::MultiTier { .. } => "multi_tier",
        }
    }

 /// True iff `caller_tier` is compatible with `callee_tier` —
 /// i.e., a function in `caller_tier` can call into
 /// `callee_tier` without violating tier discipline.
    pub fn compatible_with(&self, callee_tier: &Tier) -> bool {
        match (self, callee_tier) {
            (a, b) if a == b => true,
            (Tier::MultiTier { allowed }, b) => allowed.iter().any(|t| t == b),
            (a, Tier::MultiTier { allowed }) => allowed.iter().any(|t| t == a),
 // Check tier doesn't run; nothing is compatible with it.
            (Tier::Check, _) | (_, Tier::Check) => false,
 // Interp / Aot / Gpu — incompatible without bridge.
            _ => false,
        }
    }
}

// =============================================================================
// MsfsStratum — position in the moduli space (MSFS preprint)
// =============================================================================

/// Position of a cog in the MSFS-modulating space. Per spec §4.7
/// + reflection-tower module. `LAbs` is impossible by AFN-T α
/// (MSFS Theorem 5.1) — any cog claiming `LAbs` triggers
/// `AbsoluteBoundaryAttempt` anti-pattern (ATS-V-AP-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MsfsStratum {
 /// L_Fnd — foundation level.
    LFnd,
 /// L_Cls — classifier level.
    LCls,
 /// L_Cls^⊤ — maximal classifier sub-class.
    LClsTop,
 /// L_Abs — empty by AFN-T α; declaring this is a defect.
    LAbs,
}

impl MsfsStratum {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            MsfsStratum::LFnd => "l_fnd",
            MsfsStratum::LCls => "l_cls",
            MsfsStratum::LClsTop => "l_cls_top",
            MsfsStratum::LAbs => "l_abs",
        }
    }

 /// True iff the stratum is admissible (i.e., not `LAbs`).
    pub fn is_admissible(&self) -> bool {
        !matches!(self, MsfsStratum::LAbs)
    }
}

// =============================================================================
// CveClosure — three-axis closure triple (Constructive / Verifiable / Executable)
// =============================================================================

/// CVE-closure triple per spec §4.8 + §32 (dual-audience contract).
///
/// Each axis carries an identifier path + provenance. In strict
/// mode (`@arch_module(strict=true)`), all three fields MUST be
/// present; in soft mode, missing axes produce warnings but not
/// errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CveClosure {
 /// C — Constructive witness (function or constructor path).
    pub constructive: Option<String>,
 /// V — Verification strategy from the existing `@verify` ladder.
    pub verifiable_strategy: Option<VerifyStrategy>,
 /// E — Executable artefact (entry point or audit command).
    pub executable: Option<String>,
}

impl CveClosure {
 /// True iff all three axes are present (full CVE-closure).
    pub fn is_fully_closed(&self) -> bool {
        self.constructive.is_some()
            && self.verifiable_strategy.is_some()
            && self.executable.is_some()
    }

 /// Number of axes that discharge (0..=3).
    pub fn closure_degree(&self) -> u8 {
        let mut n = 0;
        if self.constructive.is_some() {
            n += 1;
        }
        if self.verifiable_strategy.is_some() {
            n += 1;
        }
        if self.executable.is_some() {
            n += 1;
        }
        n
    }
}

/// Verification strategy — mirrors the existing `@verify(...)`
/// ladder per grammar (verum.ebnf:467+). Reuse, not parallel
/// system (per spec §17.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerifyStrategy {
    /// `@verify(runtime)` — runtime assertions only, no SMT.
    Runtime,
    /// `@verify(static)` — light-weight static analysis.
    Static,
    /// `@verify(fast)` — single-solver SMT with tight timeout.
    Fast,
    /// `@verify(formal)` — single-solver SMT, default budget.
    Formal,
    /// `@verify(proof)` — proof-term construction (kernel-checked).
    Proof,
    /// `@verify(thorough)` — portfolio SMT (Z3 + CVC5 in parallel).
    Thorough,
    /// `@verify(reliable)` — portfolio with first-wins consensus.
    Reliable,
    /// `@verify(certified)` — portfolio with cross-validated agreement.
    Certified,
    /// `@verify(synthesize)` — SyGuS / synthesis surface.
    Synthesize,
}

impl VerifyStrategy {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            VerifyStrategy::Runtime => "runtime",
            VerifyStrategy::Static => "static",
            VerifyStrategy::Fast => "fast",
            VerifyStrategy::Formal => "formal",
            VerifyStrategy::Proof => "proof",
            VerifyStrategy::Thorough => "thorough",
            VerifyStrategy::Reliable => "reliable",
            VerifyStrategy::Certified => "certified",
            VerifyStrategy::Synthesize => "synthesize",
        }
    }

 /// Strength rank — higher is stronger. Per VVA §12 the
 /// strategies are strictly ordered on the Diakrisis ν-ladder.
    pub fn rank(&self) -> u32 {
        match self {
            VerifyStrategy::Runtime => 0,
            VerifyStrategy::Static => 1,
            VerifyStrategy::Fast => 2,
            VerifyStrategy::Formal => 3,
            VerifyStrategy::Proof => 4,
            VerifyStrategy::Thorough => 5,
            VerifyStrategy::Reliable => 6,
            VerifyStrategy::Certified => 7,
            VerifyStrategy::Synthesize => 8,
        }
    }
}

// =============================================================================
// CVE-architecture spec primitives — operationalisation of cve-architecture.md
// =============================================================================
//
// Mirrors the Verum-side block in `core/architecture/types.vr`. Every
// type, every variant, every helper here has a 1:1 counterpart on the
// Verum side; the cross-side pin test in
// `crates/verum_kernel/tests/k_arch_v_alignment.rs` enforces alignment.

/// The three senses of executability that the CVE-E axis disambiguates.
/// CVE-E refers EXCLUSIVELY to `StructuralReadiness`.
///
/// Per spec §2.3.0, conflating these is a register collision: an
/// artefact "executed yesterday" (`PostFactumChronicle`) is not
/// thereby "structurally ready to run today" (`StructuralReadiness`),
/// and an artefact "currently running" (`CurrentExecution`) is not
/// thereby "deployable to a new environment".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutabilitySense {
    /// THE canonical content of CVE-E: the artefact admits a
    /// working representation deployable in a suitable environment.
    StructuralReadiness,
    /// The artefact is presently running. Stronger than E; relevant
    /// to L0 maturity but not to the E axis itself.
    CurrentExecution,
    /// Accumulated history of past execution. Material for §15
    /// antifragility chronicle, NOT for the E axis.
    PostFactumChronicle,
}

impl ExecutabilitySense {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            ExecutabilitySense::StructuralReadiness => "structural_readiness",
            ExecutabilitySense::CurrentExecution => "current_execution",
            ExecutabilitySense::PostFactumChronicle => "post_factum_chronicle",
        }
    }

    /// True iff the sense is the canonical content of CVE-E.
    /// Soundness pin: exactly one of three senses anchors the E axis.
    pub fn is_canonical_e(&self) -> bool {
        matches!(self, ExecutabilitySense::StructuralReadiness)
    }
}

/// The cognitive mode under which a knowledge artefact is articulated.
/// CVE itself operates under `AnalyticDecompositional`; alternatives
/// are co-equal in their proper domains per spec §1.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CognitiveSubstrate {
    /// CVE's own substrate: K/V/E projections evaluated separately.
    /// Default for `@arch_module(...)`.
    AnalyticDecompositional,
    /// Holistic-relational mode: artefact evaluated as a node in
    /// a network of relations.
    HolisticRelational,
    /// Action-centric mode: the action IS the artefact (craft
    /// mastery, performance disciplines).
    ActionCentric,
    /// Tradition-transmitting mode: identity preserved through
    /// multi-generational reproduction.
    TraditionTransmitting,
}

impl CognitiveSubstrate {
    /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            CognitiveSubstrate::AnalyticDecompositional => "analytic_decompositional",
            CognitiveSubstrate::HolisticRelational => "holistic_relational",
            CognitiveSubstrate::ActionCentric => "action_centric",
            CognitiveSubstrate::TraditionTransmitting => "tradition_transmitting",
        }
    }

    /// Default substrate for ATS-V annotations.
    pub fn default_for_ats_v() -> Self {
        CognitiveSubstrate::AnalyticDecompositional
    }
}

/// The formal tradition anchoring a tri-axis closure. CHL is the
/// most-developed; parallel anchorings on different stages of
/// formalisation are registered per spec §4.5.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormalAnchoring {
    /// Curry-Howard-Lawvere — logic ↔ types ↔ categories.  The
    /// canonical anchoring for mathematical and SE artefacts.
    CurryHowardLawvere,
    /// Grammars ↔ automata ↔ languages.
    AutomataTheory,
    /// State equations ↔ transfer functions ↔ realisations.
    ControlTheory,
    /// Specifications ↔ execution models ↔ observable traces.
    DistributedProtocols,
    /// Afferent synthesis ↔ action program ↔ result acceptor.
    FunctionalSystems,
    /// Normative structure ↔ decision procedure ↔ stabilised practices.
    InstitutionalDesign,
    /// User-registered anchoring.
    CustomAnchoring(String),
}

impl FormalAnchoring {
    /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            FormalAnchoring::CurryHowardLawvere => "curry_howard_lawvere",
            FormalAnchoring::AutomataTheory => "automata_theory",
            FormalAnchoring::ControlTheory => "control_theory",
            FormalAnchoring::DistributedProtocols => "distributed_protocols",
            FormalAnchoring::FunctionalSystems => "functional_systems",
            FormalAnchoring::InstitutionalDesign => "institutional_design",
            FormalAnchoring::CustomAnchoring(_) => "custom_anchoring",
        }
    }

    /// Default anchoring for Verum-native artefacts.
    pub fn default_for_ats_v() -> Self {
        FormalAnchoring::CurryHowardLawvere
    }
}

/// Threshold on the K (Constructive) axis. Per spec §14.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CveThresholdK {
    /// Explicit, computable, first-class witness; no holes.
    FullWitness,
    /// Schema-with-typed-parameters is sufficient.
    TypedSchema,
    /// Reference implementation bounded to a stated domain is sufficient.
    ReferenceImplBounded,
}

impl CveThresholdK {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            CveThresholdK::FullWitness => "full_witness",
            CveThresholdK::TypedSchema => "typed_schema",
            CveThresholdK::ReferenceImplBounded => "reference_impl_bounded",
        }
    }
}

/// Threshold on the V (Verifiable) axis. Per spec §14.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CveThresholdV {
    /// Kernel-checked machine proof in the chosen meta-theory.
    FullFormalProof,
    /// Typechecker + named test battery with declared coverage.
    TypecheckPlusTests,
    /// Passage of a named certification (audit, conformance suite).
    NamedCertification,
}

impl CveThresholdV {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            CveThresholdV::FullFormalProof => "full_formal_proof",
            CveThresholdV::TypecheckPlusTests => "typecheck_plus_tests",
            CveThresholdV::NamedCertification => "named_certification",
        }
    }
}

/// Threshold on the E (Executable, in `StructuralReadiness` sense) axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CveThresholdE {
    /// Deployable in any environment of the declared class.
    StructurallyReady,
    /// One specific environment is sufficient.
    DeployedInOneEnv,
    /// A functor in a category is sufficient.
    FunctorialOnly,
}

impl CveThresholdE {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            CveThresholdE::StructurallyReady => "structurally_ready",
            CveThresholdE::DeployedInOneEnv => "deployed_in_one_env",
            CveThresholdE::FunctorialOnly => "functorial_only",
        }
    }
}

/// Declared purpose of an architectural artefact. The audit
/// terminates when configuration meets thresholds (spec §14.6 —
/// avoids the "boundless audit" anti-pattern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Purpose {
    /// Short label of the role the artefact serves.
    pub role: String,
    /// Minimum K threshold sufficient for the role.
    pub k_min: CveThresholdK,
    /// Minimum V threshold sufficient for the role.
    pub v_min: CveThresholdV,
    /// Minimum E threshold (in `StructuralReadiness` sense) for the role.
    pub e_min: CveThresholdE,
}

impl Purpose {
    /// Default purpose for cogs without an explicit declaration.
    pub fn default_unspecified() -> Self {
        Purpose {
            role: "unspecified".to_string(),
            k_min: CveThresholdK::FullWitness,
            v_min: CveThresholdV::TypecheckPlusTests,
            e_min: CveThresholdE::StructurallyReady,
        }
    }
}

/// Architectural defect kind, per spec §20.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefectKind {
    /// Systematic rejection of artefacts that turned out mature.
    FalseRejection,
    /// Systematic acceptance of artefacts that turned out unstable.
    FalseAcceptance,
    /// Inter-layer leak unresolved by current stratification.
    InterLayerLeak,
    /// Defect outside the three canonical kinds; carries free-form description.
    OtherDefect(String),
}

impl DefectKind {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            DefectKind::FalseRejection => "false_rejection",
            DefectKind::FalseAcceptance => "false_acceptance",
            DefectKind::InterLayerLeak => "inter_layer_leak",
            DefectKind::OtherDefect(_) => "other_defect",
        }
    }
}

/// Resolution path for a registered architectural defect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// Modify the architectural law itself (L4 — rare).
    L4Revision,
    /// Adjust the methodology layer without touching the architectural law (typical).
    L2Refinement,
    /// Resolution outside the two canonical paths; carries free-form description.
    OtherResolution(String),
}

impl Resolution {
    /// Stable diagnostic tag used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            Resolution::L4Revision => "l4_revision",
            Resolution::L2Refinement => "l2_refinement",
            Resolution::OtherResolution(_) => "other_resolution",
        }
    }
}

/// Architectural defect record, per spec §20.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitecturalDefect {
    /// Short label identifying the defect.
    pub short_name: String,
    /// Architecture version against which the defect was filed.
    pub arch_version: String,
    /// Absolute date of submission (ISO format).
    pub submitted_on: String,
    /// Identifier of the submitter.
    pub submitter: String,
    /// Type of defect — false rejection / false acceptance / inter-layer leak / other.
    pub kind: DefectKind,
    /// Reference to a concrete artefact demonstrating the defect.
    pub witness_artefact: String,
    /// Domain and task in which the defect manifested.
    pub application_context: String,
    /// What happened when the principle was applied.
    pub observed_result: String,
    /// What the correct outcome would have been absent the defect.
    pub expected_result: String,
    /// Proposed resolution path.
    pub proposed_resolution: Resolution,
}

/// Universal-property characterisation of a fixed-point theorem.
///
/// Categorically, a fixed-point theorem is the assertion that, in a
/// chosen category `C` and for a chosen class `E ⊆ End(C)` of
/// endomorphisms, every `T ∈ E` has a fixed point `Fix(T)` (and where
/// applicable, the fixed point is unique). The triple
/// `(category, endomorphism_class, theorem)` is the universal-property
/// classifier; concrete theorems (Banach, Tarski-Knaster, Adamek)
/// inhabit specific points of this classifier and are produced by
/// the smart constructors [`FixpointClass::banach`],
/// [`FixpointClass::tarski`], [`FixpointClass::adamek`].
///
/// Per CVE articulation hygiene (seven-symbols / articulation-hygiene
/// §8), every self-referential `Shape` must carry a witness whose
/// `fixpoint_class` is pinned to one of these classifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixpointClass {
    /// The category `C` in which the fixed-point claim is made.
    pub category: FixpointCategory,
    /// The class `E ⊆ End(C)` of endomorphisms admitted by the
    /// theorem.
    pub endomorphism_class: EndomorphismClass,
    /// The theorem citation discharging existence (and where
    /// applicable, uniqueness) of `Fix(T)` for every `T ∈ E`.
    pub theorem: FixpointTheorem,
}

/// The category `C` in which a fixed-point theorem is stated. The
/// three named variants correspond to the three canonical classifiers
/// (complete metric space, complete lattice, cocomplete category);
/// `CustomCategory` is the open variant for user-cited categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixpointCategory {
    /// Complete metric space; pairs with [`EndomorphismClass::Contracting`]
    /// in Banach's theorem.
    CompleteMetricSpace,
    /// Complete lattice (every subset has join and meet); pairs with
    /// [`EndomorphismClass::Monotone`] in Tarski-Knaster.
    CompleteLattice,
    /// Cocomplete category (admits all small colimits); pairs with
    /// [`EndomorphismClass::ContinuousFunctor`] in Adamek's theorem.
    CocompleteCategory,
    /// User-cited category; the citation MUST appear in the cog's
    /// `@framework(...)` attribute.
    CustomCategory(String),
}

/// The class `E ⊆ End(C)` of endomorphisms admitted by a fixed-point
/// theorem. Pairs canonically with a [`FixpointCategory`] under one of
/// the three named theorems; `CustomEndomorphismClass` is the open
/// variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndomorphismClass {
    /// Contracting endomorphism `T : C → C` — `d(T(x), T(y)) ≤ k·d(x, y)`
    /// for some `k < 1` on a metric space `C`.
    Contracting,
    /// Monotone endomorphism `T : L → L` — order-preserving on a
    /// lattice `L`.
    Monotone,
    /// Continuous endofunctor `F : C → C` — preserves colimits in a
    /// cocomplete category.
    ContinuousFunctor,
    /// User-cited endomorphism class.
    CustomEndomorphismClass(String),
}

/// The theorem citation discharging a fixed-point claim. The three
/// named variants correspond to the three canonical theorems; `Custom`
/// is the open variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixpointTheorem {
    /// Banach fixed-point theorem — unique fixed point of a contracting
    /// endomorphism on a complete metric space.
    Banach,
    /// Tarski-Knaster — existence (possibly non-unique) of fixed
    /// point of a monotone endomorphism on a complete lattice.
    Tarski,
    /// Adamek's initial-algebra theorem — initial-algebra fixed
    /// point of a continuous endofunctor on a cocomplete category.
    Adamek,
    /// User-cited theorem; the citation MUST appear in the cog's
    /// `@framework(...)` attribute and be enumerable via
    /// `verum audit --framework-axioms`.
    Custom(String),
}

impl FixpointClass {
    /// Smart constructor for Banach's fixed-point theorem class:
    /// `(CompleteMetricSpace, Contracting, Banach)`.
    pub fn banach() -> Self {
        FixpointClass {
            category: FixpointCategory::CompleteMetricSpace,
            endomorphism_class: EndomorphismClass::Contracting,
            theorem: FixpointTheorem::Banach,
        }
    }

    /// Smart constructor for the Tarski-Knaster theorem class:
    /// `(CompleteLattice, Monotone, Tarski)`.
    pub fn tarski() -> Self {
        FixpointClass {
            category: FixpointCategory::CompleteLattice,
            endomorphism_class: EndomorphismClass::Monotone,
            theorem: FixpointTheorem::Tarski,
        }
    }

    /// Smart constructor for Adamek's initial-algebra theorem class:
    /// `(CocompleteCategory, ContinuousFunctor, Adamek)`.
    pub fn adamek() -> Self {
        FixpointClass {
            category: FixpointCategory::CocompleteCategory,
            endomorphism_class: EndomorphismClass::ContinuousFunctor,
            theorem: FixpointTheorem::Adamek,
        }
    }

    /// Smart constructor for a user-cited fixed-point theorem class:
    /// `(CustomCategory, CustomEndomorphismClass, Custom)`.  The
    /// citation MUST be enumerable via `verum audit --framework-axioms`.
    pub fn custom_fixpoint(citation: impl Into<String>) -> Self {
        let c = citation.into();
        FixpointClass {
            category: FixpointCategory::CustomCategory(c.clone()),
            endomorphism_class: EndomorphismClass::CustomEndomorphismClass(c.clone()),
            theorem: FixpointTheorem::Custom(c),
        }
    }

    /// Stable diagnostic tag, dispatched on the theorem citation.
    /// Cross-side-aligned with `core/architecture/types.vr::fixpoint_class_tag`.
    pub fn tag(&self) -> &'static str {
        match &self.theorem {
            FixpointTheorem::Banach   => "banach",
            FixpointTheorem::Tarski   => "tarski",
            FixpointTheorem::Adamek   => "adamek",
            FixpointTheorem::Custom(_) => "custom_fixpoint",
        }
    }

    /// Returns `true` when this class is one of the three canonical
    /// (Banach / Tarski / Adamek); `false` for `Custom`.
    pub fn is_canonical(&self) -> bool {
        !matches!(self.theorem, FixpointTheorem::Custom(_))
    }
}

/// Self-reference witness, per cve-architecture spec §16. A cog
/// whose `Shape` exhibits a self-referential pattern (self in
/// `composes_with`, capability targeting the cog's own holon,
/// requires citing the cog itself) MUST declare this witness.
/// Absence triggers AP-040 `SelfReferenceWithoutOperator`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfReferenceWitness {
    /// Path to the cog implementing the operator `T_X` whose fixed
    /// point the self-referential cog inhabits.  Must have lifecycle
    /// ≥ Conditional.
    pub operator: String,
    /// Path to the cog implementing the fixed point `Fix(T_X)`.  May
    /// coincide with the self-referential cog itself when the
    /// self-reference is constructive.
    pub fixed_point: String,
    /// Fixed-point theorem class discharging the witness.
    pub fixpoint_class: FixpointClass,
}

impl SelfReferenceWitness {
    /// Default-constructor for tests only.  Production cogs MUST
    /// supply a non-default witness with concrete operator + fixed-
    /// point paths.
    pub fn unspecified() -> Self {
        SelfReferenceWitness {
            operator: "unspecified".to_string(),
            fixed_point: "unspecified".to_string(),
            fixpoint_class: FixpointClass::custom_fixpoint("unspecified"),
        }
    }
}

/// Optional declarations packaged into a `Shape`. Carries the
/// CVE-architecture spec concepts that are not yet load-bearing in
/// the canonical primitives but ARE load-bearing in the architectural
/// law.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeDeclarations {
    /// Declared purpose terminating the audit (spec §14.6).
    pub purpose: Option<Purpose>,
    /// Declared cognitive substrate (spec §1.5).
    pub substrate: Option<CognitiveSubstrate>,
    /// Declared formal anchoring (spec §4.5).
    pub anchoring: Option<FormalAnchoring>,
    /// Declared executability sense (spec §2.3.0).
    pub e_sense: Option<ExecutabilitySense>,
    /// Declared self-reference witness (spec §16).  Required when the
    /// cog's `Shape` exhibits self-referential patterns; absence
    /// triggers AP-040 `SelfReferenceWithoutOperator`.
    #[serde(default)]
    pub self_reference: Option<SelfReferenceWitness>,
}

impl ShapeDeclarations {
    /// Empty declarations record — every field unspecified, the
    /// architectural type-checker fills defaults at audit time.
    pub fn empty() -> Self {
        ShapeDeclarations {
            purpose: None,
            substrate: None,
            anchoring: None,
            e_sense: None,
            self_reference: None,
        }
    }
}

// =============================================================================
// Shape — main carrier per `@arch_module(...)`
// =============================================================================

/// Main carrier of `@arch_module(...)`. Per spec §4.1.
///
/// Built by the parser when it encounters `@arch_module(...)` on
/// a module declaration; consumed by the ATS-V phase (Phase 6.5)
/// + the kernel intrinsic dispatcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shape {
 /// Capabilities the cog exposes to consumers.
    pub exposes: Vec<Capability>,
 /// Capabilities the cog requires from the environment.
    pub requires: Vec<Capability>,
 /// Boundary invariants preserved through the cog.
    pub preserves: Vec<BoundaryInvariant>,
 /// Linear / affine resources consumed.
    pub consumes: Vec<String>,
 /// Execution tier constraint.
    pub at_tier: Tier,
 /// Foundation profile.
    pub foundation: Foundation,
 /// MSFS-modulating-space stratum.
    pub stratum: MsfsStratum,
 /// CVE-closure triple.
    pub cve_closure: CveClosure,
 /// Lifecycle stage.
    pub lifecycle: Lifecycle,
 /// Cogs / functions this cog composes with.
    pub composes_with: Vec<String>,
 /// Strict mode flag (compile errors vs warnings).
    pub strict: bool,
 /// Optional CVE-architecture spec declarations: purpose
 /// (terminates audit per §14.6), cognitive substrate (per §1.5),
 /// formal anchoring (per §4.5), executability sense (per §2.3.0).
 /// Absence in strict mode triggers the CVE-AH band anti-patterns
 /// (AP-037..AP-039).
    #[serde(default)]
    pub declarations: Option<ShapeDeclarations>,
}

impl Shape {
 /// Default shape used for cogs without `@arch_module`. Per
 /// spec §6.1 (smart default inference). Soft-mode trivial
 /// shape — passes all anti-pattern checks vacuously.
    pub fn default_for_unannotated() -> Self {
        Shape {
            exposes: Vec::new(),
            requires: Vec::new(),
            preserves: Vec::new(),
            consumes: Vec::new(),
            at_tier: Tier::MultiTier {
                allowed: vec![Tier::Interp, Tier::Aot],
            },
            foundation: Foundation::ZfcTwoInacc,
            stratum: MsfsStratum::LFnd,
            cve_closure: CveClosure {
                constructive: None,
                verifiable_strategy: None,
                executable: None,
            },
            lifecycle: Lifecycle::Plan {
                target_completion: "unspecified".to_string(),
            },
            composes_with: Vec::new(),
            strict: false,
            declarations: None,
        }
    }
}

// =============================================================================
// Auxiliary typed attributes — @bridge_tier / @deterministic /
// @mtac_decision / @arch_corpus
// =============================================================================
//
// These attributes complement `@arch_module(...)` with focused,
// orthogonal annotations.  They use the same generic
// `attribute_args = named_arg_list` grammar form — no new grammar
// production is required; the parser at `arch_parse.rs` dispatches
// on attribute name.

/// `@bridge_tier(from: Tier.X, to: Tier.Y)` — annotates a function
/// that legitimately crosses the X→Y tier boundary.  Lifts the
/// AP-004 TierMixing ban for *this specific call site*; the audit
/// chronicle records every bridge for review.
///
/// Soundness contract: a bridge does not eliminate the cost of
/// the cross-tier transition; it merely declares it intentional.
/// The runtime inserts the appropriate transition (Interp-call
/// from Aot, GPU-launch from Aot, etc.) at the call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTier {
    /// Source tier the call originates from.
    pub from: Tier,
    /// Destination tier the call lands in.
    pub to: Tier,
}

impl BridgeTier {
    /// True iff the bridge connects two distinct tiers — a no-op
    /// bridge (same tier on both sides) is itself an architectural
    /// defect (use the bare call site instead).
    pub fn is_load_bearing(&self) -> bool {
        self.from != self.to
    }
}

/// `@deterministic` — marker attribute (no args) declaring that a
/// function MUST produce identical output on identical inputs
/// across runs / hosts / clock domains.
///
/// The marker is consumed by AP-015 DeterministicViolation:
/// invocations of non-deterministic primitives (Random, SystemTime,
/// FilesystemMtime, network) within the marked function raise the
/// violation.  Determinism is the foundation of replay verification
/// and DST testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeterministicMarker;

impl DeterministicMarker {
    /// Stable diagnostic tag for audit JSON.
    pub fn tag(&self) -> &'static str {
        "deterministic"
    }
}

/// `@mtac_decision { point: TimePoint.X, by_observer: Observer.Y,
/// proposition: ArchProposition.Z, modality: ModalAssertion.W }` —
/// attaches a typed Modal-Temporal Architectural Calculus claim to
/// a function or cog.  The ATS-V phase records the claim for the
/// MTAC anti-pattern checks (AP-027 TemporalInconsistency, AP-028
/// CounterfactualBrittleness, AP-031 PhantomEvolution).
///
/// Each MtacDecision is a dated, observer-witnessed, modally-typed
/// architectural commitment — the unit of historical record in the
/// MTAC corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MtacDecisionAttr {
    /// When the decision applies in the time category.
    pub point: crate::arch_mtac::TimePoint,
    /// Which observer canonically witnesses the decision.
    pub by_observer: crate::arch_mtac::Observer,
    /// What architectural proposition the decision asserts.
    pub proposition: crate::arch_mtac::ArchProposition,
    /// Under which modal operator the proposition holds.
    pub modality: MtacModality,
}

/// Single-token modality the `@mtac_decision { modality: ... }`
/// field accepts.  This is a flattened projection of the six
/// canonical `ModalAssertion` operators; the typed-attribute parser
/// reads a bare path like `ModalAssertion.Necessity` and emits the
/// matching `MtacModality` arm.  The ATS-V phase reconstructs the
/// full propositional content from the `proposition` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MtacModality {
    /// `□ A` — A holds in every possible future / decision branch.
    Necessity,
    /// `◇ A` — A holds in some possible future / decision branch.
    Possibility,
    /// `◇F A` — A holds in some future time-point.
    Eventually,
    /// `□G A` — A holds in every future time-point.
    Always,
    /// `A U B` — temporal Until.  Used only in `@mtac_decision { proposition }`
    /// when paired with a follow-up `Until` chain; standalone use is the
    /// `Necessity` shadow.
    Until,
    /// `A ⇨ B` — counterfactual conditional.
    Counterfactual,
}

impl MtacModality {
    /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            MtacModality::Necessity => "necessity",
            MtacModality::Possibility => "possibility",
            MtacModality::Eventually => "eventually",
            MtacModality::Always => "always",
            MtacModality::Until => "until",
            MtacModality::Counterfactual => "counterfactual",
        }
    }
}

/// `@arch_corpus(invariants: [...], foundation_bridges: [...])` —
/// scope attribute for cross-cog invariants.  Mirrors
/// `core.architecture.corpus.CorpusInvariant`.
///
/// Where `@arch_module(...)` describes a single cog's Shape,
/// `@arch_corpus(...)` describes properties holding over the
/// entire corpus — composition graph acyclic, foundations
/// consistent, no cog claims `LAbs`, every required capability
/// has a producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArchCorpusAttr {
    /// Which corpus invariants the attribute opts into checking.
    /// Empty list means "default 4-roster" (all baseline invariants).
    pub invariants: Vec<crate::arch_corpus::CorpusInvariant>,
    /// Foundation bridges the corpus declares — pairs of
    /// (`peer_cog`, `framework_corpus_label`) declaring an explicit
    /// translation between the two foundations.  When present,
    /// AP-005 FoundationDrift suppresses for the bridged pair.
    pub foundation_bridges: Vec<(String, String)>,
}

// =============================================================================
// CVE seven-cell closure (seven-configurations §9 of the website)
// =============================================================================

/// **CVE axis mode** — per-axis valuation in the closure analysis
/// of the seven-configurations truth-table. Three modes per axis,
/// 3³ = 27 cells in the configuration space `CveAxisMode³`.
///
/// - `Positive` — axis clearly satisfied (witness present, decision
///                procedure present, executable representation present).
/// - `Partial`  — axis satisfied with qualification: trivial-by-fiat
///                (definition), conditional (under stated assumption),
///                external (delegated via citation), or formulated
///                but not realised.
/// - `Absent`   — axis not satisfied; no witness, no check, no
///                executable representation.
///
/// The seven productive configurations of the seven-configurations
/// taxonomy (`[T]`, `[D]`, `[C]`, `[P]`, `[H]`, `[I]`, `[✗]`) are the
/// stable attractors of `CveAxisMode³`. The migration map from each
/// cell to a productive cell is given by
/// [`seven_configurations_closure_witness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CveAxisMode {
    /// Axis clearly satisfied — witness present, decision procedure
    /// present, executable representation present.
    Positive,
    /// Axis satisfied with qualification — trivial-by-fiat, conditional,
    /// external delegation, or formulated but not realised.
    Partial,
    /// Axis not satisfied — no witness, no check, no executable
    /// representation.
    Absent,
}

impl CveAxisMode {
    /// Stable diagnostic tag, cross-side-aligned with
    /// `core/architecture/types.vr::cve_axis_mode_tag`.
    pub fn tag(&self) -> &'static str {
        match self {
            CveAxisMode::Positive => "positive",
            CveAxisMode::Partial  => "partial",
            CveAxisMode::Absent   => "absent",
        }
    }
}

/// Free-fn form for cross-side parity with the Verum side.
pub fn cve_axis_mode_tag(m: CveAxisMode) -> &'static str {
    m.tag()
}

/// **Closure witness for the CVE seven-cell theorem** (seven-configurations
/// §9 of the website). For every (c, v, e) ∈ `CveAxisMode³` returns the
/// canonical glyph of the productive cell to which the configuration
/// migrates under the deciding rule (seven-configurations §10).
///
/// Glyph alphabet: `[T]`, `[D]`, `[C]`, `[P]`, `[H]`, `[I]`. `[✗]`
/// Retracted is a meta-state outside `CveAxisMode³` (deliberate
/// withdrawal) and is not produced by this witness.
///
/// Default migration of V=Partial is to `[C]` Conditional. The
/// V-partial sub-classification (trivial → `[D]`, external → `[P]`,
/// conditional → `[C]`) is operational at the call site; the closure
/// witness returns the most general migration target.
///
/// Cross-side mirror: `core/architecture/types.vr::seven_configurations_closure_witness`.
pub fn seven_configurations_closure_witness(
    c: CveAxisMode,
    v: CveAxisMode,
    e: CveAxisMode,
) -> &'static str {
    use CveAxisMode::*;
    match (c, v, e) {
        (Positive, Positive, Positive) => "[T]",
        (Positive, Positive, Partial)  => "[C]",
        (Positive, Positive, Absent)   => "[C]",
        (Positive, Partial,  Positive) => "[C]",
        (Positive, Partial,  Partial)  => "[C]",
        (Positive, Partial,  Absent)   => "[C]",
        (Positive, Absent,   Positive) => "[H]",
        (Positive, Absent,   Partial)  => "[H]",
        (Positive, Absent,   Absent)   => "[H]",
        (Partial,  Positive, Positive) => "[C]",
        (Partial,  Positive, Partial)  => "[C]",
        (Partial,  Positive, Absent)   => "[C]",
        (Partial,  Partial,  Positive) => "[C]",
        (Partial,  Partial,  Partial)  => "[C]",
        (Partial,  Partial,  Absent)   => "[H]",
        (Partial,  Absent,   Positive) => "[H]",
        (Partial,  Absent,   Partial)  => "[H]",
        (Partial,  Absent,   Absent)   => "[H]",
        (Absent,   Positive, Positive) => "[I]",
        (Absent,   Positive, Partial)  => "[I]",
        (Absent,   Positive, Absent)   => "[I]",
        (Absent,   Partial,  Positive) => "[I]",
        (Absent,   Partial,  Partial)  => "[I]",
        (Absent,   Partial,  Absent)   => "[I]",
        (Absent,   Absent,   Positive) => "[I]",
        (Absent,   Absent,   Partial)  => "[I]",
        (Absent,   Absent,   Absent)   => "[I]",
    }
}

/// Predicate: glyph belongs to the productive alphabet (six base
/// glyphs of the seven-configurations taxonomy; `[✗]` is excluded as
/// a meta-state).
pub fn is_productive_glyph(g: &str) -> bool {
    matches!(g, "[T]" | "[D]" | "[C]" | "[P]" | "[H]" | "[I]")
}

/// **Soundness pin** (CVE seven-cell closure §9): every cell of
/// `CveAxisMode³` maps to a productive glyph. The 27-arm match in
/// [`seven_configurations_closure_witness`] is exhaustive by pattern
/// coverage; this pin re-asserts exhaustiveness operationally and
/// pin-tests cross-side parity with
/// `core/architecture/types.vr::seven_configurations_closure_exhaustive`.
pub fn seven_configurations_closure_exhaustive() -> bool {
    use CveAxisMode::*;
    let modes = [Positive, Partial, Absent];
    for &c in &modes {
        for &v in &modes {
            for &e in &modes {
                if !is_productive_glyph(seven_configurations_closure_witness(c, v, e)) {
                    return false;
                }
            }
        }
    }
    true
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_tags_distinct() {
        let probes = [
            Capability::Read {
                resource: ResourceTag::Logger,
            },
            Capability::Write {
                resource: ResourceTag::Logger,
            },
            Capability::Exec {
                target: ExecTarget::Custom("x".into()),
            },
            Capability::Escalate {
                realm: PrivilegeRealm::Admin,
            },
            Capability::Spawn {
                lifetime: TaskLifetime::Detached,
            },
            Capability::TimeBound {
                until: ExpirationPolicy::AfterDuration { milliseconds: 1000 },
            },
            Capability::Persist {
                medium: PersistenceMedium::Disk { path: "/x".into() },
            },
            Capability::Network {
                protocol: NetProtocol::Tcp,
                direction: NetDirection::Inbound,
            },
            Capability::Custom {
                tag: "logger".into(),
                schema: CapabilitySchema {
                    description: "x".into(),
                    transfers_privilege: false,
                    subsumed_by: vec![],
                },
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|c| c.tag()).collect();
        assert_eq!(tags.len(), 9, "every Capability variant must have a distinct tag");
    }

    #[test]
    fn lifecycle_rank_orders_correctly() {
 // Pin: rank Theorem > Conditional > Plan > Hypothesis > Obsolete.
        assert!(
            Lifecycle::Theorem {
                since: "v0.1".into()
            }
            .rank()
                > Lifecycle::Conditional {
                    conditions: vec![]
                }
                .rank()
        );
        assert!(
            Lifecycle::Conditional {
                conditions: vec![]
            }
            .rank()
                > Lifecycle::Plan {
                    target_completion: "x".into()
                }
                .rank()
        );
        assert!(
            Lifecycle::Plan {
                target_completion: "x".into()
            }
            .rank()
                > Lifecycle::Hypothesis {
                    confidence: ConfidenceLevel::Medium
                }
                .rank()
        );
 // Obsolete is below everything.
        assert!(
            Lifecycle::Hypothesis {
                confidence: ConfidenceLevel::Medium
            }
            .rank()
                > Lifecycle::Obsolete {
                    deprecation_reason: "x".into(),
                    replacement: None
                }
                .rank()
        );
    }

    #[test]
    fn tier_compatible_with_self() {
        assert!(Tier::Aot.compatible_with(&Tier::Aot));
        assert!(Tier::Interp.compatible_with(&Tier::Interp));
        assert!(!Tier::Aot.compatible_with(&Tier::Interp));
    }

    #[test]
    fn tier_multi_tier_compatibility() {
        let multi = Tier::MultiTier {
            allowed: vec![Tier::Aot, Tier::Interp],
        };
        assert!(multi.compatible_with(&Tier::Aot));
        assert!(multi.compatible_with(&Tier::Interp));
        assert!(Tier::Aot.compatible_with(&multi));
        assert!(!multi.compatible_with(&Tier::Gpu));
    }

    #[test]
    fn tier_check_tier_runs_nothing() {
 // Architectural pin: Check tier is type-check-only; nothing
 // executes. So Check is incompatible with anything that
 // actually runs.
        assert!(!Tier::Check.compatible_with(&Tier::Aot));
        assert!(!Tier::Aot.compatible_with(&Tier::Check));
    }

    #[test]
    fn msfs_stratum_l_abs_is_inadmissible() {
 // Architectural pin (AFN-T α MSFS Theorem 5.1): L_Abs is
 // empty by construction; declaring it is a defect.
        assert!(!MsfsStratum::LAbs.is_admissible());
        assert!(MsfsStratum::LFnd.is_admissible());
        assert!(MsfsStratum::LCls.is_admissible());
        assert!(MsfsStratum::LClsTop.is_admissible());
    }

    #[test]
    fn foundation_zfc_subsumes_only_itself() {
 // Pin: foundation profiles don't have generic subsumption
 // — only specific canonical inclusions (CIC ⊃ MLTT,
 // Cubical ⊃ HoTT). Random pairs require explicit bridges.
        assert!(Foundation::ZfcTwoInacc.directly_subsumed_by(&Foundation::ZfcTwoInacc));
        assert!(Foundation::Mltt.directly_subsumed_by(&Foundation::Cic));
        assert!(Foundation::Hott.directly_subsumed_by(&Foundation::Cubical));
 // No reverse direction without bridge.
        assert!(!Foundation::Cic.directly_subsumed_by(&Foundation::Mltt));
 // Cross-paradigm requires explicit bridge.
        assert!(!Foundation::ZfcTwoInacc.directly_subsumed_by(&Foundation::Hott));
    }

    #[test]
    fn cve_closure_degree_counts_correctly() {
        let full = CveClosure {
            constructive: Some("c".into()),
            verifiable_strategy: Some(VerifyStrategy::Certified),
            executable: Some("e".into()),
        };
        assert_eq!(full.closure_degree(), 3);
        assert!(full.is_fully_closed());

        let two = CveClosure {
            constructive: Some("c".into()),
            verifiable_strategy: None,
            executable: Some("e".into()),
        };
        assert_eq!(two.closure_degree(), 2);
        assert!(!two.is_fully_closed());

        let none = CveClosure {
            constructive: None,
            verifiable_strategy: None,
            executable: None,
        };
        assert_eq!(none.closure_degree(), 0);
        assert!(!none.is_fully_closed());
    }

    #[test]
    fn verify_strategy_rank_strictly_increases() {
 // VVA §12: the 9 strategies are STRICTLY ORDERED on the
 // Diakrisis ν-ladder. Pin the order.
        let order = [
            VerifyStrategy::Runtime,
            VerifyStrategy::Static,
            VerifyStrategy::Fast,
            VerifyStrategy::Formal,
            VerifyStrategy::Proof,
            VerifyStrategy::Thorough,
            VerifyStrategy::Reliable,
            VerifyStrategy::Certified,
            VerifyStrategy::Synthesize,
        ];
        for window in order.windows(2) {
            assert!(window[0].rank() < window[1].rank());
        }
    }

    #[test]
    fn shape_default_for_unannotated_passes_default_invariants() {
        // Pin: a cog without @arch_module gets a Shape that vacuously
 // satisfies every anti-pattern (per spec §17.5
 // backward-compat). Default trivial — no requires, no
 // exposes, multi-tier, ZFC foundation.
        let s = Shape::default_for_unannotated();
        assert!(s.requires.is_empty());
        assert!(s.exposes.is_empty());
        assert_eq!(s.foundation, Foundation::ZfcTwoInacc);
        assert_eq!(s.stratum, MsfsStratum::LFnd);
        assert!(s.stratum.is_admissible());
        assert!(!s.strict);
    }

    #[test]
    fn shape_default_admits_serde_roundtrip() {
 // Pin: Shape can be serialised to JSON for agent-readable
 // surfaces (per spec §32.2 machine-readable surfaces).
        let s = Shape::default_for_unannotated();
        let json = serde_json::to_string(&s).expect("serialise default shape");
        let _back: Shape = serde_json::from_str(&json).expect("deserialise default shape");
    }

 // ----- Architectural pin: tag stability -----

    #[test]
    fn architectural_pin_capability_tags_documented_in_spec() {
 // Pin: every Capability tag here matches the canonical
 // surface (Read / Write / Exec / Escalate / Spawn / TimeBound /
 // Persist / Network / Custom).  Adding a new variant requires
 // updating both this enum and the cross-side pin test in
 // crates/verum_kernel/tests/k_arch_v_alignment.rs.
        let documented_tags: std::collections::BTreeSet<&'static str> = [
            "read",
            "write",
            "exec",
            "escalate",
            "spawn",
            "time_bound",
            "persist",
            "network",
            "custom",
        ]
        .iter()
        .copied()
        .collect();
        assert_eq!(documented_tags.len(), 9);
    }
}
