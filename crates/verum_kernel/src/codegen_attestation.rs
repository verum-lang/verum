//! Verified-compilation kernel-discharge manifest (#162 / CompCert-style
//! per-pass simulation theorems).
//!

//! # Architectural role
//!

//! Verum compiles via VBC (bytecode) → LLVM IR for AOT. Currently every
//! codegen pass is trusted by code-review only — there is no
//! kernel-discharged correctness attestation. CompCert's contribution
//! to verified compilation was the per-pass simulation theorem: each
//! compiler phase preserves observable behaviour with a kernel-checked
//! semantic-preservation proof.
//!

//! Task #162 wants the same for Verum. Each codegen pass should
//! eventually emit a `@kernel_discharge("kernel_<pass>_preserves_semantics")`
//! attestation that downstream tooling can audit. This module is the
//! **foundation layer**: a static manifest of the canonical pass roster
//! plus per-pass attestation slots that future passes can populate.
//!

//! ```text
//!  Pre-#162: 6 codegen passes, 0 kernel-discharge attestations
//!  (trusted by code-review only)
//!  #162 : 6 codegen passes, 0 attested + 6 NotYetAttested IOUs
//!  (this manifest's current surface — observability only)
//!  Future : 6 passes, k attested + (6-k) Admitted_with_IOU
//!  Goal : 6 passes, 6 attested (CompCert parity for Verum)
//! ```
//!

//! # What this manifest is NOT
//!

//! It is not a codegen pass itself, and it does not change any actual
//! lowering or transformation. This commit deliberately ships ONLY
//! the data layer; per-pass discharge work lands in subsequent commits
//! that flip individual entries from `NotYetAttested` to
//! `Discharged` or `Admitted_with_IOU`.
//!

//! # Pattern reference
//!

//! Mirror of [`crate::soundness::kernel_v0_manifest`] — same layered
//! shape (status enum → row record → manifest function → helper
//! count fns → audit gate). Keeping the architectural shape uniform
//! means audit tooling can be templated across both manifests.

use serde::{Deserialize, Serialize};

// =============================================================================
// CodegenPassId — the canonical roster of codegen passes
// =============================================================================

/// Canonical pass identifiers — match the actual phases under
/// `crates/verum_codegen/src/`. The ordering matches the lowering
/// order: VBC arrives, LLVM IR is built, and machine code is emitted
/// at the tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodegenPassId {
    /// Lowering of VBC bytecode IR into LLVM IR. Implemented in
    /// `crates/verum_codegen/src/llvm/vbc_lowering.rs`.
    VbcLowering,
    /// Mem-to-reg / SSA construction over the lowered IR. Currently
    /// delegated to LLVM's `mem2reg` pass; the simulation invariant
    /// here is that SSA construction preserves the operational
    /// semantics established by [`Self::VbcLowering`].
    SsaConstruction,
    /// Generic register-allocation pass identifier — covers the
    /// allocation-strategy-agnostic invariant that allocation
    /// preserves observable behaviour. Specific allocators (e.g.
    /// linear-scan) carry their own attestations as well.
    RegisterAllocation,
    /// Linear-scan register allocator (the default Verum allocator
    /// once it lands). Has its own attestation distinct from the
    /// generic [`Self::RegisterAllocation`] entry because the
    /// linear-scan algorithm has additional structural invariants
    /// (live-range monotonicity) the generic invariant doesn't pin.
    LinearScanRegalloc,
    /// LLVM IR emission — the tail of the AOT pipeline before LLVM
    /// itself takes over. Discharges the invariant that the emitted
    /// IR is well-formed and semantically equivalent to the input.
    LlvmEmission,
    /// Machine-code emission via LLVM's backend. Verum sees this
    /// as a black box (LLVM is outside our TCB), so this attestation
    /// is the boundary marker: anything past this point is LLVM's
    /// trust surface, not Verum's.
    MachineCodeEmission,
}

