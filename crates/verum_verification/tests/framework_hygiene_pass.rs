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

// =============================================================================
// V2 (#193) — descend into impl-block methods
// =============================================================================

use verum_ast::Type;
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParam, ImplDecl, ImplItem, ImplItemKind, ImplKind,
};

fn make_function_with_attrs(name: &str, attrs: Vec<Attribute>) -> FunctionDecl {
    let mut a_list: List<Attribute> = List::new();
    for a in attrs {
        a_list.push(a);
    }
    FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident(name),
        generics: List::new(),
        params: List::<FunctionParam>::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: a_list,
        body: Maybe::<FunctionBody>::None,
        span: span(),
    }
}

fn impl_block_with(items: Vec<ImplItem>) -> ImplDecl {
    let mut item_list: List<ImplItem> = List::new();
    for it in items {
        item_list.push(it);
    }
    ImplDecl {
        is_unsafe: false,
        generics: List::new(),
        kind: ImplKind::Inherent(Type::int(span())),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        specialize_attr: Maybe::None,
        items: item_list,
        span: span(),
    }
}

fn impl_function_item(func: FunctionDecl) -> ImplItem {
    ImplItem {
        attributes: List::new(),
        visibility: Visibility::Public,
        kind: ImplItemKind::Function(func),
        span: span(),
    }
}

#[test]
fn r1_fires_on_impl_method_with_brand_prefix_name() {
    // implement Foo {
    //     @framework(diakrisis, "...") fn diakrisis_step() {}
    // }
    let bad_method = make_function_with_attrs(
        "diakrisis_step",
        vec![framework_attr("diakrisis")],
    );
    let impl_decl = impl_block_with(vec![impl_function_item(bad_method)]);
    let module = module_with(vec![ItemKind::Impl(impl_decl)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    // R1 is Warning ⇒ pass still succeeds.
    assert!(result.success);
    let r1_count = pass
        .diagnostics()
        .iter()
        .filter(|d| d.rule == "R1")
        .count();
    assert_eq!(r1_count, 1);
    assert!(
        pass.diagnostics()[0]
            .message
            .as_str()
            .contains("diakrisis_step")
    );
}

#[test]
fn r3_fires_on_impl_methods_constituting_two_corpora() {
    // Five impl-block functions per corpus, two corpora ⇒ R3.
    let mut impl_items: Vec<ImplItem> = Vec::new();
    for i in 0..5 {
        impl_items.push(impl_function_item(make_function_with_attrs(
            &format!("d_{}", i),
            vec![framework_attr("diakrisis")],
        )));
        impl_items.push(impl_function_item(make_function_with_attrs(
            &format!("a_{}", i),
            vec![framework_attr("actic")],
        )));
    }
    let impl_decl = impl_block_with(impl_items);
    let module = module_with(vec![ItemKind::Impl(impl_decl)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "R3 must error on two impl-resident corpora");
    let r3_count = pass
        .diagnostics()
        .iter()
        .filter(|d| d.rule == "R3")
        .count();
    assert_eq!(r3_count, 1);
}

#[test]
fn r4_uip_plus_univalence_rejected_via_hygiene_pass() {
    // V0 framework-conflicts wiring (#197): an axiom in the
    // `uip` corpus and another in `univalence` — the audit
    // surfaces R4 with literature citation. R4 is Error severity;
    // the pass returns success == false (fail-fast in the
    // pipeline).
    let uip_axiom = axiom_with_attrs("uip_axiom", vec![framework_attr("uip")]);
    let ua_axiom = axiom_with_attrs(
        "univalence_axiom",
        vec![framework_attr("univalence")],
    );
    let module = module_with(vec![
        ItemKind::Axiom(uip_axiom),
        ItemKind::Axiom(ua_axiom),
    ]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "uip ⊥ univalence must fail-fast");
    let r4_count = pass
        .diagnostics()
        .iter()
        .filter(|d| d.rule == "R4")
        .count();
    assert_eq!(r4_count, 1);
    let r4 = pass.diagnostics().iter().find(|d| d.rule == "R4").unwrap();
    assert_eq!(r4.severity, HygieneSeverity::Error);
    assert!(r4.message.as_str().contains("uip"));
    assert!(r4.message.as_str().contains("univalence"));
    assert!(r4.message.as_str().contains("HoTT Book"));
}

#[test]
fn r4_compatible_corpora_pass_clean() {
    // Compatible pair (lurie_htt + schreiber_dcct) — no R4
    // diagnostic should fire.
    let htt = axiom_with_attrs("y", vec![framework_attr("lurie_htt")]);
    let dcct = axiom_with_attrs("z", vec![framework_attr("schreiber_dcct")]);
    let module = module_with(vec![ItemKind::Axiom(htt), ItemKind::Axiom(dcct)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    let r4_count = pass
        .diagnostics()
        .iter()
        .filter(|d| d.rule == "R4")
        .count();
    assert_eq!(r4_count, 0);
}

#[test]
fn r3_threshold_lowered_to_two_fires_with_two_axioms_each() {
    // V3 (#203): with threshold=2, two corpora each shipping
    // exactly 2 axioms qualify as meta-classifier candidates ⇒ R3.
    // With default threshold=5, both would NOT qualify and pass.
    let mut items: Vec<ItemKind> = Vec::new();
    for i in 0..2 {
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("d_{}", i),
            vec![framework_attr("diakrisis")],
        )));
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("a_{}", i),
            vec![framework_attr("actic")],
        )));
    }
    let module = module_with(items);

    // First: default threshold (5) — neither corpus qualifies.
    let mut default_pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let res_default = default_pass.run(&module, &mut ctx).expect("pass runs");
    assert!(res_default.success, "with default threshold (5), neither corpus qualifies");

    // Second: lowered threshold (2) — BOTH corpora qualify ⇒ R3 fires.
    let mut strict_pass = HygieneRecheckPass::with_meta_classifier_threshold(2);
    let mut ctx2 = VerificationContext::new();
    let res_strict = strict_pass.run(&module, &mut ctx2).expect("pass runs");
    assert!(!res_strict.success, "with lowered threshold (2), R3 must fire");
    assert_eq!(strict_pass.error_count(), 1);
}

