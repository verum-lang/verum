//! Depth functions for kernel rules — split .
//!
//! Two distinct depth notions live here:
//!
//!   • [`m_depth`] — finite-valued M-iteration depth (Diakrisis T-2f*).
//!     Used by the baseline `K-Refine` rule.
//!   • [`m_depth_omega`] — ordinal-valued modal-depth (Theorem 136.T
//!     transfinite stratification). Used by `K-Refine-omega`.
//!     Encoded as Cantor-normal-form prefix below ε_0 via
//!     [`OrdinalDepth`].
//!
//! Plus the [`check_refine_omega`] kernel-rule entry point that gates
//! refinement-type formation on the strict ordinal inequality
//! `md^ω(P) < md^ω(A) + 1`.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::{CoreTerm, KernelError, UniverseLevel};

/// Compute the M-iteration depth of a [`CoreTerm`].
///
/// The depth function is the operational realisation of the Diakrisis
/// metaisation modality `M`: every construct that semantically *speaks
/// about* a lower-depth object bumps the depth by one. Framework axioms
/// (which assert facts about their stated body), `Quote` (which reflects
/// a term as data), `Inductive` / `Quotient` introductions (which close
/// a universe-level construction), and named-type references that
/// originate in the standard library all raise the count.
///
/// The depth bound is consumed by the `K-Refine` rule: a refinement
/// `{ x : base | P(x) }` is well-formed only when
/// `m_depth(P) < m_depth(base) + 1`, i.e. `m_depth(P) ≤ m_depth(base)`.
/// This is the Verum realisation of Diakrisis axiom T-2f* (depth-strict
/// comprehension), which — via Yanofsky 2003 — closes every
/// self-referential paradox schema in a cartesian-closed setting.
///
/// The function is defined recursively:
///
///   * `Var`, `Universe(n)` — zero (variables have no M-iteration;
///     the universe level is reported as depth to align with the
///     stratification of the depth hierarchy).
///   * `Pi`, `Lam`, `Sigma`, `App`, `Pair`, `Fst`, `Snd`, `PathTy`,
///     `Refl`, `HComp`, `Transp`, `Glue`, `Elim` — maximum of their
///     sub-terms (structural, no depth bump).
///   * `Refine { base, predicate }` — maximum of `dp(base)` and
///     `dp(predicate)`.
///   * `Inductive { path, args }` — `1 + max{ dp(arg) | arg ∈ args }`
///     (declared type constructors live one level above their
///     instantiation arguments — they are *defined* by a schema).
///   * `Axiom { body, .. }` — `dp(body) + 1` (framework axioms *speak
///     about* their stated body).
///   * `SmtProof` — `0` (certificates themselves carry no M-iteration).
///
/// Time complexity: O(n) in the size of the term tree. The kernel
/// invokes this at each `Refine` / `Inductive` / `Axiom` check point;
/// a single polynomial walk per well-formedness query.
pub fn m_depth(term: &CoreTerm) -> usize {
    match term {
        CoreTerm::Var(_) => 0,
        CoreTerm::Universe(lvl) => match lvl {
            UniverseLevel::Concrete(n) => *n as usize,
            UniverseLevel::Prop => 0,
            UniverseLevel::Variable(_) => 0,
            UniverseLevel::Succ(l) => 1 + m_depth_level(l),
            UniverseLevel::Max(a, b) => m_depth_level(a).max(m_depth_level(b)),
        },
        CoreTerm::Pi { domain, codomain, .. } => m_depth(domain).max(m_depth(codomain)),
        CoreTerm::Lam { domain, body, .. } => m_depth(domain).max(m_depth(body)),
        CoreTerm::App(f, a) => m_depth(f).max(m_depth(a)),
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => m_depth(fst_ty).max(m_depth(snd_ty)),
        CoreTerm::Pair(a, b) => m_depth(a).max(m_depth(b)),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => m_depth(p),
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            m_depth(carrier).max(m_depth(lhs)).max(m_depth(rhs))
        }
        // §7.4 V3 — heterogeneous path-over: structurally a
        // 4-child generalisation of `PathTy` whose extra component is
        // the constructor-path `path`. M-depth is the max over all four
        // children, identical to the homogeneous case extended.
        CoreTerm::PathOver { motive, path, lhs, rhs } => m_depth(motive)
            .max(m_depth(path))
            .max(m_depth(lhs))
            .max(m_depth(rhs)),
        CoreTerm::Refl(a) => m_depth(a),
        CoreTerm::HComp { phi, walls, base } => {
            m_depth(phi).max(m_depth(walls)).max(m_depth(base))
        }
        CoreTerm::Transp { path, regular, value } => {
            m_depth(path).max(m_depth(regular)).max(m_depth(value))
        }
        CoreTerm::Glue { carrier, phi, fiber, equiv } => m_depth(carrier)
            .max(m_depth(phi))
            .max(m_depth(fiber))
            .max(m_depth(equiv)),
        CoreTerm::Refine { base, predicate, .. } => m_depth(base).max(m_depth(predicate)),
        // Inductive: declared type constructors live one level above
        // their instantiation arguments (the schema is a meta-statement
        // about the arguments).
        CoreTerm::Inductive { args, .. } => {
            1 + args.iter().map(m_depth).max().unwrap_or(0)
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let case_max = cases.iter().map(m_depth).max().unwrap_or(0);
            m_depth(scrutinee).max(m_depth(motive)).max(case_max)
        }
        // SMT certificates carry no M-iteration of their own — they
        // witness a propositional fact about terms already in scope.
        CoreTerm::SmtProof(_) => 0,
        // An `Axiom` node in the kernel is a *term* — a proof witness
        // of its claimed type. Its depth is therefore `dp(ty)`, NOT
        // `dp(ty) + 1`. The schema-declaration side of a framework
        // axiom (which *would* bump by +1,) is handled
        // by the declaration-time path (`AxiomRegistry::register`) —
        // that's where we reason about the axiom as a meta-statement.
        // Here we are only looking at invocation sites.
        //
        // The load-bearing depth bump comes from `Inductive` — a
        // named schema lives strictly above its instantiation
        // arguments, which is what blocks Yanofsky α: Y → T^Y
        // (`dp(T^Y) = dp(Y) + 1` forces the strict inequality the
        // diagonal construction needs, and `K-Refine` forbids exactly
        // that equality).
        CoreTerm::Axiom { ty, .. } => m_depth(ty),
        //  ε(α) and α(ε) carry the M-depth of their argument.
        // The 2-natural equivalence τ : ε∘M ≃ A∘ε from Proposition 5.1
        // does not change m_depth at the term level — it lives at the
        // 2-cell level handled by `Kernel::check_eps_mu_coherence`.
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => m_depth(t),
        // Modal-depth: modal operators inherit M-depth from their operand.
        // The M-iteration depth (used by K-Refine) does NOT see modal
        // structure; the modal-depth (used by K-Refine-omega) is a
        // *separate* ordinal-valued quantity computed by `m_depth_omega`.
        CoreTerm::ModalBox(t) | CoreTerm::ModalDiamond(t) => m_depth(t),
        CoreTerm::ModalBigAnd(args) => {
            args.iter().map(|t| m_depth(t)).max().unwrap_or(0)
        }
        // cohesive modalities are not in the M-iteration
        // family; m_depth descends without modal increment, leaving
        // ordinal-modal-depth tracking to `m_depth_omega`.
        CoreTerm::Shape(t) | CoreTerm::Flat(t) | CoreTerm::Sharp(t) => {
            m_depth(t)
        }
        // quotient types: max over base + equiv
        // (the constructor's structural depth).
        CoreTerm::Quotient { base, equiv } => m_depth(base).max(m_depth(equiv)),
        CoreTerm::QuotIntro { value, base, equiv } => {
            m_depth(value).max(m_depth(base)).max(m_depth(equiv))
        }
        CoreTerm::QuotElim { scrutinee, motive, case } => {
            m_depth(scrutinee).max(m_depth(motive)).max(m_depth(case))
        }
    }
}

