//! T1558: resource constraints belong to resolved declarations, not leaf names.
use verum_ast::{ItemKind, Module};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("valid source")
}
fn declarations(checker: &mut TypeChecker, module: &Module, owner: &str) {
    checker.set_current_module_path(owner);
    for item in &module.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("source declaration");
        }
    }
}
fn errors(reverse: bool, source: &str) -> Vec<String> {
    errors_with_declarations(
        reverse,
        "public type affine Token is { id: Int };",
        "public type Token is { pad: Int, id: Int };",
        source,
    )
}
fn errors_with_declarations(reverse: bool, alpha: &str, beta: &str, source: &str) -> Vec<String> {
    let mut checker = TypeChecker::new();
    let alpha = parse(alpha);
    let beta = parse(beta);
    if reverse {
        declarations(&mut checker, &beta, "beta");
        declarations(&mut checker, &alpha, "alpha");
    } else {
        declarations(&mut checker, &alpha, "alpha");
        declarations(&mut checker, &beta, "beta");
    }
    let module = parse(source);
    declarations(&mut checker, &module, "consumer");
    let mut errors = Vec::new();
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            if let Err(e) = checker.register_function_signature(decl) {
                errors.push(format!("{e:?}"));
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(_) = &item.kind {
            if let Err(e) = checker.check_item(item) {
                errors.push(format!("{e:?}"));
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors
}
fn accepted(source: &str) {
    for reverse in [false, true] {
        let errors = errors(reverse, source);
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}
fn moved(source: &str) {
    for reverse in [false, true] {
        let errors = errors(reverse, source);
        assert!(
            errors.iter().any(|e| e.contains("MovedValueUsed")),
            "reverse={reverse}: {errors:?}"
        );
    }
}

#[test]
fn unrelated_qualified_ordinary_type_remains_reusable_in_both_orders() {
    accepted("fn consume(x: beta.Token) {} fn probe(x: beta.Token) { consume(x); consume(x); }");
}
#[test]
fn qualified_affine_type_still_rejects_second_consumption_in_both_orders() {
    moved("fn consume(x: alpha.Token) {} fn probe(x: alpha.Token) { consume(x); consume(x); }");
}
#[test]
fn qualified_affine_borrows_do_not_consume_in_both_orders() {
    accepted(
        "fn observe(x: &alpha.Token) {} fn consume(x: alpha.Token) {} fn probe(x: alpha.Token) { observe(&x); observe(&x); consume(x); }",
    );
}
#[test]
fn ordinary_local_declaration_shadows_foreign_resource() {
    accepted(
        "type Token is { id: Int }; fn consume(x: Token) {} fn probe(x: Token) { consume(x); consume(x); }",
    );
}
#[test]
fn alias_of_qualified_ordinary_type_remains_reusable() {
    accepted(
        "type Alias is beta.Token; fn consume(x: Alias) {} fn probe(x: Alias) { consume(x); consume(x); }",
    );
}
#[test]
fn alias_of_qualified_affine_type_keeps_consumption_constraint() {
    moved(
        "type Alias is alpha.Token; fn consume(x: Alias) {} fn probe(x: Alias) { consume(x); consume(x); }",
    );
}
#[test]
fn unknown_owner_cannot_borrow_a_foreign_resource_constraint() {
    for reverse in [false, true] {
        let errors = errors(
            reverse,
            "fn consume(x: missing.Token) {} fn probe(x: missing.Token) { consume(x); consume(x); }",
        );
        assert!(
            !errors.iter().any(|e| e.contains("MovedValueUsed")),
            "unknown owner borrowed affine alpha.Token: {errors:?}"
        );
    }
}

#[test]
fn generic_alias_preserves_affine_owner_and_substitution() {
    for reverse in [false, true] {
        let errors = errors_with_declarations(
            reverse,
            "public type affine Token<T> is { value: T };",
            "public type Token<T> is { pad: Int, value: T };",
            "type Alias<T> is alpha.Token<T>; fn consume(x: Alias<Int>) {} fn probe(x: Alias<Int>) { consume(x); consume(x); }",
        );
        assert!(
            errors.iter().any(|e| e.contains("MovedValueUsed")),
            "{errors:?}"
        );
        assert!(
            !errors.iter().any(|e| e.contains("Mismatch")),
            "alias lost substitution: {errors:?}"
        );
    }
}

#[test]
fn alias_preserves_linear_must_consume_constraint() {
    for reverse in [false, true] {
        let errors = errors_with_declarations(
            reverse,
            "public type linear Token is { id: Int };",
            "public type Token is { pad: Int, id: Int };",
            "type Alias is alpha.Token; fn make() -> Alias { alpha.Token { id: 1 } } fn probe() { let token = make(); }",
        );
        assert!(
            !errors.is_empty() && errors.iter().all(|e| e.contains("LinearNotConsumed")),
            "{errors:?}"
        );
    }
}

fn inline_errors(source: &str) -> Vec<String> {
    let mut checker = TypeChecker::new();
    let module = parse(source);
    let mut errors = Vec::new();
    for item in &module.items {
        if matches!(item.kind, ItemKind::Module(_) | ItemKind::Mount(_)) {
            if let Err(e) = checker.check_item(item) {
                errors.push(format!("{e:?}"));
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            if let Err(e) = checker.register_function_signature(decl) {
                errors.push(format!("{e:?}"));
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(_) = &item.kind {
            if let Err(e) = checker.check_item(item) {
                errors.push(format!("{e:?}"));
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors
}

#[test]
fn mounted_ordinary_sibling_does_not_acquire_affine_constraint() {
    let alpha = "module alpha { public type affine Token is { id: Int }; }";
    let beta = "module beta { public type Token is { pad: Int, id: Int }; }";
    for prefix in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let source = format!(
            "{prefix} mount beta.{{Token}}; fn consume(x: Token) {{}} fn probe(x: Token) {{ consume(x); consume(x); }}"
        );
        let errors = inline_errors(&source);
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn mounted_affine_sibling_retains_constraint() {
    let alpha = "module alpha { public type affine Token is { id: Int }; }";
    let beta = "module beta { public type Token is { pad: Int, id: Int }; }";
    for prefix in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let source = format!(
            "{prefix} mount alpha.{{Token}}; fn consume(x: Token) {{}} fn probe(x: Token) {{ consume(x); consume(x); }}"
        );
        let errors = inline_errors(&source);
        assert!(
            errors.iter().any(|e| e.contains("MovedValueUsed")),
            "{errors:?}"
        );
    }
}

#[test]
fn inline_qualified_siblings_keep_their_resource_identity() {
    let alpha = "module alpha { public type affine Token is { id: Int }; }";
    let beta = "module beta { public type Token is { pad: Int, id: Int }; }";
    for prefix in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let ordinary = inline_errors(&format!(
            "{prefix} fn consume(x: beta.Token) {{}} fn probe(x: beta.Token) {{ consume(x); consume(x); }}"
        ));
        assert!(ordinary.is_empty(), "{ordinary:?}");
        let affine = inline_errors(&format!(
            "{prefix} fn consume(x: alpha.Token) {{}} fn probe(x: alpha.Token) {{ consume(x); consume(x); }}"
        ));
        assert!(
            affine.iter().any(|e| e.contains("MovedValueUsed")),
            "{affine:?}"
        );
    }
}

#[test]
fn generic_ordinary_alias_does_not_borrow_foreign_affine_mode() {
    for reverse in [false, true] {
        let errors = errors_with_declarations(
            reverse,
            "public type affine Token<T> is { value: T };",
            "public type Token<T> is { pad: Int, value: T };",
            "type Alias<T> is beta.Token<T>; fn consume(x: Alias<Int>) {} fn probe(x: Alias<Int>) { consume(x); consume(x); }",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn mounted_generic_alias_keeps_the_declaring_target_in_both_orders() {
    let alpha = "module alpha { public type affine Token<T> is { value: T }; public type Alias<T> is Token<T>; }";
    let beta = "module beta { public type Token<T> is { pad: Int, value: T }; public type Alias<T> is Token<T>; }";
    for prefix in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let ordinary = inline_errors(&format!(
            "{prefix} mount beta.{{Alias}}; fn consume(x: Alias<Int>) {{}} fn probe(x: Alias<Int>) {{ consume(x); consume(x); }}"
        ));
        assert!(ordinary.is_empty(), "{ordinary:?}");
        let affine = inline_errors(&format!(
            "{prefix} mount alpha.{{Alias}}; fn consume(x: Alias<Int>) {{}} fn probe(x: Alias<Int>) {{ consume(x); consume(x); }}"
        ));
        assert!(
            affine.iter().any(|e| e.contains("MovedValueUsed")),
            "{affine:?}"
        );
        assert!(!affine.iter().any(|e| e.contains("Mismatch")), "{affine:?}");
    }
}

#[test]
fn mounted_generic_alias_unifies_with_its_qualified_target() {
    let alpha = "module alpha { public type affine Token<T> is { value: T }; public type Alias<T> is Token<T>; }";
    let beta = "module beta { public type Token<T> is { pad: Int, value: T }; public type Alias<T> is Token<T>; }";
    for prefix in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let errors = inline_errors(&format!(
            "{prefix} mount alpha.{{Alias}}; fn consume(x: alpha.Token<Int>) {{}} fn probe(x: Alias<Int>) {{ consume(x); }}"
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }
}
