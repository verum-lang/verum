//! Missing checked function identities never fall back to a local spelling.
#![cfg(feature = "codegen")]
use verum_ast::{ExprKind, FunctionBody, ItemKind, StmtKind, expr::ResolvedCallTarget};
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;

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
            qualified_name: "provider.operation".into(),
            type_prefix_receiver: false,
            resolver_pinned: true,
        });
        let error = VbcCodegen::new().compile_module(&module)
            .expect_err("missing exact target must not execute the local function");
        assert_eq!(error.undefined_function_name(), Some("provider.operation"), "{error:?}");
    }
}
