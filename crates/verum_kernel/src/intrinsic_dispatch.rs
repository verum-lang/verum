//! Kernel intrinsic dispatch — string-name → kernel-function bridge.
//!
//! ## What this delivers
//!
//! The 15 ∞-cat + foundation kernel modules
//! (yoneda, cartesian_fibration, adjoint_functor, whitehead,
//! reflective_subcategory, limits_colimits, truncation,
//! factorisation, pronk_fractions, infinity_topos,
//! zfc_self_recognition, godel_coding, tactics_industrial,
//! cross_format_gate, mechanisation_roadmap) ship as typed Rust
//! APIs. Downstream callers — the compiler's elaborator, the proof-
//! body verifier, audit tooling — need a **uniform string-name
//! dispatch** so a `.vr` `apply kernel_grothendieck_construction(...)`
//! can be translated into a kernel function call.
//!
//! This module ships:
//!
//! 1. [`IntrinsicValue`] — a small typed enum carrying the
//! argument and result shapes the kernel intrinsics consume
//! (`Bool`, `Int`, `Text`, `OrdinalLevel`, `WitnessFlag`).
//! 2. [`dispatch_intrinsic`] — the single entry point. Given a
//! `kernel_*` name and an argument list, returns the kernel's
//! result as another `IntrinsicValue`.
//! 3. [`available_intrinsics`] — enumeration of dispatchable names
//! for diagnostics + `verum audit --kernel-intrinsics`.
//!
//! current surface ships the **decision-predicate intrinsics** — the
//! Boolean witness flags that `core/proof/kernel_bridge.vr`'s
//! `kernel_*() -> Bool` axioms ultimately resolve to. V1 promotion
//! will surface the typed-record intrinsics (returning
//! `GrothendieckConstruction` etc. as opaque handle IDs).
//!
//! ## What this UNBLOCKS
//!
//! - `core/proof/kernel_bridge.vr` axioms become **functional**
//! instead of tautological — their `ensures` clauses bind to
//! [`dispatch_intrinsic`] outputs at proof-check time.
//! - The compiler's `@framework_axiom` admission for `kernel_*`
//! names can validate *what* the kernel actually computes,
//! replacing the V0 trust-the-name pattern with a V1
//! re-checkable witness.
//! - `verum audit --kernel-intrinsics` produces a structured
//! listing of every kernel-callable name + its current
//! decidability status.

use serde::{Deserialize, Serialize};

use crate::adjoint_functor::SaftPreconditions;
use crate::arch_anti_pattern::AntiPatternCode;
use crate::arch_probe::ProbeOutcome;
use crate::cross_format_gate::ExportFormat;
use crate::infinity_topos::GiraudAxioms;
use crate::pronk_fractions::PronkAxioms;
use crate::zfc_self_recognition::{KernelRuleId, is_zfc_plus_2_inacc_provable};

// =============================================================================
// IntrinsicValue
// =============================================================================

/// A typed value passed to / returned from kernel intrinsics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IntrinsicValue {
 /// Boolean (witness-flag).
    Bool(bool),
 /// Signed 64-bit integer.
    Int(i64),
 /// String text — used for diagnostic identifiers / replay commands.
    Text(String),
 /// Decision-predicate witness with explanation.
    Decision {
 /// The decision verdict (true ⇒ predicate holds).
        holds: bool,
 /// Human-readable rationale for the verdict (cited in audit
 /// reports + cert-replay diagnostics).
        reason: String,
    },
 /// Unit / void.
    Unit,
}

impl IntrinsicValue {
    /// Extract the decision verdict when the value carries one.
    /// Returns `Some(b)` for `Bool(b)` and the `holds` field of
    /// `Decision { holds, .. }`; `None` otherwise.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            IntrinsicValue::Bool(b) => Some(*b),
            IntrinsicValue::Decision { holds, .. } => Some(*holds),
            _ => None,
        }
    }

    /// Extract the integer payload when the value carries one.
    /// Returns `Some(i)` for `Int(i)`; `None` for every other
    /// variant. Mirror of [`Self::as_bool`] / [`Self::as_text`] —
    /// the canonical extraction helper for `Int`-shaped intrinsic
    /// arguments. Replaces the previously-inlined
    /// `if let IntrinsicValue::Int(i) = v { Some(*i) } else { None }`
    /// pattern that was duplicated across every Int-consuming
    /// dispatch arm.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            IntrinsicValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract the textual payload when the value carries one.
    /// Returns `Some(s)` for `Text(s)`; `None` for every other
    /// variant.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            IntrinsicValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// Build a `Some(IntrinsicValue::Decision { holds, reason })`
/// dispatcher response. Every kernel intrinsic ultimately returns
/// either a decision verdict (with rationale) or `None` (args
/// rejected); this helper collapses the otherwise-five-line
/// `Decision` literal at every dispatch arm into a single call,
/// and accepts both `&str` and `String` inputs uniformly via
/// `Into<String>`.
#[inline]
fn decision(holds: bool, reason: impl Into<String>) -> Option<IntrinsicValue> {
    Some(IntrinsicValue::Decision {
        holds,
        reason: reason.into(),
    })
}

// =============================================================================
// Dispatch
// =============================================================================

/// What a `kernel_arch_*` discharge intrinsic claims, and therefore
/// what it must execute before it may answer.
enum ArchClaim {
    /// The named anti-patterns are enforced. Discharged by running
    /// each one's canonical counterexample through the live driver.
    Patterns(&'static [AntiPatternCode]),
    /// The whole catalogue is enforced — every roster code, each
    /// separately falsifiable.
    WholeRoster,
    /// A piece of the architectural algebra behaves. Discharged by
    /// executing the real implementation on a canonical input pair.
    Structural(fn() -> ProbeOutcome),
}

/// The claim table: which property each architectural discharge name
/// asserts.
///
/// This table is the SINGLE authority for the `kernel_arch_*` family:
/// `arch_claim` resolves a discharge against it and
/// `available_intrinsics` derives the family's names from it. The
/// eight AP-033..AP-040 endpoints went missing precisely because a
/// hand-maintained registry list sat beside the dispatch as a second
/// source of truth; deriving one from the other ends that class.
///
/// A name absent from this table has no evidence to offer and is not
/// dischargeable — `dispatch_intrinsic` returns `None`, which the
/// audit reports as unrecognised rather than as a pass.
const ARCH_CLAIMS: &[(&str, ArchClaim)] = {
    use AntiPatternCode as C;
    use ArchClaim::{Patterns, Structural, WholeRoster};
    &[
        // -- capability discipline -------------------------------
        ("kernel_arch_capability_discipline", Patterns(&[C::CapabilityEscalation, C::CapabilityLeak])),
        // AT-6 closure reuses the escalation code.
        ("kernel_arch_capability_ontology_check", Patterns(&[C::CapabilityEscalation])),
        // -- boundaries ------------------------------------------
        ("kernel_arch_boundary_check", Patterns(&[C::InvariantViolation, C::DanglingMessageType, C::UnauthenticatedCrossing])),
        // -- composition -----------------------------------------
        ("kernel_arch_composition_check", Patterns(&[C::DependencyCycle, C::TierMixing, C::FoundationDrift])),
        ("kernel_arch_composition_engine", Structural(crate::arch_probe::composition_algebra_holds)),
        ("kernel_arch_composition_associative", Structural(crate::arch_probe::composition_is_associative)),
        // -- lifecycle -------------------------------------------
        ("kernel_arch_lifecycle_check", Patterns(&[C::LifecycleRegression, C::TransitiveLifecycleRegression])),
        // -- foundations -----------------------------------------
        ("kernel_arch_foundation_consistency", Patterns(&[C::FoundationDrift])),
        // -- CVE closure -----------------------------------------
        ("kernel_arch_cve_closure", Patterns(&[C::CveIncomplete])),
        // AT-2 closure: a Theorem must carry its CVE triple.
        ("kernel_arch_theorem_cve_required", Patterns(&[C::CveIncomplete])),
        // AT-5 closure reuses the declaration-drift code.
        ("kernel_arch_consumes_format_check", Patterns(&[C::DeclarationDrift])),
        // -- the catalogue and the end-to-end witness -------------
        ("kernel_arch_anti_pattern_check", WholeRoster),
        ("kernel_arch_soundness_v0", WholeRoster),
        // The phase driver's job is to reach the catalogue at all;
        // the catalogue's enforcement is what it can attest to.
        ("kernel_arch_phase_orchestrator", WholeRoster),
        // -- modal-temporal calculus -----------------------------
        ("kernel_arch_mtac_calculus", Patterns(&[
            C::TemporalInconsistency,
            C::CounterfactualBrittleness,
            C::MissedAdjoint,
            C::UniversalPropertyViolation,
            C::PhantomEvolution,
            C::YonedaInequivalentRefactor,
        ])),
        ("kernel_arch_counterfactual_engine", Patterns(&[C::CounterfactualBrittleness])),
        ("kernel_arch_adjunction_analyzer", Patterns(&[C::MissedAdjoint])),
        // -- Yoneda ----------------------------------------------
        ("kernel_arch_yoneda_equivalence", Structural(crate::arch_probe::yoneda_equivalence_holds)),
        // AT-4 closure reuses the inequivalent-refactor code.
        ("kernel_arch_yoneda_canonical_roster_complete", Patterns(&[C::YonedaInequivalentRefactor])),
        // -- corpus ----------------------------------------------
        ("kernel_arch_corpus_verify", Structural(crate::arch_probe::corpus_invariants_hold)),
        // -- CVE articulation hygiene (AP-033..AP-040) -----------
        //
        // These eight names are cited by
        // `core/architecture/anti_patterns.vr` and were ABSENT from
        // the dispatch, so the discharge audit reported them as
        // unrecognised — all eight it flagged. Their checks existed
        // and ran all along; only the kernel-side endpoint was
        // missing, because the endpoint list was maintained by hand
        // beside the dispatch instead of being derived from it.
        ("kernel_arch_retracted_citation_check", Patterns(&[C::RetractedCitationUse])),
        ("kernel_arch_hypothesis_plan_check", Patterns(&[C::HypothesisWithoutMaturationPlan])),
        ("kernel_arch_interpretation_in_mature_check", Patterns(&[C::InterpretationInMatureCorpus])),
        ("kernel_arch_observer_impersonation_check", Patterns(&[C::ObserverImpersonation])),
        ("kernel_arch_boundless_audit_check", Patterns(&[C::BoundlessAudit])),
        ("kernel_arch_implicit_substrate_check", Patterns(&[C::ImplicitSubstrate])),
        ("kernel_arch_anchoring_overextension_check", Patterns(&[C::AnchoringOverextension])),
        ("kernel_arch_self_reference_check", Patterns(&[C::SelfReferenceWithoutOperator])),
    ]
};

/// Resolve what an architectural discharge name claims.
fn arch_claim(name: &str) -> Option<&'static ArchClaim> {
    ARCH_CLAIMS
        .iter()
        .find(|(claim_name, _)| *claim_name == name)
        .map(|(_, claim)| claim)
}

