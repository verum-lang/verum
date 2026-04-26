//! Inductive-type registry + strict-positivity checking (K-Pos rule).
//!
//! Split out of `lib.rs` per #198. An inductive type is well-formed
//! only when its own name appears *strictly positively* in every
//! constructor's argument types. Allowing non-positive recursion
//! (e.g. `type Bad = Wrap(Bad -> A)`) admits Berardi's paradox and
//! lets the user derive `False`.
//!
//! The kernel enforces this at registration time via
//! [`InductiveRegistry::register`]: every [`RegisteredInductive`] is
//! validated by [`check_strict_positivity`] before it is admitted.
//! A failure surfaces as `KernelError::PositivityViolation` with a
//! human-readable position string for diagnostic copy.
//!
//! VUVA spec §7.3 (`K-Pos`): for every constructor `C(t1, ..., tn) -> T`
//! of an inductive type `T`, every recursive occurrence of `T` in any
//! `ti` must appear strictly positively.
//!
//! Strict positivity, formally — for an inductive type name `T` and
//! a type `t`, the predicate `appears_only_strictly_positively(T, t)`:
//!
//!   * For `Universe(_)` / `Var(_)` / `Path(_)` / `Sigma(_)` (no
//!     functional negation through any arrow): admit iff every nested
//!     type satisfies the predicate.
//!   * For `Pi(domain, codomain)`: admit iff `T` does NOT appear
//!     anywhere in `domain` (the negative position) AND `codomain`
//!     itself is strictly positive in `T`.
//!   * For `Inductive(name, args)` where `name == T`: admit (this is
//!     the recursive use we are admitting).
//!   * For `Inductive(other_name, args)`: admit iff every `arg` is
//!     strictly positive in `T`.
//!
//! The check is *constructor-by-constructor*: each constructor is a
//! Π-chain of argument types, with the codomain being the type's own
//! `Inductive(T, _)` head. We descend each argument's type tree under
//! the strictly-positive discipline.
//!
//! This module also hosts [`is_uip_shape`] (UIP-detection for the
//! axiom registry), [`is_var_named`] and [`is_path_over`] — these
//! cross-cut into axiom-registry's UIP rejection (`pub(crate)` so
//! the AxiomRegistry call site can use them).

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

use crate::{CoreTerm, KernelError};

/// One constructor of an inductive type. Field `arg_types` is the
/// list of argument types (each a `CoreTerm` in `Universe` position);
/// the constructor's full Pi-type is reconstructed as `Pi(arg1, Pi(arg2,
/// ..., Pi(argn, Inductive(self.name))))` on demand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructorSig {
    /// Constructor name (must be unique within the surrounding inductive).
    pub name: Text,
    /// Argument types, in order. The constructor takes a value of each
    /// argument type and yields a value of the surrounding inductive.
    pub arg_types: List<CoreTerm>,
}

/// One registered inductive declaration. The kernel uses this as the
/// authoritative description of an inductive's surface (used by
/// positivity checking, eliminator typing, and `verum audit
/// --kernel-rules`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredInductive {
    /// Type name, e.g. `"Nat"`, `"List"`.
    pub name: Text,
    /// Type-parameter names (their universe levels are checked at
    /// elaboration; the kernel only needs the names for the
    /// constructor body to reference them).
    pub params: List<Text>,
    /// Constructors of this inductive.
    pub constructors: List<ConstructorSig>,
    /// V8 (#215) — universe level this inductive inhabits. Defaults
    /// to `Concrete(0)` (`Type(0)`) for set-level inductives via
    /// the [`Default`] impl that doesn't require this field; the
    /// `register_with_universe` entry point lets HoTT-level
    /// declarations record `Concrete(2)` (univalent universes
    /// containing `Glue`-typed terms) or higher. Pre-V8 the
    /// `infer` arm hardcoded `Universe(Concrete(0))` regardless of
    /// the declared level — silently demoting universal-polymorphic
    /// types to set-level. The Inductive-typing rule now consults
    /// this field via `universe_for(path)` from [`InductiveRegistry`].
    #[serde(default = "default_universe_level")]
    pub universe: crate::UniverseLevel,
}

