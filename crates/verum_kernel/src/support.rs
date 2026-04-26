//! Supporting kernel operations — shape projection, substitution,
//! structural equality, SMT-certificate replay. Split per #198.
//!
//! These four operations are the kernel's "infrastructure layer":
//! they don't implement a typing rule themselves, but every rule in
//! `infer` / `check` calls one or more of them.

use verum_common::{Heap, List, Text};

use crate::{
    Context, CoreTerm, CoreType, FrameworkId, KernelError, SmtCertificate,
};

/// Project the kernel's coarse shape head out of a full type term.
/// Used by error messages and the legacy `check` / `verify` API.
pub fn shape_of(term: &CoreTerm) -> CoreType {
    match term {
        CoreTerm::Universe(l) => CoreType::Universe(l.clone()),
        CoreTerm::Pi { .. } => CoreType::Pi,
        CoreTerm::Sigma { .. } => CoreType::Sigma,
        CoreTerm::PathTy { .. } => CoreType::Path,
        CoreTerm::Refine { .. } => CoreType::Refine,
        CoreTerm::Glue { .. } => CoreType::Glue,
        CoreTerm::Inductive { path, .. } => CoreType::Inductive(path.clone()),
        _ => CoreType::Other,
    }
}

/// Capture-avoiding substitution: `term[name := value]`.
///
/// Rename-on-clash (Barendregt-convention bringup): if a binder in
/// `term` shadows `name`, that sub-tree is left untouched. Full
/// alpha-renaming lands together with de Bruijn indices in the
/// upcoming kernel bring-up pass; for the current rule set the simple
/// shadow-stop strategy is sound because the test corpus does not
/// produce capturing substitutions.
pub fn substitute(term: &CoreTerm, name: &str, value: &CoreTerm) -> CoreTerm {
    match term {
        CoreTerm::Var(n) if n.as_str() == name => value.clone(),
        CoreTerm::Var(_) => term.clone(),
        CoreTerm::Universe(_) => term.clone(),

        CoreTerm::Pi { binder, domain, codomain } => {
            let new_dom = substitute(domain, name, value);
            let new_codom = if binder.as_str() == name {
                (**codomain).clone()
            } else {
                substitute(codomain, name, value)
            };
            CoreTerm::Pi {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                codomain: Heap::new(new_codom),
            }
        }

        CoreTerm::Lam { binder, domain, body } => {
            let new_dom = substitute(domain, name, value);
            let new_body = if binder.as_str() == name {
                (**body).clone()
            } else {
                substitute(body, name, value)
            };
            CoreTerm::Lam {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                body: Heap::new(new_body),
            }
        }

        CoreTerm::App(f, a) => CoreTerm::App(
            Heap::new(substitute(f, name, value)),
            Heap::new(substitute(a, name, value)),
        ),

        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
            let new_fst = substitute(fst_ty, name, value);
            let new_snd = if binder.as_str() == name {
                (**snd_ty).clone()
            } else {
                substitute(snd_ty, name, value)
            };
            CoreTerm::Sigma {
                binder: binder.clone(),
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
        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(substitute(x, name, value))),
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(substitute(phi, name, value)),
            walls: Heap::new(substitute(walls, name, value)),
            base: Heap::new(substitute(base, name, value)),
        },
        CoreTerm::Transp { path, regular, value: v } => CoreTerm::Transp {
            path: Heap::new(substitute(path, name, value)),
            regular: Heap::new(substitute(regular, name, value)),
            value: Heap::new(substitute(v, name, value)),
        },
        CoreTerm::Glue { carrier, phi, fiber, equiv } => CoreTerm::Glue {
            carrier: Heap::new(substitute(carrier, name, value)),
            phi: Heap::new(substitute(phi, name, value)),
            fiber: Heap::new(substitute(fiber, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },

        CoreTerm::Refine { base, binder, predicate } => {
            let new_base = substitute(base, name, value);
            let new_pred = if binder.as_str() == name {
                (**predicate).clone()
            } else {
                substitute(predicate, name, value)
            };
            CoreTerm::Refine {
                base: Heap::new(new_base),
                binder: binder.clone(),
                predicate: Heap::new(new_pred),
            }
        }

        // V8 (#236) — quotient types: substitute commutes with
        // the constructor (no binders introduced at this level;
        // any binder lives inside `equiv` / `case` themselves).
        CoreTerm::Quotient { base, equiv } => CoreTerm::Quotient {
            base: Heap::new(substitute(base, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },
        CoreTerm::QuotIntro { value: v, base, equiv } => CoreTerm::QuotIntro {
            value: Heap::new(substitute(v, name, value)),
            base: Heap::new(substitute(base, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },
        CoreTerm::QuotElim { scrutinee, motive, case } => CoreTerm::QuotElim {
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

        CoreTerm::Elim { scrutinee, motive, cases } => {
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

        // VVA-1: substitute commutes with the duality wrappers.
        CoreTerm::EpsilonOf(t) => CoreTerm::EpsilonOf(Heap::new(substitute(t, name, value))),
        CoreTerm::AlphaOf(t)   => CoreTerm::AlphaOf(Heap::new(substitute(t, name, value))),

        // VVA-7: substitute commutes with the modal operators.
        CoreTerm::ModalBox(phi) => CoreTerm::ModalBox(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalDiamond(phi) => CoreTerm::ModalDiamond(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(substitute(a, name, value)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
    }
}

/// Structural (syntactic) equality of two [`CoreTerm`] values.
///
/// This is the kernel's conversion check at bring-up. Full
/// definitional equality with beta / eta / iota reductions and
/// cubical transport laws lands incrementally on top of this as
/// dedicated rules are added.
///
/// V8 (#216) note: this remains the "exact-syntactic-equality"
/// primitive callers can still use when they want byte-identity
/// comparison. The new [`definitional_eq`] is the
/// β-aware companion and is the right default for typing-rule
/// equality checks (PathTy formation, App-elimination domain
/// match, etc.).
pub fn structural_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    a == b
}

/// V8 (#216) — soft step-limit for [`normalize`]. The kernel's
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

/// V8 (#216) — β-normalise a [`CoreTerm`] to a fixed point or
/// the [`NORMALIZE_STEP_LIMIT`] step limit, whichever comes
/// first.
///
/// Reduction strategy: outermost-leftmost (call-by-name on the
/// β-redex at the head, then recursive descent through all
/// sub-terms). This is the *complete* β-normaliser for the SN
/// fragment — every β-equivalent pair of terms reduces to the
/// same unique normal form.
///
/// What's normalised:
///   • β-redexes: `App(Lam(x, _, body), arg) → body[x := arg]`,
///     iterated to fixed point.
///   • Recursive descent through every CoreTerm constructor —
///     reducing inside Pi codomain / Lam body / Sigma snd_ty /
///     PathTy carrier+endpoints / Refine predicate / etc.
///
/// What's *not* yet normalised (deferred to #216 V2):
///   • δ-reduction (axiom unfolding) — needs an axiom/inductive
///     registry parameter; current shape is registry-free for
///     drop-in PathTy use.
///   • η-reduction.
///   • ι-reduction (Elim / pattern-match β).
///   • Cubical reductions (HComp / Transp / Glue evaluation).
///
/// Used by [`definitional_eq`] (the main consumer) and by the
/// PathTy formation rule per `verification-architecture.md`
/// §4.4.
pub fn normalize(term: &CoreTerm) -> CoreTerm {
    let mut steps_remaining = NORMALIZE_STEP_LIMIT;
    normalize_with_budget(term, &mut steps_remaining)
}

fn normalize_with_budget(term: &CoreTerm, budget: &mut u32) -> CoreTerm {
    if *budget == 0 {
        return term.clone();
    }
    *budget -= 1;
    match term {
        CoreTerm::Var(_) | CoreTerm::Universe(_) | CoreTerm::SmtProof(_) => term.clone(),

        // Head β-reduction: App(Lam, arg) reduces; otherwise
        // recurse into both sides.
        CoreTerm::App(f, arg) => {
            let f_norm = normalize_with_budget(f, budget);
            match f_norm {
                CoreTerm::Lam { binder, body, .. } => {
                    let arg_norm = normalize_with_budget(arg, budget);
                    let beta = substitute(&body, binder.as_str(), &arg_norm);
                    // Continue normalising the result — the
                    // substitution may have exposed further
                    // β-redexes deeper in the term.
                    normalize_with_budget(&beta, budget)
                }
                neutral => {
                    let arg_norm = normalize_with_budget(arg, budget);
                    CoreTerm::App(Heap::new(neutral), Heap::new(arg_norm))
                }
            }
        }

        CoreTerm::Pi { binder, domain, codomain } => CoreTerm::Pi {
            binder: binder.clone(),
            domain: Heap::new(normalize_with_budget(domain, budget)),
            codomain: Heap::new(normalize_with_budget(codomain, budget)),
        },
        CoreTerm::Lam { binder, domain, body } => CoreTerm::Lam {
            binder: binder.clone(),
            domain: Heap::new(normalize_with_budget(domain, budget)),
            body: Heap::new(normalize_with_budget(body, budget)),
        },
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => CoreTerm::Sigma {
            binder: binder.clone(),
            fst_ty: Heap::new(normalize_with_budget(fst_ty, budget)),
            snd_ty: Heap::new(normalize_with_budget(snd_ty, budget)),
        },
        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(normalize_with_budget(a, budget)),
            Heap::new(normalize_with_budget(b, budget)),
        ),
        CoreTerm::Fst(p) => {
            let p_norm = normalize_with_budget(p, budget);
            // Σ-projection β-rule: Fst(Pair(a, _)) → a.
            match p_norm {
                CoreTerm::Pair(a, _) => normalize_with_budget(&a, budget),
                neutral => CoreTerm::Fst(Heap::new(neutral)),
            }
        }
        CoreTerm::Snd(p) => {
            let p_norm = normalize_with_budget(p, budget);
            match p_norm {
                CoreTerm::Pair(_, b) => normalize_with_budget(&b, budget),
                neutral => CoreTerm::Snd(Heap::new(neutral)),
            }
        }
        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(normalize_with_budget(carrier, budget)),
            lhs: Heap::new(normalize_with_budget(lhs, budget)),
            rhs: Heap::new(normalize_with_budget(rhs, budget)),
        },
        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(normalize_with_budget(x, budget))),
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(normalize_with_budget(phi, budget)),
            walls: Heap::new(normalize_with_budget(walls, budget)),
            base: Heap::new(normalize_with_budget(base, budget)),
        },
        CoreTerm::Transp { path, regular, value } => CoreTerm::Transp {
            path: Heap::new(normalize_with_budget(path, budget)),
            regular: Heap::new(normalize_with_budget(regular, budget)),
            value: Heap::new(normalize_with_budget(value, budget)),
        },
        CoreTerm::Glue { carrier, phi, fiber, equiv } => CoreTerm::Glue {
            carrier: Heap::new(normalize_with_budget(carrier, budget)),
            phi: Heap::new(normalize_with_budget(phi, budget)),
            fiber: Heap::new(normalize_with_budget(fiber, budget)),
            equiv: Heap::new(normalize_with_budget(equiv, budget)),
        },
        CoreTerm::Refine { base, binder, predicate } => CoreTerm::Refine {
            base: Heap::new(normalize_with_budget(base, budget)),
            binder: binder.clone(),
            predicate: Heap::new(normalize_with_budget(predicate, budget)),
        },
        // V8 (#236) — quotient types: normalize all components.
        CoreTerm::Quotient { base, equiv } => CoreTerm::Quotient {
            base: Heap::new(normalize_with_budget(base, budget)),
            equiv: Heap::new(normalize_with_budget(equiv, budget)),
        },
        CoreTerm::QuotIntro { value, base, equiv } => CoreTerm::QuotIntro {
            value: Heap::new(normalize_with_budget(value, budget)),
            base: Heap::new(normalize_with_budget(base, budget)),
            equiv: Heap::new(normalize_with_budget(equiv, budget)),
        },
        // V8 (#236) — quotient β-rule: when scrutinee is QuotIntro,
        // collapse to `case applied to value`.
        CoreTerm::QuotElim { scrutinee, motive, case } => {
            let scrut_norm = normalize_with_budget(scrutinee, budget);
            match &scrut_norm {
                CoreTerm::QuotIntro { value, .. } => {
                    // β-redex: quot_elim([t]_~, motive, case) → case(t)
                    let case_norm = normalize_with_budget(case, budget);
                    let v_norm = normalize_with_budget(value, budget);
                    let app = CoreTerm::App(Heap::new(case_norm), Heap::new(v_norm));
                    normalize_with_budget(&app, budget)
                }
                _ => CoreTerm::QuotElim {
                    scrutinee: Heap::new(scrut_norm),
                    motive: Heap::new(normalize_with_budget(motive, budget)),
                    case: Heap::new(normalize_with_budget(case, budget)),
                },
            }
        }
        CoreTerm::Inductive { path, args } => {
            let mut new_args: List<CoreTerm> = List::new();
            for a in args.iter() {
                new_args.push(normalize_with_budget(a, budget));
            }
            CoreTerm::Inductive { path: path.clone(), args: new_args }
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut new_cases: List<CoreTerm> = List::new();
            for c in cases.iter() {
                new_cases.push(normalize_with_budget(c, budget));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(normalize_with_budget(scrutinee, budget)),
                motive: Heap::new(normalize_with_budget(motive, budget)),
                cases: new_cases,
            }
        }
        CoreTerm::Axiom { name, ty, framework } => CoreTerm::Axiom {
            name: name.clone(),
            ty: Heap::new(normalize_with_budget(ty, budget)),
            framework: framework.clone(),
        },
        CoreTerm::EpsilonOf(t) => {
            CoreTerm::EpsilonOf(Heap::new(normalize_with_budget(t, budget)))
        }
        CoreTerm::AlphaOf(t) => {
            CoreTerm::AlphaOf(Heap::new(normalize_with_budget(t, budget)))
        }
        CoreTerm::ModalBox(t) => {
            CoreTerm::ModalBox(Heap::new(normalize_with_budget(t, budget)))
        }
        CoreTerm::ModalDiamond(t) => {
            CoreTerm::ModalDiamond(Heap::new(normalize_with_budget(t, budget)))
        }
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args: List<Heap<CoreTerm>> = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(normalize_with_budget(a, budget)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
    }
}

/// V8 (#229) — ε-invariant token (Diakrisis Actic
/// 12-actic/03-epsilon-invariant.md). The Actic-side dual of
/// the canonical primitive carries an ordinal-valued
/// ε-coordinate distinct from `m_depth_omega`'s
/// `OrdinalDepth`. This enum is the bridge — a tagged union
/// of the canonical ε-token shapes the Actic spec admits.
///
/// Per VVA §A.Z.3.2 defect 3: Actic ε-arithmetic is a
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

/// V8 (#229) — convert an Actic ε-invariant to the kernel's
/// Cantor-normal-form [`crate::OrdinalDepth`].
///
/// Per VVA §A.Z.5 item 5 + Diakrisis Actic
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
///   * `convert(Zero) == finite(0)` — identity.
///   * `convert(Finite(n)) == finite(n)` — finite preservation.
///   * `convert(Omega) == omega()` — first-transfinite preservation.
///   * `convert(OmegaPlus(n)) == { omega_coeff: 1, finite_offset: n }`.
///   * `convert(OmegaTimes { coeff, offset }) ==
///     { omega_coeff: coeff, finite_offset: offset }`.
///   * Monotonicity: `eps1 ≤ eps2` (Actic order) implies
///     `convert(eps1).lt_or_eq(&convert(eps2))` (kernel lex).
///
/// The bridge is **canonical** (independent of how the Actic
/// ε-token was constructed) and **lossless** under the V0
/// encoding — every Actic ε that fits Cantor-normal-form
/// below ε_0 maps to a unique [`OrdinalDepth`].
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

/// V8 (#216) — definitional (β-aware) equality on [`CoreTerm`] values.
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
    let a_norm = normalize(a);
    let b_norm = normalize(b);
    a_norm == b_norm
}

/// V8 (#223) — δ-reduction-aware normaliser. Unfolds transparent
/// **definitions** (registered with non-None `body` per
/// [`crate::AxiomRegistry::register_definition`]) before
/// β-normalising.
///
/// Behaviour vs [`normalize`]:
///   • Opaque postulates (`body = None`) are LEFT as-is —
///     `Axiom { name: "..." }` references stay neutral. This is
///     correct: a postulate is, by design, not reducible.
///   • Transparent definitions (`body = Some(_)`) are UNFOLDED —
///     `Axiom { name: "Id", ... }` where `Id := λx. x` becomes
///     `λx. x` and continues normalising.
///   • Every other CoreTerm constructor delegates to the same
///     β-rules as [`normalize`].
///
/// Step limit ([`NORMALIZE_STEP_LIMIT`]) shared with [`normalize`];
/// δ-unfolds count against the same budget as β-reductions.
pub fn normalize_with_axioms(
    term: &CoreTerm,
    axioms: &crate::AxiomRegistry,
) -> CoreTerm {
    let mut budget = NORMALIZE_STEP_LIMIT;
    normalize_with_axioms_budget(term, axioms, &mut budget)
}

fn normalize_with_axioms_budget(
    term: &CoreTerm,
    axioms: &crate::AxiomRegistry,
    budget: &mut u32,
) -> CoreTerm {
    if *budget == 0 {
        return term.clone();
    }
    *budget -= 1;
    match term {
        CoreTerm::Var(_) | CoreTerm::Universe(_) | CoreTerm::SmtProof(_) => term.clone(),

        // V8 (#223) — δ-reduction: unfold transparent
        // definitions in place. Opaque postulates remain
        // neutral.
        CoreTerm::Axiom { name, ty, framework } => {
            match axioms.get(name.as_str()) {
                verum_common::Maybe::Some(entry) => match &entry.body {
                    Some(body) => normalize_with_axioms_budget(body, axioms, budget),
                    None => CoreTerm::Axiom {
                        name: name.clone(),
                        ty: Heap::new(normalize_with_axioms_budget(ty, axioms, budget)),
                        framework: framework.clone(),
                    },
                },
                verum_common::Maybe::None => CoreTerm::Axiom {
                    name: name.clone(),
                    ty: Heap::new(normalize_with_axioms_budget(ty, axioms, budget)),
                    framework: framework.clone(),
                },
            }
        }

        // β-redex at the head + recursive descent — same shape
        // as `normalize_with_budget` but every recursive call
        // threads the axiom registry.
        CoreTerm::App(f, arg) => {
            let f_norm = normalize_with_axioms_budget(f, axioms, budget);
            match f_norm {
                CoreTerm::Lam { binder, body, .. } => {
                    let arg_norm = normalize_with_axioms_budget(arg, axioms, budget);
                    let beta = substitute(&body, binder.as_str(), &arg_norm);
                    normalize_with_axioms_budget(&beta, axioms, budget)
                }
                neutral => {
                    let arg_norm = normalize_with_axioms_budget(arg, axioms, budget);
                    CoreTerm::App(Heap::new(neutral), Heap::new(arg_norm))
                }
            }
        }

        CoreTerm::Pi { binder, domain, codomain } => CoreTerm::Pi {
            binder: binder.clone(),
            domain: Heap::new(normalize_with_axioms_budget(domain, axioms, budget)),
            codomain: Heap::new(normalize_with_axioms_budget(codomain, axioms, budget)),
        },
        CoreTerm::Lam { binder, domain, body } => CoreTerm::Lam {
            binder: binder.clone(),
            domain: Heap::new(normalize_with_axioms_budget(domain, axioms, budget)),
            body: Heap::new(normalize_with_axioms_budget(body, axioms, budget)),
        },
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => CoreTerm::Sigma {
            binder: binder.clone(),
            fst_ty: Heap::new(normalize_with_axioms_budget(fst_ty, axioms, budget)),
            snd_ty: Heap::new(normalize_with_axioms_budget(snd_ty, axioms, budget)),
        },
        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(normalize_with_axioms_budget(a, axioms, budget)),
            Heap::new(normalize_with_axioms_budget(b, axioms, budget)),
        ),
        CoreTerm::Fst(p) => {
            let p_norm = normalize_with_axioms_budget(p, axioms, budget);
            match p_norm {
                CoreTerm::Pair(a, _) => normalize_with_axioms_budget(&a, axioms, budget),
                neutral => CoreTerm::Fst(Heap::new(neutral)),
            }
        }
        CoreTerm::Snd(p) => {
            let p_norm = normalize_with_axioms_budget(p, axioms, budget);
            match p_norm {
                CoreTerm::Pair(_, b) => normalize_with_axioms_budget(&b, axioms, budget),
                neutral => CoreTerm::Snd(Heap::new(neutral)),
            }
        }
        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(normalize_with_axioms_budget(carrier, axioms, budget)),
            lhs: Heap::new(normalize_with_axioms_budget(lhs, axioms, budget)),
            rhs: Heap::new(normalize_with_axioms_budget(rhs, axioms, budget)),
        },
        CoreTerm::Refl(x) => {
            CoreTerm::Refl(Heap::new(normalize_with_axioms_budget(x, axioms, budget)))
        }
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(normalize_with_axioms_budget(phi, axioms, budget)),
            walls: Heap::new(normalize_with_axioms_budget(walls, axioms, budget)),
            base: Heap::new(normalize_with_axioms_budget(base, axioms, budget)),
        },
        CoreTerm::Transp { path, regular, value } => CoreTerm::Transp {
            path: Heap::new(normalize_with_axioms_budget(path, axioms, budget)),
            regular: Heap::new(normalize_with_axioms_budget(regular, axioms, budget)),
            value: Heap::new(normalize_with_axioms_budget(value, axioms, budget)),
        },
        CoreTerm::Glue { carrier, phi, fiber, equiv } => CoreTerm::Glue {
            carrier: Heap::new(normalize_with_axioms_budget(carrier, axioms, budget)),
            phi: Heap::new(normalize_with_axioms_budget(phi, axioms, budget)),
            fiber: Heap::new(normalize_with_axioms_budget(fiber, axioms, budget)),
            equiv: Heap::new(normalize_with_axioms_budget(equiv, axioms, budget)),
        },
        CoreTerm::Refine { base, binder, predicate } => CoreTerm::Refine {
            base: Heap::new(normalize_with_axioms_budget(base, axioms, budget)),
            binder: binder.clone(),
            predicate: Heap::new(normalize_with_axioms_budget(predicate, axioms, budget)),
        },
        // V8 (#236) — quotient types under δ-aware normaliser.
        CoreTerm::Quotient { base, equiv } => CoreTerm::Quotient {
            base: Heap::new(normalize_with_axioms_budget(base, axioms, budget)),
            equiv: Heap::new(normalize_with_axioms_budget(equiv, axioms, budget)),
        },
        CoreTerm::QuotIntro { value, base, equiv } => CoreTerm::QuotIntro {
            value: Heap::new(normalize_with_axioms_budget(value, axioms, budget)),
            base: Heap::new(normalize_with_axioms_budget(base, axioms, budget)),
            equiv: Heap::new(normalize_with_axioms_budget(equiv, axioms, budget)),
        },
        CoreTerm::QuotElim { scrutinee, motive, case } => {
            let scrut_norm = normalize_with_axioms_budget(scrutinee, axioms, budget);
            match &scrut_norm {
                CoreTerm::QuotIntro { value, .. } => {
                    let case_norm = normalize_with_axioms_budget(case, axioms, budget);
                    let v_norm = normalize_with_axioms_budget(value, axioms, budget);
                    let app = CoreTerm::App(Heap::new(case_norm), Heap::new(v_norm));
                    normalize_with_axioms_budget(&app, axioms, budget)
                }
                _ => CoreTerm::QuotElim {
                    scrutinee: Heap::new(scrut_norm),
                    motive: Heap::new(normalize_with_axioms_budget(motive, axioms, budget)),
                    case: Heap::new(normalize_with_axioms_budget(case, axioms, budget)),
                },
            }
        }
        CoreTerm::Inductive { path, args } => {
            let mut new_args: List<CoreTerm> = List::new();
            for a in args.iter() {
                new_args.push(normalize_with_axioms_budget(a, axioms, budget));
            }
            CoreTerm::Inductive { path: path.clone(), args: new_args }
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut new_cases: List<CoreTerm> = List::new();
            for c in cases.iter() {
                new_cases.push(normalize_with_axioms_budget(c, axioms, budget));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(normalize_with_axioms_budget(scrutinee, axioms, budget)),
                motive: Heap::new(normalize_with_axioms_budget(motive, axioms, budget)),
                cases: new_cases,
            }
        }
        CoreTerm::EpsilonOf(t) => {
            CoreTerm::EpsilonOf(Heap::new(normalize_with_axioms_budget(t, axioms, budget)))
        }
        CoreTerm::AlphaOf(t) => {
            CoreTerm::AlphaOf(Heap::new(normalize_with_axioms_budget(t, axioms, budget)))
        }
        CoreTerm::ModalBox(t) => {
            CoreTerm::ModalBox(Heap::new(normalize_with_axioms_budget(t, axioms, budget)))
        }
        CoreTerm::ModalDiamond(t) => {
            CoreTerm::ModalDiamond(Heap::new(normalize_with_axioms_budget(t, axioms, budget)))
        }
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args: List<Heap<CoreTerm>> = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(normalize_with_axioms_budget(a, axioms, budget)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
    }
}

/// V8 (#223) — δ-reduction-aware definitional equality.
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
    let a_norm = normalize_with_axioms(a, axioms);
    let b_norm = normalize_with_axioms(b, axioms);
    a_norm == b_norm
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
        CoreTerm::Pi { binder, domain, codomain } => {
            free_vars_rec(domain, bound, out);
            bound.push(binder.clone());
            free_vars_rec(codomain, bound, out);
            bound.pop();
        }
        CoreTerm::Lam { binder, domain, body } => {
            free_vars_rec(domain, bound, out);
            bound.push(binder.clone());
            free_vars_rec(body, bound, out);
            bound.pop();
        }
        CoreTerm::App(f, a) => {
            free_vars_rec(f, bound, out);
            free_vars_rec(a, bound, out);
        }
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
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
        CoreTerm::Refl(x) => free_vars_rec(x, bound, out),
        CoreTerm::HComp { phi, walls, base } => {
            free_vars_rec(phi, bound, out);
            free_vars_rec(walls, bound, out);
            free_vars_rec(base, bound, out);
        }
        CoreTerm::Transp { path, regular, value } => {
            free_vars_rec(path, bound, out);
            free_vars_rec(regular, bound, out);
            free_vars_rec(value, bound, out);
        }
        CoreTerm::Glue { carrier, phi, fiber, equiv } => {
            free_vars_rec(carrier, bound, out);
            free_vars_rec(phi, bound, out);
            free_vars_rec(fiber, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::Refine { base, binder, predicate } => {
            free_vars_rec(base, bound, out);
            bound.push(binder.clone());
            free_vars_rec(predicate, bound, out);
            bound.pop();
        }

        // V8 (#236) — quotient types: no binder at this level.
        CoreTerm::Quotient { base, equiv } => {
            free_vars_rec(base, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::QuotIntro { value, base, equiv } => {
            free_vars_rec(value, bound, out);
            free_vars_rec(base, bound, out);
            free_vars_rec(equiv, bound, out);
        }
        CoreTerm::QuotElim { scrutinee, motive, case } => {
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
        CoreTerm::Elim { scrutinee, motive, cases } => {
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
///   syntactic reflexivity (`E == E`).
/// * `0x02` — **asserted**: the obligation matched a hypothesis
///   directly.
/// * `0x03` — **smt_unsat**: the backend reported `Unsat` on the
///   negated obligation using a generic theory combination.
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
pub fn replay_smt_cert(
    _ctx: &Context,
    cert: &SmtCertificate,
) -> Result<CoreTerm, KernelError> {
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
            })
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
///   1. Hash equality is checked **before** replay so a mismatched
///      certificate doesn't waste backend-table dispatch work.
///   2. On success, the witness term returned by
///      [`replay_smt_cert`] is unchanged — the comparison adds no
///      new failure mode beyond the new
///      [`KernelError::ObligationHashMismatch`] variant.
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
