//! # Apply-graph traversal — transitive bridge-discharge classifier
//!
//! Task #150 / MSFS-L4.13 — the load-bearing piece that converts the
//! L3 → L4 promotion claim ("every step transitively reduces to
//! Verum kernel rules ∪ ZFC ∪ paper-cited external references") from
//! a *property of the immediate `apply` callsite* into a *property
//! of the entire transitive proof chain*.
//!
//! ## Why transitive matters
//!
//! Pre-#150 the bridge-discharge pre-pass at
//! `verum_compiler::phases::bridge_discharge_check::validate_proof_body_bridges`
//! walked *only the immediate apply callsites* in a theorem's proof
//! body.  Apply-targets that were not `kernel_*_strict` bridges were
//! silently skipped — the discharge claim "this proof transitively
//! grounds out in kernel-discharged bridges" was treated *on faith*
//! once the chain went through any intermediate `_full` form.
//!
//! Concretely: the standard MSFS pattern after the #143 audit refactor
//! is `corpus_thm.proof { apply stdlib_full(args); }` where
//! `stdlib_full.proof { apply kernel_strict_bridge_1(...); apply
//! kernel_strict_bridge_2(...); }`.  The pre-#150 pre-pass saw only
//! `apply stdlib_full(args)`, recognised it wasn't kernel-prefixed,
//! and accepted it.  A `stdlib_full` that bottomed out at a
//! placeholder axiom would pass the gate without raising any signal.
//!
//! Post-#150 the apply-graph walker resolves each apply-target against
//! the workspace-wide symbol table, recursively walks into its proof
//! body, and classifies the apply-graph's *leaves*.  The leaf taxonomy
//! is:
//!
//!   * [`LeafKind::KernelStrict`] — apply-target is a `kernel_*_strict`
//!     bridge that goes through `verum_kernel::dispatch_intrinsic`.
//!     The L4 trust base.
//!   * [`LeafKind::FrameworkAxiom`] — apply-target is an axiom carrying
//!     a `@framework(name, "citation")` attribute pointing to an
//!     external work (Lurie HTT, Adámek-Rosický, etc.).  Acceptable
//!     for L4 if the citation is to an authoritative external source.
//!   * [`LeafKind::PlaceholderAxiom`] — apply-target is an axiom with
//!     no `@framework` attribute and no kernel-strict prefix.  An
//!     internal stand-in awaiting promotion.  *This is the leaf class
//!     a transitive L4 claim must drive to zero.*
//!   * [`LeafKind::Unresolved`] — apply-target couldn't be located in
//!     the symbol table.  Indicates either a workspace-discovery gap
//!     or a built-in-kernel-rule reference outside the corpus.
//!
//! The composition `(kernel_strict, framework_axiom, placeholder_axiom,
//! unresolved)` for a theorem's transitive apply-graph is the L4
//! certificate.  A non-zero `placeholder_axiom` count means the L4
//! claim is *not yet load-bearing* for that theorem — every chain
//! continues to a stand-in that still requires elimination.
//!
//! ## Architecture
//!
//! Three layers:
//!
//!   1. [`ApplyTarget`] — one resolved apply-callsite reference.
//!   2. [`SymbolEntry`] / [`ApplyGraph`] — workspace-wide symbol
//!      lookup table with each entry classified at construction time.
//!   3. [`walk_transitive`] — DFS over the graph from a root theorem,
//!      classifying every leaf and accumulating the
//!      [`LeafComposition`].  Cycle detection via the visited set
//!      so mutual recursion (or self-reference) doesn't loop forever.
//!
//! The walker is *pure* (no I/O, no Z3 invocation, no kernel re-check
//! round-trip) — graph traversal + hashmap lookup.  Microsecond-scale
//! per theorem so the audit gate can walk the entire MSFS corpus
//! (37 theorems × max-depth 8) in milliseconds.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use verum_ast::decl::{ProofBody, ProofStep, ProofStepKind, TacticExpr};
use verum_ast::expr::{Expr, ExprKind};

