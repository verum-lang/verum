//! A module contributes assumptions immediately and lemmas only after discharge.
//! The dependency order schedules forward references. It never grants authority.
use super::proof_verification::{refinement_hypotheses_from_params, source_theorem_goal};
use std::collections::{BTreeSet, HashMap};
use verum_ast::decl::{FunctionParam, FunctionParamKind, Item, TheoremDecl};
use verum_ast::{BinOp, Expr, ExprKind, ItemKind, Module, PatternKind};
use verum_common::{Heap, List, Text};
use verum_smt::proof_search::{HintsDatabase, LemmaHint};

pub(crate) fn ordered<'a>(module: &'a Module, target: Option<&str>) -> Vec<&'a Item> {
    let items: Vec<_> = module
        .items
        .iter()
        .filter(|i| i.kind.proof_item_kind().is_some())
        .collect();
    let names: HashMap<_, _> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (item.kind.proof_item_name().unwrap().as_str(), i))
        .collect();
    let mut dependencies = vec![BTreeSet::new(); items.len()];
    let mut users = vec![Vec::new(); items.len()];
    for (i, item) in items.iter().enumerate() {
        if let Some(body) = item.kind.as_theorem_decl().and_then(|t| t.proof.as_ref()) {
            for name in verum_kernel::soundness::apply_graph::source_lemma_names(body) {
                if let Some(&j) = names.get(name.as_str())
                    && dependencies[i].insert(j)
                {
                    users[j].push(i);
                }
            }
        }
    }
    let mut needed = vec![target.is_none(); items.len()];
    if let Some(name) = target {
        let Some(&i) = names.get(name) else {
            return Vec::new();
        };
        let mut todo = vec![i];
        while let Some(j) = todo.pop() {
            if !std::mem::replace(&mut needed[j], true) {
                todo.extend(&dependencies[j]);
            }
        }
    }
    let mut counts: Vec<_> = dependencies.iter().map(BTreeSet::len).collect();
    let mut ready: BTreeSet<_> = (0..items.len())
        .filter(|&i| needed[i] && counts[i] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(i) = ready.pop_first() {
        needed[i] = false;
        order.push(items[i]);
        for &j in &users[i] {
            counts[j] -= 1;
            if needed[j] && counts[j] == 0 {
                ready.insert(j);
            }
        }
    }
    // A syntactic cycle might contain an independently successful alternative.
    // Such declarations still run with the currently established environment.
    order.extend(
        items
            .into_iter()
            .zip(needed)
            .filter_map(|(item, pending)| pending.then_some(item)),
    );
    order
}

fn insert_hint(
    hints: &mut HintsDatabase,
    name: &Text,
    params: &[FunctionParam],
    goal: Expr,
    hypotheses: &[Expr],
) {
    let Some(formals): Option<List<Text>> = params
        .iter()
        .map(|p| match &p.kind {
            FunctionParamKind::Regular { pattern, .. } => match &pattern.kind {
                PatternKind::Ident {
                    name,
                    by_ref: false,
                    subpattern: None,
                    ..
                } => Some(name.name.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
    else {
        return;
    };
    let proposition = hypotheses.iter().rev().fold(goal, |conclusion, premise| {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Imply,
                left: Heap::new(premise.clone()),
                right: Heap::new(conclusion),
            },
            premise.span,
        )
    });
    hints.register_lemma(
        name.clone(),
        LemmaHint {
            params: formals,
            name: name.clone(),
            priority: 500,
            lemma: Heap::new(proposition),
        },
    );
}

pub(crate) fn insert_theorem(
    hints: &mut HintsDatabase,
    theorem: &TheoremDecl,
    aliases: &HashMap<Text, Vec<Expr>>,
    variants: &HashMap<Text, Vec<Text>>,
) {
    let judgment = source_theorem_goal(theorem, aliases, variants);
    insert_hint(
        hints,
        &theorem.name.name,
        &theorem.params,
        judgment.goal,
        &judgment.hypotheses,
    );
}

pub(crate) fn seed_assumptions(
    hints: &mut HintsDatabase,
    items: &[&Item],
    aliases: &HashMap<Text, Vec<Expr>>,
    variants: &HashMap<Text, Vec<Text>>,
) {
    for &item in items {
        match &item.kind {
            ItemKind::Axiom(a) => insert_hint(
                hints,
                &a.name.name,
                &a.params,
                a.proposition.as_ref().clone(),
                &refinement_hypotheses_from_params(&a.params, aliases),
            ),
            kind => {
                if let Some(t) = kind.as_theorem_decl()
                    && t.proof.is_none()
                {
                    insert_theorem(hints, t, aliases, variants);
                }
            }
        }
    }
}
