#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Comprehensive integration tests for gradual verification system
//
// Tests cover:
// - Verification level transitions
// - Boundary detection and proof obligations
// - Cost tracking and reporting
// - Transition recommendations
// - Integration with type system and SMT

use std::time::Duration;
use verum_common::{List, Maybe, Text};
use verum_verification::*;

#[test]
fn test_verification_level_hierarchy() {
    // Test that verification levels form a proper hierarchy
    assert!(!VerificationLevel::Runtime.requires_smt());
    assert!(VerificationLevel::Static.requires_smt());
    assert!(VerificationLevel::Proof.requires_smt());

    assert!(VerificationLevel::Runtime.allows_runtime_fallback());
    assert!(VerificationLevel::Static.allows_runtime_fallback());
    assert!(!VerificationLevel::Proof.allows_runtime_fallback());
}

#[test]
fn test_verification_context_scopes() {
    let mut ctx = VerificationContext::new();

    // Root scope should be Runtime
    assert_eq!(ctx.current_level(), VerificationLevel::Runtime);

    // Push Static scope
    let static_id = ctx.push_scope(
        VerificationMode::static_mode(),
        Text::from("static_function"),
    );
    assert_eq!(ctx.current_level(), VerificationLevel::Static);

    // Push Proof scope
    let proof_id = ctx.push_scope(VerificationMode::proof(), Text::from("proof_function"));
    assert_eq!(ctx.current_level(), VerificationLevel::Proof);

    // Pop back to Static
    ctx.pop_scope().unwrap();
    assert_eq!(ctx.current_level(), VerificationLevel::Static);

    // Pop back to Runtime
    ctx.pop_scope().unwrap();
    assert_eq!(ctx.current_level(), VerificationLevel::Runtime);
}

#[test]
fn test_verification_boundary_detection() {
    let mut ctx = VerificationContext::new();

    // Create boundary from Runtime to Proof
    let boundary_id = ctx.register_boundary(
        VerificationLevel::Runtime,
        VerificationLevel::Proof,
        BoundaryKind::FunctionCall,
    );

    let boundaries = ctx.boundaries();
    assert_eq!(boundaries.len(), 1);

    let boundary = &boundaries[0];
    assert_eq!(boundary.from_level, VerificationLevel::Runtime);
    assert_eq!(boundary.to_level, VerificationLevel::Proof);
    assert!(boundary.requires_obligations());
}

#[test]
fn test_transition_analyzer() {
    let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);

    // Create stable, well-tested code metrics
    let mut metrics = CodeMetrics::default();
    metrics.test_coverage = 0.95;
    metrics.change_frequency_per_week = 0.1;
    metrics.criticality_score = 5;
    metrics.execution_frequency = 1000.0;

    // Should recommend transition from Runtime to Static
    let decision = analyzer.analyze_function(
        &Text::from("hot_function"),
        VerificationLevel::Runtime,
        &metrics,
    );

    assert!(decision.recommend);
    assert_eq!(decision.from, VerificationLevel::Runtime);
    assert_eq!(decision.to, VerificationLevel::Static);
    // Confidence should be positive (exact value depends on strategy thresholds)
    assert!(decision.confidence > 0.0);
}

#[test]
fn test_cost_model_prediction() {
    let model = CostModel::default();

    // Runtime should have near-zero verification cost
    let runtime_cost = model.predict_cost(VerificationLevel::Runtime, 0, 0);
    assert!(runtime_cost.as_millis() < 10);

    // Static should have moderate cost
    let static_cost = model.predict_cost(VerificationLevel::Static, 10, 100);
    assert!(static_cost.as_millis() > 100);
    assert!(static_cost.as_millis() < 1000);

    // Proof should have higher cost
    let proof_cost = model.predict_cost(VerificationLevel::Proof, 50, 500);
    assert!(proof_cost.as_millis() > 1000);
}

#[test]
fn test_cost_report_generation() {
    let mut costs = List::new();

    costs.push(VerificationCost::new(
        Text::from("func1"),
        VerificationLevel::Static,
        Duration::from_millis(100),
        10,
        true,
        false,
        50,
    ));

    costs.push(VerificationCost::new(
        Text::from("func2"),
        VerificationLevel::Proof,
        Duration::from_millis(2000),
        100,
        true,
        false,
        500,
    ));

    let report = CostReport::from_costs(costs, None);

    assert_eq!(report.functions_verified, 2);
    assert_eq!(report.successes, 2);
    assert_eq!(report.failures, 0);

    let formatted = report.format();
    assert!(formatted.contains("Verification Cost Report"));
}

