//! Drift pins for the meta() consolidation on
//! `verum_types::dependency_injection::Scope`.
//!
//! These pins close the structural rule that was previously implicit
//! in the derived `PartialOrd`/`Ord` on the variant declaration order:
//!
//!   * `lifetime_rank` is dense 0..=2 strictly monotone in declaration
//!     order — Singleton(0) is longest-lived, Transient(2) shortest.
//!   * `can_depend_on(dep) == (dep.rank <= self.rank)` exactly,
//!     pinned via the full 3×3 reference table.
//!   * Round-trip: `Scope::from_str(x.name()) == Some(x)` for every
//!     variant.

use verum_common::Maybe;
use verum_types::dependency_injection::Scope;

#[test]
fn meta_pin_scope_round_trip_unique_and_dense_rank() {
    assert_eq!(Scope::ALL.len(), 3);
    let mut seen = Vec::new();
    for v in Scope::ALL {
        let s = v.name();
        match Scope::from_str(s) {
            Maybe::Some(round) => {
                assert_eq!(round, *v, "Scope::{:?}: name '{}' round-trip", v, s)
            }
            Maybe::None => panic!("Scope::{:?}: from_str dropped name '{}'", v, s),
        }
        assert!(!seen.contains(&s), "Scope: duplicate name '{}'", s);
        seen.push(s);
        // `as_str` is the new meta-series synonym — must agree with
        // `name`.
        assert_eq!(v.as_str(), v.name());
    }
    // Dense rank in declaration order.
    for (i, v) in Scope::ALL.iter().enumerate() {
        assert_eq!(
            v.lifetime_rank() as usize,
            i,
            "Scope::{:?}: rank drift at slot {}",
            v,
            i
        );
    }
    // Strict monotonicity.
    for w in Scope::ALL.windows(2) {
        assert!(
            w[0].lifetime_rank() < w[1].lifetime_rank(),
            "rank monotonicity violated: {:?} -> {:?}",
            w[0],
            w[1]
        );
    }
    // Negative pin.
    assert!(matches!(Scope::from_str("__bogus__"), Maybe::None));
}

#[test]
fn meta_pin_scope_can_depend_on_table_full() {
    // Reference table — exhaustive pin for the legacy "can depend on
    // same or longer-lived scopes" rule. Rows = self, cols =
    // dependency. `true` iff `dependency.rank <= self.rank`.
    //
    //                 Singleton  Request  Transient
    //   Singleton       true      false    false
    //   Request         true      true     false
    //   Transient       true      true     true
    let table: [[bool; 3]; 3] = [
        [true, false, false],
        [true, true, false],
        [true, true, true],
    ];
    for (i, a) in Scope::ALL.iter().enumerate() {
        for (j, b) in Scope::ALL.iter().enumerate() {
            assert_eq!(
                a.can_depend_on(*b),
                table[i][j],
                "Scope::can_depend_on drift: {:?} -> {:?}",
                a,
                b
            );
        }
    }
    // Reflexivity: every scope can depend on itself.
    for v in Scope::ALL {
        assert!(
            v.can_depend_on(*v),
            "Scope::{:?}: must be self-compatible",
            v
        );
    }
    // Equivalence with the legacy derived-Ord formula
    // `dependency <= *self` — pinned across all 9 (a, b) pairs so a
    // future variant reorder can't silently invert the rule.
    for a in Scope::ALL {
        for b in Scope::ALL {
            assert_eq!(
                a.can_depend_on(*b),
                *b <= *a,
                "derived-Ord parity: a={:?}, b={:?}",
                a,
                b
            );
        }
    }
}
