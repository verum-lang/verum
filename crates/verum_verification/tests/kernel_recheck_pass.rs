//! Integration tests for `KernelRecheckPass` (#187 V0).
//!
//! End-to-end pipeline tests: build a `Module` with one or more
//! `FunctionDecl`s, run the default `VerificationPipeline`, and
//! inspect the kernel-recheck pass's outcome via the per-pass
//! `VerificationResult`.

use verum_ast::Ident;
use verum_ast::Span;
use verum_ast::Visibility;
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{RefinementPredicate, Type, TypeKind};
use verum_ast::{FileId, Item, Module, decl::ItemKind};
use verum_common::{Heap, List, Maybe, Text};
use verum_verification::{
    KernelRecheckPass, VerificationContext, VerificationPass, VerificationPipeline,
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

fn method_call_expr(receiver: Expr, method_name: &str) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(receiver),
            method: ident(method_name),
            args: List::new(),
            type_args: List::new(),
        },
        span(),
    )
}

fn refined_int(predicate_expr: Expr) -> Type {
    Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span())),
            predicate: Heap::new(RefinementPredicate {
                expr: predicate_expr,
                binding: Maybe::None,
                span: span(),
            }),
        },
        span(),
    )
}

fn regular_param(ty: Type) -> FunctionParam {
    FunctionParam {
        kind: FunctionParamKind::Regular {
            pattern: Pattern {
                kind: PatternKind::Wildcard,
                span: span(),
            },
            ty,
            default_value: Maybe::None,
        },
        attributes: List::new(),
        span: span(),
    }
}

fn make_function(name: &str, params: Vec<Type>, return_type: Maybe<Type>) -> FunctionDecl {
    let mut p_list: List<FunctionParam> = List::new();
    for ty in params {
        p_list.push(regular_param(ty));
    }
    FunctionDecl {
        visibility: Visibility::Private,
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
        params: p_list,
        return_type,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: span(),
    }
}

fn module_with(functions: Vec<FunctionDecl>) -> Module {
    let mut items: List<Item> = List::new();
    for f in functions {
        items.push(Item::new(ItemKind::Function(f), span()));
    }
    Module {
        items,
        attributes: List::new(),
        file_id: FileId::dummy(),
        span: span(),
    }
}

