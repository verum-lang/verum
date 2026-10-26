//! Core syntactic surface — `CoreTerm`, `CoreType`, `UniverseLevel`.
//! Split per #198 V7 (final lib.rs sweep).
//!
//! These three types form the kernel's syntactic foundation: every
//! other module (errors, depth, infer, support, etc.) builds on top
//! of them. They live in their own module so the explicit calculus
//! has a single, greppable home for documentation, schema-version
//! review, and red-team auditing.

use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Text};

use crate::{FrameworkId, SmtCertificate};

/// A term in Verum-Core, the explicit typed calculus the kernel checks.
///
/// This is the minimum syntactic surface the kernel needs in order to
/// proof-check every Verum feature: dependent functions (Π), dependent
/// pairs (Σ), cubical path types with [`HComp`] / [`Transport`] / [`Glue`],
/// refinement types with SMT-discharged predicates, user / stdlib /
/// higher inductive types, and framework axioms.
///
/// Surface-level constructs (match expressions, structured Isar-style
/// proofs, automated tactics, …) elaborate to these terms in
/// `verum_types` before they reach the kernel.
///
/// [`HComp`]: CoreTerm::HComp
/// [`Transport`]: CoreTerm::Transport
/// [`Glue`]: CoreTerm::Glue
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreTerm {
    /// A local variable reference, resolved by lexical name.
    Var(Text),

    /// A saturated point constructor owned by one inductive declaration.
    /// Global constructor identity is separate from local variable spelling.
    Constructor {
        /// Qualified name of the owning inductive declaration.
        inductive: Text,
        /// Constructor name within that declaration.
        name: Text,
        /// Instantiated type parameters, in declaration order.
        params: List<CoreTerm>,
        /// Constructor fields, in declaration order.
        args: List<CoreTerm>,
    },

    /// Universe `Type(i)` for level `i`, or `Prop` (propositional universe).
    Universe(UniverseLevel),

    /// Dependent function type: `Pi (x : dom) . codom`.
    Pi {
        /// The lexical binder name; conversion compares binding positions.
        binder: Text,
        /// Domain type.
        domain: Heap<CoreTerm>,
        /// Codomain term (may reference `binder`).
        codomain: Heap<CoreTerm>,
    },

    /// Lambda abstraction: `λ (x : domain) . body`.
    Lam {
        /// The bound name.
        binder: Text,
        /// Parameter type.
        domain: Heap<CoreTerm>,
        /// Body term.
        body: Heap<CoreTerm>,
    },

    /// Function application: `f a`.
    App(Heap<CoreTerm>, Heap<CoreTerm>),

    /// Dependent pair type: `Sigma (x : fst_ty) . snd_ty`.
    Sigma {
        /// The bound name.
        binder: Text,
        /// First component type.
        fst_ty: Heap<CoreTerm>,
        /// Second component type (may reference `binder`).
        snd_ty: Heap<CoreTerm>,
    },

    /// Dependent pair constructor: `(a, b)`.
    Pair(Heap<CoreTerm>, Heap<CoreTerm>),

    /// First projection: `fst p`.
    Fst(Heap<CoreTerm>),

    /// Second projection: `snd p`.
    Snd(Heap<CoreTerm>),

    /// Cubical path type: `Path<A>(a, b)`.
    PathTy {
        /// Carrier type.
        carrier: Heap<CoreTerm>,
        /// Left endpoint.
        lhs: Heap<CoreTerm>,
        /// Right endpoint.
        rhs: Heap<CoreTerm>,
    },

    /// Reflexivity path: `refl(a) : Path<A>(a, a)`.
    Refl(Heap<CoreTerm>),

    /// **dependent path-over**:
    /// `PathOver(motive, p, lhs, rhs)`.
    ///
    /// When a HIT path-constructor has endpoints whose motive
    /// images are not definitionally equal — e.g. the Suspension
    /// HIT's `merid : north ↝ south` where `motive(north) ≠
    /// motive(south)` definitionally — the eliminator's path-
    /// branch type cannot be the homogeneous `PathTy(motive(lhs),
    /// lhs_image, rhs_image)`. Instead it must be a **heterogeneous**
    /// path lying *over* the constructor-path `p` along the
    /// motive: `PathOver(motive, p, lhs_image, rhs_image)` where
    /// `lhs_image : motive(lhs)` and `rhs_image : motive(rhs)`.
    ///
    /// **Typing rule (K-PathOver-Form):**
    ///
    /// ```text
    ///  Γ ⊢ motive : B → U Γ ⊢ p : Path<B>(b₀, b₁)
    ///  Γ ⊢ lhs : motive(b₀) Γ ⊢ rhs : motive(b₁)
    ///  ───────────────────────────────────────────────
    ///  Γ ⊢ PathOver(motive, p, lhs, rhs) : U
    /// ```
    ///
    /// **Reflexive reduction:** `PathOver(motive, Refl(base), lhs, rhs)`
    /// reduces to `PathTy(motive(base), lhs, rhs)`. A neutral loop whose
    /// endpoints happen to coincide does not authorize this conversion.
    /// The path field carries evidence, never a path-type annotation.
    ///
    /// References:
    /// * HoTT Book §6.2 (Function spaces over identifications).
    /// * Cubical Agda's `PathP A x y` (the same primitive).
    PathOver {
        /// Type-family `motive : B → U` along whose image the path
        /// lies. Stored as the raw motive expression (no eta-
        /// expansion) so substitution and normalisation operate
        /// uniformly with `PathTy`.
        motive: Heap<CoreTerm>,
        /// Constructor-path in the base type `B`. Typically
        /// `Var(point_ctor)` ↝ `Var(point_ctor')` for a HIT path-
        /// constructor; arbitrary terms admitted to support
        /// elaborated higher-cell structure.
        path: Heap<CoreTerm>,
        /// Left endpoint, lives in `motive(b₀)` where `b₀` is the
        /// left endpoint of `path`.
        lhs: Heap<CoreTerm>,
        /// Right endpoint, lives in `motive(b₁)` where `b₁` is the
        /// right endpoint of `path`.
        rhs: Heap<CoreTerm>,
    },

    /// Cubical composition syntax, retained for diagnostics. Named inference
    /// refuses it until interval, partial-element and boundary judgments exist.
    HComp {
        /// Face / interval formula.
        phi: Heap<CoreTerm>,
        /// Walls: a family over the face.
        walls: Heap<CoreTerm>,
        /// Base element at `i0`.
        base: Heap<CoreTerm>,
    },

    /// Ordinary transport along evidence `path : Path<Type>(A, B)`.
    /// A source value in `A` is transported to `B`; only `Refl(A)` reduces
    /// to the source value. Generalized cubical transport requires a
    /// separate typed interval and constancy judgment, not a marker term.
    Transport {
        /// Checked path between types.
        path: Heap<CoreTerm>,
        /// Source value.
        value: Heap<CoreTerm>,
    },

    /// Gluing syntax, retained for diagnostics. Named inference refuses it
    /// until partial families and equivalence coherence are checked.
    Glue {
        /// Carrier type.
        carrier: Heap<CoreTerm>,
        /// Face formula.
        phi: Heap<CoreTerm>,
        /// Fiber type family.
        fiber: Heap<CoreTerm>,
        /// Equivalence family.
        equiv: Heap<CoreTerm>,
    },

    /// Refinement type: `{ x : base | predicate(x) }`.
    ///
    /// The kernel records the predicate but delegates decidability to
    /// the SMT backend via [`CoreTerm::SmtProof`].
    Refine {
        /// Base type.
        base: Heap<CoreTerm>,
        /// Bound variable inside the predicate.
        binder: Text,
        /// Predicate term — must have type `Bool`.
        predicate: Heap<CoreTerm>,
    },

    /// Quotient of a carrier by a type-valued binary relation. The kernel
    /// checks `base : Type(u)` and `equiv : base -> base -> Type(v)`;
    /// the quotient lives in `Type(max(u, v))`. No equivalence-law or
    /// universe-resizing assumption is implicit in formation.
    Quotient {
        /// Carrier type.
        base: Heap<CoreTerm>,
        /// Relation whose related representatives the recursor must respect.
        equiv: Heap<CoreTerm>,
    },

    /// Quotient introduction, `[value] : Quotient(base, equiv)`.
    QuotIntro {
        /// Representative, checked against `base`.
        value: Heap<CoreTerm>,
        /// Carrier type.
        base: Heap<CoreTerm>,
        /// Well-formed binary relation.
        equiv: Heap<CoreTerm>,
    },

    /// Nondependent quotient recursion with explicit relation-respect evidence.
    ///
    /// For `Q = Quotient(A, R)`, the kernel checks:
    /// `scrutinee : Q`, `motive : Q -> Type(v)` with a definitionally
    /// constant result `B`, `case : A -> B`, and
    /// `respect : (x y : A) -> R x y -> Path<B>(case x, case y)`.
    /// The result has type `motive(scrutinee)` and computes to `case(value)`
    /// on an introduction. Genuinely dependent motives require dependent
    /// transport/coherence and are rejected by this rule.
    QuotElim {
        /// The quotient term being eliminated.
        scrutinee: Heap<CoreTerm>,
        /// Definitionally constant family over the quotient.
        motive: Heap<CoreTerm>,
        /// Representative map, checked against `A -> B`.
        case: Heap<CoreTerm>,
        /// Checked proof that the representative map respects the relation.
        respect: Heap<CoreTerm>,
    },

    /// Reference to a named inductive / higher-inductive type (from
    /// the stdlib, from the user's project, or from a framework axiom).
    ///
    /// The kernel looks `path` up in its registry of declared type
    /// constructors; missing names are a kernel error, not silent
    /// pass-through.
    Inductive {
        /// Qualified type name.
        path: Text,
        /// Type-level arguments.
        args: List<CoreTerm>,
    },

    /// Inductive eliminator: `elim e motive cases`.
    Elim {
        /// Scrutinee.
        scrutinee: Heap<CoreTerm>,
        /// Motive predicate.
        motive: Heap<CoreTerm>,
        /// One branch per constructor.
        cases: List<CoreTerm>,
    },

    /// A proof term produced by an SMT backend that the kernel will
    /// replay via [`crate::replay_smt_cert`]. The certificate is not
    /// trusted by itself — the kernel re-derives a [`CoreTerm`] from it.
    SmtProof(SmtCertificate),

    /// A trusted postulate registered via
    /// [`crate::AxiomRegistry::register`].
    ///
    /// Every `Axiom` node records the framework name + citation so the
    /// TCB can be enumerated by `verum audit --framework-axioms`.
    Axiom {
        /// Axiom identifier.
        name: Text,
        /// Claimed type (the kernel does not re-check this — it is the
        /// axiom).
        ty: Heap<CoreTerm>,
        /// Framework attribution.
        framework: FrameworkId,
    },

    /// Naturality witness — `EpsilonOf(α)` represents the canonical enactment
    /// image of an articulation under the M ⊣ A biadjunction (the
    /// activation modality applied at the articulation level). The
    /// kernel uses this constructor to track the natural-equivalence
    /// τ : ε ∘ M ≃ A ∘ ε of Proposition 5.1 / Corollary 5.10.
    ///
    /// ships the constructor + `K-Eps-Mu` skeleton (see
    /// `Kernel::check_eps_mu_coherence`); the full naturality check
    /// is deferred to V1, where the τ-witness construction will be
    /// wired in.
    EpsilonOf(Heap<CoreTerm>),

    /// Naturality witness — `AlphaOf(ε)` represents the canonical articulation
    /// image of an enactment (the inverse direction of the M ⊣ A
    /// biadjunction). Together with `EpsilonOf` this enables kernel-
    /// level reasoning about the ε ↔ α duality.
    AlphaOf(Heap<CoreTerm>),

    /// Modal-depth — `ModalBox(φ)` represents `□φ` (necessity in the
    /// underlying modal logic). md^ω(□φ) = md^ω(φ) + 1 per
    /// Definition 136.D1. The K-Refine-omega rule uses the
    /// resulting ordinal to gate refinement-type formation.
    ModalBox(Heap<CoreTerm>),

    /// Modal-depth — `ModalDiamond(φ)` represents `◇φ` (possibility).
    /// md^ω(◇φ) = md^ω(φ) + 1 per Definition 136.D1.
    ModalDiamond(Heap<CoreTerm>),

    /// Modal-depth — `ModalBigAnd(P_0, ..., P_κ)` represents the
    /// transfinite conjunction `⋀_{i<κ} P_i`. md^ω of the big-and
    /// is the *supremum* of the components' md^ω-ranks, per
    /// Definition 136.D1 + Lemma 136.L0 well-founded ordinal
    /// recursion. Used to express modal axiom schemes that
    /// quantify over all possible-world labels at once.
    ModalBigAnd(List<Heap<CoreTerm>>),

    /// **shape modality `∫A`** (Schreiber DCCT, cohesive
    /// HoTT). The leftmost of the cohesive triple-adjunction
    /// `∫ ⊣ ♭ ⊣ ♯`. Computes the underlying ∞-groupoid (homotopy
    /// type) of a cohesive type, forgetting the geometric / modal
    /// structure and keeping only the homotopy data.
    ///
    /// Per the kernel admits the type former unconditionally
    /// (the cubical-set semantics interprets it via localisation at
    /// the discrete-type subuniverse); the **adjunction laws** (unit
    /// `η : A → ♭∫A`, counit `ε : ∫♭A → A`, triangle identities) are
    /// recorded as framework axioms in `core.math.frameworks.schreiber_dcct`
    /// and only become visible when that framework is loaded.
    ///
    /// Reference: Schreiber U. 2013. *Differential Cohomology in a
    /// Cohesive ∞-Topos.* §3.4 (cohesive modalities).
    Shape(Heap<CoreTerm>),

    /// **flat modality `♭A`** (Schreiber DCCT). The
    /// middle of the cohesive triple-adjunction `∫ ⊣ ♭ ⊣ ♯`.
    /// Singles out the **discrete** (constant) part of a cohesive
    /// type — the points whose cohesive structure is "trivially
    /// connected." Plays the role of the necessity modality for
    /// crispness in cohesive HoTT.
    ///
    /// `♭A` is itself a (discrete) cohesive type, hence the kernel
    /// records it at the same universe level as `A`. Adjunction
    /// laws are framework-axiomatic per `schreiber_dcct`.
    ///
    /// Reference: Shulman M. 2018. *Brouwer's fixed-point theorem
    /// in real-cohesive homotopy type theory.* §3.
    Flat(Heap<CoreTerm>),

    /// **sharp modality `♯A`** (Schreiber DCCT). The
    /// rightmost of the cohesive triple-adjunction `∫ ⊣ ♭ ⊣ ♯`.
    /// Singles out the **codiscrete** (totally cohesive) part of
    /// a type — the points whose underlying homotopy type is
    /// `∫A` packaged with a maximal cohesive structure. Dual to
    /// `♭A` under the adjunction.
    ///
    /// Reference: Schreiber U. 2013. *Differential Cohomology in a
    /// Cohesive ∞-Topos.* §3.4.
    Sharp(Heap<CoreTerm>),
}

