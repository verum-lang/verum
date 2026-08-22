//! Capability rows — the inference-first core of ATS-V-2 (T0848).
//!
//! Formalisation: `docs/architecture/ats-v2-capability-rows.md`
//! (duel-hardened; the laws below cite its sections). This module is
//! the ONE carrier of the row algebra: the lattice, the provenance
//! product, `carry(T)` composition and the domain-flow check all live
//! here, and the compiler-side inference (`ats_v_phase`) consumes it
//! rather than re-deriving any of it.
//!
//! Design pins the implementation must keep:
//!
//! * **No ⊤.** There is no "any capability" constructor anywhere in
//!   this module (§1). The widest expressible surface is an explicit
//!   set of atoms, and every widening is therefore legible in diffs.
//! * **One row operation.** Union-with-ground is the whole algebra
//!   (§2): no subtraction, no masking, no duplicate labels. Verum
//!   capabilities are never discharged by user code, so there is
//!   nothing for those operators to express.
//! * **Provenance is a meet.** `Computed ⊓ Cited = Cited` per atom
//!   (§6): a derivation path can only degrade provenance. The fact
//!   lattice is the PRODUCT of the atom powerset and the provenance
//!   meet, which is what makes the fixpoint Kleene in one line.
//! * **Generalise at summary boundaries only** (Rule G, §4): the
//!   `generalize` entry point exists for summary installation; there
//!   is deliberately no let-site helper.

use std::collections::BTreeMap;

use crate::arch::Capability;
use crate::intrinsic_dispatch::Evidence;

/// A row variable — stands for "whatever capability surface the named
/// capability-bearing parameter is instantiated with" (§2, §3).
///
/// Variables are identified by the PARAMETER they trace to, which is
/// what makes the no-silent-⊤ law auditable: every open row names its
/// sources (§4 law, clause b).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowVar {
    /// The summary (function) that binds this variable.
    pub owner: String,
    /// The capability-bearing parameter the variable traces to.
    pub param: String,
}

/// One capability atom with its provenance — the element of the
/// product lattice (§6).
///
/// Equality and ordering are BY ATOM ONLY (see `Row::join`): two
/// facts about one atom merge by provenance-meet rather than
/// coexisting, so a row never holds two rows for one atom.
#[derive(Debug, Clone)]
pub struct AtomFact {
    /// The capability.
    pub atom: Capability,
    /// Where the fact came from: `Computed` iff EVERY deriving path
    /// computed it; one cited edge anywhere makes it `Cited` (§6).
    pub evidence: Evidence,
}

/// Provenance meet: `Computed ⊓ x = x`, `Cited ⊓ _ = Cited` (the
/// first citation wins as the named source; what matters for the
/// lattice is that the result is not `Computed`).
fn evidence_meet(a: &Evidence, b: &Evidence) -> Evidence {
    match (a, b) {
        (Evidence::Computed, Evidence::Computed) => Evidence::Computed,
        (Evidence::Cited { source }, _) => Evidence::Cited {
            source: source.clone(),
        },
        (_, Evidence::Cited { source }) => Evidence::Cited {
            source: source.clone(),
        },
    }
}

/// A capability row: ground atoms plus (for an open row) the row
/// variables it may be widened by at instantiation (§2).
///
/// `⟨S⟩` is `vars.is_empty()`; `⟨S | r̄⟩` otherwise. The
/// representation keys atoms by their `Capability` so join is a
/// per-atom provenance merge, never a duplicate label — the algebra
/// has no duplicates by construction, not by discipline.
#[derive(Debug, Clone, Default)]
pub struct Row {
    /// Ground atoms, keyed by capability (BTreeMap for deterministic
    /// iteration — judgments print, and printed sets must be stable).
    atoms: BTreeMap<CapabilityKey, AtomFact>,
    /// Open-row variables, each traceable to a named parameter.
    vars: Vec<RowVar>,
}

/// `Capability` is `Hash + Eq` but not `Ord`; judgments and diffs
/// need deterministic ordering, so rows key atoms by their canonical
/// debug rendering. One rendering, one key — the map stays free of
/// duplicate atoms and iterates stably.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CapabilityKey(String);

impl CapabilityKey {
    fn of(atom: &Capability) -> Self {
        CapabilityKey(format!("{atom:?}"))
    }
}