// =============================================================================
// Modal-depth (V0) — K-Refine-omega ordinal modal-depth
// =============================================================================

/// Ordinal-valued modal-depth for the K-Refine-omega kernel rule
/// (Theorem 136.T transfinite stratification).
///
/// Encoding: Cantor-normal-form prefix below ε_0, mirroring the
/// stdlib `core.theory_interop.coord::Ordinal` shape (single source
/// of truth between kernel and stdlib). The kernel keeps its own
/// definition because it cannot depend on stdlib at the trust
/// boundary.
///
///   `OrdinalDepth { omega_coeff: 0, finite_offset: n }`  encodes  `n`
///   `OrdinalDepth { omega_coeff: 1, finite_offset: 0 }`  encodes  `ω`
///   `OrdinalDepth { omega_coeff: 1, finite_offset: k }`  encodes  `ω + k`
///   `OrdinalDepth { omega_coeff: n, finite_offset: k }`  encodes  `ω·n + k`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrdinalDepth {
    /// ω-coefficient (0 ⇒ pure finite; 1 ⇒ ω; ≥ 2 ⇒ ω·n).
    pub omega_coeff: u32,
    /// Finite additive remainder.
    pub finite_offset: u32,
}

impl OrdinalDepth {
    /// Pure-finite depth — encoding of a usize.
    pub const fn finite(n: u32) -> Self {
        Self { omega_coeff: 0, finite_offset: n }
    }

