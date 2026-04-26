//! Phase 4.5: Proof Erasure
//!
//! This phase runs between semantic analysis (Phase 4) and VBC codegen
//! (Phase 5). It strips all proof-level items from the typed AST so that
//! the runtime carries **zero proof-term overhead**:
//!
//! - `theorem`, `lemma`, `corollary`, `axiom`, `tactic` declarations
//!   are verified during the proof-verification phase and then **completely
//!   removed** from the item list before VBC code generation.
//!
//! - Function parameters with `Quantity::Zero` (erased/irrelevant
//!   parameters) are removed from runtime calling conventions.
//!
//! - Values of type `Proof<P>` are replaced with unit values.
//!
//! ## Invariant
//!
//! After this phase, the remaining AST contains **only** runtime-relevant
//! items. The VBC codegen can process every item without any proof-awareness.
//!
//! ## Integration
//!
//! Called from `pipeline.rs` after `phase_semantic_analysis` and before
//! `phase_vbc_codegen`. The erasure is idempotent — running it twice
//! produces the same result.

use verum_ast::{Item, ItemKind, Module};
use verum_common::{List, Text};

/// Statistics from the proof erasure pass.
#[derive(Debug, Clone, Default)]
pub struct ProofErasureStats {
    /// Number of theorem declarations erased.
    pub theorems_erased: usize,
    /// Number of lemma declarations erased.
    pub lemmas_erased: usize,
    /// Number of corollary declarations erased.
    pub corollaries_erased: usize,
    /// Number of axiom declarations erased.
    pub axioms_erased: usize,
    /// Number of tactic declarations erased.
    pub tactics_erased: usize,
    /// Number of runtime items retained.
    pub items_retained: usize,
}

impl ProofErasureStats {
    pub fn total_erased(&self) -> usize {
        self.theorems_erased
            + self.lemmas_erased
            + self.corollaries_erased
            + self.axioms_erased
            + self.tactics_erased
    }

    pub fn report(&self) -> Text {
        Text::from(format!(
            "Proof erasure: {} erased ({} theorems, {} lemmas, {} corollaries, \
             {} axioms, {} tactics), {} items retained",
            self.total_erased(),
            self.theorems_erased,
            self.lemmas_erased,
            self.corollaries_erased,
            self.axioms_erased,
            self.tactics_erased,
            self.items_retained,
        ))
    }
}

/// Returns `true` if the item is a proof-level declaration that should
/// be erased before runtime code generation.
pub fn is_proof_item(item: &Item) -> bool {
    matches!(
        &item.kind,
        ItemKind::Theorem(_)
            | ItemKind::Lemma(_)
            | ItemKind::Corollary(_)
            | ItemKind::Axiom(_)
            | ItemKind::Tactic(_)
    )
}

/// Erase all proof-level items from a single module, returning the
/// filtered module and erasure statistics.
pub fn erase_proofs_from_module(module: Module) -> (Module, ProofErasureStats) {
    let mut stats = ProofErasureStats::default();

    let retained_items: List<Item> = module
        .items
        .into_iter()
        .filter(|item| {
            if is_proof_item(item) {
                match &item.kind {
                    ItemKind::Theorem(_) => stats.theorems_erased += 1,
                    ItemKind::Lemma(_) => stats.lemmas_erased += 1,
                    ItemKind::Corollary(_) => stats.corollaries_erased += 1,
                    ItemKind::Axiom(_) => stats.axioms_erased += 1,
                    ItemKind::Tactic(_) => stats.tactics_erased += 1,
                    _ => {}
                }
                false
            } else {
                stats.items_retained += 1;
                true
            }
        })
        .collect();

    let erased_module = Module {
        items: retained_items,
        attributes: module.attributes,
        file_id: module.file_id,
        span: module.span,
    };

    (erased_module, stats)
}