#[test]
fn r3_threshold_raised_above_qualifying_count_silences_alert() {
    // V3 (#203): owl2_fs ships ~64 axioms; raising threshold
    // above any single corpus's count silences R3 in modules
    // that intentionally have multiple large frameworks.
    let mut items: Vec<ItemKind> = Vec::new();
    for i in 0..6 {
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("d_{}", i),
            vec![framework_attr("diakrisis")],
        )));
        items.push(ItemKind::Axiom(axiom_with_attrs(
            &format!("a_{}", i),
            vec![framework_attr("actic")],
        )));
    }
    let module = module_with(items);

    // Default threshold (5) ⇒ both qualify (6 each) ⇒ R3 fires.
    let mut default_pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let res_default = default_pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!res_default.success);

    // Threshold raised to 100 ⇒ neither qualifies ⇒ pass.
    let mut high_pass = HygieneRecheckPass::with_meta_classifier_threshold(100);
    let mut ctx2 = VerificationContext::new();
    let res_high = high_pass.run(&module, &mut ctx2).expect("pass runs");
    assert!(res_high.success, "raising threshold above corpus size silences R3");
}

#[test]
fn impl_methods_without_framework_unaffected() {
    // No @framework on impl methods ⇒ no hygiene check fires.
    let m1 = make_function_with_attrs("plain_method", vec![]);
    let impl_decl = impl_block_with(vec![impl_function_item(m1)]);
    let module = module_with(vec![ItemKind::Impl(impl_decl)]);
    let mut pass = HygieneRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.diagnostics().len(), 0);
}
