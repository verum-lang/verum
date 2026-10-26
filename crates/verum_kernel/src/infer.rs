//! Kernel typing judgment — `infer` / `check` / `verify` / `verify_full`.
//!
//! Split . The core LCF-style judgment `Γ ⊢ t : T` of the
//! kernel. Every proof term that reaches the kernel is either accepted
//! with a concrete inferred type, or rejected with a [`KernelError`].

use verum_common::{Heap, List, Maybe, Text};

use crate::axiom::AxiomRegistry;
use crate::depth::m_depth;
use crate::support::{replay_smt_cert, shape_of, substitute};
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
/// * `Var x` — lookup in `ctx`; error if unbound.
/// * `Universe l` — `Type(l+1)` (predicative hierarchy; Prop lives
///  at level 0 for the current bring-up).
/// * `Pi (x:A) B` — both `A` and `B` must check in some universe;
///  result is the universe of the larger level (max rule).
/// * `Lam (x:A) b` — extends ctx with `x:A`, checks `b` to get `B`,
///  returns `Pi (x:A) B`.
/// * `App f a` — `f` must be a `Pi (x:A) B`; `a` must check at
///  `A`; result is `B[x := a]` (capture-avoiding).
/// * `Axiom {name}` — looked up in [`AxiomRegistry`]; result is the
///  registered type.
/// * `Sigma` — fst_ty and snd_ty (extended ctx) in universes;
///  result in max of the two.
/// * `Pair` — synthesizes a non-dependent Σ; dependent-Σ
///  introduction lands with bidirectional check-mode.
/// * `Fst` / `Snd` — destructure a Σ; `Snd` substitutes `fst(pair)`
///  into the second component's binder.
/// * `PathTy` — carrier in universe, lhs/rhs check at carrier.
/// * `Refl` — `x : A ⇒ refl(x) : Path<A>(x, x)`.
/// * `Refine` — base in universe, predicate well-typed under
///  extended ctx (full `predicate : Bool` gate lands once the Bool
///  primitive is canonically registered).
/// * `Inductive` — requires an explicitly registered declaration and checks
///   its parameters against the declared universe.
/// * `HComp` — returns base's type (bring-up; full cubical
///  reduction on top).
/// * `Transp` — returns path's right-hand endpoint type.
/// * `Glue` — lives in carrier's universe.
/// * `Elim` — checks a registered ordinary inductive and all dependent branches,
///   including recursive hypotheses; returns `motive(scrutinee)`.
///
/// The **only** constructor that still returns
/// [`KernelError::NotImplemented`] is `SmtProof` — its dedicated
/// replay path lives in [`replay_smt_cert`] and lands per-backend
/// in follow-up commits (Z3 proof format first, then CVC5, E,
/// Vampire). That is the last piece needed to put every SMT backend
/// **outside** the TCB.
/// type-inference variant that consults a supplied
/// [`crate::InductiveRegistry`] when typing
/// [`CoreTerm::Inductive`] heads. Returns the registered
/// universe level for the named inductive instead of the
/// pre-V8 hardcoded `Universe(Concrete(0))` fallback.
///
/// [`infer`] checks the inductive-free fragment. Named inductive heads always
/// require an explicit declaration; missing names are errors.
pub fn infer_with_inductives(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: &crate::InductiveRegistry,
) -> Result<CoreTerm, KernelError> {
    infer_inner(ctx, term, axioms, Some(inductives))
}

/// Infer the type of a [`CoreTerm`] under `ctx` and the registered
/// `axioms`. Use [`infer_with_inductives`] when an
/// `InductiveRegistry` is needed for `K-Inductive` / `K-Elim`
/// dispatch; this entry point handles the inductive-free fragment.
pub fn infer(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreTerm, KernelError> {
    infer_inner(ctx, term, axioms, None)
}

/// type-inference variant that consults BOTH an
/// [`crate::InductiveRegistry`] AND a [`crate::KernelCoord`]
/// for the calling theorem. When the typing judgment encounters
/// a [`CoreTerm::Axiom`] reference, the K-Coord-Cite rule
///  automatically fires:
///
///  * If the axiom has a registered coord and the calling
///  theorem has a coord, [`crate::check_coord_cite`] is
///  invoked. On rejection,
///  [`crate::KernelError::CoordViolation`] propagates.
///  * If either coord is absent, the K-Coord-Cite rule
///  silently passes (axiom is unannotated or theorem
///  context disabled — graceful degradation, preserves
///  pre-V8 behaviour for legacy callers).
///
/// `allow_tier_jump` corresponds to the
/// `@require_extension(vfe_3)` (Categorical coherence K-Universe-Ascent)
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

/// Enter the same checker without an ambient theorem coordinate.
fn infer_inner(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
) -> Result<CoreTerm, KernelError> {
    infer_inner_with_coord(ctx, term, axioms, inductives, None, false)
}

/// internal — type-inference body parametrised on
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
    axioms.validate_inductive_dependencies(inductives)?;
    if let Some(registry) = inductives {
        registry.validate_axiom_dependencies(axioms)?;
    }
    Inference {
        axioms,
        inductives,
        current_coord,
        allow_tier_jump,
    }
    .infer(ctx, term)
}

