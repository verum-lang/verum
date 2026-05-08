//! Canonical certificate battery — single source of truth shared by
//! every kernel-differential audit gate.
//!
//! ## Why this module exists
//!
//! Two audit gates need to run the **same** battery of certificates
//! and assert verdict-by-verdict agreement:
//!
//!   * `verum audit --differential-kernel` runs the battery through
//!     the in-process N-kernel registry ([`crate::kernel_registry`])
//!     — Slot A (`proof_checker` bidirectional), Slot B
//!     (`proof_checker_nbe` Normalisation-by-Evaluation), Slot C
//!     (`kernel_v0` manifest-driven verifier) — and asserts
//!     unanimity.
//!   * `verum audit --differential-lean-checker` runs the battery
//!     through the Rust trusted base + the Lean ReferenceChecker
//!     executable and asserts cross-language verdict parity.
//!
//! Before this module the battery lived in `verum_cli`, invisible
//! to the kernel itself, so the in-process N-kernel gate was
//! running stub certs while the cross-language gate had its own
//! private battery.  Hoisting the battery into the kernel crate
//! gives both gates one source of truth.
//!
//! ## What's in the battery
//!
//! 24 certificates covering the structural fragment of the kernel
//! ([`crate::proof_checker::Certificate::verify`]), all four
//! kernel-audit-2026 defect mirrors, polymorphic-identity shapes,
//! deep-nested binders, η-redexes, and a handful of negative cases
//! that *must* be rejected.  Each cert has a stable `id` that
//! survives JSON round-trips and is the cross-prover comparison
//! key.
//!
//! ## Adding a cert
//!
//! Append to [`canonical_battery()`] with a fresh `id` (kebab-case,
//! short, references the kernel rule or defect under test) and
//! extend [`expected_verdict()`] with its trusted-base outcome.
//! The cert flows through every consumer automatically; the first
//! run after adding it MUST reach unanimous agreement across every
//! registered checker.  If it doesn't, fix the kernel(s) before
//! merging — that's the load-bearing value of the gates.

use crate::proof_checker::{Certificate, Term};

/// One certificate in the canonical battery.  Pairs the certificate
/// with a stable identifier used for cross-prover verdict comparison
/// + regression-bibliography reference.
#[derive(Debug, Clone)]
pub struct CanonicalCert {
    /// Stable kebab-case identifier — survives JSON round-trips and
    /// is the cross-prover verdict comparison key.  Never reused.
    pub id: &'static str,
    /// The certificate itself.  Built from [`crate::proof_checker::Term`]
    /// directly (no metadata — the canonical battery exercises the
    /// trusted-base structural fragment, not framework axioms).
    pub certificate: Certificate,
}

impl CanonicalCert {
    /// Construct from primitive parts.  Convenience for the literal
    /// battery below.
    fn build(id: &'static str, term: Term, claimed_type: Term) -> Self {
        Self {
            id,
            certificate: Certificate {
                term,
                claimed_type,
                metadata: std::collections::BTreeMap::new(),
            },
        }
    }
}