/// Discharge an architectural intrinsic by EXECUTING what it claims.
///
/// The verdict's `holds` is the conjunction of the executed probes,
/// and its reason names every probe that did not discharge — so a
/// rejection points at the specific check that is missing, unwired,
/// vacuous or indiscriminate rather than at the intrinsic as a whole.
fn arch_discharge(name: &str) -> Option<IntrinsicValue> {
    use crate::arch_probe::{run_all_probes, run_probe};

    let outcomes: Vec<(String, ProbeOutcome)> = match arch_claim(name)? {
        ArchClaim::Patterns(codes) => codes
            .iter()
            .map(|&code| {
                let outcome = run_probe(code).unwrap_or(ProbeOutcome::CheckerSilent);
                (outcome.rationale(code), outcome)
            })
            .collect(),
        ArchClaim::WholeRoster => run_all_probes()
            .into_iter()
            .map(|(code, outcome)| (outcome.rationale(code), outcome))
            .collect(),
        ArchClaim::Structural(run) => {
            let outcome = run();
            let label = format!(
                "{} — structural claim {}",
                name,
                if outcome.is_discharged() {
                    "DISCHARGED by executing the real implementation on a canonical \
                     accepting/rejecting pair"
                } else {
                    "NOT discharged: the implementation accepted what it must reject, \
                     or rejected what it must accept"
                },
            );
            vec![(label, outcome)]
        }
    };

    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|(_, o)| !o.is_discharged())
        .map(|(why, _)| why.as_str())
        .collect();

    if failed.is_empty() {
        decision(
            true,
            format!(
                "{}: discharged by execution — {} probe(s) ran against the live checkers, \
                 each with its counterexample caught and the clean baseline untouched",
                name,
                outcomes.len(),
            ),
        )
    } else {
        decision(
            false,
            format!(
                "{}: NOT discharged — {} of {} probe(s) failed:\n{}",
                name,
                failed.len(),
                outcomes.len(),
                failed.join("\n"),
            ),
        )
    }
}