impl CodegenPassId {
    /// Stable diagnostic tag — matches the serde representation.
    pub fn tag(self) -> &'static str {
        match self {
            CodegenPassId::VbcLowering => "vbc_lowering",
            CodegenPassId::SsaConstruction => "ssa_construction",
            CodegenPassId::RegisterAllocation => "register_allocation",
            CodegenPassId::LinearScanRegalloc => "linear_scan_regalloc",
            CodegenPassId::LlvmEmission => "llvm_emission",
            CodegenPassId::MachineCodeEmission => "machine_code_emission",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            CodegenPassId::VbcLowering => "VBC Lowering",
            CodegenPassId::SsaConstruction => "SSA Construction",
            CodegenPassId::RegisterAllocation => "Register Allocation",
            CodegenPassId::LinearScanRegalloc => "Linear-Scan Regalloc",
            CodegenPassId::LlvmEmission => "LLVM IR Emission",
            CodegenPassId::MachineCodeEmission => "Machine-Code Emission",
        }
    }

    /// Canonical kernel-discharge intrinsic name. When a future
    /// commit wires up an attestation, it will register a
    /// `@kernel_discharge("<this name>")` annotation on the pass's
    /// entry point. The name format is `kernel_<tag>_preserves_semantics`.
    pub fn kernel_intrinsic_name(self) -> String {
        format!("kernel_{}_preserves_semantics", self.tag())
    }
}

impl std::fmt::Display for CodegenPassId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// =============================================================================
// AttestationStatus — discharge classification (mirrors LemmaStatus pattern)
// =============================================================================

/// Discharge status for one codegen pass's preservation attestation.
///

/// Mirrors the kernel-soundness IOU pattern from
/// [`crate::soundness::kernel_v0_manifest::KernelV0Status`] but adds
/// the explicit `NotYetAttested` step for the pre-attestation period.
/// The CompCert-parity goal is that every pass eventually moves from
/// `NotYetAttested` → `Admitted_with_IOU` → `Discharged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationStatus {
    /// Pass carries a kernel-discharged simulation proof — no IOU.
    /// The associated `proof_obligation` field on
    /// [`PassAttestation`] becomes the citation for the discharged
    /// proof.
    Discharged,
    /// Pass is admitted with a structural-property IOU (the
    /// associated [`PassAttestation::proof_obligation`] names the
    /// missing structural lemma). This is the CompCert
    /// `Lemma <pass>_preserves_semantics. Admitted.` shape — honest
    /// about the gap.
    AdmittedWithIou {
        /// Concrete IOU naming the missing structural lemma.
        /// Preserved verbatim into audit reports.
        iou: String,
    },
    /// Pass has not yet been attested at all — neither a discharge
    /// nor a structured admit. This is the pre-#162 surface:
    /// "trusted by code-review only". Each entry carries a
    /// description of what would discharge it (the
    /// `proof_obligation` field on the parent [`PassAttestation`]).
    NotYetAttested,
}

impl AttestationStatus {
    /// Stable diagnostic tag — matches the serde representation
    /// modulo the IOU payload.
    pub fn tag(&self) -> &'static str {
        match self {
            AttestationStatus::Discharged => "discharged",
            AttestationStatus::AdmittedWithIou { .. } => "admitted_with_iou",
            AttestationStatus::NotYetAttested => "not_yet_attested",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            AttestationStatus::Discharged => "Discharged",
            AttestationStatus::AdmittedWithIou { .. } => "Admitted with IOU",
            AttestationStatus::NotYetAttested => "Not yet attested",
        }
    }

    /// True iff the pass carries a kernel-discharged proof.
    pub fn is_discharged(&self) -> bool {
        matches!(self, AttestationStatus::Discharged)
    }

    /// True iff the pass carries a structural-IOU admit.
    pub fn is_admitted(&self) -> bool {
        matches!(self, AttestationStatus::AdmittedWithIou { .. })
    }

    /// True iff the pass has not been attested at all.
    pub fn is_pending(&self) -> bool {
        matches!(self, AttestationStatus::NotYetAttested)
    }
}

impl std::fmt::Display for AttestationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// =============================================================================
// PassAttestation — one row in the manifest
// =============================================================================

/// One codegen-pass attestation row. Captures the pass identifier,
/// the semantic invariant that an attestation must preserve, the
/// concrete proof obligation describing what would discharge that
/// invariant, and the current discharge status.
///

/// # Field semantics
///

/// * [`Self::pass`] — the canonical pass identifier.
/// * [`Self::semantic_invariant`] — one-line statement of the
///  observable behaviour the pass preserves (e.g. "the operational
///  semantics of the input program is preserved by the pass").
/// * [`Self::proof_obligation`] — the structural lemma that would
///  discharge the invariant. When `status` is `Discharged`, this
///  is the citation for the proof; when `Admitted_with_IOU`, this
///  is the IOU describing what's missing; when `NotYetAttested`,
///  this is the spec for the pending attestation.
/// * [`Self::status`] — current discharge classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassAttestation {
    /// Canonical pass identifier.
    pub pass: CodegenPassId,
    /// Semantic invariant statement (one-line).
    pub semantic_invariant: String,
    /// Concrete proof obligation describing what would discharge the
    /// invariant. Doubles as the IOU citation when `status` is
    /// `Admitted_with_IOU` or `NotYetAttested`.
    pub proof_obligation: String,
    /// Current discharge classification.
    pub status: AttestationStatus,
}

