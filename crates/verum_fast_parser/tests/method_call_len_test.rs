use verum_ast::{Expr, ExprKind, FileId};
use verum_fast_parser::VerumParser;

fn parse_expr_test(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

#[test]
fn test_query_len() {
    let expr = parse_expr_test("query.len()");
    match &expr.kind {
        ExprKind::MethodCall {
            receiver,
            method,
            type_args: _,
            args,
        } => {
            // Verify receiver is 'query'
            if let ExprKind::Path(path) = &receiver.kind {
                assert_eq!(path.segments.len(), 1);
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    assert_eq!(ident.name.as_str(), "query");
                } else {
                    panic!("Expected name segment");
                }
            } else {
                panic!("Expected path for receiver, got: {:?}", receiver.kind);
            }

            // Verify method name is 'len'
            assert_eq!(method.name.as_str(), "len");

            // Verify no arguments
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Expected MethodCall, got: {:?}", expr.kind),
    }
}

#[test]
fn test_text_is_empty() {
    let expr = parse_expr_test("text.is_empty()");
    match &expr.kind {
        ExprKind::MethodCall {
            receiver,
            method,
            type_args: _,
            args,
        } => {
            // Verify receiver is 'text'
            if let ExprKind::Path(path) = &receiver.kind {
                assert_eq!(path.segments.len(), 1);
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    assert_eq!(ident.name.as_str(), "text");
                } else {
                    panic!("Expected name segment");
                }
            } else {
                panic!("Expected path for receiver, got: {:?}", receiver.kind);
            }

            // Verify method name is 'is_empty'
            assert_eq!(method.name.as_str(), "is_empty");

            // Verify no arguments
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Expected MethodCall, got: {:?}", expr.kind),
    }
}

#[test]
fn test_req_query_len() {
    let expr = parse_expr_test("req.query.len()");
    match &expr.kind {
        ExprKind::MethodCall {
            receiver,
            method,
            type_args: _,
            args,
        } => {
            // Verify receiver is 'req.query' (field access)
            if let ExprKind::Field {
                expr: field_expr,
                field,
            } = &receiver.kind
            {
                // Verify base is 'req'
                if let ExprKind::Path(path) = &field_expr.kind {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "req");
                    } else {
                        panic!("Expected name segment");
                    }
                } else {
                    panic!("Expected path for base, got: {:?}", field_expr.kind);
                }

                // Verify field is 'query'
                assert_eq!(field.name.as_str(), "query");
            } else {
                panic!("Expected Field for receiver, got: {:?}", receiver.kind);
            }

            // Verify method name is 'len'
            assert_eq!(method.name.as_str(), "len");

            // Verify no arguments
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Expected MethodCall, got: {:?}", expr.kind),
    }
}

#[test]
fn test_path_vs_method_call() {
    // Test that 'query.len' without parens is a Field access
    let expr = parse_expr_test("query.len");
    match &expr.kind {
        ExprKind::Field { expr, field } => {
            if let ExprKind::Path(path) = &expr.kind {
                assert_eq!(path.segments.len(), 1);
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    assert_eq!(ident.name.as_str(), "query");
                } else {
                    panic!("Expected name segment");
                }
            } else {
                panic!("Expected path, got: {:?}", expr.kind);
            }
            assert_eq!(field.name.as_str(), "len");
        }
        _ => panic!("Expected Field access, got: {:?}", expr.kind),
    }

    // Test that 'query.len()' with parens is a MethodCall
    let expr = parse_expr_test("query.len()");
    match &expr.kind {
        ExprKind::MethodCall {
            receiver,
            method,
            type_args: _,
            args,
        } => {
            if let ExprKind::Path(path) = &receiver.kind {
                assert_eq!(path.segments.len(), 1);
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    assert_eq!(ident.name.as_str(), "query");
                } else {
                    panic!("Expected name segment");
                }
            } else {
                panic!("Expected path, got: {:?}", receiver.kind);
            }
            assert_eq!(method.name.as_str(), "len");
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Expected MethodCall, got: {:?}", expr.kind),
    }
}