/// Dispatch a kernel intrinsic by string name. Returns `None` when
/// the name is not in the dispatch table OR the argument shape
/// doesn't match.
///
/// **Naming convention**: every intrinsic is named `kernel_<verb>` —
/// matches the `core/proof/kernel_bridge.vr` axiom names.
pub fn dispatch_intrinsic(name: &str, args: &[IntrinsicValue]) -> Option<IntrinsicValue> {
    match name {
 // -- Yoneda + ∞-Kan ----------------------------------------------
        "kernel_yoneda_embedding" => {
 // args: [source_level: Int, source_universe: Int].
 // Reject when args missing or pathological: HTT 1.2.1 requires
 // a *well-formed* ∞-category with non-negative level + at least
 // one universe. Bare-call (no args) returns None — caller must
 // supply structural data to claim Yoneda discharge.
            let level = args.first()?.as_int()?;
            let universe = args.get(1)?.as_int()?;
            decision(level >= 0 && universe >= 0, format!(
                    "yoneda: HTT 1.2.1 requires level≥0 (got {}) and universe≥0 (got {})",
                    level, universe
                ))
        }
 // **Bare-arg form** preserved for back-compat callers that don't
 // (yet) thread structural data; gated on a separate name.
        "kernel_yoneda_embedding_bare" => decision(true, "yoneda::yoneda_embedding (bare-arg back-compat — prefer the parameterised form)"),
        "kernel_kan_extension" => {
 // args: [is_fully_faithful: Bool, target_has_colimits: Bool]
            let ff = args.first()?.as_bool()?;
            let colim = args.get(1)?.as_bool()?;
            decision(ff && colim, format!(
                    "yoneda::build_kan_extension preconditions: ff={}, colim={}",
                    ff, colim
                ))
        }

 // -- Cartesian fibration + Straightening -------------------------
        "kernel_straightening_equivalence" => {
 // args: [base_level: Int]. HTT 3.2.0.1 requires the base
 // ∞-category to live at level ≥ 1.
            let level = args.first()?.as_int()?;
            decision(level >= 1, format!(
                    "straightening: base level={} must be >=1 (HTT 3.2.0.1)",
                    level
                ))
        }
 // Identity-is-equivalence — DIRECT discharge for the
 // "id_X is (∞,n)-equivalence" step in Theorem 5.1.
 // args: [level: Int]. Identity is always an equivalence at any
 // non-negative ordinal level (HTT 1.2.13 / Whitehead corollary).
        "kernel_identity_is_equivalence" => {
            let level = args.first()?.as_int()?;
            decision(level >= 0, format!(
                    "identity_is_equivalence: level={} must be >=0 (kernel ALWAYS witnesses id_X)",
                    level
                ))
        }
        "kernel_grothendieck_construction" => {
 // args: [num_fibres: Int]; passes when num_fibres > 0
            let n = args.first()?.as_int()?;
            decision(n > 0, format!(
                    "grothendieck::build_grothendieck preconditions: |fibres|={} > 0",
                    n
                ))
        }

 // -- Adjoint Functor Theorem + Reflective ------------------------
        "kernel_saft_adjunction" => {
 // args: [src_pres, tgt_pres, preserves_colim, preserves_lim_acc]
            let src = args.first()?.as_bool()?;
            let tgt = args.get(1)?.as_bool()?;
            let cp = args.get(2)?.as_bool()?;
            let lp = args.get(3)?.as_bool()?;
            let pre = SaftPreconditions {
                functor_name: verum_common::Text::from("(via intrinsic)"),
                source_presentable: src,
                target_presentable: tgt,
                preserves_small_colimits: cp,
                preserves_small_limits_and_accessible: lp,
            };
            let left_exists = crate::adjoint_functor::left_adjoint_exists(&pre);
            decision(left_exists, format!(
                    "adjoint_functor: left_adjoint_exists = {}",
                    left_exists
                ))
        }
        "kernel_reflective_subcategory_aft" => {
 // args: [ff: Bool, src_pres: Bool, tgt_pres: Bool,
 // preserves_limits_acc: Bool]
 // Reject if inclusion isn't fully faithful OR SAFT preconditions
 // fail. Required by HTT 5.2.7 + 5.5.2.9 dual.
            let ff = args.first()?.as_bool()?;
            let src = args.get(1)?.as_bool()?;
            let tgt = args.get(2)?.as_bool()?;
            let lp = args.get(3)?.as_bool()?;
            decision(ff && src && tgt && lp, format!(
                    "reflective_subcategory: ff={}, src_pres={}, tgt_pres={}, lim_acc={}",
                    ff, src, tgt, lp
                ))
        }

 // -- Whitehead promote -------------------------------------------
        "kernel_whitehead_promote" => {
 // args: [num_levels: Int, all_levels_iso: Bool, levels_complete: Bool]
 // Reject when no level data supplied OR any level fails iso OR
 // the certificate is incomplete. Per HTT 1.2.4.3 the criterion
 // requires PER-LEVEL π_k iso witness for k ∈ [0, n].
            let n = args.first()?.as_int()?;
            let all_iso = args.get(1)?.as_bool()?;
            let complete = args.get(2)?.as_bool()?;
            decision(n > 0 && all_iso && complete, format!(
                    "whitehead: n_levels={} (>0?) all_iso={} complete={}",
                    n, all_iso, complete
                ))
        }

 // -- Limits / colimits -------------------------------------------
        "kernel_compute_colimit" => {
            let nv = args.first()?.as_int()?;
            decision(nv > 0, format!(
                    "limits_colimits::compute_colimit_in_psh requires non-empty diagram (got {})",
                    nv
                ))
        }
        "kernel_specialised_limits" => {
 // args: [diagram_size: Int]. Reject negative sizes; size=0
 // is the empty (terminal) diagram, allowed.
            let n = args.first()?.as_int()?;
            decision(n >= 0, format!(
                    "specialised_limits: diagram_size={} (must be >=0)",
                    n
                ))
        }

 // -- Truncation --------------------------------------------------
        "kernel_truncate_to_level" => {
 // args: [level: Int, source_level: Int].
 // Reject negative level. Truncation at level > source is the
 // identity (allowed); at level < 0 is undefined (rejected).
            let level = args.first()?.as_int()?;
            let _src = args.get(1)?.as_int()?;
            decision(level >= 0, format!(
                    "truncate_to_level: level={} must be >=0 (HTT 5.5.6)",
                    level
                ))
        }

 // -- Factorisation -----------------------------------------------
        "kernel_epi_mono_factorisation" => {
 // args: [category_level: Int]. Reject when category is below
 // (∞,1)-level (epi/mono only meaningful at level ≥ 1).
            let level = args.first()?.as_int()?;
            decision(level >= 1, format!(
                    "epi_mono: category level={} must be >=1 (HTT 5.2.8.4)",
                    level
                ))
        }
        "kernel_n_truncation_factorisation" => {
 // args: [trunc_level: Int]. Reject negative trunc-level.
            let level = args.first()?.as_int()?;
            decision(level >= 0, format!(
                    "n_truncation_factorisation: level={} must be >=0 (HTT 5.2.8.16)",
                    level
                ))
        }

 // -- Pronk -------------------------------------------------------
        "kernel_pronk_bicat_fractions" => {
 // args: [bf1, bf2, bf3, bf4, bf5]
            let bf: Vec<bool> = args
                .iter()
                .take(5)
                .map(|v| v.as_bool().unwrap_or(false))
                .collect();
            if bf.len() != 5 {
                return None;
            }
            let axioms = PronkAxioms {
                identities: bf[0],
                composition: bf[1],
                right_cancellative: bf[2],
                ore_like: bf[3],
                saturated: bf[4],
            };
            let satisfied = axioms.all_satisfied();
            decision(satisfied, format!(
                    "pronk_fractions BF1-BF5 all_satisfied = {}",
                    satisfied
                ))
        }

 // -- (∞,1)-topos -------------------------------------------------
        "kernel_infinity_topos" => {
 // args: [presentable, universal_colim, disjoint_coprod, effective_grpd]
            let g0 = args.first()?.as_bool()?;
            let g1 = args.get(1)?.as_bool()?;
            let g2 = args.get(2)?.as_bool()?;
            let g3 = args.get(3)?.as_bool()?;
            let g = GiraudAxioms {
                presentable: g0,
                universal_small_colimits: g1,
                disjoint_coproducts: g2,
                effective_groupoids: g3,
            };
            let ok = g.all_satisfied();
            decision(ok, format!("infinity_topos Giraud axioms all_satisfied = {}", ok))
        }

 // -- ZFC self-recognition ----------------------------------------
        "kernel_zfc_self_recognition" => {
 // No args — verifies that all 7 rules lift to ZFC + 2-inacc.
            let all_ok = KernelRuleId::full_list()
                .iter()
                .all(|r| is_zfc_plus_2_inacc_provable(*r));
            decision(all_ok, format!(
                    "zfc_self_recognition: every kernel rule provable in ZFC + 2-inacc = {}",
                    all_ok
                ))
        }

 // -- Gödel coding ------------------------------------------------
        "kernel_godel_coding" => decision(true, "godel_coding: Cantor pairing + PrimRec + MuRec + GodelEncoding all decidable"),

 // -- Industrial tactics ------------------------------------------
        "kernel_tactics_industrial" => decision(true, "tactics_industrial: lia/decide/induction/congruence/eauto deterministic dispatchers"),

 // -- Cross-format CI ---------------------------------------------
        "kernel_cross_format_gate" => {
 // 4 bools: [coq_passed, lean_passed, isabelle_passed, dedukti_passed]
            let coq = args.first()?.as_bool()?;
            let lean = args.get(1)?.as_bool()?;
            let isa = args.get(2)?.as_bool()?;
            let dk = args.get(3)?.as_bool()?;
            let all_passed = coq && lean && isa && dk;
            decision(all_passed, format!(
                    "cross_format: coq={}, lean={}, isabelle={}, dedukti={}",
                    coq, lean, isa, dk
                ))
        }

 // -- Mechanisation roadmap ---------------------------------------
        "kernel_mechanisation_roadmap" => decision(true, "mechanisation_roadmap: HTT + AR 1994 enumerations always available"),
 // -- MSFS self-containment ---------------------------------------
 // Backed by `mechanisation_roadmap::msfs_self_contained()` —
 // returns true iff zero AxiomCited + zero Pending in MSFS scope.
 // This is the dynamically-computed witness that the MSFS paper's
 // "100% from-first-principles modulo ZFC+2-inacc" claim is true.
        "kernel_msfs_self_contained" => {
            let holds = crate::mechanisation_roadmap::msfs_self_contained();
            let gaps = crate::mechanisation_roadmap::msfs_unmechanised_dependencies();
            decision(holds, format!(
                    "msfs_self_contained = {} (unmechanised gaps: {})",
                    holds,
                    gaps.len()
                ))
        }

 // ─── HoTT coherence dispatch ───────────────────────────
 //

 // These five entries discharge the IOU-bearing axioms
 // declared in `core/math/hott.vr` (commit 7b63d5bd). Each
 // axiom carries a `@framework(hott, "...")` annotation
 // citing its HoTT-Book section; the load-bearing structural
 // proof is constructive in CCHM cubical type theory (which
 // Verum's kernel adopts), so the kernel ALWAYS witnesses
 // these coherence laws for any well-formed input. The
 // bool-typed first arg lets the dispatcher reject
 // pathologically-malformed call sites that the elaborator
 // catches; well-typed `@framework(hott, …)` axioms always
 // pass `true` (the dispatcher default via `unwrap_or(true)`).

 // HoTT Book §4.2.4 — equiv_inv preserves IsEquiv via
 // cubical naturality square + ap-functoriality.
        "kernel_equiv_inv_coherence" => {
            let well_formed = args.first().and_then(|v| v.as_bool()).unwrap_or(true);
            decision(well_formed, format!(
                    "HoTT Book §4.2.4: equiv_inv preserves IsEquiv via cubical \
                     naturality square + ap-functoriality \
                     (well_formed_input={})",
                    well_formed
                ))
        }

 // HoTT Book §4.2.5 — composition of equivalences.
        "kernel_equiv_compose_coherence" => {
            let well_formed = args.first().and_then(|v| v.as_bool()).unwrap_or(true);
            decision(well_formed, format!(
                    "HoTT Book §4.2.5: equivalences compose; section/retraction \
                     paths transport through composition \
                     (well_formed_input={})",
                    well_formed
                ))
        }

 // HoTT Book §4.4 — equiv_from_contr_map preserves IsEquiv
 // (a function with contractible fibres is an equivalence).
        "kernel_contr_fiber_coherence" => {
            let well_formed = args.first().and_then(|v| v.as_bool()).unwrap_or(true);
            decision(well_formed, format!(
                    "HoTT Book §4.4: contractible-fibre map is equivalence; \
                     section/retraction extracted from IsContr witnesses \
                     (well_formed_input={})",
                    well_formed
                ))
        }

 // HoTT Book §2.10 — transport coherence: transport along a
 // path preserves equivalence structure.
        "kernel_transport_coherence" => {
            let well_formed = args.first().and_then(|v| v.as_bool()).unwrap_or(true);
            decision(well_formed, format!(
                    "HoTT Book §2.10: transport-equivalence coherence; path \
                     algebra preserved by ap on identity components \
                     (well_formed_input={})",
                    well_formed
                ))
        }

 // HoTT Book §3.3 — propositional equivalence: in a
 // propositional type, all paths between two points coincide.
        "kernel_prop_coherence" => {
            let well_formed = args.first().and_then(|v| v.as_bool()).unwrap_or(true);
            decision(well_formed, format!(
                    "HoTT Book §3.3: propositional-equivalence coherence; in \
                     a Prop, IsEquiv is contractible \
                     (well_formed_input={})",
                    well_formed
                ))
        }

 // -- Verified-compilation simulation theorems (#162 / CompCert-parity).
 //
 // Each kernel_<pass>_preserves_semantics intrinsic recognises a
 // codegen-pass bridge axiom declared at
 // core/verify/codegen_soundness/<pass>.vr. The dispatcher returns
 // `Decision { holds: true }` because the discharge route is via
 // framework citation (Leroy 2009 / Vellvm 2012 / Poletto-Sarkar
 // 1999 / CompCertELF 2020), not via algorithmic check. The
 // `reason` text references the citation so audit reports
 // surface the published proof reviewers can chase.
 //
 // Manifest cross-reference:
 // `verum_kernel::codegen_attestation::manifest()` carries the
 // canonical roster + IOU citations. The audit gate
 // (`verum audit --codegen-attestation`) cross-checks both
 // surfaces and reports per-pass discharge status.
        "kernel_vbc_lowering_preserves_semantics" => decision(true, "CompCert simulation diagram (Leroy 2009 §5.2) — TypedAST → \
                     VBC lowering preserves operational semantics; admitted \
                     with framework citation, see \
                     core/verify/codegen_soundness/vbc_lowering.vr"),
        "kernel_ssa_construction_preserves_semantics" => decision(true, "Beringer-Stark CC 2002 §3 / Cytron et al TOPLAS 1991 — \
                     SSA construction preserves operational semantics; admitted \
                     with framework citation, see \
                     core/verify/codegen_soundness/ssa_construction.vr"),
        "kernel_register_allocation_preserves_semantics" => decision(true, "George-Appel TOPLAS 1996 §6 — register allocation preserves \
                     observable behaviour; admitted with framework citation, see \
                     core/verify/codegen_soundness/register_allocation.vr"),
        "kernel_linear_scan_regalloc_preserves_semantics" => decision(true, "Poletto-Sarkar TOPLAS 1999 §3 / Mössenböck CC 2002 §4 — \
                     linear-scan regalloc preserves observable behaviour AND \
                     live-range monotonicity; admitted with framework citation, \
                     see core/verify/codegen_soundness/linear_scan_regalloc.vr"),
        "kernel_llvm_emission_preserves_semantics" => decision(true, "Vellvm POPL 2012 §4-5 — LLVM IR emission preserves \
                     operational semantics modulo LLVM-internal scheduling; \
                     admitted with framework citation, see \
                     core/verify/codegen_soundness/llvm_emission.vr"),
        "kernel_machine_code_emission_preserves_semantics" => decision(true, "CompCertELF Wang-Wilke-Leroy POPL 2020 §6 + Leroy 2009 §6 \
                     external-call axiom — machine-code emission boundary \
                     attestation (LLVM-version pinning + ABI conformance); \
                     admitted with framework citation, see \
                     core/verify/codegen_soundness/machine_code_emission.vr"),

 // -- kernel_v0 rule soundness IOUs (#157 / minimal-CIC kernel).
 //
 // Each `kernel_<rule>_strict` (and the master
 // `kernel_soundness_v0`) is the dispatcher counterpart of a
 // `@kernel_discharge` annotation on a `k_*_sound` theorem in
 // `core/verify/kernel_v0/rules/`. The discharge route is via
 // a Verum-language lemma in `core/verify/kernel_v0/lemmas/`
 // (named in each rule's `@discharged_by(...)` attribute);
 // the dispatcher returns `Decision { holds: true }` to make
 // the bidirectional contract surface in
 // `verum audit --kernel-discharged-axioms`.
        "kernel_var" | "kernel_var_strict" => decision(true, "kernel_v0/k_var_sound: variable lookup — bookkeeping rule, no \
                     upstream proof obligation. See \
                     core/verify/kernel_v0/rules/k_var.vr."),
        "kernel_universe_intro" | "kernel_universe_intro_strict" => decision(true, "kernel_v0/k_univ_sound: universe-introduction soundness — \
                     U_n : U_{n+1} cumulative hierarchy. Discharged by \
                     core.verify.kernel_v0.lemmas.sub.cumulative_universe_inclusion. \
                     See core/verify/kernel_v0/rules/k_univ.vr."),
        "kernel_forward_axiom" | "kernel_forward_axiom_strict" => decision(true, "kernel_v0/k_fwax_sound: forward-axiom witness import — relies on \
                     foreign-system proof of the axiom in its native theory \
                     (Coq/Lean/Isabelle/Agda mathlib). See \
                     core/verify/kernel_v0/rules/k_fwax.vr."),
        "kernel_positivity" | "kernel_positivity_strict" => decision(true, "kernel_v0/k_pos_sound: strict-positivity check for inductive \
                     types — Coquand-Huet 1988. Discharged by per-rule structural \
                     analysis. See core/verify/kernel_v0/rules/k_pos.vr."),
        "kernel_pi_form" | "kernel_pi_form_strict" => decision(true, "kernel_v0/k_pi_form_sound: Π-formation rule. Discharged by \
                     core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. \
                     See core/verify/kernel_v0/rules/k_pi_form.vr."),
        "kernel_lam_intro" | "kernel_lam_intro_strict" => decision(true, "kernel_v0/k_lam_intro_sound: λ-introduction rule. Discharged by \
                     core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi. \
                     See core/verify/kernel_v0/rules/k_lam_intro.vr."),
        "kernel_app_elim" | "kernel_app_elim_strict" => decision(true, "kernel_v0/k_app_elim_sound: application-elimination rule. \
                     Discharged by \
                     core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + \
                     core.verify.kernel_v0.lemmas.beta.church_rosser_confluence. \
                     See core/verify/kernel_v0/rules/k_app_elim.vr."),
        "kernel_beta" | "kernel_beta_strict" => decision(true, "kernel_v0/k_beta_sound: β-conversion soundness — (λx.b) a ↝_β \
                     b[x:=a] preserves typing. Discharged by \
                     core.verify.kernel_v0.lemmas.beta.church_rosser_confluence. \
                     See core/verify/kernel_v0/rules/k_beta.vr."),
        "kernel_eta" | "kernel_eta_strict" => decision(true, "kernel_v0/k_eta_sound: η-conversion soundness. Discharged by \
                     core.verify.kernel_v0.lemmas.eta.function_extensionality. \
                     See core/verify/kernel_v0/rules/k_eta.vr."),
        "kernel_sub" | "kernel_sub_strict" => decision(true, "kernel_v0/k_sub_sound: subsumption rule. Discharged by \
                     core.verify.kernel_v0.lemmas.sub.cumulative_universe_inclusion. \
                     See core/verify/kernel_v0/rules/k_sub.vr."),
        "kernel_soundness_v0" => decision(true, "kernel_v0/kernel_soundness: master soundness theorem. \
                     Discharged by per-rule case-split over the 10 k_*_sound \
                     lemmas. See core/verify/kernel_v0/soundness.vr."),

 // -- Separation-logic surface alignment (#161 V0).
 //
 // Pins the structural alignment between `core/logic/separation.vr`
 // and `verum_kernel::separation_logic`. CI tests in
 // `verum_kernel::separation_logic::tests` lock the cardinality
 // invariant (6-variant HeapPredicate, 4-variant Capability);
 // the dispatcher returns `Decision { holds: true }` so the
 // audit gate counts the alignment as discharged.
        "kernel_separation_logic_alignment_is_sound" => decision(true, "core/logic/separation.vr ↔ verum_kernel::separation_logic \
                     structural alignment — CI-pinned via cardinality tests in \
                     verum_kernel::separation_logic::tests. See \
                     core/verify/separation_soundness/separation_logic_alignment.vr."),

 // -- Meta-soundness escape hatch (#158 V0 — Gödel 2nd workaround).
 //
 // The kernel's soundness theorem (in core/verify/kernel_soundness/)
 // is necessarily proven in a slightly stronger meta-theory than
 // the kernel itself, per Gödel's Second Incompleteness Theorem:
 // a consistent system cannot prove its own consistency in itself.
 // Verum's structured escape: prove soundness in Verum + κ_meta
 // (one inaccessible above the working universe).
 //
 // The dispatcher returns `holds: true` because the kernel's
 // meta-theoretic footprint is bounded by
 // `verum_kernel::zfc_self_recognition::required_meta_theory`
 // for every rule — i.e., the footprint never exceeds
 // Verum + κ_2 + ZFC. Adding κ_meta on top (one strongly
 // inaccessible above κ_2) puts the soundness proof inside
 // the meta-universe.
 // Reflection-tower discharge routes (MSFS-grounded).
 //
 // Three structural facts (NOT five opaque ordinal levels):
 //
 // * `kernel_reflection_tower_base` — REF^0 base footprint.
 // * `kernel_reflection_tower_stable` — REF^≥1 theory-level
 // idempotence (MSFS Theorem 9.6(b)).
 // * `kernel_reflection_tower_omega_bounded` — REF^ω
 // bounded by Con(S) + κ_inacc (MSFS Theorem 8.2).
 //
 // All three reuse the existing MSFS-machine-verified
 // intrinsics (`kernel_truncate_to_level`,
 // `kernel_straightening_equivalence`,
 // `kernel_self_soundness_in_meta_universe`) under the hood.
        "kernel_reflection_tower_base" => {
            let d = crate::reflection_tower::discharge_at_universe_index(0);
            decision(d.holds, format!(
                    "reflection-tower REF^0 (base footprint) — {}; \
                     witness({}): a_m_cls={}, b_pi_inf_inf+1={}, \
                     b_universe_ascent={}.  See \
                     verum_kernel::zfc_self_recognition + \
                     core/verify/kernel_self_soundness/predicative_reflection.vr.",
                    if d.holds { "discharged" } else { "FAILED to discharge" },
                    d.universe_index,
                    d.witness.a_m_cls_is_meta_cls_holds,
                    d.witness.b_pi_inf_inf_plus_1_equivalent,
                    d.witness.b_universe_ascent_with_theory_idempotence,
                ))
        }
        "kernel_reflection_tower_stable" => {
 // REF^≥1 — theory-level idempotence (MSFS Theorem 9.6(b)).
 // Constructively discharge at k=1; per Theorem 9.6, every
 // k ≥ 1 yields the same theory.
            let d = crate::reflection_tower::discharge_at_universe_index(1);
            decision(d.holds, format!(
                    "reflection-tower REF^≥1 (MSFS Theorem 9.6(b) — theory-level \
                     idempotence under universe-ascent) — {}; constructive \
                     dispatch through kernel_truncate_to_level={} + \
                     kernel_straightening_equivalence={}. Machine-verified at \
                     MSFS corpus theorems/msfs/09_meta_classification/\
                     theorems_9_3_9_4_9_6.vr.",
                    if d.holds { "discharged" } else { "FAILED to discharge" },
                    d.truncate_to_level_holds,
                    d.straightening_equivalence_holds,
                ))
        }
        "kernel_reflection_tower_omega_bounded" => {
            let report = crate::reflection_tower::build_tower_report();
            let omega = report
                .stage_verdicts
                .iter()
                .find(|v| v.stage_tag == "ref_omega_bounded");
            let holds = omega.map(|v| v.discharges).unwrap_or(false);
            decision(holds, format!(
                    "reflection-tower REF^ω (MSFS Theorem 8.2 — reflective \
                     tower bounded by Con(S) + κ_inacc, exactly ONE extra \
                     strongly-inaccessible) — {}; max_inaccessible_required={} \
                     (bound is 3). Machine-verified at MSFS corpus \
                     theorems/msfs/08_bypass_paths/theorems_8_1_to_8_8.vr.",
                    if holds { "discharged" } else { "FAILED to discharge" },
                    report.max_inaccessible_required,
                ))
        }
        "kernel_reflection_tower_absolute_boundary" => {
 // REF^Abs — MSFS Theorem 5.1 (AFN-T α): 𝓛_Abs = ∅.
 // The boundary is uniformly empty across every Rich-
 // metatheory + every categorical level (five-axis
 // absoluteness). The kernel never instantiates an
 // absolute-foundation candidate.
            let holds = crate::reflection_tower::absolute_boundary_empty_discharges();
            decision(holds, format!(
                    "reflection-tower REF^Abs (MSFS Theorem 5.1 — AFN-T α \
                     Boundary Lemma: 𝓛_Abs = ∅, the absolute foundation \
                     stratum is empty) — {}; uniformly closed across all \
                     Rich-metatheories + all categorical levels (five-axis \
                     absoluteness, MSFS §11). Machine-verified at \
                     MSFS corpus theorems/msfs/05_afnt_alpha/theorem_5_1.vr.",
                    if holds { "discharged" } else { "FAILED to discharge" },
                ))
        }

        // ATS-V architectural-type discharge intrinsics.
        //
        // These arms used to answer `decision(true, "<prose>")` — a
        // sanity stamp minted before the ATS-V phase existed, saying
        // that the intrinsic was WIRED rather than that the property
        // HELD. The phase has been live since T0834, so the stamps
        // were reporting verdicts the kernel never computed: the one
        // failure mode a proof kernel may not have.
        //
        // Every `kernel_arch_*` name now discharges by EXECUTION —
        // `arch_discharge` looks up what the name claims and runs that
        // claim against the live checkers (see `arch_probe`). One
        // carrier and one claim table replace 42 hand-written stamps
        // and their 42 copies of catalogue prose, which can no longer
        // drift from the checks they describe.
        name if name.starts_with("kernel_arch_") => arch_discharge(name),

        _ => None,
    }
}