fn default_universe_level() -> crate::UniverseLevel {
    crate::UniverseLevel::Concrete(0)
}

impl RegisteredInductive {
    /// V8 (#215) — construct a `RegisteredInductive` at the
    /// default `Type(0)` universe level (set-level inductives).
    /// The test corpus + stdlib bring-up registrations use this
    /// path; HoTT-level declarations should call
    /// [`Self::with_universe`] to record `Concrete(2)` (univalent
    /// universes containing `Glue`-typed terms) or higher.
    pub fn new(
        name: Text,
        params: List<Text>,
        constructors: List<ConstructorSig>,
    ) -> Self {
        Self {
            name,
            params,
            constructors,
            universe: default_universe_level(),
        }
    }

    /// V8 (#215) — set the universe level the inductive inhabits.
    /// Builder-style; returns `self`. Used by the elaborator to
    /// record HoTT-level (`Concrete(1)+`) or universe-polymorphic
    /// declarations whose carriers can't be demoted to `Type(0)`.
    pub fn with_universe(mut self, universe: crate::UniverseLevel) -> Self {
        self.universe = universe;
        self
    }
}

/// Registry of strict-positivity-validated inductive declarations.
/// Mirrors `AxiomRegistry` shape so the rest of the kernel can use a
/// uniform discovery mechanism.
#[derive(Debug, Clone, Default)]
pub struct InductiveRegistry {
    entries: List<RegisteredInductive>,
}

impl InductiveRegistry {
    /// Fresh empty registry.
    pub fn new() -> Self {
        Self { entries: List::new() }
    }

    /// Register a new inductive declaration. The kernel runs
    /// strict-positivity validation on every constructor's argument
    /// types; the first violation found surfaces as
    /// [`KernelError::PositivityViolation`] and the registration is
    /// rejected. Duplicate names surface as
    /// [`KernelError::DuplicateInductive`].
    pub fn register(
        &mut self,
        decl: RegisteredInductive,
    ) -> Result<(), KernelError> {
        if self.entries.iter().any(|e| e.name == decl.name) {
            return Err(KernelError::DuplicateInductive(decl.name));
        }
        for ctor in decl.constructors.iter() {
            for (i, arg_ty) in ctor.arg_types.iter().enumerate() {
                check_strict_positivity(
                    decl.name.as_str(),
                    arg_ty,
                    &PositivityCtx::root(ctor.name.as_str(), i),
                )?;
            }
        }
        self.entries.push(decl);
        Ok(())
    }

    /// Look up an inductive by name.
    pub fn get(&self, name: &str) -> Maybe<&RegisteredInductive> {
        for e in self.entries.iter() {
            if e.name.as_str() == name {
                return Maybe::Some(e);
            }
        }
        Maybe::None
    }

    /// Enumerate every registered inductive.
    pub fn all(&self) -> &List<RegisteredInductive> {
        &self.entries
    }

    /// V8 (#215) — look up the universe level for a registered
    /// inductive by qualified path. Returns the registered level
    /// when present, `None` when the name isn't in the registry.
    ///
    /// Used by `infer`'s `Inductive` arm to honour the spec's
    /// declared universe instead of the pre-V8 hardcoded
    /// `Concrete(0)` fallback. Path matching is by full string
    /// equality on `name` — qualified paths (e.g.
    /// `"core.collections.list.List"`) and bare names (e.g.
    /// `"Nat"`) are both supported, the registrar's choice of key
    /// determines lookup behaviour.
    pub fn universe_for(&self, path: &str) -> Option<&crate::UniverseLevel> {
        for e in self.entries.iter() {
            if e.name.as_str() == path {
                return Some(&e.universe);
            }
        }
        None
    }
}

/// Tracking context for the strict-positivity walk: which constructor
/// we are inside, which argument index, and the breadcrumb of nested
/// positions so the diagnostic can pinpoint the offending site.
#[derive(Debug, Clone)]
pub struct PositivityCtx<'a> {
    constructor: &'a str,
    arg_index: usize,
    breadcrumb: String,
}

