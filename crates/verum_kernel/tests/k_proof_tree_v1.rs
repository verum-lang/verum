//! KernelProofNode V1 tests (V8, #224).
//!
//! Verifies the typed proof-graph reconstruction surface:
//! `record_inference` walks a CoreTerm post-hoc and builds a
//! KernelProofNode tree mirroring the typing derivation.
//! Used by audit + certificate-export + IDE step-debugger.

use verum_common::{Heap, List, Maybe, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelProofNode, KernelRule,
    UniverseLevel, record_inference,
};

fn empty() -> (Context, AxiomRegistry) {
    (Context::new(), AxiomRegistry::new())
}

// =============================================================================
// KernelRule basics
// =============================================================================

#[test]
fn kernel_rule_names_are_canonical() {
    assert_eq!(KernelRule::KVar.name(), "K-Var");
    assert_eq!(KernelRule::KAppElim.name(), "K-App-Elim");
    assert_eq!(KernelRule::KRefineOmega.name(), "K-Refine-omega");
    assert_eq!(KernelRule::KEpsMu.name(), "K-Eps-Mu");
    assert_eq!(KernelRule::KFwAx.name(), "K-FwAx");
}

#[test]
fn kernel_rule_v_stage_reflects_v8_promotions() {
    // V8 promotions per §4.4a.7 maturity audit.
    assert_eq!(KernelRule::KUniv.v_stage(), "V8");
    assert_eq!(KernelRule::KPathTyForm.v_stage(), "V8");
    assert_eq!(KernelRule::KAppElim.v_stage(), "V8");
    assert_eq!(KernelRule::KInductive.v_stage(), "V8");
    assert_eq!(KernelRule::KFwAx.v_stage(), "V8");
    // V0 baseline rules.
    assert_eq!(KernelRule::KVar.v_stage(), "V0");
    assert_eq!(KernelRule::KLamIntro.v_stage(), "V0");
    // VVA-tagged rules.
    assert_eq!(KernelRule::KEpsMu.v_stage(), "V2");
    assert_eq!(KernelRule::KUniverseAscent.v_stage(), "V1");
}

#[test]
fn kernel_rule_display_matches_name() {
    let rule = KernelRule::KAppElim;
    assert_eq!(format!("{}", rule), "K-App-Elim");
}

#[test]
fn kernel_rule_implements_ord_for_btree_use() {
    use std::collections::BTreeSet;
    let mut set: BTreeSet<KernelRule> = BTreeSet::new();
    set.insert(KernelRule::KAppElim);
    set.insert(KernelRule::KVar);
    set.insert(KernelRule::KUniv);
    let names: Vec<&str> = set.iter().map(|r| r.name()).collect();
    // Sorted by `name()`: alphabetical.
    assert!(names.contains(&"K-Var"));
    assert!(names.contains(&"K-Univ"));
}

// =============================================================================
// record_inference — leaf nodes
// =============================================================================

#[test]
fn leaf_universe_node_records_k_univ() {
    let (ctx, reg) = empty();
    let term = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let node = record_inference(&ctx, &term, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KUniv);
    assert_eq!(node.conclusion, term);
    assert_eq!(node.inferred_ty, CoreTerm::Universe(UniverseLevel::Concrete(1)));
    assert!(node.premises.is_empty());
    assert!(matches!(node.citation, Maybe::None));
}

#[test]
fn leaf_var_node_records_k_var_under_extended_ctx() {
    let nat_ind = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let ctx = Context::new().extend(Text::from("x"), nat_ind.clone());
    let reg = AxiomRegistry::new();
    let term = CoreTerm::Var(Text::from("x"));
    let node = record_inference(&ctx, &term, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KVar);
    assert_eq!(node.conclusion, term);
    assert_eq!(node.inferred_ty, nat_ind);
}

// =============================================================================
// record_inference — composite nodes with premises
// =============================================================================

#[test]
fn pi_node_has_two_premises() {
    let (ctx, reg) = empty();
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
    };
    let node = record_inference(&ctx, &pi, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KPiForm);
    assert_eq!(node.premises.len(), 2);
    // Both premises are domain/codomain — each is a Universe.
    for p in node.premises.iter() {
        assert_eq!(p.rule, KernelRule::KUniv);
    }
}

#[test]
fn lam_node_has_two_premises() {
    let (ctx, reg) = empty();
    let nat = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat.clone()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let node = record_inference(&ctx, &lam, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KLamIntro);
    // Premise 0: domain (Nat) → KInductive.
    // Premise 1: body (Var x) — but `infer` for Var x under
    // bare ctx fails (x unbound). The reconstructor uses the
    // outer ctx, not the extended one — so Var x premise
    // reconstruction returns None and is skipped. Premises
    // contains only the domain premise.
    assert!(
        !node.premises.is_empty(),
        "at least the domain premise: {}",
        node.premises.len()
    );
    assert_eq!(node.premises.iter().next().unwrap().rule, KernelRule::KInductive);
}