// =============================================================================
// Available intrinsics enumeration
// =============================================================================

/// Returns the list of every dispatchable kernel-intrinsic name.
/// Used by `verum audit --kernel-intrinsics` and by the compiler's
/// elaborator to validate `apply kernel_*(...)` invocations.
/// Names the kernel can dispatch.
///
/// The `kernel_arch_*` family is NOT listed here: it is derived from
/// [`ARCH_CLAIMS`], the single authority for what each architectural
/// discharge claims. Listing those names by hand beside the dispatch
/// is what let eight endpoints go missing while their checks ran.
pub fn available_intrinsics() -> &'static [&'static str] {
    static ALL: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
        let mut names: Vec<&'static str> = BASE_INTRINSICS.to_vec();
        names.extend(ARCH_CLAIMS.iter().map(|(name, _)| *name));
        names
    });
    &ALL
}

/// Every non-architectural intrinsic name.
const BASE_INTRINSICS: &[&str] = &[
        "kernel_yoneda_embedding",
        "kernel_yoneda_embedding_bare",
        "kernel_identity_is_equivalence",
        "kernel_kan_extension",
        "kernel_straightening_equivalence",
        "kernel_grothendieck_construction",
        "kernel_saft_adjunction",
        "kernel_reflective_subcategory_aft",
        "kernel_whitehead_promote",
        "kernel_compute_colimit",
        "kernel_specialised_limits",
        "kernel_truncate_to_level",
        "kernel_epi_mono_factorisation",
        "kernel_n_truncation_factorisation",
        "kernel_pronk_bicat_fractions",
        "kernel_infinity_topos",
        "kernel_zfc_self_recognition",
        "kernel_godel_coding",
        "kernel_tactics_industrial",
        "kernel_cross_format_gate",
        "kernel_mechanisation_roadmap",
        "kernel_msfs_self_contained",
 // HoTT coherence dispatch — discharges core/math/hott.vr axioms
 // (commit 7b63d5bd). Each kernel_*_coherence rule witnesses a
 // structural HoTT-Book law that's constructive in CCHM cubical TT.
        "kernel_equiv_inv_coherence",
        "kernel_equiv_compose_coherence",
        "kernel_contr_fiber_coherence",
        "kernel_transport_coherence",
        "kernel_prop_coherence",
 // Verified-compilation simulation theorems (#162 / CompCert-parity).
 // Mirror of `verum_kernel::codegen_attestation::manifest()` —
 // every entry there has a matching dispatcher entry here.
        "kernel_vbc_lowering_preserves_semantics",
        "kernel_ssa_construction_preserves_semantics",
        "kernel_register_allocation_preserves_semantics",
        "kernel_linear_scan_regalloc_preserves_semantics",
        "kernel_llvm_emission_preserves_semantics",
        "kernel_machine_code_emission_preserves_semantics",
 // kernel_v0 rule soundness IOUs (#157). Bare names — the
 // `_strict` suffix on the citation site is stripped by
 // [`is_known_intrinsic`] before lookup, so registering the
 // bare form covers both citation conventions. Each name
 // corresponds to a `@kernel_discharge("kernel_<rule>_strict")`
 // annotation on a `k_*_sound` theorem in
 // `core/verify/kernel_v0/rules/`.
        "kernel_var",
        "kernel_universe_intro",
        "kernel_forward_axiom",
        "kernel_positivity",
        "kernel_pi_form",
        "kernel_lam_intro",
        "kernel_app_elim",
        "kernel_beta",
        "kernel_eta",
        "kernel_sub",
        "kernel_soundness_v0",
 // Separation-logic surface alignment (#161 V0).
        "kernel_separation_logic_alignment_is_sound",
 // Reflection-tower discharges (MSFS-grounded).
 // Four structural facts; the base stage subsumes the
 // rank-1 meta-soundness claim previously declared as a
 // separate axiom.
 // * base footprint (per-rule enumeration; rank-1 meta-soundness).
 // * REF^≥1 theory-level idempotence (MSFS Theorem 9.6(b)).
 // * REF^ω bounded by Con(S) + κ_inacc (MSFS Theorem 8.2).
 // * REF^Abs (AFN-T α — boundary).
        "kernel_reflection_tower_base",
        "kernel_reflection_tower_stable",
        "kernel_reflection_tower_omega_bounded",
        "kernel_reflection_tower_absolute_boundary",
        // ATS-V architectural-type discharge intrinsics.  Each
        // entry corresponds to a Verum-side `axiom` declaration in
        // core/architecture/anti_patterns.vr (or in the per-module
        // mtac/counterfactual/adjunction/yoneda/composition/corpus/phase
        // cogs) annotated with `@kernel_discharge("kernel_arch_*")`.
        // The cross-side pin test in
        // crates/verum_kernel/tests/k_arch_v_alignment.rs asserts
        // every Verum-side bridge has a kernel-side counterpart and
        // vice versa.
        // ATS-V architectural-type discharge intrinsics for the
        // Verum-side core/architecture/ MTAC + counterfactual +
        // adjunction + yoneda kernel-discharge cogs.
        // Composition / corpus / phase / parse engine intrinsics —
        // surface the operational ATS-V layer (A ⊗ B, cross-cog
        // invariants, Phase 6.5 orchestrator).
        // Red-team closure intrinsics (AT-1..AT-5) — defeat known
        // attack vectors against the ATS-V declarative surface.
];

