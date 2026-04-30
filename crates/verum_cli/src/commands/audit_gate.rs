//! Unified audit-gate trait.
//!
//! Pre-#169, `crates/verum_cli/src/commands/audit.rs` carried 45
//! ad-hoc `audit_X_with_format(format: AuditFormat) -> Result<()>`
//! free functions.  Each gate had its own per-function entry point,
//! its own dispatch in `main.rs`, and its own JSON-emission shape.
//! Adding a new audit dimension meant editing audit.rs, main.rs,
//! and the CLI argument parser — three coupled edit sites.
//!
//! Post-#169, every gate implements [`AuditGate`].  The CLI
//! dispatch becomes a registry lookup; the JSON-emission contract
//! is enforced by the trait; new gates plug in by registering an
//! instance.  The 45 free functions become trait implementations
//! incrementally — the migration pattern is documented below.
//!
//! ## Migration pattern
//!
//! Each existing `audit_<name>_with_format(format)` function
//! becomes a unit-struct implementing [`AuditGate`]:
//!
//! ```ignore
//! pub struct FrameworkAxiomsGate;
//!
//! impl AuditGate for FrameworkAxiomsGate {
//!     fn name(&self) -> &'static str { "framework-axioms" }
//!     fn description(&self) -> &'static str {
//!         "Enumerate every @framework(<corpus>, \"<citation>\") marker in the project."
//!     }
//!     fn run(&self, format: AuditFormat) -> crate::error::Result<()> {
//!         super::audit::audit_framework_axioms_with_format(format)
//!     }
//! }
//! ```
//!
//! Wrapping the existing function preserves all current behaviour;
//! a future pass can inline the body and delete the free function.
//!
//! ## Dispatch
//!
//! [`AuditRegistry::default()`] returns a registry pre-populated
//! with every migrated gate.  `main.rs` resolves a `--gate <name>`
//! argument via [`AuditRegistry::get`] and calls `run(format)`.
//! Unknown gate names return [`AuditDispatchError::UnknownGate`].

use std::collections::BTreeMap;

use crate::error::Result;

/// Output format selector for audit gates.  Re-exported here so
/// the trait surface is self-contained; mirrors
/// [`super::audit::AuditFormat`].
pub use super::audit::AuditFormat;

/// **One audit gate** — checks a single L4-relevant invariant of
/// the Verum corpus / kernel / project state and emits a report.
///
/// Implementations are typically zero-sized unit structs; the
/// gate's logic is deterministic from the project state on disk
/// (no per-instance state).
pub trait AuditGate {
    /// Stable identifier — kebab-case, used as the CLI flag name
    /// (e.g. `--framework-axioms`, `--kernel-rules`).
    fn name(&self) -> &'static str;

    /// One-line human-readable description for `verum audit --help`.
    fn description(&self) -> &'static str;

    /// Run the gate.  Emits output in the requested format on
    /// stdout (Plain) or as machine-parseable JSON (Json).  Returns
    /// `Ok(())` iff the audit invariant holds; non-zero exit on
    /// invariant violation.
    fn run(&self, format: AuditFormat) -> Result<()>;
}

/// Errors from the registry dispatch layer.
#[derive(Debug)]
pub enum AuditDispatchError {
    /// Requested gate name doesn't appear in the registry.
    UnknownGate(String),
}

impl std::fmt::Display for AuditDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditDispatchError::UnknownGate(name) => {
                write!(f, "unknown audit gate: {}", name)
            }
        }
    }
}

impl std::error::Error for AuditDispatchError {}

/// Registry of audit gates.  Lookup by stable name.
pub struct AuditRegistry {
    gates: BTreeMap<&'static str, Box<dyn AuditGate>>,
}

impl AuditRegistry {
    /// Construct an empty registry.  Callers register gates via
    /// [`Self::register`].  Most consumers use [`Self::default`]
    /// which returns a registry pre-populated with every migrated
    /// gate.
    pub fn new() -> Self {
        Self {
            gates: BTreeMap::new(),
        }
    }

    /// Register a gate.  Replaces any prior gate registered under
    /// the same name (last-write-wins).
    pub fn register(&mut self, gate: Box<dyn AuditGate>) {
        self.gates.insert(gate.name(), gate);
    }

    /// Look up a gate by stable name.
    pub fn get(&self, name: &str) -> Option<&dyn AuditGate> {
        self.gates.get(name).map(|g| g.as_ref())
    }

    /// Enumerate every registered gate's name + description.
    pub fn list(&self) -> Vec<(&'static str, &'static str)> {
        self.gates
            .iter()
            .map(|(name, gate)| (*name, gate.description()))
            .collect()
    }