#[test]
fn test_budget_enforcement() {
    let threshold = CostThreshold::static_default();

    // Cost within budget
    let ok_cost = VerificationCost::new(
        Text::from("func1"),
        VerificationLevel::Static,
        Duration::from_millis(1000),
        10,
        true,
        false,
        50,
    );
    assert!(!threshold.exceeds(&ok_cost));

    // Cost exceeding budget
    let over_cost = VerificationCost::new(
        Text::from("func2"),
        VerificationLevel::Static,
        Duration::from_millis(10000),
        1000,
        true,
        false,
        5000,
    );
    assert!(threshold.exceeds(&over_cost));
}

#[test]
fn test_verification_decision_criteria() {
    let criteria = DecisionCriteria::production();

    // Good decision
    let good_decision = VerificationDecision::new(
        VerificationLevel::Static,
        Text::from("Recommended for production"),
        Duration::from_millis(5000),
        15.0,
        0.9,
    );
    assert!(criteria.meets_criteria(&good_decision));

    // Too expensive
    let expensive_decision = VerificationDecision::new(
        VerificationLevel::Proof,
        Text::from("Too expensive"),
        Duration::from_millis(100000),
        15.0,
        0.9,
    );
    assert!(!criteria.meets_criteria(&expensive_decision));

    // Low confidence
    let low_confidence = VerificationDecision::new(
        VerificationLevel::Static,
        Text::from("Low confidence"),
        Duration::from_millis(5000),
        15.0,
        0.5,
    );
    assert!(!criteria.meets_criteria(&low_confidence));
}

#[test]
fn test_transition_strategy_conservative() {
    let strategy = TransitionStrategy::Conservative;

    let decision = TransitionDecision::transition(
        VerificationLevel::Runtime,
        VerificationLevel::Static,
        0.85, // Good confidence
        20.0, // Good benefit
        8.0,  // Low cost increase
        Text::from("test"),
        List::new(),
    );

    // Should not pass conservative threshold (requires 0.95 confidence)
    assert!(!decision.passes_threshold(&strategy));

    // Higher confidence should pass
    let high_conf_decision = TransitionDecision::transition(
        VerificationLevel::Runtime,
        VerificationLevel::Static,
        0.96,
        20.0,
        8.0,
        Text::from("test"),
        List::new(),
    );
    assert!(high_conf_decision.passes_threshold(&strategy));
}

#[test]
fn test_transition_strategy_aggressive() {
    let strategy = TransitionStrategy::Aggressive;

    let decision = TransitionDecision::transition(
        VerificationLevel::Runtime,
        VerificationLevel::Proof,
        0.65, // Moderate confidence
        25.0, // Good benefit
        40.0, // High cost increase
        Text::from("test"),
        List::new(),
    );

    // Should pass aggressive threshold
    assert!(decision.passes_threshold(&strategy));
}

#[test]
fn test_migration_path_creation() {
    let mut steps = List::new();
    steps.push(MigrationStep::automated(Text::from(
        "Enable static verification",
    )));
    steps.push(MigrationStep::manual(
        Text::from("Add loop invariants"),
        Duration::from_secs(3600),
    ));

    let path = MigrationPath::new(VerificationLevel::Runtime, VerificationLevel::Static, steps);

    assert_eq!(path.start, VerificationLevel::Runtime);
    assert_eq!(path.target, VerificationLevel::Static);
    assert!(!path.fully_automated);
    assert!(!path.is_complete());
    assert!(path.next_step().is_some());
}

#[test]
fn test_code_metrics_classification() {
    let mut metrics = CodeMetrics::default();

    // Unstable code
    metrics.change_frequency_per_week = 5.0;
    assert!(!metrics.is_stable());

    // Stable code
    metrics.change_frequency_per_week = 0.5;
    assert!(metrics.is_stable());

    // Very stable code
    metrics.change_frequency_per_week = 0.05;
    assert!(metrics.is_very_stable());

    // Critical code
    metrics.criticality_score = 9;
    assert!(metrics.is_critical());
}