#[test]
fn static_analysis_pipeline_includes_kernel_recheck() {
    // The static-analysis pipeline now runs 5 passes:
    //   [0] LevelInferencePass
    //   [1] KernelRecheckPass
    //   [2] HygieneRecheckPass        (#190)
    //   [3] BoundaryDetectionPass
    //   [4] TransitionRecommendationPass
    let module = module_with(vec![make_function(
        "id",
        vec![Type::int(span())],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pipeline = VerificationPipeline::static_analysis_pipeline();
    let mut ctx = VerificationContext::new();
    let results = pipeline.run_all(&module, &mut ctx).expect("pipeline runs");
    assert_eq!(results.len(), 5, "default pipeline should have 5 passes");
}

#[test]
fn kernel_recheck_pass_succeeds_on_clean_module() {
    // A module with no refinements has zero K-rule sites — the
    // pass MUST succeed.
    let module = module_with(vec![make_function(
        "plain",
        vec![Type::int(span())],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(
        result.success,
        "clean module must produce a success result"
    );
    assert_eq!(pass.rejections().len(), 0);
}

#[test]
fn kernel_recheck_pass_succeeds_on_well_formed_refinement() {
    // `fn f(x: Int{p}) -> Int` — predicate p is rank 0; base is
    // rank 0; 0 < 0+1 ⇒ K-Refine-omega accepts.
    let pred = path_expr("p");
    let refined = refined_int(pred);
    let module = module_with(vec![make_function(
        "f",
        vec![refined],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.rejections().len(), 0);
    // The well-formed refinement still produces a verification-cost
    // entry (problem_size = 1 K-rule site).
    assert!(
        result.costs.iter().any(|c| c.problem_size == 1),
        "expected at least one K-rule site recorded"
    );
}

#[test]
fn kernel_recheck_pass_rejects_modal_overshoot() {
    // `fn g(x: Int{p.box().box()}) -> Int` — predicate has md^ω = 2,
    // base has md^ω = 0. 2 < 1 is false ⇒ K-Refine-omega rejects.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let refined = refined_int(boxed);
    let module = module_with(vec![make_function(
        "g",
        vec![refined],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(
        !result.success,
        "modal overshoot must produce a failure result"
    );
    assert_eq!(pass.rejections().len(), 1);
    let rejection = pass.rejections().get(0).unwrap();
    assert!(
        rejection.as_str().contains("g"),
        "rejection label should mention function 'g': {}",
        rejection.as_str()
    );
}

#[test]
fn kernel_recheck_pass_walks_multiple_functions() {
    // One clean function and one rejected function in the same
    // module. The pass should walk both and surface the one
    // rejection.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad_refined = refined_int(boxed);
    let good_refined = refined_int(path_expr("p"));
    let module = module_with(vec![
        make_function("good", vec![good_refined], Maybe::Some(Type::int(span()))),
        make_function("bad", vec![bad_refined], Maybe::Some(Type::int(span()))),
    ]);
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success);
    assert_eq!(pass.rejections().len(), 1);
    assert!(pass.rejections().get(0).unwrap().as_str().contains("bad"));
    // Both functions should produce cost entries.
    assert_eq!(result.costs.len(), 2);
}

#[test]
fn pipeline_fail_fast_halts_subsequent_passes_on_kernel_failure() {
    // The default pipeline order is
    // [LevelInference, KernelRecheck, BoundaryDetection, TransitionRecommendation].
    // When KernelRecheck rejects, BoundaryDetection and
    // TransitionRecommendation MUST NOT run — verification passes
    // form a strict ordering and downstream passes presume kernel
    // invariants hold.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let refined = refined_int(boxed);
    let module = module_with(vec![make_function(
        "halts",
        vec![refined],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pipeline = VerificationPipeline::static_analysis_pipeline();
    let mut ctx = VerificationContext::new();
    let results = pipeline.run_all(&module, &mut ctx).expect("pipeline runs");
    assert_eq!(
        results.len(),
        2,
        "fail-fast: only LevelInference + KernelRecheck should run; got {} results",
        results.len()
    );
    assert!(
        results.get(0).unwrap().success,
        "LevelInference should succeed"
    );
    assert!(
        !results.get(1).unwrap().success,
        "KernelRecheck should fail"
    );
}

#[test]
fn pipeline_runs_all_passes_when_module_clean() {
    // Sanity: a clean module exercises the full 5-pass pipeline
    // (no fail-fast trigger).
    let module = module_with(vec![make_function(
        "clean",
        vec![Type::int(span())],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pipeline = VerificationPipeline::static_analysis_pipeline();
    let mut ctx = VerificationContext::new();
    let results = pipeline.run_all(&module, &mut ctx).expect("pipeline runs");
    assert_eq!(results.len(), 5, "clean module runs all 5 passes");
    for r in results.iter() {
        assert!(r.success, "every pass should succeed on a clean module");
    }
}

// =============================================================================
// V5 (#192) — KernelRecheckPass descends into impl-block methods
// =============================================================================

fn impl_item_with_function(func: FunctionDecl) -> verum_ast::decl::ImplItem {
    verum_ast::decl::ImplItem {
        attributes: List::new(),
        visibility: Visibility::Private,
        kind: verum_ast::decl::ImplItemKind::Function(func),
        span: span(),
    }
}

fn impl_block(for_type: Type, items: Vec<verum_ast::decl::ImplItem>) -> verum_ast::decl::ImplDecl {
    let mut item_list: List<verum_ast::decl::ImplItem> = List::new();
    for it in items {
        item_list.push(it);
    }
    verum_ast::decl::ImplDecl {
        is_unsafe: false,
        generics: List::new(),
        kind: verum_ast::decl::ImplKind::Inherent(for_type),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        specialize_attr: Maybe::None,
        items: item_list,
        span: span(),
    }
}

#[test]
fn kernel_recheck_pass_descends_into_impl_block_methods() {
    // implement Foo {
    //     fn impl_method(x: Int{p.box().box()}) -> Int { ... }
    // }
    // Modal-overshoot in an impl-method param must be caught.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad_refined = refined_int(boxed);
    let bad_method = make_function(
        "impl_method",
        vec![bad_refined],
        Maybe::Some(Type::int(span())),
    );
    let impl_decl = impl_block(
        Type::int(span()),
        vec![impl_item_with_function(bad_method)],
    );
    let module = module_with(vec![]);
    // Manually splice in the Impl item (module_with helper is
    // built around make_function-produced Functions).
    let mut module = module;
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Impl(impl_decl),
        span(),
    ));

    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(
        !result.success,
        "modal overshoot in impl-method must reject"
    );
    assert_eq!(pass.rejections().len(), 1);
    let rejection = pass.rejections().get(0).unwrap();
    assert!(
        rejection.as_str().contains("impl_method"),
        "rejection should mention 'impl_method': {}",
        rejection.as_str()
    );
}

#[test]
fn kernel_recheck_pass_clean_impl_block_passes() {
    let pred = path_expr("p");
    let good_refined = refined_int(pred);
    let good_method = make_function(
        "impl_method",
        vec![good_refined],
        Maybe::Some(Type::int(span())),
    );
    let impl_decl = impl_block(
        Type::int(span()),
        vec![impl_item_with_function(good_method)],
    );
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Impl(impl_decl),
        span(),
    ));

    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.rejections().len(), 0);
}

#[test]
fn kernel_recheck_pass_walks_top_level_and_impl_methods_together() {
    // Mix: top-level fn (clean) + impl block with a bad method.
    let top = make_function(
        "top",
        vec![Type::int(span())],
        Maybe::Some(Type::int(span())),
    );
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad_refined = refined_int(boxed);
    let bad_method = make_function(
        "im_bad",
        vec![bad_refined],
        Maybe::Some(Type::int(span())),
    );
    let impl_decl = impl_block(
        Type::int(span()),
        vec![impl_item_with_function(bad_method)],
    );
    let mut module = module_with(vec![top]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Impl(impl_decl),
        span(),
    ));

    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success);
    // 2 cost entries: top-level fn + impl method.
    assert_eq!(result.costs.len(), 2);
    // Only the impl method was rejected.
    assert_eq!(pass.rejections().len(), 1);
    assert!(pass.rejections().get(0).unwrap().as_str().contains("im_bad"));
}

// =============================================================================
// V6 (#200) — descend into Module + Theorem/Lemma/Corollary/Axiom items
// =============================================================================

fn axiom_decl(name: &str, params: Vec<Type>, return_type: Maybe<Type>) -> verum_ast::decl::AxiomDecl {
    let mut p_list: List<verum_ast::decl::FunctionParam> = List::new();
    for ty in params {
        p_list.push(regular_param(ty));
    }
    verum_ast::decl::AxiomDecl {
        visibility: Visibility::Public,
        name: ident(name),
        generics: List::new(),
        params: p_list,
        return_type,
        proposition: Heap::new(verum_ast::expr::Expr::new(
            verum_ast::expr::ExprKind::Path(verum_ast::ty::Path::single(ident("true"))),
            span(),
        )),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        attributes: List::new(),
        span: span(),
    }
}

fn theorem_decl(name: &str, params: Vec<Type>, return_type: Maybe<Type>) -> verum_ast::decl::TheoremDecl {
    let mut p_list: List<verum_ast::decl::FunctionParam> = List::new();
    for ty in params {
        p_list.push(regular_param(ty));
    }
    verum_ast::decl::TheoremDecl {
        visibility: Visibility::Public,
        name: ident(name),
        generics: List::new(),
        params: p_list,
        return_type,
        requires: List::new(),
        ensures: List::new(),
        proposition: Heap::new(verum_ast::expr::Expr::new(
            verum_ast::expr::ExprKind::Path(verum_ast::ty::Path::single(ident("true"))),
            span(),
        )),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        proof: Maybe::None,
        attributes: List::new(),
        span: span(),
    }
}

#[test]
fn kernel_recheck_pass_walks_axiom_signatures() {
    // axiom positive_overshoot(x: Int{p.box().box()}) -> Int
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad = refined_int(boxed);
    let axiom = axiom_decl("positive_overshoot", vec![bad], Maybe::Some(Type::int(span())));
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Axiom(axiom),
        span(),
    ));
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "axiom modal overshoot must reject under V6");
    assert_eq!(pass.rejections().len(), 1);
    assert!(pass.rejections().get(0).unwrap().as_str().contains("positive_overshoot"));
}

#[test]
fn kernel_recheck_pass_walks_theorem_signatures() {
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad = refined_int(boxed);
    let thm = theorem_decl("plus_comm_modal", vec![bad], Maybe::Some(Type::int(span())));
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Theorem(thm),
        span(),
    ));
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "theorem modal overshoot must reject under V6");
    assert!(pass.rejections().get(0).unwrap().as_str().contains("plus_comm_modal"));
}

#[test]
fn kernel_recheck_pass_walks_lemma_via_theorem_decl() {
    // Lemma uses TheoremDecl too (per ItemKind::Lemma(TheoremDecl)).
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad = refined_int(boxed);
    let lemma = theorem_decl("helper_modal", vec![bad], Maybe::Some(Type::int(span())));
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Lemma(lemma),
        span(),
    ));
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success);
    assert!(pass.rejections().get(0).unwrap().as_str().contains("helper_modal"));
}

