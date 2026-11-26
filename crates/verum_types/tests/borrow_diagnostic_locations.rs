use verum_ast::Span;
use verum_types::{TypeError, source_files::SourceFileRegistry};

#[test]
fn an_escaping_protocol_result_points_to_the_call_inside_a_nested_record() {
    let source = "
        type Pair<T> is { first:T, second:T };
        type Envelope<T> is { pair:Pair<T>, serial:Int };
        type Sequence is protocol {
            fn duplicate(&self) -> Self;
            fn pair(self) -> Envelope<Self> {
                Envelope {pair:Pair {first:self.duplicate(), second:self}, serial:10}
            }
        };
    ";
    let module = verum_fast_parser::FastParser::new()
        .parse_module_str(source, verum_ast::FileId::new(1)).unwrap();
    let mut checker = verum_types::TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("demo");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let error = errors.iter().chain(checker.diagnostic_sources())
        .find(|error| error.to_string().contains("does not live long enough"))
        .unwrap_or_else(|| panic!("{errors:?}"));
    let span = error.span();
    assert_eq!(&source[span.start as usize..span.end as usize], "self.duplicate()");
}

#[test]
fn ownership_errors_keep_the_use_and_the_related_location() {
    use TypeError::*;
    let files = SourceFileRegistry::new();
    let input = files.register_file("consumer.vr", "fn run() {\n    db.close();\n}");
    let source = files.register_file("origin.vr", "let b = &db;\n");
    let action = Span::new(18, 23, input);
    let origin = Span::new(8, 11, source);
    let cases = [
        (
            BorrowConflict {
                var: "db".into(),
                existing_borrow_span: origin,
                existing_is_mut: false,
                new_borrow_span: action,
                new_is_mut: true,
            },
            "E310",
            "previous immutable borrow occurs here",
        ),
        (
            FieldBorrowConflict {
                var: "db".into(),
                field: "state".into(),
                existing_span: origin,
                new_span: action,
            },
            "E311",
            "previous borrow of field occurs here",
        ),
        (
            DanglingReference {
                var: "db".into(),
                ref_span: action,
                drop_span: Some(origin),
            },
            "E312",
            "value goes out of scope here",
        ),
        (
            MoveWhileBorrowed {
                var: "db".into(),
                move_span: action,
                borrow_span: origin,
            },
            "E313",
            "borrow occurs here",
        ),
        (
            AssignWhileBorrowed {
                var: "db".into(),
                assign_span: action,
                borrow_span: origin,
            },
            "E314",
            "borrow occurs here",
        ),
    ];
    for (error, code, message) in cases {
        let diagnostic = error.to_diagnostic_with_span(Some(|s| files.span_to_line_col(s)));
        assert_eq!(diagnostic.code(), Some(code));
        assert_eq!(
            diagnostic.primary_span(),
            Some(&files.span_to_line_col(action))
        );
        let [location] = diagnostic.secondary_labels() else {
            panic!("{diagnostic:?}")
        };
        assert_eq!(location.span, files.span_to_line_col(origin));
        assert_eq!(location.message.as_str(), message);
    }
}

#[test]
fn an_unknown_borrow_origin_does_not_create_a_navigation_target() {
    let files = SourceFileRegistry::new();
    let file = files.register_file("consumer.vr", "db.close()");
    let action = Span::new(3, 8, file);
    let error = TypeError::MoveWhileBorrowed {
        var: "db".into(),
        move_span: action,
        borrow_span: Span::dummy(),
    };
    let diagnostic = error.to_diagnostic_with_span(Some(|s| files.span_to_line_col(s)));
    assert_eq!(
        diagnostic.primary_span(),
        Some(&files.span_to_line_col(action))
    );
    assert!(diagnostic.secondary_labels().is_empty());
}
