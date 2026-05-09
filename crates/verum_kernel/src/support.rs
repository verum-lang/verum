//! Supporting kernel operations — shape projection, substitution,
//! structural equality, SMT-certificate replay. Split .
//!

//! These four operations are the kernel's "infrastructure layer":
//! they don't implement a typing rule themselves, but every rule in
//! `infer` / `check` calls one or more of them.

use verum_common::{Heap, List, Text};

use crate::{Context, CoreTerm, CoreType, FrameworkId, KernelError, SmtCertificate};

/// Project the kernel's coarse shape head out of a full type term.
/// Used by error messages and the legacy `check` / `verify` API.
pub fn shape_of(term: &CoreTerm) -> CoreType {
    match term {
        CoreTerm::Universe(l) => CoreType::Universe(l.clone()),
        CoreTerm::Pi { .. } => CoreType::Pi,
        CoreTerm::Sigma { .. } => CoreType::Sigma,
        CoreTerm::PathTy { .. } => CoreType::Path,
        // PathOver shares the Path shape
        // tag for diagnostics; a downstream PathOverCarrier-vs-
        // PathTy-carrier mismatch surfaces through the normal
        // structural-eq path.
        CoreTerm::PathOver { .. } => CoreType::Path,
        CoreTerm::Refine { .. } => CoreType::Refine,
        CoreTerm::Glue { .. } => CoreType::Glue,
        CoreTerm::Inductive { path, .. } => CoreType::Inductive(path.clone()),
        _ => CoreType::Other,
    }
}

/// Generate a fresh name not free in `value`, not free in `body`,
/// and not equal to `blocked_name`.  Used by capture-avoiding
/// substitution to alpha-rename a binder before descending into a
/// body where naive substitution would capture a free variable of
/// `value`.
///
/// The candidate sequence is `base_α0`, `base_α1`, … — bounded by
/// the number of distinct names appearing in `value` and `body`,
/// each itself linear in tree size.  In practice the loop exits in
/// the first iteration because the synthetic-suffix names rarely
/// collide with user-source identifiers.
fn fresh_binder_name(
    base: &str,
    value: &CoreTerm,
    body: &CoreTerm,
    blocked_name: &str,
) -> Text {
    let mut counter: u64 = 0;
    loop {
        let candidate = format!("{}_α{}", base, counter);
        if candidate.as_str() != blocked_name
            && !var_occurs_free(value, candidate.as_str())
            && !var_occurs_free(body, candidate.as_str())
        {
            return Text::from(candidate);
        }
        counter = counter.wrapping_add(1);
    }
}

/// Substitute `value` for `name` under a binder, returning the
/// (possibly-renamed) binder and the rewritten body.  Handles the
/// three Barendregt cases:
///
///   1. binder == name → shadow-stop (body unchanged).
///   2. binder ∈ free_vars(value) → **capture risk**: pick fresh
///      name, alpha-rename body to use it, then substitute into
///      the renamed body.  Both passes route through `substitute`
///      so the recursion stays capture-aware.
///   3. otherwise → simple recursion.
fn subst_binder_body(
    binder: &Text,
    body: &CoreTerm,
    name: &str,
    value: &CoreTerm,
) -> (Text, CoreTerm) {
    if binder.as_str() == name {
        return (binder.clone(), (*body).clone());
    }
    if var_occurs_free(value, binder.as_str()) {
        let fresh = fresh_binder_name(binder.as_str(), value, body, name);
        let renamed_body =
            substitute(body, binder.as_str(), &CoreTerm::Var(fresh.clone()));
        let new_body = substitute(&renamed_body, name, value);
        return (fresh, new_body);
    }
    (binder.clone(), substitute(body, name, value))
}

