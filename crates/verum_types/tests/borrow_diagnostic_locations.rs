use verum_ast::Span;
use verum_types::{TypeError, source_files::SourceFileRegistry};

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
                drop_span: origin,
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
