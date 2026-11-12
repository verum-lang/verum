use super::TypeChecker;
use verum_ast::{ItemKind, cfg::CfgEvaluator};
use verum_fast_parser::Parser;

#[test]
fn lookup_borrows_the_exact_target_selected_declaration() {
    let module = Parser::new(
        "@cfg(target_os = \"linux\") type Payload is { count: Int };
         @cfg(target_os = \"windows\") type Payload is { active: Bool };",
    )
    .parse_module()
    .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    for (target, index) in [
        ("x86_64-unknown-linux-gnu", 0),
        ("x86_64-pc-windows-msvc", 1),
    ] {
        checker.cfg_evaluator = CfgEvaluator::for_target(target);
        let selected = checker
            .local_source_type_declaration(&module, "Payload")
            .unwrap();
        let ItemKind::Type(original) = &module.items[index].kind else {
            panic!("type declaration")
        };
        assert!(
            std::ptr::eq(selected, original),
            "lookup copied the declaration body"
        );
    }
    checker.cfg_evaluator = CfgEvaluator::for_target("aarch64-apple-darwin");
    assert!(
        checker
            .local_source_type_declaration(&module, "Payload")
            .is_none()
    );
    assert!(
        checker
            .local_source_type_declaration(&module, "Missing")
            .is_none()
    );
}