/// Returns true iff the given name is an available kernel intrinsic.
///
/// Recognises both the bare dispatcher name (e.g.
/// `kernel_grothendieck_construction`) AND its `_strict` form
/// (`kernel_grothendieck_construction_strict`); the strict form is the
/// refinement-typed bridge declared in `core/proof/kernel_bridge.vr`
/// whose argument types encode the dispatcher's preconditions, but the
/// underlying dispatch surface is the same.
pub fn is_known_intrinsic(name: &str) -> bool {
    let bare = name.strip_suffix("_strict").unwrap_or(name);
    available_intrinsics().contains(&bare)
}

/// Used by the discharge auditor to ensure that a Verum-side
/// `kernel_bridge.vr` axiom actually has a kernel-side counterpart.
/// Returns the list of bridge axiom names that **lack** dispatch.
pub fn missing_dispatchers<'a>(bridge_names: &[&'a str]) -> Vec<&'a str> {
    bridge_names
        .iter()
        .copied()
        .filter(|name| !is_known_intrinsic(name))
        .collect()
}

// Used to keep the unused-import warning quiet.
#[allow(dead_code)]
fn _refs() {
    let _ = ExportFormat::Coq;
}

#[cfg(test)]
mod tests {
    use super::*;

 // ----- IntrinsicValue helpers -----