/// Classification of a single apply-graph leaf.
///
/// A leaf is a node whose body either (a) doesn't recurse further
/// (axiom — no `apply` callsites in its body) or (b) is a kernel
/// bridge consumed by the dispatcher (`kernel_*_strict`).  Theorem
/// nodes whose proof body contains nested `apply`s are never leaves
/// themselves; they decompose into the leaves of their children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LeafKind {
    /// `kernel_*_strict` bridge — the trust base.  The dispatcher
    /// (in `verum_kernel::intrinsic_dispatch`) is the algorithmic
    /// witness for these.
    KernelStrict,
    /// Axiom carrying a `@framework(name, "citation")` attribute —
    /// trusted via external authoritative reference (Lurie HTT,
    /// Adámek-Rosický, etc.).  Acceptable for L4 only if the
    /// citation resolves to a real external proof.
    FrameworkAxiom,
    /// Plain axiom — no framework citation, not a kernel bridge.
    /// An *internal stand-in* awaiting promotion to a theorem with
    /// a real proof.  This is the leaf class a load-bearing L4
    /// claim must drive to zero.
    PlaceholderAxiom,
    /// Apply-target couldn't be resolved in the workspace's symbol
    /// table.  Either the workspace discovery missed a file (rare —
    /// the audit walker scans the manifest's tree) or the apply-
    /// target is a built-in kernel rule that lives outside the
    /// corpus (exotic).
    Unresolved,
}

impl LeafKind {
    /// Human-readable label for diagnostics + audit-report rendering.
    pub fn label(self) -> &'static str {
        match self {
            LeafKind::KernelStrict => "kernel_strict",
            LeafKind::FrameworkAxiom => "framework_axiom",
            LeafKind::PlaceholderAxiom => "placeholder_axiom",
            LeafKind::Unresolved => "unresolved",
        }
    }
}

/// One leaf hit during a transitive walk — the leaf classification
/// plus the apply-target name that produced it.  Carries enough
/// context for the audit report to reconstruct the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafHit {
    /// The apply-target's resolved symbol name (or the unresolved
    /// name as written in the source for `LeafKind::Unresolved`).
    pub symbol: String,
    /// The leaf's classification.
    pub kind: LeafKind,
    /// The chain of intermediate symbols traversed from the root
    /// to this leaf (root excluded; leaf included).  Useful for
    /// pinpointing which intermediate `_full` form leaks to a
    /// placeholder.
    pub chain: Vec<String>,
}

/// Composition of leaf classes for one transitive walk.  The L4
/// promotion claim is "every leaf is `KernelStrict` or
/// `FrameworkAxiom`" — i.e., `placeholder_axiom == 0` AND
/// `unresolved == 0`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LeafComposition {
    pub kernel_strict: usize,
    pub framework_axiom: usize,
    pub placeholder_axiom: usize,
    pub unresolved: usize,
    /// Per-leaf details for human-readable rendering / programmatic
    /// post-processing (e.g., the audit JSON).
    pub leaves: Vec<LeafHit>,
}

impl LeafComposition {
    /// `true` iff every leaf is in the L4-acceptable set
    /// (kernel_strict ∪ framework_axiom).
    pub fn is_l4_load_bearing(&self) -> bool {
        self.placeholder_axiom == 0 && self.unresolved == 0
    }

    /// Total leaf count across all classes.
    pub fn total(&self) -> usize {
        self.kernel_strict + self.framework_axiom + self.placeholder_axiom + self.unresolved
    }

    fn record(&mut self, hit: LeafHit) {
        match hit.kind {
            LeafKind::KernelStrict => self.kernel_strict += 1,
            LeafKind::FrameworkAxiom => self.framework_axiom += 1,
            LeafKind::PlaceholderAxiom => self.placeholder_axiom += 1,
            LeafKind::Unresolved => self.unresolved += 1,
        }
        self.leaves.push(hit);
    }
}

/// One symbol's classification in the workspace-wide symbol table.
///
/// At graph-construction time every theorem / axiom declaration in
/// the workspace is inserted with one of the three entry kinds.
/// `Theorem` entries carry their proof body so the walker can recurse;
/// the two axiom kinds are leaf classifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SymbolEntry {
    /// A theorem with a structured proof body — recurse into its
    /// `apply` callsites.
    Theorem {
        /// Names of apply-targets discovered in the proof body
        /// (in declaration order).  Pre-extracted to keep the
        /// recursive walker free of AST dependencies.
        apply_targets: Vec<String>,
    },
    /// A `kernel_*_strict` bridge or any axiom whose name starts
    /// with `kernel_` — the L4 trust base.
    KernelBridge,
    /// An axiom carrying a `@framework(...)` attribute pointing to
    /// an external authoritative reference.
    FrameworkAxiom,
    /// A plain axiom (no framework citation, no kernel prefix) — an
    /// internal stand-in.
    PlaceholderAxiom,
}

