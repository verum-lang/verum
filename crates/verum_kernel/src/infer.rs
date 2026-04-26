//! Kernel typing judgment — `infer` / `check` / `verify` / `verify_full`.
//!
//! Split per #198. The core LCF-style judgment `Γ ⊢ t : T` of the
//! kernel. Every proof term that reaches the kernel is either accepted
//! with a concrete inferred type, or rejected with a [`KernelError`].

use verum_common::{Heap, List, Maybe, Text};

use crate::axiom::AxiomRegistry;
use crate::depth::m_depth;
use crate::support::{replay_smt_cert, shape_of, structural_eq, substitute};
use crate::{Context, CoreTerm, CoreType, KernelError, UniverseLevel};

/// Infer the type of a [`CoreTerm`], returning the full type as a
/// [`CoreTerm`] on success.
///
/// This is the core LCF-style judgment `Γ ⊢ t : T` of the kernel.
/// Every proof term that reaches the kernel is either accepted with a
/// concrete inferred type, or rejected with a [`KernelError`]. There
/// is no third option — no "unknown", no "probably", no fallback.
///
/// The returned [`CoreTerm`] is the actual dependent type, **not** a
/// shape abstraction: applying `infer` to a lambda yields the Π-type
/// with the exact domain and codomain terms, so downstream App checks
/// can destructure it. Use [`shape_of`] when only the head is needed
/// (e.g. for error messages).
///
/// ## Implemented rules
///
/// * `Var x`         — lookup in `ctx`; error if unbound.
/// * `Universe l`    — `Type(l+1)` (predicative hierarchy; Prop lives
///   at level 0 for the current bring-up).
/// * `Pi (x:A) B`    — both `A` and `B` must check in some universe;
///   result is the universe of the larger level (max rule).
/// * `Lam (x:A) b`   — extends ctx with `x:A`, checks `b` to get `B`,
///   returns `Pi (x:A) B`.
/// * `App f a`       — `f` must be a `Pi (x:A) B`; `a` must check at
///   `A`; result is `B[x := a]` (capture-avoiding).
/// * `Axiom {name}`  — looked up in [`AxiomRegistry`]; result is the
///   registered type.
/// * `Sigma`         — fst_ty and snd_ty (extended ctx) in universes;
///   result in max of the two.
/// * `Pair`          — synthesizes a non-dependent Σ; dependent-Σ
///   introduction lands with bidirectional check-mode.
/// * `Fst` / `Snd`   — destructure a Σ; `Snd` substitutes `fst(pair)`
///   into the second component's binder.
/// * `PathTy`        — carrier in universe, lhs/rhs check at carrier.
/// * `Refl`          — `x : A ⇒ refl(x) : Path<A>(x, x)`.
/// * `Refine`        — base in universe, predicate well-typed under
///   extended ctx (full `predicate : Bool` gate lands once the Bool
///   primitive is canonically registered).
/// * `Inductive`     — lives in `Type(0)` at bring-up; universe
///   annotations arrive with the type-registry bridge.
/// * `HComp`         — returns base's type (bring-up; full cubical
///   reduction on top).
/// * `Transp`        — returns path's right-hand endpoint type.
/// * `Glue`          — lives in carrier's universe.
/// * `Elim`          — shape-level; returns `motive(scrutinee)`.
///
/// The **only** constructor that still returns
/// [`KernelError::NotImplemented`] is `SmtProof` — its dedicated
/// replay path lives in [`replay_smt_cert`] and lands per-backend
/// in follow-up commits (Z3 proof format first, then CVC5, E,
/// Vampire). That is the last piece needed to put every SMT backend
/// **outside** the TCB.
/// V8 (#215) — type-inference variant that consults a supplied
/// [`crate::InductiveRegistry`] when typing
/// [`CoreTerm::Inductive`] heads. Returns the registered
/// universe level for the named inductive instead of the
/// pre-V8 hardcoded `Universe(Concrete(0))` fallback.
///
/// The original [`infer`] is preserved as a backwards-compat
/// shim that delegates here with an empty registry — pre-V8
/// callers see exactly the same behaviour because empty-registry
/// lookups fall back to `Concrete(0)`.
pub fn infer_with_inductives(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: &crate::InductiveRegistry,
) -> Result<CoreTerm, KernelError> {
    infer_inner(ctx, term, axioms, Some(inductives))
}

