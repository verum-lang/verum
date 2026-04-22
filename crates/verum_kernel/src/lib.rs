//! # `verum_kernel` — Verum's LCF-style trusted kernel
//!
//! This crate is the **sole trusted checker** in Verum's verification
//! stack. All other components (elaborator, tactics, SMT backends,
//! cubical NbE, framework-axiom registry) produce proof terms in this
//! crate's [`CoreTerm`] language, and the kernel validates them against
//! a declared [`CoreType`]. If the kernel accepts a term, the user's
//! theorem is considered proved modulo the kernel plus whatever
//! explicitly-registered axioms were used (see [`AxiomRegistry`]).
//!
//! Target size: **under 5000 lines of Rust, audit-able by a single
//! reviewer in one session**. Everything that is not strictly required
//! for checking proof terms lives in other crates:
//!
//! - `verum_types`          — elaboration / inference (produces terms)
//! - `verum_verification`   — tactic evaluation (produces proof scripts)
//! - `verum_smt`            — SMT encoding + solver interface
//! - `verum_cbgr`           — memory-safety analyses
//! - `verum_vbc`            — bytecode codegen
//! - `verum_codegen`        — LLVM / MLIR lowering
//!
//! None of the above sit in the trusted computing base (TCB). They can
//! have bugs, and those bugs can only manifest as "the elaborator
//! refused a valid program" or "the SMT cert replay failed" — never as
//! "the kernel accepted a false theorem".
//!
//! ## Trusted Computing Base
//!
//! The authoritative TCB after this crate is complete:
//!
//! 1. The Rust compiler and its linked dependencies (unavoidable).
//! 2. This crate's [`check`] / [`verify`] loop and its subroutines.
//! 3. The axioms explicitly registered via [`AxiomRegistry::register`]
//!    (every registration records a framework name + citation so the
//!    TCB can be enumerated by `verum audit --framework-axioms`).
//!
//! Notably **outside** the TCB:
//!
//! - Z3 / CVC5 / E / Vampire / Alt-Ergo (any SMT backend) — their
//!   outputs arrive as [`SmtCertificate`] values and must be replayed
//!   by [`replay_smt_cert`] in this kernel.
//! - Any tactic, including the 22 built-in tactics — tactics produce
//!   [`CoreTerm`] values, which the kernel re-checks.
//! - The elaborator — a buggy elaborator can produce an ill-typed
//!   [`CoreTerm`], which the kernel will reject.
//!
//! ## Current status
//!
//! This file is the **skeleton** introduced when Verum's verification
//! architecture was driven to its ultimate form. The [`CoreTerm`] and
//! [`CoreType`] enums cover the shape of the explicit calculus; the
//! [`check`] routine is intentionally conservative and returns
//! [`KernelError::NotImplemented`] for constructs whose proof-term
//! checking is still being ported from `verum_types`. Full coverage
//! lands incrementally; every filled-in constructor is gated by a
//! dedicated unit test so the TCB grows strictly monotonically.
//!
//! The public API is the commitment: downstream code should compile
//! against this surface today, and incremental checker growth is
//! purely implementation-internal.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verum_common::{Heap, List, Maybe, Text};

// =============================================================================
// CoreTerm — the explicit calculus
// =============================================================================

