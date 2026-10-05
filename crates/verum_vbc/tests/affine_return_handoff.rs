//! T1602: direct affine return transfers its slot before lexical exit cleanup.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const DECLARATIONS: &str = r#"
type Count is { value: Int };
type affine Watch is { counter: &mut Count, digit: Int };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
"#;
fn compile(body: &str, bootstrap: bool) -> Result<VbcModule, Text> {
    let source = format!("{DECLARATIONS}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("return_transfer"));
    if bootstrap {
        codegen
            .collect_unit_declarations(&[&ast])
            .map_err(|e| Text::from(e.to_string()))?;
        codegen
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .map_err(|e| Text::from(e.to_string()))?;
        codegen
            .finalize_module_from_state()
            .map_err(|e| Text::from(e.to_string()))
    } else {
        codegen
            .compile_module(&ast)
            .map_err(|e| Text::from(e.to_string()))
    }
}
fn run(body: &str, bootstrap: bool) -> i64 {
    let module = compile(body, bootstrap).expect("source lowering");
    let probe = module
        .find_function_by_name("return_transfer.probe")
        .unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(probe)
        .expect("execution")
        .as_i64()
}

#[test]
fn direct_return_cleans_other_locals_before_the_caller_receives_ownership() {
    let source = r#"
fn make(counter: &mut Count) -> Watch {
    let first: Watch = Watch { counter, digit: 1 };
    let returned: Watch = Watch { counter, digit: 3 };
    let last: Watch = Watch { counter, digit: 2 };
    return returned;
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let received: Watch = make(&mut counter); if counter.value != 21 { return counter.value; } }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 213, "bootstrap={bootstrap}");
    }
}

#[test]
fn early_return_preserves_the_untaken_branch_and_cleans_nested_shadowed_bindings() {
    for flag in ["true", "false"] {
        let source = format!(
            r#"
fn make(counter: &mut Count, flag: Bool) -> Watch {{
    let held: Watch = Watch {{ counter, digit: 1 }};
    {{
        let held: Watch = Watch {{ counter, digit: 2 }};
        if flag {{
            let returned: Watch = Watch {{ counter, digit: 3 }};
            return returned;
        }}
    }}
    let returned: Watch = Watch {{ counter, digit: 3 }};
    return returned;
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let received: Watch = make(&mut counter, {flag}); if counter.value != 21 {{ return counter.value; }} }}
    counter.value
}}
"#
        );
        assert_eq!(run(&source, false), 213, "flag={flag}");
    }
}

#[test]
fn returned_moved_local_and_explicit_drop_keep_one_obligation() {
    let source = r#"
fn make(counter: &mut Count) -> Watch {
    let original: Watch = Watch { counter, digit: 4 };
    let returned = original;
    return returned;
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let received: Watch = make(&mut counter); if counter.value != 0 { return 99; } drop(received); }
    counter.value
}
"#;
    assert_eq!(run(source, false), 4);
}

#[test]
fn unsupported_affine_result_origins_refuse_in_both_source_routes() {
    for body in [
        "fn make(value: Watch) -> Watch { return value; }",
        "fn make(counter: &mut Count) -> Watch { let value: Watch = Watch { counter, digit: 1 }; defer { counter.value = 99; } return value; }",
        "fn make(a: Watch, b: Watch, flag: Bool) -> Watch { return if flag { a } else { b }; }",
        "fn make(value: Watch) -> Watch { let ref borrowed = value; return borrowed; }",
        "fn make(value: Watch) -> Watch { let whole @ Watch { counter, digit } = value; return whole; }",
    ] {
        for bootstrap in [false, true] {
            let error = compile(body, bootstrap)
                .err()
                .expect("unproven affine result must not gain cleanup authority");
            assert!(
                error.contains("affine return") && error.contains("direct local"),
                "{body}: {error}"
            );
        }
    }
}

#[test]
fn explicit_return_in_each_branch_transfers_only_the_selected_local() {
    for (flag, expected) in [("true", 21), ("false", 12)] {
        let source = format!(
            r#"
fn make(counter: &mut Count, flag: Bool) -> Watch {{
    let first: Watch = Watch {{ counter, digit: 1 }};
    let second: Watch = Watch {{ counter, digit: 2 }};
    if flag {{ return first; }} else {{ return second; }}
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let received: Watch = make(&mut counter, {flag}); }}
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), expected);
        }
    }
}