/// Capture-avoiding substitution: `term[name := value]`.
///
/// Every binder construct (Pi / Lam / Sigma / Refine) routes its
/// body through [`subst_binder_body`], which uniformly handles
/// shadow-stop, capture-avoidance via fresh-naming, and the
/// no-conflict fast path.  Pre-bringup the function used a
/// "shadow-stop only" strategy whose soundness depended on the
/// corpus never producing a binder whose name occurred free in the
/// substituted value — a fragile invariant.  The current
/// implementation is structurally correct for every well-formed
/// `term` and every `value`.
pub fn substitute(term: &CoreTerm, name: &str, value: &CoreTerm) -> CoreTerm {
    // Fast-path (#100, task #44): if `name` doesn't occur free in
    // `term`, the entire substitution reduces to a clone. Walking
    // the term once with an early-exit `var_occurs_free` is cheaper
    // than running the full reconstructing substitute, because:
    //

    //  • `var_occurs_free` returns immediately as soon as it finds
    //  a single free occurrence (linear-in-tree-depth at best,
    //  linear-in-tree-size at worst — same as substitute itself).
    //  • For terms WITHOUT a free occurrence, the second walk
    //  (substitute proper) is replaced by a single shallow Heap
    //  clone, which is O(N) of shared Arc bumps (no deep copy of
    //  the underlying tree).
    //

    // In mount-core typechecking the dominant case is substitute
    // into refinement predicates / dependent types where the bound
    // variable name is used in only a few leaves, so this short-
    // circuits the vast majority of subterm walks.
    if !var_occurs_free(term, name) {
        return term.clone();
    }
    match term {
        CoreTerm::Var(n) if n.as_str() == name => value.clone(),
        CoreTerm::Var(_) => term.clone(),
        CoreTerm::Universe(_) => term.clone(),

        CoreTerm::Pi {
            binder,
            domain,
            codomain,
        } => {
            let new_dom = substitute(domain, name, value);
            let (new_binder, new_codom) =
                subst_binder_body(binder, codomain, name, value);
            CoreTerm::Pi {
                binder: new_binder,
                domain: Heap::new(new_dom),
                codomain: Heap::new(new_codom),
            }
        }

        CoreTerm::Lam {
            binder,
            domain,
            body,
        } => {
            let new_dom = substitute(domain, name, value);
            let (new_binder, new_body) =
                subst_binder_body(binder, body, name, value);
            CoreTerm::Lam {
                binder: new_binder,
                domain: Heap::new(new_dom),
                body: Heap::new(new_body),
            }
        }

        CoreTerm::App(f, a) => CoreTerm::App(
            Heap::new(substitute(f, name, value)),
            Heap::new(substitute(a, name, value)),
        ),

        CoreTerm::Sigma {
            binder,
            fst_ty,
            snd_ty,
        } => {
            let new_fst = substitute(fst_ty, name, value);
            let (new_binder, new_snd) =
                subst_binder_body(binder, snd_ty, name, value);
            CoreTerm::Sigma {
                binder: new_binder,
                fst_ty: Heap::new(new_fst),
                snd_ty: Heap::new(new_snd),
            }
        }

        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(substitute(a, name, value)),
            Heap::new(substitute(b, name, value)),
        ),
        CoreTerm::Fst(p) => CoreTerm::Fst(Heap::new(substitute(p, name, value))),
        CoreTerm::Snd(p) => CoreTerm::Snd(Heap::new(substitute(p, name, value))),

        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(substitute(carrier, name, value)),
            lhs: Heap::new(substitute(lhs, name, value)),
            rhs: Heap::new(substitute(rhs, name, value)),
        },
        CoreTerm::PathOver {
            motive,
            path,
            lhs,
            rhs,
        } => CoreTerm::PathOver {
            motive: Heap::new(substitute(motive, name, value)),
            path: Heap::new(substitute(path, name, value)),
            lhs: Heap::new(substitute(lhs, name, value)),
            rhs: Heap::new(substitute(rhs, name, value)),
        },
        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(substitute(x, name, value))),
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(substitute(phi, name, value)),
            walls: Heap::new(substitute(walls, name, value)),
            base: Heap::new(substitute(base, name, value)),
        },
        CoreTerm::Transp {
            path,
            regular,
            value: v,
        } => CoreTerm::Transp {
            path: Heap::new(substitute(path, name, value)),
            regular: Heap::new(substitute(regular, name, value)),
            value: Heap::new(substitute(v, name, value)),
        },
        CoreTerm::Glue {
            carrier,
            phi,
            fiber,
            equiv,
        } => CoreTerm::Glue {
            carrier: Heap::new(substitute(carrier, name, value)),
            phi: Heap::new(substitute(phi, name, value)),
            fiber: Heap::new(substitute(fiber, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },

        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => {
            let new_base = substitute(base, name, value);
            let (new_binder, new_pred) =
                subst_binder_body(binder, predicate, name, value);
            CoreTerm::Refine {
                base: Heap::new(new_base),
                binder: new_binder,
                predicate: Heap::new(new_pred),
            }
        }

        // quotient types: substitute commutes with
        // the constructor (no binders introduced at this level;
        // any binder lives inside `equiv` / `case` themselves).
        CoreTerm::Quotient { base, equiv } => CoreTerm::Quotient {
            base: Heap::new(substitute(base, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },
        CoreTerm::QuotIntro {
            value: v,
            base,
            equiv,
        } => CoreTerm::QuotIntro {
            value: Heap::new(substitute(v, name, value)),
            base: Heap::new(substitute(base, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },
        CoreTerm::QuotElim {
            scrutinee,
            motive,
            case,
        } => CoreTerm::QuotElim {
            scrutinee: Heap::new(substitute(scrutinee, name, value)),
            motive: Heap::new(substitute(motive, name, value)),
            case: Heap::new(substitute(case, name, value)),
        },

        CoreTerm::Inductive { path, args } => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(substitute(a, name, value));
            }
            CoreTerm::Inductive {
                path: path.clone(),
                args: new_args,
            }
        }

        CoreTerm::Elim {
            scrutinee,
            motive,
            cases,
        } => {
            let mut new_cases = List::new();
            for c in cases.iter() {
                new_cases.push(substitute(c, name, value));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(substitute(scrutinee, name, value)),
                motive: Heap::new(substitute(motive, name, value)),
                cases: new_cases,
            }
        }

        CoreTerm::SmtProof(_) | CoreTerm::Axiom { .. } => term.clone(),

        //  substitute commutes with the duality wrappers.
        CoreTerm::EpsilonOf(t) => CoreTerm::EpsilonOf(Heap::new(substitute(t, name, value))),
        CoreTerm::AlphaOf(t) => CoreTerm::AlphaOf(Heap::new(substitute(t, name, value))),

        // Modal-depth: substitute commutes with the modal operators.
        CoreTerm::ModalBox(phi) => CoreTerm::ModalBox(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalDiamond(phi) => {
            CoreTerm::ModalDiamond(Heap::new(substitute(phi, name, value)))
        }
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(substitute(a, name, value)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
        // cohesive modalities ∫ ⊣ ♭ ⊣ ♯ commute with substitute.
        CoreTerm::Shape(t) => CoreTerm::Shape(Heap::new(substitute(t, name, value))),
        CoreTerm::Flat(t) => CoreTerm::Flat(Heap::new(substitute(t, name, value))),
        CoreTerm::Sharp(t) => CoreTerm::Sharp(Heap::new(substitute(t, name, value))),
    }
}

/// Structural (syntactic) equality of two [`CoreTerm`] values.
///

/// This is the kernel's conversion check at bring-up. Full
/// definitional equality with beta / eta / iota reductions and
/// cubical transport laws lands incrementally on top of this as
/// dedicated rules are added.
///

/// note: this remains the "exact-syntactic-equality"
/// primitive callers can still use when they want byte-identity
/// comparison. The new [`definitional_eq`] is the
/// β-aware companion and is the right default for typing-rule
/// equality checks (PathTy formation, App-elimination domain
/// match, etc.).
pub fn structural_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    a == b
}

/// soft step-limit for [`normalize`]. The kernel's
/// well-typed fragment is strongly normalising (per §4.5
/// metatheory inheriting from CCHM + Prop-subsingleton framework
/// axioms), so the limit is defensive against pathological
/// inputs that bypass the elaborator (e.g., serialised
/// certificates referencing user-supplied terms). At the limit
/// the normalizer returns the partially-reduced term — a sound
/// over-approximation: incomplete reduction means
/// [`definitional_eq`] may report false-negatives, never false-
/// positives.
pub const NORMALIZE_STEP_LIMIT: u32 = 10_000;

/// Context threaded through the unified normaliser
/// [`normalize_core`].  Carries the step budget plus optional
/// axiom and inductive registries that gate δ-reduction (axiom
/// unfolding) and HIT-eliminator β-reduction respectively.
///
/// When both fields are `None` the normaliser delivers plain β
/// + Σ-projection + cubical reductions + QuotElim β + PathOver
/// degenerate-case collapse.  When `axioms` is `Some` it adds
/// δ-reduction; when `inductives` is `Some` it adds HIT-elim β
/// and path-constructor β; both can be active simultaneously.
struct NormaliseCtx<'a> {
    /// Step budget — decremented at every recursive call.  When
    /// it reaches zero the normaliser returns the partially
    /// reduced term unchanged (sound: incomplete reduction yields
    /// false negatives in `definitional_eq`, never false positives).
    budget: u32,
    /// δ-reduction registry.  When `Some`, transparent axioms
    /// (those carrying a body) unfold during normalisation;
    /// opaque postulates remain residual.
    axioms: Option<&'a crate::AxiomRegistry>,
    /// HIT-eliminator registry.  When `Some`, `Elim { scrutinee,
    /// motive, cases }` fires the dependent-eliminator β-rule
    /// against the scrutinee's point/path constructor.
    inductives: Option<&'a crate::InductiveRegistry>,
}

impl<'a> NormaliseCtx<'a> {
    /// Construct a fresh context with the given budget and no
    /// registries.  Equivalent to the plain `normalize` regime.
    fn new(budget: u32) -> Self {
        Self {
            budget,
            axioms: None,
            inductives: None,
        }
    }
}

/// Unified normaliser for [`CoreTerm`] — the single source of
/// truth for the kernel's reduction rules.
///
/// Replaces the previous fan-out into three near-identical
/// `_with_budget` helpers (plain / axiom-aware / inductive-aware)
/// that drifted in which reductions they fired — the
/// axiom-aware and inductive-aware variants silently skipped the
/// cubical reductions that the plain path performed.
///
/// Reductions fired (uniformly, with the gates noted):
///
///   * β at App                   — always
///   * Σ-projection at Fst/Snd    — always
///   * QuotElim β                 — always
///   * PathOver-degenerate        — always (closed-loop collapse)
///   * HComp face-bot / face-top  — always
///   * Transp i1 / Refl / const   — always
///   * Glue face-bot / face-top   — always
///   * δ at Axiom                 — when `ctx.axioms.is_some()`
///   * HIT-elim β at Elim         — when `ctx.inductives.is_some()`
///   * path-constructor β at Elim — when `ctx.inductives.is_some()`
fn normalize_core(term: &CoreTerm, ctx: &mut NormaliseCtx<'_>) -> CoreTerm {
    if ctx.budget == 0 {
        return term.clone();
    }
    ctx.budget -= 1;
    match term {
        CoreTerm::Var(_) | CoreTerm::Universe(_) | CoreTerm::SmtProof(_) => term.clone(),

        // δ-reduction at Axiom — fires only when the axiom
        // registry is supplied and the postulate has an attached
        // body (transparent definitions).  Opaque postulates and
        // unknown names normalise their type defensively.
        CoreTerm::Axiom {
            name,
            ty,
            framework,
        } => {
            if let Some(axs) = ctx.axioms
                && let verum_common::Maybe::Some(entry) = axs.get(name.as_str())
                && let Some(body) = &entry.body
            {
                return normalize_core(body, ctx);
            }
            CoreTerm::Axiom {
                name: name.clone(),
                ty: Heap::new(normalize_core(ty, ctx)),
                framework: framework.clone(),
            }
        }

        // β at App.
        CoreTerm::App(f, arg) => {
            let f_norm = normalize_core(f, ctx);
            match f_norm {
                CoreTerm::Lam { binder, body, .. } => {
                    let arg_norm = normalize_core(arg, ctx);
                    let beta = substitute(&body, binder.as_str(), &arg_norm);
                    normalize_core(&beta, ctx)
                }
                neutral => {
                    let arg_norm = normalize_core(arg, ctx);
                    CoreTerm::App(Heap::new(neutral), Heap::new(arg_norm))
                }
            }
        }

        CoreTerm::Pi {
            binder,
            domain,
            codomain,
        } => CoreTerm::Pi {
            binder: binder.clone(),
            domain: Heap::new(normalize_core(domain, ctx)),
            codomain: Heap::new(normalize_core(codomain, ctx)),
        },
        CoreTerm::Lam {
            binder,
            domain,
            body,
        } => CoreTerm::Lam {
            binder: binder.clone(),
            domain: Heap::new(normalize_core(domain, ctx)),
            body: Heap::new(normalize_core(body, ctx)),
        },
        CoreTerm::Sigma {
            binder,
            fst_ty,
            snd_ty,
        } => CoreTerm::Sigma {
            binder: binder.clone(),
            fst_ty: Heap::new(normalize_core(fst_ty, ctx)),
            snd_ty: Heap::new(normalize_core(snd_ty, ctx)),
        },
        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(normalize_core(a, ctx)),
            Heap::new(normalize_core(b, ctx)),
        ),

        // Σ-projection β at Fst/Snd.
        CoreTerm::Fst(p) => {
            let p_norm = normalize_core(p, ctx);
            match p_norm {
                CoreTerm::Pair(a, _) => normalize_core(&a, ctx),
                neutral => CoreTerm::Fst(Heap::new(neutral)),
            }
        }
        CoreTerm::Snd(p) => {
            let p_norm = normalize_core(p, ctx);
            match p_norm {
                CoreTerm::Pair(_, b) => normalize_core(&b, ctx),
                neutral => CoreTerm::Snd(Heap::new(neutral)),
            }
        }

        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(normalize_core(carrier, ctx)),
            lhs: Heap::new(normalize_core(lhs, ctx)),
            rhs: Heap::new(normalize_core(rhs, ctx)),
        },

        // PathOver — degenerate-case rewrite to homogeneous PathTy
        // when the constructor-path's endpoints coincide.
        CoreTerm::PathOver {
            motive,
            path,
            lhs,
            rhs,
        } => {
            let lhs_n = normalize_core(lhs, ctx);
            let rhs_n = normalize_core(rhs, ctx);
            let path_n = normalize_core(path, ctx);
            let motive_n = normalize_core(motive, ctx);
            if let CoreTerm::PathTy {
                lhs: pl, rhs: pr, ..
            } = &path_n
                && structural_eq(pl, pr)
            {
                let carrier = CoreTerm::App(Heap::new(motive_n.clone()), pl.clone());
                return CoreTerm::PathTy {
                    carrier: Heap::new(normalize_core(&carrier, ctx)),
                    lhs: Heap::new(lhs_n),
                    rhs: Heap::new(rhs_n),
                };
            }
            CoreTerm::PathOver {
                motive: Heap::new(motive_n),
                path: Heap::new(path_n),
                lhs: Heap::new(lhs_n),
                rhs: Heap::new(rhs_n),
            }
        }

        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(normalize_core(x, ctx))),

        // HComp face reductions:
        //   hcomp {⊥} u a ↪ a            (empty face system)
        //   hcomp {⊤} u a ↪ u i1 1=1     (whole face constrained)
        CoreTerm::HComp { phi, walls, base } => {
            let phi_n = normalize_core(phi, ctx);
            let walls_n = normalize_core(walls, ctx);
            let base_n = normalize_core(base, ctx);
            if is_face_bot(&phi_n) {
                return base_n;
            }
            if is_face_top(&phi_n) {
                if let CoreTerm::Lam { binder, body, .. } = &walls_n {
                    let i1_term = CoreTerm::Var(Text::from("i1"));
                    let applied = substitute(body, binder.as_str(), &i1_term);
                    return normalize_core(&applied, ctx);
                }
                return CoreTerm::App(
                    Heap::new(walls_n),
                    Heap::new(CoreTerm::Var(Text::from("i1"))),
                );
            }
            CoreTerm::HComp {
                phi: Heap::new(phi_n),
                walls: Heap::new(walls_n),
                base: Heap::new(base_n),
            }
        }

        // Transp identity reductions:
        //   transp A 1 a ↪ a              (transp-fill)
        //   transp refl _ a ↪ a           (transp-on-refl)
        //   transp (λ_. A) _ a ↪ a        (transp-const, binder unused)
        CoreTerm::Transp {
            path,
            regular,
            value,
        } => {
            let path_n = normalize_core(path, ctx);
            let regular_n = normalize_core(regular, ctx);
            let value_n = normalize_core(value, ctx);
            if is_interval_one(&regular_n) {
                return value_n;
            }
            if let CoreTerm::Refl(_) = &path_n {
                return value_n;
            }
            if let CoreTerm::Lam { binder, body, .. } = &path_n
                && !body_uses_binder(body, binder.as_str())
            {
                return value_n;
            }
            CoreTerm::Transp {
                path: Heap::new(path_n),
                regular: Heap::new(regular_n),
                value: Heap::new(value_n),
            }
        }

        // Glue face reductions:
        //   Glue A {⊥} T e ↪ A     (no face constrained)
        //   Glue A {⊤} T e ↪ T 1=1 (whole face constrained)
        CoreTerm::Glue {
            carrier,
            phi,
            fiber,
            equiv,
        } => {
            let carrier_n = normalize_core(carrier, ctx);
            let phi_n = normalize_core(phi, ctx);
            let fiber_n = normalize_core(fiber, ctx);
            let equiv_n = normalize_core(equiv, ctx);
            if is_face_bot(&phi_n) {
                return carrier_n;
            }
            if is_face_top(&phi_n) {
                if let CoreTerm::Lam { binder, body, .. } = &fiber_n {
                    let unit_witness = CoreTerm::Var(Text::from("1=1"));
                    let applied = substitute(body, binder.as_str(), &unit_witness);
                    return normalize_core(&applied, ctx);
                }
                return fiber_n;
            }
            CoreTerm::Glue {
                carrier: Heap::new(carrier_n),
                phi: Heap::new(phi_n),
                fiber: Heap::new(fiber_n),
                equiv: Heap::new(equiv_n),
            }
        }

        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => CoreTerm::Refine {
            base: Heap::new(normalize_core(base, ctx)),
            binder: binder.clone(),
            predicate: Heap::new(normalize_core(predicate, ctx)),
        },

        CoreTerm::Quotient { base, equiv } => CoreTerm::Quotient {
            base: Heap::new(normalize_core(base, ctx)),
            equiv: Heap::new(normalize_core(equiv, ctx)),
        },
        CoreTerm::QuotIntro { value, base, equiv } => CoreTerm::QuotIntro {
            value: Heap::new(normalize_core(value, ctx)),
            base: Heap::new(normalize_core(base, ctx)),
            equiv: Heap::new(normalize_core(equiv, ctx)),
        },

        // QuotElim β: when scrutinee normalises to QuotIntro,
        // collapse to `case applied to value`.
        CoreTerm::QuotElim {
            scrutinee,
            motive,
            case,
        } => {
            let scrut_norm = normalize_core(scrutinee, ctx);
            match &scrut_norm {
                CoreTerm::QuotIntro { value, .. } => {
                    let case_norm = normalize_core(case, ctx);
                    let v_norm = normalize_core(value, ctx);
                    let app = CoreTerm::App(Heap::new(case_norm), Heap::new(v_norm));
                    normalize_core(&app, ctx)
                }
                _ => CoreTerm::QuotElim {
                    scrutinee: Heap::new(scrut_norm),
                    motive: Heap::new(normalize_core(motive, ctx)),
                    case: Heap::new(normalize_core(case, ctx)),
                },
            }
        }

        CoreTerm::Inductive { path, args } => {
            let mut new_args: List<CoreTerm> = List::new();
            for a in args.iter() {
                new_args.push(normalize_core(a, ctx));
            }
            CoreTerm::Inductive {
                path: path.clone(),
                args: new_args,
            }
        }

        // Elim — HIT-elim β + path-constructor β when the
        // inductive registry is supplied.
        CoreTerm::Elim {
            scrutinee,
            motive,
            cases,
        } => {
            let scrut_norm = normalize_core(scrutinee, ctx);
            if let Some(inds) = ctx.inductives {
                // HIT-elim β: applied-ctor-chain matches a
                // registered point ctor — emit
                //   case_C(a1, [rec(a1)], a2, [rec(a2)], …)
                // with a recursor call inserted after every
                // recursive argument.
                if let Some((ctor_name, ctor_args)) = decompose_app_spine(&scrut_norm)
                    && let Some((parent, ctor_idx)) = inds.lookup_point_ctor(ctor_name.as_str())
                    && ctor_idx < cases.iter().count()
                {
                    let case_body = cases.iter().nth(ctor_idx).unwrap().clone();
                    let ctor_sig = parent.constructors.iter().nth(ctor_idx).unwrap();
                    let parent_name = parent.name.clone();
                    let mut beta = case_body;
                    for (arg_idx, arg_ty) in ctor_sig.arg_types.iter().enumerate() {
                        let arg = ctor_args.get(arg_idx).cloned().unwrap_or_else(|| {
                            CoreTerm::Var(Text::from(
                                format!("<missing-arg-{}>", arg_idx).as_str(),
                            ))
                        });
                        beta = CoreTerm::App(Heap::new(beta), Heap::new(arg.clone()));
                        if matches!(
                            arg_ty,
                            CoreTerm::Inductive { path, .. } if path == &parent_name
                        ) {
                            let rec_call = CoreTerm::Elim {
                                scrutinee: Heap::new(arg),
                                motive: motive.clone(),
                                cases: cases.clone(),
                            };
                            beta = CoreTerm::App(Heap::new(beta), Heap::new(rec_call));
                        }
                    }
                    return normalize_core(&beta, ctx);
                }
                // Path-constructor β: bare Var matches a registered
                // path ctor → return the corresponding case branch.
                if let CoreTerm::Var(scrut_name) = &scrut_norm
                    && let Some((parent, path_idx)) = inds.lookup_path_ctor(scrut_name.as_str())
                {
                    let point_count = parent.constructors.iter().count();
                    let case_idx = point_count + path_idx;
                    if case_idx < cases.iter().count() {
                        let case_body = cases.iter().nth(case_idx).unwrap().clone();
                        return normalize_core(&case_body, ctx);
                    }
                }
            }
            // No β fired — neutral form, descend into children.
            let mut new_cases: List<CoreTerm> = List::new();
            for c in cases.iter() {
                new_cases.push(normalize_core(c, ctx));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(scrut_norm),
                motive: Heap::new(normalize_core(motive, ctx)),
                cases: new_cases,
            }
        }

        CoreTerm::EpsilonOf(t) => CoreTerm::EpsilonOf(Heap::new(normalize_core(t, ctx))),
        CoreTerm::AlphaOf(t) => CoreTerm::AlphaOf(Heap::new(normalize_core(t, ctx))),
        CoreTerm::ModalBox(t) => CoreTerm::ModalBox(Heap::new(normalize_core(t, ctx))),
        CoreTerm::ModalDiamond(t) => CoreTerm::ModalDiamond(Heap::new(normalize_core(t, ctx))),
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args: List<Heap<CoreTerm>> = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(normalize_core(a, ctx)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
        // Cohesive modalities ∫ ⊣ ♭ ⊣ ♯ — triple-adjunction
        // reductions are framework axioms (`schreiber_dcct`),
        // discharged by the framework attestation layer rather
        // than fired by the kernel normaliser.
        CoreTerm::Shape(t) => CoreTerm::Shape(Heap::new(normalize_core(t, ctx))),
        CoreTerm::Flat(t) => CoreTerm::Flat(Heap::new(normalize_core(t, ctx))),
        CoreTerm::Sharp(t) => CoreTerm::Sharp(Heap::new(normalize_core(t, ctx))),
    }
}

