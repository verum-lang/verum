//! Region Calculus — Tofte-Talpin style memory-region tracking.
//!
//! In a region calculus, every heap allocation is tagged with a
//! **region** — a named lifetime within which the allocation lives.
//! Regions are introduced by `letregion ρ in e` (which allocates
//! a fresh empty region for `e` and reclaims it on exit) and
//! consumed by allocation `new[ρ] v` (which puts `v` into region
//! `ρ`).
//!
//! The type system enforces that no value escapes its containing
//! region: a function returning a value of type `T at ρ` may only
//! release `ρ` via the caller, never deallocate it locally.
//!
//! Tofte-Talpin gave a complete inference algorithm in 1997 for
//! the ML fragment without explicit annotations. This module
//! provides the **algebraic core**: region names, region sets,
//! escape checking, and region-polymorphism substitution.
//!
//! ## Why not just borrow checking?
//!
//! Verum's CBGR + lifetime system already handles many of the
//! same use cases. Region calculus is complementary: it gives a
//! more *static* discipline (no run-time CBGR check, fully erased
//! at codegen) at the cost of stricter type discipline. The two
//! coexist — region-polymorphic code can interop with CBGR-managed
//! code through region-to-lifetime translation.
//!
//! ## API
//!
//! * [`Region`] — a named region.
//! * [`RegionSet`] — set of regions a computation touches.
//! * [`RegionEnv`] — the regions in scope at a program point.
//! * [`RegionType`] — a type paired with the regions it depends on.
//! * [`check_no_escape`] — soundness gate: a returned type's
//!   regions must all be in the caller's region environment.

use std::collections::BTreeSet;

use verum_common::Text;

/// A region — an abstract memory lifetime named by a unique
/// identifier. Regions form a flat namespace: they are equal iff
/// their names are equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Region {
    pub name: Text,
}

impl Region {
    pub fn new(name: impl Into<Text>) -> Self {
        Self { name: name.into() }
    }
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ρ{}", self.name.as_str())
    }
}

/// A set of regions. Backed by `BTreeSet` for deterministic
/// ordering across diagnostic and proof_stability runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionSet {
    regions: BTreeSet<Region>,
}

impl RegionSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn singleton(r: Region) -> Self {
        let mut s = Self::new();
        s.regions.insert(r);
        s
    }

    pub fn insert(&mut self, r: Region) {
        self.regions.insert(r);
    }

    pub fn contains(&self, r: &Region) -> bool {
        self.regions.contains(r)
    }

    pub fn union(&self, other: &RegionSet) -> RegionSet {
        let mut out = self.clone();
        for r in &other.regions {
            out.regions.insert(r.clone());
        }
        out
    }

    pub fn difference(&self, other: &RegionSet) -> RegionSet {
        let mut out = RegionSet::new();
        for r in &self.regions {
            if !other.regions.contains(r) {
                out.regions.insert(r.clone());
            }
        }
        out
    }

    pub fn is_subset_of(&self, other: &RegionSet) -> bool {
        self.regions.iter().all(|r| other.regions.contains(r))
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Region> {
        self.regions.iter()
    }
}

/// The region environment in scope at a program point — the set
/// of regions the current computation may legally allocate into
/// or return values from.
#[derive(Debug, Clone, Default)]
pub struct RegionEnv {
    in_scope: BTreeSet<Region>,
}

impl RegionEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from<I: IntoIterator<Item = Region>>(iter: I) -> Self {
        Self {
            in_scope: iter.into_iter().collect(),
        }
    }

    /// Add a region to the environment (entering `letregion ρ`).
    pub fn push(&mut self, r: Region) {
        self.in_scope.insert(r);
    }

    /// Remove a region from the environment (exiting `letregion ρ`).
    pub fn pop(&mut self, r: &Region) -> bool {
        self.in_scope.remove(r)
    }

    pub fn contains(&self, r: &Region) -> bool {
        self.in_scope.contains(r)
    }

    pub fn as_set(&self) -> RegionSet {
        RegionSet {
            regions: self.in_scope.clone(),
        }
    }
}

/// A type paired with the regions it depends on. The opaque
/// `payload` carries the structural type information; the
/// `regions` set tracks every region whose lifetime the type
/// references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionType {
    pub payload: Text,
    pub regions: RegionSet,
}

impl RegionType {
    pub fn new(payload: impl Into<Text>, regions: RegionSet) -> Self {
        Self {
            payload: payload.into(),
            regions,
        }
    }

    /// Apply a region substitution: replace each region in
    /// `from_to` with its target.
    pub fn substitute(
        &self,
        from_to: &std::collections::HashMap<Region, Region>,
    ) -> RegionType {
        let mut new_set = RegionSet::new();
        for r in self.regions.iter() {
            let target = from_to.get(r).cloned().unwrap_or_else(|| r.clone());
            new_set.insert(target);
        }
        RegionType::new(self.payload.clone(), new_set)
    }
}

/// An escape violation: a returned type references a region not
/// in the caller's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapeViolation {
    pub region: Region,
    pub returned_type: Text,
}

impl std::fmt::Display for EscapeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "region escape: returned type `{}` depends on `{}`, not in caller's region environment",
            self.returned_type.as_str(),
            self.region
        )
    }
}

impl std::error::Error for EscapeViolation {}