#[test]
fn kernel_recheck_pass_clean_axiom_passes() {
    // Well-formed axiom with atomic refinement should pass.
    let pred = path_expr("p");
    let good = refined_int(pred);
    let axiom = axiom_decl("safe_axiom", vec![good], Maybe::Some(Type::int(span())));
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Axiom(axiom),
        span(),
    ));
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(result.success);
    assert_eq!(pass.rejections().len(), 0);
    // The axiom did produce a cost entry (V6 records every walked decl).
    assert!(result.costs.iter().any(|c| c.problem_size == 1));
}

#[test]
fn kernel_recheck_pass_descends_into_nested_module_items() {
    // Module containing one bad function — V6 must descend
    // through ItemKind::Module and recheck the nested axiom/fn.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let bad = refined_int(boxed);
    let bad_fn = make_function("nested_bad", vec![bad], Maybe::Some(Type::int(span())));
    let mut nested_items: List<verum_ast::Item> = List::new();
    nested_items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Function(bad_fn),
        span(),
    ));
    let module_decl = verum_ast::decl::ModuleDecl {
        visibility: Visibility::Public,
        name: ident("inner"),
        items: Maybe::Some(nested_items),
        profile: Maybe::None,
        features: Maybe::None,
        contexts: List::new(),
        span: span(),
    };
    let mut module = module_with(vec![]);
    module.items.push(verum_ast::Item::new(
        verum_ast::decl::ItemKind::Module(module_decl),
        span(),
    ));
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "nested-module bad fn must be caught by V6");
    assert!(pass.rejections().get(0).unwrap().as_str().contains("nested_bad"));
}

