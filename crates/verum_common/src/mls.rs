//! Multi-Level Security (MLS) classification lattice — Phase 2a (#282).
//!

//! Foundational primitive for Verum's information-flow analysis. Phase 1
//! (#266) established the call-site friction layer: every dangerous
//! declaration (extern fn, unsafe fn) must carry an explicit
//! `@classification(level)` attribute matching the manifest floor.
//!

//! This module promotes the enum from its private home in
//! `verum_compiler::phases::safety_gate` to the shared layer so the
//! type checker (Phase 2b: propagation through Pi-types) and the
//! context system (Phase 3: side-effect classification) can consume
//! the same lattice without re-defining it.
//!

//! # Lattice
//!

//! The classification levels form a total order:
//!

//! ```text
//!  Public ⊑ Secret ⊑ TopSecret
//! ```
//!

//! - **Join (⊔)** — least upper bound. The classification of a value
//!  derived from multiple sources is the join of the source
//!  classifications. Adding `Secret` and `Public` produces `Secret`;
//!  adding `Secret` and `TopSecret` produces `TopSecret`.
//!

//! - **Meet (⊓)** — greatest lower bound. The classification floor
//!  that ALL of a set of contexts can write into. Used for sink-
//!  detection: the meet of every consumer's classification gives the
//!  minimum classification a value must have to flow into all of
//!  them.
//!

//! - **Subsumes (⊒)** — `a ⊒ b` iff `a` is at least as classified as
//!  `b`. Used for the surface gate (`@classification(top_secret)`
//!  satisfies `mls_level = "secret"` because TopSecret ⊒ Secret).
//!

//! # Phase Roadmap
//!

//! - **Phase 1 (#266)**: surface gate at safety_gate.rs — closed.
//! - **Phase 2a (#282)**: this module — lattice primitive.
//! - **Phase 2b (#282-Followup)**: type-level taint propagation. Add
//!  `Classification` annotation to function parameter types; the
//!  unifier joins source classifications when binding values.
//! - **Phase 3 (#283)**: side-effect classification. The context
//!  system tracks which contexts are low-classification sinks; values
//!  above the sink's classification require explicit `@declassify`.

use serde::{Deserialize, Serialize};

/// MLS classification level.
///

/// Total-ordered: `Public < Secret < TopSecret`. The `Ord` impl is
/// the lattice's height ordering — directly usable for `cmp` and
/// `min`/`max` (which compute meet/join).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Serialize, Deserialize, Default,
)]
pub enum MlsLevel {
    /// No classification — the default for every value not
    /// explicitly classified.
    #[default]
    Public,
    /// Secret-level data. Cannot flow into Public sinks without
    /// `@declassify`.
    Secret,
    /// Top-secret data. Cannot flow into Secret or Public sinks
    /// without `@declassify`.
    TopSecret,
}

impl MlsLevel {
    /// Parse from manifest string. Unknown values fall back to
    /// `Public` (the safe default — un-annotated values are
    /// unclassified).
    ///

    /// Accepts: `"public"`, `"secret"`, `"top_secret"`. The hyphen
    /// form `"top-secret"` is also accepted as an alias.
    pub fn from_manifest_str(s: &str) -> Self {
        match s {
            "secret" => MlsLevel::Secret,
            "top_secret" | "top-secret" => MlsLevel::TopSecret,
            _ => MlsLevel::Public,
        }
    }