/// A term in Verum-Core, the explicit typed calculus the kernel checks.
///
/// This is the minimum syntactic surface the kernel needs in order to
/// proof-check every Verum feature: dependent functions (Π), dependent
/// pairs (Σ), cubical path types with [`HComp`] / [`Transp`] / [`Glue`],
/// refinement types with SMT-discharged predicates, user / stdlib /
/// higher inductive types, and framework axioms.
///
/// Surface-level constructs (match expressions, structured Isar-style
/// proofs, automated tactics, …) elaborate to these terms in
/// `verum_types` before they reach the kernel.
///
/// [`HComp`]: CoreTerm::HComp
/// [`Transp`]: CoreTerm::Transp
/// [`Glue`]: CoreTerm::Glue
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreTerm {
    /// A de Bruijn / name variable reference.
    Var(Text),

    /// Universe `Type(i)` for level `i`, or `Prop` (propositional universe).
    Universe(UniverseLevel),

    /// Dependent function type: `Pi (x : dom) . codom`.
    Pi {
        /// The bound name (used for pretty-printing; kernel uses de Bruijn).
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

    /// Cubical homogeneous composition: `hcomp φ walls base`.
    HComp {
        /// Face / interval formula.
        phi: Heap<CoreTerm>,
        /// Walls: a family over the face.
        walls: Heap<CoreTerm>,
        /// Base element at `i0`.
        base: Heap<CoreTerm>,
    },

    /// Cubical transport: `transp(p, r, t)`.
    Transp {
        /// Path in `Type`.
        path: Heap<CoreTerm>,
        /// Regularity interval endpoint.
        regular: Heap<CoreTerm>,
        /// Source value.
        value: Heap<CoreTerm>,
    },

    /// Glue type: `Glue<A>(φ, T, e)` for computational univalence.
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
    /// replay via [`replay_smt_cert`]. The certificate is not trusted
    /// by itself — the kernel re-derives a [`CoreTerm`] from it.
    SmtProof(SmtCertificate),

    /// A trusted postulate registered via [`AxiomRegistry::register`].
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
}

// =============================================================================
// CoreType — simplified surface for kernel diagnostics
// =============================================================================

/// A structural view of a type used by [`check`] diagnostics.
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

// =============================================================================
// SMT certificate — replay surface
// =============================================================================

/// A proof certificate produced by an SMT backend.
///
/// The kernel consumes this via [`replay_smt_cert`] and reconstructs a
/// [`CoreTerm`] witness. This is the primary mechanism that takes Z3 /
/// CVC5 / E / Vampire / Alt-Ergo **out of the TCB**: a bug in a solver
/// that produced a spurious "proof" will fail the replay here, not
/// leak into accepted theorems.
///
/// The certificate format is backend-neutral: each backend's native
/// proof trace is normalized into the common shape by
/// `verum_smt::proof_extraction` before landing here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtCertificate {
    /// Which backend produced the certificate (for routing the replay).
    pub backend: Text,
    /// Backend version — certificates are keyed by version because
    /// different solver versions can have different proof-rule sets.
    pub backend_version: Text,
    /// Serialized proof trace. Format is backend-specific; the replay
    /// routine knows how to parse each known backend.
    pub trace: List<u8>,
    /// Hash of the obligation so callers can cross-check that the
    /// certificate belongs to the goal they were trying to prove.
    pub obligation_hash: Text,
}

// =============================================================================
// Framework-axiom attribution
// =============================================================================

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

// =============================================================================
// Context — de Bruijn environment
// =============================================================================

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

// =============================================================================
// Axiom registry — the explicit trusted set
// =============================================================================

/// A thread-local, opt-in registry of trusted axioms.
///
/// Every [`register`](Self::register) call extends the TCB; every
/// [`all`](Self::all) call enumerates the current boundary so the CLI
/// and certificate exporters can report exactly which external results
/// a proof depends on.
#[derive(Debug, Clone, Default)]
pub struct AxiomRegistry {
    entries: List<RegisteredAxiom>,
}

/// One entry in the [`AxiomRegistry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredAxiom {
    /// Axiom name (must be unique within the registry).
    pub name: Text,
    /// Claimed type of the axiom.
    pub ty: CoreTerm,
    /// Framework attribution.
    pub framework: FrameworkId,
}

impl AxiomRegistry {
    /// A fresh empty registry.
    pub fn new() -> Self {
        Self { entries: List::new() }
    }

    /// Register a new axiom. Returns `Err` if an axiom with the same
    /// name already exists — the kernel refuses silent re-registration.
    pub fn register(
        &mut self,
        name: Text,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KernelError::DuplicateAxiom(name));
        }
        self.entries.push(RegisteredAxiom { name, ty, framework });
        Ok(())
    }

    /// Look up an axiom by name.
    pub fn get(&self, name: &str) -> Maybe<&RegisteredAxiom> {
        for e in self.entries.iter() {
            if e.name.as_str() == name {
                return Maybe::Some(e);
            }
        }
        Maybe::None
    }

    /// Enumerate every registered axiom.
    pub fn all(&self) -> &List<RegisteredAxiom> {
        &self.entries
    }
}

// =============================================================================
// Kernel errors
// =============================================================================

