use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker, TypeError, computational_properties::PropertyRequirement};

fn check(source: &str) -> (TypeChecker, List<TypeError>) {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_builtins();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    (checker, errors)
}

pub fn errors(source: &str) -> List<Text> {
    let (checker, errors) = check(source);
    errors.iter().map(|error| format!("{error:?}").into())
        .chain(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| d.message().into()))
        .collect()
}

pub fn assert_open_callback_rejected(source: &str, rejected: &str, params: List<Type>) {
    let (checker, errors) = check(source);
    let expected = Type::function_with_requirement(params.clone(), Type::Int,
        PropertyRequirement::source_callable(false)).diagnostic_text();
    let open = Type::function(params, Type::Int).diagnostic_text();
    assert!(errors.iter().chain(checker.diagnostic_sources()).any(|error| {
        matches!(error, TypeError::Mismatch { expected: wanted, actual, span }
            if wanted == &expected && actual == &open
                && source.get(span.start as usize..span.end as usize) == Some(rejected))
    }), "expected {expected} to reject {open} at {rejected}: {errors:?}");
}