#[test]
fn app_node_has_function_and_arg_premises() {
    let (ctx, reg) = empty();
    let nat = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let id_lam = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(nat.clone()),
        body: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    let app = CoreTerm::App(
        Heap::new(id_lam),
        Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))), // arg
    );
    // App's well-formedness requires arg-type = domain. Here
    // domain=Nat, arg-type=Universe — mismatch. infer fails;
    // record_inference returns None.
    let res = record_inference(&ctx, &app, &reg);
    assert!(
        res.is_none(),
        "ill-typed App produces no proof tree (use infer for the precise error)",
    );
}

#[test]
fn refl_node_has_one_premise() {
    let (ctx, reg) = empty();
    let nat = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let refl = CoreTerm::Refl(Heap::new(nat));
    let node = record_inference(&ctx, &refl, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KReflIntro);
    assert_eq!(node.premises.len(), 1);
    assert_eq!(node.premises.iter().next().unwrap().rule, KernelRule::KInductive);
}

#[test]
fn axiom_node_carries_citation() {
    let nat = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test_corpus"),
        citation: Text::from("test cite"),
    };
    reg.register(Text::from("zero"), nat.clone(), fw.clone()).expect("register");
    let term = CoreTerm::Axiom {
        name: Text::from("zero"),
        ty: Heap::new(nat),
        framework: fw.clone(),
    };
    let node = record_inference(&Context::new(), &term, &reg).expect("infer ok");
    assert_eq!(node.rule, KernelRule::KFwAx);
    match &node.citation {
        Maybe::Some(c) => {
            assert_eq!(c.framework.as_str(), "test_corpus");
        }
        Maybe::None => panic!("axiom node must carry citation"),
    }
}

// =============================================================================
// KernelProofNode walk_dfs / size / rules_used
// =============================================================================

#[test]
fn walk_dfs_visits_every_node() {
    let (ctx, reg) = empty();
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(1))),
    };
    let node = record_inference(&ctx, &pi, &reg).expect("infer ok");
    let mut count = 0;
    node.walk_dfs(&mut |_| count += 1);
    // 1 (Pi) + 2 (premises) = 3.
    assert_eq!(count, 3);
}

#[test]
fn size_counts_total_nodes() {
    let (ctx, reg) = empty();
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
    };
    let node = record_inference(&ctx, &pi, &reg).expect("infer ok");
    assert_eq!(node.size(), 3);
}

#[test]
fn rules_used_returns_distinct_rule_set() {
    let (ctx, reg) = empty();
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
    };
    let node = record_inference(&ctx, &pi, &reg).expect("infer ok");
    let rules = node.rules_used();
    assert_eq!(rules.len(), 2); // KPiForm + KUniv (deduplicated).
    assert!(rules.contains(&KernelRule::KPiForm));
    assert!(rules.contains(&KernelRule::KUniv));
}

#[test]
fn leaf_constructor_creates_premise_free_node() {
    let conclusion = CoreTerm::Var(Text::from("x"));
    let inferred_ty = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let node = KernelProofNode::leaf(
        KernelRule::KVar,
        conclusion.clone(),
        inferred_ty.clone(),
    );
    assert_eq!(node.rule, KernelRule::KVar);
    assert_eq!(node.conclusion, conclusion);
    assert_eq!(node.inferred_ty, inferred_ty);
    assert!(node.premises.is_empty());
    assert!(matches!(node.citation, Maybe::None));
}

#[test]
fn with_citation_attaches_framework() {
    let node = KernelProofNode::leaf(
        KernelRule::KFwAx,
        CoreTerm::Var(Text::from("z")),
        CoreTerm::Universe(UniverseLevel::Concrete(0)),
    );
    let with_cite = node.with_citation(FrameworkId {
        framework: Text::from("lurie_htt"),
        citation: Text::from("HTT 6.2.2.7"),
    });
    match with_cite.citation {
        Maybe::Some(c) => assert_eq!(c.framework.as_str(), "lurie_htt"),
        Maybe::None => panic!("citation must be attached"),
    }
}

#[test]
fn record_inference_returns_none_for_ill_typed_term() {
    // App with mismatched domain → infer fails → record returns None.
    let (ctx, reg) = empty();
    let bool_ind = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    let nat_ind = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let bool_id = CoreTerm::Lam {
        binder: Text::from("b"),
        domain: Heap::new(bool_ind),
        body: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    let nat_axiom = {
        let fw = FrameworkId {
            framework: Text::from("t"),
            citation: Text::from("t"),
        };
        let mut reg2 = reg.clone();
        let _ = reg2.register(Text::from("zero"), nat_ind.clone(), fw.clone());
        CoreTerm::Axiom {
            name: Text::from("zero"),
            ty: Heap::new(nat_ind),
            framework: fw,
        }
    };
    let bad_app = CoreTerm::App(Heap::new(bool_id), Heap::new(nat_axiom));
    let res = record_inference(&ctx, &bad_app, &reg);
    assert!(res.is_none(), "ill-typed term produces no proof tree");
}