impl Row {
    /// `⟨∅⟩` — the bottom of the lattice.
    pub fn empty() -> Self {
        Row::default()
    }

    /// A closed row over computed atoms.
    pub fn computed(atoms: impl IntoIterator<Item = Capability>) -> Self {
        let mut row = Row::empty();
        for atom in atoms {
            row.add(AtomFact {
                atom,
                evidence: Evidence::Computed,
            });
        }
        row
    }

    /// A closed row over atoms cited from a named authority (extern
    /// pins, protocol max-Shapes — §4/§5b). Panics on an empty
    /// source, same contract as `Evidence::cited` (T0841).
    pub fn cited(atoms: impl IntoIterator<Item = Capability>, source: &str) -> Self {
        assert!(
            !source.trim().is_empty(),
            "a cited row must name its authority — an empty source is a \
             stamped verdict, the exact defect class Evidence exists to kill"
        );
        let mut row = Row::empty();
        for atom in atoms {
            row.add(AtomFact {
                atom,
                evidence: Evidence::Cited {
                    source: source.to_string(),
                },
            });
        }
        row
    }

    /// Add one fact, merging provenance by meet if the atom is
    /// already present (§6).
    pub fn add(&mut self, fact: AtomFact) {
        let key = CapabilityKey::of(&fact.atom);
        match self.atoms.get_mut(&key) {
            Some(existing) => {
                existing.evidence = evidence_meet(&existing.evidence, &fact.evidence);
            }
            None => {
                self.atoms.insert(key, fact);
            }
        }
    }

    /// Open this row by a variable (a capability-bearing parameter
    /// the body may call or hand onward — `mix(r̄)`, §3).
    pub fn open_over(&mut self, var: RowVar) {
        if !self.vars.contains(&var) {
            self.vars.push(var);
        }
    }

    /// Join (union) — the ONE operation of the algebra (§2). Atoms
    /// union with per-atom provenance meet; variables union as sets.
    pub fn join(&mut self, other: &Row) {
        for fact in other.atoms.values() {
            self.add(fact.clone());
        }
        for var in &other.vars {
            self.open_over(var.clone());
        }
    }

    /// Instantiate one variable with a row (call-site substitution,
    /// §2). Substitution is monotone in both arguments: the result
    /// contains every atom of `self` (minus nothing) plus everything
    /// `replacement` brings.
    pub fn instantiate(&mut self, var: &RowVar, replacement: &Row) {
        if let Some(pos) = self.vars.iter().position(|v| v == var) {
            self.vars.remove(pos);
            self.join(replacement);
        }
    }

    /// Ground atoms, deterministically ordered.
    pub fn facts(&self) -> impl Iterator<Item = &AtomFact> {
        self.atoms.values()
    }

    /// Remaining open variables (empty ⟺ the row is closed).
    pub fn variables(&self) -> &[RowVar] {
        &self.vars
    }

    /// Is this row closed (`⟨S⟩`)?
    pub fn is_closed(&self) -> bool {
        self.vars.is_empty()
    }

    /// Number of ground atoms.
    pub fn atom_count(&self) -> usize {
        self.atoms.len()
    }

    /// Subsumption `self ⊑ other` on ground atoms (§2): every atom of
    /// `self` appears in `other`. Variables must match as sets for an
    /// open-row comparison to be meaningful; a closed row is ⊑ any
    /// row holding its atoms.
    pub fn subsumed_by(&self, other: &Row) -> bool {
        self.atoms.keys().all(|k| other.atoms.contains_key(k))
            && self.vars.iter().all(|v| other.vars.contains(v))
    }

    /// The two-direction judgment's raw material (design §2): atoms
    /// in `self` but not in `pinned` (escalation candidates), and
    /// atoms in `pinned` but not in `self` (dead-right candidates).
    /// Both directions return FACTS, so the diagnostic can print each
    /// atom's provenance.
    pub fn judge_against<'a>(
        &'a self,
        pinned: &'a Row,
    ) -> (Vec<&'a AtomFact>, Vec<&'a AtomFact>) {
        let escalations = self
            .atoms
            .iter()
            .filter(|(k, _)| !pinned.atoms.contains_key(*k))
            .map(|(_, f)| f)
            .collect();
        let dead_rights = pinned
            .atoms
            .iter()
            .filter(|(k, _)| !self.atoms.contains_key(*k))
            .map(|(_, f)| f)
            .collect();
        (escalations, dead_rights)
    }
}

