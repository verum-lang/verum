//! PubGrub-based dependency resolver (P4.1).
//!
//! Wraps the [`pubgrub`](https://docs.rs/pubgrub) crate's
//! conflict-driven clause-learning solver in an adapter that takes
//! the existing Verum cog registry's data model (cog name +
//! `semver::Version` + `semver::VersionReq`) and returns either a
//! resolved dependency graph or a structured [`ResolverError`]
//! (cf. `super::resolver_errors`, P4.3).
//!
//! # Why PubGrub
//!
//! PubGrub's strengths over the existing `sat_resolver` (DPLL-based,
//! Verum's first-pass implementation):
//!
//! 1. **Linear-time best-case** — DPLL's worst case is exponential
//!    in the number of variables; PubGrub's incompatibility-driven
//!    backtracking with conflict-clause learning visits each
//!    "interesting" set of versions at most once.
//! 2. **Better diagnostics** — when resolution fails, the algorithm
//!    produces a *human-readable derivation* of the conflict ("A
//!    requires B ≥ 2; C requires B < 2; therefore A and C are
//!    incompatible") that maps cleanly onto our P4.3
//!    [`ResolverError::VersionConflict`] requirement-chain shape.
//! 3. **Battle-tested** — the upstream crate underpins Cargo's
//!    in-progress migration, uv (Astral's Python package manager),
//!    and Dart's `pub` resolver. Vetted constraint semantics, bug
//!    fixes upstreamed.
//!
//! # Surface
//!
//! Two entry points:
//!
//!   [`PubGrubBuilder`] — collect the dependency database in-memory,
//!   then resolve.
//!
//!   [`resolve`] — given a builder, run PubGrub and return the
//!   selected versions or a [`ResolverError`].
//!
//! The existing legacy `DependencyResolver` (graph-walking SAT) is
//! preserved alongside; this module is a parallel, opt-in path.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;

use pubgrub::{
    resolve as pubgrub_resolve, DependencyProvider, OfflineDependencyProvider,
    PackageResolutionStatistics, PubGrubError, Ranges,
};
use semver::{Version, VersionReq};

use super::resolver_errors::{RequirementOrigin, RequirerSpec, ResolverError};

/// Cog name. Newtype around `String` so PubGrub's `Package` trait
/// (which requires `Eq + Hash + Clone + Debug + Display`) is
/// satisfied by the standard derives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CogName(pub String);

impl std::fmt::Display for CogName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<S: Into<String>> From<S> for CogName {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

/// Version-requirement bridge: PubGrub's [`Ranges`] over
/// `semver::Version` is the natural shape, but mapping
/// `semver::VersionReq` (Cargo-style operator strings, e.g. `^1.2`,
/// `>=2, <3`) onto a `Ranges<Version>` requires evaluating the req
/// against every concrete candidate version.
///
/// The [`PubGrubBuilder`] collects every `(name, version)` pair the
/// caller knows about, then materialises each `(name, req)` edge as
/// a finite [`Ranges<Version>`] — the union of single-version
/// "exact" ranges for every candidate that satisfies `req`. This is
/// finite and exact for our use case: we never resolve against an
/// unbounded version stream.
#[derive(Debug, Clone)]
pub struct DepEdge {
    pub name: CogName,
    pub requirement: VersionReq,
}

/// Database of `(name, version) → [direct deps]` plus the universe
/// of versions known per package. Built incrementally; resolve at
/// the end.
#[derive(Debug, Default)]
pub struct PubGrubBuilder {
    /// Every version the resolver may consider, per package.
    versions: BTreeMap<CogName, Vec<Version>>,
    /// Direct deps of `(package, version)`.
    edges: HashMap<(CogName, Version), Vec<DepEdge>>,
}

impl PubGrubBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a candidate `version` of `package`. Idempotent.
    pub fn add_package_version(&mut self, package: impl Into<CogName>, version: Version) {
        let name = package.into();
        let versions = self.versions.entry(name).or_default();
        if !versions.iter().any(|v| v == &version) {
            versions.push(version);
        }
    }