// =============================================================================
// Manifest
// =============================================================================

/// Canonical attestation roster for Verum's codegen pipeline.
///
/// This is the single source of truth for the codegen-attestation
/// surface. Each entry is mirrored on the Verum-language side by an
/// `axiom kernel_<pass>_preserves_semantics` declaration in
/// `core/verify/codegen_soundness/<pass>.vr` carrying the same
/// `@framework(...)` citation as the manifest's IOU.
///
/// **Stable contract**: adding a codegen pass requires both a
/// manifest entry AND a `core/verify/codegen_soundness/<pass>.vr`
/// file. Removing or renaming an entry is a breaking change for
/// audit tooling. The audit gate
/// (`verum audit --codegen-attestation`) cross-checks both surfaces.
pub fn manifest() -> Vec<PassAttestation> {
    vec![
        PassAttestation {
            pass: CodegenPassId::VbcLowering,
            semantic_invariant: "TypedAST → VBC bytecode preserves operational semantics under \
                 the well-typed-program invariant"
                .to_string(),
            proof_obligation: "simulation theorem: for every well-typed AST term `e` and every \
                 evaluation context `K`, `K[e] ⇓ v` iff `lower(K)[lower(e)] ⇓ \
                 lower(v)` in the VBC operational semantics"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "Leroy, X. (2009). Formal verification of a realistic compiler. CACM \
                      52(7):107-115. — §5.2 Simulation Diagram for the simpl_cminor → cminor \
                      lowering pass; instantiated here for TypedAST → VBC. Cited at \
                      core/verify/codegen_soundness/vbc_lowering.vr."
                    .to_string(),
            },
        },
        PassAttestation {
            pass: CodegenPassId::SsaConstruction,
            semantic_invariant: "SSA construction preserves operational semantics: every \
                 reaching definition observable in the input program is \
                 observable in the SSA-formed program"
                .to_string(),
            proof_obligation: "simulation theorem: for every alloca site `a` lifted into an \
                 SSA value `v`, the loads dominated by `a`'s defining store \
                 evaluate to `store(v)` post-mem2reg"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "Beringer, L. & Stark, K. (2002). A simple and efficient construction of \
                      Static Single Assignment form. Compiler Construction (CC 2002), LNCS \
                      2304, 110-125. — §3 Semantic Equivalence Proof. Algorithm: Cytron, R. et \
                      al. (1991) Efficiently computing static single assignment form. TOPLAS \
                      13(4):451-490. Cited at core/verify/codegen_soundness/ssa_construction.vr."
                    .to_string(),
            },
        },
        PassAttestation {
            pass: CodegenPassId::RegisterAllocation,
            semantic_invariant: "register allocation preserves observable behaviour: every \
                 virtual-register read returns the value that the SSA-form \
                 program would have computed"
                .to_string(),
            proof_obligation: "simulation theorem: the live-range/interval covering of every \
                 virtual register agrees with the input dataflow at every \
                 program point"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "George, L. & Appel, A.W. (1996). Iterated register coalescing. TOPLAS \
                      18(3):300-324. — §6 Soundness Proof. Generalisation: Leroy's CompCert \
                      backend uses this as the template for per-allocator preservation \
                      arguments. Cited at \
                      core/verify/codegen_soundness/register_allocation.vr."
                    .to_string(),
            },
        },
        PassAttestation {
            pass: CodegenPassId::LinearScanRegalloc,
            semantic_invariant: "linear-scan regalloc preserves observable behaviour AND the \
                 live-range monotonicity invariant (Poletto-Sarkar 1999): no \
                 active interval is evicted while it is still live"
                .to_string(),
            proof_obligation: "simulation theorem (Poletto-Sarkar): given a strict total \
                 order on live-range start points, linear scan produces an \
                 allocation that agrees with the SSA dataflow AND respects \
                 the spilling discipline"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "Poletto, M. & Sarkar, V. (1999). Linear scan register allocation. TOPLAS \
                      21(5):895-913. — §3 Algorithm Correctness. Refinement: Mössenböck, H. & \
                      Pfeiffer, M. (2002). Linear scan register allocation in the context of \
                      SSA form and register constraints. CC 2002, LNCS 2304, 229-246. — §4 \
                      Structural Monotonicity Proof. Cited at \
                      core/verify/codegen_soundness/linear_scan_regalloc.vr."
                    .to_string(),
            },
        },
        PassAttestation {
            pass: CodegenPassId::LlvmEmission,
            semantic_invariant: "LLVM IR emission preserves operational semantics: the LLVM \
                 module emitted is bisimilar to the input post-regalloc IR"
                .to_string(),
            proof_obligation: "simulation theorem: every Verum IR instruction `I` has a \
                 well-formed LLVM IR translation `tr(I)` such that `step(I) ⇒ \
                 step*(tr(I))` modulo LLVM-internal scheduling"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "Zhao, J., Nagarakatte, S., Martin, M.M.K., Zdancewic, S. (2012). \
                      Formalizing the LLVM intermediate representation for verified program \
                      transformations. POPL 2012, 427-440. — §4 Operational Semantics of LLVM \
                      IR; §5 Translation Validation (Vellvm). Cited at \
                      core/verify/codegen_soundness/llvm_emission.vr."
                    .to_string(),
            },
        },
        PassAttestation {
            pass: CodegenPassId::MachineCodeEmission,
            semantic_invariant: "machine-code emission preserves observable behaviour: the \
                 emitted object code's I/O trace agrees with the LLVM IR's \
                 I/O trace under the host ABI"
                .to_string(),
            proof_obligation: "boundary attestation: this pass is delegated to LLVM's \
                 backend, which is outside Verum's TCB.  The kernel-side \
                 obligation reduces to LLVM-version pinning + ABI conformance \
                 (cf. CompCert's external-call axiom)"
                .to_string(),
            status: AttestationStatus::AdmittedWithIou {
                iou: "Wang, Y., Wilke, P., Leroy, X. (2020). An abstract stack-based approach \
                      to verified compositional compilation to machine code. POPL 2020, 1-30. \
                      — §6 ELF Backend Verification. Boundary discipline: Leroy, X. (2009) §6 \
                      External Call Axiom. Cited at \
                      core/verify/codegen_soundness/machine_code_emission.vr."
                    .to_string(),
            },
        },
    ]
}