    /// Total registered gate count.
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    /// Run a gate by name.  Returns
    /// [`AuditDispatchError::UnknownGate`] when the name doesn't
    /// resolve.
    pub fn run(
        &self,
        name: &str,
        format: AuditFormat,
    ) -> std::result::Result<Result<()>, AuditDispatchError> {
        let gate = self
            .get(name)
            .ok_or_else(|| AuditDispatchError::UnknownGate(name.to_string()))?;
        Ok(gate.run(format))
    }
}

impl Default for AuditRegistry {
    fn default() -> Self {
        let mut r = Self::new();
        // Full migration of all 24 audit gates.  Each gate is one
        // unit struct + one `impl AuditGate` block; the trait impl
        // routes through the existing `audit::audit_*_with_format`
        // function, so all current behaviour is preserved.
        r.register(Box::new(AccessibilityGate));
        r.register(Box::new(ApplyGraphGate));
        r.register(Box::new(BridgeAdmitsGate));
        r.register(Box::new(BridgeDischargeGate));
        r.register(Box::new(BundleGate));
        r.register(Box::new(CoherentGate));
        r.register(Box::new(CoordGate));
        r.register(Box::new(CoordConsistencyGate));
        r.register(Box::new(CrossFormatRoundtripGate));
        r.register(Box::new(EpsilonGate));
        r.register(Box::new(FrameworkAxiomsGate));
        r.register(Box::new(FrameworkConflictsGate));
        r.register(Box::new(FrameworkSoundnessGate));
        r.register(Box::new(HygieneGate));
        r.register(Box::new(HygieneStrictGate));
        r.register(Box::new(KernelRechecksGate));
        r.register(Box::new(KernelSoundnessGate));
        r.register(Box::new(LadderMonotonicityGate));
        r.register(Box::new(Owl2ClassifyGate));
        r.register(Box::new(ProofHonestyGate));
        r.register(Box::new(ProofTermLibraryGate));
        r.register(Box::new(RoundTripGate));
        r.register(Box::new(SignaturesGate));
        r.register(Box::new(SoundnessIouGate));
        r
    }
}

// =============================================================================
// Audit gates — one struct + impl per gate.  Each impl wraps the
// existing `audit::audit_*_with_format` free function; future
// inlining passes can move the body and delete the free function.
// =============================================================================

/// `verum audit --accessibility` — accessibility audit of public surfaces.
pub struct AccessibilityGate;
impl AuditGate for AccessibilityGate {
    fn name(&self) -> &'static str { "accessibility" }
    fn description(&self) -> &'static str {
        "Accessibility audit: report public surfaces missing visibility annotations."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_accessibility_with_format(format)
    }
}

/// `verum audit --apply-graph` — transitive bridge-discharge walker.
pub struct ApplyGraphGate;
impl AuditGate for ApplyGraphGate {
    fn name(&self) -> &'static str { "apply-graph" }
    fn description(&self) -> &'static str {
        "Walk the apply-graph DFS-style; classify every leaf (kernel_strict / framework_axiom / placeholder_axiom / unresolved)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_apply_graph_with_format(format)
    }
}

/// `verum audit --bridge-admits` — track @kernel_discharge admit roster.
pub struct BridgeAdmitsGate;
impl AuditGate for BridgeAdmitsGate {
    fn name(&self) -> &'static str { "bridge-admits" }
    fn description(&self) -> &'static str {
        "Enumerate every @kernel_discharge admit, listing the bridging dispatcher intrinsic and its discharge status."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_bridge_admits_with_format(format)
    }
}

/// `verum audit --bridge-discharge` — kernel-bridge resolution audit.
pub struct BridgeDischargeGate;
impl AuditGate for BridgeDischargeGate {
    fn name(&self) -> &'static str { "bridge-discharge" }
    fn description(&self) -> &'static str {
        "Audit the @kernel_discharge bridge between Verum theorems and the dispatcher's intrinsic verifiers."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_bridge_discharge_with_format(format)
    }
}

/// `verum audit --bundle` — composed L4-load-bearing dispatcher.
pub struct BundleGate;
impl AuditGate for BundleGate {
    fn name(&self) -> &'static str { "bundle" }
    fn description(&self) -> &'static str {
        "Composed audit bundle: runs every load-bearing gate in dependency order; emits unified verdict."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_bundle_with_format(format)
    }
}

/// `verum audit --coherent` — coherence-condition audit.
pub struct CoherentGate;
impl AuditGate for CoherentGate {
    fn name(&self) -> &'static str { "coherent" }
    fn description(&self) -> &'static str {
        "Audit categorical coherence conditions (associator / unitor / pentagon / triangle)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_coherent_with_format(format)
    }
}

/// `verum audit --coord` — coordinate-system audit.
pub struct CoordGate;
impl AuditGate for CoordGate {
    fn name(&self) -> &'static str { "coord" }
    fn description(&self) -> &'static str {
        "Coordinate-system audit: verify diakrisis / cohesive-triple / framework consistency."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_coord_with_format(format)
    }
}