    /// Declare that `package@version` directly requires `dep_name`
    /// matching `dep_req`. The caller must also have registered
    /// `(dep_name, every_candidate)` via [`add_package_version`] —
    /// the resolver only considers registered versions.
    pub fn add_dependency(
        &mut self,
        package: impl Into<CogName>,
        version: Version,
        dep_name: impl Into<CogName>,
        dep_req: VersionReq,
    ) {
        let key = (package.into(), version);
        self.edges.entry(key).or_default().push(DepEdge {
            name: dep_name.into(),
            requirement: dep_req,
        });
    }

    /// All registered versions of `package`, sorted descending
    /// (latest first). Used by `choose_version` so the resolver
    /// prefers the most recent compatible version.
    pub fn versions_descending(&self, package: &CogName) -> Vec<Version> {
        let mut out = self
            .versions
            .get(package)
            .cloned()
            .unwrap_or_default();
        out.sort();
        out.reverse();
        out
    }
}

/// One resolved package version in the final solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: Version,
}

/// Feed [`PubGrubBuilder`] into PubGrub and return the resolved
/// version assignment, or a structured [`ResolverError`] mapped from
/// the underlying [`PubGrubError`]. Failures preserve the requirement
/// chain when the failure mode is `VersionConflict` (PubGrub's
/// `NoSolution` derivation), so the existing P4.3 diagnostic
/// renderer produces actionable output without further glue.
pub fn resolve(
    builder: &PubGrubBuilder,
    root_package: impl Into<CogName>,
    root_version: Version,
) -> Result<Vec<ResolvedPackage>, ResolverError> {
    let provider = VerumProvider::new(builder);
    let root = root_package.into();
    let root_for_solver = root.clone();

    match pubgrub_resolve(&provider, root_for_solver, root_version) {
        Ok(selected) => {
            let mut out: Vec<ResolvedPackage> = selected
                .iter()
                .map(|(name, version)| ResolvedPackage {
                    name: name.0.clone(),
                    version: version.clone(),
                })
                .collect();
            out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
            Ok(out)
        }
        Err(err) => Err(map_pubgrub_error(err, &provider)),
    }
}

// ── DependencyProvider impl ─────────────────────────────────────────

/// Internal adapter implementing PubGrub's [`DependencyProvider`].
///
/// Reuses the [`pubgrub::OfflineDependencyProvider`] as the actual
/// provider — it already implements every trait method correctly
/// for the case where the dependency graph is fully known up front.
/// Our [`PubGrubBuilder`] populates one of these on demand.
struct VerumProvider {
    inner: OfflineDependencyProvider<CogName, Ranges<Version>>,
    /// Saved for diagnostic reconstruction in `map_pubgrub_error`.
    edges: HashMap<(CogName, Version), Vec<DepEdge>>,
    available_versions: BTreeMap<CogName, Vec<Version>>,
}

impl VerumProvider {
    fn new(builder: &PubGrubBuilder) -> Self {
        let mut inner: OfflineDependencyProvider<CogName, Ranges<Version>> =
            OfflineDependencyProvider::new();

        // Register every (package, version) — `add_dependencies` adds
        // both the package version itself (with no deps if absent
        // from `edges`) and any declared edges.
        for (name, versions) in &builder.versions {
            for version in versions {
                let edges_for = builder
                    .edges
                    .get(&(name.clone(), version.clone()))
                    .cloned()
                    .unwrap_or_default();
                let pubgrub_deps: Vec<(CogName, Ranges<Version>)> = edges_for
                    .into_iter()
                    .map(|edge| {
                        let candidates = builder.versions.get(&edge.name).cloned().unwrap_or_default();
                        let range = req_to_ranges(&edge.requirement, &candidates);
                        (edge.name, range)
                    })
                    .collect();
                inner.add_dependencies(name.clone(), version.clone(), pubgrub_deps);
            }
        }

        Self {
            inner,
            edges: builder.edges.clone(),
            available_versions: builder.versions.clone(),
        }
    }