/// One immutable environment follows every recursive typing obligation.
struct Inference<'a> {
    axioms: &'a AxiomRegistry,
    inductives: Option<&'a crate::InductiveRegistry>,
    current_coord: Option<&'a crate::KernelCoord>,
    allow_tier_jump: bool,
}

impl Inference<'_> {
    fn infer(&self, ctx: &Context, term: &CoreTerm) -> Result<CoreTerm, KernelError> {
        let Self {
            axioms,
            inductives,
            current_coord,
            allow_tier_jump,
        } = *self;
        match term {
            CoreTerm::Var(name) => match ctx.lookup(name.as_str()) {
                Maybe::Some(ty) => Ok(ty.clone()),
                Maybe::None => Err(KernelError::UnboundVariable(name.clone())),
            },

            CoreTerm::Constructor {
                inductive,
                name,
                params,
                args,
            } => {
                let registry =
                    inductives.ok_or_else(|| KernelError::UnknownInductive(inductive.clone()))?;
                let declaration = registry
                    .get(inductive.as_str())
                    .ok_or_else(|| KernelError::UnknownInductive(inductive.clone()))?;
                self.validate_inductive_parameters(ctx, declaration, params, registry)?;
                let constructor = declaration
                    .constructors
                    .iter()
                    .find(|ctor| &ctor.name == name)
                    .ok_or_else(|| {
                        KernelError::InvalidEliminator(Text::from(
                            "constructor is not owned by this declaration",
                        ))
                    })?;
                if args.len() != constructor.arg_types.len() {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "constructor field count does not match its declaration",
                    )));
                }
                let instantiated = self.validate_constructor_signature(
                    ctx,
                    declaration,
                    constructor,
                    params,
                    registry,
                )?;
                for (argument, expected) in args.iter().zip(instantiated.arg_types.iter()) {
                    let actual = self.infer(ctx, argument)?;
                    if !convertible_in(&actual, expected, axioms, inductives) {
                        return Err(KernelError::TypeMismatch {
                            expected: shape_of(expected),
                            actual: shape_of(&actual),
                        });
                    }
                }
                Ok(CoreTerm::Inductive {
                    path: inductive.clone(),
                    args: params.clone(),
                })
            }

            // Universe `Type(n)` inhabits `Type(n+1)`; `Prop` inhabits `Type(0)`.
            //

            //  soundness fix: `saturating_add(1)` at u32::MAX
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
                            return Err(KernelError::UniverseLevelOverflow { level: *n });
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
            CoreTerm::Pi {
                binder,
                domain,
                codomain,
            } => {
                let dom_ty = self.infer(ctx, domain)?;
                let dom_level = universe_level(&dom_ty)?;
                let extended = ctx.extend(binder.clone(), (**domain).clone());
                let codom_ty = self.infer(&extended, codomain)?;
                let codom_level = universe_level(&codom_ty)?;
                Ok(CoreTerm::Universe(UniverseLevel::Max(
                    Heap::new(dom_level),
                    Heap::new(codom_level),
                )))
            }

            // Lam-introduction: under ctx extended with binder, body has
            // type B; result is Pi (binder:domain) B.
            CoreTerm::Lam {
                binder,
                domain,
                body,
            } => {
                let domain_kind = self.infer(ctx, domain)?;
                universe_level(&normalize_in(&domain_kind, axioms, inductives))?;
                let extended = ctx.extend(binder.clone(), (**domain).clone());
                let body_ty = self.infer(&extended, body)?;
                Ok(CoreTerm::Pi {
                    binder: binder.clone(),
                    domain: domain.clone(),
                    codomain: Heap::new(body_ty),
                })
            }

            // App-elimination: f : Pi (x:A) B, a : A ⇒ f a : B[x := a].
            //

            // domain-against-arg-type comparison uses
            // `definitional_eq` (β-aware) instead of `structural_eq`
            // (byte-identity). Pre-V8 a Π whose domain has a β-redex
            // (e.g., `(λT:Type. T) Nat ≡_β Nat`) and an arg typed at
            // `Nat` would FALSELY REJECT — the domain's redex never got
            // reduced before the equality check. Mirrors the
            // PathTy fix; same monotone strengthening (only widens
            // acceptance, never weakens).
            CoreTerm::App(f, arg) => {
                let f_ty = normalize_in(&self.infer(ctx, f)?, axioms, inductives);
                match f_ty {
                    CoreTerm::Pi {
                        binder,
                        domain,
                        codomain,
                    } => {
                        let arg_ty = self.infer(ctx, arg)?;
                        if !convertible_in(&arg_ty, &domain, axioms, inductives) {
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
            CoreTerm::Sigma {
                binder,
                fst_ty,
                snd_ty,
            } => {
                let fst_univ = self.infer(ctx, fst_ty)?;
                let fst_level = universe_level(&fst_univ)?;
                let extended = ctx.extend(binder.clone(), (**fst_ty).clone());
                let snd_univ = self.infer(&extended, snd_ty)?;
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
            // bidirectional elaboration. Until then we keep the simpler
            // rule here and tag the restriction.
            CoreTerm::Pair(fst, snd) => {
                let fst_ty = self.infer(ctx, fst)?;
                let snd_ty = self.infer(ctx, snd)?;
                Ok(CoreTerm::Sigma {
                    binder: Text::from("_"),
                    fst_ty: Heap::new(fst_ty),
                    snd_ty: Heap::new(snd_ty),
                })
            }

            CoreTerm::Fst(pair) => {
                let pair_ty = self.infer(ctx, pair)?;
                match pair_ty {
                    CoreTerm::Sigma { fst_ty, .. } => Ok((*fst_ty).clone()),
                    other => Err(KernelError::NotAPair(shape_of(&other))),
                }
            }

            CoreTerm::Snd(pair) => {
                let pair_ty = self.infer(ctx, pair)?;
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

            // endpoint-against-carrier comparison uses
            // `definitional_eq` (β-aware) instead of `structural_eq`
            // (byte-identity). Pre-V8 a path with carrier = `App(Lam,
            // Nat)` (β-equal to `Nat`) and endpoints typed at `Nat`
            // would be FALSELY REJECTED — the elaborator's β-redex
            // never got reduced before the equality check. The new
            // normalize() reduces both sides to β-normal form before
            // comparing, so the typing rule is complete on the
            // SN fragment.
            CoreTerm::PathTy { carrier, lhs, rhs } => {
                let carrier_univ = self.infer(ctx, carrier)?;
                let carrier_level = universe_level(&carrier_univ)?;
                let lhs_ty = self.infer(ctx, lhs)?;
                if !convertible_in(&lhs_ty, carrier, axioms, inductives) {
                    return Err(KernelError::TypeMismatch {
                        expected: shape_of(carrier),
                        actual: shape_of(&lhs_ty),
                    });
                }
                let rhs_ty = self.infer(ctx, rhs)?;
                if !convertible_in(&rhs_ty, carrier, axioms, inductives) {
                    return Err(KernelError::TypeMismatch {
                        expected: shape_of(carrier),
                        actual: shape_of(&rhs_ty),
                    });
                }
                Ok(CoreTerm::Universe(carrier_level))
            }

            // Reflexivity: refl(x) : Path<A>(x, x) where x : A.
            CoreTerm::Refl(x) => {
                let x_ty = self.infer(ctx, x)?;
                Ok(CoreTerm::PathTy {
                    carrier: Heap::new(x_ty),
                    lhs: x.clone(),
                    rhs: x.clone(),
                })
            }

            // A dependent path requires actual base-path evidence and endpoints
            // in the two fibers of the same checked family.
            CoreTerm::PathOver {
                motive,
                path,
                lhs,
                rhs,
            } => {
                let path_ty = normalize_in(&self.infer(ctx, path)?, axioms, inductives);
                let CoreTerm::PathTy {
                    carrier,
                    lhs: start,
                    rhs: end,
                } = path_ty
                else {
                    return Err(KernelError::TypeMismatch {
                        expected: CoreType::Path,
                        actual: shape_of(&path_ty),
                    });
                };
                let motive_ty = normalize_in(&self.infer(ctx, motive)?, axioms, inductives);
                let CoreTerm::Pi {
                    domain, codomain, ..
                } = motive_ty
                else {
                    return Err(KernelError::NotAFunction(shape_of(&motive_ty)));
                };
                require_conversion(&domain, &carrier, axioms, inductives)?;
                let level = universe_level(&normalize_in(&codomain, axioms, inductives))?;
                let left_fiber = CoreTerm::App(motive.clone(), start);
                let right_fiber = CoreTerm::App(motive.clone(), end);
                require_conversion(&self.infer(ctx, lhs)?, &left_fiber, axioms, inductives)?;
                require_conversion(&self.infer(ctx, rhs)?, &right_fiber, axioms, inductives)?;
                Ok(CoreTerm::Universe(level))
            }
            // HComp: `hcomp φ walls base` produces the i1-face of the
            // composition cube whose base is `base` (its i0-face) and
            // sides are `walls` (the family indexed by φ). The result
            // inhabits the same type as `base` — composition does not
            // change the carrier.
            //

            // Checks performed:
            //  * `phi` is well-typed — conservative, no interval
            //  subsumption yet; full cofibration-calculus lands with
            //  the dedicated cubical-kernel pass (task #89-adjacent).
            //  * `walls` is well-typed as some family.
            //  * `base` is well-typed; its type is returned.
            //

            // Rejected shapes: ill-typed subterms surface the underlying
            // `KernelError` rather than being swallowed, so a spurious
            // composition cannot sneak into the TCB.
            CoreTerm::HComp { phi, walls, base } => {
                let _ = self.infer(ctx, phi)?;
                let _ = self.infer(ctx, walls)?;
                self.infer(ctx, base)
            }

            // Transp: `transp(p, r, t)` where `p : Path<Type>(A, B)`,
            // `r : I` (regularity endpoint), `t : A` — result inhabits
            // `B`, the path's right-hand endpoint.
            //

            // Checks performed:
            //  * `path` is well-typed and its type is `PathTy { lhs, rhs }`
            //  (not just some arbitrary term).
            //  * `regular` is well-typed (interval-subsumption deferred).
            //  * `value` is well-typed; result type is the path's `rhs`.
            //

            // On a non-PathTy path type (e.g. a neutral whose head is
            // still an unsolved type-meta), we conservatively fall back
            // to the `value`'s own type — the alternative would be
            // rejecting every proof-in-progress transp, which blocks
            // bring-up. The full cubical pass will tighten this to a
            // hard error.
            CoreTerm::Transp {
                path,
                regular,
                value,
            } => {
                let path_ty = normalize_in(&self.infer(ctx, path)?, axioms, inductives);
                let _ = self.infer(ctx, regular)?;
                match path_ty {
                    CoreTerm::PathTy { carrier, lhs, rhs } => {
                        universe_level(&normalize_in(&carrier, axioms, inductives))?;
                        universe_level(&normalize_in(&self.infer(ctx, &lhs)?, axioms, inductives))?;
                        universe_level(&normalize_in(&self.infer(ctx, &rhs)?, axioms, inductives))?;
                        require_conversion(&self.infer(ctx, value)?, &lhs, axioms, inductives)?;
                        Ok(*rhs)
                    }
                    other => Err(KernelError::TypeMismatch {
                        expected: CoreType::Path,
                        actual: shape_of(&other),
                    }),
                }
            }

            // Glue: `Glue<A>(φ, T, e) : Type_n` where A is the carrier
            // type in `Type_n`, φ is the face formula, T is the partial
            // type family on φ, and e is the equivalence family between
            // T and A on φ.
            //

            // Checks performed:
            //  * `carrier` is in a universe; its level determines the
            //  Glue type's universe.
            //  * `phi`, `fiber`, `equiv` are each well-typed under the
            //  current context.
            //

            // Full univalence computation (Glue-beta, φ-equiv coherence,
            // unglue) lands in the cubical-kernel follow-up — at this
            // phase the kernel certifies that the Glue constructor was
            // assembled from well-typed pieces and is a type at the
            // right universe level.
            CoreTerm::Glue {
                carrier,
                phi,
                fiber,
                equiv,
            } => {
                let carrier_univ = self.infer(ctx, carrier)?;
                let carrier_level = universe_level(&carrier_univ)?;
                let _ = self.infer(ctx, phi)?;
                let _ = self.infer(ctx, fiber)?;
                let _ = self.infer(ctx, equiv)?;
                Ok(CoreTerm::Universe(carrier_level))
            }

            // Refine: {x : base | predicate}. base must inhabit a universe,
            // predicate must check under the extended ctx (bound to Bool at
            // full-rule closure; shape-level at bring-up).
            //

            // K-Refine (/ §4.4 / Diakrisis T-2f*): the predicate's
            // M-iteration depth MUST be strictly less than base's depth + 1.
            // Per Yanofsky 2003 this closes every self-referential paradox
            // schema in a cartesian-closed setting by blocking the exact
            // equality `dp(α) = dp(T^α)` that Russell/Curry/Gödel-type
            // diagonals require. Enforced BEFORE well-typedness inference
            // of the predicate so a depth-violating term is rejected early
            // with a precise diagnostic.
            CoreTerm::Refine {
                base,
                binder,
                predicate,
            } => {
                let base_univ = self.infer(ctx, base)?;
                let base_level = universe_level(&base_univ)?;

                // K-Refine depth check — the single load-bearing Diakrisis
                // rule in the Verum kernel.
                let base_depth = m_depth(base);
                let pred_depth = m_depth(predicate);
                if pred_depth > base_depth {
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
                let _ = self.infer(&extended, predicate)?;
                Ok(CoreTerm::Universe(base_level))
            }

            CoreTerm::Quotient { base, equiv } => self.infer_quotient_type(ctx, base, equiv),

            CoreTerm::QuotIntro { value, base, equiv } => {
                self.infer_quotient_type(ctx, base, equiv)?;
                let actual = self.infer(ctx, value)?;
                require_conversion(&actual, base, axioms, inductives)?;
                Ok(CoreTerm::Quotient {
                    base: base.clone(),
                    equiv: equiv.clone(),
                })
            }

            CoreTerm::QuotElim {
                scrutinee,
                motive,
                case,
                respect,
            } => {
                let quotient = normalize_in(&self.infer(ctx, scrutinee)?, axioms, inductives);
                let CoreTerm::Quotient { base, equiv } = &quotient else {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "quotient recursor requires a quotient scrutinee",
                    )));
                };
                self.infer_quotient_type(ctx, base, equiv)?;
                let motive_ty = normalize_in(&self.infer(ctx, motive)?, axioms, inductives);
                let CoreTerm::Pi {
                    domain, codomain, ..
                } = motive_ty
                else {
                    return Err(KernelError::NotAFunction(shape_of(&motive_ty)));
                };
                require_conversion(&domain, &quotient, axioms, inductives)?;
                universe_level(&normalize_in(&codomain, axioms, inductives))?;

                // An arbitrary fresh quotient variable prevents one representative
                // from being mistaken for proof that a dependent family is constant.
                let terms = [
                    scrutinee.as_ref(),
                    motive.as_ref(),
                    case.as_ref(),
                    respect.as_ref(),
                ];
                let q = quotient_fresh_name(ctx, "q", &terms, &[]);
                let result = normalize_in(
                    &CoreTerm::App(motive.clone(), Heap::new(CoreTerm::Var(q.clone()))),
                    axioms,
                    inductives,
                );
                if crate::support::var_occurs_free(&result, q.as_str()) {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "dependent quotient elimination requires checked transport coherence",
                    )));
                }
                universe_level(&normalize_in(
                    &self.infer(ctx, &result)?,
                    axioms,
                    inductives,
                ))?;
                let x = quotient_fresh_name(ctx, "x", &terms, &[q]);
                let y = quotient_fresh_name(ctx, "y", &terms, &[x.clone()]);
                let r = quotient_fresh_name(ctx, "r", &terms, &[x.clone(), y.clone()]);
                let x_term = CoreTerm::Var(x.clone());
                let y_term = CoreTerm::Var(y.clone());
                let case_ty = CoreTerm::Pi {
                    binder: x.clone(),
                    domain: base.clone(),
                    codomain: Heap::new(result.clone()),
                };
                require_conversion(&self.infer(ctx, case)?, &case_ty, axioms, inductives)?;
                let relation = CoreTerm::App(
                    Heap::new(CoreTerm::App(equiv.clone(), Heap::new(x_term.clone()))),
                    Heap::new(y_term.clone()),
                );
                let respects = CoreTerm::Pi {
                    binder: x,
                    domain: base.clone(),
                    codomain: Heap::new(CoreTerm::Pi {
                        binder: y,
                        domain: base.clone(),
                        codomain: Heap::new(CoreTerm::Pi {
                            binder: r,
                            domain: Heap::new(relation),
                            codomain: Heap::new(CoreTerm::PathTy {
                                carrier: Heap::new(result),
                                lhs: Heap::new(CoreTerm::App(case.clone(), Heap::new(x_term))),
                                rhs: Heap::new(CoreTerm::App(case.clone(), Heap::new(y_term))),
                            }),
                        }),
                    }),
                };
                // The proof obligation is itself a checked type, not an elaborator
                // assertion or a shape attached to otherwise unrelated evidence.
                universe_level(&normalize_in(
                    &self.infer(ctx, &respects)?,
                    axioms,
                    inductives,
                ))?;
                require_conversion(&self.infer(ctx, respect)?, &respects, axioms, inductives)?;
                Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
            }

            // Declaration identity, parameters and universe are authoritative.
            CoreTerm::Inductive { path, args } => {
                let level = match inductives.and_then(|registry| registry.get(path.as_str())) {
                    Some(declaration) => {
                        self.validate_inductive_parameters(
                            ctx,
                            declaration,
                            args,
                            inductives.unwrap(),
                        )?;
                        declaration.universe.clone()
                    }
                    None => return Err(KernelError::UnknownInductive(path.clone())),
                };
                Ok(CoreTerm::Universe(level))
            }

            // Elimination is justified by the registered declaration and every
            // branch. Checking only motive/scrutinee admits an arbitrary goal.
            CoreTerm::Elim {
                scrutinee,
                motive,
                cases,
            } => {
                let motive_ty = normalize_in(&self.infer(ctx, motive)?, axioms, inductives);
                let CoreTerm::Pi {
                    domain, codomain, ..
                } = motive_ty
                else {
                    return Err(KernelError::NotAFunction(shape_of(&motive_ty)));
                };
                let scrut_ty = self.infer(ctx, scrutinee)?;
                if !convertible_in(&scrut_ty, &domain, axioms, inductives) {
                    return Err(KernelError::TypeMismatch {
                        expected: shape_of(&domain),
                        actual: shape_of(&scrut_ty),
                    });
                }
                universe_level(&normalize_in(&codomain, axioms, inductives))?;
                let head = normalize_in(&scrut_ty, axioms, inductives);
                let CoreTerm::Inductive { path, args } = &head else {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "scrutinee has no registered inductive declaration",
                    )));
                };
                let declaration = inductives
                    .and_then(|registry| registry.get(path.as_str()))
                    .ok_or_else(|| KernelError::UnknownInductive(path.clone()))?;
                if !declaration.path_constructors.is_empty() {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "higher inductive elimination requires checked path coherence",
                    )));
                }
                if args.len() != declaration.params.len()
                    || cases.len() != declaration.constructors.len()
                {
                    return Err(KernelError::InvalidEliminator(Text::from(
                        "parameter and branch counts must match the inductive declaration",
                    )));
                }
                for (case, constructor) in cases.iter().zip(declaration.constructors.iter()) {
                    self.validate_constructor_signature(
                        ctx,
                        declaration,
                        constructor,
                        args,
                        inductives.unwrap(),
                    )?;
                    let expected = crate::inductive::instantiated_constructor_case_type(
                        motive,
                        declaration,
                        constructor,
                        args,
                    );
                    let kind = self.infer(ctx, &expected)?;
                    universe_level(&normalize_in(&kind, axioms, inductives))?;
                    let actual = self.infer(ctx, case)?;
                    if !convertible_in(&actual, &expected, axioms, inductives) {
                        return Err(KernelError::TypeMismatch {
                            expected: shape_of(&expected),
                            actual: shape_of(&actual),
                        });
                    }
                }
                Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
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
                    // K-Coord-Cite gate. Auto-fires
                    // when both the calling theorem and the
                    // referenced axiom have populated coords. If
                    // either is absent, rule passes (pre-V8
                    // behaviour preserved for unannotated code).
                    if let (Some(theorem_coord), Some(axiom_coord)) = (current_coord, &entry.coord)
                    {
                        crate::check_coord_cite(theorem_coord, axiom_coord, name, allow_tier_jump)?;
                    }
                    Ok(entry.ty.clone())
                }
                Maybe::None => Err(KernelError::UnknownInductive(name.clone())),
            },

            // Naturality witness: ε(α) and α(ε) are constructor markers for the
            // articulation/enactment duality. They inherit the type of
            // their argument (ε and α are endo-2-functors at the term
            // level — the M⊣A biadjunction structure shows up only at
            // the 2-cell level). Future work will refine the type to track
            // whether the result lives in the articulation 2-category
            // or the enactment 2-category.
            CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => self.infer(ctx, t),

            // Modal-depth: modal operators inhabit `Prop`. The kernel
            // verifies that the operand is well-typed (regardless of
            // whether it inhabits `Prop` or any other type — modality
            // can be applied to any well-formed term, the resulting
            // proposition is always at the propositional layer).
            CoreTerm::ModalBox(phi) | CoreTerm::ModalDiamond(phi) => {
                let _ = self.infer(ctx, phi)?;
                Ok(CoreTerm::Universe(UniverseLevel::Prop))
            }
            CoreTerm::ModalBigAnd(args) => {
                for a in args.iter() {
                    let _ = self.infer(ctx, a)?;
                }
                Ok(CoreTerm::Universe(UniverseLevel::Prop))
            }

            // cohesive modalities ∫ ⊣ ♭ ⊣ ♯ are
            // **type-level endofunctors**. For a type `A : Type_i`,
            // each of `∫A`, `♭A`, `♯A` is itself a type at the same
            // universe level — the modality records the cohesive
            // modal structure without bumping the universe. The
            // operand must be in some universe (Type_i / Prop); the
            // kernel reports `KernelError::TypeMismatch` otherwise.
            //

            // The triple-adjunction laws ∫ ⊣ ♭ ⊣ ♯ are framework
            // axioms (`schreiber_dcct`); the kernel admits the type
            // formers unconditionally, leaving the algebraic content
            // to the framework system.
            CoreTerm::Shape(t) | CoreTerm::Flat(t) | CoreTerm::Sharp(t) => {
                let operand_ty = self.infer(ctx, t)?;
                match &operand_ty {
                    CoreTerm::Universe(level) => Ok(CoreTerm::Universe(level.clone())),
                    _ => Err(KernelError::TypeMismatch {
                        expected: CoreType::Universe(UniverseLevel::Concrete(0)),
                        actual: shape_of(&operand_ty),
                    }),
                }
            }
        }
    }
}

