use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    let mut errors: Vec<String> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| error.to_string())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| error.to_string()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|error| format!("{error:?}")),
    );
    errors
}
#[test]
fn included_method_attributes_preserve_real_body_checking() {
    for attributes in ["@inline", "@cfg(all())", "@inline @cfg(all())"] {
        let source = format!("type Cell is {{}}; implement Cell {{ {attributes} fn answer(&self) -> Int {{ 42 }} }}");
        let result = errors(&source);
        assert!(result.is_empty(), "{attributes}: {result:?}");
        let result = errors(&source.replace("42", "missing_value"));
        assert!(result.iter().any(|error| error.contains("missing_value")), "included body was lost: {result:?}");
    }
}

#[test]
fn excluded_methods_do_not_remove_either_retained_neighbor() {
    let source = "type Cell is {}; implement Cell { @inline fn before(&self) -> Int { 3 } @cfg(any()) fn absent(&self) -> Int { excluded_value } @inline fn after(&self) -> Int { 5 } }";
    assert!(errors(source).is_empty(), "{:?}", errors(source));
    for (value, missing) in [("3", "missing_before"), ("5", "missing_after")] {
        let result = errors(&source.replace(value, missing));
        assert!(result.iter().any(|error| error.contains(missing)), "retained body was lost: {result:?}");
        assert!(!result.iter().any(|error| error.contains("excluded_value")), "excluded body was checked: {result:?}");
    }
}