/// A function summary: `∀r̄. ⟨S_f | mix(r̄)⟩` (§3). Variables listed
/// here are the ∀-bound ones — Rule G (§4) means summaries are the
/// ONLY place quantification happens.
#[derive(Debug, Clone)]
pub struct Summary {
    /// Qualified function name the summary describes.
    pub function: String,
    /// The body row: own atoms plus the mixed-in parameter variables.
    pub row: Row,
}

impl Summary {
    /// Install a summary — the single generalisation point (Rule G,
    /// §4). The row's open variables become the summary's ∀-bound
    /// variables by virtue of living in an installed summary; there
    /// is deliberately NO other entry point that quantifies.
    pub fn install(function: impl Into<String>, row: Row) -> Self {
        Summary {
            function: function.into(),
            row,
        }
    }

    /// Instantiate this summary at a call site: substitute each
    /// bound variable with the argument row supplied for its
    /// parameter (missing arguments instantiate to `⟨∅⟩` — a
    /// non-capability-bearing argument brings nothing).
    pub fn instantiate(&self, args: &BTreeMap<String, Row>) -> Row {
        let mut row = self.row.clone();
        for var in self.row.variables().to_vec() {
            let replacement = args.get(&var.param).cloned().unwrap_or_else(Row::empty);
            row.instantiate(&var, &replacement);
        }
        row
    }
}

/// The domain-flow law (§5a, duel strike 1): on a cross-domain edge
/// with payload carry `carry`, every ENFORCED payload atom must be
/// inside the destination domain's allow-set — otherwise the right
/// would die at the boundary at runtime, and ATS-V-2 refuses
/// surprise kills. Returns the atoms that would die (empty ⟺ the
/// edge is legal).
///
/// `enforced` is the derived enforceable-class filter (design §2 —
/// the list is read off the enforcement machinery, never
/// hand-written); `allow_dst` is `allow(D_dst)` from the manifest.
pub fn atoms_dying_at_boundary<'a>(
    carry: &'a Row,
    enforced: &dyn Fn(&Capability) -> bool,
    allow_dst: &Row,
) -> Vec<&'a AtomFact> {
    carry
        .facts()
        .filter(|f| enforced(&f.atom))
        .filter(|f| {
            !allow_dst
                .atoms
                .contains_key(&CapabilityKey::of(&f.atom))
        })
        .collect()
}

// ============================================================================
// The module solver — SCC fixpoint over per-function facts (§6)
// ============================================================================

/// Everything inference EXTRACTS from one function's body — the
/// solver's input row. The extraction layer (compiler-side, AST-aware)
/// produces these; the solver here is AST-blind on purpose: the
/// algebra and the algorithm stay in the kernel where they are
/// unit-pinned, and the compiler contributes only facts.
#[derive(Debug, Clone, Default)]
pub struct FnFacts {
    /// Ontology atoms from direct primitive calls in the body
    /// (`S_f`'s ground part, §3).
    pub own: Row,
    /// DIRECT calls to named functions (resolved within the module —
    /// the local call-graph edges the fixpoint runs over).
    pub callees: Vec<String>,
    /// Capability-bearing parameters the body may call or hand onward
    /// (`mix(r̄)`, §3). Each becomes a row variable of the summary.
    pub mixed_params: Vec<String>,
}

/// Per-module inference input: function name → facts.
#[derive(Debug, Clone, Default)]
pub struct ModuleFacts {
    /// Facts per function, keyed by the name callees use.
    pub functions: BTreeMap<String, FnFacts>,
}

