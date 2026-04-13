//! Coinductive Analysis — productivity, bisimulation, observation traces.
//!
//! Where inductive types are *least* fixed points (built bottom-up
//! by finite constructors), **coinductive** types are *greatest*
//! fixed points — infinite objects defined by what we can observe
//! about them. Streams, infinite trees, processes, and π-calculus
//! processes are the classic examples.
//!
//! ## Productivity
//!
//! A corecursive definition is **productive** iff every prefix of
//! its observable output is computed in finite time. The standard
//! syntactic check is *guarded corecursion*: every recursive
//! self-reference must occur **immediately** under a constructor of
//! the coinductive type, never inside another function call or
//! reduction.
//!
//! ## Bisimulation
//!
//! Two coinductive values are **bisimilar** iff every observation
//! we can make about one we can make about the other, and the
//! results are themselves bisimilar. This is the natural notion of
//! equality for coinductive types — propositional equality is
//! generally too strong for infinite objects.
//!
//! This module provides:
//!
//! * [`CorecursiveCall`] — describes a self-reference site
//! * [`check_productivity`] — syntactic guarded-corecursion check
//! * [`Observation`] — a single step of a coinductive value
//! * [`bisimilar_up_to`] — bisimulation via finite observation traces

use verum_common::{List, Text};

/// One self-reference site inside a corecursive definition. The
/// productivity check classifies each call by its *guard depth* —
/// the number of constructors of the coinductive type that wrap
/// the call. A guard depth of zero (the call is at the top of the
/// definition body) means the call is **unguarded** and the
/// definition is non-productive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorecursiveCall {
    /// Name of the function being called recursively.
    pub callee: Text,
    /// How many coinductive constructors wrap this call.
    /// `0` means the call is at the top — non-productive.
    pub guard_depth: u32,
}

impl CorecursiveCall {
    pub fn new(callee: impl Into<Text>, guard_depth: u32) -> Self {
        Self {
            callee: callee.into(),
            guard_depth,
        }
    }

    pub fn is_guarded(&self) -> bool {
        self.guard_depth >= 1
    }
}

/// Result of a productivity check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductivityResult {
    /// All recursive calls are guarded — definition is productive.
    Productive,
    /// At least one recursive call is unguarded.
    NonProductive {
        unguarded_calls: List<Text>,
    },
}

impl ProductivityResult {
    pub fn is_productive(&self) -> bool {
        matches!(self, ProductivityResult::Productive)
    }
}

/// Check productivity of a corecursive definition by examining all
/// self-reference sites. Productive iff every call has guard
/// depth ≥ 1.
pub fn check_productivity(calls: &[CorecursiveCall]) -> ProductivityResult {
    let unguarded: Vec<Text> = calls
        .iter()
        .filter(|c| !c.is_guarded())
        .map(|c| c.callee.clone())
        .collect();
    if unguarded.is_empty() {
        ProductivityResult::Productive
    } else {
        ProductivityResult::NonProductive {
            unguarded_calls: unguarded.into_iter().collect(),
        }
    }
}

/// A single observation made on a coinductive value: a label
/// (which destructor was applied) plus the resulting payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Observation {
    pub label: Text,
    pub payload: Text,
}

impl Observation {
    pub fn new(label: impl Into<Text>, payload: impl Into<Text>) -> Self {
        Self {
            label: label.into(),
            payload: payload.into(),
        }
    }
}

/// A finite trace of observations on a coinductive value.
/// Bisimilarity is checked by comparing traces up to a depth.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObservationTrace {
    steps: List<Observation>,
}

impl ObservationTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter<I: IntoIterator<Item = Observation>>(iter: I) -> Self {
        Self {
            steps: iter.into_iter().collect(),
        }
    }

    pub fn push(&mut self, obs: Observation) {
        self.steps.push(obs);
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Observation> {
        self.steps.iter()
    }

    /// Take the first `n` observations as a new trace.
    pub fn prefix(&self, n: usize) -> ObservationTrace {
        ObservationTrace {
            steps: self.steps.iter().take(n).cloned().collect(),
        }
    }
}

