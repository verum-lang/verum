use verum_ast::Span;
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, source_files::SourceFileRegistry};

#[test]
fn rejected_fields_point_to_the_member_in_the_correct_source_file() {
    let files = SourceFileRegistry::new();
    for (body, code) in [
        ("fn probe<T>(value: T) -> Int { value.absent }", "E103"),
        ("fn probe(value: Bool) -> Int { value.absent }", "E103"),
        (
            "type Row is { count: Int }; fn probe(value: Row) -> Int { value.absent }",
            "E404",
        ),
    ] {
        let source = format!("// строка 🧮\n{body}");
        let owner = files.register_file(format!("{code}-{body}.vr"), &source);
        let module = FastParser::new().parse_module_str(&source, owner).unwrap();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        let mut errors = checker.prepare_module_declarations(&module);
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
        errors.extend(checker.take_deferred_errors());
        let start = source.rfind("absent").unwrap();
        let target = Span::new(start as u32, (start + "absent".len()) as u32, owner);
        let expected = files.span_to_line_col(target);
        assert!(
            errors.iter().any(|error| {
                let diagnostic =
                    error.to_diagnostic_with_span(Some(|span| files.span_to_line_col(span)));
                diagnostic.code() == Some(code) && diagnostic.primary_span() == Some(&expected)
            }),
            "{source}: {errors:?}"
        );
    }
}