/// Soundness gate: a returned type's regions must all be in the
/// caller's environment. Returns the first escaping region (in
/// alphabetical order for determinism) or `Ok(())` if all good.
pub fn check_no_escape(
    returned: &RegionType,
    caller_env: &RegionEnv,
) -> Result<(), EscapeViolation> {
    let env_set = caller_env.as_set();
    for r in returned.regions.iter() {
        if !env_set.contains(r) {
            return Err(EscapeViolation {
                region: r.clone(),
                returned_type: returned.payload.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Region {
        Region::new(name)
    }

    #[test]
    fn region_display_prefixes_rho() {
        assert_eq!(format!("{}", r("alpha")), "ρalpha");
    }

    #[test]
    fn region_set_singleton_contains_only_one() {
        let s = RegionSet::singleton(r("a"));
        assert_eq!(s.len(), 1);
        assert!(s.contains(&r("a")));
        assert!(!s.contains(&r("b")));
    }

    #[test]
    fn region_set_union_merges() {
        let mut s1 = RegionSet::new();
        s1.insert(r("a"));
        let mut s2 = RegionSet::new();
        s2.insert(r("b"));
        let u = s1.union(&s2);
        assert_eq!(u.len(), 2);
        assert!(u.contains(&r("a")));
        assert!(u.contains(&r("b")));
    }

    #[test]
    fn region_set_difference_excludes_other() {
        let mut s1 = RegionSet::new();
        s1.insert(r("a"));
        s1.insert(r("b"));
        s1.insert(r("c"));
        let mut s2 = RegionSet::new();
        s2.insert(r("b"));
        let d = s1.difference(&s2);
        assert_eq!(d.len(), 2);
        assert!(d.contains(&r("a")));
        assert!(d.contains(&r("c")));
    }

    #[test]
    fn region_set_subset_relation() {
        let mut s1 = RegionSet::new();
        s1.insert(r("a"));
        let mut s2 = RegionSet::new();
        s2.insert(r("a"));
        s2.insert(r("b"));
        assert!(s1.is_subset_of(&s2));
        assert!(!s2.is_subset_of(&s1));
    }

    #[test]
    fn region_set_iter_is_alphabetical() {
        let mut s = RegionSet::new();
        s.insert(r("zeta"));
        s.insert(r("alpha"));
        s.insert(r("mu"));
        let names: Vec<&str> = s.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn env_push_pop_round_trip() {
        let mut env = RegionEnv::new();
        env.push(r("a"));
        assert!(env.contains(&r("a")));
        assert!(env.pop(&r("a")));
        assert!(!env.contains(&r("a")));
    }

    #[test]
    fn pop_unknown_region_returns_false() {
        let mut env = RegionEnv::new();
        assert!(!env.pop(&r("never_pushed")));
    }

    #[test]
    fn region_type_substitute_renames_regions() {
        let mut original = RegionSet::new();
        original.insert(r("a"));
        original.insert(r("b"));
        let ty = RegionType::new("Box<Int>", original);

        let mut from_to = std::collections::HashMap::new();
        from_to.insert(r("a"), r("x"));

        let renamed = ty.substitute(&from_to);
        assert!(renamed.regions.contains(&r("x")));
        assert!(renamed.regions.contains(&r("b")));
        assert!(!renamed.regions.contains(&r("a")));
    }

    #[test]
    fn check_no_escape_passes_when_all_in_scope() {
        let env = RegionEnv::from([r("a"), r("b")]);
        let returned = RegionType::new("T", RegionSet::singleton(r("a")));
        assert!(check_no_escape(&returned, &env).is_ok());
    }

    #[test]
    fn check_no_escape_rejects_unknown_region() {
        let env = RegionEnv::from([r("a")]);
        let returned = RegionType::new("T", RegionSet::singleton(r("locallyalloc")));
        let err = check_no_escape(&returned, &env).unwrap_err();
        assert_eq!(err.region.name.as_str(), "locallyalloc");
    }

    #[test]
    fn check_no_escape_passes_with_empty_regions() {
        // A region-free type (like Int) escapes nothing.
        let env = RegionEnv::new();
        let returned = RegionType::new("Int", RegionSet::new());
        assert!(check_no_escape(&returned, &env).is_ok());
    }

    #[test]
    fn region_set_union_with_empty_is_self() {
        let mut s = RegionSet::new();
        s.insert(r("a"));
        let u = s.union(&RegionSet::new());
        assert_eq!(u, s);
    }

    #[test]
    fn region_type_substitute_with_empty_map_is_identity() {
        let mut original = RegionSet::new();
        original.insert(r("a"));
        let ty = RegionType::new("T", original);
        let from_to = std::collections::HashMap::new();
        let result = ty.substitute(&from_to);
        assert_eq!(result, ty);
    }

    #[test]
    fn env_as_set_reflects_current_scope() {
        let mut env = RegionEnv::new();
        env.push(r("a"));
        env.push(r("b"));
        let s = env.as_set();
        assert_eq!(s.len(), 2);
        assert!(s.contains(&r("a")));
        assert!(s.contains(&r("b")));
    }

    #[test]
    fn escape_violation_display_includes_region_name() {
        let v = EscapeViolation {
            region: r("local"),
            returned_type: Text::from("Box<Int>"),
        };
        let s = format!("{}", v);
        assert!(s.contains("ρlocal"));
        assert!(s.contains("Box<Int>"));
    }
}