impl CoreTerm {
    /// Visit every immediate term child, preserving declaration metadata.
    /// The exhaustive match keeps structural checks complete as syntax grows.
    pub(crate) fn visit_children(&self, mut visit: impl FnMut(&CoreTerm)) {
        use CoreTerm::*;
        match self {
            Var(_) | Universe(_) | SmtProof(_) => {}
            Constructor { params, args, .. } => {
                params.iter().chain(args.iter()).for_each(&mut visit)
            }
            Pi {
                domain, codomain, ..
            } => {
                visit(domain);
                visit(codomain);
            }
            Lam { domain, body, .. } => {
                visit(domain);
                visit(body);
            }
            Sigma { fst_ty, snd_ty, .. } => {
                visit(fst_ty);
                visit(snd_ty);
            }
            Refine {
                base, predicate, ..
            } => {
                visit(base);
                visit(predicate);
            }
            App(a, b)
            | Pair(a, b)
            | Transport { path: a, value: b }
            | Quotient { base: a, equiv: b } => {
                visit(a);
                visit(b);
            }
            Fst(a) | Snd(a) | Refl(a) | EpsilonOf(a) | AlphaOf(a) | ModalBox(a)
            | ModalDiamond(a) | Shape(a) | Flat(a) | Sharp(a) => visit(a),
            PathTy {
                carrier: a,
                lhs: b,
                rhs: c,
            }
            | HComp {
                phi: a,
                walls: b,
                base: c,
            }
            | QuotIntro {
                value: a,
                base: b,
                equiv: c,
            } => {
                visit(a);
                visit(b);
                visit(c);
            }
            QuotElim {
                scrutinee: a,
                motive: b,
                case: c,
                respect: d,
            }
            | PathOver {
                motive: a,
                path: b,
                lhs: c,
                rhs: d,
            }
            | Glue {
                carrier: a,
                phi: b,
                fiber: c,
                equiv: d,
            } => {
                visit(a);
                visit(b);
                visit(c);
                visit(d);
            }
            Inductive { args, .. } => args.iter().for_each(&mut visit),
            Elim {
                scrutinee,
                motive,
                cases,
            } => {
                visit(scrutinee);
                visit(motive);
                cases.iter().for_each(&mut visit);
            }
            ModalBigAnd(args) => args.iter().for_each(|term| visit(term)),
            Axiom { ty, .. } => visit(ty),
        }
    }
}

/// A structural view of a type used by [`crate::check`] diagnostics.
///
/// Internally, the kernel works directly with [`CoreTerm`] values in
/// `Universe` position. This enum exists so that errors like
/// "expected a Pi type, got Int" can be reported without copying large
/// [`CoreTerm`] trees around.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreType {
    /// A universe `Type(i)` or `Prop`.
    Universe(UniverseLevel),
    /// A Π-type head.
    Pi,
    /// A Σ-type head.
    Sigma,
    /// A cubical path-type head.
    Path,
    /// A refinement-type head.
    Refine,
    /// A glue-type head.
    Glue,
    /// A named inductive / user / HIT head.
    Inductive(Text),
    /// Any other shape — deliberately coarse.
    Other,
}

/// A universe level — concrete, variable, or successor of another level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UniverseLevel {
    /// `Type(n)` for a concrete non-negative `n`.
    Concrete(u32),
    /// `Type(u)` for a level variable `u`.
    Variable(Text),
    /// `u + 1`.
    Succ(Heap<UniverseLevel>),
    /// `max(u, v)`.
    Max(Heap<UniverseLevel>, Heap<UniverseLevel>),
    /// The propositional universe `Prop`.
    Prop,
}
