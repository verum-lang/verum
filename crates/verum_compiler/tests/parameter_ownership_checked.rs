//! Checked method selection retains argument and projected-result ownership.
use verum_ast::{Visitor, expr::ResolvedCallTarget};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module,
};

fn checked_run(source: &str) -> i64 {
    let mut ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let mut errors = checker.prepare_module_declarations(&ast);
    errors.extend(
        ast.items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    assert!(errors.is_empty(), "{errors:?}");
    assert!(
        checker.diagnostic_sources().is_empty(),
        "{:?}",
        checker.diagnostic_sources()
    );
    checker.commit_resolved_call_targets(&mut ast);
    struct Selected(usize);
    impl Visitor for Selected {
        fn visit_expr(&mut self, expr: &verum_ast::Expr) {
            if matches!(
                expr.resolved_call_target,
                Some(ResolvedCallTarget::StaticCall { .. })
            ) {
                self.0 += 1;
            }
            verum_ast::visitor::walk_expr(self, expr);
        }
    }
    let mut selected = Selected(0);
    for item in &ast.items {
        selected.visit_item(item);
    }
    assert!(
        selected.0 > 0,
        "control must use the checker's selected call route"
    );
    let original = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    let main = module.find_function_by_name("main").unwrap();
    Interpreter::try_new_validated(module.into())
        .unwrap()
        .execute_function(main)
        .unwrap()
        .try_as_i64()
        .unwrap()
}

#[test]
fn checked_consuming_method_preserves_the_callers_nested_owner() {
    assert_eq!(
        checked_run(
            r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        implement Branch { fn consume(self) {} }
        fn main() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            original.consume();
            original.leaf.value
        }
    "#
        ),
        73
    );
}

#[test]
fn checked_parameter_projection_survives_its_parameter_cleanup() {
    assert_eq!(
        checked_run(
            r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        implement Branch { fn project(self) -> Leaf { self.leaf } }
        fn main() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            let projected = original.project();
            original.leaf.value + projected.value
        }
    "#
        ),
        146
    );
}