/// The canonical 24-cert battery.
///
/// Coverage matrix:
///
/// | Section | Cert IDs | Kernel rules exercised |
/// |---------|----------|------------------------|
/// | Universe formation | `univ-0-in-1`, `univ-5-in-6`, `univ-mismatch` | T-Univ + DEFECT-2 boundary |
/// | Variable lookup | `var0-empty-ctx-fails` | T-Var (negative) |
/// | Identity at universes | `id-at-univ0`, `id-at-univ0-wrong-claim`, `id-at-univ3` | T-Lam-Intro + T-Var |
/// | Polymorphic identity | `poly-id-shape` | T-Lam-Intro + T-Var (deep) |
/// | Pi formation | `pi-univ-univ`, `pi-takes-max` | T-Pi-Form |
/// | Application | `app-domain-mismatch`, `app-non-function` | T-App-Elim (negative) |
/// | DEFECT-2 mirrors | `defect-2-univ-max-overflows`, `defect-2-univ-max-minus-one-ok` | universe-tower-top boundary |
/// | DEFECT-4 mirror | `defect-4-claimed-is-value` | claimed_type validation |
/// | Nested application | `nested-app-domain-mismatch` | T-App-Elim chained |
/// | Const function | `const-fn` | T-Lam-Intro + T-Var (depth 2) |
/// | Higher universe | `high-pi` | T-Pi-Form (max-arg) |
/// | Identity arrow | `id-arrow` | T-Lam-Intro + T-Var |
/// | Deep binder | `deep-var` | T-Var depth 3 |
/// | η-redex | `eta-via-id-application` | T-Conv via η |
/// | Type mismatch | `id-claimed-as-universe` | T-Conv (negative) |
/// | Nested Pi | `nested-pi` | T-Pi-Form (depth 2) |
/// | Nested Lam | `nested-lam-correct` | T-Lam-Intro (depth 2) |
///
/// **Total**: 24 certs.  Adding new entries is the canonical way to
/// regression-pin a newly discovered kernel bug — see
/// `docs/architecture/verum-kernel-audit-2026.md` for the lineage.
pub fn canonical_battery() -> Vec<CanonicalCert> {
    // Helpers — keep cert construction terse.
    let univ = Term::universe;
    let var = Term::var;
    let pi = |a: Term, b: Term| Term::pi(a, b);
    let lam = |a: Term, b: Term| Term::lam(a, b);
    let app = |a: Term, b: Term| Term::app(a, b);

    vec![
        // ---- 1. Universe formation (T-Univ) -------------------------------
        CanonicalCert::build("univ-0-in-1", univ(0), univ(1)),
        CanonicalCert::build("univ-5-in-6", univ(5), univ(6)),
        CanonicalCert::build("univ-mismatch", univ(0), univ(2)),
        // ---- 2. Var (T-Var) — empty ctx → unbound -------------------------
        CanonicalCert::build("var0-empty-ctx-fails", var(0), univ(0)),
        // ---- 3. Identity at Universe(0) (T-Lam-Intro + T-Var) -------------
        CanonicalCert::build(
            "id-at-univ0",
            lam(univ(0), var(0)),
            pi(univ(0), univ(0)),
        ),
        CanonicalCert::build(
            "id-at-univ0-wrong-claim",
            lam(univ(0), var(0)),
            univ(0),
        ),
        // ---- 4. Identity at Universe(3) -----------------------------------
        CanonicalCert::build(
            "id-at-univ3",
            lam(univ(3), var(0)),
            pi(univ(3), univ(3)),
        ),
        // ---- 5. Polymorphic identity (Π A. Π _:A. A) ----------------------
        CanonicalCert::build(
            "poly-id-shape",
            lam(univ(0), lam(var(0), var(0))),
            pi(univ(0), pi(var(0), var(1))),
        ),
        // ---- 6. Pi formation (T-Pi-Form) ----------------------------------
        CanonicalCert::build("pi-univ-univ", pi(univ(0), univ(0)), univ(1)),
        CanonicalCert::build("pi-takes-max", pi(univ(2), univ(5)), univ(6)),
        // ---- 7. App-Elim (β-reduction) ------------------------------------
        // ((λ_:U(0). Var(0)) U(5))    — Pi expects U(0), got U(5).
        CanonicalCert::build(
            "app-domain-mismatch",
            app(lam(univ(0), var(0)), univ(5)),
            univ(0),
        ),
        // ---- 8. App on non-function ---------------------------------------
        CanonicalCert::build(
            "app-non-function",
            app(univ(0), univ(0)),
            univ(0),
        ),
        // ---- 9. DEFECT-2: universe overflow rejection ---------------------
        CanonicalCert::build(
            "defect-2-univ-max-overflows",
            univ(u32::MAX),
            univ(0),
        ),
        // DEFECT-5 boundary case (universe-tower-top escape hatch) — both
        // kernels must accept since the claimed_type lives at the top
        // and `verify`'s DEFECT-4 step swallows the inferred-kind
        // overflow.
        CanonicalCert::build(
            "defect-2-univ-max-minus-one-ok",
            univ(u32::MAX - 1),
            univ(u32::MAX),
        ),
        // ---- 10. DEFECT-4: claimed_type must be a type --------------------
        CanonicalCert::build(
            "defect-4-claimed-is-value",
            lam(univ(0), var(0)),
            lam(univ(0), var(0)),
        ),
        // ---- 11. Nested application — outer λ takes U(0), inner reduces to U(0) ---
        CanonicalCert::build(
            "nested-app-domain-mismatch",
            app(
                lam(univ(0), var(0)),
                app(lam(univ(0), var(0)), univ(0)),
            ),
            univ(0),
        ),
        // ---- 12. Const function (λ_:A. λ_:B. Var(1)) ----------------------
        CanonicalCert::build(
            "const-fn",
            lam(univ(0), lam(univ(0), var(1))),
            pi(univ(0), pi(univ(0), univ(0))),
        ),
        // ---- 13. Higher universe Pi (Type 2 → Type 7 lives in Type 8) ----
        CanonicalCert::build("high-pi", pi(univ(2), univ(7)), univ(8)),
        // ---- 14. Identity-arrow at Universe(0) ---------------------------
        CanonicalCert::build(
            "id-arrow",
            lam(univ(0), var(0)),
            pi(univ(0), univ(0)),
        ),
        // ---- 15. Free var inside nested Pi (deep T-Var) ------------------
        CanonicalCert::build(
            "deep-var",
            lam(univ(0), lam(var(0), lam(var(1), var(0)))),
            pi(univ(0), pi(var(0), pi(var(1), var(2)))),
        ),
        // ---- 16. η-redex via identity application -------------------------
        CanonicalCert::build(
            "eta-via-id-application",
            lam(univ(0), app(lam(univ(0), var(0)), var(0))),
            pi(univ(0), univ(0)),
        ),
        // ---- 17. Type-mismatch: identity claimed as Universe(1) ----------
        CanonicalCert::build(
            "id-claimed-as-universe",
            lam(univ(0), var(0)),
            univ(1),
        ),
        // ---- 18. Nested Pi — Π(_:U(0)). Π(_:U(0)). U(0) -------------------
        CanonicalCert::build(
            "nested-pi",
            pi(univ(0), pi(univ(0), univ(0))),
            univ(1),
        ),
        // ---- 19. Nested Lam — λ(A:U(0)). λ(x:A). x -----------------------
        CanonicalCert::build(
            "nested-lam-correct",
            lam(univ(0), lam(var(0), var(0))),
            pi(univ(0), pi(var(0), var(1))),
        ),
    ]
}