/// Two coinductive values are bisimilar **up to depth `k`** iff
/// their first `k` observations agree pairwise. Genuine bisimilarity
/// is the limit of this as `k → ∞` — for the productivity-checked
/// fragment of the language, a finite-depth check is sound for all
/// observable contexts up to that depth.
pub fn bisimilar_up_to(
    left: &ObservationTrace,
    right: &ObservationTrace,
    depth: usize,
) -> BisimulationResult {
    let l = left.prefix(depth);
    let r = right.prefix(depth);

    if l.len() != r.len() {
        return BisimulationResult::Distinct {
            divergence_at: l.len().min(r.len()),
            reason: BisimulationDivergence::TruncatedTraces,
        };
    }

    for (i, (a, b)) in l.iter().zip(r.iter()).enumerate() {
        if a != b {
            return BisimulationResult::Distinct {
                divergence_at: i,
                reason: BisimulationDivergence::ObservationMismatch {
                    left: a.clone(),
                    right: b.clone(),
                },
            };
        }
    }

    BisimulationResult::Bisimilar { observed_depth: l.len() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BisimulationResult {
    /// Traces agree up to the depth examined.
    Bisimilar { observed_depth: usize },
    /// Traces diverge at some step.
    Distinct {
        divergence_at: usize,
        reason: BisimulationDivergence,
    },
}

impl BisimulationResult {
    pub fn is_bisimilar(&self) -> bool {
        matches!(self, BisimulationResult::Bisimilar { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BisimulationDivergence {
    /// One trace is shorter than the requested depth.
    TruncatedTraces,
    /// Both traces have the same length, but observations differ.
    ObservationMismatch {
        left: Observation,
        right: Observation,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corecursive_call_with_guard_is_guarded() {
        let c = CorecursiveCall::new("stream", 1);
        assert!(c.is_guarded());
    }

    #[test]
    fn corecursive_call_at_top_is_unguarded() {
        let c = CorecursiveCall::new("loop", 0);
        assert!(!c.is_guarded());
    }

    #[test]
    fn empty_call_set_is_productive() {
        let r = check_productivity(&[]);
        assert!(r.is_productive());
    }

    #[test]
    fn all_guarded_calls_are_productive() {
        let calls = vec![
            CorecursiveCall::new("ones", 1),
            CorecursiveCall::new("ones", 2),
        ];
        let r = check_productivity(&calls);
        assert_eq!(r, ProductivityResult::Productive);
    }

    #[test]
    fn one_unguarded_call_makes_non_productive() {
        let calls = vec![
            CorecursiveCall::new("loop", 0),
            CorecursiveCall::new("safe", 1),
        ];
        let r = check_productivity(&calls);
        match r {
            ProductivityResult::NonProductive { unguarded_calls } => {
                assert_eq!(unguarded_calls.len(), 1);
                assert_eq!(unguarded_calls[0].as_str(), "loop");
            }
            _ => panic!("expected non-productive"),
        }
    }

    #[test]
    fn observation_equality_is_label_and_payload() {
        let a = Observation::new("head", "1");
        let b = Observation::new("head", "1");
        let c = Observation::new("head", "2");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn empty_traces_bisimilar_at_depth_zero() {
        let l = ObservationTrace::new();
        let r = ObservationTrace::new();
        let result = bisimilar_up_to(&l, &r, 0);
        assert!(result.is_bisimilar());
    }

    #[test]
    fn identical_traces_bisimilar() {
        let l = ObservationTrace::from_iter([
            Observation::new("head", "1"),
            Observation::new("tail.head", "2"),
        ]);
        let r = l.clone();
        let result = bisimilar_up_to(&l, &r, 5);
        match result {
            BisimulationResult::Bisimilar { observed_depth } => {
                assert_eq!(observed_depth, 2);
            }
            _ => panic!("expected bisimilar"),
        }
    }

    #[test]
    fn diverging_traces_yield_observation_mismatch() {
        let l = ObservationTrace::from_iter([
            Observation::new("head", "1"),
            Observation::new("tail.head", "2"),
        ]);
        let r = ObservationTrace::from_iter([
            Observation::new("head", "1"),
            Observation::new("tail.head", "999"), // diverges here
        ]);
        let result = bisimilar_up_to(&l, &r, 5);
        match result {
            BisimulationResult::Distinct {
                divergence_at,
                reason,
            } => {
                assert_eq!(divergence_at, 1);
                assert!(matches!(
                    reason,
                    BisimulationDivergence::ObservationMismatch { .. }
                ));
            }
            _ => panic!("expected distinct"),
        }
    }

    #[test]
    fn depth_limited_check_passes_when_prefix_agrees() {
        let l = ObservationTrace::from_iter([
            Observation::new("a", "1"),
            Observation::new("b", "2"),
            Observation::new("c", "3"), // would diverge here
        ]);
        let r = ObservationTrace::from_iter([
            Observation::new("a", "1"),
            Observation::new("b", "2"),
            Observation::new("c", "999"),
        ]);
        // Only check first 2 — those agree
        let result = bisimilar_up_to(&l, &r, 2);
        assert!(result.is_bisimilar());
    }

    #[test]
    fn truncated_trace_yields_truncated_divergence() {
        let l = ObservationTrace::from_iter([Observation::new("a", "1")]);
        let r = ObservationTrace::from_iter([
            Observation::new("a", "1"),
            Observation::new("b", "2"),
        ]);
        let result = bisimilar_up_to(&l, &r, 5);
        match result {
            BisimulationResult::Distinct { reason, .. } => {
                assert_eq!(reason, BisimulationDivergence::TruncatedTraces);
            }
            _ => panic!("expected distinct (truncated)"),
        }
    }

    #[test]
    fn prefix_extracts_initial_segment() {
        let t = ObservationTrace::from_iter([
            Observation::new("a", "1"),
            Observation::new("b", "2"),
            Observation::new("c", "3"),
        ]);
        let p = t.prefix(2);
        assert_eq!(p.len(), 2);
        let first = p.iter().next().unwrap();
        assert_eq!(first.label.as_str(), "a");
    }

    #[test]
    fn productivity_result_is_productive_method() {
        assert!(ProductivityResult::Productive.is_productive());
        assert!(!ProductivityResult::NonProductive {
            unguarded_calls: List::new(),
        }
        .is_productive());
    }

    #[test]
    fn deeply_guarded_call_still_productive() {
        let calls = vec![CorecursiveCall::new("very_safe", 5)];
        assert!(check_productivity(&calls).is_productive());
    }
}