impl SymbolEntry {
    /// Classify a symbol as a leaf or recursive theorem entry.
    /// Returns the leaf classification when the entry is a leaf,
    /// or `None` when the entry is a theorem (and the walker should
    /// recurse).
    pub fn leaf_kind(&self) -> Option<LeafKind> {
        match self {
            SymbolEntry::Theorem { .. } => None,
            SymbolEntry::KernelBridge => Some(LeafKind::KernelStrict),
            SymbolEntry::FrameworkAxiom => Some(LeafKind::FrameworkAxiom),
            SymbolEntry::PlaceholderAxiom => Some(LeafKind::PlaceholderAxiom),
        }
    }
}

/// Workspace-wide symbol table.  Built once per audit run from a
/// scan of every `.vr` file under the manifest root.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyGraph {
    pub entries: BTreeMap<String, SymbolEntry>,
}

impl ApplyGraph {
    /// Construct a fresh, empty graph.  Populate via [`ApplyGraph::insert`]
    /// before traversing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a symbol with its pre-classified entry.  Repeated
    /// inserts of the same name overwrite (last-write-wins) — useful
    /// when a workspace file has both an `axiom` declaration and a
    /// later `theorem` promotion sharing the same identifier.
    pub fn insert(&mut self, name: impl Into<String>, entry: SymbolEntry) {
        self.entries.insert(name.into(), entry);
    }

    /// Look up a symbol's entry by name.  Returns `None` for
    /// undeclared symbols (the walker treats those as
    /// [`LeafKind::Unresolved`]).
    pub fn get(&self, name: &str) -> Option<&SymbolEntry> {
        self.entries.get(name)
    }
}

/// Walk the transitive apply-graph rooted at `root` and classify
/// every leaf.  The walker visits each symbol at most once
/// (cycle-safe via the visited set), so mutual recursion is
/// finite-time.
///
/// `max_depth` caps the walk depth as a safety stop (defensive: a
/// well-formed corpus reaches every leaf in 4–6 hops, but a
/// pathological cycle around the visited-set would still terminate
/// finitely; the depth cap is just a *display-friendly* termination
/// criterion).
///
/// Returns the [`LeafComposition`] — read-only summary the audit
/// gate emits as JSON / human-readable text.
pub fn walk_transitive(
    graph: &ApplyGraph,
    root: &str,
    max_depth: usize,
) -> LeafComposition {
    let mut composition = LeafComposition::default();
    let mut visited: HashSet<String> = HashSet::new();
    let mut chain: Vec<String> = Vec::new();
    walk_node(graph, root, &mut composition, &mut visited, &mut chain, max_depth);
    composition
}

