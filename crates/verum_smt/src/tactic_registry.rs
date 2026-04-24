//! Cog-level registry for user-authored tactic packages.
//!
//! This module is the shared lookup surface that binds tactic names
//! at the cog-dependency layer rather than at the compiler-intrinsic
//! layer. The four consumers of the registry:
//!
//! 1. **stdlib** `core.proof.tactics.*` — loaded at session start as
//!    a well-known package with a fixed revision.
//! 2. **User project** — `@tactic` / `@tactic meta fn` declarations
//!    in the project's own `.vr` files register into the project
//!    package.
//! 3. **Imported cogs** — a cog that exports `@tactic` declarations
//!    loads into a package named after the cog's path.
//! 4. **`verum verify`** — looks up tactics by name when the proof
//!    DSL refers to them, producing a clear "unknown tactic in
//!    which package" diagnostic on miss.
//!
//! Semantics
//!
//! Each registered entry is keyed by `(package, name)` and carries
//! a revision tag for version-skew diagnostics. Duplicate
//! registration is rejected — the compiler surfaces it as
//! `E701 duplicate_tactic_registration`. Lookup first searches the
//! current-project package, then imported-cog packages in import
//! order, then the stdlib package — so a project's local tactic
//! always shadows an imported or stdlib tactic with the same name
//! (the shadowing is explicit, not accidental).
//!
//! The registry is deliberately unaware of tactic semantics — it
//! stores opaque `TacticBody` handles that the user_tactic compiler
//! consumes. Decoupling registration from execution lets the
//! stdlib's core.proof.tactics package (task #85) and user
//! packages (this task) share one storage model.

use std::collections::BTreeMap;

use verum_common::{List, Maybe, Text};

/// A registered tactic declaration.
///
/// Carries enough metadata for diagnostics (`package`, `revision`,
/// `source_span` once the parser patch lands) plus the opaque body
/// the executor consumes.
#[derive(Debug, Clone)]
pub struct TacticDecl {
    /// The tactic's short name, as referenced from the proof DSL.
    pub name: Text,
    /// The package the tactic belongs to. Stdlib tactics carry
    /// `"core.proof.tactics"`; user tactics carry the project
    /// cog name; imported tactics carry the exporting cog's path.
    pub package: Text,
    /// Revision tag from the cog manifest — used by
    /// cross-package-boundary diagnostics ("tactic X used at
    /// revision r1 but package Y is revision r2").
    pub revision: Text,
    /// Opaque body. The user_tactic compiler consumes this via the
    /// `compile_tactic` pipeline; the registry does not interpret
    /// it. The body is stored as the source-form
    /// `TacticExpr`-equivalent rendered string — re-parsed on
    /// lookup to avoid a cross-module type dep.
    pub body: Text,
}

/// Scope disambiguator used by the lookup algorithm.
///
/// The enum order determines search order: `Project` first,
/// `ImportedCog` next (iterated in registration order), `Stdlib`
/// last. A project-local tactic always shadows the stdlib or an
/// imported cog tactic with the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageScope {
    Project,
    ImportedCog,
    Stdlib,
}

/// Main registry — cog-level table of tactic declarations.
#[derive(Debug, Default, Clone)]
pub struct TacticPackageRegistry {
    /// `package_name -> (scope, Vec<TacticDecl>)`. BTreeMap
    /// preserves iteration order for deterministic diagnostics.
    entries: BTreeMap<Text, (PackageScope, List<TacticDecl>)>,
    /// Import order of non-stdlib packages — used for shadowing
    /// resolution among imported cogs.
    import_order: List<Text>,
}