    /// Read access to the dependency edges for diagnostic mapping.
    fn edges_of(&self, package: &CogName, version: &Version) -> Vec<DepEdge> {
        self.edges
            .get(&(package.clone(), version.clone()))
            .cloned()
            .unwrap_or_default()
    }

    /// All known versions of `package`, sorted ascending.
    fn versions_of(&self, package: &CogName) -> Vec<Version> {
        self.available_versions
            .get(package)
            .cloned()
            .unwrap_or_default()
    }
}

impl DependencyProvider for VerumProvider {
    type P = CogName;
    type V = Version;
    type VS = Ranges<Version>;
    type Priority = (u32, u32);
    type M = String;
    type Err = Infallible;

    fn prioritize(
        &self,
        package: &Self::P,
        range: &Self::VS,
        stats: &PackageResolutionStatistics,
    ) -> Self::Priority {
        // Priority key = (conflict_count, candidate_count).
        // PubGrub picks the *highest* priority first; we encode
        // "fewer candidates / more conflicts" → higher u32 to drive
        // the resolver toward the most-constrained variable first
        // (classic CDCL heuristic).
        let candidates = self.versions_of(package).len();
        let in_range = self
            .versions_of(package)
            .iter()
            .filter(|v| range.contains(v))
            .count();
        let inverse_count = (candidates.saturating_sub(in_range)) as u32;
        (stats.conflict_count(), inverse_count)
    }

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        // Pick highest version in range.
        let mut versions = self.versions_of(package);
        versions.sort();
        Ok(versions.into_iter().rev().find(|v| range.contains(v)))
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<pubgrub::Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        self.inner.get_dependencies(package, version)
    }
}

/// Convert a `semver::VersionReq` into a `Ranges<Version>` over the
/// finite candidate set. We materialise the satisfying set
/// explicitly: each accepted version becomes a singleton range, and
/// the result is the union of those singletons. This is exact for
/// our resolver because we only ever consider registered versions
/// (no "live registry" stream of unknown versions).
fn req_to_ranges(req: &VersionReq, candidates: &[Version]) -> Ranges<Version> {
    let matching: Vec<&Version> = candidates.iter().filter(|v| req.matches(v)).collect();
    if matching.is_empty() {
        return Ranges::empty();
    }
    let mut acc = Ranges::singleton(matching[0].clone());
    for v in matching.iter().skip(1) {
        acc = acc.union(&Ranges::singleton((*v).clone()));
    }
    acc
}

// ── Error mapping ──────────────────────────────────────────────────

fn map_pubgrub_error(
    err: PubGrubError<VerumProvider>,
    provider: &VerumProvider,
) -> ResolverError {
    match err {
        PubGrubError::NoSolution(derivation) => map_no_solution(&derivation, provider),
        PubGrubError::ErrorRetrievingDependencies { package, version, .. } => {
            ResolverError::no_matching_version(
                package.0.clone(),
                version.to_string(),
                provider
                    .versions_of(&package)
                    .iter()
                    .map(|v| v.to_string())
                    .collect(),
            )
        }
        PubGrubError::ErrorChoosingVersion { package, .. } => {
            ResolverError::no_matching_version(
                package.0.clone(),
                "<resolver-internal>".to_string(),
                provider
                    .versions_of(&package)
                    .iter()
                    .map(|v| v.to_string())
                    .collect(),
            )
        }
        PubGrubError::ErrorInShouldCancel(_) => {
            ResolverError::cycle(vec!["<resolver cancelled>".to_string()])
        }
    }
}