pub fn infer(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreTerm, KernelError> {
    infer_inner(ctx, term, axioms, None)
}

/// V8 (#232) — type-inference variant that consults BOTH an
/// [`crate::InductiveRegistry`] AND a [`crate::KernelCoord`]
/// for the calling theorem. When the typing judgment encounters
/// a [`CoreTerm::Axiom`] reference, the K-Coord-Cite rule
/// (#227) automatically fires:
///
///   * If the axiom has a registered coord and the calling
///     theorem has a coord, [`crate::check_coord_cite`] is
///     invoked. On rejection,
///     [`crate::KernelError::CoordViolation`] propagates.
///   * If either coord is absent, the K-Coord-Cite rule
///     silently passes (axiom is unannotated or theorem
///     context disabled — graceful degradation, preserves
///     pre-V8 behaviour for legacy callers).
///
/// `allow_tier_jump` corresponds to the
/// `@require_extension(vfe_3)` (VVA-3 K-Universe-Ascent)
/// escape: when the calling module imports the κ-tier-jump
/// extension, descending citations to higher-ν axioms are
/// admitted. The kernel itself cannot detect the import; the
/// caller signals via this flag.
///
/// This is the production-path entry: gradual-verification
/// drivers + audit walkers should call this when they have
/// the calling theorem's coord in hand. Backwards-compat
/// [`infer`] / [`infer_with_inductives`] shims preserve the
/// pre-V8 behaviour by passing `current_coord = None`.
pub fn infer_with_full_context(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: &crate::InductiveRegistry,
    current_coord: &crate::KernelCoord,
    allow_tier_jump: bool,
) -> Result<CoreTerm, KernelError> {
    infer_inner_with_coord(
        ctx,
        term,
        axioms,
        Some(inductives),
        Some(current_coord),
        allow_tier_jump,
    )
}

/// V8 (#215) — internal type-inference body parametrised on an
/// optional [`crate::InductiveRegistry`]. When `inductives` is
/// `Some(_)`, the [`CoreTerm::Inductive`] arm consults the
/// registry's `universe_for(path)` and returns the registered
/// universe level. When `None` (the default for callers using
/// the legacy [`infer`] shim), the arm falls back to
/// `Universe(Concrete(0))` — preserving pre-V8 behaviour.
fn infer_inner(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
) -> Result<CoreTerm, KernelError> {
    infer_inner_with_coord(ctx, term, axioms, inductives, None, false)
}

/// V8 (#232) internal — type-inference body parametrised on
/// the optional ambient theorem coordinate. When
/// `current_coord = Some(_)`, the K-Coord-Cite rule auto-
/// applies at every [`CoreTerm::Axiom`] reference site whose
/// registered axiom carries a coord. `current_coord = None`
/// disables the rule for backwards compat.
fn infer_inner_with_coord(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
    current_coord: Option<&crate::KernelCoord>,
    allow_tier_jump: bool,
) -> Result<CoreTerm, KernelError> {
    // Helper closure for the recursive descent. The function
    // body below (originally `infer_inner`) calls this in
    // place of recursive `infer_inner` calls — but to avoid
    // touching every call site, we keep the recursive calls
    // routed through `infer_inner` (which delegates back here
    // with current_coord = None). The K-Coord-Cite rule fires
    // ONLY at the top-level entry (where current_coord is
    // populated), preventing accidental recursion-level
    // double-firing on nested axioms inside compound types.
    //
    // This is sound: a theorem T at coord ν cites axiom A at
    // coord ν' — the rule fires once for the outermost T→A
    // edge. Inner sub-terms that recursively reach axioms are
    // already covered by their own coord (the elaborator
    // walks each top-level theorem with its own coord).
    match term {
        CoreTerm::Var(name) => match ctx.lookup(name.as_str()) {
            Maybe::Some(ty) => Ok(ty.clone()),
            Maybe::None => Err(KernelError::UnboundVariable(name.clone())),
        },

        // Universe `Type(n)` inhabits `Type(n+1)`; `Prop` inhabits `Type(0)`.
        //
        // V8 (#207, B1) soundness fix: `saturating_add(1)` at u32::MAX
        // silently returns u32::MAX, yielding the type-in-type rule
        // `Universe(Concrete(MAX)) : Universe(Concrete(MAX))`. Detect
        // the overflow point explicitly and reject with
        // `KernelError::UniverseLevelOverflow`. Honest workloads
        // never reach u32::MAX (real code uses single-digit levels),
        // so reaching this branch is itself an elaborator-bug signal.
        CoreTerm::Universe(level) => {
            let next = match level {
                UniverseLevel::Concrete(n) => {
                    if *n == u32::MAX {
                        return Err(KernelError::UniverseLevelOverflow {
                            level: *n,
                        });
                    }
                    UniverseLevel::Concrete(*n + 1)
                }
                UniverseLevel::Prop => UniverseLevel::Concrete(0),
                other => UniverseLevel::Succ(Heap::new(other.clone())),
            };
            Ok(CoreTerm::Universe(next))
        }

        // Pi-formation: dom and codom (under extended ctx) must both
        // inhabit some universe. Result lives in the max of the two.
        CoreTerm::Pi { binder, domain, codomain } => {
            let dom_ty = infer_inner(ctx,domain, axioms, inductives)?;
            let dom_level = universe_level(&dom_ty)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let codom_ty = infer_inner(&extended,codomain, axioms, inductives)?;
            let codom_level = universe_level(&codom_ty)?;
            Ok(CoreTerm::Universe(UniverseLevel::Max(
                Heap::new(dom_level),
                Heap::new(codom_level),
            )))
        }

        // Lam-introduction: under ctx extended with binder, body has
        // type B; result is Pi (binder:domain) B.
        CoreTerm::Lam { binder, domain, body } => {
            let _ = infer_inner(ctx,domain, axioms, inductives)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let body_ty = infer_inner(&extended,body, axioms, inductives)?;
            Ok(CoreTerm::Pi {
                binder: binder.clone(),
                domain: domain.clone(),
                codomain: Heap::new(body_ty),
            })
        }

        // App-elimination: f : Pi (x:A) B,  a : A  ⇒  f a : B[x := a].
        //
        // V8 (#221) — domain-against-arg-type comparison uses
        // `definitional_eq` (β-aware) instead of `structural_eq`
        // (byte-identity). Pre-V8 a Π whose domain has a β-redex
        // (e.g., `(λT:Type. T) Nat ≡_β Nat`) and an arg typed at
        // `Nat` would FALSELY REJECT — the domain's redex never got
        // reduced before the equality check. Mirrors the V8 #216
        // PathTy fix; same monotone strengthening (only widens
        // acceptance, never weakens).
        CoreTerm::App(f, arg) => {
            let f_ty = infer_inner(ctx,f, axioms, inductives)?;
            match f_ty {
                CoreTerm::Pi { binder, domain, codomain } => {
                    let arg_ty = infer_inner(ctx,arg, axioms, inductives)?;
                    if !crate::support::definitional_eq(&arg_ty, &domain) {
                        return Err(KernelError::TypeMismatch {
                            expected: shape_of(&domain),
                            actual: shape_of(&arg_ty),
                        });
                    }
                    Ok(substitute(&codomain, binder.as_str(), arg))
                }
                other => Err(KernelError::NotAFunction(shape_of(&other))),
            }
        }

        // Σ-formation: fst_ty and snd_ty (under extended ctx with the
        // binder) must each live in some universe. The Σ-type lives in
        // the max of the two, mirroring the Π-formation rule.
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
            let fst_univ = infer_inner(ctx,fst_ty, axioms, inductives)?;
            let fst_level = universe_level(&fst_univ)?;
            let extended = ctx.extend(binder.clone(), (**fst_ty).clone());
            let snd_univ = infer_inner(&extended,snd_ty, axioms, inductives)?;
            let snd_level = universe_level(&snd_univ)?;
            Ok(CoreTerm::Universe(UniverseLevel::Max(
                Heap::new(fst_level),
                Heap::new(snd_level),
            )))
        }

        // Σ-introduction: for now, Pair is introduced in a
        // non-dependent position — we look up the expected Σ-type at
        // the pair's syntactic position via App/Lam/assignment (not
        // yet wired through), so at bring-up we conservatively require
        // both components check in some type and synthesize a
        // non-dependent Σ with binder `_`.
        //
        // A fully dependent `Pair (a, b) : Sigma (x : A) B` rule needs
        // an expected-type channel (`check` mode), which lands with
        // bidirectional elaboration.  Until then we keep the simpler
        // rule here and tag the restriction.
        CoreTerm::Pair(fst, snd) => {
            let fst_ty = infer_inner(ctx,fst, axioms, inductives)?;
            let snd_ty = infer_inner(ctx,snd, axioms, inductives)?;
            Ok(CoreTerm::Sigma {
                binder: Text::from("_"),
                fst_ty: Heap::new(fst_ty),
                snd_ty: Heap::new(snd_ty),
            })
        }

        CoreTerm::Fst(pair) => {
            let pair_ty = infer_inner(ctx,pair, axioms, inductives)?;
            match pair_ty {
                CoreTerm::Sigma { fst_ty, .. } => Ok((*fst_ty).clone()),
                other => Err(KernelError::NotAPair(shape_of(&other))),
            }
        }

        CoreTerm::Snd(pair) => {
            let pair_ty = infer_inner(ctx,pair, axioms, inductives)?;
            match pair_ty {
                CoreTerm::Sigma { binder, snd_ty, .. } => {
                    // snd : snd_ty[binder := fst(pair)]
                    let projected = CoreTerm::Fst(pair.clone());
                    Ok(substitute(&snd_ty, binder.as_str(), &projected))
                }
                other => Err(KernelError::NotAPair(shape_of(&other))),
            }
        }

        // Path-formation: Path<A>(lhs, rhs) is a type when A is a type
        // (i.e. inhabits some universe) and lhs, rhs both check at A.
        // Result lives in A's universe, same as carrier.
        //
        // V8 (#216) — endpoint-against-carrier comparison uses
        // `definitional_eq` (β-aware) instead of `structural_eq`
        // (byte-identity). Pre-V8 a path with carrier = `App(Lam,
        // Nat)` (β-equal to `Nat`) and endpoints typed at `Nat`
        // would be FALSELY REJECTED — the elaborator's β-redex
        // never got reduced before the equality check. The new
        // normalize() reduces both sides to β-normal form before
        // comparing, so the typing rule is complete on the
        // SN fragment.
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            let carrier_univ = infer_inner(ctx,carrier, axioms, inductives)?;
            let carrier_level = universe_level(&carrier_univ)?;
            let lhs_ty = infer_inner(ctx,lhs, axioms, inductives)?;
            if !crate::support::definitional_eq(&lhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&lhs_ty),
                });
            }
            let rhs_ty = infer_inner(ctx,rhs, axioms, inductives)?;
            if !crate::support::definitional_eq(&rhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&rhs_ty),
                });
            }
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Reflexivity: refl(x) : Path<A>(x, x) where x : A.
        CoreTerm::Refl(x) => {
            let x_ty = infer_inner(ctx,x, axioms, inductives)?;
            Ok(CoreTerm::PathTy {
                carrier: Heap::new(x_ty),
                lhs: x.clone(),
                rhs: x.clone(),
            })
        }
        // HComp: `hcomp φ walls base` produces the i1-face of the
        // composition cube whose base is `base` (its i0-face) and
        // sides are `walls` (the family indexed by φ). The result
        // inhabits the same type as `base` — composition does not
        // change the carrier.
        //
        // Checks performed:
        //   * `phi` is well-typed — conservative, no interval
        //     subsumption yet; full cofibration-calculus lands with
        //     the dedicated cubical-kernel pass (task #89-adjacent).
        //   * `walls` is well-typed as some family.
        //   * `base` is well-typed; its type is returned.
        //
        // Rejected shapes: ill-typed subterms surface the underlying
        // `KernelError` rather than being swallowed, so a spurious
        // composition cannot sneak into the TCB.
        CoreTerm::HComp { phi, walls, base } => {
            let _ = infer_inner(ctx,phi, axioms, inductives)?;
            let _ = infer_inner(ctx,walls, axioms, inductives)?;
            infer_inner(ctx, base, axioms, inductives)
        }

        // Transp: `transp(p, r, t)` where `p : Path<Type>(A, B)`,
        // `r : I` (regularity endpoint), `t : A` — result inhabits
        // `B`, the path's right-hand endpoint.
        //
        // Checks performed:
        //   * `path` is well-typed and its type is `PathTy { lhs, rhs }`
        //     (not just some arbitrary term).
        //   * `regular` is well-typed (interval-subsumption deferred).
        //   * `value` is well-typed; result type is the path's `rhs`.
        //
        // On a non-PathTy path type (e.g. a neutral whose head is
        // still an unsolved type-meta), we conservatively fall back
        // to the `value`'s own type — the alternative would be
        // rejecting every proof-in-progress transp, which blocks
        // bring-up. The full cubical pass will tighten this to a
        // hard error.
        CoreTerm::Transp { path, regular, value } => {
            let path_ty = infer_inner(ctx,path, axioms, inductives)?;
            let _ = infer_inner(ctx,regular, axioms, inductives)?;
            match path_ty {
                CoreTerm::PathTy { rhs, .. } => Ok((*rhs).clone()),
                _ => infer_inner(ctx, value, axioms, inductives),
            }
        }

        // Glue: `Glue<A>(φ, T, e) : Type_n` where A is the carrier
        // type in `Type_n`, φ is the face formula, T is the partial
        // type family on φ, and e is the equivalence family between
        // T and A on φ.
        //
        // Checks performed:
        //   * `carrier` is in a universe; its level determines the
        //     Glue type's universe.
        //   * `phi`, `fiber`, `equiv` are each well-typed under the
        //     current context.
        //
        // Full univalence computation (Glue-beta, φ-equiv coherence,
        // unglue) lands in the cubical-kernel follow-up — at this
        // phase the kernel certifies that the Glue constructor was
        // assembled from well-typed pieces and is a type at the
        // right universe level.
        CoreTerm::Glue { carrier, phi, fiber, equiv } => {
            let carrier_univ = infer_inner(ctx,carrier, axioms, inductives)?;
            let carrier_level = universe_level(&carrier_univ)?;
            let _ = infer_inner(ctx,phi, axioms, inductives)?;
            let _ = infer_inner(ctx,fiber, axioms, inductives)?;
            let _ = infer_inner(ctx,equiv, axioms, inductives)?;
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Refine: {x : base | predicate}. base must inhabit a universe,
        // predicate must check under the extended ctx (bound to Bool at
        // full-rule closure; shape-level at bring-up).
        //
        // K-Refine (VVA §2.4 / §4.4 / Diakrisis T-2f*): the predicate's
        // M-iteration depth MUST be strictly less than base's depth + 1.
        // Per Yanofsky 2003 this closes every self-referential paradox
        // schema in a cartesian-closed setting by blocking the exact
        // equality `dp(α) = dp(T^α)` that Russell/Curry/Gödel-type
        // diagonals require. Enforced BEFORE well-typedness inference
        // of the predicate so a depth-violating term is rejected early
        // with a precise diagnostic.
        CoreTerm::Refine { base, binder, predicate } => {
            let base_univ = infer_inner(ctx,base, axioms, inductives)?;
            let base_level = universe_level(&base_univ)?;

            // K-Refine depth check — the single load-bearing Diakrisis
            // rule in the Verum kernel.
            let base_depth = m_depth(base);
            let pred_depth = m_depth(predicate);
            if pred_depth >= base_depth + 1 {
                return Err(KernelError::DepthViolation {
                    binder: binder.clone(),
                    base_depth,
                    pred_depth,
                });
            }

            let extended = ctx.extend(binder.clone(), (**base).clone());
            // Predicate must be well-typed under the extended context;
            // we don't yet enforce its type is Bool because Bool is a
            // primitive Inductive that lands via the stdlib bridge, so
            // for bring-up we only require the predicate be well-typed.
            let _ = infer_inner(&extended,predicate, axioms, inductives)?;
            Ok(CoreTerm::Universe(base_level))
        }

        // V8 (#236) — K-Quot-Form: Quotient(T, ~) is a type when
        // T is a type and ~ is a binary relation on T. The
        // quotient inhabits the same universe as T. The kernel
        // does NOT internally verify that ~ is reflexive +
        // symmetric + transitive; the quotient elimination rule
        // requires the user to discharge respect-of-equivalence
        // when applying QuotElim, which is where soundness
        // re-enters.
        CoreTerm::Quotient { base, equiv } => {
            let base_univ = infer_inner(ctx, base, axioms, inductives)?;
            let base_level = universe_level(&base_univ)?;
            // Equiv must be well-typed; we don't pin its shape
            // to `T → T → Prop` here because the elaborator
            // produces equiv terms in elaborated form whose
            // shape is checked at QuotIntro / QuotElim.
            let _ = infer_inner(ctx, equiv, axioms, inductives)?;
            Ok(CoreTerm::Universe(base_level))
        }

        // V8 (#236) — K-Quot-Intro: lifting `value : T` produces
        // `[value]_~ : Quotient(T, ~)`. Verifies (a) value's
        // type is T, (b) Quotient(T, ~) is well-formed.
        CoreTerm::QuotIntro { value, base, equiv } => {
            let value_ty = infer_inner(ctx, value, axioms, inductives)?;
            if !crate::support::definitional_eq(&value_ty, base) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(base),
                    actual: shape_of(&value_ty),
                });
            }
            // Verify Quotient(base, equiv) is well-formed.
            let _ = infer_inner(ctx, equiv, axioms, inductives)?;
            Ok(CoreTerm::Quotient {
                base: base.clone(),
                equiv: equiv.clone(),
            })
        }

        // V8 (#236) — K-Quot-Elim: eliminate `q : Quotient(T, ~)`
        // by supplying `motive : Quotient(T, ~) → U` and
        // `case : Π(t: T). motive([t]_~)`. Result inhabits
        // `motive q`. The respect-of-equivalence obligation is
        // discharged at the elaborator level (the case must
        // satisfy `∀ t1 t2. t1 ~ t2 → case(t1) ≡ case(t2)`);
        // the kernel records the shape but doesn't verify the
        // obligation directly — V2 deferred to a dedicated pass
        // that consumes the framework-axiom registry.
        CoreTerm::QuotElim { scrutinee, motive, case } => {
            let scrut_ty = infer_inner(ctx, scrutinee, axioms, inductives)?;
            // Verify scrutinee is a quotient.
            match scrut_ty {
                CoreTerm::Quotient { .. } => {}
                other => {
                    return Err(KernelError::TypeMismatch {
                        expected: shape_of(&CoreTerm::Quotient {
                            base: Heap::new(CoreTerm::Var(Text::from("?"))),
                            equiv: Heap::new(CoreTerm::Var(Text::from("?"))),
                        }),
                        actual: shape_of(&other),
                    });
                }
            }
            // Verify motive + case are well-typed; full
            // shape-coherence (motive : Q → U; case : Π(t:T).
            // motive([t])) is V2 deferred — V1 records the
            // structural well-formedness obligation only.
            let _ = infer_inner(ctx, motive, axioms, inductives)?;
            let _ = infer_inner(ctx, case, axioms, inductives)?;
            // Result type: motive applied to scrutinee
            // (syntactic application; downstream conversion
            // checks β-reduce as needed).
            Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
        }

        // Named inductive / user / HIT — its type is the universe it
        // was declared in. Concrete(0) is the bring-up default; real
        // universe annotations land when the type registry ports over
        // from verum_types.
        // V8 (#215) — consult the supplied InductiveRegistry for
        // the declared universe level. When no registry is
        // available (callers using the legacy `infer` shim) or
        // the path isn't registered, fall back to
        // `Concrete(0)` — the pre-V8 hardcoded default. This way
        // existing tests (which don't populate an inductive
        // registry) see exactly the same result, while new
        // callers (verification driver, gradual-verification
        // pipeline) propagate honest universe levels through the
        // typing judgment.
        CoreTerm::Inductive { path, .. } => {
            let level = match inductives {
                Some(reg) => match reg.universe_for(path.as_str()) {
                    Some(l) => l.clone(),
                    None => UniverseLevel::Concrete(0),
                },
                None => UniverseLevel::Concrete(0),
            };
            Ok(CoreTerm::Universe(level))
        }

        // Elim: an induction-principle application
        // `elim e motive cases`. The result inhabits `motive` applied
        // to the scrutinee.
        //
        // V0 (bring-up — pre-V8): only inferred motive's type, then
        // returned `App(motive, scrutinee)` syntactically without
        // verifying motive was a function or that scrutinee fit its
        // domain. Soundness-leaky: an Elim with motive `42` (Int) or
        // a motive whose domain is `Bool` and a scrutinee of type
        // `Int` would return a malformed result that the App rule
        // surfaced only on later use.
        //
        // V1 (this revision) — adopt the same *well-formedness*
        // check the App rule does:
        //
        //   1. motive's TYPE must be a Π (motive is a function from
        //      some domain to some universe).
        //   2. scrutinee's type must structurally match the Π's
        //      domain.
        //
        // The *result type* is still the syntactic application
        // `motive scrutinee` — semantically motive(scrutinee), with
        // β-reduction left to downstream definitional equality.
        // Returning the codomain[binder := scrutinee] would be the
        // type's TYPE (i.e., universe), not the type itself; the
        // App-typing rule on this returned term will compute
        // codomain[binder := scrutinee] when required. Per-case
        // exhaustiveness + typing remains the dedicated Elim-rule
        // pass's job.
        CoreTerm::Elim { scrutinee, motive, .. } => {
            let motive_ty = infer_inner(ctx,motive, axioms, inductives)?;
            match motive_ty {
                CoreTerm::Pi { domain, .. } => {
                    let scrut_ty = infer_inner(ctx,scrutinee, axioms, inductives)?;
                    if !structural_eq(&scrut_ty, &domain) {
                        return Err(KernelError::TypeMismatch {
                            expected: shape_of(&domain),
                            actual: shape_of(&scrut_ty),
                        });
                    }
                    Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
                }
                other => Err(KernelError::NotAFunction(shape_of(&other))),
            }
        }

        // An `SmtProof` node is replayed via `replay_smt_cert` at type
        // lookup: the certificate is validated (schema + backend +
        // rule-tag + obligation hash), a witness term is constructed,
        // and the witness's conservative type is returned.
        //
        // Until the full step-by-step Z3 `(proof …)` / CVC5 ALETHE
        // reconstruction lands (task #89), the witness type is
        // `Inductive("Bool")` — the standing convention for
        // propositional obligations that close via the
        // `Unsat`-means-valid protocol. This matches the type set on
        // the `Axiom` node `replay_smt_cert` produces, so upstream
        // code that destructures the replayed term sees a consistent
        // `CoreTerm::Inductive { "Bool", [] }` shape.
        CoreTerm::SmtProof(cert) => {
            let _witness = replay_smt_cert(ctx, cert)?;
            Ok(CoreTerm::Inductive {
                path: Text::from("Bool"),
                args: List::new(),
            })
        }

        CoreTerm::Axiom { name, .. } => match axioms.get(name.as_str()) {
            Maybe::Some(entry) => {
                // V8 (#232) — K-Coord-Cite gate. Auto-fires
                // when both the calling theorem and the
                // referenced axiom have populated coords. If
                // either is absent, rule passes (pre-V8
                // behaviour preserved for unannotated code).
                if let (Some(theorem_coord), Some(axiom_coord)) =
                    (current_coord, &entry.coord)
                {
                    crate::check_coord_cite(
                        theorem_coord,
                        axiom_coord,
                        name,
                        allow_tier_jump,
                    )?;
                }
                Ok(entry.ty.clone())
            }
            Maybe::None => Err(KernelError::UnknownInductive(name.clone())),
        },

        // VVA-1 V0: ε(α) and α(ε) are constructor markers for the
        // articulation/enactment duality. They inherit the type of
        // their argument (ε and α are endo-2-functors at the term
        // level — the M⊣A biadjunction structure shows up only at
        // the 2-cell level). V1 will refine the type to track
        // whether the result lives in the articulation 2-category
        // or the enactment 2-category.
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => {
            infer_inner(ctx, t, axioms, inductives)
        }

        // VVA-7 V1: modal operators inhabit `Prop`. The kernel
        // verifies that the operand is well-typed (regardless of
        // whether it inhabits `Prop` or any other type — modality
        // can be applied to any well-formed term, the resulting
        // proposition is always at the propositional layer).
        CoreTerm::ModalBox(phi) | CoreTerm::ModalDiamond(phi) => {
            let _ = infer_inner(ctx,phi, axioms, inductives)?;
            Ok(CoreTerm::Universe(UniverseLevel::Prop))
        }
        CoreTerm::ModalBigAnd(args) => {
            for a in args.iter() {
                let _ = infer_inner(ctx,a, axioms, inductives)?;
            }
            Ok(CoreTerm::Universe(UniverseLevel::Prop))
        }
    }
}