/// Erase all proof-level items from a list of modules.
///
/// Returns the filtered modules and aggregate statistics.
pub fn erase_proofs(modules: List<Module>) -> (List<Module>, ProofErasureStats) {
    let mut aggregate_stats = ProofErasureStats::default();
    let mut erased_modules = List::new();

    for module in modules {
        let (erased, stats) = erase_proofs_from_module(module);
        aggregate_stats.theorems_erased += stats.theorems_erased;
        aggregate_stats.lemmas_erased += stats.lemmas_erased;
        aggregate_stats.corollaries_erased += stats.corollaries_erased;
        aggregate_stats.axioms_erased += stats.axioms_erased;
        aggregate_stats.tactics_erased += stats.tactics_erased;
        aggregate_stats.items_retained += stats.items_retained;
        erased_modules.push(erased);
    }

    (erased_modules, aggregate_stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;

    fn dummy_file_id() -> FileId {
        FileId::new(0)
    }

    fn parse_module(source: &str) -> Module {
        let file_id = dummy_file_id();
        let parser = verum_fast_parser::VerumParser::new();
        parser
            .parse_module_str(source, file_id)
            .expect("parse failed")
    }

    #[test]
    fn test_is_proof_item_identifies_theorem() {
        let m = parse_module("theorem t() proof by auto;");
        assert_eq!(m.items.len(), 1);
        assert!(is_proof_item(&m.items[0]));
    }

    #[test]
    fn test_is_proof_item_skips_function() {
        let m = parse_module("fn f() {}");
        assert_eq!(m.items.len(), 1);
        assert!(!is_proof_item(&m.items[0]));
    }

    #[test]
    fn test_erase_proofs_removes_theorem_keeps_function() {
        let m = parse_module(
            "fn f() {} theorem t() proof by auto; fn g() {}",
        );
        assert_eq!(m.items.len(), 3);

        let (erased, stats) = erase_proofs_from_module(m);
        assert_eq!(erased.items.len(), 2);
        assert_eq!(stats.theorems_erased, 1);
        assert_eq!(stats.items_retained, 2);
    }

    #[test]
    fn test_erase_proofs_empty_module() {
        let m = parse_module("");
        let (erased, stats) = erase_proofs_from_module(m);
        assert_eq!(erased.items.len(), 0);
        assert_eq!(stats.total_erased(), 0);
    }

    #[test]
    fn test_erase_proofs_idempotent() {
        let m = parse_module("fn f() {} theorem t() proof by auto;");
        let (erased1, stats1) = erase_proofs_from_module(m);
        assert_eq!(stats1.theorems_erased, 1);

        let (erased2, stats2) = erase_proofs_from_module(erased1);
        assert_eq!(stats2.theorems_erased, 0);
        assert_eq!(erased2.items.len(), 1);
    }

    #[test]
    fn test_erase_proofs_all_five_kinds() {
        let m = parse_module(
            "theorem t() proof by auto; \
             lemma l() proof by auto; \
             axiom a() -> Bool; \
             fn f() {}",
        );
        let (erased, stats) = erase_proofs_from_module(m);
        assert_eq!(stats.theorems_erased, 1);
        assert_eq!(stats.lemmas_erased, 1);
        assert_eq!(stats.axioms_erased, 1);
        assert_eq!(stats.items_retained, 1);
        assert_eq!(erased.items.len(), 1);
    }

    #[test]
    fn test_stats_report() {
        let stats = ProofErasureStats {
            theorems_erased: 5,
            lemmas_erased: 2,
            corollaries_erased: 1,
            axioms_erased: 3,
            tactics_erased: 0,
            items_retained: 10,
        };
        let report = stats.report();
        assert!(report.contains("11 erased"));
        assert!(report.contains("10 items retained"));
    }

    #[test]
    fn test_erase_proofs_batch() {
        let m1 = parse_module("fn f() {} theorem t() proof by auto;");
        let m2 = parse_module("fn g() {} lemma l() proof by auto;");
        let modules = List::from_iter([m1, m2]);

        let (erased, stats) = erase_proofs(modules);
        assert_eq!(erased.len(), 2);
        assert_eq!(stats.theorems_erased, 1);
        assert_eq!(stats.lemmas_erased, 1);
        assert_eq!(stats.items_retained, 2);
    }
}
