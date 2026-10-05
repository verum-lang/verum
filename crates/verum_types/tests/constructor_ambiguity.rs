//! T1593: constructor ownership is contextual, never declaration-order choice.
use std::process::{Command, Output};
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    let early_modules = std::env::var("T1593_CASE").as_deref() == Ok("foreign_early");
    if early_modules {
        for item in &ast.items {
            if matches!(item.kind, ItemKind::Module(_)) {
                checker.check_item(item).expect("earlier module");
            }
        }
    }
    for item in &ast.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("type registration");
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(decl) = &item.kind {
            checker
                .register_function_signature(decl)
                .expect("function registration");
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err())
        .map(|error| Text::from(format!("{error:?}")))
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

fn in_mode(mode: &str, case: &str) -> Output {
    Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "constructor_law_child", "--nocapture"])
        .env("VERUM_LANGUAGE_LAWS", mode)
        .env("T1593_CASE", case)
        .output()
        .expect("isolated law mode")
}

#[test]
fn constructor_law_child() {
    let Ok(case) = std::env::var("T1593_CASE") else {
        return;
    };
    for reversed in [false, true] {
        let payload = matches!(
            case.as_str(),
            "payload_ambiguous"
                | "function"
                | "closure"
                | "payload_expected"
                | "foreign_function"
                | "foreign_early"
        );
        let declarations = if payload && reversed {
            "type B is Failed | Pending(Int); type A is Pending(Int) | Done;"
        } else if payload {
            "type A is Pending(Int) | Done; type B is Failed | Pending(Int);"
        } else if reversed {
            "type B is Pending | Failed; type A is Pending | Done;"
        } else {
            "type A is Pending | Done; type B is Pending | Failed;"
        };
        let body = match case.as_str() {
            "foreign_function" | "foreign_early" => {
                "module foreign { public fn Pending(value: Int) -> Int { value + 30 } } fn probe() { let value = Pending(7); }"
            }
            "payload_ambiguous" => "fn probe() { let value = Pending(7); }",
            "payload_expected" => "fn a() -> A { Pending(30) } fn b() -> B { Pending(7) }",
            "function" => {
                "fn Pending(value: Int) -> Int { value + 30 } fn probe() -> Int { Pending(7) }"
            }
            "closure" => "fn probe() -> Int { let Pending = |value: Int| value + 30; Pending(7) }",
            "ambiguous" => "fn probe() { let value = Pending; }",
            "expected" => "fn a() -> A { Pending } fn b() -> B { Pending }",
            "callee" => {
                "fn take_a(value: A) {} fn take_b(value: B) {} fn probe() { take_a(Pending); take_b(Pending); }"
            }
            "qualified" => "fn a() -> A { A.Pending } fn b() -> B { B.Pending }",
            "lexical" => "fn probe() -> Int { let Pending = 37; Pending }",
            _ => panic!("unknown fixture"),
        };
        let found = errors(&format!("{declarations} {body}"));
        eprintln!("case={case} reverse={reversed} errors={found:?}");
        if matches!(
            case.as_str(),
            "ambiguous" | "payload_ambiguous" | "foreign_function" | "foreign_early"
        ) && std::env::var("VERUM_LANGUAGE_LAWS").as_deref() == Ok("strict")
        {
            assert!(
                found.iter().any(|error| error.contains("E431")),
                "strict ambiguity must diagnose exact owners: {found:?}"
            );
        } else {
            assert!(found.is_empty(), "{case}, reverse={reversed}: {found:?}");
        }
    }
}

fn assert_ambiguity_mode(mode: &str) {
    let result = in_mode(mode, "ambiguous");
    let stderr = Text::from_utf8(result.stderr).expect("diagnostic UTF8");
    assert!(result.status.success(), "{mode}: {stderr}");
    assert!(
        stderr.contains("E431"),
        "{mode} must diagnose ambiguity: {stderr}"
    );
    assert!(
        stderr.contains("A") && stderr.contains("B"),
        "owners: {stderr}"
    );
}

#[test]
fn ambiguous_constructor_warns_in_ordinary_mode() {
    assert_ambiguity_mode("warn");
}

#[test]
fn ambiguous_constructor_errors_in_strict_mode() {
    assert_ambiguity_mode("strict");
}

#[test]
fn callee_parameter_owner_disambiguates_both_orders() {
    for mode in ["warn", "strict"] {
        let result = in_mode(mode, "callee");
        let stderr = Text::from_utf8(result.stderr).expect("diagnostic UTF8");
        assert!(result.status.success(), "{mode}: {stderr}");
        assert!(!stderr.contains("E431"), "known parameter owner: {stderr}");
    }
}

#[test]
fn expected_owner_disambiguates_both_constructor_orders() {
    for mode in ["warn", "strict"] {
        let result = in_mode(mode, "expected");
        let stderr = Text::from_utf8(result.stderr).expect("diagnostic UTF8");
        assert!(result.status.success(), "{mode}: {stderr}");
        assert!(!stderr.contains("E431"), "known expected owner: {stderr}");
    }
}

#[test]
fn qualified_constructor_keeps_its_exact_owner_in_both_modes() {
    for mode in ["warn", "strict"] {
        let result = in_mode(mode, "qualified");
        let stderr = Text::from_utf8(result.stderr).expect("diagnostic UTF8");
        assert!(result.status.success(), "{mode}: {stderr}");
        assert!(!stderr.contains("E431"), "explicit owner: {stderr}");
    }
}

#[test]
fn lexical_value_binding_outranks_constructor_names() {
    for mode in ["warn", "strict"] {
        let result = in_mode(mode, "lexical");
        let stderr = Text::from_utf8(result.stderr).expect("diagnostic UTF8");
        assert!(result.status.success(), "{mode}: {stderr}");
        assert!(!stderr.contains("E431"), "lexical binding: {stderr}");
    }
}

#[test]
fn payload_ambiguity_is_diagnosed_in_both_modes() {
    for mode in ["warn", "strict"] {
        let result = in_mode(mode, "payload_ambiguous");
        let stderr = Text::from_utf8(result.stderr).expect("UTF8");
        assert!(result.status.success(), "{mode}: {stderr}");
        assert!(stderr.contains("E431"), "{stderr}");
    }
}

#[test]
fn payload_expected_owner_and_explicit_function_bindings_win() {
    for case in ["payload_expected", "function", "closure"] {
        for mode in ["warn", "strict"] {
            let result = in_mode(mode, case);
            let stderr = Text::from_utf8(result.stderr).expect("UTF8");
            assert!(result.status.success(), "{case}/{mode}: {stderr}");
            assert!(!stderr.contains("E431"), "{stderr}");
        }
    }
}

#[test]
fn foreign_function_does_not_authorize_root_constructor_binding() {
    for case in ["foreign_function", "foreign_early"] {
        for mode in ["warn", "strict"] {
            let result = in_mode(mode, case);
            let stderr = Text::from_utf8(result.stderr).expect("UTF8");
            assert!(result.status.success(), "{case}/{mode}: {stderr}");
            assert!(stderr.contains("E431"), "{stderr}");
        }
    }
}