/// Map PubGrub's NoSolution derivation into a P4.3 VersionConflict.
///
/// The full PubGrub derivation is a tree of incompatibilities; for
/// the typed-error surface we summarise it to the (most recent)
/// conflicted package and the requirement chain that touched it.
fn map_no_solution(
    derivation: &pubgrub::DerivationTree<CogName, Ranges<Version>, String>,
    provider: &VerumProvider,
) -> ResolverError {
    // Collect every (requirer, requirement) pair on every package
    // mentioned in the derivation. The derivation tree's leaves are
    // the original "from external" incompatibilities (these encode
    // the user's manifest declarations).
    let mut packages: Vec<CogName> = Vec::new();
    collect_packages(derivation, &mut packages);
    packages.sort();
    packages.dedup();

    if packages.is_empty() {
        return ResolverError::cycle(vec!["<no-solution: empty derivation>".to_string()]);
    }

    // Pick the actual conflicted package: the one with ≥ 2 distinct
    // version requirements declared on it across the dependency
    // graph. The simple "most-mentioned in derivation" heuristic
    // picks the root every time (root is in every leaf path), which
    // is rarely the actually-conflicted package.
    let mut requirement_count: HashMap<CogName, usize> = HashMap::new();
    for ((_, _), edges) in &provider.edges {
        for edge in edges {
            *requirement_count.entry(edge.name.clone()).or_insert(0) += 1;
        }
    }
    let conflict_pkg = requirement_count
        .iter()
        .filter(|&(_, c)| *c >= 2)
        // Among packages with ≥ 2 reqs, prefer one mentioned in the
        // derivation (which means PubGrub touched it during solving).
        .max_by_key(|&(p, c)| (derivation_mentions(derivation, p), *c))
        .map(|(p, _)| p.clone())
        // Fallback: the most-mentioned package in the derivation
        // tree (excluding the root if possible).
        .or_else(|| {
            let mut candidates: Vec<&CogName> = packages.iter().collect();
            candidates.sort_by_key(|p| std::cmp::Reverse(derivation_mentions(derivation, p)));
            candidates.into_iter().next().cloned()
        })
        .unwrap_or_else(|| packages[0].clone());

    // For each candidate version of conflict_pkg, collect every
    // (requirer, requirement) edge. This is the chain we surface.
    let mut requirements: Vec<RequirementOrigin> = Vec::new();
    for ((req_pkg, req_ver), edges) in &provider.edges {
        for edge in edges {
            if edge.name == conflict_pkg {
                requirements.push(RequirementOrigin {
                    requirer: Some(RequirerSpec {
                        name: req_pkg.0.clone(),
                        version: req_ver.to_string(),
                    }),
                    requirement: edge.requirement.to_string(),
                });
            }
        }
    }
    requirements.sort_by(|a, b| {
        let key = |o: &RequirementOrigin| match &o.requirer {
            Some(r) => (r.name.clone(), r.version.clone(), o.requirement.clone()),
            None => (String::new(), String::new(), o.requirement.clone()),
        };
        key(a).cmp(&key(b))
    });
    requirements.dedup();

    let present_versions: Vec<String> = provider
        .versions_of(&conflict_pkg)
        .iter()
        .map(|v| v.to_string())
        .collect();

    ResolverError::version_conflict(conflict_pkg.0, requirements, present_versions)
}

fn collect_packages(
    tree: &pubgrub::DerivationTree<CogName, Ranges<Version>, String>,
    out: &mut Vec<CogName>,
) {
    use pubgrub::DerivationTree;
    match tree {
        DerivationTree::External(ext) => {
            collect_packages_external(ext, out);
        }
        DerivationTree::Derived(derived) => {
            for (p, _) in derived.terms.iter() {
                out.push(p.clone());
            }
            collect_packages(&derived.cause1, out);
            collect_packages(&derived.cause2, out);
        }
    }
}

fn collect_packages_external(
    ext: &pubgrub::External<CogName, Ranges<Version>, String>,
    out: &mut Vec<CogName>,
) {
    use pubgrub::External;
    match ext {
        External::NotRoot(p, _) => out.push(p.clone()),
        External::NoVersions(p, _) => out.push(p.clone()),
        External::FromDependencyOf(p, _, dep, _) => {
            out.push(p.clone());
            out.push(dep.clone());
        }
        External::Custom(p, _, _) => out.push(p.clone()),
    }
}

fn derivation_mentions(
    tree: &pubgrub::DerivationTree<CogName, Ranges<Version>, String>,
    target: &CogName,
) -> usize {
    use pubgrub::DerivationTree;
    let mut hits = 0;
    match tree {
        DerivationTree::External(ext) => {
            hits += external_mentions(ext, target);
        }
        DerivationTree::Derived(derived) => {
            for (p, _) in derived.terms.iter() {
                if p == target {
                    hits += 1;
                }
            }
            hits += derivation_mentions(&derived.cause1, target);
            hits += derivation_mentions(&derived.cause2, target);
        }
    }
    hits
}

