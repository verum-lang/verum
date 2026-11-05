use verum_ast::cfg::TargetConfig;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

#[test]
fn statement_selection_uses_the_installed_compilation_target() {
    let module = Parser::new(
        "fn probe() -> Int { @cfg(target_os = \"windows\") let incompatible: Bool = 42; 42 }",
    )
    .parse_module()
    .unwrap();
    for (target, allowed) in [
        ("aarch64-apple-darwin", true),
        ("x86_64-pc-windows-msvc", false),
    ] {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        checker.set_cfg_target(TargetConfig::for_target(target));
        let mut errors = checker.prepare_module_declarations(&module);
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
        errors.extend(checker.take_deferred_errors());
        let errors: verum_common::List<_> = errors.iter().chain(checker.diagnostic_sources().iter()).map(|error| format!("{error:?}")).collect();
        if allowed {
            assert!(errors.is_empty(), "{target}: {errors:?}");
        } else {
            assert!(
                errors
                    .iter()
                    .any(|error| format!("{error:?}").contains("Bool")),
                "target condition ignored: {errors:?}"
            );
        }
    }
}
