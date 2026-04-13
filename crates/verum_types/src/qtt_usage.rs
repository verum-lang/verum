//! Quantitative Type Theory (QTT) usage analysis.
//!
//! In QTT every binding carries a `Quantity` describing how many
//! times it may be used:
//!
//! | Quantity   | Meaning                                |
//! |------------|----------------------------------------|
//! | `Zero`     | Erased — usable only at type level     |
//! | `One`      | **Linear** — used exactly once at runtime |
//! | `Omega`    | Unrestricted — used any number of times |
//! | `AtMost(n)`| **Affine** — used ≤ n times              |
//! | `Graded(n)`| Used at most n times (annotated)        |
//!
//! The `Quantity` type and its algebraic operations (`add`, `mul`,
//! `allows`) live in `crate::ty::Quantity`. This module provides
//! the **usage tracker** that walks an expression body, counts
//! occurrences of each binding, and validates against the declared
//! quantities.
//!
//! ## Branch semantics
//!
//! For control-flow nodes the usage count is the *maximum* across
//! branches (worst-case execution path), not the sum:
//!
//! ```text
//!     usage(if c then e1 else e2) = max(usage(e1), usage(e2))
//!     usage(match s { p1 => e1; ... pn => en }) = max(usage(e1), ..., usage(en))
//! ```
//!
//! Sequential composition (block, function call) sums:
//!
//! ```text
//!     usage(e1; e2) = usage(e1) + usage(e2)
//!     usage(f(e1, e2)) = usage(f) + usage(e1) + usage(e2)
//! ```
//!
//! ## Soundness
//!
//! For Linear (`One`) bindings, **both** branches of a conditional
//! must use the binding exactly once — otherwise some execution
//! path leaks the resource and another consumes it twice (when the
//! branches are joined). The tracker enforces this by requiring
//! that branch usages *agree* (same observed count) for any binding
//! whose declared quantity is `One`.
//!
//! ## Integration status
//!
//! This module is the standalone analysis core. Wiring it into the
//! type checker (which calls `check_function` after inference)
//! is a separate integration step — keeping the analysis pure makes
//! it independently testable and reusable for tools (LSP usage
//! warnings, CBGR escape analysis, codegen erasure planner).

use std::collections::HashMap;

use verum_common::Text;

use crate::ty::Quantity;

/// Observed usage count for a single binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageCount {
    /// Number of times the binding is consumed at runtime.
    /// Zero-quantity (erased) and type-level uses do not increment
    /// this counter.
    pub runtime: u32,
}

impl UsageCount {
    pub const ZERO: Self = UsageCount { runtime: 0 };
    pub const ONCE: Self = UsageCount { runtime: 1 };

    /// Sequential composition: counts add.
    pub fn add(self, other: UsageCount) -> UsageCount {
        UsageCount {
            runtime: self.runtime.saturating_add(other.runtime),
        }
    }

    /// Branching composition: take the worst-case path.
    /// For Linear bindings the caller must additionally check the
    /// branches agree — see `merge_branches_for_linear`.
    pub fn max(self, other: UsageCount) -> UsageCount {
        UsageCount {
            runtime: self.runtime.max(other.runtime),
        }
    }
}

/// A QTT usage map: binding name → observed count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageMap {
    counts: HashMap<Text, UsageCount>,
}

impl UsageMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a single use of `name`.
    pub fn use_once(&mut self, name: Text) {
        let entry = self.counts.entry(name).or_insert(UsageCount::ZERO);
        entry.runtime = entry.runtime.saturating_add(1);
    }

    /// Sequential merge: add counts pointwise.
    pub fn merge_sequential(mut self, other: UsageMap) -> Self {
        for (k, v) in other.counts {
            let entry = self.counts.entry(k).or_insert(UsageCount::ZERO);
            *entry = entry.add(v);
        }
        self
    }

    /// Branching merge: take the maximum count pointwise. For Linear
    /// bindings the caller is responsible for additionally calling
    /// `assert_branches_agree` — taking max alone is correct for
    /// Affine and Omega but unsound for Linear.
    pub fn merge_branches_max(mut self, other: UsageMap) -> Self {
        // First, raise self counts to max with other.
        for (k, v) in &other.counts {
            let entry = self.counts.entry(k.clone()).or_insert(UsageCount::ZERO);
            *entry = entry.max(*v);
        }
        // Bindings present in self but not other already have their
        // own count, which is ≥ 0 — and other implicitly contributes
        // 0, so max stays unchanged. Nothing to do.
        self
    }

    pub fn lookup(&self, name: &Text) -> UsageCount {
        self.counts.get(name).copied().unwrap_or(UsageCount::ZERO)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Text, &UsageCount)> {
        self.counts.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }
}

