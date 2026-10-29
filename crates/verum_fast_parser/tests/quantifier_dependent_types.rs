use verum_ast::{Expr, ExprKind, decl::ItemKind};
use verum_common::span::FileId;
use verum_fast_parser::FastParser;

fn parse(expression: &str) -> Expr {
    let source = format!("theorem t(): {expression} proof by assumption;");
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .unwrap_or_else(|errors| panic!("{source}\n{errors:?}"));
    let ItemKind::Theorem(theorem) = &module.items[0].kind else {
        panic!("theorem")
    };
    *theorem.proposition.clone()
}

#[test]
fn quantifier_annotations_share_the_regular_type_parser() {
    for quantifier in ["forall", "exists"] {
        for ty in [
            "Path<a>(x,y)",
            "p(x)",
            "Indexed<a>(x)",
            "Type",
            "Type(3)",
            "Type(u)",
            "fn(a)->p(x)",
            "fn<t:meta a>(p(t))->p(t)",
            "fn(core.Model.Type)->a",
            "&p(x)",
            "&&a",
            "*const p(x)",
            "meta p(x)",
            "List<core.Model.Type>",
            "(core.Model.Type,p(x))",
            "[core.Model.Type;4]",
            "GenRef<p(x)>",
        ] {
            // Replace delimiters with spaces so source coordinates remain identical.
            // Compare the complete AST, including every dependent argument and span.
            let bare = parse(&format!("{quantifier}  h:{ty} . check(h)"));
            let grouped = parse(&format!("{quantifier} (h:{ty}). check(h)"));
            assert_eq!(bare, grouped, "{quantifier} {ty}");
            let body = match bare.kind {
                ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => body,
                _ => panic!("quantifier"),
            };
            assert!(
                matches!(body.kind, ExprKind::Call { .. }),
                "body must survive the type boundary"
            );
        }
    }
}

#[test]
fn guards_domains_and_multiple_bindings_keep_their_ownership() {
    for expression in [
        "forall h:p(x) where enabled(h). h == h",
        "forall h:p(x) in values. h == h",
        "exists h:Path<a>(x,y), k:Path<a>(y,z). valid(h,k)",
        "forall h:fn(a)->p(x) where enabled(h). h == h",
    ] {
        let parsed = parse(expression);
        let bindings = match &parsed.kind {
            ExprKind::Forall { bindings, .. } | ExprKind::Exists { bindings, .. } => bindings,
            _ => panic!("quantifier"),
        };
        assert_eq!(bindings[0].guard.is_some(), expression.contains("where"));
        assert_eq!(bindings[0].domain.is_some(), expression.contains(" in "));
        assert_eq!(
            bindings.len(),
            if expression.contains(", k:") { 2 } else { 1 }
        );
    }
}

#[test]
fn malformed_dependent_annotations_are_not_truncated_to_valid_prefixes() {
    for ty in [
        "Path<a>(x)",
        "Path<a>(x,)",
        "p(",
        "Type()",
        "Indexed<a>(x,",
        "fn(a)->",
    ] {
        let source = format!("theorem t():forall h:{ty}. check(h) proof by assumption;");
        assert!(
            FastParser::new()
                .parse_module_str(&source, FileId::new(0))
                .is_err(),
            "{source}"
        );
    }
}