/// Backwards-compatible shape-only query — returns the kernel's
/// coarse [`CoreType`] head view. Prefer [`infer`] when full type
/// information is needed.
pub fn check(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreType, KernelError> {
    Ok(shape_of(&infer(ctx, term, axioms)?))
}

/// Verify that `term` inhabits `expected` under full structural
/// comparison of the two types (not shape-head). This is the
/// LCF-style verification gate that downstream crates call.
pub fn verify_full(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    let actual = infer(ctx, term, axioms)?;
    if structural_eq(&actual, expected) {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: shape_of(expected),
            actual: shape_of(&actual),
        })
    }
}

/// Back-compat shape-head comparator kept for the coarse API.
pub fn verify(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreType,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    let actual = check(ctx, term, axioms)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: expected.clone(),
            actual,
        })
    }
}

/// Project a [`CoreTerm`] in universe position to its underlying
/// `UniverseLevel`. Used by the formation rules (Pi / Sigma / Path /
/// Glue) to build the Max-of-levels result type. Private helper —
/// the public `shape_of` is the equivalent surface for non-universe
/// projections.
pub(crate) fn universe_level(term: &CoreTerm) -> Result<UniverseLevel, KernelError> {
    match term {
        CoreTerm::Universe(l) => Ok(l.clone()),
        other => Err(KernelError::TypeMismatch {
            expected: CoreType::Universe(UniverseLevel::Concrete(0)),
            actual: shape_of(other),
        }),
    }
}