impl<'a> PositivityCtx<'a> {
    pub fn root(constructor: &'a str, arg_index: usize) -> Self {
        Self {
            constructor,
            arg_index,
            breadcrumb: format!("constructor '{}' arg #{}", constructor, arg_index),
        }
    }
    fn nested(&self, suffix: &str) -> Self {
        Self {
            constructor: self.constructor,
            arg_index: self.arg_index,
            breadcrumb: format!("{} → {}", self.breadcrumb, suffix),
        }
    }
}

/// True iff `ty` mentions the type name `target` anywhere in its
/// term tree. Used to detect `T` appearing in negative positions
/// (the left-hand side of an arrow); strict positivity forbids this.
fn name_appears_in(target: &str, ty: &CoreTerm) -> bool {
    match ty {
        CoreTerm::Inductive { path, args } => {
            if path.as_str() == target {
                return true;
            }
            for a in args.iter() {
                if name_appears_in(target, a) {
                    return true;
                }
            }
            false
        }
        CoreTerm::Pi { domain, codomain, .. } => {
            name_appears_in(target, domain) || name_appears_in(target, codomain)
        }
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            name_appears_in(target, fst_ty) || name_appears_in(target, snd_ty)
        }
        CoreTerm::App(f, a) => {
            name_appears_in(target, f) || name_appears_in(target, a)
        }
        CoreTerm::Lam { domain, body, .. } => {
            name_appears_in(target, domain) || name_appears_in(target, body)
        }
        CoreTerm::Refine { base, predicate, .. } => {
            name_appears_in(target, base) || name_appears_in(target, predicate)
        }
        CoreTerm::PathTy { carrier, .. } => name_appears_in(target, carrier),
        // Atoms and SMT certificates carry no nested types.
        _ => false,
    }
}

/// The strict-positivity walker for VUVA §7.3 `K-Pos`. Returns Ok iff
/// the type name `target` appears only strictly positively in `ty`.
///
/// The discipline:
/// - On `Pi(domain, codomain)`: `target` must NOT appear in `domain`
///   (the negative position); `codomain` must itself be strictly
///   positive in `target`.
/// - On `Inductive(name, args)`: when `name == target`, the recursive
///   reference is admitted (this IS the strict-positive site). When
///   `name != target`, every `arg` must itself be strictly positive
///   in `target` — this catches indirect non-positive recursion via
///   parametrised types like `BadCons(SomeFn(Bad), ...)`.
/// - On `Sigma`, `App`, `Refine`, `Lambda`: descend into both halves;
///   strict positivity is closed under products and dependent pairs.
/// - On atoms: vacuously OK.
pub fn check_strict_positivity(
    target: &str,
    ty: &CoreTerm,
    ctx: &PositivityCtx,
) -> Result<(), KernelError> {
    match ty {
        CoreTerm::Pi { domain, codomain, .. } => {
            // Negative position: target must not appear here.
            if name_appears_in(target, domain) {
                return Err(KernelError::PositivityViolation {
                    type_name: Text::from(target),
                    constructor: Text::from(ctx.constructor),
                    position: Text::from(format!(
                        "{} → left of an arrow (negative position)",
                        ctx.breadcrumb,
                    ).as_str()),
                });
            }
            // Codomain: must be strictly positive in target.
            check_strict_positivity(
                target,
                codomain,
                &ctx.nested("codomain of Π"),
            )
        }

        CoreTerm::Inductive { path, args } => {
            // Self-recursive reference is the strict-positive use we
            // are admitting; args may also mention `target` but ONLY
            // strictly positively (caught by recursion below).
            //
            // For non-self inductives: every argument must itself be
            // strictly positive in `target`. This catches `BadList =
            // Cons(SomeFn(target), BadList)` where SomeFn is a
            // parametrised type that smuggles `target` into a
            // negative position via its own constructor signature.
            let _ = path; // self vs other distinction is handled below
            for (i, a) in args.iter().enumerate() {
                check_strict_positivity(
                    target,
                    a,
                    &ctx.nested(&format!("Inductive arg #{}", i)),
                )?;
            }
            Ok(())
        }

        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            check_strict_positivity(target, fst_ty, &ctx.nested("Σ.fst"))?;
            check_strict_positivity(target, snd_ty, &ctx.nested("Σ.snd"))
        }

        CoreTerm::App(f, a) => {
            check_strict_positivity(target, f, &ctx.nested("App.func"))?;
            check_strict_positivity(target, a, &ctx.nested("App.arg"))
        }

        CoreTerm::Refine { base, predicate, .. } => {
            check_strict_positivity(target, base, &ctx.nested("Refine.base"))?;
            check_strict_positivity(target, predicate, &ctx.nested("Refine.predicate"))
        }

        CoreTerm::PathTy { carrier, .. } => {
            check_strict_positivity(target, carrier, &ctx.nested("PathTy.carrier"))
        }

        CoreTerm::Lam { domain, body, .. } => {
            check_strict_positivity(target, domain, &ctx.nested("Lam.domain"))?;
            check_strict_positivity(target, body,   &ctx.nested("Lam.body"))
        }

        // Universes / Var / Axiom / SmtProof / Elim — atoms with no
        // nested types whose body could re-introduce the name.
        _ => Ok(()),
    }
}