impl Inference<'_> {
    /// Formation checks relation applications at arbitrary carrier elements.
    /// Its proof universe participates in the quotient universe; no resizing is assumed.
    fn infer_quotient_type(
        &self,
        ctx: &Context,
        base: &CoreTerm,
        equiv: &CoreTerm,
    ) -> Result<CoreTerm, KernelError> {
        let axioms = self.axioms;
        let inductives = self.inductives;
        let base_level =
            universe_level(&normalize_in(&self.infer(ctx, base)?, axioms, inductives))?;
        let x = quotient_fresh_name(ctx, "x", &[base, equiv], &[]);
        let y = quotient_fresh_name(ctx, "y", &[base, equiv], &[x.clone()]);
        let extended = ctx
            .extend(x.clone(), base.clone())
            .extend(y.clone(), base.clone());
        let relation = CoreTerm::App(
            Heap::new(CoreTerm::App(
                Heap::new(equiv.clone()),
                Heap::new(CoreTerm::Var(x)),
            )),
            Heap::new(CoreTerm::Var(y)),
        );
        let relation_level = universe_level(&normalize_in(
            &self.infer(&extended, &relation)?,
            axioms,
            inductives,
        ))?;
        let level = if level_at_most(&relation_level, &base_level) {
            base_level
        } else if level_at_most(&base_level, &relation_level) {
            relation_level
        } else {
            UniverseLevel::Max(Heap::new(base_level), Heap::new(relation_level))
        };
        Ok(CoreTerm::Universe(level))
    }
}

