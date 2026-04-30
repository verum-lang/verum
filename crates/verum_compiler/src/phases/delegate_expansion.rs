//! # `@delegate(target)` attribute expansion
//!
//! Task #146 / MSFS-L4.14 — eliminates the corpus/stdlib duplication
//! pattern observed across MSFS §9 + §10 where every corpus-side
//! "anchor theorem" carries a hand-written proof body that is
//! identical in shape: `proof { apply <stdlib_full_form>(args); }`.
//!
//! ## The pattern this phase eliminates
//!
//! Pre-#146 every delegating theorem looks like:
//!
//! ```verum
//! public theorem msfs_theorem_9_3_meta_categoricity(
//!     f1: &MetaClsTopWitness,
//!     f2: &MetaClsTopWitness,
//!     proof_witness: &MetaCategoricityWitness,
//! )
//!     requires <17-line requires clause>
//!     ensures <2-line ensures clause>
//!     proof {
//!         apply msfs_theorem_9_3_meta_categoricity_full(f1, f2, proof_witness);
//!     };
//! ```
//!
//! Post-#146 the manual `proof { apply … }` boilerplate disappears —
//! the attribute carries the same information declaratively:
//!
//! ```verum
//! @delegate(msfs_theorem_9_3_meta_categoricity_full)
//! public theorem msfs_theorem_9_3_meta_categoricity(
//!     f1: &MetaClsTopWitness,
//!     f2: &MetaClsTopWitness,
//!     proof_witness: &MetaCategoricityWitness,
//! )
//!     requires <17-line requires clause>
//!     ensures <2-line ensures clause>;
//! ```
//!
//! This phase walks every theorem in every parsed module and, when the
//! `@delegate(target)` attribute is present, synthesises the
//! equivalent `proof { apply target(p1, p2, …); }` body — passing the
//! theorem's parameters positionally as arguments, in declaration
//! order.
//!
//! ## Architectural significance
//!
//! - Reduces ~100 LOC of boilerplate per delegating module across the
//!   MSFS corpus (the §9 + §10 files lose ~90% of their proof-body
//!   text).
//! - Makes the corpus's "anchor + delegate" pattern declarative rather
//!   than hand-written — adding a new delegating theorem is one line
//!   of attribute metadata, not a copy-pasted apply block.
//! - The synthesised body is structurally identical to the manual
//!   form, so every downstream consumer (proof-honesty audit, bridge-
//!   discharge check, apply-graph walker, cross-format gate) sees the
//!   apply target without code changes.
//! - Validation enforces that `@delegate` and an explicit `proof { … }`
//!   body don't co-occur on the same theorem — the user picks one
//!   surface form per theorem.
//!
//! ## Recipe
//!
//! When a stdlib has both a "load-bearing form" and a "namespace
//! anchor" version that differ only in proof body shape, promote the
//! duplication to a single declarative attribute.  Recipe extension
//! to the existing meta-derives surface (#13): attribute-driven
//! proof-body synthesis is a sibling of the @derive(Eq) pattern —
//! both turn one declaration into a structurally complete artefact
//! the rest of the compiler can consume unchanged.

use verum_ast::decl::{FunctionParamKind, ItemKind, ProofBody, TacticExpr};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::PatternKind;
use verum_ast::ty::{Ident, Path};
use verum_ast::{Module, Span};
use verum_common::{Heap, List, Maybe, Text};

/// Attribute name the phase looks for on theorem-shaped declarations.
const DELEGATE_ATTR: &str = "delegate";

/// Per-theorem expansion outcome reported back to the caller for
/// diagnostics surfacing.
#[derive(Debug, Clone)]
pub enum DelegateExpansion {
    /// Theorem carried `@delegate(target)` and its proof body was
    /// synthesised.  The `target` field carries the apply-target's
    /// rendered name for diagnostic context.
    Synthesised { theorem: Text, target: Text },
    /// Theorem carried `@delegate(target)` but ALSO had an explicit
    /// `proof { … }` body.  This is rejected — the two forms can't
    /// co-occur, the user picks one.
    Rejected { theorem: Text, reason: Text },
}