/// A QTT violation: a binding was used more (or fewer for Linear)
/// times than its declared quantity allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QttViolation {
    pub binding: Text,
    pub declared: Quantity,
    pub observed: u32,
    pub kind: ViolationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// Used more times than allowed (consumes Linear/Affine twice).
    OverUse,
    /// Linear binding used zero times (resource leak).
    UnderUse,
    /// Branch arms disagree on Linear binding usage (one path
    /// consumes, another leaks).
    BranchDisagreement { left: u32, right: u32 },
    /// Erased (Zero) binding used at runtime.
    ErasedUsedAtRuntime,
}

impl std::fmt::Display for QttViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ViolationKind::OverUse => write!(
                f,
                "binding `{}` declared `{}` used {} times (over-use)",
                self.binding.as_str(),
                self.declared,
                self.observed
            ),
            ViolationKind::UnderUse => write!(
                f,
                "linear binding `{}` used {} times (must be exactly 1)",
                self.binding.as_str(),
                self.observed
            ),
            ViolationKind::BranchDisagreement { left, right } => write!(
                f,
                "linear binding `{}` used {} times in one branch, {} in another",
                self.binding.as_str(),
                left,
                right
            ),
            ViolationKind::ErasedUsedAtRuntime => write!(
                f,
                "erased (Zero-quantity) binding `{}` cannot be used at runtime",
                self.binding.as_str()
            ),
        }
    }
}

impl std::error::Error for QttViolation {}

/// Validate observed usage against declared quantity for a single
/// binding.
pub fn check_binding(
    name: &Text,
    declared: Quantity,
    observed: UsageCount,
) -> Result<(), QttViolation> {
    let n = observed.runtime;
    match declared {
        Quantity::Zero => {
            if n > 0 {
                return Err(QttViolation {
                    binding: name.clone(),
                    declared,
                    observed: n,
                    kind: ViolationKind::ErasedUsedAtRuntime,
                });
            }
        }
        Quantity::One => {
            if n != 1 {
                return Err(QttViolation {
                    binding: name.clone(),
                    declared,
                    observed: n,
                    kind: if n == 0 {
                        ViolationKind::UnderUse
                    } else {
                        ViolationKind::OverUse
                    },
                });
            }
        }
        Quantity::Omega => {
            // Anything goes.
        }
        Quantity::AtMost(max) | Quantity::Graded(max) => {
            if n > max {
                return Err(QttViolation {
                    binding: name.clone(),
                    declared,
                    observed: n,
                    kind: ViolationKind::OverUse,
                });
            }
        }
    }
    Ok(())
}

/// Validate a full usage map against a declaration map.
///
/// `declarations` carries each binding's declared quantity;
/// `observed` is the usage map produced by walking the function
/// body. Returns the first violation found (deterministic by
/// alphabetical ordering of binding names) or `Ok` if all bindings
/// are within their declared quantities.
pub fn check_usage(
    declarations: &HashMap<Text, Quantity>,
    observed: &UsageMap,
) -> Result<(), QttViolation> {
    let mut names: Vec<&Text> = declarations.keys().collect();
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for name in names {
        let declared = declarations[name];
        let count = observed.lookup(name);
        check_binding(name, declared, count)?;
    }
    Ok(())
}