impl TacticPackageRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tactic declaration.
    ///
    /// Returns `Err(RegistryError::DuplicateRegistration)` if
    /// `(package, name)` is already bound. Callers that want to
    /// intentionally shadow must place the new entry in a
    /// different-scoped package and rely on the lookup algorithm.
    pub fn register(
        &mut self,
        scope: PackageScope,
        decl: TacticDecl,
    ) -> Result<(), RegistryError> {
        let package = decl.package.clone();
        let entry = self
            .entries
            .entry(package.clone())
            .or_insert_with(|| (scope, List::new()));

        if entry.0 != scope {
            return Err(RegistryError::ScopeConflict {
                package: package.clone(),
                existing: entry.0,
                attempted: scope,
            });
        }

        // Reject duplicate (package, name) pairs.
        if entry
            .1
            .iter()
            .any(|d| d.name.as_str() == decl.name.as_str())
        {
            return Err(RegistryError::DuplicateRegistration {
                package: decl.package.clone(),
                name: decl.name.clone(),
            });
        }

        entry.1.push(decl);

        // Track import order for non-stdlib imported cogs; the
        // stdlib package sits at the end of lookup by construction
        // (its scope bucket is consulted last).
        if matches!(scope, PackageScope::ImportedCog)
            && !self.import_order.iter().any(|p| p.as_str() == package.as_str())
        {
            self.import_order.push(package);
        }

        Ok(())
    }

    /// Look up a tactic by name. Returns the first matching
    /// declaration following the Project → ImportedCog → Stdlib
    /// search order.
    ///
    /// Returns `None` if no package contains the name.
    pub fn lookup(&self, name: &str) -> Maybe<&TacticDecl> {
        // Phase 1: search every Project-scoped package.
        for (_, (scope, decls)) in &self.entries {
            if matches!(scope, PackageScope::Project) {
                for d in decls {
                    if d.name.as_str() == name {
                        return Maybe::Some(d);
                    }
                }
            }
        }

        // Phase 2: search ImportedCog-scoped packages in
        // registration order.
        for package in &self.import_order {
            if let Some((_, decls)) = self.entries.get(package) {
                for d in decls {
                    if d.name.as_str() == name {
                        return Maybe::Some(d);
                    }
                }
            }
        }

        // Phase 3: search Stdlib-scoped packages.
        for (_, (scope, decls)) in &self.entries {
            if matches!(scope, PackageScope::Stdlib) {
                for d in decls {
                    if d.name.as_str() == name {
                        return Maybe::Some(d);
                    }
                }
            }
        }

        Maybe::None
    }

    /// Count of registered tactics across every package.
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .map(|(_, decls)| decls.len())
            .sum()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over (package, scope) tuples — used by the CLI
    /// `verum audit --tactic-packages` report renderer.
    pub fn packages(&self) -> impl Iterator<Item = (&Text, PackageScope)> {
        self.entries.iter().map(|(name, (scope, _))| (name, *scope))
    }
}