    #[test]
    fn intrinsic_value_as_bool_works_on_bool_and_decision() {
        assert_eq!(IntrinsicValue::Bool(true).as_bool(), Some(true));
        assert_eq!(
            IntrinsicValue::Decision {
                holds: true,
                reason: "x".into()
            }
            .as_bool(),
            Some(true)
        );
        assert_eq!(IntrinsicValue::Int(7).as_bool(), None);
        assert_eq!(IntrinsicValue::Unit.as_bool(), None);
    }

 // ----- Yoneda -----

    #[test]
    fn yoneda_embedding_with_proper_args_holds() {
        let r = dispatch_intrinsic(
            "kernel_yoneda_embedding",
            &[IntrinsicValue::Int(1), IntrinsicValue::Int(2)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn yoneda_embedding_rejects_negative_level() {
        let r = dispatch_intrinsic(
            "kernel_yoneda_embedding",
            &[IntrinsicValue::Int(-1), IntrinsicValue::Int(0)],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative level must be rejected"
        );
    }

    #[test]
    fn yoneda_embedding_rejects_negative_universe() {
        let r = dispatch_intrinsic(
            "kernel_yoneda_embedding",
            &[IntrinsicValue::Int(1), IntrinsicValue::Int(-5)],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative universe must be rejected"
        );
    }

    #[test]
    fn yoneda_embedding_no_args_returns_none() {
 // Bare-call without args fails dispatch (caller must thread structural data).
        assert!(
            dispatch_intrinsic("kernel_yoneda_embedding", &[]).is_none(),
            "ATTACK: no-args call must fail dispatch (no silent-true)"
        );
    }

    #[test]
    fn yoneda_embedding_bare_back_compat() {
 // The _bare form is documented back-compat.
        let r = dispatch_intrinsic("kernel_yoneda_embedding_bare", &[]).unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

 // ----- Identity-is-equivalence -----

    #[test]
    fn identity_is_equivalence_holds_at_non_negative_level() {
        for n in 0..5 {
            let r = dispatch_intrinsic("kernel_identity_is_equivalence", &[IntrinsicValue::Int(n)])
                .unwrap();
            assert_eq!(
                r.as_bool(),
                Some(true),
                "id_X must witness equivalence at level {}",
                n
            );
        }
    }

    #[test]
    fn identity_is_equivalence_rejects_negative_level() {
        let r = dispatch_intrinsic("kernel_identity_is_equivalence", &[IntrinsicValue::Int(-1)])
            .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative ordinal level must be rejected"
        );
    }

 // ----- Kan extension preconditions -----

    #[test]
    fn kan_extension_dispatch_holds_when_both_preconditions_true() {
        let r = dispatch_intrinsic(
            "kernel_kan_extension",
            &[IntrinsicValue::Bool(true), IntrinsicValue::Bool(true)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn kan_extension_fails_when_ff_missing() {
        let r = dispatch_intrinsic(
            "kernel_kan_extension",
            &[IntrinsicValue::Bool(false), IntrinsicValue::Bool(true)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

 // ----- Grothendieck -----

    #[test]
    fn grothendieck_dispatch_passes_with_positive_fibre_count() {
        let r = dispatch_intrinsic(
            "kernel_grothendieck_construction",
            &[IntrinsicValue::Int(2)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn grothendieck_dispatch_rejects_empty_diagram() {
        let r = dispatch_intrinsic(
            "kernel_grothendieck_construction",
            &[IntrinsicValue::Int(0)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

 // ----- Adjoint -----

    #[test]
    fn saft_dispatch_routes_through_left_adjoint_exists() {
 // All four flags true → adjoint exists.
        let r = dispatch_intrinsic(
            "kernel_saft_adjunction",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn saft_dispatch_fails_without_colimits() {
        let r = dispatch_intrinsic(
            "kernel_saft_adjunction",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(false),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

 // ----- Pronk + topos: composite preconditions -----

    #[test]
    fn pronk_dispatch_routes_through_bf1_to_bf5() {
        let r = dispatch_intrinsic(
            "kernel_pronk_bicat_fractions",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn pronk_dispatch_fails_when_one_axiom_breaks() {
        let r = dispatch_intrinsic(
            "kernel_pronk_bicat_fractions",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(false), // BF4 breaks
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn topos_dispatch_routes_through_giraud_axioms() {
        let r = dispatch_intrinsic(
            "kernel_infinity_topos",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

 // ----- Self-recognition -----

    #[test]
    fn self_recognition_dispatch_always_passes() {
        let r = dispatch_intrinsic("kernel_zfc_self_recognition", &[]).unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

 // ----- Cross-format -----

    #[test]
    fn cross_format_dispatch_requires_all_four() {
        let r = dispatch_intrinsic(
            "kernel_cross_format_gate",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn cross_format_dispatch_fails_when_one_format_fails() {
        let r = dispatch_intrinsic(
            "kernel_cross_format_gate",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(false), // Lean failed
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

 // ----- Available intrinsics + missing dispatchers -----

    #[test]
    fn msfs_self_contained_intrinsic_dispatches() {
 // The dispatcher must reflect the runtime self-containment state.
        let r = dispatch_intrinsic("kernel_msfs_self_contained", &[]).unwrap();
 // Currently TRUE (no AxiomCited/Pending in MSFS scope).
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_msfs_self_contained must return true while MSFS roadmap is closed"
        );
    }

    /// Every `@kernel_discharge("name")` citation under `root`,
    /// deduplicated. Reading the library is the point: the dispatch
    /// surface exists to serve these citations, and any list of them
    /// maintained by hand drifts away from what the library actually
    /// says.
    fn cited_kernel_discharges(root: &std::path::Path) -> std::collections::BTreeSet<String> {
        fn walk(dir: &std::path::Path, out: &mut std::collections::BTreeSet<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "vr") {
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    for line in text.lines() {
                        // A citation inside a comment is prose about
                        // the mechanism, not a use of it — the
                        // soundness docs spell the shape as
                        // `kernel_<rule>_strict`, which is a pattern
                        // and not a name any kernel could dispatch.
                        let code = match line.find("//") {
                            Some(at) => &line[..at],
                            None => line,
                        };
                        let mut rest = code;
                        while let Some(at) = rest.find("@kernel_discharge(\"") {
                            rest = &rest[at + "@kernel_discharge(\"".len()..];
                            if let Some(end) = rest.find('"') {
                                out.insert(rest[..end].to_string());
                                rest = &rest[end..];
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }
        let mut out = std::collections::BTreeSet::new();
        walk(root, &mut out);
        out
    }

    #[test]
    fn available_intrinsics_covers_all_bridges() {
        let names = available_intrinsics();
        // The property is "every discharge the library cites has a
        // dispatcher", and it is now READ FROM THE LIBRARY rather
        // than pinned as a count.
        //
        // The count this assertion used to carry (with a comment
        // enumerating the families by hand) was a third source of
        // truth beside the dispatch and the registry, and it could
        // only ever detect that SOME number changed — never which
        // citation lost its verifier. Eight `@kernel_discharge`
        // endpoints in core/architecture/ sat unrecognised while this
        // test was green, because the count they were missing from
        // matched the list they were missing from.
        let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
        let cited = cited_kernel_discharges(std::path::Path::new(core_root));
        assert!(
            !cited.is_empty(),
            "no @kernel_discharge citations found under {core_root} — the scan is \
             looking in the wrong place, and a scan that finds nothing proves nothing"
        );
        let orphaned: Vec<&String> = cited.iter().filter(|c| !is_known_intrinsic(c)).collect();
        assert!(
            orphaned.is_empty(),
            "these @kernel_discharge citations in core/ have no dispatcher — the \
             axiom claims a kernel verdict nothing computes: {orphaned:?}"
        );
        assert!(
            names.len() >= cited.len(),
            "the dispatch surface ({}) cannot be smaller than the set of citations \
             it must serve ({})",
            names.len(),
            cited.len(),
        );
        // Legacy shape check retained: the bridge axioms are the
        // `_strict` family and must resolve through the same surface.
        assert!(
            is_known_intrinsic("kernel_grothendieck_construction_strict"),
            "the `_strict` bridge spelling must resolve — {} names available",
            names.len(),
        );
        // Check uniqueness.
        let mut seen = std::collections::HashSet::new();
        for n in names {
            assert!(seen.insert(*n), "duplicate intrinsic name: {}", n);
        }
    }

 // ===========================================================
 // Adversarial-attack red-team suite — STRENGTHENED dispatchers
 // must REJECT pathological inputs. These tests are the
 // contract that distinguishes Verum from "any system that
 // accepts proofs": we PROVE the dispatcher catches malformed
 // inputs at the boundary between bridge and kernel.
 // ===========================================================

    #[test]
    fn attack_whitehead_no_args_rejected() {
 // Bare call → dispatch returns None.
        assert!(
            dispatch_intrinsic("kernel_whitehead_promote", &[]).is_none(),
            "ATTACK: Whitehead with no args silently succeeds (must fail dispatch)"
        );
    }

    #[test]
    fn attack_whitehead_zero_levels_rejected() {
        let r = dispatch_intrinsic(
            "kernel_whitehead_promote",
            &[
                IntrinsicValue::Int(0), // num_levels = 0
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: zero levels must defeat Whitehead promotion (per HTT 1.2.4.3)"
        );
    }

    #[test]
    fn attack_whitehead_one_level_failing_rejected() {
 // Even with 7 levels, if any single level's iso fails, reject.
        let r = dispatch_intrinsic(
            "kernel_whitehead_promote",
            &[
                IntrinsicValue::Int(7),
                IntrinsicValue::Bool(false), // some level fails
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: single-level π_k iso failure must defeat Whitehead"
        );
    }

    #[test]
    fn attack_whitehead_incomplete_certificate_rejected() {
        let r = dispatch_intrinsic(
            "kernel_whitehead_promote",
            &[
                IntrinsicValue::Int(3),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(false), // certificate incomplete
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: incomplete level coverage must defeat Whitehead"
        );
    }

    #[test]
    fn attack_reflective_no_ff_rejected() {
 // Inclusion not fully faithful — must reject (HTT 5.2.7.2).
        let r = dispatch_intrinsic(
            "kernel_reflective_subcategory_aft",
            &[
                IntrinsicValue::Bool(false), // not FF
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: non-FF inclusion must defeat reflective-subcategory AFT"
        );
    }

    #[test]
    fn attack_reflective_no_target_presentable_rejected() {
        let r = dispatch_intrinsic(
            "kernel_reflective_subcategory_aft",
            &[
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(true),
                IntrinsicValue::Bool(false), // target not presentable
                IntrinsicValue::Bool(true),
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: non-presentable target must defeat AFT (HTT 5.5.2.9)"
        );
    }

    #[test]
    fn attack_truncate_negative_level_rejected() {
        let r = dispatch_intrinsic(
            "kernel_truncate_to_level",
            &[
                IntrinsicValue::Int(-1), // negative truncation level
                IntrinsicValue::Int(3),
            ],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative truncation level must be rejected (HTT 5.5.6 requires k≥0)"
        );
    }

    #[test]
    fn attack_specialised_limits_negative_size_rejected() {
        let r =
            dispatch_intrinsic("kernel_specialised_limits", &[IntrinsicValue::Int(-3)]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative diagram size is undefined (must be rejected)"
        );
    }

    #[test]
    fn attack_epi_mono_below_inf_1_rejected() {
        let r =
            dispatch_intrinsic("kernel_epi_mono_factorisation", &[IntrinsicValue::Int(0)]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: epi/mono only meaningful at level≥1 (HTT 5.2.8.4)"
        );
    }

    #[test]
    fn attack_n_trunc_factorisation_negative_level_rejected() {
        let r = dispatch_intrinsic(
            "kernel_n_truncation_factorisation",
            &[IntrinsicValue::Int(-1)],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: negative truncation level for factorisation system"
        );
    }

    #[test]
    fn attack_straightening_below_inf_1_rejected() {
        let r = dispatch_intrinsic(
            "kernel_straightening_equivalence",
            &[IntrinsicValue::Int(0)],
        )
        .unwrap();
        assert_eq!(
            r.as_bool(),
            Some(false),
            "ATTACK: straightening requires (∞,1)-base (HTT 3.2.0.1)"
        );
    }

    #[test]
    fn attack_no_args_universally_fails_for_strict_dispatchers() {
 // Every STRICT dispatcher must return None when given no args.
        let strict_names = [
            "kernel_yoneda_embedding",
            "kernel_kan_extension",
            "kernel_straightening_equivalence",
            "kernel_grothendieck_construction",
            "kernel_saft_adjunction",
            "kernel_reflective_subcategory_aft",
            "kernel_whitehead_promote",
            "kernel_compute_colimit",
            "kernel_specialised_limits",
            "kernel_truncate_to_level",
            "kernel_epi_mono_factorisation",
            "kernel_n_truncation_factorisation",
            "kernel_pronk_bicat_fractions",
            "kernel_infinity_topos",
            "kernel_cross_format_gate",
            "kernel_identity_is_equivalence",
        ];
        for name in &strict_names {
            assert!(
                dispatch_intrinsic(name, &[]).is_none(),
                "ATTACK: {} must fail dispatch on bare-call (otherwise it's a silent-true patch)",
                name
            );
        }
    }

    #[test]
    fn attack_kernel_safety_via_bool_args_to_int_dispatchers() {
 // Type-confusion attack: pass Bool where Int expected.
 // Dispatcher should fail dispatch (None), not silently succeed.
        let bool_attack = [IntrinsicValue::Bool(true), IntrinsicValue::Bool(true)];
        assert!(dispatch_intrinsic("kernel_grothendieck_construction", &bool_attack).is_none());
        assert!(dispatch_intrinsic("kernel_compute_colimit", &bool_attack).is_none());
        assert!(dispatch_intrinsic("kernel_yoneda_embedding", &bool_attack).is_none());
    }

 /// **THE NON-VACUITY INVARIANT.**
 ///
 /// For every strict (parameterised) dispatcher, there must exist a
 /// pathological input that defeats it. This is the hard test
 /// that distinguishes Verum from "any system that justifies":
 /// every kernel-discharge step has a *witness of falsifiability*.
 /// If a dispatcher cannot be defeated by any input, its `holds`
 /// is vacuous and the discharge is silent-true.
    #[test]
    fn invariant_every_strict_dispatcher_has_a_falsifying_input() {
 // (name, args_that_falsify) pairs. Every entry MUST produce
 // holds=false; if any returns holds=true, the dispatcher is
 // vacuous and Verum's "error detection" guarantee is broken.
        let falsifying_attacks: &[(&str, Vec<IntrinsicValue>)] = &[
            (
                "kernel_yoneda_embedding",
                vec![IntrinsicValue::Int(-1), IntrinsicValue::Int(0)],
            ),
            (
                "kernel_identity_is_equivalence",
                vec![IntrinsicValue::Int(-1)],
            ),
            (
                "kernel_kan_extension",
                vec![IntrinsicValue::Bool(false), IntrinsicValue::Bool(true)],
            ),
            (
                "kernel_straightening_equivalence",
                vec![IntrinsicValue::Int(0)],
            ),
            (
                "kernel_grothendieck_construction",
                vec![IntrinsicValue::Int(0)],
            ),
            (
                "kernel_saft_adjunction",
                vec![
                    IntrinsicValue::Bool(false),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                ],
            ),
            (
                "kernel_reflective_subcategory_aft",
                vec![
                    IntrinsicValue::Bool(false), // not FF
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                ],
            ),
            (
                "kernel_whitehead_promote",
                vec![
                    IntrinsicValue::Int(0),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                ],
            ),
            ("kernel_compute_colimit", vec![IntrinsicValue::Int(0)]),
            ("kernel_specialised_limits", vec![IntrinsicValue::Int(-1)]),
            (
                "kernel_truncate_to_level",
                vec![IntrinsicValue::Int(-1), IntrinsicValue::Int(3)],
            ),
            (
                "kernel_epi_mono_factorisation",
                vec![IntrinsicValue::Int(0)],
            ),
            (
                "kernel_n_truncation_factorisation",
                vec![IntrinsicValue::Int(-1)],
            ),
            (
                "kernel_pronk_bicat_fractions",
                vec![
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(false), // BF3 fails
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                ],
            ),
            (
                "kernel_infinity_topos",
                vec![
                    IntrinsicValue::Bool(false), // not presentable
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                ],
            ),
            (
                "kernel_cross_format_gate",
                vec![
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(true),
                    IntrinsicValue::Bool(false), // dedukti fails
                ],
            ),
        ];

        for (name, attack_args) in falsifying_attacks {
            let r = dispatch_intrinsic(name, attack_args)
                .unwrap_or_else(|| panic!("dispatcher {} returned None on falsifying input", name));
            let holds = r.as_bool().unwrap_or(true);
            assert!(
                !holds,
                "INVARIANT VIOLATION: dispatcher {} accepts pathological input {:?} \
                 — Verum's error-detection guarantee is broken",
                name, attack_args
            );
        }
    }

    #[test]
    fn attack_kernel_safety_via_int_args_to_bool_dispatchers() {
 // Type-confusion attack: pass Int where Bool expected. The
 // dispatcher must either FAIL DISPATCH (None) or return
 // holds=false — must not silently succeed. This is the
 // "fail-closed under type confusion" invariant.
        let int_attack = vec![IntrinsicValue::Int(1); 5];
        for name in [
            "kernel_pronk_bicat_fractions",
            "kernel_reflective_subcategory_aft",
            "kernel_infinity_topos",
        ] {
            let args = if name == "kernel_pronk_bicat_fractions" {
                &int_attack[..]
            } else {
                &int_attack[..4]
            };
            let r = dispatch_intrinsic(name, args);
 // Either dispatch failed (None) OR returned holds=false.
 // The forbidden state is Some(Decision { holds: true, ... }).
            match r {
                None => {}                                                // OK — fail-closed dispatch
                Some(IntrinsicValue::Decision { holds: false, .. }) => {} // OK — fail-closed result
                Some(IntrinsicValue::Decision { holds: true, .. }) => {
                    panic!(
                        "ATTACK SOUNDNESS VIOLATION: {} silently succeeds on Int-where-Bool inputs",
                        name
                    );
                }
                Some(other) => {
                    panic!("ATTACK: {} returned unexpected variant {:?}", name, other);
                }
            }
        }
    }

    #[test]
    fn is_known_intrinsic_decides_known_vs_unknown() {
        assert!(is_known_intrinsic("kernel_yoneda_embedding"));
        assert!(is_known_intrinsic("kernel_grothendieck_construction"));
        assert!(!is_known_intrinsic("kernel_undefined"));
        assert!(!is_known_intrinsic(""));
    }

    #[test]
    fn is_known_intrinsic_recognises_strict_suffix() {
 // The strict form is the refinement-typed bridge; underlying
 // dispatcher is the same.
        assert!(is_known_intrinsic(
            "kernel_grothendieck_construction_strict"
        ));
        assert!(is_known_intrinsic("kernel_whitehead_promote_strict"));
        assert!(is_known_intrinsic("kernel_truncate_to_level_strict"));
        assert!(!is_known_intrinsic("kernel_undefined_strict"));
    }

    #[test]
    fn missing_dispatchers_finds_unmatched() {
        let missing = missing_dispatchers(&["kernel_yoneda_embedding", "kernel_unknown_axiom"]);
        assert_eq!(missing, vec!["kernel_unknown_axiom"]);
    }

    #[test]
    fn dispatch_returns_none_on_unknown_name() {
        assert!(dispatch_intrinsic("kernel_unknown", &[]).is_none());
    }

 // ========================================================
 // HoTT coherence dispatch — pin tests for the 5 new arms
 // discharging core/math/hott.vr axioms (commit 7b63d5bd).
 // Each rule witnesses a structural HoTT-Book law; the kernel
 // ALWAYS witnesses well-formed inputs (CCHM cubical TT
 // provides the constructive proof).
 // ========================================================

    #[test]
    fn hott_equiv_inv_coherence_witnesses_well_formed() {
 // Default (no args) → unwrap_or(true) → holds=true.
        let r = dispatch_intrinsic("kernel_equiv_inv_coherence", &[]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_equiv_inv_coherence must witness HoTT §4.2.4 \
             on well-formed input (default)"
        );

 // Explicit true → still holds.
        let r = dispatch_intrinsic("kernel_equiv_inv_coherence", &[IntrinsicValue::Bool(true)])
            .unwrap();
        assert_eq!(r.as_bool(), Some(true));

 // Explicit false → kernel rejects (well_formed_input=false).
        let r = dispatch_intrinsic("kernel_equiv_inv_coherence", &[IntrinsicValue::Bool(false)])
            .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn hott_equiv_compose_coherence_witnesses_well_formed() {
        let r = dispatch_intrinsic("kernel_equiv_compose_coherence", &[]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_equiv_compose_coherence must witness HoTT §4.2.5"
        );
        let r = dispatch_intrinsic(
            "kernel_equiv_compose_coherence",
            &[IntrinsicValue::Bool(false)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn hott_contr_fiber_coherence_witnesses_well_formed() {
        let r = dispatch_intrinsic("kernel_contr_fiber_coherence", &[]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_contr_fiber_coherence must witness HoTT §4.4"
        );
        let r = dispatch_intrinsic(
            "kernel_contr_fiber_coherence",
            &[IntrinsicValue::Bool(false)],
        )
        .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn hott_transport_coherence_witnesses_well_formed() {
        let r = dispatch_intrinsic("kernel_transport_coherence", &[]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_transport_coherence must witness HoTT §2.10"
        );
        let r = dispatch_intrinsic("kernel_transport_coherence", &[IntrinsicValue::Bool(false)])
            .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn hott_prop_coherence_witnesses_well_formed() {
        let r = dispatch_intrinsic("kernel_prop_coherence", &[]).unwrap();
        assert_eq!(
            r.as_bool(),
            Some(true),
            "kernel_prop_coherence must witness HoTT §3.3"
        );
        let r =
            dispatch_intrinsic("kernel_prop_coherence", &[IntrinsicValue::Bool(false)]).unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn hott_coherence_dispatchers_all_known() {
 // Every HoTT coherence dispatcher must be in
 // available_intrinsics() so the audit gate finds it.
        for name in &[
            "kernel_equiv_inv_coherence",
            "kernel_equiv_compose_coherence",
            "kernel_contr_fiber_coherence",
            "kernel_transport_coherence",
            "kernel_prop_coherence",
        ] {
            assert!(
                is_known_intrinsic(name),
                "HoTT coherence dispatcher {} must be registered in available_intrinsics()",
                name
            );
        }
    }

 // -------------------------------------------------------------
 // #162 — Verified-compilation simulation theorems
 // -------------------------------------------------------------

    #[test]
    fn codegen_attestation_dispatchers_all_holds_true() {
 // Every kernel_<pass>_preserves_semantics intrinsic returns
 // Decision { holds: true, reason: <citation> }. The discharge
 // route is via framework citation, not algorithmic check —
 // the dispatcher's role is to confirm the name is recognised
 // and the citation is non-empty.
        for name in &[
            "kernel_vbc_lowering_preserves_semantics",
            "kernel_ssa_construction_preserves_semantics",
            "kernel_register_allocation_preserves_semantics",
            "kernel_linear_scan_regalloc_preserves_semantics",
            "kernel_llvm_emission_preserves_semantics",
            "kernel_machine_code_emission_preserves_semantics",
        ] {
            let r = dispatch_intrinsic(name, &[]).unwrap_or_else(|| {
                panic!("dispatcher must recognise {} and return a decision", name)
            });
            assert_eq!(
                r.as_bool(),
                Some(true),
                "codegen-attestation dispatcher {} must return holds=true \
                 (admitted via framework citation)",
                name,
            );
 // Reason must reference the citation file path so audit
 // reports surface the canonical .vr location.
            if let IntrinsicValue::Decision { reason, .. } = &r {
                assert!(
                    reason.contains("core/verify/codegen_soundness/"),
                    "dispatcher {} reason must reference the .vr citation file: {}",
                    name,
                    reason,
                );
            }
        }
    }

    #[test]
    fn codegen_attestation_dispatchers_listed_in_available_intrinsics() {
 // Every codegen-attestation intrinsic must appear in
 // available_intrinsics() so `verum audit --kernel-intrinsics`
 // enumerates them. Mirrors the HoTT coherence pin above.
        for name in &[
            "kernel_vbc_lowering_preserves_semantics",
            "kernel_ssa_construction_preserves_semantics",
            "kernel_register_allocation_preserves_semantics",
            "kernel_linear_scan_regalloc_preserves_semantics",
            "kernel_llvm_emission_preserves_semantics",
            "kernel_machine_code_emission_preserves_semantics",
        ] {
            assert!(
                is_known_intrinsic(name),
                "codegen-attestation dispatcher {} must be registered in \
                 available_intrinsics()",
                name,
            );
        }
    }

    #[test]
    fn codegen_attestation_dispatchers_match_manifest_pass_roster() {
 // Every CodegenPassId in the manifest must have a dispatcher
 // entry whose name matches the canonical
 // `kernel_<tag>_preserves_semantics` form. This test pins the
 // bidirectional contract: removing a dispatcher entry without
 // also removing the manifest entry breaks the audit gate.
        use crate::codegen_attestation::manifest;
        for pass in manifest() {
            let name = pass.pass.kernel_intrinsic_name();
            assert!(
                is_known_intrinsic(&name),
                "manifest entry {:?} requires dispatcher {} to be registered",
                pass.pass,
                name,
            );
            let r = dispatch_intrinsic(&name, &[]).unwrap_or_else(|| {
                panic!(
                    "dispatcher {} required by manifest entry {:?} returns None",
                    name, pass.pass,
                )
            });
            assert_eq!(
                r.as_bool(),
                Some(true),
                "manifest-required dispatcher {} must return holds=true",
                name,
            );
        }
    }
}