/// β + cubical normaliser without δ or HIT-elim β.  Drives the
/// PathTy formation rule and is the canonical normaliser for
/// closed terms whose definitional equality doesn't depend on
/// axiom unfolding or inductive eliminator firing.
///
/// What's normalised:
///
///   * β-redexes at App                — `(λx. body) arg → body[arg/x]`
///   * Σ-projection β at Fst/Snd       — `Fst(a, _) → a` etc.
///   * QuotElim β                      — `quot_elim([t]_~, _, c) → c(t)`
///   * PathOver-degenerate              — closed-loop collapse to PathTy
///   * Cubical reductions               — HComp / Transp / Glue face rules
///
/// Used by [`definitional_eq`] (the main consumer) and by the
/// PathTy formation rule per `verification-architecture.md` §4.4.
pub fn normalize(term: &CoreTerm) -> CoreTerm {
    let mut ctx = NormaliseCtx::new(NORMALIZE_STEP_LIMIT);
    normalize_core(term, &mut ctx)
}

/// Normalise with both axiom δ-reduction and HIT-elim β fired
/// simultaneously.  Equivalent to running normalize, normalize
/// _with_axioms, and normalize_with_inductives in sequence —
/// but in a single pass that interleaves the rules so a
/// δ-unfolded body's HIT-eliminator can fire immediately, and
/// vice versa.
pub fn normalize_full(
    term: &CoreTerm,
    axioms: &crate::AxiomRegistry,
    inductives: &crate::InductiveRegistry,
) -> CoreTerm {
    let mut ctx = NormaliseCtx {
        budget: NORMALIZE_STEP_LIMIT,
        axioms: Some(axioms),
        inductives: Some(inductives),
    };
    normalize_core(term, &mut ctx)
}

// =============================================================================
// Cubical face / interval helpers (#98 hardening)
// =============================================================================
//