fn walk_node(
    graph: &ApplyGraph,
    name: &str,
    composition: &mut LeafComposition,
    visited: &mut HashSet<String>,
    chain: &mut Vec<String>,
    remaining_depth: usize,
) {
    if visited.contains(name) {
        // Already accounted for under its first walk path; cycle-safe.
        return;
    }
    visited.insert(name.to_string());
    chain.push(name.to_string());

    if remaining_depth == 0 {
        // Depth cap reached — surface as Unresolved so the report
        // flags the runaway chain rather than silently truncating.
        composition.record(LeafHit {
            symbol: name.to_string(),
            kind: LeafKind::Unresolved,
            chain: chain.clone(),
        });
        chain.pop();
        return;
    }

    match graph.get(name) {
        Some(SymbolEntry::Theorem { apply_targets }) => {
            // Recurse into each apply-target.  Theorem nodes never
            // produce a leaf hit themselves — only their descendants do.
            // Snapshot the targets so the borrow on `graph` doesn't
            // interfere with the recursive borrow of `composition`.
            let targets: Vec<String> = apply_targets.clone();
            for target in &targets {
                walk_node(graph, target, composition, visited, chain, remaining_depth - 1);
            }
        }
        Some(entry) => {
            // Leaf — record the classification.
            let kind = entry.leaf_kind().unwrap_or(LeafKind::Unresolved);
            composition.record(LeafHit {
                symbol: name.to_string(),
                kind,
                chain: chain.clone(),
            });
        }
        None => {
            // Symbol not in the workspace — apply name-based fallback
            // classification.  Three workspace-boundary patterns
            // surface as recognised leaves rather than Unresolved:
            //
            //   1. `kernel_*` — stdlib kernel bridges outside the
            //      audited corpus tree.  Treat as `KernelStrict`
            //      (the dispatcher is the algorithmic witness).
            //
            //   2. Foreign-framework prefixes (`mathlib4.`,
            //      `coq_stdlib.`, `lean4_stdlib.`, `zfc.`,
            //      `mathlib.`) — apply targets cited from upstream
            //      vetted proofs.  Treat as `FrameworkAxiom` (the
            //      audit-report records the citation; the reviewer
            //      independently verifies the upstream proof).
            //      This is the workspace-boundary mirror of the
            //      `@framework(...)` attribute check on axioms.
            //
            //   3. Everything else — `Unresolved` so the audit
            //      report flags the workspace-discovery gap loudly.
            let kind = if name.starts_with("kernel_") {
                LeafKind::KernelStrict
            } else if is_foreign_framework_target(name) {
                LeafKind::FrameworkAxiom
            } else {
                LeafKind::Unresolved
            };
            composition.record(LeafHit {
                symbol: name.to_string(),
                kind,
                chain: chain.clone(),
            });
        }
    }

    chain.pop();
}