    /// Render as the canonical manifest spelling
    /// (`"public" | "secret" | "top_secret"`).
    pub fn as_manifest_str(&self) -> &'static str {
        match self {
            MlsLevel::Public => "public",
            MlsLevel::Secret => "secret",
            MlsLevel::TopSecret => "top_secret",
        }
    }

    /// Lattice join — least upper bound. The classification of a
    /// value derived from `self` AND `other` is `self.join(other)`.
    ///

    /// For the total order, this is `max(self, other)`.
    #[inline]
    pub fn join(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    /// Lattice meet — greatest lower bound. The minimum
    /// classification a value can have and still flow into BOTH of
    /// the levels at `self` and `other`.
    ///

    /// For the total order, this is `min(self, other)`.
    #[inline]
    pub fn meet(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    /// Subsumption — `self ⊒ required` iff `self` is at least as
    /// classified as `required`. Used by the Phase-1 surface gate
    /// (`@classification(top_secret)` subsumes manifest floor
    /// `secret`).
    #[inline]
    pub fn subsumes(self, required: Self) -> bool {
        self >= required
    }
}

impl std::fmt::Display for MlsLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_manifest_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_public() {
        // Pin: the safe default — every value un-annotated is Public.
        assert_eq!(MlsLevel::default(), MlsLevel::Public);
    }

    #[test]
    fn total_order_matches_documented_height() {
        // Pin: lattice height — Public < Secret < TopSecret.
        // The Ord derivation must match the documented enum
        // declaration order.
        assert!(MlsLevel::Public < MlsLevel::Secret);
        assert!(MlsLevel::Secret < MlsLevel::TopSecret);
        assert!(MlsLevel::Public < MlsLevel::TopSecret);
    }

    #[test]
    fn from_manifest_str_round_trips_canonical() {
        // Pin: every variant's canonical manifest form parses
        // back to itself.
        for level in [MlsLevel::Public, MlsLevel::Secret, MlsLevel::TopSecret] {
            let s = level.as_manifest_str();
            assert_eq!(MlsLevel::from_manifest_str(s), level,
                "round-trip failed for {:?}", level);
        }
    }

    #[test]
    fn from_manifest_str_accepts_hyphen_alias() {
        // Pin: `"top-secret"` (hyphen) is accepted as an alias for
        // `"top_secret"` for ergonomic CLI / configuration reasons.
        assert_eq!(MlsLevel::from_manifest_str("top-secret"),
                   MlsLevel::TopSecret);
    }

    #[test]
    fn from_manifest_str_unknown_falls_back_to_public() {
        // Pin: unknown / typo values map to Public — the safe
        // default. Callers performing strict validation should
        // intercept upstream (see LanguageFeatures::validate).
        assert_eq!(MlsLevel::from_manifest_str("classified"),
                   MlsLevel::Public);
        assert_eq!(MlsLevel::from_manifest_str(""),
                   MlsLevel::Public);
    }

    #[test]
    fn join_is_max_on_total_order() {
        // Pin: join (least upper bound) on a total order = max.
        assert_eq!(MlsLevel::Public.join(MlsLevel::Secret),
                   MlsLevel::Secret);
        assert_eq!(MlsLevel::Secret.join(MlsLevel::Public),
                   MlsLevel::Secret);
        assert_eq!(MlsLevel::Secret.join(MlsLevel::TopSecret),
                   MlsLevel::TopSecret);
        assert_eq!(MlsLevel::TopSecret.join(MlsLevel::Public),
                   MlsLevel::TopSecret);
    }

    #[test]
    fn meet_is_min_on_total_order() {
        // Pin: meet (greatest lower bound) on a total order = min.
        assert_eq!(MlsLevel::Public.meet(MlsLevel::Secret),
                   MlsLevel::Public);
        assert_eq!(MlsLevel::Secret.meet(MlsLevel::TopSecret),
                   MlsLevel::Secret);
        assert_eq!(MlsLevel::TopSecret.meet(MlsLevel::Public),
                   MlsLevel::Public);
    }

    #[test]
    fn join_is_idempotent() {
        // Pin: x.join(x) = x for every level. Algebraic invariant.
        for level in [MlsLevel::Public, MlsLevel::Secret, MlsLevel::TopSecret] {
            assert_eq!(level.join(level), level);
        }
    }

    #[test]
    fn join_is_commutative() {
        // Pin: x.join(y) = y.join(x). Algebraic invariant.
        let pairs = [
            (MlsLevel::Public, MlsLevel::Secret),
            (MlsLevel::Secret, MlsLevel::TopSecret),
            (MlsLevel::Public, MlsLevel::TopSecret),
        ];
        for (a, b) in pairs {
            assert_eq!(a.join(b), b.join(a));
            assert_eq!(a.meet(b), b.meet(a));
        }
    }

    #[test]
    fn join_is_associative() {
        // Pin: (x ⊔ y) ⊔ z = x ⊔ (y ⊔ z). Algebraic invariant.
        let levels = [MlsLevel::Public, MlsLevel::Secret, MlsLevel::TopSecret];
        for &a in &levels {
            for &b in &levels {
                for &c in &levels {
                    assert_eq!(a.join(b).join(c), a.join(b.join(c)));
                    assert_eq!(a.meet(b).meet(c), a.meet(b.meet(c)));
                }
            }
        }
    }

    #[test]
    fn absorption_law_holds() {
        // Pin: x ⊔ (x ⊓ y) = x AND x ⊓ (x ⊔ y) = x.
        // Lattice absorption laws — together with associativity +
        // commutativity, prove this is a lattice.
        let levels = [MlsLevel::Public, MlsLevel::Secret, MlsLevel::TopSecret];
        for &a in &levels {
            for &b in &levels {
                assert_eq!(a.join(a.meet(b)), a);
                assert_eq!(a.meet(a.join(b)), a);
            }
        }
    }

    #[test]
    fn subsumes_matches_geq() {
        // Pin: subsumes is exactly the >= relation on the lattice.
        // Used by Phase-1 surface gate.
        assert!(MlsLevel::TopSecret.subsumes(MlsLevel::Secret));
        assert!(MlsLevel::TopSecret.subsumes(MlsLevel::Public));
        assert!(MlsLevel::Secret.subsumes(MlsLevel::Public));
        assert!(MlsLevel::Public.subsumes(MlsLevel::Public));
        // Lower does NOT subsume higher — the inverse direction.
        assert!(!MlsLevel::Public.subsumes(MlsLevel::Secret));
        assert!(!MlsLevel::Secret.subsumes(MlsLevel::TopSecret));
    }

    #[test]
    fn display_matches_manifest_str() {
        // Pin: Display impl matches the canonical manifest spelling
        // so error-message rendering stays consistent.
        assert_eq!(format!("{}", MlsLevel::Public), "public");
        assert_eq!(format!("{}", MlsLevel::Secret), "secret");
        assert_eq!(format!("{}", MlsLevel::TopSecret), "top_secret");
    }

    #[test]
    fn level_is_copy_and_hash() {
        // Pin: the lattice value type must be `Copy` (passed by
        // value through the type checker without reference
        // gymnastics) and `Hash` (usable as a HashMap key for
        // per-binding classification tracking).
        fn assert_copy<T: Copy>() {}
        fn assert_hash<T: std::hash::Hash>() {}
        assert_copy::<MlsLevel>();
        assert_hash::<MlsLevel>();
    }
}