fn quotient_fresh_name(ctx: &Context, stem: &str, terms: &[&CoreTerm], reserved: &[Text]) -> Text {
    for index in 0u64.. {
        let candidate = Text::from(format!("_quot_{stem}_{index}"));
        if ctx.lookup(candidate.as_str()).is_none()
            && !reserved.contains(&candidate)
            && terms
                .iter()
                .all(|term| !crate::support::var_occurs_free(term, candidate.as_str()))
        {
            return candidate;
        }
    }
    unreachable!("a finite context and finite terms cannot occupy all fresh names")
}

fn require_conversion(
    actual: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
) -> Result<(), KernelError> {
    if convertible_in(actual, expected, axioms, inductives) {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: shape_of(expected),
            actual: shape_of(actual),
        })
    }
}

fn normalize_in(
    term: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
) -> CoreTerm {
    match inductives {
        Some(registry) => crate::support::normalize_full(term, axioms, registry),
        None => crate::support::normalize_with_axioms(term, axioms),
    }
}

fn convertible_in(
    a: &CoreTerm,
    b: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: Option<&crate::InductiveRegistry>,
) -> bool {
    a == b
        || crate::equivalence::alpha_equivalent(
            &normalize_in(a, axioms, inductives),
            &normalize_in(b, axioms, inductives),
        )
}