#[test]
fn static_analysis_pipeline_kernel_recheck_result_visible() {
    // The KernelRecheckPass result is the second entry in the
    // run_all output. On modal overshoot the second result is
    // a failure with non-empty costs.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let refined = refined_int(boxed);
    let module = module_with(vec![make_function(
        "h",
        vec![refined],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pipeline = VerificationPipeline::static_analysis_pipeline();
    let mut ctx = VerificationContext::new();
    let results = pipeline.run_all(&module, &mut ctx).expect("pipeline runs");
    // [0]=LevelInference, [1]=KernelRecheck, [2]=BoundaryDetection, [3]=TransitionRecommendation
    let kernel_result = results.get(1).expect("KernelRecheck pass result");
    assert!(
        !kernel_result.success,
        "modal overshoot must surface as a failed pipeline pass"
    );
    assert_eq!(kernel_result.costs.len(), 1, "one function in module");
}

// =============================================================================
// V8 (#211, B12) — VFE @require_extension governance gating
// =============================================================================

use verum_verification::vfe_gate::{EnabledExtensions, VfePolicy};

fn module_with_attrs(
    functions: Vec<FunctionDecl>,
    attrs: Vec<verum_ast::attr::Attribute>,
) -> Module {
    let mut items: List<Item> = List::new();
    for f in functions {
        items.push(Item::new(ItemKind::Function(f), span()));
    }
    let mut attr_list: List<verum_ast::attr::Attribute> = List::new();
    for a in attrs {
        attr_list.push(a);
    }
    Module {
        items,
        attributes: attr_list,
        file_id: FileId::dummy(),
        span: span(),
    }
}

fn require_extension_attr(ext: &str) -> verum_ast::attr::Attribute {
    let mut segs: List<verum_ast::ty::PathSegment> = List::new();
    segs.push(verum_ast::ty::PathSegment::Name(ident(ext)));
    let mut args: List<Expr> = List::new();
    args.push(Expr::new(
        ExprKind::Path(verum_ast::ty::Path::new(segs, span())),
        span(),
    ));
    verum_ast::attr::Attribute {
        name: Text::from("require_extension"),
        args: Maybe::Some(args),
        span: span(),
    }
}

#[test]
fn b12_default_policy_runs_all_rules_active() {
    // V8 default policy = AllRulesActive — pre-V8 behaviour
    // preserved. A modal-overshoot module without any
    // @require_extension annotation is still rejected.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let module = module_with(vec![make_function(
        "g",
        vec![refined_int(boxed)],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pass = KernelRecheckPass::new();
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(!result.success, "default policy still runs the recheck");
}

#[test]
fn b12_opt_in_policy_skips_module_without_require() {
    // OptInOnly + no @require_extension(vfe_7) → walker skipped,
    // result is success even for a modal-overshoot module. This
    // proves the gate actually short-circuits the work.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let module = module_with(vec![make_function(
        "g",
        vec![refined_int(boxed)],
        Maybe::Some(Type::int(span())),
    )]);
    let mut pass = KernelRecheckPass::new().with_policy(VfePolicy::OptInOnly);
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(
        result.success,
        "OptInOnly + no @require_extension → recheck skipped"
    );
    assert_eq!(pass.rejections().len(), 0);
}