    /// `ω`.
    pub const fn omega() -> Self {
        Self { omega_coeff: 1, finite_offset: 0 }
    }

    /// Lex ordering: `(omega_coeff, finite_offset)` lex.
    pub fn lt(&self, other: &Self) -> bool {
        if self.omega_coeff < other.omega_coeff { return true; }
        if self.omega_coeff > other.omega_coeff { return false; }
        self.finite_offset < other.finite_offset
    }

    /// `+ 1` — adds one to the ordinal in Cantor-normal form.
    ///
    /// **Soundness fix B4 **: pre-fix, the implementation
    /// used `finite_offset.saturating_add(1)`, which silently capped
    /// finite_offset at u32::MAX. The kernel rule
    /// `m_depth_omega(P) < base.succ()` then *accepted* a maximally-
    /// nested predicate over an omega-base — `(0, MAX).lt(&(1, 0))`
    /// is true under lex, so the rule passed regardless of how many
    /// modal operators the predicate actually contained. Soundness
    /// hole.
    ///
    /// V2 (this revision): when `finite_offset == u32::MAX`, the
    /// successor advances to the **next omega tier**: `(c, MAX) +
    /// 1 = (c + 1, 0)`. The omega coefficient itself saturates at
    /// `u32::MAX` (a strictly-larger ordinal would require ω², which
    /// the V0 lex encoding doesn't represent — the kernel rejects
    /// the rare-but-real case of `omega_coeff == MAX` next-succ by
    /// staying at `(MAX, MAX)`, the largest representable ordinal,
    /// and the K-rule then correctly rejects since
    /// `(MAX, MAX).lt(&(MAX, MAX)) == false`).
    ///
    /// V3 will lift this to ω² + ε_0 limit-ordinal arithmetic per
    /// Cantor-normal form §3.2 (Pohlers 2009); for V0/V1/V2 the
    /// ω·n + k encoding is sufficient because `m_depth_omega` walks
    /// CoreTerms whose ModalBox/ModalDiamond depth is bounded by
    /// term size (at most ω after the saturation cascade through
    /// MAX).
    pub fn succ(&self) -> Self {
        if self.finite_offset == u32::MAX {
            // Cantor-normal-form carry: (c, MAX) + 1 = (c+1, 0).
            // omega_coeff saturates at MAX (the largest ordinal we
            // can represent in the V0 encoding). Further succ at
            // (MAX, MAX) stays at (MAX, MAX) — a deliberate fix
            // point that the kernel rule then rejects via
            // `pred.lt(&base.succ()) == pred.lt(&(MAX, MAX)) == false`
            // for any pred at (MAX, MAX), which is the desired
            // sound conservative-rejection behaviour.
            Self {
                omega_coeff: self.omega_coeff.saturating_add(1),
                finite_offset: 0,
            }
        } else {
            Self {
                omega_coeff: self.omega_coeff,
                finite_offset: self.finite_offset + 1,
            }
        }
    }