impl Inference<'_> {
    fn validate_constructor_signature(
        &self,
        ctx: &Context,
        declaration: &crate::RegisteredInductive,
        constructor: &crate::ConstructorSig,
        params: &[CoreTerm],
        registry: &crate::InductiveRegistry,
    ) -> Result<crate::ConstructorSig, KernelError> {
        let axioms = self.axioms;
        let instantiated = declaration.instantiate_constructor(constructor, params);
        for (index, (raw, field)) in constructor
            .arg_types
            .iter()
            .zip(instantiated.arg_types.iter())
            .enumerate()
        {
            // Check the declaration before parameter substitution: a caller's type
            // argument is not a new recursive occurrence in the declaration. But a
            // transparent definition may expose a hidden negative occurrence.
            registry.check_field_positivity(
                declaration,
                constructor,
                index,
                &normalize_in(raw, axioms, Some(registry)),
            )?;
            self.validate_inductive_field(ctx, field, declaration, registry)?;
        }
        Ok(instantiated)
    }

    /// Parameter universes are bounded by the declaration's universe. The current
    /// declaration schema has no independent parameter-universe telescope.
    fn validate_inductive_parameters(
        &self,
        ctx: &Context,
        declaration: &crate::RegisteredInductive,
        params: &[CoreTerm],
        registry: &crate::InductiveRegistry,
    ) -> Result<(), KernelError> {
        if params.len() != declaration.params.len() {
            return Err(KernelError::InvalidEliminator(Text::from(
                "inductive parameter count does not match its declaration",
            )));
        }
        for parameter in params {
            self.validate_inductive_field(ctx, parameter, declaration, registry)?;
        }
        Ok(())
    }

    fn validate_inductive_field(
        &self,
        ctx: &Context,
        field: &CoreTerm,
        declaration: &crate::RegisteredInductive,
        registry: &crate::InductiveRegistry,
    ) -> Result<(), KernelError> {
        let axioms = self.axioms;
        let kind = self.infer(ctx, field)?;
        let level = universe_level(&normalize_in(&kind, axioms, Some(registry)))?;
        if level_at_most(&level, &declaration.universe) {
            Ok(())
        } else {
            Err(KernelError::InvalidEliminator(Text::from(
                "constructor field or parameter exceeds the declared universe",
            )))
        }
    }
}