// The cubical primitives (`HComp` / `Transp` / `Glue`) are
// parameterised by a face formula `phi` and an interval endpoint
// `regular`. Pre-this-module those were just opaque `CoreTerm`s
// — even when `phi = ⊥` (no face constrained) the kernel
// preserved `HComp` instead of reducing to its base. That broke
// the Kan-fibrancy contract: cubical reductions documented in
// `verum_verification::cubical::canonical_rules` were *named* but
// not actually performed.
//

// Hardening: recognise the canonical face / interval marker terms
// at the kernel level and wire each catalogue rule into
// `normalize_core` below. The marker convention is the same
// the cubical surface uses (`⊤` / `⊥` / `i0` / `i1` plus their
// ASCII aliases) — this is the *kernel-level* implementation of the
// face-formula convention.
//

// Rule citations match `cubical::canonical_rules` names so the
// catalogue ↔ kernel correspondence is explicit.

/// True iff `term` is the canonical face-top marker (`⊤` / `1` /
/// `top` / `true`). Used by reductions that fire when `phi = ⊤`.
pub fn is_face_top(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::Var(name) => matches!(name.as_str(), "⊤" | "1" | "top" | "true" | "i1"),
        _ => false,
    }
}

/// True iff `term` is the canonical face-bot marker (`⊥` / `0` /
/// `bot` / `false`). Used by reductions that fire when `phi = ⊥`.
pub fn is_face_bot(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::Var(name) => matches!(name.as_str(), "⊥" | "0" | "bot" | "false" | "i0"),
        _ => false,
    }
}

/// True iff `term` is the interval endpoint `i1` (the cubical
/// "everything-known" point). Equivalent to `is_face_top` for the
/// V0 marker convention but exposed separately so the
/// `transp-fill` rule cites the precise cubical constant.
pub fn is_interval_one(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::Var(name) => matches!(name.as_str(), "i1" | "1" | "true"),
        _ => false,
    }
}

/// True iff `binder` occurs free in `body`. Respects shadowing
/// introduced by inner binders (Pi / Lam / Sigma / Refine /
/// PathOver-motive).  Used by the cubical `transp-const` rule:
/// when the path-of-types is a constant lambda (binder unused in
/// body) transport reduces to the identity.
///
/// Routes through [`var_occurs_free`] — both functions implement
/// identical semantics with identical binder discipline.  Before
/// the dedup, this carried a nested copy of the recursion with a
/// `_ => true` conservative fallback; the canonical helper now
/// covers every variant exhaustively, so the precision the cubical
/// `transp-const` rule sees is strictly better (variants previously
/// answering "true" pessimistically now give the precise answer,
/// which means the rule fires more aggressively where it's
/// genuinely warranted — sound, since the precise answer is always
/// at most the conservative one).
#[inline]
fn body_uses_binder(body: &CoreTerm, binder: &str) -> bool {
    var_occurs_free(body, binder)
}


/// ε-invariant token (Diakrisis Actic
/// 12-actic/03-epsilon-invariant.md). The Actic-side dual of
/// the canonical primitive carries an ordinal-valued
/// ε-coordinate distinct from `m_depth_omega`'s
/// `OrdinalDepth`. This enum is the bridge — a tagged union
/// of the canonical ε-token shapes the Actic spec admits.
///

/// Per defect 3: Actic ε-arithmetic is a
/// different ordinal arithmetic from the kernel's
/// Cantor-normal-form `OrdinalDepth`. This type captures the
/// shape; [`convert_eps_to_md_omega`] performs the canonical
/// conversion (Actic ε-coord → kernel `OrdinalDepth`)
/// preserving order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpsInvariant {
    /// `ε_0` — the identity ε-coordinate (Actic neutral).
    Zero,
    /// `ε_finite(n)` — finite cardinal n.
    Finite(u32),
    /// `ε_omega` — first transfinite ε.
    Omega,
    /// `ε_omega_plus(n)` — `ω + n`.
    OmegaPlus(u32),
    /// `ε_omega_n(coeff, offset)` — `ω·coeff + offset`.
    OmegaTimes {
        /// Coefficient `c` in `ω·c + offset`. Must be ≥ 1; `c == 0`
        /// would collapse to a finite ordinal and is ruled out by
        /// constructor-side validation.
        coeff: u32,
        /// Finite tail `offset` in `ω·c + offset`. May be zero.
        offset: u32,
    },
}

/// convert an Actic ε-invariant to the kernel's
/// Cantor-normal-form [`crate::OrdinalDepth`].
///

/// Per item 5 + Diakrisis Actic
/// 12-actic/03-epsilon-invariant.md: the Actic ε-coordinate
/// and the kernel's modal-depth ordinal are *different*
/// ordinal arithmetics (the former is a coordinate in the
/// AC-stratum classifier; the latter measures syntactic
/// modal-depth of refinement predicates). Both factor through
/// Cantor normal form, however, so a canonical
/// order-preserving embedding exists. This function is that
/// embedding.
///

/// Properties (verified by tests):
///  * `convert(Zero) == finite(0)` — identity.
///  * `convert(Finite(n)) == finite(n)` — finite preservation.
///  * `convert(Omega) == omega()` — first-transfinite preservation.
///  * `convert(OmegaPlus(n)) == { omega_coeff: 1, finite_offset: n }`.
///  * `convert(OmegaTimes { coeff, offset }) ==
///  { omega_coeff: coeff, finite_offset: offset }`.
///  * Monotonicity: `eps1 ≤ eps2` (Actic order) implies
///  `convert(eps1).lt_or_eq(&convert(eps2))` (kernel lex).
///

/// The bridge is **canonical** (independent of how the Actic
/// ε-token was constructed) and **lossless** under the V0
/// encoding — every Actic ε that fits Cantor-normal-form
/// below ε_0 maps to a unique [`crate::OrdinalDepth`].
pub fn convert_eps_to_md_omega(eps: &EpsInvariant) -> crate::OrdinalDepth {
    match eps {
        EpsInvariant::Zero => crate::OrdinalDepth::finite(0),
        EpsInvariant::Finite(n) => crate::OrdinalDepth::finite(*n),
        EpsInvariant::Omega => crate::OrdinalDepth::omega(),
        EpsInvariant::OmegaPlus(n) => crate::OrdinalDepth {
            omega_coeff: 1,
            finite_offset: *n,
        },
        EpsInvariant::OmegaTimes { coeff, offset } => crate::OrdinalDepth {
            omega_coeff: *coeff,
            finite_offset: *offset,
        },
    }
}

/// definitional (β-aware) equality on [`CoreTerm`] values.
///

/// Normalises both sides via [`normalize`] and then performs
/// structural comparison. Two terms compare equal under
/// `definitional_eq` iff they have the same β-normal form.
///

/// This is the right equality for typing-rule conversion
/// checks: PathTy endpoint matching, App domain matching, etc.
/// Replacing [`structural_eq`] with this is monotone (only
/// widens the accept set — every pair admitted by structural_eq
/// is admitted by definitional_eq) and sound (the SN-fragment
/// invariant guarantees normal forms are unique up to α).
pub fn definitional_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    // Fast-path: same structural hash → trivially equal. Same
    // optimization as `definitional_eq_with_axioms` (#43); skips
    // both normalize calls when terms are syntactically identical.
    // Dominant case during mount-core typechecking where the same
    // Type / Bool / Maybe<T> appears repeatedly.
    if crate::normalize_cache::StructuralHash::of(a)
        == crate::normalize_cache::StructuralHash::of(b)
    {
        return true;
    }
    let a_norm = normalize(a);
    let b_norm = normalize(b);
    a_norm == b_norm
}

/// δ-reduction-aware normaliser. Unfolds transparent
/// **definitions** (registered with non-None `body` per
/// [`crate::AxiomRegistry::register_definition`]) before
/// β-normalising.
///

/// Behaviour vs [`normalize`]:
///  • Opaque postulates (`body = None`) are LEFT as-is —
///  `Axiom { name: "..." }` references stay neutral. This is
///  correct: a postulate is, by design, not reducible.
///  • Transparent definitions (`body = Some(_)`) are UNFOLDED —
///  `Axiom { name: "Id", ... }` where `Id := λx. x` becomes
///  `λx. x` and continues normalising.
///  • Every other CoreTerm constructor — including the cubical
///  Kan-fibrancy reductions — delegates to [`normalize_core`],
///  which guarantees identical behaviour to [`normalize`] for
///  every variant other than `Axiom`.
///

/// Step limit ([`NORMALIZE_STEP_LIMIT`]) shared with [`normalize`];
/// δ-unfolds count against the same budget as β-reductions.
pub fn normalize_with_axioms(term: &CoreTerm, axioms: &crate::AxiomRegistry) -> CoreTerm {
    let mut ctx = NormaliseCtx {
        budget: NORMALIZE_STEP_LIMIT,
        axioms: Some(axioms),
        inductives: None,
    };
    normalize_core(term, &mut ctx)
}


// =============================================================================
// Inductive-aware normaliser (HIT eliminator β-rule).
// =============================================================================

/// V8.1 (§7.4 V3 β-rule) — normalise a term with β-reduction,
/// the cubical Kan-fibrancy reductions, and the **HIT eliminator
/// β-rule** firing against a supplied inductive registry. All
/// reductions share the canonical [`normalize_core`] driver, so
/// behaviour stays in lock-step with [`normalize`] and
/// [`normalize_with_axioms`] for every CoreTerm variant.
///

/// The eliminator β-rule fires when an `Elim { scrutinee, motive,
/// cases }` term is encountered with a scrutinee of the form
/// `App-chain(Var(C), arg1, ..., argn)` where `C` matches the
/// i-th point ctor of some registered inductive `T` AND
/// `cases.len() == T.constructors.len()`. The rewrite produces
/// `App-chain(c_i, arg1, ..., argm, recursor-calls)` where each
/// *recursive* argument (those whose ctor `arg_types[j]` is
/// `Inductive { path: T, .. }` for the same parent inductive) is
/// followed by a recursor call `Elim(motive, cases)(arg_j)` so the
/// case body has both the raw recursive argument AND its image
/// under the eliminator. This matches the standard dependent-
/// eliminator β-rule (Coq / Lean / Agda all generate this shape
/// automatically).
///

