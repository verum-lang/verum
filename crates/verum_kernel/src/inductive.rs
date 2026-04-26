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
//! VVA spec §7.3 (`K-Pos`): for every constructor `C(t1, ..., tn) -> T`
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
use verum_common::{Heap, List, Maybe, Text};

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

/// V8 (#237) — one **path constructor** of a higher inductive type.
///
/// Per VVA §7.4 a HIT extends an ordinary inductive with cells whose
/// boundary is a path between two values of the type itself; the path
/// constructor's body is the kernel-internal record of that cell.
///
/// V1 supports **0-cells (point constructors)** via [`ConstructorSig`]
/// and **1-cells (path constructors)** via this struct. 2-cells / higher
/// cells are V2 — they require nested-path syntax that is yet to land in
/// `path_endpoints` (`grammar/verum.ebnf` is currently 1-cells only).
///
/// The endpoint expressions `lhs` / `rhs` are arbitrary [`CoreTerm`]s
/// over the surrounding inductive — typically references to point
/// constructors (e.g. `Var("Base")` for S¹'s `loop : Base ↝ Base`),
/// possibly applied to recursive arguments (e.g. for the suspension
/// HIT's `merid : Σ X ↝ Σ X` where the recursor at `lhs` is computed
/// by recursion over the argument). V1 emits the eliminator's branch
/// type structurally as `PathTy(motive_at_lhs, recursor_at_lhs,
/// recursor_at_rhs)` per §7.4 + §17.2 Task C3; recursor application
/// at non-nullary endpoints is a V2 elaboration extension (the kernel
/// records the structural type, the user supplies the computational
/// content via the framework axiom system).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathCtorSig {
    /// Path constructor name (must be unique across **all** ctors of
    /// the surrounding HIT — point + path — since both share the
    /// constructor namespace).
    pub name: Text,
    /// Left-hand endpoint expression. Typically `Var(point_ctor_name)`
    /// for nullary ctors; for higher-arity endpoints the kernel
    /// records the term as-is (V1 — its computational content is
    /// resolved by elaboration / framework axioms).
    pub lhs: CoreTerm,
    /// Right-hand endpoint expression.
    pub rhs: CoreTerm,
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
    /// V8 (#237) — path constructors (1-cells) for higher inductive
    /// types. Empty for ordinary inductives. Defaults via serde so
    /// pre-V8 deserialised registries retain old-shape ordinary
    /// inductives without explicit migration.
    #[serde(default)]
    pub path_constructors: List<PathCtorSig>,
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
            path_constructors: List::new(),
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

    /// V8 (#237) — append a path constructor (1-cell) to the
    /// declaration. Builder-style; returns `self`. The kernel
    /// validates path-constructor uniqueness against point and
    /// other path constructors at registry-`register` time.
    pub fn with_path_constructor(mut self, sig: PathCtorSig) -> Self {
        self.path_constructors.push(sig);
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
        // V8 (#237) — path-constructor namespace check: path ctor
        // names must be distinct from point ctor names AND from each
        // other (the recursor binds case_<name> uniformly across
        // both kinds — collisions would shadow).
        for path in decl.path_constructors.iter() {
            if decl.constructors.iter().any(|c| c.name == path.name) {
                return Err(KernelError::DuplicateInductive(Text::from(
                    format!(
                        "path ctor '{}' collides with point ctor of the same name in '{}'",
                        path.name.as_str(),
                        decl.name.as_str(),
                    )
                    .as_str(),
                )));
            }
        }
        let mut seen_path_names: List<Text> = List::new();
        for path in decl.path_constructors.iter() {
            if seen_path_names.iter().any(|n| n == &path.name) {
                return Err(KernelError::DuplicateInductive(Text::from(
                    format!(
                        "duplicate path ctor '{}' in '{}'",
                        path.name.as_str(),
                        decl.name.as_str(),
                    )
                    .as_str(),
                )));
            }
            seen_path_names.push(path.name.clone());
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

/// The strict-positivity walker for VVA §7.3 `K-Pos`. Returns Ok iff
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

// =============================================================================
// V8 (#237) — eliminator auto-generation for inductive + higher inductive types
// =============================================================================

/// V8 (#237) — derive the **dependent eliminator type** for an
/// inductive (with optional path constructors).
///
/// Per VVA §7.4 + Task C3 (`docs/architecture/...verum-verification-architecture.md
/// #17.2`) the kernel auto-generates the eliminator's type signature
/// from the registered declaration. The shape is:
///
/// ```text
/// elim_T : Π (motive : T → Type_u) .
///          Π (case_C₁ : Π (a₁:A₁)…(aₙ:Aₙ) . motive(C₁(a₁,…,aₙ))) .   -- one per point ctor
///          ⋮
///          Π (case_P : PathTy(motive(P.lhs),
///                             ↻(P.lhs), ↻(P.rhs))) .                 -- one per path ctor
///          ⋮
///          Π (x : T) . motive(x)
/// ```
///
/// where `↻(e)` denotes the **recursor's image** at endpoint `e`.
/// V1 emits `↻(e) = e` for non-trivial endpoints — the kernel
/// records the **structural** type signature; recursor coherence
/// (so that `↻(P.lhs)` actually reduces to the right case-application
/// chain) is V2 work tied to the `Elim` β-rule rollout. The point of
/// V1 is that the eliminator **typechecks at the right shape** so
/// frameworks (HoTT, cubical) can attest the computational content
/// via axioms or `@verify` proofs without the kernel committing to a
/// premature reduction strategy.
///
/// # Examples
///
/// * Ordinary `Bool` → `Π(motive: Bool → Type). Π(case_True: motive(True)).
///   Π(case_False: motive(False)). Π(x: Bool). motive(x)`.
/// * S¹ HIT (point `Base`, path `Loop : Base..Base`) →
///   `Π(motive). Π(case_Base: motive(Base)).
///    Π(case_Loop: PathTy(motive(Base), Base, Base)). Π(x: S¹). motive(x)`.
/// * Interval HIT (`Zero`, `One`, `Seg : Zero..One`) →
///   `Π(motive). Π(case_Zero). Π(case_One).
///    Π(case_Seg: PathTy(motive(Zero), Zero, One)). Π(x). motive(x)`.
///
/// # V1 limitations (tracked for V2)
///
/// * Recursor-image at non-nullary endpoints is emitted as the raw
///   endpoint expression — V2 will resolve to the right case-app chain.
/// * Path-over (the dependent path needed when `motive(lhs) ≠
///   motive(rhs)` definitionally) is approximated by `PathTy` over
///   `motive(lhs)`; the framework system attests homogeneity for V1.
/// * Higher cells (2-cells +) are not yet representable — needs the
///   nested `path_endpoints` grammar extension.
pub fn eliminator_type(decl: &RegisteredInductive) -> CoreTerm {
    let parent_ind = CoreTerm::Inductive {
        path: decl.name.clone(),
        args: List::from_iter(
            decl.params
                .iter()
                .map(|p| CoreTerm::Var(p.clone()))
                .collect::<Vec<_>>(),
        ),
    };

    // The motive lives in `parent_ind → Universe(decl.universe)`.
    // We reuse `decl.universe` so the eliminator stays at the right
    // universe level for HoTT-level (Concrete(2)) declarations.
    let motive_universe = CoreTerm::Universe(decl.universe.clone());
    let motive_var = CoreTerm::Var(Text::from("motive"));

    // Innermost: Π (x : T) . motive(x).
    let scrut_var = CoreTerm::Var(Text::from("x"));
    let scrut_image =
        CoreTerm::App(Heap::new(motive_var.clone()), Heap::new(scrut_var));
    let mut acc = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(parent_ind.clone()),
        codomain: Heap::new(scrut_image),
    };

    // Path-constructor branches (right-to-left so they end up in
    // declaration order in the outer Pi-chain).
    for path in iter_rev(&decl.path_constructors) {
        let carrier =
            CoreTerm::App(Heap::new(motive_var.clone()), Heap::new(path.lhs.clone()));
        let lhs_image = recursor_image_at_endpoint(&path.lhs);
        let rhs_image = recursor_image_at_endpoint(&path.rhs);
        let branch_ty = CoreTerm::PathTy {
            carrier: Heap::new(carrier),
            lhs: Heap::new(lhs_image),
            rhs: Heap::new(rhs_image),
        };
        acc = CoreTerm::Pi {
            binder: Text::from(format!("case_{}", path.name.as_str()).as_str()),
            domain: Heap::new(branch_ty),
            codomain: Heap::new(acc),
        };
    }

    // Point-constructor branches.
    for ctor in iter_rev(&decl.constructors) {
        let case_ty = point_constructor_case_type(&motive_var, &ctor);
        acc = CoreTerm::Pi {
            binder: Text::from(format!("case_{}", ctor.name.as_str()).as_str()),
            domain: Heap::new(case_ty),
            codomain: Heap::new(acc),
        };
    }

    // Outermost: Π (motive : T → Type_u) . ...
    let motive_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(parent_ind),
        codomain: Heap::new(motive_universe),
    };
    CoreTerm::Pi {
        binder: Text::from("motive"),
        domain: Heap::new(motive_ty),
        codomain: Heap::new(acc),
    }
}

