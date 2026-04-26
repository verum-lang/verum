//! Integration tests for `HygieneRecheckPass` (#190).
//!
//! End-to-end pipeline tests: build a `Module` with synthetic
//! axiom/theorem declarations carrying `@framework(...)` attributes,
//! run the pass, and inspect the accumulated diagnostics for R1+R2+R3
//! firings.

use verum_ast::Ident;
use verum_ast::Span;
use verum_ast::Visibility;
use verum_ast::attr::Attribute;
use verum_ast::decl::AxiomDecl;
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::Path;
use verum_ast::{FileId, Item, Module, decl::ItemKind};
use verum_common::{Heap, List, Maybe, Text};
use verum_verification::{
    HygieneRecheckPass, HygieneSeverity, VerificationContext, VerificationPass,
};

fn span() -> Span {
    Span::default()
}

fn ident(name: &str) -> Ident {
    Ident {
        name: Text::from(name),
        span: span(),
    }
}

fn path_expr(name: &str) -> Expr {
    Expr::ident(ident(name))
}

/// Synthesize an `@framework(<corpus>)` attribute. The hygiene
/// walker reads only the corpus name (first positional arg); the
/// citation string isn't needed for R1/R3.
fn framework_attr(corpus: &str) -> Attribute {
    let mut args: List<Expr> = List::new();
    args.push(path_expr(corpus));
    Attribute {
        name: Text::from("framework"),
        args: Maybe::Some(args),
        span: span(),
    }
}

fn axiom_with_attrs(name: &str, attrs: Vec<Attribute>) -> AxiomDecl {
    let mut a_list: List<Attribute> = List::new();
    for a in attrs {
        a_list.push(a);
    }
    AxiomDecl {
        visibility: Visibility::Public,
        name: ident(name),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        proposition: Heap::new(Expr::new(
            ExprKind::Path(Path::single(ident("true"))),
            span(),
        )),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        attributes: a_list,
        span: span(),
    }
}

fn module_with(items: Vec<ItemKind>) -> Module {
    let mut item_list: List<Item> = List::new();
    for k in items {
        item_list.push(Item::new(k, span()));
    }
    Module {
        items: item_list,
        attributes: List::new(),
        file_id: FileId::dummy(),
        span: span(),
    }
}

#[test]
fn clean_module_with_framework_passes_r1() {
    // One @framework axiom with a foundation-neutral name. R1
    // accepts; R3 sees only one corpus with one axiom (< 5)
    // ⇒ no meta-classifier candidate.
    let axiom = axiom_with_attrs("classify", vec![framework_attr("diakrisis")]);
    let module = module_with(vec![ItemKind::Axiom(axiom)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.diagnostics().len(), 0);
}

#[test]
fn brand_prefix_axiom_name_warns_r1() {
    let axiom = axiom_with_attrs(
        "diakrisis_classify",
        vec![framework_attr("diakrisis")],
    );
    let module = module_with(vec![ItemKind::Axiom(axiom)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    // R1 is Warning ⇒ the pass still succeeds (no Error).
    assert!(result.success);
    assert_eq!(pass.diagnostics().len(), 1);
    let d = &pass.diagnostics()[0];
    assert_eq!(d.rule, "R1");
    assert_eq!(d.severity, HygieneSeverity::Warning);
    assert!(d.message.as_str().contains("diakrisis_classify"));
}

#[test]
fn r3_fires_when_two_corpora_each_have_five_axioms() {
    // Build two distinct corpora ("diakrisis", "actic") each with
    // 5 axioms ⇒ both qualify as meta-classifier candidates ⇒
    // R3 errors.
    let mut items: Vec<ItemKind> = Vec::new();
    for i in 0..5 {
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("d_axiom_{}", i),
            vec![framework_attr("diakrisis")],
        )));
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("a_axiom_{}", i),
            vec![framework_attr("actic")],
        )));
    }
    let module = module_with(items);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    // R3 is Error ⇒ pass fails.
    assert!(!result.success);
    let r3_count = pass
        .diagnostics()
        .iter()
        .filter(|d| d.rule == "R3")
        .count();
    assert_eq!(r3_count, 1);
    assert_eq!(pass.error_count(), 1);
}

#[test]
fn r3_quiet_when_only_one_corpus_qualifies() {
    // Five axioms in one corpus + one axiom in another ⇒ only
    // first qualifies as meta-classifier candidate.
    let mut items: Vec<ItemKind> = Vec::new();
    for i in 0..5 {
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("d_{}", i),
            vec![framework_attr("diakrisis")],
        )));
    }
    items.push(ItemKind::Axiom(axiom_with_attrs(
        "a_one",
        vec![framework_attr("actic")],
    )));
    let module = module_with(items);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.error_count(), 0);
}

#[test]
fn no_framework_attribute_no_check() {
    // R1 only fires on items that *carry* a @framework annotation —
    // user code without any framework attribute is exempt.
    let axiom = axiom_with_attrs("diakrisis_step", vec![]);
    let module = module_with(vec![ItemKind::Axiom(axiom)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.diagnostics().len(), 0);
}

#[test]
fn empty_module_passes_without_diagnostics() {
    let module = module_with(vec![]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.diagnostics().len(), 0);
}