// A conservative order on the existing level syntax. Unresolved constraints
// are refused; no symbolic universe comparison is guessed from a name.
pub(crate) fn level_at_most(left: &UniverseLevel, right: &UniverseLevel) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (UniverseLevel::Prop, _) => true,
        (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => a <= b,
        (UniverseLevel::Max(a, b), _) => level_at_most(a, right) && level_at_most(b, right),
        (_, UniverseLevel::Max(a, b)) => level_at_most(left, a) || level_at_most(left, b),
        (UniverseLevel::Succ(a), UniverseLevel::Succ(b)) => level_at_most(a, b),
        (UniverseLevel::Succ(a), UniverseLevel::Concrete(b)) if *b > 0 => {
            level_at_most(a, &UniverseLevel::Concrete(b - 1))
        }
        (UniverseLevel::Concrete(a), UniverseLevel::Succ(b)) if *a > 0 => {
            level_at_most(&UniverseLevel::Concrete(a - 1), b)
        }
        (UniverseLevel::Concrete(0), UniverseLevel::Succ(_)) => true,
        _ => false,
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

/// Verify that `term` inhabits `expected` under full β-/ι-/δ-aware
/// definitional comparison of the two types (not shape-head). This is
/// the LCF-style verification gate that downstream crates call.
///
/// **M-VVA Sub-2.3 (2026-04-28):** uses `definitional_eq_with_axioms`
/// instead of `structural_eq`. Lifting is monotone — every pair
/// admitted by `structural_eq` is admitted by `definitional_eq` per
/// the SN-fragment uniqueness invariant in support.rs:579-582 — and
/// closes a soundness-completeness gap where downstream crates produced
/// a β-reducible `actual` type that `structural_eq` rejected even
/// though its β-normal form matched `expected`.
pub fn verify_full(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    verify_with_inductives(
        ctx,
        term,
        expected,
        axioms,
        &crate::InductiveRegistry::new(),
    )
}

/// Verify an ordinary inductive proof using its explicit declaration registry.
/// Both the goal's formation and the term's branch obligations are checked.
pub fn verify_with_inductives(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
    inductives: &crate::InductiveRegistry,
) -> Result<(), KernelError> {
    let kind = infer_with_inductives(ctx, expected, axioms, inductives)?;
    universe_level(&crate::support::normalize_full(&kind, axioms, inductives))?;
    let actual = infer_with_inductives(ctx, term, axioms, inductives)?;
    if convertible_in(&actual, expected, axioms, Some(inductives)) {
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