/// For Linear bindings, branches of a conditional must agree on
/// usage count. Returns `Ok` if every Linear binding has identical
/// counts in both arms; returns the first disagreement otherwise.
pub fn assert_branches_agree(
    declarations: &HashMap<Text, Quantity>,
    left: &UsageMap,
    right: &UsageMap,
) -> Result<(), QttViolation> {
    let mut names: Vec<&Text> = declarations.keys().collect();
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for name in names {
        if declarations[name] == Quantity::One {
            let l = left.lookup(name).runtime;
            let r = right.lookup(name).runtime;
            if l != r {
                return Err(QttViolation {
                    binding: name.clone(),
                    declared: Quantity::One,
                    observed: l.max(r),
                    kind: ViolationKind::BranchDisagreement {
                        left: l,
                        right: r,
                    },
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Text {
        Text::from(s)
    }

    fn decls(items: &[(&str, Quantity)]) -> HashMap<Text, Quantity> {
        items.iter().map(|(n, q)| (t(n), *q)).collect()
    }

    #[test]
    fn empty_usage_passes_omega_decls() {
        let d = decls(&[("x", Quantity::Omega), ("y", Quantity::Omega)]);
        let u = UsageMap::new();
        assert!(check_usage(&d, &u).is_ok());
    }

    #[test]
    fn linear_used_once_passes() {
        let d = decls(&[("x", Quantity::One)]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        assert!(check_usage(&d, &u).is_ok());
    }

    #[test]
    fn linear_unused_fails_under_use() {
        let d = decls(&[("x", Quantity::One)]);
        let u = UsageMap::new();
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.kind, ViolationKind::UnderUse);
        assert_eq!(err.observed, 0);
    }

    #[test]
    fn linear_used_twice_fails_over_use() {
        let d = decls(&[("x", Quantity::One)]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        u.use_once(t("x"));
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.kind, ViolationKind::OverUse);
        assert_eq!(err.observed, 2);
    }

    #[test]
    fn affine_two_uses_passes_at_most_2() {
        let d = decls(&[("x", Quantity::AtMost(2))]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        u.use_once(t("x"));
        assert!(check_usage(&d, &u).is_ok());
    }

    #[test]
    fn affine_three_uses_fails_at_most_2() {
        let d = decls(&[("x", Quantity::AtMost(2))]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        u.use_once(t("x"));
        u.use_once(t("x"));
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.kind, ViolationKind::OverUse);
    }

    #[test]
    fn omega_unbounded_passes() {
        let d = decls(&[("x", Quantity::Omega)]);
        let mut u = UsageMap::new();
        for _ in 0..1000 {
            u.use_once(t("x"));
        }
        assert!(check_usage(&d, &u).is_ok());
    }

    #[test]
    fn zero_used_at_runtime_fails() {
        let d = decls(&[("x", Quantity::Zero)]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.kind, ViolationKind::ErasedUsedAtRuntime);
    }

    #[test]
    fn zero_unused_passes() {
        let d = decls(&[("x", Quantity::Zero)]);
        let u = UsageMap::new();
        assert!(check_usage(&d, &u).is_ok());
    }

    #[test]
    fn sequential_merge_sums_counts() {
        let mut a = UsageMap::new();
        a.use_once(t("x"));
        let mut b = UsageMap::new();
        b.use_once(t("x"));
        b.use_once(t("y"));
        let merged = a.merge_sequential(b);
        assert_eq!(merged.lookup(&t("x")).runtime, 2);
        assert_eq!(merged.lookup(&t("y")).runtime, 1);
    }

    #[test]
    fn branches_max_takes_worst_case() {
        let mut a = UsageMap::new();
        a.use_once(t("x"));
        a.use_once(t("x"));
        let mut b = UsageMap::new();
        b.use_once(t("x"));
        b.use_once(t("y"));
        b.use_once(t("y"));
        let merged = a.merge_branches_max(b);
        assert_eq!(merged.lookup(&t("x")).runtime, 2);
        assert_eq!(merged.lookup(&t("y")).runtime, 2);
    }

    #[test]
    fn linear_branches_must_agree() {
        let d = decls(&[("x", Quantity::One)]);
        let mut a = UsageMap::new();
        a.use_once(t("x"));
        let b = UsageMap::new(); // empty — leaks the resource
        let err = assert_branches_agree(&d, &a, &b).unwrap_err();
        assert!(matches!(
            err.kind,
            ViolationKind::BranchDisagreement { left: 1, right: 0 }
        ));
    }

    #[test]
    fn linear_branches_agree_passes() {
        let d = decls(&[("x", Quantity::One)]);
        let mut a = UsageMap::new();
        a.use_once(t("x"));
        let mut b = UsageMap::new();
        b.use_once(t("x"));
        assert!(assert_branches_agree(&d, &a, &b).is_ok());
    }

    #[test]
    fn affine_branches_can_differ() {
        // For Affine, branch disagreement is fine — max is taken.
        let d = decls(&[("x", Quantity::AtMost(1))]);
        let mut a = UsageMap::new();
        a.use_once(t("x"));
        let b = UsageMap::new();
        // No disagreement check for Affine.
        assert!(assert_branches_agree(&d, &a, &b).is_ok());
        // And the max is still ≤ 1.
        let merged = a.merge_branches_max(b);
        assert!(check_usage(&d, &merged).is_ok());
    }

    #[test]
    fn deterministic_violation_order() {
        // When multiple bindings violate, return the alphabetically
        // first one for deterministic diagnostics.
        let d = decls(&[
            ("zebra", Quantity::One),
            ("alpha", Quantity::One),
        ]);
        let u = UsageMap::new(); // both zero — both under-use
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.binding.as_str(), "alpha");
    }

    #[test]
    fn graded_quantity_acts_like_at_most() {
        let d = decls(&[("x", Quantity::Graded(3))]);
        let mut u = UsageMap::new();
        u.use_once(t("x"));
        u.use_once(t("x"));
        u.use_once(t("x"));
        assert!(check_usage(&d, &u).is_ok());

        u.use_once(t("x"));
        let err = check_usage(&d, &u).unwrap_err();
        assert_eq!(err.kind, ViolationKind::OverUse);
    }

    #[test]
    fn usage_count_saturates_on_overflow() {
        let mut c = UsageCount {
            runtime: u32::MAX,
        };
        c = c.add(UsageCount::ONCE);
        // Should not panic / overflow.
        assert_eq!(c.runtime, u32::MAX);
    }
}