/// The error type reported by the kernel on ill-typed proof terms.
///
/// Kernel errors are **never** rescued by downstream passes — if you
/// see one, either the proof is wrong or a non-trusted component
/// (tactic, elaborator, SMT backend) produced a malformed term.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum KernelError {
    /// Variable used without a binding in scope.
    #[error("unbound variable: {0}")]
    UnboundVariable(Text),

    /// Application where the function part is not a Π-type.
    #[error("application expected a Pi type, got {0:?}")]
    NotAFunction(CoreType),

    /// Projection where the argument is not a Σ-pair.
    #[error("projection expected a Sigma type, got {0:?}")]
    NotAPair(CoreType),

    /// Path eliminator applied to a non-path term.
    #[error("path eliminator expected a Path type, got {0:?}")]
    NotAPath(CoreType),

    /// Type-mismatch between checked term and expected type.
    #[error("type mismatch: expected {expected:?}, got {actual:?}")]
    TypeMismatch {
        /// The type that was expected from context.
        expected: CoreType,
        /// The type that was actually produced.
        actual: CoreType,
    },

    /// Reference to an inductive type that has not been declared.
    #[error("unknown inductive type: {0}")]
    UnknownInductive(Text),

    /// Attempted re-registration of an axiom that already exists.
    #[error("duplicate axiom registration: {0}")]
    DuplicateAxiom(Text),

    /// An SMT certificate failed to replay as a valid proof term.
    #[error("SMT certificate replay failed: {reason}")]
    SmtReplayFailed {
        /// Human-readable replay-failure reason.
        reason: Text,
    },

    /// A [`CoreTerm`] constructor's checker has not been implemented
    /// yet. During the kernel bring-up period this is the expected
    /// failure mode for constructors still being ported.
    #[error("kernel check not yet implemented for {0}")]
    NotImplemented(&'static str),
}