/// `verum audit --coord-consistency` — coordinate consistency audit.
pub struct CoordConsistencyGate;
impl AuditGate for CoordConsistencyGate {
    fn name(&self) -> &'static str { "coord-consistency" }
    fn description(&self) -> &'static str {
        "Coordinate-consistency audit: cross-coordinate-system invariants."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_coord_consistency_with_format(format)
    }
}

/// `verum audit --cross-format-roundtrip` — emit + foreign re-check.
pub struct CrossFormatRoundtripGate;
impl AuditGate for CrossFormatRoundtripGate {
    fn name(&self) -> &'static str { "cross-format-roundtrip" }
    fn description(&self) -> &'static str {
        "Emit corpus theorems to Coq/Lean; re-check via the foreign toolchain (native or Docker)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_cross_format_roundtrip_with_format(format)
    }
}

/// `verum audit --epsilon` — ε-parameter audit.
pub struct EpsilonGate;
impl AuditGate for EpsilonGate {
    fn name(&self) -> &'static str { "epsilon" }
    fn description(&self) -> &'static str {
        "ε-parameter audit: scan diakrisis epsilon-rule applications for consistency."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_epsilon_with_format(format)
    }
}

/// `verum audit --framework-axioms` — @framework citation enumeration.
pub struct FrameworkAxiomsGate;
impl AuditGate for FrameworkAxiomsGate {
    fn name(&self) -> &'static str { "framework-axioms" }
    fn description(&self) -> &'static str {
        "Enumerate every @framework(<corpus>, \"<citation>\") marker in the project, grouping by corpus."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_framework_axioms_with_format(format)
    }
}

/// `verum audit --framework-conflicts` — contradictory-axiom detector.
pub struct FrameworkConflictsGate;
impl AuditGate for FrameworkConflictsGate {
    fn name(&self) -> &'static str { "framework-conflicts" }
    fn description(&self) -> &'static str {
        "Detect contradictory framework axioms (e.g. univalence + uip declared simultaneously)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_framework_conflicts_with_format(format)
    }
}

/// `verum audit --framework-soundness` — framework citation soundness.
pub struct FrameworkSoundnessGate;
impl AuditGate for FrameworkSoundnessGate {
    fn name(&self) -> &'static str { "framework-soundness" }
    fn description(&self) -> &'static str {
        "Soundness audit: every @framework citation resolves to a legitimate upstream proof or axiom."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_framework_soundness_with_format(format)
    }
}

/// `verum audit --hygiene` — macro-hygiene audit.
pub struct HygieneGate;
impl AuditGate for HygieneGate {
    fn name(&self) -> &'static str { "hygiene" }
    fn description(&self) -> &'static str {
        "Macro-hygiene audit: detect accidental capture / undeclared captures in macro expansions."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_hygiene_with_format(format)
    }
}

/// `verum audit --hygiene-strict` — strict macro-hygiene audit.
pub struct HygieneStrictGate;
impl AuditGate for HygieneStrictGate {
    fn name(&self) -> &'static str { "hygiene-strict" }
    fn description(&self) -> &'static str {
        "Strict macro-hygiene audit: same as --hygiene but rejects on every violation (no warnings)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_hygiene_strict_with_format(format)
    }
}

/// `verum audit --kernel-recheck` — kernel re-check audit.
pub struct KernelRechecksGate;
impl AuditGate for KernelRechecksGate {
    fn name(&self) -> &'static str { "kernel-recheck" }
    fn description(&self) -> &'static str {
        "Kernel re-check audit: walk every theorem with @verify(formal) and re-discharge its proof body via the kernel."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_kernel_recheck_with_format(format)
    }
}

/// `verum audit --kernel-soundness` — per-rule soundness-lemma status.
pub struct KernelSoundnessGate;
impl AuditGate for KernelSoundnessGate {
    fn name(&self) -> &'static str { "kernel-soundness" }
    fn description(&self) -> &'static str {
        "Walk every kernel rule's per-rule soundness lemma; report Proved / Admitted / DischargedByFramework status."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_kernel_soundness_with_format(format)
    }
}

/// `verum audit --ladder-monotonicity` — verify-ladder monotonicity.
pub struct LadderMonotonicityGate;
impl AuditGate for LadderMonotonicityGate {
    fn name(&self) -> &'static str { "ladder-monotonicity" }
    fn description(&self) -> &'static str {
        "Verification-ladder monotonicity: no theorem proves at L_n if it doesn't also prove at L_{n+1}."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_ladder_monotonicity_with_format(format)
    }
}

