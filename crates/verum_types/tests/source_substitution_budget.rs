//! Source inference preserves bounded-substitution errors as ordinary diagnostics.
use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn reference_chain(depth: usize) -> Text {
    let mut source =
        Text::from("type Sample is {ptr:&Int,total:Int}; fn forward(value:Sample)->Sample{");
    let mut previous = Text::from("value");
    for index in 0..depth {
        let current: Text = format!("alias{index}").into();
        source.push_str(&format!("let {current}=&{previous};"));
        previous = current;
    }
    source.push_str(&format!(
        "Sample{{ptr:{previous}.ptr,total:{previous}.total}}}}"
    ));
    source.push_str("fn after(value:&Int)->&Int{value}");
    source
}

fn check(source: &str) -> List<(Text, Span)> {
    let file = FileId::new(17);
    let module = FastParser::new()
        .parse_module_str(source, file)
        .expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("sample");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| (error.to_string().into(), error.span()))
        .collect()
}

#[test]
fn supported_source_reference_chains_remain_accepted() {
    std::thread::spawn(|| {
        for depth in [4, 8, 16] {
            let source = reference_chain(depth);
            let errors = check(&source);
            assert!(errors.is_empty(), "depth {depth}: {errors:?}");
        }
    })
    .join()
    .expect("ordinary worker stack");
}

#[test]
fn excessive_source_reference_chains_report_the_substitution_budget() {
    std::thread::spawn(|| {
        for depth in [32, 64, 96] {
            let source = reference_chain(depth);
            let errors = check(&source);
            assert!(
                errors
                    .iter()
                    .any(|(message, _)| message.contains("structural depth budget")),
                "depth {depth} lost the bounded-substitution refusal: {errors:?}"
            );
            let following_function = source.find("fn after").unwrap() as u32;
            for (message, span) in errors {
                assert_eq!(
                    span.file_id,
                    FileId::new(17),
                    "source provenance: {message}"
                );
                assert!(span.start < span.end, "source range: {message}");
                assert!(
                    span.end <= following_function,
                    "failed scope leaked into the next function: {message}"
                );
            }
        }
    })
    .join()
    .expect("bounded source rejection on an ordinary stack");
}