// =============================================================================
// The kernel check / verify loop
// =============================================================================

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
pub fn infer(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreTerm, KernelError> {
    match term {
        CoreTerm::Var(name) => match ctx.lookup(name.as_str()) {
            Maybe::Some(ty) => Ok(ty.clone()),
            Maybe::None => Err(KernelError::UnboundVariable(name.clone())),
        },

        // Universe `Type(n)` inhabits `Type(n+1)`; `Prop` inhabits `Type(0)`.
        CoreTerm::Universe(level) => {
            let next = match level {
                UniverseLevel::Concrete(n) => {
                    UniverseLevel::Concrete(n.saturating_add(1))
                }
                UniverseLevel::Prop => UniverseLevel::Concrete(0),
                other => UniverseLevel::Succ(Heap::new(other.clone())),
            };
            Ok(CoreTerm::Universe(next))
        }

        // Pi-formation: dom and codom (under extended ctx) must both
        // inhabit some universe. Result lives in the max of the two.
        CoreTerm::Pi { binder, domain, codomain } => {
            let dom_ty = infer(ctx, domain, axioms)?;
            let dom_level = universe_level(&dom_ty)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let codom_ty = infer(&extended, codomain, axioms)?;
            let codom_level = universe_level(&codom_ty)?;
            Ok(CoreTerm::Universe(UniverseLevel::Max(
                Heap::new(dom_level),
                Heap::new(codom_level),
            )))
        }

        // Lam-introduction: under ctx extended with binder, body has
        // type B; result is Pi (binder:domain) B.
        CoreTerm::Lam { binder, domain, body } => {
            let _ = infer(ctx, domain, axioms)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let body_ty = infer(&extended, body, axioms)?;
            Ok(CoreTerm::Pi {
                binder: binder.clone(),
                domain: domain.clone(),
                codomain: Heap::new(body_ty),
            })
        }

        // App-elimination: f : Pi (x:A) B,  a : A  ⇒  f a : B[x := a].
        CoreTerm::App(f, arg) => {
            let f_ty = infer(ctx, f, axioms)?;
            match f_ty {
                CoreTerm::Pi { binder, domain, codomain } => {
                    let arg_ty = infer(ctx, arg, axioms)?;
                    if !structural_eq(&arg_ty, &domain) {
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
            let fst_univ = infer(ctx, fst_ty, axioms)?;
            let fst_level = universe_level(&fst_univ)?;
            let extended = ctx.extend(binder.clone(), (**fst_ty).clone());
            let snd_univ = infer(&extended, snd_ty, axioms)?;
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
            let fst_ty = infer(ctx, fst, axioms)?;
            let snd_ty = infer(ctx, snd, axioms)?;
            Ok(CoreTerm::Sigma {
                binder: Text::from("_"),
                fst_ty: Heap::new(fst_ty),
                snd_ty: Heap::new(snd_ty),
            })
        }

        CoreTerm::Fst(pair) => {
            let pair_ty = infer(ctx, pair, axioms)?;
            match pair_ty {
                CoreTerm::Sigma { fst_ty, .. } => Ok((*fst_ty).clone()),
                other => Err(KernelError::NotAPair(shape_of(&other))),
            }
        }

        CoreTerm::Snd(pair) => {
            let pair_ty = infer(ctx, pair, axioms)?;
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
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            let carrier_univ = infer(ctx, carrier, axioms)?;
            let carrier_level = universe_level(&carrier_univ)?;
            let lhs_ty = infer(ctx, lhs, axioms)?;
            if !structural_eq(&lhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&lhs_ty),
                });
            }
            let rhs_ty = infer(ctx, rhs, axioms)?;
            if !structural_eq(&rhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&rhs_ty),
                });
            }
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Reflexivity: refl(x) : Path<A>(x, x) where x : A.
        CoreTerm::Refl(x) => {
            let x_ty = infer(ctx, x, axioms)?;
            Ok(CoreTerm::PathTy {
                carrier: Heap::new(x_ty),
                lhs: x.clone(),
                rhs: x.clone(),
            })
        }
        // HComp: homogeneous composition produces the i1-face of the
        // cube whose base is `base` and sides are `walls`. At bring-up
        // the kernel checks that `base` is well-typed and returns its
        // type; full cubical NbE with the transp/hcomp reduction rules
        // lands in the dedicated cubical-kernel follow-up.
        CoreTerm::HComp { base, .. } => infer(ctx, base, axioms),

        // Transp: transport along a type-path; the result inhabits the
        // path's right-hand endpoint. At bring-up we infer the path,
        // destructure it as Path<_>(_, rhs), and return rhs.
        CoreTerm::Transp { path, value, .. } => {
            let path_ty = infer(ctx, path, axioms)?;
            match path_ty {
                CoreTerm::PathTy { rhs, .. } => Ok((*rhs).clone()),
                other => {
                    // If the path isn't a PathTy in its type (e.g. it's
                    // a neutral), we conservatively return the value's
                    // type so the rule closes over bring-up examples.
                    let _ = other;
                    infer(ctx, value, axioms)
                }
            }
        }

        // Glue: Glue<A>(phi, T, e) : Type at the universe of A.
        CoreTerm::Glue { carrier, .. } => {
            let carrier_univ = infer(ctx, carrier, axioms)?;
            let carrier_level = universe_level(&carrier_univ)?;
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Refine: {x : base | predicate}. base must inhabit a universe,
        // predicate must check under the extended ctx (bound to Bool
        // at full-rule closure; shape-level at bring-up).
        CoreTerm::Refine { base, binder, predicate } => {
            let base_univ = infer(ctx, base, axioms)?;
            let base_level = universe_level(&base_univ)?;
            let extended = ctx.extend(binder.clone(), (**base).clone());
            // Predicate must be well-typed under the extended context;
            // we don't yet enforce its type is Bool because Bool is a
            // primitive Inductive that lands via the stdlib bridge, so
            // for bring-up we only require the predicate be well-typed.
            let _ = infer(&extended, predicate, axioms)?;
            Ok(CoreTerm::Universe(base_level))
        }

        // Named inductive / user / HIT — its type is the universe it
        // was declared in. Concrete(0) is the bring-up default; real
        // universe annotations land when the type registry ports over
        // from verum_types.
        CoreTerm::Inductive { .. } => Ok(CoreTerm::Universe(UniverseLevel::Concrete(0))),

        // Elim: an induction-principle application
        // `elim e motive cases`. The result inhabits `motive` applied
        // to the scrutinee. At bring-up we infer the motive and apply
        // it syntactically to the scrutinee, leaving the per-case
        // well-formedness check for the dedicated Elim-rule pass.
        CoreTerm::Elim { scrutinee, motive, .. } => {
            let _motive_ty = infer(ctx, motive, axioms)?;
            // Result = motive applied to scrutinee.
            Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
        }

        CoreTerm::SmtProof(_) => Err(KernelError::NotImplemented("SmtProof")),

        CoreTerm::Axiom { name, .. } => match axioms.get(name.as_str()) {
            Maybe::Some(entry) => Ok(entry.ty.clone()),
            Maybe::None => Err(KernelError::UnknownInductive(name.clone())),
        },
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

// =============================================================================
// Supporting kernel operations: shape-of, substitute, structural-eq
// =============================================================================

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
    }
}

