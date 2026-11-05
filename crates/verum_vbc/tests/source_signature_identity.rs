//! Complete body-free signatures compare independently of source positions.
use verum_ast::source_signature::{capture_source_signature, semantically_equal_signatures};
use verum_ast::{Expr, FileId, FunctionDecl, ItemKind, Literal};
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
fn signature(source: &str, file: u32) -> FunctionDecl {
    let file = FileId::new(file);
    let module = VerumParser::new()
        .parse_module(Lexer::new(source, file), file)
        .unwrap();
    let function = module
        .items
        .iter()
        .find_map(|item| {
            if let ItemKind::Function(function) = &item.kind {
                Some(function)
            } else {
                None
            }
        })
        .unwrap();
    capture_source_signature(function, function.is_method()).unwrap()
}
const SOURCE: &str = "@intrinsic(\"identity_fixture\") public fn selected<T: Ord=Int,N:meta Int>(value:T)->T requires true;";
#[test]
fn shifted_locations_preserve_complete_signature() {
    let left = signature(SOURCE, 1);
    let right = signature(&format!("// moved source\n\n{SOURCE}"), 937);
    assert_ne!(left, right);
    assert!(semantically_equal_signatures(&left, &right).unwrap());
}
#[test]
fn semantic_header_changes_remain_distinct() {
    let left = signature(SOURCE, 1);
    for changed in [
        SOURCE.replace("Ord=Int", "Clone=Int"),
        SOURCE.replace("Ord=Int", "Ord=Bool"),
        SOURCE.replace("meta Int", "meta Bool"),
        SOURCE.replace("public fn", "public async fn"),
        SOURCE.replace("requires true", "requires false"),
        SOURCE.replace("identity_fixture", "other_intrinsic"),
        SOURCE.replace("value:T", "other:T"),
    ] {
        assert!(
            !semantically_equal_signatures(&left, &signature(&changed, 1)).unwrap(),
            "{changed}"
        );
    }
}
#[test]
fn user_location_spellings_are_not_erased() {
    let left = signature(
        "@attribute(span=\"Span\",file_id=\"FileId\") public fn selected(value:Text=\"span\")->Text;",
        1,
    );
    for source in [
        "@attribute(span=\"different\",file_id=\"FileId\") public fn selected(value:Text=\"span\")->Text;",
        "@attribute(span=\"Span\",file_id=\"different\") public fn selected(value:Text=\"span\")->Text;",
        "@attribute(span=\"Span\",file_id=\"FileId\") public fn selected(value:Text=\"file_id\")->Text;",
    ] {
        assert!(!semantically_equal_signatures(&left, &signature(source, 1)).unwrap());
    }
}
#[test]
fn literal_float_bits_are_preserved() {
    let mut left = signature("public fn selected()->Float;", 1);
    let mut right = left.clone();
    left.requires
        .push(Expr::literal(Literal::float(0.0, left.span)));
    right
        .requires
        .push(Expr::literal(Literal::float(-0.0, right.span)));
    assert!(!semantically_equal_signatures(&left, &right).unwrap());
    let nan = f64::from_bits(0x7ff8_0000_0000_0001);
    left.requires[0] = Expr::literal(Literal::float(nan, left.span));
    right.requires[0] = Expr::literal(Literal::float(nan, right.span));
    assert!(semantically_equal_signatures(&left, &right).unwrap());
    right.requires[0] = Expr::literal(Literal::float(
        f64::from_bits(0x7ff8_0000_0000_0002),
        right.span,
    ));
    assert!(!semantically_equal_signatures(&left, &right).unwrap());
}
#[test]
fn executable_body_is_not_admitted_as_signature() {
    let file = FileId::new(1);
    let module = VerumParser::new()
        .parse_module(Lexer::new("fn selected()->Int{1}", file), file)
        .unwrap();
    let ItemKind::Function(source) = &module.items[0].kind else {
        panic!()
    };
    assert!(semantically_equal_signatures(source, source).is_err());
}
#[test]
fn oversized_signature_is_refused_before_identity_allocation() {
    let mut signature = signature("fn selected()->Text;", 1);
    signature.requires.push(Expr::literal(Literal::string(
        "x".repeat(1_048_577).into(),
        signature.span,
    )));
    assert!(semantically_equal_signatures(&signature, &signature).is_err());
}