#[test]
fn test_valid_transitions() {
    let ctx = VerificationContext::new();

    // Can go to more restrictive
    assert!(ctx.is_valid_transition(VerificationLevel::Runtime, VerificationLevel::Static));
    assert!(ctx.is_valid_transition(VerificationLevel::Static, VerificationLevel::Proof));

    // Cannot go to less restrictive without obligations
    assert!(!ctx.is_valid_transition(VerificationLevel::Static, VerificationLevel::Runtime));
    assert!(!ctx.is_valid_transition(VerificationLevel::Proof, VerificationLevel::Static));

    // Same level always valid
    assert!(ctx.is_valid_transition(VerificationLevel::Runtime, VerificationLevel::Runtime));
}

#[test]
fn test_integration_type_system() {
    use verum_types::Type;

    // Test primitive types don't require verification
    assert_eq!(
        TypeSystemIntegration::recommend_level(&Type::Int),
        VerificationLevel::Runtime
    );
    assert_eq!(
        TypeSystemIntegration::recommend_level(&Type::Bool),
        VerificationLevel::Runtime
    );
    assert_eq!(
        TypeSystemIntegration::recommend_level(&Type::Text),
        VerificationLevel::Runtime
    );

    // Create a refined type: Int{x: x > 0} (Positive integer)
    use verum_ast::expr::{BinOp, Expr, ExprKind};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path};

    // Create the refinement predicate: it > 0
    let it_ident = Ident::new("it", Span::dummy());
    let it_var = Expr::new(ExprKind::Path(Path::from_ident(it_ident)), Span::dummy());
    let zero_lit = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: 0,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    );
    let predicate = Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Box::new(it_var),
            right: Box::new(zero_lit),
        },
        Span::dummy(),
    );

    let refined_type = Type::Refined {
        base: Box::new(Type::Int),
        predicate: verum_types::RefinementPredicate::inline(predicate, Span::dummy()),
    };

    // Refined types should require static verification
    assert!(TypeSystemIntegration::requires_verification(&refined_type));
    assert_eq!(
        TypeSystemIntegration::recommend_level(&refined_type),
        VerificationLevel::Static
    );

    // Test nested refined types in functions
    let func_with_refined_return = Type::Function {
        params: vec![Type::Int].into(),
        return_type: Box::new(refined_type.clone()),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };
    assert!(TypeSystemIntegration::requires_verification(
        &func_with_refined_return
    ));

    // Test arrays containing refined types
    let array_of_refined = Type::Array {
        element: Box::new(refined_type),
        size: Some(10),
    };
    assert!(TypeSystemIntegration::requires_verification(
        &array_of_refined
    ));
}

#[test]
fn test_integration_smt() {
    use verum_smt::VerifyMode;

    assert_eq!(
        SmtIntegration::to_smt_mode(VerificationLevel::Runtime),
        VerifyMode::Runtime
    );
    assert_eq!(
        SmtIntegration::to_smt_mode(VerificationLevel::Static),
        VerifyMode::Auto
    );
    assert_eq!(
        SmtIntegration::to_smt_mode(VerificationLevel::Proof),
        VerifyMode::Proof
    );
}

#[test]
fn test_integration_codegen() {
    // Runtime always emits checks
    assert!(CodegenIntegration::emit_runtime_checks(
        VerificationLevel::Runtime,
        false
    ));
    assert!(CodegenIntegration::emit_runtime_checks(
        VerificationLevel::Runtime,
        true
    ));

    // Static omits checks if proven safe
    assert!(CodegenIntegration::emit_runtime_checks(
        VerificationLevel::Static,
        false
    ));
    assert!(!CodegenIntegration::emit_runtime_checks(
        VerificationLevel::Static,
        true
    ));

    // Proof omits checks if proven safe
    assert!(!CodegenIntegration::emit_runtime_checks(
        VerificationLevel::Proof,
        true
    ));
}

