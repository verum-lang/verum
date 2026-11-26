//! Missing checked function identities never fall back to a local spelling.
#![cfg(feature = "codegen")]
use verum_ast::{ExprKind, FunctionBody, ItemKind, StmtKind, expr::ResolvedCallTarget};
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;

#[test]
fn a_stale_selection_in_a_method_family_cannot_execute_its_first_body() {
    let source = "module measurements;
        type Meter is { sample: Int };
        type Reading is protocol { fn read(&self)->Int; };
        implement Reading for Meter { fn read(&self)->Int { 19 } }
        implement Meter { fn read(&self)->Int { self.sample } }
        fn probe()->Int { let meter = Meter { sample: 73 }; meter.read() }";
    let mut module = Parser::new(source).parse_module().unwrap();
    let Some(ItemKind::Function(probe)) = module.items.last_mut().map(|item| &mut item.kind) else { panic!("probe") };
    let Some(FunctionBody::Block(body)) = &mut probe.body else { panic!("body") };
    let call = body.expr.as_mut().unwrap();
    assert!(matches!(&call.kind, ExprKind::MethodCall { .. }));
    call.set_resolved_call_target(ResolvedCallTarget::StaticCall {
        qualified_name: "measurements.Meter.read".into(),
        source_method: Some(("measurements".into(), call.span)),
        type_prefix_receiver: false,
        resolver_pinned: false,
    });
    let mut generator = VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("measurements"));
    let error = generator.compile_module(&module).expect_err("call site cannot identify a method declaration");
    assert!(error.to_string().contains("no unique implementation"), "{error}");
}

#[test]
fn a_missing_selected_function_is_refused_for_calls_and_values() {
    for body in ["operation()", "let callback:fn()->Int=operation; callback()"] {
        let source = format!("fn operation()->Int {{73}} fn main()->Int {{{body}}}");
        let mut module = Parser::new(&source).parse_module().unwrap();
        let ItemKind::Function(function) = &mut module.items[1].kind else { panic!("main") };
        let Some(FunctionBody::Block(block)) = &mut function.body else { panic!("body") };
        let selected = if let Some(statement) = block.stmts.first_mut() {
            let StmtKind::Let { value: Some(value), .. } = &mut statement.kind else { panic!("binding") };
            value
        } else {
            let ExprKind::Call { func, .. } = &mut block.expr.as_mut().unwrap().kind else { panic!("call") };
            func.as_mut()
        };
        assert!(matches!(&selected.kind, ExprKind::Path(path) if path.as_ident().is_some_and(|name| name.as_str() == "operation")));
        selected.resolved_call_target = Some(ResolvedCallTarget::StaticCall {
            source_method: None,
            qualified_name: "provider.operation".into(),
            type_prefix_receiver: false,
            resolver_pinned: true,
        });
        let error = VbcCodegen::new().compile_module(&module)
            .expect_err("missing exact target must not execute the local function");
        assert_eq!(error.undefined_function_name(), Some("provider.operation"), "{error:?}");
    }
}
