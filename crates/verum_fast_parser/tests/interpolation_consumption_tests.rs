//! An interpolation must consume its entire expression (T1418).
//! Spec: Verum Grammar §interpolation / §safe_interpolation.

use verum_ast::FileId;
use verum_fast_parser::VerumParser;

#[test]
fn casts_leave_format_specifiers_owned_by_the_interpolation() {
    use verum_ast::{
        Expr, ExprKind, TypeKind,
        visitor::{Visitor, walk_expr},
    };

    #[derive(Default)]
    struct Inspect {
        casts: Vec<TypeKind>,
        methods: Vec<String>,
    }
    impl Visitor for Inspect {
        fn visit_expr(&mut self, expr: &Expr) {
            match &expr.kind {
                ExprKind::Cast { ty, .. } => self.casts.push(ty.kind.clone()),
                ExprKind::MethodCall { method, .. } => self.methods.push(method.name.to_string()),
                _ => {}
            }
            walk_expr(self, expr);
        }
    }

    let parser = VerumParser::new();
    for (source, expected_type, method) in [
        (r#"f"{value as Int:04x}""#, TypeKind::Int, "to_hex"),
        (r#"f"{value as Int:x}""#, TypeKind::Int, "to_hex"),
        (r#"f"{value as Float:.4}""#, TypeKind::Float, "to_precision"),
        (r#"f"\\u{{{value as Int:04x}}}""#, TypeKind::Int, "to_hex"),
    ] {
        let expression = parser.parse_expr_str(source, FileId::new(0)).expect(source);
        let mut inspected = Inspect::default();
        inspected.visit_expr(&expression);
        assert_eq!(inspected.casts, vec![expected_type], "{source}");
        assert!(
            inspected.methods.iter().any(|actual| actual == method),
            "{source}"
        );
    }
    // Cast syntax admits a refined type, whereas the named sigma binding
    // remains a separate type/binding production.
    parser
        .parse_expr_str("value as Int{> 0}", FileId::new(0))
        .unwrap();
    parser
        .parse_type_str("value: Int where value > 0", FileId::new(0))
        .unwrap();
    assert!(
        parser
            .parse_expr_str("value as Int:04x", FileId::new(0))
            .is_err()
    );
}

#[test]
fn real_stdlib_cast_interpolations_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    for relative in [
        "base/primitives.vr",
        "math/simple.vr",
        "text/char.vr",
        "text/text.vr",
    ] {
        let source = std::fs::read_to_string(root.join(relative)).unwrap();
        VerumParser::new()
            .parse_module_str(&source, FileId::new(0))
            .expect(relative);
    }
}

#[test]
fn trailing_interpolation_tokens_are_refused() {
    let parser = VerumParser::new();
    for source in [
        r#"f"x={a + 1 garbage tokens here}""#,
        r#"f"x={a if false}""#,
        r#"f"x={a; b}""#,
        r#"f"x={a + 1 garbage:x}""#,
        r#"f"""x={a + 1 garbage}""""#,
        r#"html"x={a + 1 garbage}""#,
        r#"html"x={@raw a + 1 garbage}""#,
        r#"sql"id={a + 1 garbage}""#,
        r#"rx"[a-z]{1,256}""#,
    ] {
        let errors = parser
            .parse_expr_str(source, FileId::new(17))
            .expect_err(source);
        assert!(
            errors.iter().any(|error| error.error_code() == "E005"),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn complete_interpolations_still_parse() {
    let parser = VerumParser::new();
    for source in [
        r#"f"x={a + 1}""#,
        r#"f"one={a} two={a + 1}""#,
        r#"f"x={ a + 1 }""#,
        r#"f"x={a:x}""#,
        r#"f"x={(a + 1):04x}""#,
        r#"f"x={if a < b { a } else { b }:04x}""#,
        r#"f"x={if a > b { a } else { b }:04x}""#,
        r#"f"x={\"a:b\"}""#,
        r#"f"x={\"a:b\":>8}""#,
        r#"f"x={{escaped}} {a}""#,
        r#"f"""x={a + 1}""""#,
        r#"html"x={a + 1}""#,
        r#"html"x={@raw a + 1}""#,
        r#"sql"id={a + 1}""#,
    ] {
        parser
            .parse_expr_str(source, FileId::new(17))
            .expect(source);
    }
}

#[test]
fn trailing_token_diagnostic_points_to_its_source_bytes() {
    let parser = VerumParser::new();
    for source in [
        r#"f"x={a + 1 garbage}""#,
        r#"f"привет={   a + 1 garbage}""#,
        r#"html"привет={@raw   a + 1 garbage}""#,
        r#"f"text={\"hello\" garbage}""#,
        r#"f"text={\"привет🙂\" garbage}""#,
    ] {
        let errors = parser
            .parse_expr_str(source, FileId::new(17))
            .expect_err(source);
        let error = errors
            .iter()
            .find(|e| e.error_code() == "E005")
            .expect("E005");
        let start = source.find("garbage").expect("trailing token") as u32;
        assert_eq!(error.span.start, start, "{source}: {error:?}");
        assert_eq!(error.span.end, start + 7, "{source}: {error:?}");
        assert_eq!(error.span.file_id, FileId::new(17));
    }
}

#[test]
fn conformance_specs_pin_both_acceptance_and_refusal() {
    let parser = VerumParser::new();
    for source in [
        include_str!(
            "../../../vcs/specs/L0-critical/parser/expressions/literals/interpolation_trailing_tokens.vr"
        ),
        include_str!(
            "../../../vcs/specs/L0-critical/parser/expressions/literals/interpolation_trailing_guard.vr"
        ),
        include_str!(
            "../../../vcs/specs/L0-critical/parser/expressions/literals/interpolation_raw_trailing_tokens.vr"
        ),
    ] {
        let errors = parser
            .parse_module_str(source, FileId::new(17))
            .expect_err(source);
        assert!(
            errors.iter().any(|error| error.error_code() == "E005"),
            "{errors:?}"
        );
    }
    parser.parse_module_str(
        include_str!("../../../vcs/specs/L0-critical/parser/expressions/literals/interpolation_complete_expression.vr"),
        FileId::new(17),
    ).expect("complete expressions must parse");
}