/// **Path constructors** are NOT handled here — their β-rule
/// involves path substitution and is tracked under §7.4 V3.1.
/// When the scrutinee normalises to anything other than a
/// recognised point-ctor application, the term remains in neutral
/// form and only its children are normalised.
pub fn normalize_with_inductives(
    term: &CoreTerm,
    inductives: &crate::InductiveRegistry,
) -> CoreTerm {
    let mut ctx = NormaliseCtx {
        budget: NORMALIZE_STEP_LIMIT,
        axioms: None,
        inductives: Some(inductives),
    };
    normalize_core(term, &mut ctx)
}

/// Walk an `App` spine, returning `(head_var_name, args_in_order)`
/// when the head is a `Var`. Returns `None` for any other shape.
/// The args list is in **applied** order (left-to-right).
fn decompose_app_spine(term: &CoreTerm) -> Option<(Text, Vec<CoreTerm>)> {
    let mut spine: Vec<CoreTerm> = Vec::new();
    let mut head = term;
    loop {
        match head {
            CoreTerm::App(f, a) => {
                spine.push((**a).clone());
                head = f;
            }
            CoreTerm::Var(name) => {
                spine.reverse();
                return Some((name.clone(), spine));
            }
            _ => return None,
        }
    }
}


/// δ-reduction-aware definitional equality.
///

/// Normalises both sides via [`normalize_with_axioms`] (β + δ)
/// and compares structurally. Two terms compare equal iff they
/// have the same βδ-normal form against the supplied axiom
/// registry.
pub fn definitional_eq_with_axioms(
    a: &CoreTerm,
    b: &CoreTerm,
    axioms: &crate::AxiomRegistry,
) -> bool {
    // Fast-path: structural hash equality short-circuits the
    // double-normalize dance for trivially-equal terms (#100, task #43).
    //

    // In practice mount-core typechecking calls definitional_eq with
    // the SAME term on both sides constantly — `Int =? Int`,
    // `Bool =? Bool`, refinement-pred bodies that haven't been
    // β-reduced yet but are structurally identical. Skipping both
    // normalize calls saves O(N) tree walk + substitution per
    // skipped pair; when both inputs are the same Rust `&CoreTerm`
    // this is a near-zero op (the hash result is cached implicitly
    // because `format!` produces the same bytes).
    //

    // Axiom-registry independence: if structural_eq holds, normalize
    // would produce the same result regardless of axioms (δ-reduction
    // is a function of the term and the axiom set; identical terms
    // always δ-reduce identically given the same registry). So we
    // don't need to fold the axiom fingerprint into the fast path.
    if crate::normalize_cache::StructuralHash::of(a)
        == crate::normalize_cache::StructuralHash::of(b)
    {
        return true;
    }
    let a_norm = normalize_with_axioms(a, axioms);
    let b_norm = normalize_with_axioms(b, axioms);
    a_norm == b_norm
}

/// Early-exit free-variable test (#100, task #44).
///

/// Returns `true` iff `name` occurs free in `term`. Walks the
/// term recursively but returns at the first occurrence — much
/// faster than computing the full `free_vars` set when only one
/// answer is needed.
///

/// Used by [`substitute`] as a precondition check to short-circuit
/// the no-op case (`name` doesn't appear → substitute returns a
/// shallow clone instead of recursively reconstructing).
///

/// Binder semantics: `name` is shadowed by `Pi`/`Lam`/`Sigma`
/// binders that bind `name` exactly — sub-trees under those
/// binders are skipped, matching `substitute`'s shadow-stop rule.
pub fn var_occurs_free(term: &CoreTerm, name: &str) -> bool {
    match term {
        CoreTerm::Var(n) => n.as_str() == name,
        CoreTerm::Universe(_) | CoreTerm::SmtProof(_) => false,
        CoreTerm::Pi {
            binder,
            domain,
            codomain,
        } => {
            var_occurs_free(domain, name)
                || (binder.as_str() != name && var_occurs_free(codomain, name))
        }
        CoreTerm::Lam {
            binder,
            domain,
            body,
        } => {
            var_occurs_free(domain, name)
                || (binder.as_str() != name && var_occurs_free(body, name))
        }
        CoreTerm::App(f, a) => var_occurs_free(f, name) || var_occurs_free(a, name),
        CoreTerm::Sigma {
            binder,
            fst_ty,
            snd_ty,
        } => {
            var_occurs_free(fst_ty, name)
                || (binder.as_str() != name && var_occurs_free(snd_ty, name))
        }
        CoreTerm::Pair(a, b) => var_occurs_free(a, name) || var_occurs_free(b, name),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => var_occurs_free(p, name),
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            var_occurs_free(carrier, name)
                || var_occurs_free(lhs, name)
                || var_occurs_free(rhs, name)
        }
        CoreTerm::Refl(x) => var_occurs_free(x, name),
        CoreTerm::PathOver {
            motive,
            path,
            lhs,
            rhs,
        } => {
            var_occurs_free(motive, name)
                || var_occurs_free(path, name)
                || var_occurs_free(lhs, name)
                || var_occurs_free(rhs, name)
        }

        // Cubical primitives: descend through every component with
        // proper early-exit (no top-level binders).
        CoreTerm::HComp { phi, walls, base } => {
            var_occurs_free(phi, name)
                || var_occurs_free(walls, name)
                || var_occurs_free(base, name)
        }
        CoreTerm::Transp {
            path,
            regular,
            value,
        } => {
            var_occurs_free(path, name)
                || var_occurs_free(regular, name)
                || var_occurs_free(value, name)
        }
        CoreTerm::Glue {
            carrier,
            phi,
            fiber,
            equiv,
        } => {
            var_occurs_free(carrier, name)
                || var_occurs_free(phi, name)
                || var_occurs_free(fiber, name)
                || var_occurs_free(equiv, name)
        }

        // Refinement: descend into base; descend into predicate
        // only when the binder doesn't shadow `name`.
        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => {
            var_occurs_free(base, name)
                || (binder.as_str() != name && var_occurs_free(predicate, name))
        }

        // Quotients: no binders at the top level.
        CoreTerm::Quotient { base, equiv } => {
            var_occurs_free(base, name) || var_occurs_free(equiv, name)
        }
        CoreTerm::QuotIntro { value, base, equiv } => {
            var_occurs_free(value, name)
                || var_occurs_free(base, name)
                || var_occurs_free(equiv, name)
        }
        CoreTerm::QuotElim {
            scrutinee,
            motive,
            case,
        } => {
            var_occurs_free(scrutinee, name)
                || var_occurs_free(motive, name)
                || var_occurs_free(case, name)
        }

        // Inductive: qualified path is a global identifier (not a
        // free variable); only generic args need scanning.
        CoreTerm::Inductive { args, .. } => {
            args.iter().any(|a| var_occurs_free(a, name))
        }

        // Elim: scrutinee + motive + every case.
        CoreTerm::Elim {
            scrutinee,
            motive,
            cases,
        } => {
            var_occurs_free(scrutinee, name)
                || var_occurs_free(motive, name)
                || cases.iter().any(|c| var_occurs_free(c, name))
        }

        // Axiom: name is a global identifier; descend into ty
        // defensively (matches free_vars_rec).
        CoreTerm::Axiom { ty, .. } => var_occurs_free(ty, name),

        // Diakrisis morphism + ε-of: descend through wrapped term.
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => var_occurs_free(t, name),

        // Modal operators: descend through the boxed/diamonded term.
        CoreTerm::ModalBox(t) | CoreTerm::ModalDiamond(t) => var_occurs_free(t, name),
        CoreTerm::ModalBigAnd(args) => {
            args.iter().any(|a| var_occurs_free(a, name))
        }

        // Cohesive modalities: descend through inner.
        CoreTerm::Shape(t) | CoreTerm::Flat(t) | CoreTerm::Sharp(t) => {
            var_occurs_free(t, name)
        }
    }
}

/// V8 — collect the **free variable set** of a [`CoreTerm`].
///

/// A variable `Var(name)` is *free* in a term iff no enclosing
/// binder (`Pi`, `Lam`, `Sigma`, `Refine`) introduces a binding
/// for `name`. The walker descends through every sub-term while
/// maintaining a binder-stack; on encountering a `Var`, it checks
/// whether `name` is in the stack — if not, it's free.
///

/// Returned set is a [`std::collections::BTreeSet`] for
/// deterministic iteration (the caller often renders the set
/// into a diagnostic message; sorted output keeps test golden
/// values stable across hash-DOS-randomised builds).
///

/// Used by [`crate::axiom::AxiomRegistry::register_subsingleton`]
/// to enforce the `K-FwAx` closed-proposition route per
/// `verification-architecture.md` §4.4.
pub fn free_vars(term: &CoreTerm) -> std::collections::BTreeSet<Text> {
    let mut out = std::collections::BTreeSet::new();
    let mut bound: Vec<Text> = Vec::new();
    free_vars_rec(term, &mut bound, &mut out);
    out
}