#[test]
fn borrowed_and_raw_locals_do_not_destroy_the_returned_referent() {
    let source = r#"
fn make(counter: &mut Count) -> Watch {
    let returned: Watch = Watch { counter, digit: 6 };
    let borrowed: &Watch = &returned;
    let raw: *const Watch = &returned as *const Watch;
    return returned;
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let received: Watch = make(&mut counter); if counter.value != 0 { return 99; } }
    counter.value
}
"#;
    assert_eq!(run(source, false), 6);
    let aliased = source
        .replace("fn make", "type Borrow is &Watch; fn make")
        .replace("let borrowed: &Watch", "let borrowed: Borrow");
    assert_eq!(run(&aliased, false), 6, "reference alias");
    let generic_alias = source
        .replace("fn make", "type Borrow<T> is &T; fn make")
        .replace("let borrowed: &Watch", "let borrowed: Borrow<Watch>");
    assert_eq!(run(&generic_alias, false), 6, "generic reference alias");
    // A borrowed function result does not select the affine return path.
    compile(
        "fn borrow(value: &Watch) -> &Watch { return value; }",
        false,
    )
    .unwrap();
    compile("fn opaque<T>(value: T) -> T { return value; }", false).unwrap();
}

#[test]
fn direct_handoff_survives_wire_without_observation_receipts() {
    use verum_vbc::{deserialize::deserialize_module, serialize::serialize_module};
    let source = r#"
fn make(counter: &mut Count) -> Watch {
    let dropped: Watch = Watch { counter, digit: 1 };
    let returned: Watch = Watch { counter, digit: 2 };
    return returned;
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let received: Watch = make(&mut counter); }
    counter.value
}
"#;
    let mut module = compile(source, true).unwrap();
    for function in &mut module.functions {
        function.value_uses = None;
    }
    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();
    let probe = loaded
        .find_function_by_name("return_transfer.probe")
        .unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(loaded))
            .execute_function(probe)
            .unwrap()
            .as_i64(),
        12
    );
}

#[test]
fn fresh_affine_sums_keep_the_existing_constructor_route_without_new_permission() {
    // Mirrors the fresh Result.Ok(Guard { ... }) / Err(e) return shapes in
    // the actual MySQL/Postgres pool APIs. This checks compilation only:
    // aggregate destruction is a separate, still-unimplemented contract.
    let source = r#"
type Outcome is Ready(Watch) | Failed(Int);
fn make(counter: &mut Count, flag: Bool) -> Outcome {
    if flag { return Outcome.Ready(Watch { counter, digit: 7 }); }
    return Outcome.Failed(17);
}
"#;
    for bootstrap in [false, true] {
        let module = compile(source, bootstrap).unwrap();
        let outcome = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Outcome"))
            .unwrap();
        assert_eq!(
            module.resource_discipline(&verum_vbc::TypeRef::Concrete(outcome.id)),
            verum_common::ResourceDiscipline::Affine
        );
    }
}

#[test]
fn aggregate_and_call_uses_of_named_affine_locals_cannot_bypass_the_guard() {
    for source in [
        r#"
type Outcome is Ready(Watch) | Failed(Int);
fn make(counter: &mut Count) -> Outcome {
    let held: Watch = Watch { counter, digit: 1 };
    return Outcome.Ready(held);
}
"#,
        r#"
fn make(counter: &mut Count, flag: Bool) -> Watch {
    let held: Watch = Watch { counter, digit: 1 };
    return if flag { held } else { Watch { counter, digit: 2 } };
}
"#,
        r#"
fn forward(value: Watch) -> Watch { value }
fn make(counter: &mut Count) -> Watch {
    let held: Watch = Watch { counter, digit: 1 };
    return forward(held);
}
"#,
    ] {
        for bootstrap in [false, true] {
            let error = compile(source, bootstrap)
                .err()
                .expect("named affine handoff needs its producer");
            assert!(
                error.contains("affine return") && error.contains("direct local"),
                "{error}"
            );
        }
    }
}

#[test]
fn returned_nested_scope_does_not_resolve_departed_names_as_outer_affine_bindings() {
    let source = r#"
type Outcome is Ready(Watch) | Failed(Int);
fn make(counter: &mut Count) -> Outcome {
    let held: Watch = Watch { counter, digit: 1 };
    return { let held: Int = 7; Outcome.Failed(held) };
}
"#;
    for bootstrap in [false, true] {
        let module = compile(source, bootstrap).expect("inner Int is not the outer affine binding");
        let outcome = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Outcome"))
            .unwrap();
        assert_eq!(
            module.resource_discipline(&verum_vbc::TypeRef::Concrete(outcome.id)),
            verum_common::ResourceDiscipline::Affine
        );
    }
}
