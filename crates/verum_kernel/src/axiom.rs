//! Axiom registry — explicit trusted-axiom set + AST loader.
//!
//! Split per #198. Two complementary surfaces:
//!
//!   • [`AxiomRegistry`] / [`RegisteredAxiom`] — the in-memory
//!     trusted-base set. Every `register` call extends the TCB;
//!     every `all` call enumerates the current boundary so the CLI
//!     and certificate exporters can report exactly which external
//!     results a proof depends on. UIP-shape axioms are syntactically
//!     rejected to preserve cubical-univalence soundness.
//!
//!   • [`load_framework_axioms`] — AST-level loader that scans a
//!     parsed Verum module for `@framework(identifier, "citation")`
//!     attributes on axiom declarations and inserts a
//!     `RegisteredAxiom` for each. Surfaces malformed attribute shapes
//!     as a non-fatal report row so callers can aggregate before
//!     exiting.

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

use crate::{CoreTerm, FrameworkId, KernelError, UniverseLevel};

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
    ///
    /// Also rejects axioms whose statement is structurally equivalent
    /// to **Uniqueness of Identity Proofs** (UIP):
    ///
    /// ```text
    /// Π A. Π (a b : A). Π (p q : PathTy(A, a, b)). PathTy(PathTy(A, a, b), p, q)
    /// ```
    ///
    /// UIP is a statement that any two proofs of the same equality are
    /// themselves equal. It is **incompatible with univalence**: if the
    /// kernel admitted UIP alongside the `ua` axiom and the `Glue` rule,
    /// users could derive `Path<U>(A, B) = Path<U>(A, B) ≡ Refl` for
    /// any `Equiv(A, B)` — collapsing the higher-path structure that
    /// cubical type theory was designed to preserve.
    ///
    /// Detection is syntactic: we look for the exact shape
    /// `Pi A. Pi a. Pi b. Pi p. Pi q. PathTy(PathTy(A, a, b), p, q)`.
    /// More elaborate reductions (axioms that imply UIP transitively)
    /// are out of scope — this check catches the direct case, which
    /// is the common pitfall.
    ///
    /// Corresponds to rule 10 in `docs/verification/trusted-kernel.md`.
    pub fn register(
        &mut self,
        name: Text,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KernelError::DuplicateAxiom(name));
        }
        if crate::inductive::is_uip_shape(&ty) {
            return Err(KernelError::UipForbidden(name));
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
// AST → AxiomRegistry loader
// =============================================================================

/// Scan a parsed Verum module and register every axiom that carries a
/// `@framework(identifier, "citation")` attribute.
///
/// This closes the architectural loop for trusted-boundary declarations:
///
///   1. Source `.vr` file declares `@framework(lurie_htt, "HTT 6.2.2.7")
///      axiom …;`.
///   2. `verum_fast_parser` parses it into an `Item` whose decl carries
///      the attribute in either `Item.attributes` or its
///      `AxiomDecl.attributes` list.
///   3. This loader extracts each `FrameworkAttr` and inserts a
///      `RegisteredAxiom` into the `AxiomRegistry`.
///   4. Any subsequent `infer` call on a `CoreTerm::Axiom { name, .. }`
///      that names one of the loaded axioms succeeds against the
///      registered type.
///
/// Two errors can surface:
///
/// - [`KernelError::DuplicateAxiom`] — two axioms with the same name
///   carried a `@framework(...)` marker.
/// - [`LoadAxiomsReport::malformed`] — a `@framework(...)` attribute
///   was syntactically parsed but had the wrong argument shape
///   (non-identifier first arg, non-string second arg, wrong arg
///   count). This is surfaced in the report rather than aborting,
///   so callers can aggregate all malformations before exiting.
///
/// The axiom type stored in the registry is a placeholder
/// (`CoreTerm::Universe(Concrete(0))`) at this bring-up stage — the
/// compiler's type elaborator is responsible for supplying the real
/// declared type when it calls into the kernel. The registry's
/// purpose here is TCB *attribution* (what framework, what citation),
/// not type storage.
pub fn load_framework_axioms(
    module: &verum_ast::Module,
    registry: &mut AxiomRegistry,
) -> LoadAxiomsReport {
    use verum_ast::attr::FrameworkAttr;
    use verum_ast::decl::ItemKind;

    let mut report = LoadAxiomsReport::default();

    for item in module.items.iter() {
        // Only axiom declarations get auto-registered. Theorems /
        // lemmas / corollaries carry @framework markers too, but
        // they are *consumers* of axioms, not postulates themselves —
        // the elaborator handles their registration once its own
        // proof-term is emitted.
        let (name, decl_attrs) = match &item.kind {
            ItemKind::Axiom(decl) => (decl.name.name.clone(), &decl.attributes),
            _ => continue,
        };

        // Walk both the outer Item.attributes and the inner decl
        // attributes — the parser can place the marker on either.
        let mut found: Maybe<FrameworkAttr> = Maybe::None;
        for attrs in [&item.attributes, decl_attrs] {
            for attr in attrs.iter() {
                if !attr.is_named("framework") {
                    continue;
                }
                match FrameworkAttr::from_attribute(attr) {
                    Maybe::Some(fw) => {
                        if matches!(found, Maybe::None) {
                            found = Maybe::Some(fw);
                        }
                    }
                    Maybe::None => {
                        report.malformed.push(name.clone());
                    }
                }
            }
        }

        if let Maybe::Some(fw) = found {
            let framework = FrameworkId {
                framework: fw.name.clone(),
                citation: fw.citation.clone(),
            };
            // Placeholder type at bring-up — the elaborator supplies
            // the real declared type when it submits the proof term.
            let placeholder_ty = CoreTerm::Universe(UniverseLevel::Concrete(0));
            match registry.register(name.clone(), placeholder_ty, framework) {
                Ok(()) => report.registered.push(name),
                Err(KernelError::DuplicateAxiom(n)) => {
                    report.duplicates.push(n);
                }
                Err(_) => {
                    // Register only returns DuplicateAxiom today;
                    // other error branches are defensive for when the
                    // register API grows.
                    report.malformed.push(name);
                }
            }
        }
    }

    report
}

/// Outcome of [`load_framework_axioms`]. Returned by value so callers
/// can aggregate across multiple modules before reporting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadAxiomsReport {
    /// Axiom names successfully inserted into the registry.
    pub registered: List<Text>,
    /// Axiom names that were already in the registry.
    pub duplicates: List<Text>,
    /// Axiom names whose `@framework(...)` attribute had a
    /// malformed argument shape (wrong arg count, non-identifier
    /// first arg, non-string second arg).
    pub malformed: List<Text>,
}

impl LoadAxiomsReport {
    /// Did the load complete with no errors at all?
    pub fn is_clean(&self) -> bool {
        self.duplicates.is_empty() && self.malformed.is_empty()
    }
}