    /// Render as canonical Unicode text.
    pub fn render(&self) -> String {
        if self.omega_coeff == 0 {
            return self.finite_offset.to_string();
        }
        let head = if self.omega_coeff == 1 {
            String::from("ω")
        } else {
            format!("ω·{}", self.omega_coeff)
        };
        if self.finite_offset == 0 {
            head
        } else {
            format!("{}+{}", head, self.finite_offset)
        }
    }
}

/// Modal-depth — `K-Refine-omega` modal-depth function `md^ω`.
///
/// Per Definition 136.D1 (transfinite modal language L^ω_α):
///
///   md^ω(atomic)       = 0
///   md^ω(□φ)           = md^ω(φ) + 1
///   md^ω(◇φ)           = md^ω(φ) + 1
///   md^ω(⋀_{i<κ} P_i)  = sup_i md^ω(P_i)
///   md^ω(structural)   = max(md^ω of children)
///
/// Walks the term tree once, descending through all term shapes.
/// For non-modal terms the walk reduces to `max-of-children` which
/// preserves bit-identical behaviour with the V0 skeleton (modal
/// operators were the only Rank-bumping shapes anyway).
///
/// Termination: well-founded over the term tree depth (every
/// recursion descends to a strictly smaller subterm). Per Lemma
/// 136.L0 the ordinal recursion is well-defined for every term in
/// the canonical-primitive language L^ω_α.
///
/// Blocks: Berry, paradoxical Löb, paraconsistent
/// Curry, Beth-Monk ω-iteration, and any ω·k or ω^ω modal-paradox
/// witness. The K-Refine-omega rule (`check_refine_omega`) routes
/// the result through `OrdinalDepth::lt` to gate refinement-type
/// formation.
pub fn m_depth_omega(term: &CoreTerm) -> OrdinalDepth {
    match term {
        // Atomic / variable — md^ω = 0.
        CoreTerm::Var(_) => OrdinalDepth::finite(0),
        CoreTerm::Universe(_) => OrdinalDepth::finite(0),

        // Modal operators — the load-bearing recursion.
        CoreTerm::ModalBox(phi) | CoreTerm::ModalDiamond(phi) => {
            m_depth_omega(phi).succ()
        }
        CoreTerm::ModalBigAnd(args) => {
            // sup_i md^ω(P_i). For finite arity the supremum is the
            // pointwise max under the lex ordering. Empty conjunction
            // is the identity (md^ω = 0).
            let mut sup = OrdinalDepth::finite(0);
            for arg in args.iter() {
                let r = m_depth_omega(arg);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }

        // Structural — descend into immediate children, take max.
        CoreTerm::Pi { domain, codomain, .. } => {
            ord_max(m_depth_omega(domain), m_depth_omega(codomain))
        }
        CoreTerm::Lam { domain, body, .. } => {
            ord_max(m_depth_omega(domain), m_depth_omega(body))
        }
        CoreTerm::App(f, a) => ord_max(m_depth_omega(f), m_depth_omega(a)),
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            ord_max(m_depth_omega(fst_ty), m_depth_omega(snd_ty))
        }
        CoreTerm::Pair(a, b) => ord_max(m_depth_omega(a), m_depth_omega(b)),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => m_depth_omega(p),
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            ord_max(
                m_depth_omega(carrier),
                ord_max(m_depth_omega(lhs), m_depth_omega(rhs)),
            )
        }
        // §7.4 V3 — heterogeneous path-over: 4-child
        // generalisation of `PathTy`. Modal-depth ordinal is the
        // pairwise max over motive / path / lhs / rhs.
        CoreTerm::PathOver { motive, path, lhs, rhs } => ord_max(
            ord_max(m_depth_omega(motive), m_depth_omega(path)),
            ord_max(m_depth_omega(lhs), m_depth_omega(rhs)),
        ),
        CoreTerm::Refl(a) => m_depth_omega(a),
        CoreTerm::HComp { phi, walls, base } => ord_max(
            m_depth_omega(phi),
            ord_max(m_depth_omega(walls), m_depth_omega(base)),
        ),
        CoreTerm::Transp { path, regular, value } => ord_max(
            m_depth_omega(path),
            ord_max(m_depth_omega(regular), m_depth_omega(value)),
        ),
        CoreTerm::Glue { carrier, phi, fiber, equiv } => ord_max(
            ord_max(m_depth_omega(carrier), m_depth_omega(phi)),
            ord_max(m_depth_omega(fiber), m_depth_omega(equiv)),
        ),
        CoreTerm::Refine { base, predicate, .. } => {
            ord_max(m_depth_omega(base), m_depth_omega(predicate))
        }
        // quotient types under modal-depth ordinal.
        CoreTerm::Quotient { base, equiv } => {
            ord_max(m_depth_omega(base), m_depth_omega(equiv))
        }
        CoreTerm::QuotIntro { value, base, equiv } => ord_max(
            m_depth_omega(value),
            ord_max(m_depth_omega(base), m_depth_omega(equiv)),
        ),
        CoreTerm::QuotElim { scrutinee, motive, case } => ord_max(
            m_depth_omega(scrutinee),
            ord_max(m_depth_omega(motive), m_depth_omega(case)),
        ),
        CoreTerm::Inductive { args, .. } => {
            let mut sup = OrdinalDepth::finite(0);
            for arg in args.iter() {
                let r = m_depth_omega(arg);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut sup = ord_max(m_depth_omega(scrutinee), m_depth_omega(motive));
            for case in cases.iter() {
                let r = m_depth_omega(case);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }
        CoreTerm::SmtProof(_) => OrdinalDepth::finite(0),
        CoreTerm::Axiom { ty, .. } => m_depth_omega(ty),
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => m_depth_omega(t),
        // cohesive modalities. ∫ ⊣ ♭ ⊣ ♯ are bona-fide
        // modalities; each application bumps the ordinal modal-depth
        // by 1 (per Definition 136.D1's modality-as-Galois-connection
        // generalisation). The K-Refine-omega rule routes the result
        // through the same gate as ModalBox / ModalDiamond.
        CoreTerm::Shape(t) | CoreTerm::Flat(t) | CoreTerm::Sharp(t) => {
            m_depth_omega(t).succ()
        }
    }
}

/// Local helper — pointwise lex max for OrdinalDepth.
pub(crate) fn ord_max(a: OrdinalDepth, b: OrdinalDepth) -> OrdinalDepth {
    if a.lt(&b) { b } else { a }
}

/// Modal-depth (V0) — `K-Refine-omega` rule entry point.
///
/// Verifies the transfinite-stratification invariant
///
/// ```text
///     md^ω(P) < md^ω(A) + 1
/// ```
///
/// for a refinement type `{x : A | P(x)}`. V0 calls `m_depth_omega`
/// (skeleton) and applies the lex-ordinal `lt` test; V1 will route
/// modal operators through the full md^ω computation.
///
/// Returns `Ok(())` when the invariant holds, otherwise
/// `KernelError::ModalDepthExceeded` with both ranks rendered as
/// canonical Unicode text.
pub fn check_refine_omega(
    binder: &Text,
    base: &CoreTerm,
    predicate: &CoreTerm,
) -> Result<(), KernelError> {
    let base_rank = m_depth_omega(base);
    let pred_rank = m_depth_omega(predicate);
    let upper = base_rank.succ();
    if pred_rank.lt(&upper) {
        Ok(())
    } else {
        Err(KernelError::ModalDepthExceeded {
            binder: binder.clone(),
            base_rank: Text::from(base_rank.render()),
            pred_rank: Text::from(pred_rank.render()),
        })
    }
}

/// Auxiliary `m_depth` over [`UniverseLevel`] — extracted so the main
/// walker stays flat. Mirrors the `Universe` arm's cases.
fn m_depth_level(level: &UniverseLevel) -> usize {
    match level {
        UniverseLevel::Concrete(n) => *n as usize,
        UniverseLevel::Prop => 0,
        UniverseLevel::Variable(_) => 0,
        UniverseLevel::Succ(l) => 1 + m_depth_level(l),
        UniverseLevel::Max(a, b) => m_depth_level(a).max(m_depth_level(b)),
    }
}
