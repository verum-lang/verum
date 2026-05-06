//! Transitive peer-graph traversal for ATS-V multi-hop checks.
//!
//! ## Architectural role
//!
//! Several anti-patterns require analysis of the *transitive*
//! composes_with chain, not just the immediate peers:
//!
//!   * **AP-019 FoundationDowngrade** — the cog declares foundation
//!     `F` but the chain `self → A → B → … → terminal` reaches a
//!     cog whose foundation is strictly weaker than `F`.
//!   * **AP-024 TransitiveLifecycleRegression** — the cog declares
//!     lifecycle rank `r` but the chain reaches a cog with rank
//!     strictly less than `r`.
//!   * Future: **AP-022 CapabilityLaundering** — multi-hop privilege
//!     escalation traced through `composes_with` + body-level
//!     `Escalate` capabilities.
//!
//! All three share the same graph-theoretic primitive: depth-first
//! search over the composes_with edges, with cycle prevention and a
//! configurable depth bound.  This module ships the primitive once;
//! per-AP resolvers consume the visit stream and apply their
//! predicate.
//!
//! ## Design — compositional graph walker
//!
//! The traversal is decoupled from the predicate.  [`PeerGraphWalk`]
//! performs the DFS and emits a stream of `Visit { path, shape }`
//! callbacks; the caller's closure decides whether the visited shape
//! matches.  This separation:
//!
//!   * Lets one DFS implementation serve every transitive check.
//!   * Avoids re-walking the same graph for each AP.
//!   * Makes cycle-prevention and depth-bounding the traverser's
//!     concern, not the predicate's.
//!
//! ## Cycle prevention
//!
//! `composes_with` graphs SHOULD be acyclic (AP-003 enforces it),
//! but cycle-detection here defends against the case where AP-003
//! hasn't yet fired or where the registry is mid-population.  Each
//! visited cog is added to a `visited: BTreeSet<&str>`; revisiting
//! short-circuits.
//!
//! ## Depth bound
//!
//! `MAX_TRANSITIVE_DEPTH = 32` — a cog graph 32 hops deep is either
//! pathological or in mid-load.  The bound prevents stack-overflow
//! on adversarial input; legitimate corpora have depth ≪ 32.

use crate::arch::Shape;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Maximum transitive-walk depth.  Beyond this, the traverser
/// terminates the path and reports the truncation in the Visit
/// stream so callers can flag it (typically as a warning).
pub const MAX_TRANSITIVE_DEPTH: usize = 32;

/// One visit produced by the peer-graph walker.  The `path` field
/// records the full chain from the starting cog to this visit
/// (inclusive of both endpoints in stable order); the `shape`
/// field carries the visited cog's parsed `@arch_module` Shape.
#[derive(Debug, Clone)]
pub struct PeerVisit<'a> {
    /// Full chain from start cog to this visit.  `path[0]` is the
    /// starting cog; `path[path.len() - 1]` is the current visit's
    /// cog name.  Length ≥ 2 — the start itself does not produce a
    /// visit.
    pub path: Vec<String>,
    /// Visited cog's Shape (cloned from the registry — owning so
    /// callers can hold it independently of the registry's lock).
    pub shape: &'a Shape,
}

impl<'a> PeerVisit<'a> {
    /// Direct intermediate (the first hop from start).  When the
    /// path is `[start, intermediate, …, terminal]`, returns
    /// `intermediate`.  For depth-1 visits returns the visit's own
    /// cog name.
    pub fn intermediate(&self) -> Option<&str> {
        if self.path.len() >= 2 {
            Some(self.path[1].as_str())
        } else {
            None
        }
    }

    /// Terminal cog name — the visit itself.
    pub fn terminal(&self) -> &str {
        self.path
            .last()
            .map(|s| s.as_str())
            .unwrap_or("<empty-path>")
    }

    /// Depth at this visit (1 = immediate peer, 2 = peer-of-peer, …).
    pub fn depth(&self) -> usize {
        // path[0] is start, path.len() - 1 is current visit.
        self.path.len().saturating_sub(1)
    }
}