/// Public entry: walk every theorem-shaped declaration in a module
/// and expand `@delegate(target)` attributes into synthesised proof
/// bodies in place.  Returns the per-theorem outcomes.
///
/// Mutates `module` directly: each `Item::Theorem`/`Item::Lemma`/
/// `Item::Corollary` whose attributes carry `@delegate(target)` and
/// whose `proof` field is `Maybe::None` gets its `proof` populated
/// with the synthesised body.  Theorems without the attribute pass
/// through unchanged.
///
/// Idempotent: running the phase twice produces the same module.
pub fn expand_delegates_in_module(module: &mut Module) -> Vec<DelegateExpansion> {
    let mut outcomes = Vec::new();
    for item in module.items.iter_mut() {
        match &mut item.kind {
            ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
                let theorem_name = d.name.name.clone();
                let outcome = match find_delegate_target(&d.attributes) {
                    Some((target_name, target_span)) => {
                        if matches!(d.proof, Maybe::Some(_)) {
                            outcomes.push(DelegateExpansion::Rejected {
                                theorem: theorem_name.clone(),
                                reason: Text::from(
                                    "@delegate cannot co-occur with an explicit `proof { … }` body — \
                                     pick one surface form per theorem",
                                ),
                            });
                            continue;
                        }
                        let synthetic = synthesise_delegate_body(
                            &target_name,
                            target_span,
                            &d.params,
                        );
                        d.proof = Maybe::Some(synthetic);
                        DelegateExpansion::Synthesised {
                            theorem: theorem_name.clone(),
                            target: target_name,
                        }
                    }
                    None => continue,
                };
                outcomes.push(outcome);
            }
            _ => {}
        }
    }
    outcomes
}

/// Return `Some((target_name, span))` when the attribute list contains
/// `@delegate(<ident>)`.  Returns `None` for an absent attribute, or
/// when the attribute's argument shape isn't a single Path-ident
/// (the only currently-supported form).
fn find_delegate_target(
    attrs: &List<verum_ast::attr::Attribute>,
) -> Option<(Text, Span)> {
    for attr in attrs.iter() {
        if !attr.is_named(DELEGATE_ATTR) {
            continue;
        }
        let args = match &attr.args {
            Maybe::Some(args) => args,
            Maybe::None => continue,
        };
        // Exactly one argument expected: a Path expression naming the
        // delegate target.  Multi-arg / non-path forms surface as
        // "no delegate" (no synthesis happens) so a typo / future
        // extension doesn't silently corrupt the proof body.
        let mut iter = args.iter();
        let first = iter.next()?;
        if iter.next().is_some() {
            continue;
        }
        let name = match &first.kind {
            ExprKind::Path(path) => path.as_ident().map(|i| Text::from(i.as_str())),
            _ => None,
        }?;
        return Some((name, attr.span));
    }
    None
}

