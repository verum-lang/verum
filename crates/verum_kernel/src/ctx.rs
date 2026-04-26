//! Typing context + framework-axiom attribution. Split per #198 V7.
//!
//! Two related types:
//!
//!   • [`FrameworkId`] — stable identifier for an external mathematical
//!     framework whose theorems Verum postulates as axioms. Every
//!     registered axiom carries one of these so `verum audit
//!     --framework-axioms` can enumerate the exact set of external
//!     results on which any given proof relies.
//!
//!   • [`Context`] — the typing context maintained during checking.
//!     Each binding maps a name to its declared type. The kernel never
//!     performs inference — it only checks.

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

use crate::CoreTerm;

/// A stable identifier for an external mathematical framework whose
/// theorems Verum postulates as axioms.
///
/// Every registered axiom carries one of these so `verum audit
/// --framework-axioms` can enumerate the exact set of external results
/// (Lurie HTT, Schreiber DCCT, Connes reconstruction, Petz
/// classification, Arnold-Mather catastrophe, Baez-Dolan coherence, …)
/// on which any given proof relies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameworkId {
    /// Short machine-readable framework identifier,
    /// e.g. `"lurie_htt"`, `"schreiber_dcct"`, `"connes_reconstruction"`.
    pub framework: Text,
    /// Citation string pointing at the specific result,
    /// e.g. `"HTT 6.2.2.7"`, `"DCCT §3.9"`, `"Connes 2008 axiom (vii)"`.
    pub citation: Text,
}

/// V8 (#227) — kernel-side mirror of the Verum-stdlib
/// `(Fw, ν, τ)` coordinate per spec §A.Z.2. Each registered axiom
/// optionally carries one; the `K-Coord-Cite` rule consults the
/// pair (theorem_coord, axiom_coord) and rejects when an axiom at
/// a strictly higher ν is cited by a theorem at lower ν.
///
/// `nu` reuses the kernel's [`crate::OrdinalDepth`] type so the
/// finite/ω/ω·n arithmetic is shared with `m_depth_omega`. This
/// keeps the kernel single-sourced for ordinal arithmetic.
///
/// `tau` is the trust tier:
///   * `true` — canonical (well-validated, peer-reviewed, in the
///     Standard catalogue per §6.2).
///   * `false` — under construction or not-yet-validated.
///
/// The kernel does NOT enforce τ-discipline at the typing-rule
/// layer; τ is for tooling (`verum audit --coord` surfaces it).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KernelCoord {
    /// Framework slug (matches `FrameworkId::framework`).
    pub fw: Text,
    /// Canonical depth coordinate (per Diakrisis 09-applications/02-canonical-nu-table.md).
    pub nu: crate::OrdinalDepth,
    /// Trust tier: canonical (true) or staged (false).
    pub tau: bool,
}

impl KernelCoord {
    /// Construct a canonical coordinate at the given framework + ν.
    pub fn canonical(fw: Text, nu: crate::OrdinalDepth) -> Self {
        Self { fw, nu, tau: true }
    }

    /// Construct a staged (τ=false) coordinate.
    pub fn staged(fw: Text, nu: crate::OrdinalDepth) -> Self {
        Self { fw, nu, tau: false }
    }
}

/// V8 (#227) — `K-Coord-Cite` kernel rule.
///
/// Per spec §A.Z.5 item 2: when a theorem at coordinate
/// `theorem_coord` cites an axiom at coordinate `axiom_coord`,
/// the rule requires `axiom_coord.nu ≤ theorem_coord.nu`
/// (lex on [`crate::OrdinalDepth`]). The intuition: a theorem
/// in a "lower" depth tier (e.g., set-level Type_0) cannot freely
/// reference a theorem in a "higher" depth tier (e.g., HoTT
/// ω-level) without explicit universe-ascent governance.
///
/// `allow_tier_jump = true` — VVA-3 K-Universe-Ascent escape:
/// when the calling module imports `core.math.frameworks.diakrisis_stack_model`
/// (or sets `@require_extension(vfe_3)`), the rule admits a
/// tier-jump to the next κ-level per Theorem 131.T. The kernel
/// itself cannot detect the import; the caller signals via this
/// flag.
///
/// On rejection, [`crate::KernelError::CoordViolation`] is
/// returned with both coordinates surfaced for diagnostic.
pub fn check_coord_cite(
    theorem_coord: &KernelCoord,
    axiom_coord: &KernelCoord,
    axiom_name: &Text,
    allow_tier_jump: bool,
) -> Result<(), crate::KernelError> {
    // Ascending citations (axiom.nu ≤ theorem.nu) — always
    // accepted. Includes equal coords.
    if axiom_coord.nu == theorem_coord.nu || axiom_coord.nu.lt(&theorem_coord.nu) {
        return Ok(());
    }
    // Descending citation (axiom.nu > theorem.nu): reject unless
    // the tier-jump escape is enabled.
    if allow_tier_jump {
        return Ok(());
    }
    Err(crate::KernelError::CoordViolation {
        axiom_name: axiom_name.clone(),
        theorem_fw: theorem_coord.fw.clone(),
        theorem_nu: Text::from(theorem_coord.nu.render()),
        axiom_fw: axiom_coord.fw.clone(),
        axiom_nu: Text::from(axiom_coord.nu.render()),
    })
}

/// The typing context maintained during checking.
///
/// Each binding maps a name to its declared type. The kernel never
/// performs inference — it only checks.
#[derive(Debug, Clone, Default)]
pub struct Context {
    bindings: List<(Text, CoreTerm)>,
}

impl Context {
    /// An empty context.
    pub fn new() -> Self {
        Self { bindings: List::new() }
    }

    /// Extend the context with a new typed binding. Shadowing is
    /// allowed and mirrors surface semantics.
    pub fn extend(&self, name: Text, ty: CoreTerm) -> Self {
        let mut fresh = self.clone();
        fresh.bindings.push((name, ty));
        fresh
    }

    /// Look up the type of a variable. Returns the innermost binding
    /// (shadowing-respecting).
    pub fn lookup(&self, name: &str) -> Maybe<&CoreTerm> {
        for (n, ty) in self.bindings.iter().rev() {
            if n.as_str() == name {
                return Maybe::Some(ty);
            }
        }
        Maybe::None
    }

    /// Number of bindings currently in scope.
    pub fn depth(&self) -> usize {
        self.bindings.len()
    }
}
