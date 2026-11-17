//! Attribute names use the same named-argument separators for keywords and identifiers.
use verum_ast::{ArrayExpr, BinOp, ExprKind, PathSegment};
use verum_fast_parser::Parser;

#[test]
fn keyword_named_arguments_preserve_assignment_and_array_values() {
    for name in [
        "static", "module", "type", "fn", "const", "unsafe", "proof", "pure", "async", "where",
        "requires", "ensures", "mount", "custom",
    ] {
        for separator in ["=", ":"] {
            for value in ["[]", "[1, 2]"] {
                let source =
                    format!("@arch_module({name} {separator} {value}) module fixture.root;");
                let module = Parser::new(&source)
                    .parse_module()
                    .unwrap_or_else(|error| panic!("{source}: {error:?}"));
                let header = module.file_header_index().unwrap();
                let args = module.items[header].attributes[0].args.as_ref().unwrap();
                assert_eq!(args.len(), 1);
                let ExprKind::Binary {
                    op: BinOp::Assign,
                    left,
                    right,
                } = &args[0].kind
                else {
                    panic!("named argument lost its assignment: {source}");
                };
                let ExprKind::Path(path) = &left.kind else {
                    panic!("missing argument name");
                };
                let PathSegment::Name(ident) = &path.segments[0] else {
                    panic!("missing identifier");
                };
                assert_eq!(ident.name.as_str(), name);
                let ExprKind::Array(ArrayExpr::List(elements)) = &right.kind else {
                    panic!("argument value changed: {source}");
                };
                assert_eq!(elements.len(), if value == "[]" { 0 } else { 2 });
            }
        }
    }
}

#[test]
fn bare_keyword_argument_stays_a_path_and_missing_values_are_rejected() {
    let module = Parser::new("@verify(proof) module fixture.root;")
        .parse_module()
        .unwrap();
    let header = module.file_header_index().unwrap();
    let args = module.items[header].attributes[0].args.as_ref().unwrap();
    assert!(matches!(&args[0].kind, ExprKind::Path(_)));
    for separator in ["=", ":"] {
        let source = format!("@arch_module(requires {separator}) module fixture.root;");
        assert!(Parser::new(&source).parse_module().is_err(), "{source}");
    }
}
