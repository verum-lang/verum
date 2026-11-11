use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn check(source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("source must reach type checking");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("stack_fixture");
    checker.set_current_module_path("stack_fixture.main");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|error| error.to_string().into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string().into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| error.to_string().into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    errors
}

#[test]
fn nested_calls_are_checked_on_an_ordinary_worker_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for depth in [1, 8, 16, 32] {
                let source = format!(
                    "fn identity(value: Int) -> Int {{ value }} \
                     fn nested() -> Int {{ {}42{} }}",
                    "identity(".repeat(depth),
                    ")".repeat(depth),
                );
                let errors = check(&source);
                assert!(errors.is_empty(), "depth {depth}: {errors:?}");
                let invalid = source.replace("42", "true");
                assert!(
                    !check(&invalid).is_empty(),
                    "depth {depth}: wrong argument accepted"
                );
            }
            for depth in [64, 96] {
                let source = format!(
                    "fn identity(value: Int) -> Int {{ value }} \
                     fn nested() -> Int {{ {}42{} }}",
                    "identity(".repeat(depth),
                    ")".repeat(depth),
                );
                let errors = check(&source);
                assert!(
                    errors.iter().any(|error| error.contains("depth")),
                    "depth {depth}: missing resource-limit diagnostic: {errors:?}"
                );
                assert!(check("fn after() -> Int { 42 }").is_empty());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