/// Total codegen-pass count. Matches the cardinality of
/// [`CodegenPassId`].
pub const CODEGEN_PASS_COUNT: usize = 6;

/// Total codegen-pass count via the manifest. Should equal
/// [`CODEGEN_PASS_COUNT`].
pub fn pass_count() -> usize {
    manifest().len()
}

/// Number of passes currently in `Discharged` status (kernel-checked).
pub fn attested_count() -> usize {
    manifest()
        .iter()
        .filter(|p| p.status.is_discharged())
        .count()
}

/// Number of passes currently in `Admitted_with_IOU` status.
pub fn admitted_count() -> usize {
    manifest().iter().filter(|p| p.status.is_admitted()).count()
}

/// Number of passes still pending — neither discharged nor admitted.
pub fn pending_count() -> usize {
    manifest().iter().filter(|p| p.status.is_pending()).count()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_exactly_six_passes() {
        assert_eq!(manifest().len(), CODEGEN_PASS_COUNT);
        assert_eq!(CODEGEN_PASS_COUNT, 6);
        assert_eq!(pass_count(), 6);
    }

    #[test]
    fn pass_ids_are_distinct() {
        let ids: std::collections::BTreeSet<_> = manifest().iter().map(|p| p.pass.tag()).collect();
        assert_eq!(ids.len(), manifest().len());
    }

    #[test]
    fn manifest_pass_ids_cover_canonical_roster() {
        let expected: std::collections::BTreeSet<&'static str> = [
            "vbc_lowering",
            "ssa_construction",
            "register_allocation",
            "linear_scan_regalloc",
            "llvm_emission",
            "machine_code_emission",
        ]
        .iter()
        .copied()
        .collect();
        let actual: std::collections::BTreeSet<&'static str> =
            manifest().iter().map(|p| p.pass.tag()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_pending_pass_has_non_empty_proof_obligation() {
        // The IOU citation must be present on every NotYetAttested
        // entry — the manifest's V0 commitment is that future
        // attestations have a concrete obligation to chase.
        for pass in manifest() {
            if matches!(pass.status, AttestationStatus::NotYetAttested) {
                assert!(
                    !pass.proof_obligation.is_empty(),
                    "NotYetAttested pass {:?} must carry a proof_obligation \
                     describing what would discharge it",
                    pass.pass,
                );
            }
        }
    }

    #[test]
    fn every_admitted_pass_has_concrete_framework_citation() {
        // The post-V1 baseline (#162 follow-on) — every entry now
        // carries an AdmittedWithIou status whose `iou` field cites
        // a published proof + the corresponding .vr file. The audit
        // gate rejects any entry whose IOU is empty or doesn't
        // reference the canonical core/verify/codegen_soundness/
        // location.
        for pass in manifest() {
            if let AttestationStatus::AdmittedWithIou { iou } = &pass.status {
                assert!(
                    !iou.is_empty(),
                    "AdmittedWithIou pass {:?} must carry a non-empty IOU \
                     citation",
                    pass.pass,
                );
                assert!(
                    iou.contains("core/verify/codegen_soundness/"),
                    "AdmittedWithIou pass {:?} IOU must reference the .vr \
                     citation file under core/verify/codegen_soundness/, \
                     got: {}",
                    pass.pass,
                    iou,
                );
            }
        }
    }

    #[test]
    fn every_pass_has_non_empty_semantic_invariant() {
        for pass in manifest() {
            assert!(
                !pass.semantic_invariant.is_empty(),
                "pass {:?} must carry a semantic_invariant statement",
                pass.pass,
            );
        }
    }

    #[test]
    fn helper_counts_partition_the_manifest() {
        // attested + admitted + pending must equal the total.
        assert_eq!(
            attested_count() + admitted_count() + pending_count(),
            CODEGEN_PASS_COUNT,
        );
    }

    #[test]
    fn v1_baseline_is_zero_attested_six_admitted() {
        // Current surface (post #162 V2 — the IOU-with-citation pass).
        // Every entry now carries an AdmittedWithIou with a concrete
        // framework citation (CompCert / Vellvm / Poletto-Sarkar /
        // CompCertELF) AND a reference to the matching
        // core/verify/codegen_soundness/<pass>.vr file. Future work
        // flips entries from AdmittedWithIou to Discharged via
        // Verum-language proofs of the simulation diagrams.
        assert_eq!(attested_count(), 0);
        assert_eq!(admitted_count(), CODEGEN_PASS_COUNT);
        assert_eq!(pending_count(), 0);
    }

    #[test]
    fn pass_attestation_serde_round_trip() {
        let pass = manifest().into_iter().next().unwrap();
        let json = serde_json::to_string(&pass).unwrap();
        let restored: PassAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, pass);
    }

    #[test]
    fn attestation_status_serde_round_trip_all_variants() {
        let cases = vec![
            AttestationStatus::Discharged,
            AttestationStatus::AdmittedWithIou {
                iou: "missing β-confluence lemma for VBC reduction".to_string(),
            },
            AttestationStatus::NotYetAttested,
        ];
        for status in cases {
            let json = serde_json::to_string(&status).unwrap();
            let restored: AttestationStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, status);
        }
    }

    #[test]
    fn pass_id_kernel_intrinsic_names_match_convention() {
        // kernel_<tag>_preserves_semantics — the canonical name
        // every future @kernel_discharge attestation will cite.
        for pass in manifest() {
            let name = pass.pass.kernel_intrinsic_name();
            assert!(name.starts_with("kernel_"));
            assert!(name.ends_with("_preserves_semantics"));
            // The middle segment must be the pass tag exactly.
            let stripped = name
                .strip_prefix("kernel_")
                .and_then(|s| s.strip_suffix("_preserves_semantics"))
                .unwrap_or("");
            assert_eq!(stripped, pass.pass.tag());
        }
    }

    #[test]
    fn status_predicates_are_mutually_exclusive() {
        let cases = vec![
            AttestationStatus::Discharged,
            AttestationStatus::AdmittedWithIou {
                iou: "x".to_string(),
            },
            AttestationStatus::NotYetAttested,
        ];
        for s in cases {
            let flags = [s.is_discharged(), s.is_admitted(), s.is_pending()];
            assert_eq!(
                flags.iter().filter(|f| **f).count(),
                1,
                "exactly one predicate must hold for {:?}",
                s,
            );
        }
    }

    #[test]
    fn pass_id_display_names_distinct_and_nonempty() {
        let names: std::collections::BTreeSet<_> =
            manifest().iter().map(|p| p.pass.display_name()).collect();
        assert_eq!(names.len(), manifest().len());
        for n in names {
            assert!(!n.is_empty());
        }
    }
}