fn free_vars_rec(
    term: &CoreTerm,
    bound: &mut Vec<Text>,
    out: &mut std::collections::BTreeSet<Text>,
) {
    match term {
        CoreTerm::Var(n) => {
            if !bound.iter().any(|b| b == n) {
                out.insert(n.clone());
            }
        }
        CoreTerm::Universe(_) => {}
        CoreTerm::Pi {
            binder,
            domain,
            codomain,
        } => {
            free_vars_rec(domain, bound, out);
            bound.push(binder.clone());
            free_vars_rec(codomain, bound, out);
            bound.pop();
        }
        CoreTerm::Lam {
            binder,
            domain,
            body,
        } => {
            free_vars_rec(domain, bound, out);
            bound.push(binder.clone());
            free_vars_rec(body, bound, out);
            bound.pop();
        }
        CoreTerm::App(f, a) => {
            free_vars_rec(f, bound, out);
            free_vars_rec(a, bound, out);
        }
        CoreTerm::Sigma {
            binder,
            fst_ty,
            snd_ty,
        } => {
            free_vars_rec(fst_ty, bound, out);
            bound.push(binder.clone());
            free_vars_rec(snd_ty, bound, out);
            bound.pop();
        }
        CoreTerm::Pair(a, b) => {
            free_vars_rec(a, bound, out);
            free_vars_rec(b, bound, out);
        }
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => {
            free_vars_rec(p, bound, out);
        }
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            free_vars_rec(carrier, bound, out);
            free_vars_rec(lhs, bound, out);
            free_vars_rec(rhs, bound, out);
        }
        CoreTerm::PathOver {
            motive,
            path,
            lhs,
            rhs,
        } => {
            free_vars_rec(motive, bound, out);
            free_vars_rec(path, bound, out);
            free_vars_rec(lhs, bound, out);
            free_vars_rec(rhs, bound, out);
        }
        CoreTerm::Refl(x) => free_vars_rec(x, bound, out),
        CoreTerm::HComp { phi, walls, base } => {
            free_vars_rec(phi, bound, out);
            free_vars_rec(walls, bound, out);
            free_vars_rec(base, bound, out);
        }
        CoreTerm::Transp {
            path,
            regular,
            value,
        } => {
            free_vars_rec(path, bound, out);
            free_vars_rec(regular, bound, out);
            free_vars_rec(value, bound, out);
        }
        CoreTerm::Glue {
            carrier,
            phi,
            fiber,
            equiv,
        } => {
            free_vars_rec(carrier, bound, out);
            free_vars_rec(phi, bound, out);
            free_vars_rec(fiber, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => {
            free_vars_rec(base, bound, out);
            bound.push(binder.clone());
            free_vars_rec(predicate, bound, out);
            bound.pop();
        }

        // quotient types: no binder at this level.
        CoreTerm::Quotient { base, equiv } => {
            free_vars_rec(base, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::QuotIntro { value, base, equiv } => {
            free_vars_rec(value, bound, out);
            free_vars_rec(base, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::QuotElim {
            scrutinee,
            motive,
            case,
        } => {
            free_vars_rec(scrutinee, bound, out);
            free_vars_rec(motive, bound, out);
            free_vars_rec(case, bound, out);
        }
        CoreTerm::Inductive { args, .. } => {
            // The `path` is a global qualified name (e.g.
            // "core.collections.list.List"); not a free
            // variable, by construction. Generic arguments
            // contain their own free-var trees.
            for a in args.iter() {
                free_vars_rec(a, bound, out);
            }
        }
        CoreTerm::Elim {
            scrutinee,
            motive,
            cases,
        } => {
            free_vars_rec(scrutinee, bound, out);
            free_vars_rec(motive, bound, out);
            for c in cases.iter() {
                free_vars_rec(c, bound, out);
            }
        }
        CoreTerm::SmtProof(_) => {
            // Certificates carry only opaque trace bytes + hash
            // strings — no syntactic variables to collect.
        }
        CoreTerm::Axiom { ty, .. } => {
            // The axiom's name is a global identifier (registry
            // key); not a free variable. Its claimed type is
            // already closed by definition (a registered axiom
            // is a closed term), but we still descend
            // defensively in case the ty CoreTerm carries
            // generic arguments.
            free_vars_rec(ty, bound, out);
        }
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => {
            free_vars_rec(t, bound, out);
        }
        CoreTerm::ModalBox(t) | CoreTerm::ModalDiamond(t) => {
            free_vars_rec(t, bound, out);
        }
        CoreTerm::ModalBigAnd(args) => {
            for a in args.iter() {
                free_vars_rec(a, bound, out);
            }
        }
        // cohesive modalities descend.
        CoreTerm::Shape(t) | CoreTerm::Flat(t) | CoreTerm::Sharp(t) => {
            free_vars_rec(t, bound, out);
        }
    }
}

/// Replay an [`SmtCertificate`] into a [`CoreTerm`] witness.
///

/// This is the routine that puts Z3 / CVC5 / E / Vampire / Alt-Ergo
/// **outside** the TCB: any SMT-produced proof must be independently
/// reconstructed here before the kernel will admit it as a witness.
///

/// # Supported certificate shapes
///

/// The first phase of the replay ships support for **trust-tag
/// certificates** — a minimal shape the SMT layer emits when a goal
/// closes via the standard `Unsat`-means-valid protocol. The
/// certificate's `trace` is a single-byte tag identifying which of
/// three rule families the backend used:
///

/// * `0x01` — **refl**: the obligation was discharged by
///  syntactic reflexivity (`E == E`).
/// * `0x02` — **asserted**: the obligation matched a hypothesis
///  directly.
/// * `0x03` — **smt_unsat**: the backend reported `Unsat` on the
///  negated obligation using a generic theory combination.
///

/// For each recognised tag the replay constructs a `CoreTerm::Axiom`
/// labelled with the backend's name and the rule family. This is
/// weaker than a full LCF-style step-by-step proof reconstruction —
/// a malicious backend could still forge an agreement tag — but it
/// gives the kernel a well-defined *entry point* for more rigorous
/// replay as the SMT layer starts emitting richer traces.
///

/// **Obligation-hash semantics (V8, doc/code reconciliation).**
/// This function checks that `cert.obligation_hash` is non-empty
/// (rejecting with [`KernelError::MissingObligationHash`] on
/// failure) and embeds the hash into the witness's `Axiom` name.
/// It does NOT compare the hash against any caller-supplied
/// expected hash — the pre-V8 doc claim of such a comparison was
/// false. Callers that have an expected hash (e.g., proving a
/// specific goal whose obligation hash was just computed) MUST
/// use [`replay_smt_cert_with_obligation`] instead, which
/// threads the expected hash through and rejects on mismatch via
/// [`KernelError::ObligationHashMismatch`].
///

/// Future phases (one per backend): parse Z3's `(proof …)` tree
/// format, CVC5's `ALETHE` format, reconstruct each rule's witness
/// term compositionally.
pub fn replay_smt_cert(_ctx: &Context, cert: &SmtCertificate) -> Result<CoreTerm, KernelError> {
    // Envelope schema gate — reject future-version certificates
    // rather than silently accepting an unknown shape.
    cert.validate_schema()?;

    // Known backends — the rule table below only applies to these.
    let backend = cert.backend.as_str();
    if !matches!(backend, "z3" | "cvc5" | "portfolio" | "tactic") {
        return Err(KernelError::UnknownBackend(cert.backend.clone()));
    }

    // The trace must be non-empty; the first byte is the rule tag.
    let rule_tag = match cert.trace.iter().next().copied() {
        Some(t) => t,
        None => return Err(KernelError::EmptyCertificate),
    };

    let rule_name = match rule_tag {
        0x01 => "refl",
        0x02 => "asserted",
        0x03 => "smt_unsat",
        other => {
            return Err(KernelError::UnknownRule {
                backend: cert.backend.clone(),
                tag: other,
            });
        }
    };

    // Sanity-check the obligation hash is present.
    if cert.obligation_hash.as_str().is_empty() {
        return Err(KernelError::MissingObligationHash);
    }

    // Construct the witness term. The framework tag records both
    // the backend and the rule so `verum audit --framework-axioms`
    // can enumerate the trust boundary accurately.
    let framework = FrameworkId {
        framework: Text::from(format!("{}:{}", backend, rule_name)),
        citation: cert.obligation_hash.clone(),
    };
    // The axiom's type is Prop — it's a propositional witness. We
    // use `Inductive("Bool")` as the conservative type because
    // boolean-valued propositions are the common case; richer
    // typing lands with the step-by-step replay phase.
    let axiom_ty = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    Ok(CoreTerm::Axiom {
        name: Text::from(format!(
            "smt_cert:{}:{}:{}",
            backend,
            rule_name,
            cert.obligation_hash.as_str()
        )),
        ty: Heap::new(axiom_ty),
        framework,
    })
}

/// V8 — replay an SMT certificate **and** verify its
/// `obligation_hash` matches the supplied `expected_hash`.
///

/// This is the soundness-correct path for any caller that has a
/// concrete goal in hand (e.g., the gradual-verification driver
/// computing the expected obligation hash from the goal AST and
/// then matching certificates against it). It composes the
/// non-comparison primitive [`replay_smt_cert`] with the explicit
/// hash equality check the V0 doc *claimed* but didn't perform.
///

/// Behaviour:
///  1. Hash equality is checked **before** replay so a mismatched
///  certificate doesn't waste backend-table dispatch work.
///  2. On success, the witness term returned by
///  [`replay_smt_cert`] is unchanged — the comparison adds no
///  new failure mode beyond the new
///  [`KernelError::ObligationHashMismatch`] variant.
pub fn replay_smt_cert_with_obligation(
    ctx: &Context,
    cert: &SmtCertificate,
    expected_hash: &str,
) -> Result<CoreTerm, KernelError> {
    if cert.obligation_hash.as_str() != expected_hash {
        return Err(KernelError::ObligationHashMismatch {
            expected: Text::from(expected_hash),
            actual: cert.obligation_hash.clone(),
        });
    }
    replay_smt_cert(ctx, cert)
}

// =============================================================================
// K-Refine — boolean conjunction helpers for refine-of-refine fold.
// =============================================================================

/// Canonical name of the boolean conjunction connective. Kept as a
/// single source of truth so the K-Refine fold (which builds the
/// conjoined predicate) and the recogniser (which detects already-
/// folded shapes for idempotence) agree byte-for-byte.
pub const CONJUNCTION_NAME: &str = "∧";

/// Build the canonical Bool conjunction `p1 ∧ p2` as a CoreTerm.
///

/// Internally the conjunction is encoded as a curried application
/// `App(App(Var("∧"), p1), p2)` rather than a dedicated AST variant,
/// so existing kernel infrastructure (substitute, free_vars,
/// normalize) treats it uniformly with other binary operators.
///

/// **Idempotence**: `make_conjunction(p, p)` does NOT collapse to
/// `p` — that would require knowing the predicate has Bool type,
/// which the kernel re-checker enforces at refinement-formation
/// time. The K-Refine Refine fold relies on this: it calls
/// `make_conjunction` only after both predicates have already
/// passed the V0 K-Refine-Form gate.
pub fn make_conjunction(p1: &CoreTerm, p2: &CoreTerm) -> CoreTerm {
    let conn = CoreTerm::Var(Text::from(CONJUNCTION_NAME));
    CoreTerm::App(
        Heap::new(CoreTerm::App(Heap::new(conn), Heap::new(p1.clone()))),
        Heap::new(p2.clone()),
    )
}

/// Recognise a canonical conjunction shape `p1 ∧ p2` produced by
/// [`make_conjunction`]. Returns `Some((p1, p2))` on match, `None`
/// otherwise. Used by the K-Refine Refine fold to keep iteration
/// idempotent (a fold that produced `Refine(B, p1 ∧ p2)` must NOT
/// re-fold against any further predicate without first recognising
/// the existing conjunction shape).
pub fn is_conjunction(t: &CoreTerm) -> Option<(&CoreTerm, &CoreTerm)> {
    if let CoreTerm::App(outer, p2) = t
        && let CoreTerm::App(conn, p1) = outer.as_ref()
        && let CoreTerm::Var(name) = conn.as_ref()
        && name.as_str() == CONJUNCTION_NAME
    {
        return Some((p1.as_ref(), p2.as_ref()));
    }
    None
}

/// K-Refine Refine fold:
///

///  `Refine(Refine(B, x: p₁), x: p₂)` → `Refine(B, x: p₁ ∧ p₂)`
///

/// When the outer and inner binders differ, the inner predicate is
/// alpha-renamed to use the outer binder so the conjunction is
/// well-scoped (substitute(p₁, inner_binder, Var(outer_binder))).
///

/// Returns `Some(folded)` when the term has the canonical
/// nested-Refine shape; `None` otherwise. Idempotent under
/// composition: applying twice produces the same shape (the second
/// application sees `Refine(B, p₁ ∧ p₂)` which is no longer
/// `Refine(Refine, _)` — fold doesn't trigger).
///

/// Soundness: the fold preserves the refinement semantics —
/// `{ x : { y : B | p₁(y) } | p₂(x) }` ≡ `{ x : B | p₁(x) ∧ p₂(x) }`
/// for every `x : B` in the underlying type theory (predicate-
/// extensionality + BHK conjunction = pair-of-proofs).
pub fn fold_refine_of_refine(term: &CoreTerm) -> Option<CoreTerm> {
    let (outer_base, outer_binder, outer_pred) = match term {
        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => (base.as_ref(), binder, predicate.as_ref()),
        _ => return None,
    };
    let (inner_base, inner_binder, inner_pred) = match outer_base {
        CoreTerm::Refine {
            base,
            binder,
            predicate,
        } => (base.as_ref(), binder, predicate.as_ref()),
        _ => return None,
    };
    // Alpha-rename inner predicate to use the outer binder when the
    // names differ. The substitute helper is name-respecting and
    // skips inside binders that capture the same name, so renaming
    // the inner predicate is a structural rewrite without scope leaks.
    let renamed_inner_pred = if outer_binder == inner_binder {
        inner_pred.clone()
    } else {
        let outer_var = CoreTerm::Var(outer_binder.clone());
        substitute(inner_pred, inner_binder.as_str(), &outer_var)
    };
    let conjoined = make_conjunction(&renamed_inner_pred, outer_pred);
    Some(CoreTerm::Refine {
        base: Heap::new(inner_base.clone()),
        binder: outer_binder.clone(),
        predicate: Heap::new(conjoined),
    })
}

#[cfg(test)]
mod conjunction_tests {
    use super::*;

    fn var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }

    #[test]
    fn make_conjunction_builds_curried_app() {
        let p = var("p");
        let q = var("q");
        let c = make_conjunction(&p, &q);
        match c {
            CoreTerm::App(outer, q_actual) => {
                assert_eq!(q_actual.as_ref(), &q);
                match outer.as_ref() {
                    CoreTerm::App(conn, p_actual) => {
                        assert_eq!(p_actual.as_ref(), &p);
                        assert!(matches!(conn.as_ref(),
                            CoreTerm::Var(name) if name.as_str() == CONJUNCTION_NAME));
                    }
                    _ => panic!("inner is not App"),
                }
            }
            _ => panic!("outer is not App"),
        }
    }

    #[test]
    fn is_conjunction_recognises_make_conjunction() {
        let p = var("p");
        let q = var("q");
        let c = make_conjunction(&p, &q);
        let (p_back, q_back) = is_conjunction(&c).expect("must recognise");
        assert_eq!(p_back, &p);
        assert_eq!(q_back, &q);
    }

    #[test]
    fn is_conjunction_rejects_unrelated_app() {
        let app = CoreTerm::App(Heap::new(var("f")), Heap::new(var("x")));
        assert!(is_conjunction(&app).is_none());
    }

    #[test]
    fn is_conjunction_rejects_var() {
        assert!(is_conjunction(&var("p")).is_none());
    }

    #[test]
    fn fold_refine_of_refine_collapses_same_binder() {
        let b = var("Int");
        let p = var("p");
        let q = var("q");
        let inner = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("x"),
            predicate: Heap::new(p.clone()),
        };
        let outer = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(q.clone()),
        };
        let folded = fold_refine_of_refine(&outer).expect("must fold");
        match folded {
            CoreTerm::Refine {
                base,
                binder,
                predicate,
            } => {
                assert_eq!(base.as_ref(), &b);
                assert_eq!(binder.as_str(), "x");
                let (p_back, q_back) =
                    is_conjunction(predicate.as_ref()).expect("predicate is conjunction");
                assert_eq!(p_back, &p);
                assert_eq!(q_back, &q);
            }
            _ => panic!("not Refine"),
        }
    }

    #[test]
    fn fold_refine_alpha_renames_inner_binder() {
        // Refine(Refine(B, y: p(y)), x: q(x))
        //  → Refine(B, x: p(x) ∧ q(x))
        let b = var("Int");
        let inner = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("y"),
            predicate: Heap::new(CoreTerm::App(Heap::new(var("p")), Heap::new(var("y")))),
        };
        let outer = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(CoreTerm::App(Heap::new(var("q")), Heap::new(var("x")))),
        };
        let folded = fold_refine_of_refine(&outer).expect("must fold");
        match folded {
            CoreTerm::Refine {
                base,
                binder,
                predicate,
            } => {
                assert_eq!(base.as_ref(), &b);
                assert_eq!(binder.as_str(), "x");
                let (p_x, q_x) =
                    is_conjunction(predicate.as_ref()).expect("predicate is conjunction");
                // p(y) → p(x) — y was renamed to x.
                match p_x {
                    CoreTerm::App(head, arg) => {
                        assert_eq!(head.as_ref(), &var("p"));
                        assert_eq!(
                            arg.as_ref(),
                            &var("x"),
                            "inner binder y must be alpha-renamed to x"
                        );
                    }
                    _ => panic!("p_x is not App"),
                }
                match q_x {
                    CoreTerm::App(head, arg) => {
                        assert_eq!(head.as_ref(), &var("q"));
                        assert_eq!(arg.as_ref(), &var("x"));
                    }
                    _ => panic!("q_x is not App"),
                }
            }
            _ => panic!("not Refine"),
        }
    }

    #[test]
    fn fold_refine_returns_none_on_non_nested() {
        // Refine(B, x: p) — single-level refinement, no fold.
        let r = CoreTerm::Refine {
            base: Heap::new(var("Int")),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        assert!(
            fold_refine_of_refine(&r).is_none(),
            "single Refine must not fold"
        );
    }

    #[test]
    fn fold_refine_returns_none_on_non_refine() {
        assert!(fold_refine_of_refine(&var("foo")).is_none());
    }

    #[test]
    fn fold_refine_idempotent_under_repeated_application() {
        // After folding once, the result is Refine(B, p ∧ q) — a
        // single Refine, so a second fold returns None.
        let b = var("Int");
        let inner = CoreTerm::Refine {
            base: Heap::new(b),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        let outer = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(var("q")),
        };
        let folded_once = fold_refine_of_refine(&outer).expect("first fold");
        let folded_twice = fold_refine_of_refine(&folded_once);
        assert!(
            folded_twice.is_none(),
            "second fold must be a no-op (idempotence)"
        );
    }

    #[test]
    fn fold_refine_three_level_collapses_recursively_when_called_twice() {
        // Refine(Refine(Refine(B, x: p), x: q), x: r)
        //  first fold: Refine(Refine(B, x: p ∧ q), x: r) -- WAIT
        // Actually the fold is OUTERMOST-only: it folds the top two
        // levels. To fully collapse a 3-level stack the caller must
        // iterate. We pin the OUTERMOST-once contract here.
        let b = var("Int");
        let l1 = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        let l2 = CoreTerm::Refine {
            base: Heap::new(l1),
            binder: Text::from("x"),
            predicate: Heap::new(var("q")),
        };
        let l3 = CoreTerm::Refine {
            base: Heap::new(l2),
            binder: Text::from("x"),
            predicate: Heap::new(var("r")),
        };
        // First fold collapses outermost two levels:
        // Refine(Refine(B, x: p), x: q ∧ r)
        let folded_outer = fold_refine_of_refine(&l3).expect("outer fold");
        // The base of the outermost Refine should now BE a Refine
        // (the inner that wasn't yet collapsed).
        match &folded_outer {
            CoreTerm::Refine {
                base, predicate, ..
            } => {
                assert!(
                    matches!(base.as_ref(), CoreTerm::Refine { .. }),
                    "after one fold, base must still be a Refine for 3-level case"
                );
                let (p_q, r) =
                    is_conjunction(predicate.as_ref()).expect("outer predicate is q ∧ r");
                // After alpha-rename, p_q should be q (NOT q ∧ r) and r is r.
                assert_eq!(p_q, &var("q"), "outer fold's lhs is q (renamed inner)");
                assert_eq!(r, &var("r"));
            }
            _ => panic!("not Refine"),
        }
    }
}

// ============================================================================
// substitute() — capture-avoidance pin tests
//
// The Barendregt-convention bringup completed: substitute now picks
// fresh names for binders whose declared name occurs free in the
// substituted value.  These tests pin the three classes:
//
//   1. Shadow-stop:    binder == name → body unchanged.
//   2. Capture-avoid:  binder ∈ free_vars(value) → fresh-rename.
//   3. Simple recurse: no conflict → straight recursion.
//
// Plus a regression test: the value's free variables must STILL be
// free after substitution (no silent capture).
// ============================================================================

#[cfg(test)]
mod substitute_capture_tests {
    use super::*;

    fn var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }

    fn pi(binder: &str, dom: CoreTerm, codom: CoreTerm) -> CoreTerm {
        CoreTerm::Pi {
            binder: Text::from(binder),
            domain: Heap::new(dom),
            codomain: Heap::new(codom),
        }
    }

    fn lam(binder: &str, dom: CoreTerm, body: CoreTerm) -> CoreTerm {
        CoreTerm::Lam {
            binder: Text::from(binder),
            domain: Heap::new(dom),
            body: Heap::new(body),
        }
    }

    fn sigma(binder: &str, fst: CoreTerm, snd: CoreTerm) -> CoreTerm {
        CoreTerm::Sigma {
            binder: Text::from(binder),
            fst_ty: Heap::new(fst),
            snd_ty: Heap::new(snd),
        }
    }

    fn refine(base: CoreTerm, binder: &str, pred: CoreTerm) -> CoreTerm {
        CoreTerm::Refine {
            base: Heap::new(base),
            binder: Text::from(binder),
            predicate: Heap::new(pred),
        }
    }

    // ---- Shadow-stop ----

    #[test]
    fn shadow_stop_pi_when_binder_equals_name() {
        // substitute(Π(x: A, x → x), "x", v) — x is shadowed by Π's
        // binder; codomain stays untouched, only the domain is
        // affected (binder doesn't shadow there).
        let term = pi("x", var("A"), CoreTerm::App(Heap::new(var("x")), Heap::new(var("x"))));
        let v = var("REPLACEMENT");
        let result = substitute(&term, "x", &v);
        match result {
            CoreTerm::Pi { binder, domain, codomain } => {
                assert_eq!(binder.as_str(), "x", "binder unchanged");
                // domain has no `x` to shadow — but no `x` to substitute either.
                // (Domain holds A, so substitution doesn't fire.)
                assert_eq!(*domain, var("A"));
                // codomain unchanged due to shadow-stop.
                assert_eq!(
                    *codomain,
                    CoreTerm::App(Heap::new(var("x")), Heap::new(var("x"))),
                    "shadow-stop preserves body verbatim",
                );
            }
            _ => panic!("expected Pi, got {:?}", result),
        }
    }

    // ---- Capture-avoidance ----

    #[test]
    fn capture_avoidance_pi_renames_binder_when_value_uses_it() {
        // substitute(Π(x: A, _ : x), name="A", value=Var("x"))
        //   Naive: Π(x: x, _ : x)  // captured!
        //   Correct: Π(x_α0: x, _ : x_α0)
        let term = pi("x", var("A"), var("x"));
        let v = var("x");
        let result = substitute(&term, "A", &v);
        match result {
            CoreTerm::Pi { binder, domain, codomain } => {
                assert_ne!(
                    binder.as_str(),
                    "x",
                    "binder MUST be renamed — naive substitution would capture",
                );
                // Domain becomes the substituted value: Var("x").
                assert_eq!(*domain, var("x"), "domain holds substituted value");
                // Codomain references the FRESH binder, not the
                // outer x (which would have been captured).
                assert_eq!(
                    *codomain,
                    CoreTerm::Var(binder.clone()),
                    "codomain references the fresh binder, not the outer x",
                );
            }
            _ => panic!("expected Pi, got {:?}", result),
        }
    }

    #[test]
    fn capture_avoidance_lam_renames_binder() {
        // substitute(λ(x: A. body), name="A", value=Var("x"))
        // where body = Var("x") — the body's `x` is the bound λ-var
        // (free reference would be different).
        // Build a body that references the OUTER A and the inner x:
        // body = App(Var("A"), Var("x"))
        let term = lam("x", var("A"), CoreTerm::App(Heap::new(var("A")), Heap::new(var("x"))));
        let v = var("x");
        let result = substitute(&term, "A", &v);
        match result {
            CoreTerm::Lam { binder, domain, body } => {
                assert_ne!(
                    binder.as_str(),
                    "x",
                    "binder must be renamed to avoid capturing the value's free x",
                );
                assert_eq!(*domain, var("x"));
                // body should be App(<value=x>, Var(<fresh>))
                match &*body {
                    CoreTerm::App(f, a) => {
                        assert_eq!(**f, var("x"), "App's f became the value");
                        assert_eq!(
                            **a,
                            CoreTerm::Var(binder.clone()),
                            "App's a refers to fresh binder",
                        );
                    }
                    _ => panic!("expected App body, got {:?}", body),
                }
            }
            _ => panic!("expected Lam, got {:?}", result),
        }
    }

    #[test]
    fn capture_avoidance_sigma_renames_binder() {
        // substitute(Σ(y: A, y → A), "A", Var("y"))
        //   Naive: Σ(y: y, y → y)  // first y captured!
        //   Correct: Σ(y_α0: y, y_α0 → y)
        let term = sigma(
            "y",
            var("A"),
            CoreTerm::App(Heap::new(var("y")), Heap::new(var("A"))),
        );
        let v = var("y");
        let result = substitute(&term, "A", &v);
        match result {
            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                assert_ne!(binder.as_str(), "y");
                assert_eq!(*fst_ty, var("y"), "first component is the value");
                match &*snd_ty {
                    CoreTerm::App(f, a) => {
                        assert_eq!(
                            **f,
                            CoreTerm::Var(binder.clone()),
                            "snd's first arg is the fresh binder",
                        );
                        assert_eq!(**a, var("y"), "snd's second arg is value's free y");
                    }
                    _ => panic!("expected App in snd_ty"),
                }
            }
            _ => panic!("expected Sigma"),
        }
    }

    #[test]
    fn capture_avoidance_refine_renames_binder() {
        // substitute(Refine(A, x: <App x A>), "A", Var("x"))
        //   Naive: Refine(x, x: <App x x>) — outer x captured
        //   Correct: Refine(x, x_α0: <App x_α0 x>)
        let term = refine(
            var("A"),
            "x",
            CoreTerm::App(Heap::new(var("x")), Heap::new(var("A"))),
        );
        let v = var("x");
        let result = substitute(&term, "A", &v);
        match result {
            CoreTerm::Refine { base, binder, predicate } => {
                assert_eq!(*base, var("x"));
                assert_ne!(binder.as_str(), "x");
                match &*predicate {
                    CoreTerm::App(f, a) => {
                        assert_eq!(**f, CoreTerm::Var(binder.clone()));
                        assert_eq!(**a, var("x"));
                    }
                    _ => panic!("expected App in predicate"),
                }
            }
            _ => panic!("expected Refine"),
        }
    }

    // ---- No-conflict simple-recurse ----

    #[test]
    fn no_conflict_simple_recurse_preserves_binder() {
        // substitute(Π(x: A, x → A), "A", v) where x is NOT free
        //   in v.  Binder unchanged; A → v in both domain and
        //   codomain.
        let term = pi("x", var("A"), CoreTerm::App(Heap::new(var("x")), Heap::new(var("A"))));
        let v = var("y"); // y, not x — no capture risk
        let result = substitute(&term, "A", &v);
        match result {
            CoreTerm::Pi { binder, domain, codomain } => {
                assert_eq!(binder.as_str(), "x", "binder unchanged in no-conflict case");
                assert_eq!(*domain, var("y"));
                match &*codomain {
                    CoreTerm::App(f, a) => {
                        assert_eq!(**f, var("x"));
                        assert_eq!(**a, var("y"));
                    }
                    _ => panic!("expected App"),
                }
            }
            _ => panic!("expected Pi"),
        }
    }

    // ---- Free-variable invariant ----

    #[test]
    fn substituted_value_free_vars_remain_free() {
        // The load-bearing soundness invariant: every name free in
        // `value` must STILL be free in `substitute(term, name, value)`.
        // Naive substitution can violate this through capture; the
        // capture-avoiding substitute preserves it for every binder
        // shape we cover above.
        let cases: Vec<(CoreTerm, &str, CoreTerm, &[&str])> = vec![
            // Pi case — `x` free in value, captured by Π's binder.
            (pi("x", var("A"), var("x")), "A", var("x"), &["x"]),
            // Lam case.
            (
                lam("z", var("A"), CoreTerm::App(Heap::new(var("A")), Heap::new(var("z")))),
                "A",
                var("z"),
                &["z"],
            ),
            // Sigma case.
            (
                sigma(
                    "y",
                    var("A"),
                    CoreTerm::App(Heap::new(var("y")), Heap::new(var("A"))),
                ),
                "A",
                var("y"),
                &["y"],
            ),
            // Refine case.
            (
                refine(
                    var("A"),
                    "x",
                    CoreTerm::App(Heap::new(var("x")), Heap::new(var("A"))),
                ),
                "A",
                var("x"),
                &["x"],
            ),
        ];
        for (term, name, v, must_remain_free) in cases {
            let result = substitute(&term, name, &v);
            for n in must_remain_free {
                assert!(
                    var_occurs_free(&result, n),
                    "{} must remain free after substitute(_, {}, {:?}); got {:?}",
                    n,
                    name,
                    v,
                    result,
                );
            }
        }
    }
}
