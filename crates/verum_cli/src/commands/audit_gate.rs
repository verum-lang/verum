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
        // Initial migration batch — proof of concept covering the
        // most-used L4 gates.  Future passes wrap the remaining
        // ~40 ad-hoc `audit_X_with_format` functions; each is one
        // unit struct + impl AuditGate per gate.
        r.register(Box::new(FrameworkAxiomsGate));
        r.register(Box::new(FrameworkConflictsGate));
        r.register(Box::new(KernelSoundnessGate));
        r.register(Box::new(BridgeDischargeGate));
        r.register(Box::new(SoundnessIouGate));
        r
    }
}

// =============================================================================
// Initial migration batch — wrappers over existing free functions.
// =============================================================================

/// `verum audit --framework-axioms`
pub struct FrameworkAxiomsGate;
impl AuditGate for FrameworkAxiomsGate {
    fn name(&self) -> &'static str {
        "framework-axioms"
    }
    fn description(&self) -> &'static str {
        "Enumerate every @framework(<corpus>, \"<citation>\") marker in the project, grouping by corpus."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_framework_axioms_with_format(format)
    }
}

/// `verum audit --framework-conflicts`
pub struct FrameworkConflictsGate;
impl AuditGate for FrameworkConflictsGate {
    fn name(&self) -> &'static str {
        "framework-conflicts"
    }
    fn description(&self) -> &'static str {
        "Detect contradictory framework axioms (e.g., univalence + uip declared simultaneously)."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_framework_conflicts_with_format(format)
    }
}

/// `verum audit --kernel-soundness`
pub struct KernelSoundnessGate;
impl AuditGate for KernelSoundnessGate {
    fn name(&self) -> &'static str {
        "kernel-soundness"
    }
    fn description(&self) -> &'static str {
        "Walk every kernel rule's per-rule soundness lemma; report Proved / Admitted / DischargedByFramework status."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_kernel_soundness_with_format(format)
    }
}

/// `verum audit --bridge-discharge`
pub struct BridgeDischargeGate;
impl AuditGate for BridgeDischargeGate {
    fn name(&self) -> &'static str {
        "bridge-discharge"
    }
    fn description(&self) -> &'static str {
        "Audit the @kernel_discharge bridge between Verum theorems and the dispatcher's intrinsic verifiers."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_bridge_discharge_with_format(format)
    }
}

/// `verum audit --soundness-iou`
pub struct SoundnessIouGate;
impl AuditGate for SoundnessIouGate {
    fn name(&self) -> &'static str {
        "soundness-iou"
    }
    fn description(&self) -> &'static str {
        "IOU dashboard: per-kernel-rule soundness-lemma admit reasons, grouped by RuleCategory."
    }
    fn run(&self, format: AuditFormat) -> Result<()> {
        super::audit::audit_soundness_iou_with_format(format)
    }
}

// =============================================================================
// Migration TODO
// =============================================================================
//
// The remaining ~40 audit-gate free functions in `audit.rs` follow
// the same wrapper pattern.  Each migration is mechanical:
//
//   1. Add a unit struct: `pub struct <Name>Gate;`
//   2. `impl AuditGate for <Name>Gate` with `name`, `description`,
//      `run` methods.
//   3. Register the gate in `AuditRegistry::default()`.
//
// Migrated batches should land as separate commits under
// `feat(verum_cli/audit_gate): migrate <gate-name> (#169)`.
//
// Once every gate is migrated, the inlining pass can move the
// gate body from the free function into the trait impl and
// delete the free function — closing the refactor.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_default_includes_initial_batch() {
        let r = AuditRegistry::default();
        assert!(r.get("framework-axioms").is_some());
        assert!(r.get("framework-conflicts").is_some());
        assert!(r.get("kernel-soundness").is_some());
        assert!(r.get("bridge-discharge").is_some());
        assert!(r.get("soundness-iou").is_some());
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
        assert_eq!(entries.len(), 5);
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
                    c.is_ascii_lowercase() || c == '-',
                    "gate name '{}' should be kebab-case",
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