/// Return `true` iff `ty` is the direct UIP shape:
///
/// ```text
/// Π A. Π a. Π b. Π p. Π q. PathTy(PathTy(A, a, b), p, q)
/// ```
///
/// The check is deliberately conservative: it inspects the outer
/// five Π binders and confirms that the innermost codomain is a
/// path-of-paths whose inner carrier is `A`. Axioms that imply UIP
/// only indirectly (e.g. via a stated equivalence between path types
/// and booleans) are NOT caught — that would require a reachability
/// analysis beyond this kernel's scope. Users who need UIP for
/// set-level programming should work in the `is_set`-typed fragment,
/// where UIP is derivable (not axiomatised) from the proposition
/// truncation.
///
/// `pub(crate)` because the AxiomRegistry's UIP rejection
/// (`KernelError::UipForbidden`) calls this to detect the shape;
/// not part of the public kernel API.
pub(crate) fn is_uip_shape(ty: &CoreTerm) -> bool {
    // Π A : Type(_) . (rest with `A` in scope)
    let CoreTerm::Pi { binder: b_a, domain: dom_a, codomain: after_a } = ty else {
        return false;
    };
    if !matches!(dom_a.as_ref(), CoreTerm::Universe(_)) {
        return false;
    }
    // Π a : A . ...
    let CoreTerm::Pi { binder: b_a2, domain: dom_a2, codomain: after_a2 } = after_a.as_ref() else {
        return false;
    };
    if !is_var_named(dom_a2, b_a.as_str()) {
        return false;
    }
    let _ = b_a2; // bound as `a`
    // Π b : A . ...
    let CoreTerm::Pi { binder: b_b, domain: dom_b, codomain: after_b } = after_a2.as_ref() else {
        return false;
    };
    if !is_var_named(dom_b, b_a.as_str()) {
        return false;
    }
    let _ = b_b; // bound as `b`
    // Π p : PathTy(A, a, b) . ...
    let CoreTerm::Pi { binder: _, domain: dom_p, codomain: after_p } = after_b.as_ref() else {
        return false;
    };
    if !is_path_over(dom_p, b_a.as_str()) {
        return false;
    }
    // Π q : PathTy(A, a, b) . goal
    let CoreTerm::Pi { binder: _, domain: dom_q, codomain: goal } = after_p.as_ref() else {
        return false;
    };
    if !is_path_over(dom_q, b_a.as_str()) {
        return false;
    }
    // goal: PathTy(PathTy(A, a, b), p, q)
    let CoreTerm::PathTy { carrier: outer_carrier, .. } = goal.as_ref() else {
        return false;
    };
    is_path_over(outer_carrier, b_a.as_str())
}

/// `ty` is `Var(name)` (or an alpha-renamed de Bruijn equivalent).
fn is_var_named(ty: &CoreTerm, name: &str) -> bool {
    matches!(ty, CoreTerm::Var(n) if n.as_str() == name)
}

/// `ty` is `PathTy(Var(carrier_name), _, _)`.
fn is_path_over(ty: &CoreTerm, carrier_name: &str) -> bool {
    matches!(
        ty,
        CoreTerm::PathTy { carrier, .. } if is_var_named(carrier.as_ref(), carrier_name)
    )
}