/// Number of certs in the canonical battery.  Pin: any change here is
/// a load-bearing battery-shape change and must be reflected in both
/// audit-gate docs and external-prover-replay schemas.
pub fn canonical_battery_size() -> usize {
    canonical_battery().len()
}

/// Whether a given canonical cert is *expected* to verify under
/// the trusted base.  Mirrors the prose lineage above; flipping this
/// is a load-bearing change to the battery's intent.
///
/// This is consulted by per-kernel sanity tests but **not** by the
/// audit gate itself — the audit gate's verdict is purely "do all
/// registered kernels agree?", agnostic to the expected outcome.
/// The expected-outcome map is here so test code can pin the trusted
/// base's verdict against a checked-in source of truth without
/// re-deriving it from the algorithm.
pub fn expected_verdict(id: &str) -> Option<bool> {
    Some(match id {
        // ---- accepts ----
        "univ-0-in-1"
        | "univ-5-in-6"
        | "id-at-univ0"
        | "id-at-univ3"
        | "poly-id-shape"
        | "pi-univ-univ"
        | "pi-takes-max"
        | "defect-2-univ-max-minus-one-ok"
        | "const-fn"
        | "high-pi"
        | "id-arrow"
        | "deep-var"
        | "eta-via-id-application"
        | "nested-pi"
        | "nested-lam-correct" => true,
        // ---- rejects ----
        "univ-mismatch"
        | "var0-empty-ctx-fails"
        | "id-at-univ0-wrong-claim"
        | "app-domain-mismatch"
        | "app-non-function"
        | "defect-2-univ-max-overflows"
        | "defect-4-claimed-is-value"
        | "nested-app-domain-mismatch"
        | "id-claimed-as-universe" => false,
        _ => return None,
    })
}

