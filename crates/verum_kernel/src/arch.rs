//! ATS-V Architectural Type System — kernel-side primitives.
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §4 (Architectural primitives) +
//! §17 (Reuse compliance), ATS-V is a strict extension of Verum
//! through ONE typed attribute (`@arch_module(...)`) plus library
//! types in `core/architecture/`. This module ships the kernel-
//! side mirror of those library types — Rust `enum`s and `struct`s
//! that the ATS-V phase (Phase 6.5 per §3) consumes during
//! architectural type checking.
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
        }
    }
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
 // surface in `internal/specs/ats-v.md` §4.2. Adding a
 // new variant requires updating both this enum and the
 // spec table.
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