fn external_mentions(
    ext: &pubgrub::External<CogName, Ranges<Version>, String>,
    target: &CogName,
) -> usize {
    use pubgrub::External;
    let mut hits = 0;
    match ext {
        External::NotRoot(p, _) => {
            if p == target {
                hits += 1;
            }
        }
        External::NoVersions(p, _) => {
            if p == target {
                hits += 1;
            }
        }
        External::FromDependencyOf(p, _, dep, _) => {
            if p == target {
                hits += 1;
            }
            if dep == target {
                hits += 1;
            }
        }
        External::Custom(p, _, _) => {
            if p == target {
                hits += 1;
            }
        }
    }
    hits
}

// `RefCell` is unused publicly but keeps lifetimes happy in
// dependency-provider mutation paths if we ever extend with
// dynamically-fetched candidates.
#[allow(dead_code)]
fn _unused_marker(_x: RefCell<()>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ver(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn req(s: &str) -> VersionReq {
        VersionReq::parse(s).unwrap()
    }

    fn build_basic() -> PubGrubBuilder {
        // Universe:
        //   root @ 1.0.0  →  json ^1
        //   json @ 1.0.0
        //   json @ 1.4.0
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        b.add_package_version("json", ver("1.0.0"));
        b.add_package_version("json", ver("1.4.0"));
        b.add_dependency("root", ver("1.0.0"), "json", req("^1"));
        b
    }

    #[test]
    fn resolve_simple_picks_highest_version() {
        let b = build_basic();
        let solution = resolve(&b, "root", ver("1.0.0")).unwrap();
        let json = solution.iter().find(|p| p.name == "json").unwrap();
        assert_eq!(json.version, ver("1.4.0"));
    }

    #[test]
    fn resolve_with_no_compatible_version_yields_error() {
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        b.add_package_version("widget", ver("1.0.0"));
        b.add_dependency("root", ver("1.0.0"), "widget", req("^99"));
        let err = resolve(&b, "root", ver("1.0.0")).unwrap_err();
        // Either VersionConflict or NoMatchingVersion, depending on
        // PubGrub derivation shape. Both carry diagnostic info.
        assert!(
            matches!(
                err,
                ResolverError::VersionConflict { .. } | ResolverError::NoMatchingVersion { .. }
            ),
            "expected typed conflict / no-match, got {err:?}"
        );
    }

    #[test]
    fn resolve_dependency_chain() {
        // root → a → b
        // We expect both a and b in the solution.
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        b.add_package_version("a", ver("1.0.0"));
        b.add_package_version("b", ver("1.0.0"));
        b.add_dependency("root", ver("1.0.0"), "a", req("^1"));
        b.add_dependency("a", ver("1.0.0"), "b", req("^1"));
        let solution = resolve(&b, "root", ver("1.0.0")).unwrap();
        assert!(solution.iter().any(|p| p.name == "a"));
        assert!(solution.iter().any(|p| p.name == "b"));
    }

    #[test]
    fn resolve_diamond_picks_compatible_version() {
        // root → a → c ^1
        // root → b → c ^1
        // Both branches converge on the highest c that satisfies ^1.
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        b.add_package_version("a", ver("1.0.0"));
        b.add_package_version("b", ver("1.0.0"));
        b.add_package_version("c", ver("1.0.0"));
        b.add_package_version("c", ver("1.5.0"));
        b.add_package_version("c", ver("2.0.0"));
        b.add_dependency("root", ver("1.0.0"), "a", req("^1"));
        b.add_dependency("root", ver("1.0.0"), "b", req("^1"));
        b.add_dependency("a", ver("1.0.0"), "c", req("^1"));
        b.add_dependency("b", ver("1.0.0"), "c", req("^1"));
        let solution = resolve(&b, "root", ver("1.0.0")).unwrap();
        let c = solution.iter().find(|p| p.name == "c").unwrap();
        assert_eq!(c.version, ver("1.5.0"));
    }

    #[test]
    fn resolve_diamond_conflict_surfaces_error() {
        // root → a 1.0 → c ^1
        // root → b 1.0 → c ^2
        // Only c=1.0.0 and c=2.0.0 exist. No version satisfies BOTH
        // ^1 and ^2.
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        b.add_package_version("a", ver("1.0.0"));
        b.add_package_version("b", ver("1.0.0"));
        b.add_package_version("c", ver("1.0.0"));
        b.add_package_version("c", ver("2.0.0"));
        b.add_dependency("root", ver("1.0.0"), "a", req("^1"));
        b.add_dependency("root", ver("1.0.0"), "b", req("^1"));
        b.add_dependency("a", ver("1.0.0"), "c", req("^1"));
        b.add_dependency("b", ver("1.0.0"), "c", req("^2"));
        let err = resolve(&b, "root", ver("1.0.0")).unwrap_err();
        match err {
            ResolverError::VersionConflict {
                package,
                requirements,
                ..
            } => {
                assert_eq!(package, "c");
                // Both ^1 and ^2 should appear in the chain.
                let req_strs: Vec<_> = requirements
                    .iter()
                    .map(|o| o.requirement.clone())
                    .collect();
                assert!(req_strs.iter().any(|r| r.contains('^') && r.contains('1')));
                assert!(req_strs.iter().any(|r| r.contains('^') && r.contains('2')));
            }
            other => panic!("expected VersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn req_to_ranges_singleton_intersection() {
        let candidates = vec![ver("1.0.0"), ver("1.5.0"), ver("2.0.0")];
        let r = req_to_ranges(&req("^1"), &candidates);
        assert!(r.contains(&ver("1.0.0")));
        assert!(r.contains(&ver("1.5.0")));
        assert!(!r.contains(&ver("2.0.0")));
    }

    #[test]
    fn req_to_ranges_no_match_is_empty() {
        let candidates = vec![ver("1.0.0"), ver("1.5.0")];
        let r = req_to_ranges(&req("^99"), &candidates);
        assert!(!r.contains(&ver("1.0.0")));
        assert!(!r.contains(&ver("1.5.0")));
        assert!(r == Ranges::empty());
    }

    #[test]
    fn versions_descending_returns_latest_first() {
        let mut b = PubGrubBuilder::new();
        b.add_package_version("x", ver("1.0.0"));
        b.add_package_version("x", ver("3.0.0"));
        b.add_package_version("x", ver("2.0.0"));
        let descending = b.versions_descending(&CogName::from("x"));
        assert_eq!(descending, vec![ver("3.0.0"), ver("2.0.0"), ver("1.0.0")]);
    }

    #[test]
    fn add_package_version_is_idempotent() {
        let mut b = PubGrubBuilder::new();
        b.add_package_version("x", ver("1.0.0"));
        b.add_package_version("x", ver("1.0.0"));
        b.add_package_version("x", ver("1.0.0"));
        assert_eq!(b.versions_descending(&CogName::from("x")).len(), 1);
    }

    #[test]
    fn resolve_root_alone_yields_single_package() {
        let mut b = PubGrubBuilder::new();
        b.add_package_version("root", ver("1.0.0"));
        let solution = resolve(&b, "root", ver("1.0.0")).unwrap();
        assert_eq!(solution.len(), 1);
        assert_eq!(solution[0].name, "root");
        assert_eq!(solution[0].version, ver("1.0.0"));
    }

    #[test]
    fn resolve_solution_is_sorted() {
        let mut b = PubGrubBuilder::new();
        b.add_package_version("zebra", ver("1.0.0"));
        b.add_package_version("alpha", ver("1.0.0"));
        b.add_package_version("middle", ver("1.0.0"));
        b.add_dependency("zebra", ver("1.0.0"), "alpha", req("^1"));
        b.add_dependency("zebra", ver("1.0.0"), "middle", req("^1"));
        let solution = resolve(&b, "zebra", ver("1.0.0")).unwrap();
        let names: Vec<_> = solution.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }
}