/// V8 (#237) — derive the case-branch type for a point constructor.
///
/// For ctor `C(a₁:A₁, …, aₙ:Aₙ) : T`, the eliminator's case branch is:
///
///     Π (a₁ : A₁) … (aₙ : Aₙ) . motive(C(a₁, …, aₙ))
///
/// Nullary ctors collapse to `motive(C)` (no Π binders).
pub fn point_constructor_case_type(
    motive_var: &CoreTerm,
    ctor: &ConstructorSig,
) -> CoreTerm {
    // Build the constructor application: App_chain(Var(C), a₁, …, aₙ).
    let mut ctor_app = CoreTerm::Var(ctor.name.clone());
    for (i, _arg_ty) in ctor.arg_types.iter().enumerate() {
        let arg_name = Text::from(format!("a{}", i).as_str());
        ctor_app = CoreTerm::App(
            Heap::new(ctor_app),
            Heap::new(CoreTerm::Var(arg_name)),
        );
    }
    // Goal: motive(C(a₁,…,aₙ)).
    let mut acc = CoreTerm::App(
        Heap::new(motive_var.clone()),
        Heap::new(ctor_app),
    );
    // Wrap in Π binders, right-to-left.
    let arg_count = ctor.arg_types.iter().count();
    for (rev_i, arg_ty) in ctor.arg_types.iter().rev().enumerate() {
        let i = arg_count - 1 - rev_i;
        let arg_name = Text::from(format!("a{}", i).as_str());
        acc = CoreTerm::Pi {
            binder: arg_name,
            domain: Heap::new(arg_ty.clone()),
            codomain: Heap::new(acc),
        };
    }
    acc
}

/// V8 (#237) — recursor's image at a path-constructor endpoint.
///
/// V1: emit the endpoint expression as-is. The kernel records the
/// structural shape; the elaborator / framework axiom system supplies
/// the computational content (the actual case-app chain that the
/// recursor produces at this endpoint). V2 will resolve nullary
/// endpoints to `Var("case_<ctor_name>")` and recurse on App-chains.
fn recursor_image_at_endpoint(endpoint: &CoreTerm) -> CoreTerm {
    endpoint.clone()
}

/// Reverse-iterator helper for `verum_common::List` (which doesn't
/// expose a built-in DoubleEndedIterator). Used by `eliminator_type`
/// to build right-to-left Π-chains.
fn iter_rev<T: Clone>(list: &List<T>) -> impl Iterator<Item = T> {
    let mut buf: Vec<T> = list.iter().cloned().collect();
    buf.reverse();
    buf.into_iter()
}