#[test]
fn b12_opt_in_policy_runs_when_module_requires_vfe_7() {
    // OptInOnly + @require_extension(vfe_7) on the module → walker
    // runs and rejects the modal-overshoot.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let module = module_with_attrs(
        vec![make_function(
            "g",
            vec![refined_int(boxed)],
            Maybe::Some(Type::int(span())),
        )],
        vec![require_extension_attr("vfe_7")],
    );
    let mut pass = KernelRecheckPass::new().with_policy(VfePolicy::OptInOnly);
    let mut ctx = VerificationContext::new();
    let result = pass.run(&module, &mut ctx).expect("pass runs");
    assert!(
        !result.success,
        "OptInOnly + @require_extension(vfe_7) → recheck runs and rejects"
    );
}

#[test]
fn b12_extension_set_reads_module_level_require() {
    // White-box test of EnabledExtensions::from_module — it must
    // pick up the module-level @require_extension annotation.
    let module = module_with_attrs(
        vec![],
        vec![require_extension_attr("vfe_7")],
    );
    let set = EnabledExtensions::from_module(&module);
    assert!(set.requires("vfe_7"));
    assert!(!set.requires("vfe_1"));
    assert!(!set.disables("vfe_7"));
}

#[test]
fn b12_policy_accessor_returns_configured_policy() {
    // Sanity: the with_policy builder + policy() accessor agree.
    let pass = KernelRecheckPass::new().with_policy(VfePolicy::OptInOnly);
    assert_eq!(pass.policy(), VfePolicy::OptInOnly);
    let pass2 = KernelRecheckPass::new();
    assert_eq!(pass2.policy(), VfePolicy::AllRulesActive);
}