#[test]
fn test_verification_pipeline() {
    use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, ItemKind, Visibility};
    use verum_ast::expr::{BinOp, Block, Expr, ExprKind};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::pattern::{Pattern, PatternKind};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, Type as AstType, TypeKind};
    use verum_ast::{Item, Module};
    use verum_common::Maybe;
    use verum_common::List;

    // Create a test module with a simple function:
    // fn increment(x: Int) -> Int { x + 1 }
    let x_ident = Ident::new("x", Span::dummy());
    let x_param = FunctionParam {
        kind: FunctionParamKind::Regular {
            pattern: Pattern::new(
                PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: x_ident.clone(),
                    subpattern: Maybe::None,
                },
                Span::dummy(),
            ),
            ty: AstType::new(
                TypeKind::Path(Path::from_ident(x_ident.clone())),
                Span::dummy(),
            ),
            default_value: Maybe::None,
        },
        attributes: List::new(),
        span: Span::dummy(),
    };

    // Body: x + 1
    let x_ref = Expr::new(ExprKind::Path(Path::from_ident(x_ident)), Span::dummy());
    let one_lit = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: 1,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    );
    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(x_ref),
            right: Box::new(one_lit),
        },
        Span::dummy(),
    );

    let body_block = Block {
        stmts: List::new(),
        expr: Some(Box::new(add_expr)),
        span: Span::dummy(),
    };

    let func_name = Ident::new("increment", Span::dummy());
    let int_ident = Ident::new("Int", Span::dummy());
    let func = FunctionDecl {
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
        name: func_name.clone(),
        generics: List::new(),
        params: vec![x_param].into(),
        return_type: Maybe::Some(AstType::new(
            TypeKind::Path(Path::from_ident(int_ident)),
            Span::dummy(),
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(verum_ast::decl::FunctionBody::Block(body_block)),
        span: Span::dummy(),
    };

    let item = Item::new(ItemKind::Function(func), Span::dummy());

    let test_module = Module {
        items: vec![item].into(),
        attributes: Vec::new().into(),
        file_id: verum_ast::FileId::dummy(),
        span: Span::dummy(),
    };

    // Create and run the verification pipeline
    let mut pipeline = VerificationPipeline::static_analysis_pipeline();
    let mut ctx = VerificationContext::new();

    // Run all verification passes on the module
    let results = pipeline
        .run_all(&test_module, &mut ctx)
        .expect("Pipeline should succeed");

    // Verify we got results from all passes
    assert!(!results.is_empty(), "Pipeline should produce results");

    // Each pass should succeed
    for result in results.iter() {
        assert!(result.success, "All verification passes should succeed");
    }

    // Check that the pipeline detected the function
    let total_functions: usize = results.iter().map(|r| r.functions_verified).sum();
    assert!(
        total_functions > 0,
        "Pipeline should verify at least one function"
    );
}

#[test]
fn test_end_to_end_gradual_verification() {
    // Test complete gradual verification workflow
    let mut ctx = VerificationContext::new();

    // 1. Start with runtime verification
    let func_id = ctx.push_scope(VerificationMode::runtime(), Text::from("user_function"));
    assert_eq!(ctx.current_level(), VerificationLevel::Runtime);

    // 2. Analyze for transition
    let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);
    let mut metrics = CodeMetrics::default();
    metrics.test_coverage = 0.95;
    metrics.change_frequency_per_week = 0.1;

    let decision = analyzer.analyze_function(
        &Text::from("user_function"),
        VerificationLevel::Runtime,
        &metrics,
    );

    // 3. Should recommend Static
    assert!(decision.recommend);
    assert_eq!(decision.to, VerificationLevel::Static);

    // 4. Transition to Static
    ctx.pop_scope().unwrap();
    let static_id = ctx.push_scope(VerificationMode::static_mode(), Text::from("user_function"));
    assert_eq!(ctx.current_level(), VerificationLevel::Static);

    // 5. After more stability, analyze again
    metrics.criticality_score = 9;
    metrics.change_frequency_per_week = 0.05;

    let proof_decision = analyzer.analyze_function(
        &Text::from("user_function"),
        VerificationLevel::Static,
        &metrics,
    );

    // 6. Should recommend Proof for critical code
    assert!(proof_decision.recommend);
    assert_eq!(proof_decision.to, VerificationLevel::Proof);

    // 7. Transition to Proof
    ctx.pop_scope().unwrap();
    let proof_id = ctx.push_scope(VerificationMode::proof(), Text::from("user_function"));
    assert_eq!(ctx.current_level(), VerificationLevel::Proof);
}
