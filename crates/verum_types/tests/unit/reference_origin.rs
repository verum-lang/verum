use super::*;
use verum_ast::{FileId, Ident, Path, Span};
use verum_common::Text;

fn receiver(name: &str) -> Expr {
    let span = Span::new(1, 2, FileId::new(1));
    Expr::new(ExprKind::Path(Path::single(Ident::new(name, span))), span)
}

#[test]
fn projected_payload_origins_follow_deep_reference_chains_on_an_ordinary_stack() {
    std::thread::spawn(|| {
        for depth in [64, 128, 256] {
            let mut checker = TypeChecker::with_minimal_context();
            checker.ctx.env.push_scope();
            let mut ty = Type::Generic { name: "Sample".into(), args: List::new() };
            checker.ctx.env.insert_mono("owner", ty.clone());
            let mut name = Text::from("owner");
            for index in 0..depth {
                let origin = checker.ctx.env.record_source(&name, false).unwrap();
                name = format!("alias{index}").into();
                ty = Type::reference(false, ty);
                checker.ctx.env.insert_mono(name.as_str(), ty.clone());
                checker.ctx.env.set_reference_origin(&name, ReferenceOrigin::Exact(origin));
            }
            let value = receiver(&name);
            checker.type_registry.register_expr(value.span, ty);
            assert_eq!(checker.projected_value_origins(&value), Some(List::new()));
        }
    }).join().expect("ordinary worker stack");
}

#[test]
fn cyclic_reference_aliases_cannot_prove_empty_payload_origins() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.ctx.env.push_scope();
    let alias = Type::Generic { name: "Cycle".into(), args: List::new() };
    checker.ctx.type_aliases.insert("Cycle".into(), Type::reference(false, alias.clone()));
    checker.ctx.env.insert_mono("value", alias.clone());
    checker.ctx.env.set_reference_origin("value", ReferenceOrigin::Parameter);
    let value = receiver("value");
    checker.type_registry.register_expr(value.span, alias);
    assert_eq!(checker.projected_value_origins(&value), None);
}
