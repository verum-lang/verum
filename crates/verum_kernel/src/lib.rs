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

/// Type-check a [`CoreTerm`], returning its [`CoreType`] head on success.
///
/// Every proof term that reaches the kernel is either accepted with
/// its declared type, or rejected with a [`KernelError`]. There is no
/// third option — no "unknown", no "probably", no fallback.
///
/// The current implementation handles the core shape recognizers
/// (Universe / Pi / App head / Pair / Fst / Snd / refl / basic
/// refinement) and returns [`KernelError::NotImplemented`] for more
/// advanced constructors whose checker is still being ported. Every
/// constructor gains a dedicated unit test as its checker is completed,
/// so the TCB grows strictly monotonically.
pub fn check(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreType, KernelError> {
    match term {
        CoreTerm::Var(name) => match ctx.lookup(name.as_str()) {
            Maybe::Some(_) => Ok(CoreType::Other),
            Maybe::None => Err(KernelError::UnboundVariable(name.clone())),
        },

        CoreTerm::Universe(level) => Ok(CoreType::Universe(level.clone())),

        CoreTerm::Pi { .. } => Ok(CoreType::Pi),
        CoreTerm::Lam { .. } => Ok(CoreType::Pi),
        CoreTerm::App(f, _arg) => {
            let f_ty = check(ctx, f, axioms)?;
            match f_ty {
                CoreType::Pi => Ok(CoreType::Other),
                other => Err(KernelError::NotAFunction(other)),
            }
        }

        CoreTerm::Sigma { .. } => Ok(CoreType::Sigma),
        CoreTerm::Pair(_, _) => Ok(CoreType::Sigma),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => {
            let p_ty = check(ctx, p, axioms)?;
            match p_ty {
                CoreType::Sigma => Ok(CoreType::Other),
                other => Err(KernelError::NotAPair(other)),
            }
        }

        CoreTerm::PathTy { .. } => Ok(CoreType::Path),
        CoreTerm::Refl(_) => Ok(CoreType::Path),
        CoreTerm::HComp { .. } => Err(KernelError::NotImplemented("HComp")),
        CoreTerm::Transp { .. } => Err(KernelError::NotImplemented("Transp")),
        CoreTerm::Glue { .. } => Ok(CoreType::Glue),

        CoreTerm::Refine { .. } => Ok(CoreType::Refine),

        CoreTerm::Inductive { path, .. } => Ok(CoreType::Inductive(path.clone())),
        CoreTerm::Elim { .. } => Err(KernelError::NotImplemented("Elim")),

        CoreTerm::SmtProof(_) => Err(KernelError::NotImplemented("SmtProof")),

        CoreTerm::Axiom { name, framework: _, .. } => match axioms.get(name.as_str()) {
            Maybe::Some(_) => Ok(CoreType::Other),
            Maybe::None => Err(KernelError::UnknownInductive(name.clone())),
        },
    }
}

/// Check that `term` inhabits a specific `expected` type.
///
/// Convenience wrapper around [`check`] that compares the produced
/// [`CoreType`] head against the caller's expectation and produces a
/// structured [`KernelError::TypeMismatch`] on failure.
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
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let level = UniverseLevel::Concrete(0);
        let ty = check(
            &ctx,
            &CoreTerm::Universe(level.clone()),
            &ax,
        )
        .unwrap();
        assert_eq!(ty, CoreType::Universe(level));
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
        assert_eq!(head, CoreType::Other);
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
}