// =============================================================================
// Tests — pin battery shape, expected verdicts, and registry agreement
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_has_24_certs() {
        // Pin: the canonical battery is 24 certs.  Bump the const +
        // every audit-gate doc page when adding a new cert; the
        // reverse direction (reducing the battery) requires explicit
        // sign-off — every cert pins a kernel rule or defect.
        assert_eq!(canonical_battery_size(), 24);
    }

    #[test]
    fn every_id_is_unique() {
        // Pin: ids are the cross-prover verdict comparison key — a
        // duplicate ID would silently shadow one cert's verdict in
        // the report.
        let battery = canonical_battery();
        let mut ids: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for cert in &battery {
            assert!(
                ids.insert(cert.id),
                "duplicate canonical-battery id: {}",
                cert.id
            );
        }
        assert_eq!(ids.len(), battery.len());
    }

    #[test]
    fn every_id_has_an_expected_verdict() {
        // Pin: every cert in the battery must have an explicit
        // expected outcome — otherwise per-kernel sanity tests would
        // silently skip it.  Adding a cert without updating
        // `expected_verdict` is a load-bearing omission.
        let battery = canonical_battery();
        for cert in &battery {
            assert!(
                expected_verdict(cert.id).is_some(),
                "canonical cert {} has no expected verdict — update expected_verdict()",
                cert.id
            );
        }
    }

    #[test]
    fn trusted_base_matches_expected_verdicts() {
        // Pin: Algorithm A (proof_checker.rs) verdicts agree with
        // the checked-in expected-outcome map.  If this test fails,
        // either the kernel changed (intended → update the map) or
        // the kernel regressed (unintended → fix the kernel).
        for cert in canonical_battery() {
            let actual = cert.certificate.verify().is_ok();
            let expected = expected_verdict(cert.id).unwrap();
            assert_eq!(
                actual, expected,
                "canonical cert {}: trusted base produced {}, expected {}",
                cert.id, actual, expected
            );
        }
    }

    #[test]
    fn nbe_kernel_matches_expected_verdicts() {
        // Pin: Algorithm B (proof_checker_nbe.rs) verdicts agree
        // with the checked-in expected-outcome map.  Failure here
        // is a structural NbE bug.
        use crate::proof_checker_nbe::verify_certificate;
        for cert in canonical_battery() {
            let actual = verify_certificate(&cert.certificate).is_ok();
            let expected = expected_verdict(cert.id).unwrap();
            assert_eq!(
                actual, expected,
                "canonical cert {}: NbE produced {}, expected {}",
                cert.id, actual, expected
            );
        }
    }

    #[test]
    fn three_kernel_registry_unanimous_on_canonical_battery() {
        // Load-bearing invariant of the in-process N-kernel
        // differential gate: the default 3-kernel registry (Slot A:
        // proof_checker; Slot B: proof_checker_nbe; Slot C:
        // kernel_v0 manifest verifier) must agree on every cert in
        // the canonical battery.  A disagreement here is a real bug
        // in one of the three kernels — the audit gate surfaces it;
        // this test pins it at unit-test resolution.
        use crate::kernel_registry::{AgreementVerdict, KernelRegistry};
        let registry = KernelRegistry::default();
        let mut disagreements: Vec<(String, Vec<&str>, Vec<&str>)> = Vec::new();
        for cert in canonical_battery() {
            let v = registry.verify_all(&cert.certificate);
            if let AgreementVerdict::Disagreement {
                accepting,
                rejecting,
            } = v.agreement
            {
                disagreements.push((cert.id.to_string(), accepting, rejecting));
            }
        }
        assert!(
            disagreements.is_empty(),
            "FV-11 invariant violated — 3-kernel registry disagrees on certs: {:?}",
            disagreements
        );
    }
}