/// `verum audit --owl2-classify` — OWL2 ontology classification audit.
pub struct Owl2ClassifyGate;
impl AuditGate for Owl2ClassifyGate {
    fn name(&self) -> &'static str { "owl2-classify" }
    fn description(&self) -> &'static str {
        "OWL2 classification audit: walk ontology files and report DL-fragment classification."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_owl2_classify_with_format(format)
    }
}

/// `verum audit --proof-honesty` — admitted-proof reporting.
pub struct ProofHonestyGate;
impl AuditGate for ProofHonestyGate {
    fn name(&self) -> &'static str { "proof-honesty" }
    fn description(&self) -> &'static str {
        "Proof-honesty audit: every theorem with `Admitted.`/`sorry` carries a structured admit reason."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_proof_honesty_with_format(format)
    }
}

/// `verum audit --proof-term-library` — canonical-certificate library audit.
pub struct ProofTermLibraryGate;
impl AuditGate for ProofTermLibraryGate {
    fn name(&self) -> &'static str { "proof-term-library" }
    fn description(&self) -> &'static str {
        "Walk core/verify/proof_term_examples/; verify every canonical certificate via the minimal kernel."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_proof_term_library_with_format(format)
    }
}

/// `verum audit --round-trip` — emit + re-import round-trip audit.
pub struct RoundTripGate;
impl AuditGate for RoundTripGate {
    fn name(&self) -> &'static str { "round-trip" }
    fn description(&self) -> &'static str {
        "Round-trip audit: emit corpus theorems to a foreign format, re-import, and confirm equivalence."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_round_trip_with_format(format)
    }
}

/// `verum audit --signatures` — provenance-signature audit.
pub struct SignaturesGate;
impl AuditGate for SignaturesGate {
    fn name(&self) -> &'static str { "signatures" }
    fn description(&self) -> &'static str {
        "Verify every cross-format-emitted file's `verum_signature: <kernel_version>:<blake3>` header pins to the source state."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_signatures_with_format(format)
    }
}

/// `verum audit --soundness-iou` — IOU dashboard.
pub struct SoundnessIouGate;
impl AuditGate for SoundnessIouGate {
    fn name(&self) -> &'static str { "soundness-iou" }
    fn description(&self) -> &'static str {
        "IOU dashboard: per-kernel-rule soundness-lemma admit reasons, grouped by RuleCategory."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_soundness_iou_with_format(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_default_includes_every_gate() {
        let r = AuditRegistry::default();
        for name in [
            "accessibility",
            "apply-graph",
            "bridge-admits",
            "bridge-discharge",
            "bundle",
            "coherent",
            "coord",
            "coord-consistency",
            "cross-format-roundtrip",
            "epsilon",
            "framework-axioms",
            "framework-conflicts",
            "framework-soundness",
            "hygiene",
            "hygiene-strict",
            "kernel-recheck",
            "kernel-soundness",
            "ladder-monotonicity",
            "owl2-classify",
            "proof-honesty",
            "proof-term-library",
            "round-trip",
            "signatures",
            "soundness-iou",
        ] {
            assert!(
                r.get(name).is_some(),
                "default registry must include `{}`",
                name,
            );
        }
        assert_eq!(r.len(), 24, "expected 24 gates in the default registry");
    }

    #[test]
    fn registry_unknown_gate_returns_dispatch_error() {
        let r = AuditRegistry::default();
        match r.run("nonexistent-gate", AuditFormat::Plain) {
            Err(AuditDispatchError::UnknownGate(name)) => {
                assert_eq!(name, "nonexistent-gate");
            }
            other => panic!("expected UnknownGate, got {:?}", other),
        }
    }

    #[test]
    fn registry_list_returns_name_description_pairs() {
        let r = AuditRegistry::default();
        let entries = r.list();
        assert_eq!(entries.len(), 24);
        for (name, desc) in &entries {
            assert!(!name.is_empty());
            assert!(!desc.is_empty());
            assert!(desc.len() > 20, "description should be one line");
        }
    }

    #[test]
    fn gate_names_are_kebab_case() {
        let r = AuditRegistry::default();
        for (name, _) in r.list() {
            for c in name.chars() {
                assert!(
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                    "gate name '{}' should be lowercase-kebab-case (digits allowed)",
                    name,
                );
            }
        }
    }

    #[test]
    fn gate_names_are_distinct() {
        let r = AuditRegistry::default();
        let names: std::collections::BTreeSet<_> = r.list().iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), r.len(), "every gate must have a distinct name");
    }

    #[test]
    fn empty_registry_is_empty() {
        let r = AuditRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn register_replaces_existing_entry() {
        let mut r = AuditRegistry::new();
        r.register(Box::new(FrameworkAxiomsGate));
        r.register(Box::new(FrameworkAxiomsGate));
        assert_eq!(r.len(), 1, "second register replaces first");
    }
}