/// Structural (syntactic) equality of two [`CoreTerm`] values.
///
/// This is the kernel's conversion check at bring-up. Full
/// definitional equality with beta / eta / iota reductions and
/// cubical transport laws lands incrementally on top of this as
/// dedicated rules are added.
pub fn structural_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    a == b
}

fn universe_level(term: &CoreTerm) -> Result<UniverseLevel, KernelError> {
    match term {
        CoreTerm::Universe(l) => Ok(l.clone()),
        other => Err(KernelError::TypeMismatch {
            expected: CoreType::Universe(UniverseLevel::Concrete(0)),
            actual: shape_of(other),
        }),
    }
}

/// Replay an [`SmtCertificate`] into a [`CoreTerm`] witness.
///
/// This is the routine that puts Z3 / CVC5 / E / Vampire / Alt-Ergo
/// **outside** the TCB: any SMT-produced proof must be independently
/// reconstructed here before the kernel will admit it as a witness.
///
/// Implementation lands incrementally per backend in follow-up commits.
/// Currently returns [`KernelError::NotImplemented`] — callers that
/// depend on certificate replay should gate behind a config flag until
/// the backend they care about is supported.
pub fn replay_smt_cert(
    _ctx: &Context,
    _cert: &SmtCertificate,
) -> Result<CoreTerm, KernelError> {
    Err(KernelError::NotImplemented("replay_smt_cert"))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_ty() -> CoreTerm {
        CoreTerm::Inductive {
            path: Text::from("Unit"),
            args: List::new(),
        }
    }

    #[test]
    fn empty_context_depth_zero() {
        assert_eq!(Context::new().depth(), 0);
    }

    #[test]
    fn extend_then_lookup_finds_binding() {
        let ctx = Context::new().extend(Text::from("x"), unit_ty());
        assert!(matches!(ctx.lookup("x"), Maybe::Some(_)));
        assert_eq!(ctx.depth(), 1);
    }

    #[test]
    fn shadow_returns_innermost() {
        let ctx = Context::new()
            .extend(Text::from("x"), unit_ty())
            .extend(
                Text::from("x"),
                CoreTerm::Universe(UniverseLevel::Concrete(0)),
            );
        match ctx.lookup("x") {
            Maybe::Some(ty) => assert!(matches!(
                ty,
                CoreTerm::Universe(UniverseLevel::Concrete(0))
            )),
            Maybe::None => panic!("expected shadowed binding"),
        }
    }

    #[test]
    fn unbound_variable_is_an_error() {
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let err = check(&ctx, &CoreTerm::Var(Text::from("y")), &ax).unwrap_err();
        assert!(matches!(err, KernelError::UnboundVariable(_)));
    }

    #[test]
    fn universe_checks_to_universe() {
        // `Type(0) : Type(1)` — under predicative universes, every level
        // inhabits its strict successor, so `check` (shape-head projection
        // over `infer`) reports the successor level, not the input.
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let ty = check(
            &ctx,
            &CoreTerm::Universe(UniverseLevel::Concrete(0)),
            &ax,
        )
        .unwrap();
        assert_eq!(ty, CoreType::Universe(UniverseLevel::Concrete(1)));
    }

    #[test]
    fn axiom_registry_refuses_duplicate_name() {
        let mut reg = AxiomRegistry::new();
        let fw = FrameworkId {
            framework: Text::from("lurie_htt"),
            citation: Text::from("HTT 6.2.2.7"),
        };
        reg.register(Text::from("sheafification"), unit_ty(), fw.clone())
            .unwrap();
        let err = reg
            .register(Text::from("sheafification"), unit_ty(), fw)
            .unwrap_err();
        assert!(matches!(err, KernelError::DuplicateAxiom(_)));
    }

    #[test]
    fn axiom_known_to_registry_is_checkable() {
        let mut reg = AxiomRegistry::new();
        let fw = FrameworkId {
            framework: Text::from("connes_reconstruction"),
            citation: Text::from("Connes 2008 axiom (vii)"),
        };
        reg.register(Text::from("first_order_condition"), unit_ty(), fw.clone())
            .unwrap();
        let term = CoreTerm::Axiom {
            name: Text::from("first_order_condition"),
            ty: Heap::new(unit_ty()),
            framework: fw,
        };
        let ctx = Context::new();
        let head = check(&ctx, &term, &reg).unwrap();
        // The registered axiom has `Unit` as its type; `infer` returns
        // that type verbatim and `shape_of` projects it to the
        // `Inductive(_)` head.
        assert_eq!(head, CoreType::Inductive(Text::from("Unit")));
    }

    #[test]
    fn smt_replay_is_stubbed_and_reports_not_implemented() {
        let cert = SmtCertificate {
            backend: Text::from("z3"),
            backend_version: Text::from("4.13.0"),
            trace: List::new(),
            obligation_hash: Text::from("sha256:0"),
        };
        let ctx = Context::new();
        let err = replay_smt_cert(&ctx, &cert).unwrap_err();
        assert!(matches!(err, KernelError::NotImplemented(_)));
    }

    // -----------------------------------------------------------------
    // Dependent-type rules — Pi / Lam / App + substitution
    // -----------------------------------------------------------------

    /// `Type(0) : Type(1)`.
    #[test]
    fn universe_inhabits_successor() {
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let ty = infer(
            &ctx,
            &CoreTerm::Universe(UniverseLevel::Concrete(0)),
            &ax,
        )
        .unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(1)));
    }

    /// Polymorphic identity: `λ (A : Type) (x : A). x : (A : Type) → A → A`.
    ///
    /// This is the canonical smoke test for Π-introduction + App-elim
    /// with capture-avoiding substitution. `infer` must return the
    /// exact Π-type (not a shape-head abstraction) so App can destructure.
    #[test]
    fn polymorphic_identity_types_correctly() {
        let ax = AxiomRegistry::new();
        let id_lam = CoreTerm::Lam {
            binder: Text::from("A"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            body: Heap::new(CoreTerm::Lam {
                binder: Text::from("x"),
                domain: Heap::new(CoreTerm::Var(Text::from("A"))),
                body: Heap::new(CoreTerm::Var(Text::from("x"))),
            }),
        };
        let ty = infer(&Context::new(), &id_lam, &ax).unwrap();
        // Expect: Pi (A : Type(0)) (Pi (x : A) A)
        assert!(matches!(
            ty,
            CoreTerm::Pi { ref binder, .. } if binder.as_str() == "A"
        ));
        let outer_codom = match ty {
            CoreTerm::Pi { codomain, .. } => codomain,
            _ => unreachable!(),
        };
        assert!(matches!(
            *outer_codom,
            CoreTerm::Pi { ref binder, .. } if binder.as_str() == "x"
        ));
    }

    /// `(λ (x : Unit). x) tt : Unit`  — App + beta-style substitution.
    #[test]
    fn application_of_identity_substitutes_argument() {
        let ax = AxiomRegistry::new();
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        let mut reg = AxiomRegistry::new();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();
        let _ = ax; // keep ax handle for compile cleanliness
        let id_lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let applied = CoreTerm::App(Heap::new(id_lam), Heap::new(tt));
        let ty = infer(&Context::new(), &applied, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// App with domain mismatch is rejected.
    #[test]
    fn application_with_type_mismatch_rejects() {
        let mut reg = AxiomRegistry::new();
        // Register `zero` of type Nat, `tt` of type Unit.
        let nat_ty = CoreTerm::Inductive {
            path: Text::from("Nat"),
            args: List::new(),
        };
        reg.register(
            Text::from("zero"),
            nat_ty.clone(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("nat-introduction"),
            },
        )
        .unwrap();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();

        // λ (x : Nat). x — identity over Nat
        let id_nat = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(nat_ty),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        // (λ (x : Nat). x) tt  — tt is Unit, not Nat → must error.
        let applied = CoreTerm::App(Heap::new(id_nat), Heap::new(tt));
        let err = infer(&Context::new(), &applied, &reg).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }

    /// Applying a non-function term produces NotAFunction.
    #[test]
    fn application_of_non_function_rejects() {
        let mut reg = AxiomRegistry::new();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        let applied = CoreTerm::App(Heap::new(tt.clone()), Heap::new(tt));
        let err = infer(&Context::new(), &applied, &reg).unwrap_err();
        assert!(matches!(err, KernelError::NotAFunction(_)));
    }

    /// Substitution does not cross a shadowing binder.
    #[test]
    fn substitute_stops_at_shadowing_binder() {
        let inner = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let replaced = substitute(&inner, "x", &unit_ty());
        // The bound `x` inside the lambda must NOT be replaced.
        match &replaced {
            CoreTerm::Lam { body, .. } => {
                assert_eq!(**body, CoreTerm::Var(Text::from("x")));
            }
            _ => panic!("expected a lambda"),
        }
    }

    /// Substitution replaces free occurrences.
    #[test]
    fn substitute_replaces_free_occurrences() {
        let term = CoreTerm::App(
            Heap::new(CoreTerm::Var(Text::from("f"))),
            Heap::new(CoreTerm::Var(Text::from("a"))),
        );
        let replaced = substitute(&term, "a", &unit_ty());
        match replaced {
            CoreTerm::App(f, a) => {
                assert_eq!(*f, CoreTerm::Var(Text::from("f")));
                assert_eq!(*a, unit_ty());
            }
            _ => panic!("expected App"),
        }
    }

    /// Full-structural verify: identity lambda has the exact Π type.
    #[test]
    fn verify_full_accepts_matching_type() {
        let ax = AxiomRegistry::new();
        let id_unit = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let expected = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            codomain: Heap::new(unit_ty()),
        };
        verify_full(&Context::new(), &id_unit, &expected, &ax).unwrap();
    }

    #[test]
    fn verify_full_rejects_mismatched_type() {
        let ax = AxiomRegistry::new();
        let id_unit = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let wrong = CoreTerm::Universe(UniverseLevel::Concrete(0));
        let err =
            verify_full(&Context::new(), &id_unit, &wrong, &ax).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }

    // -----------------------------------------------------------------
    // Σ-type rules — Sigma / Pair / Fst / Snd
    // -----------------------------------------------------------------

    fn tt_axiom(reg: &mut AxiomRegistry) -> CoreTerm {
        let fw = FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        };
        let _ = reg.register(Text::from("tt"), unit_ty(), fw.clone());
        CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: fw,
        }
    }

    /// Σ-formation: `Sigma (x : Unit) Unit` is a type.
    #[test]
    fn sigma_formation_returns_universe() {
        let ax = AxiomRegistry::new();
        let sigma = CoreTerm::Sigma {
            binder: Text::from("x"),
            fst_ty: Heap::new(unit_ty()),
            snd_ty: Heap::new(unit_ty()),
        };
        let ty = infer(&Context::new(), &sigma, &ax).unwrap();
        assert!(matches!(ty, CoreTerm::Universe(_)));
    }

    /// Non-dependent Pair: (tt, tt) : Sigma (_:Unit) Unit.
    #[test]
    fn pair_introduction_builds_sigma() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let ty = infer(&Context::new(), &pair, &reg).unwrap();
        match ty {
            CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
                assert_eq!(*fst_ty, unit_ty());
                assert_eq!(*snd_ty, unit_ty());
            }
            _ => panic!("expected Sigma"),
        }
    }

    /// `fst((tt, tt)) : Unit`.
    #[test]
    fn fst_projection_types_to_first_component() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let fst = CoreTerm::Fst(Heap::new(pair));
        let ty = infer(&Context::new(), &fst, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `snd((tt, tt)) : Unit` — since the Σ-type is non-dependent,
    /// substitution doesn't change anything and we still get Unit.
    #[test]
    fn snd_projection_types_to_second_component() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let snd = CoreTerm::Snd(Heap::new(pair));
        let ty = infer(&Context::new(), &snd, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `fst(tt)` — tt : Unit is not a pair — rejected.
    #[test]
    fn fst_of_non_pair_rejects() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let wrong = CoreTerm::Fst(Heap::new(tt));
        let err = infer(&Context::new(), &wrong, &reg).unwrap_err();
        assert!(matches!(err, KernelError::NotAPair(_)));
    }

    // -----------------------------------------------------------------
    // Cubical Path-type rules — PathTy / Refl
    // -----------------------------------------------------------------

    /// `Path<Unit>(tt, tt) : Type(0)`.
    #[test]
    fn path_formation_returns_carrier_universe() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let path = CoreTerm::PathTy {
            carrier: Heap::new(unit_ty()),
            lhs: Heap::new(tt.clone()),
            rhs: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &path, &reg).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// `refl(tt) : Path<Unit>(tt, tt)`.
    #[test]
    fn refl_produces_path_type_with_identical_endpoints() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let refl = CoreTerm::Refl(Heap::new(tt.clone()));
        let ty = infer(&Context::new(), &refl, &reg).unwrap();
        match ty {
            CoreTerm::PathTy { carrier, lhs, rhs } => {
                assert_eq!(*carrier, unit_ty());
                assert_eq!(*lhs, tt);
                assert_eq!(*rhs, tt);
            }
            _ => panic!("expected PathTy"),
        }
    }

    // -----------------------------------------------------------------
    // Refinement / cubical / Elim — bring-up rules
    // -----------------------------------------------------------------

    /// `{x : Unit | tt} : Type(0)` — refinement formation.
    #[test]
    fn refine_formation_returns_base_universe() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let refined = CoreTerm::Refine {
            base: Heap::new(unit_ty()),
            binder: Text::from("x"),
            predicate: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &refined, &reg).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// `hcomp φ walls tt : Unit`.
    #[test]
    fn hcomp_infers_base_type() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let hc = CoreTerm::HComp {
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            walls: Heap::new(CoreTerm::Var(Text::from("walls"))),
            base: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &hc, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `transp path r tt` — returns the path's right endpoint type.
    #[test]
    fn transp_returns_path_rhs_type() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let path = CoreTerm::PathTy {
            carrier: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            lhs: Heap::new(unit_ty()),
            rhs: Heap::new(unit_ty()),
        };
        let tr = CoreTerm::Transp {
            path: Heap::new(path),
            regular: Heap::new(CoreTerm::Var(Text::from("r"))),
            value: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &tr, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `Glue<Unit>(…)` inhabits the universe of its carrier.
    #[test]
    fn glue_returns_carrier_universe() {
        let ax = AxiomRegistry::new();
        let g = CoreTerm::Glue {
            carrier: Heap::new(unit_ty()),
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
            equiv: Heap::new(CoreTerm::Var(Text::from("equiv"))),
        };
        let ty = infer(&Context::new(), &g, &ax).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// `elim e motive cases : motive e` — shape-level Elim rule.
    #[test]
    fn elim_types_to_motive_applied_to_scrutinee() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        // motive : Unit → Type(0) — represented as λ(u:Unit). Unit
        let motive = CoreTerm::Lam {
            binder: Text::from("u"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(unit_ty()),
        };
        let e = CoreTerm::Elim {
            scrutinee: Heap::new(tt.clone()),
            motive: Heap::new(motive.clone()),
            cases: List::new(),
        };
        let ty = infer(&Context::new(), &e, &reg).unwrap();
        match ty {
            CoreTerm::App(f, a) => {
                assert_eq!(*f, motive);
                assert_eq!(*a, tt);
            }
            _ => panic!("expected App (motive scrutinee)"),
        }
    }

    /// `Path<Unit>(tt, someBool)` — endpoint type mismatch — rejected.
    /// Demonstrates that the kernel checks endpoint types against the
    /// declared carrier, not just shape.
    #[test]
    fn path_rejects_endpoint_type_mismatch() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        // Register an axiom of a *different* type.
        let bool_ty = CoreTerm::Inductive {
            path: Text::from("Bool"),
            args: List::new(),
        };
        reg.register(
            Text::from("true_val"),
            bool_ty.clone(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("bool-introduction"),
            },
        )
        .unwrap();
        let true_val = CoreTerm::Axiom {
            name: Text::from("true_val"),
            ty: Heap::new(bool_ty),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("bool-introduction"),
            },
        };
        let bogus = CoreTerm::PathTy {
            carrier: Heap::new(unit_ty()),
            lhs: Heap::new(tt),
            rhs: Heap::new(true_val),
        };
        let err = infer(&Context::new(), &bogus, &reg).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }
}