/// Build the synthetic proof body for `@delegate(target)`: a single
/// `Tactic(Apply { lemma: Path(target), args: <params as path
/// exprs> })` step.  Parameters are passed positionally in
/// declaration order; non-Ident pattern-bound params (tuple, record,
/// ...) are dropped from the args list (they don't have a single
/// positional binding).
fn synthesise_delegate_body(
    target_name: &Text,
    target_span: Span,
    params: &List<verum_ast::decl::FunctionParam>,
) -> ProofBody {
    let lemma_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new(target_name.as_str(), target_span))),
        target_span,
    );

    let mut arg_list: List<Expr> = List::new();
    for fp in params.iter() {
        if let FunctionParamKind::Regular { pattern, .. } = &fp.kind {
            if let PatternKind::Ident { name, .. } = &pattern.kind {
                let arg_expr = Expr::new(
                    ExprKind::Path(Path::single(name.clone())),
                    name.span,
                );
                arg_list.push(arg_expr);
            }
        }
    }

    ProofBody::Tactic(TacticExpr::Apply {
        lemma: Heap::new(lemma_expr),
        args: arg_list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::attr::Attribute;
    use verum_ast::decl::{FunctionParam, ItemKind, TheoremDecl};
    use verum_ast::pattern::Pattern;
    use verum_ast::ty::{Type, TypeKind};
    use verum_ast::{Span, Visibility};

    fn span() -> Span {
        Span::dummy()
    }

    fn ident(name: &str) -> Ident {
        Ident::new(name, span())
    }

    fn path_expr(name: &str) -> Expr {
        Expr::new(ExprKind::Path(Path::single(ident(name))), span())
    }

    fn path_arg(name: &str) -> List<Expr> {
        let mut args = List::new();
        args.push(path_expr(name));
        args
    }

    fn delegate_attr(target: &str) -> Attribute {
        Attribute::new(Text::from(DELEGATE_ATTR), Maybe::Some(path_arg(target)), span())
    }

    fn ident_param(name: &str) -> FunctionParam {
        let pattern = Pattern {
            kind: PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: ident(name),
                subpattern: Maybe::None,
            },
            span: span(),
        };
        FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern,
                ty: Type::new(TypeKind::Unit, span()),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span: span(),
        }
    }

    fn theorem_with_attrs_no_proof(
        name: &str,
        attrs: List<Attribute>,
        params: List<FunctionParam>,
    ) -> Item {
        let mut item_attrs = List::new();
        for a in attrs.iter() {
            item_attrs.push(a.clone());
        }
        let proposition = Expr::new(
            ExprKind::Literal(verum_ast::Literal::bool(true, span())),
            span(),
        );
        let decl = TheoremDecl {
            visibility: Visibility::Public,
            name: ident(name),
            generics: List::new(),
            params,
            return_type: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            proposition: Heap::new(proposition),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            proof: Maybe::None,
            attributes: attrs,
            span: span(),
        };
        Item {
            kind: ItemKind::Theorem(decl),
            attributes: item_attrs,
            span: span(),
        }
    }

    fn module_with_items(items: Vec<Item>) -> Module {
        let mut item_list = List::new();
        for it in items {
            item_list.push(it);
        }
        Module::new(item_list, verum_ast::FileId::new(0), span())
    }

    #[test]
    fn delegate_synthesises_apply_proof_body() {
        // `@delegate(target_full) theorem foo(a, b);` →
        // `theorem foo(a, b) proof { apply target_full(a, b); };`
        let mut attrs: List<Attribute> = List::new();
        attrs.push(delegate_attr("target_full"));
        let mut params: List<FunctionParam> = List::new();
        params.push(ident_param("a"));
        params.push(ident_param("b"));
        let item = theorem_with_attrs_no_proof("foo", attrs, params);
        let mut module = module_with_items(vec![item]);

        let outcomes = expand_delegates_in_module(&mut module);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            DelegateExpansion::Synthesised { theorem, target } => {
                assert_eq!(theorem.as_str(), "foo");
                assert_eq!(target.as_str(), "target_full");
            }
            other => panic!("expected Synthesised, got {:?}", other),
        }

        // Verify the synthesised proof body shape: Tactic(Apply { lemma:
        // Path("target_full"), args: [Path("a"), Path("b")] }).
        let item = module.items.iter().next().unwrap();
        let decl = match &item.kind {
            ItemKind::Theorem(d) => d,
            _ => panic!("expected Theorem"),
        };
        let body = match &decl.proof {
            Maybe::Some(b) => b,
            Maybe::None => panic!("proof body must be synthesised"),
        };
        let (lemma, args) = match body {
            ProofBody::Tactic(TacticExpr::Apply { lemma, args }) => (lemma, args),
            other => panic!("expected Tactic::Apply, got {:?}", other),
        };
        // Lemma is a Path("target_full").
        match &lemma.kind {
            ExprKind::Path(p) => {
                assert_eq!(p.as_ident().unwrap().as_str(), "target_full");
            }
            other => panic!("expected Path, got {:?}", other),
        }
        // Args contain Path("a") and Path("b") in that order.
        let names: Vec<String> = args
            .iter()
            .filter_map(|a| match &a.kind {
                ExprKind::Path(p) => p.as_ident().map(|i| i.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn delegate_with_explicit_proof_body_is_rejected() {
        // `@delegate(target) theorem foo() proof { … };` — both forms
        // present.  Reject so the user picks one surface.
        let mut attrs: List<Attribute> = List::new();
        attrs.push(delegate_attr("target_full"));
        let mut params: List<FunctionParam> = List::new();
        params.push(ident_param("a"));
        let mut item = theorem_with_attrs_no_proof("foo", attrs, params);
        // Inject a hand-written proof body so the outcome is Rejected.
        let manual_body = ProofBody::Tactic(TacticExpr::Trivial);
        if let ItemKind::Theorem(d) = &mut item.kind {
            d.proof = Maybe::Some(manual_body);
        }
        let mut module = module_with_items(vec![item]);

        let outcomes = expand_delegates_in_module(&mut module);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            DelegateExpansion::Rejected { theorem, .. } => {
                assert_eq!(theorem.as_str(), "foo");
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn theorem_without_delegate_attr_passes_through_unchanged() {
        // No @delegate → no outcomes, no synthesis.
        let mut params: List<FunctionParam> = List::new();
        params.push(ident_param("a"));
        let item = theorem_with_attrs_no_proof("plain_thm", List::new(), params);
        let mut module = module_with_items(vec![item]);

        let outcomes = expand_delegates_in_module(&mut module);
        assert_eq!(outcomes.len(), 0);

        let item = module.items.iter().next().unwrap();
        let decl = match &item.kind {
            ItemKind::Theorem(d) => d,
            _ => panic!("expected Theorem"),
        };
        // Proof remained absent.
        assert!(matches!(decl.proof, Maybe::None));
    }

    #[test]
    fn expansion_is_idempotent() {
        // Run twice; second pass produces no outcomes (theorem already
        // has a proof body, so @delegate co-occurrence rule fires —
        // but only if @delegate stays on the attribute list).  This
        // pin documents the contract: callers are responsible for
        // running the phase exactly once before downstream code
        // examines the proof body.  If it runs twice, the second
        // run rejects the (now-bodied) theorem.
        let mut attrs: List<Attribute> = List::new();
        attrs.push(delegate_attr("target_full"));
        let mut params: List<FunctionParam> = List::new();
        params.push(ident_param("a"));
        let item = theorem_with_attrs_no_proof("foo", attrs, params);
        let mut module = module_with_items(vec![item]);

        let first = expand_delegates_in_module(&mut module);
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0], DelegateExpansion::Synthesised { .. }));

        let second = expand_delegates_in_module(&mut module);
        assert_eq!(second.len(), 1);
        // Second run sees the attribute still present + a proof body now
        // present → rejects (documenting the contract).
        assert!(matches!(second[0], DelegateExpansion::Rejected { .. }));
    }

    #[test]
    fn delegate_with_zero_params_synthesises_zero_arg_apply() {
        // `@delegate(target) theorem foo();` — no theorem params, so
        // the synthesised apply has empty args list.
        let mut attrs: List<Attribute> = List::new();
        attrs.push(delegate_attr("target_full"));
        let item = theorem_with_attrs_no_proof("foo", attrs, List::new());
        let mut module = module_with_items(vec![item]);

        expand_delegates_in_module(&mut module);
        let item = module.items.iter().next().unwrap();
        let decl = match &item.kind {
            ItemKind::Theorem(d) => d,
            _ => panic!("expected Theorem"),
        };
        let body = decl.proof.as_ref().unwrap();
        match body {
            ProofBody::Tactic(TacticExpr::Apply { args, .. }) => {
                assert_eq!(args.iter().count(), 0);
            }
            _ => panic!("expected Tactic::Apply"),
        }
    }

    #[test]
    fn delegate_attribute_with_non_ident_arg_is_skipped() {
        // `@delegate(42)` — invalid arg shape (literal, not path).
        // No synthesis happens; the attribute is ignored.  The user
        // is responsible for noticing the unbound theorem-without-
        // proof after a downstream pass.
        let bogus_attr = Attribute::new(
            Text::from(DELEGATE_ATTR),
            Maybe::Some({
                let mut a = List::new();
                a.push(Expr::new(
                    ExprKind::Literal(verum_ast::Literal::int(42, span())),
                    span(),
                ));
                a
            }),
            span(),
        );
        let mut attrs: List<Attribute> = List::new();
        attrs.push(bogus_attr);
        let item = theorem_with_attrs_no_proof("foo", attrs, List::new());
        let mut module = module_with_items(vec![item]);

        let outcomes = expand_delegates_in_module(&mut module);
        assert_eq!(outcomes.len(), 0);

        let item = module.items.iter().next().unwrap();
        let decl = match &item.kind {
            ItemKind::Theorem(d) => d,
            _ => panic!("expected Theorem"),
        };
        assert!(matches!(decl.proof, Maybe::None));
    }
}