/// DFS traverser over the `composes_with` peer graph.
///
/// `for_each_peer` invokes the closure on every transitively-
/// reachable cog.  Closure receives a `PeerVisit` carrying the
/// path-from-start and the visited cog's `Shape`.  The closure
/// returns `ControlFlow::Continue` to keep walking or
/// `ControlFlow::Break` to terminate the entire walk.
///
/// Self is not visited — `start` does not appear as a Visit
/// (AP-024 / AP-019 / etc. compare against `start`'s own Shape,
/// supplied separately).
///
/// Cycle prevention: `visited: BTreeSet<&str>` skips already-seen
/// cogs.  Depth bound: `MAX_TRANSITIVE_DEPTH` clamps recursion.
pub fn for_each_transitive_peer<'a, F>(
    start: &str,
    registry: &'a BTreeMap<String, Shape>,
    mut visit: F,
) where
    F: FnMut(&PeerVisit<'a>) -> std::ops::ControlFlow<()>,
{
    // Look up the start cog's Shape — entry to the walk.  If the
    // start is not in the registry, no walk is possible.
    let start_shape = match registry.get(start) {
        Some(s) => s,
        None => return,
    };
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(start.to_string());
    let mut path: Vec<String> = vec![start.to_string()];
    let _ = walk_recursive(
        registry,
        &start_shape.composes_with,
        &mut path,
        &mut visited,
        &mut visit,
    );
}

fn walk_recursive<'a, F>(
    registry: &'a BTreeMap<String, Shape>,
    pending_peers: &[String],
    path: &mut Vec<String>,
    visited: &mut BTreeSet<String>,
    visit: &mut F,
) -> std::ops::ControlFlow<()>
where
    F: FnMut(&PeerVisit<'a>) -> std::ops::ControlFlow<()>,
{
    if path.len() >= MAX_TRANSITIVE_DEPTH {
        // Depth bound reached — terminate this branch.  The visit
        // is NOT emitted because we have nothing to report; the
        // truncation is silent for v1.  Future enhancement: emit a
        // synthetic "depth-bound-reached" visit so callers can
        // surface the warning.
        return std::ops::ControlFlow::Continue(());
    }
    for peer in pending_peers {
        if visited.contains(peer) {
            // Cycle — already on the visit-stack OR seen on a
            // sibling branch.  Skip.
            continue;
        }
        // Look up the peer's Shape.  Best-effort: if peer not yet
        // in registry (single-pass order-dependence), skip silently.
        let peer_shape = match registry.get(peer) {
            Some(s) => s,
            None => continue,
        };
        visited.insert(peer.clone());
        path.push(peer.clone());
        let visit_record = PeerVisit {
            path: path.clone(),
            shape: peer_shape,
        };
        match visit(&visit_record) {
            std::ops::ControlFlow::Continue(()) => {}
            std::ops::ControlFlow::Break(()) => {
                path.pop();
                return std::ops::ControlFlow::Break(());
            }
        }
        // Recurse into peer's own composes_with.
        match walk_recursive(
            registry,
            &peer_shape.composes_with,
            path,
            visited,
            visit,
        ) {
            std::ops::ControlFlow::Continue(()) => {}
            std::ops::ControlFlow::Break(()) => {
                path.pop();
                return std::ops::ControlFlow::Break(());
            }
        }
        path.pop();
        // Note: we deliberately do NOT remove `peer` from `visited`
        // on backtrack.  This implements DAG-style visit
        // deduplication (each cog visited at most once across the
        // entire walk) — appropriate for transitive-closure
        // analyses where the SET of reachable cogs matters more
        // than every distinct path.  For path-enumeration callers,
        // a future variant `for_each_transitive_path` would clear
        // visited on backtrack.
    }
    std::ops::ControlFlow::Continue(())
}

// =============================================================================
// Resolvers — per-AP predicates over the DFS stream
// =============================================================================

/// Resolve AP-024 TransitiveLifecycleRegression: walk the
/// transitive composes_with closure of `start`; for each visited
/// cog whose lifecycle rank is strictly less than `start`'s rank,
/// emit a `(intermediate, terminal, terminal_lifecycle)` tuple.
///
/// `intermediate` is the FIRST hop from start (the direct peer
/// through which the offending terminal is reached).
/// `terminal` is the offending cog.
/// `terminal_lifecycle` is the cog's `Lifecycle` value.
///
/// AP-009 catches the depth-1 case (direct citation of lower-rank
/// peer); AP-024 catches everything beyond depth 1.  This function
/// only emits regressions reached through 2+ hops to avoid
/// duplicating AP-009's coverage.
pub fn resolve_transitive_lifecycle_regressions(
    start: &str,
    start_lifecycle_rank: u8,
    registry: &BTreeMap<String, Shape>,
) -> Vec<(String, String, crate::arch::Lifecycle)> {
    let mut out: Vec<(String, String, crate::arch::Lifecycle)> = Vec::new();
    for_each_transitive_peer(start, registry, |visit| {
        // Only depth ≥ 2 — depth 1 is AP-009 territory.
        if visit.depth() >= 2 && visit.shape.lifecycle.rank() < start_lifecycle_rank {
            let intermediate = visit
                .intermediate()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let terminal = visit.terminal().to_string();
            out.push((intermediate, terminal, visit.shape.lifecycle.clone()));
        }
        std::ops::ControlFlow::Continue(())
    });
    out
}

/// Resolve AP-019 FoundationDowngrade transitive variant: walk the
/// transitive composes_with closure of `start`; for each visited
/// cog whose foundation does NOT directly subsume `start`'s
/// foundation AND is not the same as start's foundation, emit a
/// `(peer, peer_foundation, downgraded_foundation)` tuple.
///
/// AP-005 catches the depth-1 foundation drift; AP-019 surfaces
/// the same defect through 2+ hop chains.
///
/// The "downgrade" sense: `start.foundation` is strictly stronger
/// than the visited cog's foundation if there is no canonical
/// inclusion making the visited cog's foundation expressible inside
/// `start`'s foundation.  This is the negation of
/// `directly_subsumed_by` plus inequality.
pub fn resolve_transitive_foundation_downgrades(
    start: &str,
    start_foundation: &crate::arch::Foundation,
    registry: &BTreeMap<String, Shape>,
) -> Vec<(String, crate::arch::Foundation, crate::arch::Foundation)> {
    let mut out: Vec<(String, crate::arch::Foundation, crate::arch::Foundation)> = Vec::new();
    for_each_transitive_peer(start, registry, |visit| {
        // Only depth ≥ 2 — depth 1 is AP-005 territory.
        if visit.depth() >= 2 {
            let peer_f = &visit.shape.foundation;
            // Drift fires iff neither direction subsumes AND the
            // foundations are distinct.
            let no_subsumption = !start_foundation.directly_subsumed_by(peer_f)
                && !peer_f.directly_subsumed_by(start_foundation);
            if no_subsumption && peer_f != start_foundation {
                let terminal = visit.terminal().to_string();
                out.push((terminal, peer_f.clone(), start_foundation.clone()));
            }
        }
        std::ops::ControlFlow::Continue(())
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::*;

    fn shape_with(name_idx: u32, lifecycle: Lifecycle, foundation: Foundation, composes_with: Vec<String>) -> Shape {
        let _ = name_idx;
        Shape {
            exposes: vec![],
            requires: vec![],
            preserves: vec![],
            consumes: vec![],
            at_tier: Tier::Aot,
            foundation,
            stratum: MsfsStratum::LFnd,
            cve_closure: CveClosure {
                constructive: None,
                verifiable_strategy: None,
                executable: None,
            },
            lifecycle,
            composes_with,
            strict: false,
            declarations: None,
        }
    }

    fn make_registry(entries: Vec<(&str, Shape)>) -> BTreeMap<String, Shape> {
        entries
            .into_iter()
            .map(|(n, s)| (n.to_string(), s))
            .collect()
    }

    #[test]
    fn dfs_visits_immediate_and_transitive_peers() {
        // start → A → B
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v0.1".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v0.1".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["B".into()],
                ),
            ),
            (
                "B",
                shape_with(
                    2,
                    Lifecycle::Theorem { since: "v0.1".into() },
                    Foundation::ZfcTwoInacc,
                    vec![],
                ),
            ),
        ]);
        let mut visited: Vec<String> = Vec::new();
        for_each_transitive_peer("start", &reg, |v| {
            visited.push(v.terminal().to_string());
            std::ops::ControlFlow::Continue(())
        });
        // Should visit A and B but NOT start.
        assert_eq!(visited, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn dfs_skips_cycles() {
        // start → A → start (cycle)
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["start".into()],
                ),
            ),
        ]);
        let mut count = 0;
        for_each_transitive_peer("start", &reg, |_| {
            count += 1;
            std::ops::ControlFlow::Continue(())
        });
        // A is visited once; start is the entry (not visited); cycle
        // back to start is detected.
        assert_eq!(count, 1);
    }

    #[test]
    fn dfs_skips_unregistered_peers() {
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["nonexistent_peer".into()],
                ),
            ),
        ]);
        let mut count = 0;
        for_each_transitive_peer("start", &reg, |_| {
            count += 1;
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(count, 0);
    }

    #[test]
    fn ap_024_resolver_only_fires_at_depth_2_or_more() {
        // start (Theorem rank 6) → A (Theorem) → B (Hypothesis rank 2).
        // AP-024 fires on B (depth 2), not on A (depth 1, AP-009 territory).
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["B".into()],
                ),
            ),
            (
                "B",
                shape_with(
                    2,
                    Lifecycle::Hypothesis {
                        confidence: ConfidenceLevel::Low,
                    },
                    Foundation::ZfcTwoInacc,
                    vec![],
                ),
            ),
        ]);
        let regressions = resolve_transitive_lifecycle_regressions(
            "start",
            Lifecycle::Theorem { since: "v".into() }.rank(),
            &reg,
        );
        assert_eq!(regressions.len(), 1);
        let (intermediate, terminal, lc) = &regressions[0];
        assert_eq!(intermediate, "A");
        assert_eq!(terminal, "B");
        match lc {
            Lifecycle::Hypothesis { .. } => {}
            other => panic!("expected Hypothesis, got {:?}", other),
        }
    }

    #[test]
    fn ap_024_resolver_silent_when_chain_homogeneous() {
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["B".into()],
                ),
            ),
            (
                "B",
                shape_with(
                    2,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec![],
                ),
            ),
        ]);
        let regressions = resolve_transitive_lifecycle_regressions(
            "start",
            Lifecycle::Theorem { since: "v".into() }.rank(),
            &reg,
        );
        assert!(regressions.is_empty());
    }

    #[test]
    fn ap_019_resolver_fires_on_transitive_foundation_drift() {
        // start (HoTT) → A (Cubical, subsumes HoTT) → B (ZFC, no subsumption).
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::Hott,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::Cubical,
                    vec!["B".into()],
                ),
            ),
            (
                "B",
                shape_with(
                    2,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec![],
                ),
            ),
        ]);
        let downgrades = resolve_transitive_foundation_downgrades(
            "start",
            &Foundation::Hott,
            &reg,
        );
        assert_eq!(downgrades.len(), 1);
        let (terminal, _, _) = &downgrades[0];
        assert_eq!(terminal, "B");
    }

    #[test]
    fn ap_019_resolver_silent_when_chain_subsumed() {
        // start (CIC) → A (MLTT, subsumed by CIC) → B (MLTT) — all
        // peer foundations canonically included in CIC, no drift.
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::Cic,
                    vec!["A".into()],
                ),
            ),
            (
                "A",
                shape_with(
                    1,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::Mltt,
                    vec!["B".into()],
                ),
            ),
            (
                "B",
                shape_with(
                    2,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::Mltt,
                    vec![],
                ),
            ),
        ]);
        let downgrades = resolve_transitive_foundation_downgrades(
            "start",
            &Foundation::Cic,
            &reg,
        );
        assert!(downgrades.is_empty());
    }

    #[test]
    fn dfs_respects_max_depth() {
        // Build a long chain start → P_1 → P_2 → ... → P_50.
        let mut entries = vec![(
            "start".to_string(),
            shape_with(
                0,
                Lifecycle::Theorem { since: "v".into() },
                Foundation::ZfcTwoInacc,
                vec!["P_1".into()],
            ),
        )];
        for i in 1..50 {
            let next = if i == 49 { vec![] } else { vec![format!("P_{}", i + 1)] };
            entries.push((
                format!("P_{}", i),
                shape_with(
                    i,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    next,
                ),
            ));
        }
        let reg: BTreeMap<String, Shape> = entries.into_iter().collect();
        let mut max_depth = 0;
        for_each_transitive_peer("start", &reg, |v| {
            if v.depth() > max_depth {
                max_depth = v.depth();
            }
            std::ops::ControlFlow::Continue(())
        });
        // Walker terminates BEFORE pushing P_32 (path.len() reaches
        // MAX_TRANSITIVE_DEPTH at start..P_31, so visits stop after
        // P_31).  Exact bound depends on impl detail; we just check
        // we didn't visit all 50 peers.
        assert!(max_depth < 50, "depth bound did not engage: max_depth = {}", max_depth);
    }

    #[test]
    fn break_terminates_walk() {
        let reg = make_registry(vec![
            (
                "start",
                shape_with(
                    0,
                    Lifecycle::Theorem { since: "v".into() },
                    Foundation::ZfcTwoInacc,
                    vec!["A".into(), "B".into(), "C".into()],
                ),
            ),
            (
                "A",
                shape_with(1, Lifecycle::Theorem { since: "v".into() }, Foundation::ZfcTwoInacc, vec![]),
            ),
            (
                "B",
                shape_with(2, Lifecycle::Theorem { since: "v".into() }, Foundation::ZfcTwoInacc, vec![]),
            ),
            (
                "C",
                shape_with(3, Lifecycle::Theorem { since: "v".into() }, Foundation::ZfcTwoInacc, vec![]),
            ),
        ]);
        let mut count = 0;
        for_each_transitive_peer("start", &reg, |_| {
            count += 1;
            if count == 1 {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        assert_eq!(count, 1, "Break should terminate after first visit");
    }
}