impl ModuleFacts {
    /// Solve to summaries: Tarjan SCCs over the local call graph in
    /// reverse topological order; within an SCC, Kleene-iterate joins
    /// until stable (§6 — the lattice is the finite product of atom
    /// powerset × provenance meet, so stabilisation is guaranteed and
    /// the iteration count is bounded by `|𝒦| × |SCC|`).
    ///
    /// A callee with NO facts entry is an UNRESOLVED edge. Per the
    /// no-silent-⊤ law (§4) it must not silently widen OR silently
    /// vanish: it is returned in `unresolved` so the caller turns it
    /// into a fixpoint obligation or a diagnostic — never a guess.
    pub fn solve(&self) -> ModuleSummaries {
        // --- Tarjan, iterative, over the name graph. -----------------
        let names: Vec<&String> = self.functions.keys().collect();
        let index_of: BTreeMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let n = names.len();
        let adj: Vec<Vec<usize>> = names
            .iter()
            .map(|name| {
                self.functions[*name]
                    .callees
                    .iter()
                    .filter_map(|c| index_of.get(c.as_str()).copied())
                    .collect()
            })
            .collect();

        let mut sccs: Vec<Vec<usize>> = Vec::new();
        {
            let mut index = vec![usize::MAX; n];
            let mut low = vec![0usize; n];
            let mut on_stack = vec![false; n];
            let mut stack: Vec<usize> = Vec::new();
            let mut next_index = 0usize;
            // Iterative Tarjan: (node, child-cursor) frames.
            for root in 0..n {
                if index[root] != usize::MAX {
                    continue;
                }
                let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
                while let Some(&mut (v, ref mut cursor)) = frames.last_mut() {
                    if *cursor == 0 {
                        index[v] = next_index;
                        low[v] = next_index;
                        next_index += 1;
                        stack.push(v);
                        on_stack[v] = true;
                    }
                    if *cursor < adj[v].len() {
                        let w = adj[v][*cursor];
                        *cursor += 1;
                        if index[w] == usize::MAX {
                            frames.push((w, 0));
                        } else if on_stack[w] {
                            low[v] = low[v].min(index[w]);
                        }
                    } else {
                        if low[v] == index[v] {
                            let mut comp = Vec::new();
                            loop {
                                let w = stack.pop().expect("tarjan stack underflow");
                                on_stack[w] = false;
                                comp.push(w);
                                if w == v {
                                    break;
                                }
                            }
                            sccs.push(comp);
                        }
                        let (child, _) = frames.pop().expect("frame underflow");
                        if let Some(&mut (parent, _)) = frames.last_mut() {
                            low[parent] = low[parent].min(low[child]);
                        }
                    }
                }
            }
        }
        // Tarjan emits SCCs in REVERSE topological order of the
        // condensation — exactly the order summaries must be
        // installed (callees before callers).

        let mut summaries: BTreeMap<String, Row> = BTreeMap::new();
        let mut unresolved: Vec<UnresolvedEdge> = Vec::new();

        for comp in &sccs {
            // Kleene iteration within the component.
            loop {
                let mut changed = false;
                for &vi in comp {
                    let name = names[vi].clone();
                    let facts = &self.functions[&name];
                    let mut row = facts.own.clone();
                    for p in &facts.mixed_params {
                        row.open_over(RowVar {
                            owner: name.clone(),
                            param: p.clone(),
                        });
                    }
                    for callee in &facts.callees {
                        match summaries.get(callee) {
                            Some(callee_row) => row.join(callee_row),
                            None if self.functions.contains_key(callee) => {
                                // Same-SCC callee not yet stabilised —
                                // its current partial summary is what
                                // the iteration refines; nothing to do
                                // this pass.
                            }
                            None => {
                                let edge = UnresolvedEdge {
                                    caller: name.clone(),
                                    callee: callee.clone(),
                                };
                                if !unresolved.contains(&edge) {
                                    unresolved.push(edge);
                                }
                            }
                        }
                    }
                    let prev = summaries.get(&name);
                    let grew = match prev {
                        None => true,
                        Some(p) => {
                            row.atom_count() > p.atom_count()
                                || row.variables().len() > p.variables().len()
                        }
                    };
                    if grew {
                        summaries.insert(name, row);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
        }

        ModuleSummaries {
            summaries,
            unresolved,
        }
    }
}

/// A call edge the solver could not resolve to any summary — the
/// caller must surface it (obligation or diagnostic), never guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedEdge {
    /// The calling function.
    pub caller: String,
    /// The name that resolved to nothing.
    pub callee: String,
}

/// The solver's output: installed summaries plus the unresolved
/// edges the no-silent-⊤ law forbids swallowing.
#[derive(Debug, Clone, Default)]
pub struct ModuleSummaries {
    /// Function name → its solved row (Rule G: these ARE the
    /// generalisation points).
    pub summaries: BTreeMap<String, Row>,
    /// Edges with no target summary — surfaced, not guessed.
    pub unresolved: Vec<UnresolvedEdge>,
}

impl ModuleSummaries {
    /// The module's inferred surface: the join of every summary's
    /// ground atoms (§5's clause (a); value-flow escapes join in at
    /// the extraction layer as additional facts).
    pub fn module_surface(&self) -> Row {
        let mut out = Row::empty();
        for row in self.summaries.values() {
            out.join(row);
        }
        out
    }
}

#[cfg(test)]
mod solver_tests {
    //! Solver pins live INLINE deliberately: the workspace `--lib`
    //! CI job is the gated surface, and the solver is exactly the
    //! kind of quiet machinery that must not depend on a non-gated
    //! suite (the T0834 lesson).

    use super::*;
    use crate::arch::{Capability, NetDirection, NetProtocol, ResourceTag};

    fn read_atom() -> Capability {
        Capability::Read {
            resource: ResourceTag::Logger,
        }
    }

    fn net_atom() -> Capability {
        Capability::Network {
            protocol: NetProtocol::Tcp,
            direction: NetDirection::Outbound,
        }
    }

    /// Transitivity: caller absorbs callee atoms through the graph.
    #[test]
    fn atoms_flow_up_the_call_graph() {
        let mut m = ModuleFacts::default();
        m.functions.insert(
            "leaf".into(),
            FnFacts {
                own: Row::computed([net_atom()]),
                ..Default::default()
            },
        );
        m.functions.insert(
            "mid".into(),
            FnFacts {
                callees: vec!["leaf".into()],
                ..Default::default()
            },
        );
        m.functions.insert(
            "top".into(),
            FnFacts {
                own: Row::computed([read_atom()]),
                callees: vec!["mid".into()],
                ..Default::default()
            },
        );
        let solved = m.solve();
        assert!(solved.unresolved.is_empty());
        assert_eq!(solved.summaries["top"].atom_count(), 2);
        assert_eq!(solved.summaries["mid"].atom_count(), 1);
    }

    /// Mutual recursion stabilises with the union of both bodies.
    #[test]
    fn scc_fixpoint_stabilises_mutual_recursion() {
        let mut m = ModuleFacts::default();
        m.functions.insert(
            "ping".into(),
            FnFacts {
                own: Row::computed([read_atom()]),
                callees: vec!["pong".into()],
                ..Default::default()
            },
        );
        m.functions.insert(
            "pong".into(),
            FnFacts {
                own: Row::computed([net_atom()]),
                callees: vec!["ping".into()],
                ..Default::default()
            },
        );
        let solved = m.solve();
        assert_eq!(solved.summaries["ping"].atom_count(), 2);
        assert_eq!(solved.summaries["pong"].atom_count(), 2);
    }

    /// The no-silent-⊤ law at the solver: an unknown callee is
    /// SURFACED, and the caller's row does not silently widen.
    #[test]
    fn unknown_callee_is_surfaced_never_guessed() {
        let mut m = ModuleFacts::default();
        m.functions.insert(
            "caller".into(),
            FnFacts {
                callees: vec!["mystery".into()],
                ..Default::default()
            },
        );
        let solved = m.solve();
        assert_eq!(solved.unresolved.len(), 1);
        assert_eq!(solved.unresolved[0].callee, "mystery");
        assert_eq!(solved.summaries["caller"].atom_count(), 0);
    }

    /// Mixed params become row variables of the summary (Rule G) —
    /// and an innocent function with no capability traffic stays ∅.
    #[test]
    fn mixed_params_open_the_summary_and_innocents_stay_empty() {
        let mut m = ModuleFacts::default();
        m.functions.insert(
            "apply".into(),
            FnFacts {
                mixed_params: vec!["f".into()],
                ..Default::default()
            },
        );
        m.functions.insert(
            "pure_math".into(),
            FnFacts::default(),
        );
        let solved = m.solve();
        assert_eq!(solved.summaries["apply"].variables().len(), 1);
        assert!(solved.summaries["pure_math"].is_closed());
        assert_eq!(solved.summaries["pure_math"].atom_count(), 0);
    }
}
