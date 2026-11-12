//! Callable properties belong only to the selected target's executable body.
use verum_ast::cfg::TargetConfig;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str, target: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_cfg_target(TargetConfig::for_target(target));
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().into()),
    );
    errors
}

#[test]
fn excluded_calls_and_local_operations_do_not_contribute_properties() {
    for body in [
        "pending();",
        "let value = async { 7 };",
        "undeclared_platform_call();",
    ] {
        let source = format!(
            "async fn pending() {{}}
            fn handler() {{ @cfg(target_os = \"windows\") {{ {body} }} }}
            static mut HANDLER: fn() = handler;"
        );
        let accepted = errors(&source, "aarch64-apple-darwin");
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
        assert!(
            !errors(&source, "x86_64-pc-windows-msvc").is_empty(),
            "active call ignored: {source}"
        );
    }
}

#[test]
fn target_selection_also_applies_to_purity() {
    let source = "extern \"C\" { fn touch(); }
        pure fn probe() { @cfg(target_os = \"linux\") { unsafe { touch(); } } }";
    let accepted = errors(source, "x86_64-pc-windows-msvc");
    assert!(accepted.is_empty(), "{accepted:?}");
    let rejected = errors(source, "x86_64-unknown-linux-gnu");
    assert!(
        rejected
            .iter()
            .any(|error| error.contains("ImpurePureFunction")),
        "active FFI ignored: {rejected:?}"
    );
}

#[test]
fn structural_sequence_operations_supply_synchronous_call_evidence() {
    for (argument, body) in [
        ("&[Byte]", "data.len()"),
        ("&[Byte]", "let ptr = data.as_ptr(); data.len()"),
        ("&[Byte]", "let ptr = data.offset(1); data.len()"),
        ("&mut [Byte]", "let ptr = data.as_mut_ptr(); data.len()"),
        ("[Byte; 3]", "data.len()"),
    ] {
        let source = format!(
            "fn probe(data: {argument}) -> Int {{ {body} }}
            static mut PROBE: fn({argument}) -> Int = probe;"
        );
        let accepted = errors(&source, "aarch64-apple-darwin");
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
    }
    let rejected = errors(
        "async fn pending() -> Int { 7 }
        fn probe(data: &[Byte]) -> Int { data.len() + pending() }
        static mut PROBE: fn(&[Byte]) -> Int = probe;",
        "aarch64-apple-darwin",
    );
    assert!(
        !rejected.is_empty(),
        "structural operation erased an async sibling"
    );
    assert!(
        !errors(
            "fn invalid(data: &[Byte]) { data.offset(true); }",
            "aarch64-apple-darwin"
        )
        .is_empty(),
        "structural pointer offset accepted a non-integer operand"
    );
}