/// Errors that registration can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The same `(package, name)` was registered twice. Surface as
    /// `E701 duplicate_tactic_registration` at the CLI renderer.
    DuplicateRegistration { package: Text, name: Text },

    /// The same package was registered under two different scopes
    /// (e.g. once as Project, once as ImportedCog). Usually a
    /// manifest-merge bug; surface as `E702 package_scope_conflict`.
    ScopeConflict {
        package: Text,
        existing: PackageScope,
        attempted: PackageScope,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateRegistration { package, name } => write!(
                f,
                "E701 duplicate tactic registration: `{}` is already \
                 registered in package `{}`",
                name.as_str(),
                package.as_str()
            ),
            Self::ScopeConflict {
                package,
                existing,
                attempted,
            } => write!(
                f,
                "E702 package scope conflict: package `{}` was registered \
                 as {:?}, cannot re-register as {:?}",
                package.as_str(),
                existing,
                attempted
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, package: &str) -> TacticDecl {
        TacticDecl {
            name: Text::from(name),
            package: Text::from(package),
            revision: Text::from("0.1.0"),
            body: Text::from("auto"),
        }
    }

    #[test]
    fn empty_registry_has_no_entries() {
        let r = TacticPackageRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(matches!(r.lookup("any"), Maybe::None));
    }

    #[test]
    fn register_and_lookup_single_entry() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Stdlib, decl("refl", "core.proof.tactics"))
            .unwrap();
        let found = match r.lookup("refl") {
            Maybe::Some(d) => d,
            Maybe::None => panic!("expected Some"),
        };
        assert_eq!(found.name.as_str(), "refl");
        assert_eq!(found.package.as_str(), "core.proof.tactics");
    }

    #[test]
    fn duplicate_registration_returns_error() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Project, decl("my_tac", "my_project"))
            .unwrap();
        let err = r
            .register(PackageScope::Project, decl("my_tac", "my_project"))
            .unwrap_err();
        assert!(matches!(
            err,
            RegistryError::DuplicateRegistration { .. }
        ));
    }

    #[test]
    fn scope_conflict_rejected() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Project, decl("a", "pkg"))
            .unwrap();
        let err = r
            .register(PackageScope::Stdlib, decl("b", "pkg"))
            .unwrap_err();
        assert!(matches!(err, RegistryError::ScopeConflict { .. }));
    }

    #[test]
    fn project_shadows_stdlib() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Stdlib, decl("auto", "core.proof.tactics"))
            .unwrap();
        r.register(PackageScope::Project, decl("auto", "my_project"))
            .unwrap();
        // Project scope wins.
        let found = match r.lookup("auto") {
            Maybe::Some(d) => d,
            Maybe::None => panic!("expected Some"),
        };
        assert_eq!(found.package.as_str(), "my_project");
    }

    #[test]
    fn imported_cog_shadows_stdlib_but_not_project() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Stdlib, decl("auto", "core.proof.tactics"))
            .unwrap();
        r.register(PackageScope::ImportedCog, decl("auto", "third_party"))
            .unwrap();
        // Imported shadows Stdlib:
        let found = match r.lookup("auto") {
            Maybe::Some(d) => d,
            _ => panic!("expected Some"),
        };
        assert_eq!(found.package.as_str(), "third_party");

        // Adding project-scope shadows Imported:
        r.register(PackageScope::Project, decl("auto", "my_project"))
            .unwrap();
        let found = match r.lookup("auto") {
            Maybe::Some(d) => d,
            _ => panic!("expected Some"),
        };
        assert_eq!(found.package.as_str(), "my_project");
    }

    #[test]
    fn imported_cog_search_honours_registration_order() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::ImportedCog, decl("t", "cog_a"))
            .unwrap();
        r.register(PackageScope::ImportedCog, decl("t", "cog_b"))
            .unwrap();
        // Both registered — cog_a first, so lookup returns cog_a.
        let found = match r.lookup("t") {
            Maybe::Some(d) => d,
            _ => panic!(),
        };
        assert_eq!(found.package.as_str(), "cog_a");
    }

    #[test]
    fn unknown_name_returns_none() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Project, decl("foo", "pkg"))
            .unwrap();
        assert!(matches!(r.lookup("bar"), Maybe::None));
    }

    #[test]
    fn packages_iterator_enumerates_every_registered_package() {
        let mut r = TacticPackageRegistry::new();
        r.register(PackageScope::Stdlib, decl("a", "core.proof.tactics"))
            .unwrap();
        r.register(PackageScope::Project, decl("b", "my_project"))
            .unwrap();
        r.register(PackageScope::ImportedCog, decl("c", "imported"))
            .unwrap();
        let pkgs: Vec<(String, PackageScope)> = r
            .packages()
            .map(|(n, s)| (n.as_str().to_string(), s))
            .collect();
        assert_eq!(pkgs.len(), 3);
        assert!(pkgs.iter().any(|(n, s)| n == "core.proof.tactics"
            && *s == PackageScope::Stdlib));
        assert!(pkgs
            .iter()
            .any(|(n, s)| n == "my_project" && *s == PackageScope::Project));
        assert!(pkgs
            .iter()
            .any(|(n, s)| n == "imported" && *s == PackageScope::ImportedCog));
    }
}