/// **Foreign-framework target classification** — decides whether an
/// apply-target name resolved-outside-workspace is a *cited upstream
/// proof* (mathlib4, Coq stdlib, Lean stdlib, ZFC) versus a genuine
/// workspace-discovery gap.
///
/// **Recognised prefixes** (matched at the start of the dotted path):
///
///   - `mathlib4.`      — Lean 4 mathlib
///   - `mathlib.`       — generic mathlib (Lean 3 / 4 hybrid corpus)
///   - `coq_stdlib.`    — Coq standard library
///   - `lean4_stdlib.`  — Lean 4 core library
///   - `lean_stdlib.`   — Lean 3 / generic Lean library
///   - `zfc.`           — ZFC-foundational citations (e.g.
///                        `zfc.extensionality`, `zfc.foundation`)
///   - `agda_stdlib.`   — Agda standard library
///   - `isabelle.`      — Isabelle/HOL library
///
/// **Why this lives in the apply-graph walker, not on the
/// `@framework` attribute**: the attribute is on the *target*'s
/// declaration site (the lemma stub).  When the target lives outside
/// the audited workspace tree, there's no `@framework(...)` attribute
/// available — the walker only has the apply-callsite name.  The
/// prefix-based classification mirrors the `kernel_*` workspace-
/// boundary fallback that was already in place for stdlib kernel
/// bridges.
///
/// **Discharges**: `kernel_v0/lemmas/*.vr` apply chains land on these
/// prefixes (e.g., `apply mathlib4.lambda.ChurchRosser`).  Without
/// this classifier they'd surface as `Unresolved` and the audit gate
/// would penalise the L4-load-bearing claim.
pub fn is_foreign_framework_target(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "mathlib4.",
        "mathlib.",
        "coq_stdlib.",
        "coq.",
        "lean4_stdlib.",
        "lean_stdlib.",
        "lean4.",
        "zfc.",
        "agda_stdlib.",
        "agda.",
        "isabelle.",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

// =============================================================================
// AST helpers — extract apply-target names from a proof body.
// =============================================================================

/// Walk a `ProofBody` and collect every `apply <symbol>(args)`
/// target's name in encounter order.  Skips non-apply tactics.
///
/// This is the compile-time half of the apply-graph: invoking it on
/// every theorem in the workspace populates each `SymbolEntry::Theorem`'s
/// `apply_targets` list.  Pure AST walk — no semantic resolution.
pub fn extract_apply_targets(body: &ProofBody) -> Vec<String> {
    let mut targets = Vec::new();
    walk_proof_body(body, &mut targets);
    targets
}

fn walk_proof_body(body: &ProofBody, targets: &mut Vec<String>) {
    match body {
        ProofBody::Structured(s) => {
            for step in s.steps.iter() {
                walk_proof_step(step, targets);
            }
        }
        ProofBody::Tactic(t) => walk_tactic(t, targets),
        ProofBody::Term(_) | ProofBody::ByMethod(_) => {}
    }
}

fn walk_proof_step(step: &ProofStep, targets: &mut Vec<String>) {
    match &step.kind {
        ProofStepKind::Tactic(t) => walk_tactic(t, targets),
        ProofStepKind::Have { justification, .. }
        | ProofStepKind::Show { justification, .. }
        | ProofStepKind::Suffices { justification, .. } => {
            walk_tactic(justification, targets);
        }
        ProofStepKind::Cases { cases, .. } => {
            for case in cases.iter() {
                for sub in case.proof.iter() {
                    walk_proof_step(sub, targets);
                }
            }
        }
        ProofStepKind::Focus { steps, .. } => {
            for sub in steps.iter() {
                walk_proof_step(sub, targets);
            }
        }
        ProofStepKind::Calc(chain) => {
            for cstep in chain.steps.iter() {
                walk_tactic(&cstep.justification, targets);
            }
        }
        ProofStepKind::Let { .. } | ProofStepKind::Obtain { .. } => {}
    }
}

fn walk_tactic(tactic: &TacticExpr, targets: &mut Vec<String>) {
    match tactic {
        TacticExpr::Apply { lemma, args } => {
            // Two parser shapes (matches bridge_discharge_check):
            //   * Apply{lemma:Call{func, args:[..]}, args:[]}
            //   * Apply{lemma:Path, args:[..]}
            let effective_lemma: &Expr = match &lemma.kind {
                ExprKind::Call { func, .. } if args.is_empty() => func.as_ref(),
                _ => lemma,
            };
            if let Some(name) = expr_as_path_name(effective_lemma) {
                targets.push(name);
            }
        }
        TacticExpr::Try(inner)
        | TacticExpr::Repeat(inner)
        | TacticExpr::AllGoals(inner)
        | TacticExpr::Focus(inner) => walk_tactic(inner, targets),
        TacticExpr::TryElse { body, fallback } => {
            walk_tactic(body, targets);
            walk_tactic(fallback, targets);
        }
        TacticExpr::Seq(tacs) | TacticExpr::Alt(tacs) => {
            for t in tacs.iter() {
                walk_tactic(t, targets);
            }
        }
        // Other tactic forms don't carry an `apply lemma` payload.
        _ => {}
    }
}

fn expr_as_path_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Path(path) => path.as_ident().map(|i| i.as_str().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_with(entries: Vec<(&str, SymbolEntry)>) -> ApplyGraph {
        let mut g = ApplyGraph::new();
        for (name, entry) in entries {
            g.insert(name, entry);
        }
        g
    }

    #[test]
    fn single_kernel_strict_leaf_classifies_as_kernel() {
        let g = graph_with(vec![
            (
                "thm_a",
                SymbolEntry::Theorem {
                    apply_targets: vec!["kernel_truncate_to_level_strict".to_string()],
                },
            ),
            ("kernel_truncate_to_level_strict", SymbolEntry::KernelBridge),
        ]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.kernel_strict, 1);
        assert_eq!(comp.placeholder_axiom, 0);
        assert!(comp.is_l4_load_bearing());
    }

    #[test]
    fn placeholder_axiom_breaks_l4_load_bearing() {
        // thm_a → apply some_internal_axiom (placeholder).
        let g = graph_with(vec![
            (
                "thm_a",
                SymbolEntry::Theorem {
                    apply_targets: vec!["some_internal_axiom".to_string()],
                },
            ),
            ("some_internal_axiom", SymbolEntry::PlaceholderAxiom),
        ]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.placeholder_axiom, 1);
        assert_eq!(comp.kernel_strict, 0);
        assert!(!comp.is_l4_load_bearing());
    }

    #[test]
    fn framework_axiom_is_l4_load_bearing() {
        // External-citation axioms (Lurie HTT, etc.) are L4-acceptable.
        let g = graph_with(vec![
            (
                "thm_a",
                SymbolEntry::Theorem {
                    apply_targets: vec!["msfs_lemma_A_2_lurie_htt".to_string()],
                },
            ),
            ("msfs_lemma_A_2_lurie_htt", SymbolEntry::FrameworkAxiom),
        ]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.framework_axiom, 1);
        assert!(comp.is_l4_load_bearing());
    }

    #[test]
    fn transitive_chain_descends_through_full_form() {
        // The MSFS pattern: corpus_thm → stdlib_full → kernel_strict.
        let g = graph_with(vec![
            (
                "corpus_thm",
                SymbolEntry::Theorem {
                    apply_targets: vec!["stdlib_full".to_string()],
                },
            ),
            (
                "stdlib_full",
                SymbolEntry::Theorem {
                    apply_targets: vec![
                        "kernel_step_1_strict".to_string(),
                        "kernel_step_2_strict".to_string(),
                    ],
                },
            ),
            ("kernel_step_1_strict", SymbolEntry::KernelBridge),
            ("kernel_step_2_strict", SymbolEntry::KernelBridge),
        ]);
        let comp = walk_transitive(&g, "corpus_thm", 8);
        assert_eq!(comp.kernel_strict, 2);
        assert_eq!(comp.placeholder_axiom, 0);
        assert!(comp.is_l4_load_bearing());
    }

    #[test]
    fn placeholder_in_intermediate_full_form_breaks_chain() {
        // The defect this gate exists to catch: stdlib_full bottoms
        // out at a placeholder.  The corpus theorem looked clean to
        // the immediate-discharge gate but its transitive chain leaks.
        let g = graph_with(vec![
            (
                "corpus_thm",
                SymbolEntry::Theorem {
                    apply_targets: vec!["stdlib_full".to_string()],
                },
            ),
            (
                "stdlib_full",
                SymbolEntry::Theorem {
                    apply_targets: vec!["placeholder_step".to_string()],
                },
            ),
            ("placeholder_step", SymbolEntry::PlaceholderAxiom),
        ]);
        let comp = walk_transitive(&g, "corpus_thm", 8);
        assert_eq!(comp.placeholder_axiom, 1);
        assert!(!comp.is_l4_load_bearing());
        // The chain captures the whole path so the report can pinpoint
        // which intermediate node leaks.
        assert_eq!(
            comp.leaves[0].chain,
            vec![
                "corpus_thm".to_string(),
                "stdlib_full".to_string(),
                "placeholder_step".to_string(),
            ],
        );
    }

    #[test]
    fn unresolved_apply_target_classifies_as_unresolved() {
        let g = graph_with(vec![(
            "thm_a",
            SymbolEntry::Theorem {
                apply_targets: vec!["some_undeclared_symbol".to_string()],
            },
        )]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.unresolved, 1);
        assert!(!comp.is_l4_load_bearing());
    }

    #[test]
    fn kernel_prefixed_unresolved_classifies_as_kernel_strict() {
        // Kernel bridges declared in the verum stdlib (outside the
        // audited corpus tree) appear as un-symbol-tabled apply
        // targets.  The `kernel_` naming convention is the
        // architectural contract — name-based fallback classifies
        // them as KernelStrict so the corpus's L4 verdict isn't
        // penalised for the workspace boundary.
        let g = graph_with(vec![(
            "thm_a",
            SymbolEntry::Theorem {
                apply_targets: vec!["kernel_truncate_to_level_strict".to_string()],
            },
        )]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.kernel_strict, 1);
        assert_eq!(comp.unresolved, 0);
        assert!(comp.is_l4_load_bearing());
    }

    #[test]
    fn cycle_detection_terminates() {
        // a → b → a (cycle).  Walker must terminate; both nodes are
        // theorems with no real leaves, so the composition is empty.
        let g = graph_with(vec![
            (
                "a",
                SymbolEntry::Theorem {
                    apply_targets: vec!["b".to_string()],
                },
            ),
            (
                "b",
                SymbolEntry::Theorem {
                    apply_targets: vec!["a".to_string()],
                },
            ),
        ]);
        let comp = walk_transitive(&g, "a", 8);
        // Both nodes are theorems, neither produces a leaf, the cycle
        // breaks at the visited check on second `a` — total leaves 0.
        assert_eq!(comp.total(), 0);
    }

    #[test]
    fn depth_cap_surfaces_as_unresolved() {
        // Chain longer than max_depth → the deepest node hits the
        // cap and surfaces as Unresolved (display-friendly stop).
        let g = graph_with(vec![
            (
                "a",
                SymbolEntry::Theorem {
                    apply_targets: vec!["b".to_string()],
                },
            ),
            (
                "b",
                SymbolEntry::Theorem {
                    apply_targets: vec!["c".to_string()],
                },
            ),
            ("c", SymbolEntry::KernelBridge),
        ]);
        // max_depth=2 → walks a, then b, but `c` would need depth 3.
        let comp = walk_transitive(&g, "a", 2);
        assert_eq!(comp.unresolved, 1);
        assert_eq!(comp.kernel_strict, 0);
    }

    #[test]
    fn mixed_chain_classifies_each_leaf_separately() {
        // thm_a has 3 apply targets, each a different leaf class.
        let g = graph_with(vec![
            (
                "thm_a",
                SymbolEntry::Theorem {
                    apply_targets: vec![
                        "kernel_strict_x".to_string(),
                        "lurie_htt_axiom".to_string(),
                        "internal_placeholder".to_string(),
                    ],
                },
            ),
            ("kernel_strict_x", SymbolEntry::KernelBridge),
            ("lurie_htt_axiom", SymbolEntry::FrameworkAxiom),
            ("internal_placeholder", SymbolEntry::PlaceholderAxiom),
        ]);
        let comp = walk_transitive(&g, "thm_a", 8);
        assert_eq!(comp.kernel_strict, 1);
        assert_eq!(comp.framework_axiom, 1);
        assert_eq!(comp.placeholder_axiom, 1);
        assert_eq!(comp.total(), 3);
        assert!(!comp.is_l4_load_bearing());
    }

    #[test]
    fn leaf_kind_label_is_stable() {
        // The label is part of the audit-report contract — pin it so
        // downstream tooling can rely on the four canonical strings.
        assert_eq!(LeafKind::KernelStrict.label(), "kernel_strict");
        assert_eq!(LeafKind::FrameworkAxiom.label(), "framework_axiom");
        assert_eq!(LeafKind::PlaceholderAxiom.label(), "placeholder_axiom");
        assert_eq!(LeafKind::Unresolved.label(), "unresolved");
    }

    #[test]
    fn foreign_framework_prefixes_classify_as_framework_axiom() {
        // Recognised foreign-framework prefixes should land on
        // `FrameworkAxiom`, not `Unresolved`.  This covers the
        // `kernel_v0/lemmas/*.vr` apply-chain pattern.
        for prefix in &[
            "mathlib4.lambda.ChurchRosser",
            "mathlib.set_theory.cumulative_hierarchy",
            "coq_stdlib.Logic.FunctionalExtensionality",
            "coq.SetTheory.Zermelo_Fraenkel",
            "lean4_stdlib.Function.funext",
            "lean_stdlib.Init.Logic",
            "lean4.Mathlib.CategoryTheory.Closed.Cartesian",
            "zfc.extensionality",
            "zfc.foundation",
            "agda_stdlib.Data.Nat",
            "isabelle.HOL.Real",
        ] {
            assert!(
                is_foreign_framework_target(prefix),
                "prefix `{}` should classify as foreign-framework target",
                prefix,
            );
        }
    }

    #[test]
    fn unrecognised_names_do_not_classify_as_framework() {
        for name in &[
            "some_internal_axiom",
            "msfs_lemma_3_4",
            "thm_undeclared",
            "k_var_sound",
        ] {
            assert!(
                !is_foreign_framework_target(name),
                "name `{}` must NOT classify as foreign-framework target",
                name,
            );
        }
    }

    #[test]
    fn foreign_framework_apply_chain_is_l4_load_bearing() {
        // Pattern: theorem applies a foreign-framework target by name
        // (no workspace declaration).  The walker's name-based
        // fallback must recognise the prefix and classify as
        // `FrameworkAxiom`, keeping the L4 verdict load-bearing.
        let g = graph_with(vec![(
            "church_rosser_confluence",
            SymbolEntry::Theorem {
                apply_targets: vec!["mathlib4.lambda.ChurchRosser".to_string()],
            },
        )]);
        let comp = walk_transitive(&g, "church_rosser_confluence", 8);
        assert_eq!(comp.framework_axiom, 1);
        assert_eq!(comp.unresolved, 0);
        assert!(
            comp.is_l4_load_bearing(),
            "framework-cited apply chain must keep L4 load-bearing verdict",
        );
    }
}
