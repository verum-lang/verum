//! Failed constant initializers retain their diagnostics through both source routes.
#![cfg(feature = "codegen")]

use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, CodegenErrorKind, ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[test]
fn cyclic_layout_constant_is_not_a_successful_nil_function() {
    let source = "const N: Int = ([Byte; N]).size; fn probe()->Int {N}";
    let ast = Parser::new(source).parse_module().expect("grammar");
    let error = VbcCodegen::new()
        .compile_module(&ast)
        .err()
        .expect("cyclic constant");
    assert!(error.to_string().contains("cyclic constant"), "{error}");
}

#[test]
fn unresolved_initializer_keeps_the_original_expression_error() {
    let ast = Parser::new(
        "const BAD:Int={let local=7; missing_constant_value + local}; fn probe()->Int {BAD}",
    )
    .parse_module()
    .expect("grammar");
    let error = VbcCodegen::new()
        .compile_module(&ast)
        .err()
        .expect("unresolved initializer");
    assert!(
        matches!(error.kind, CodegenErrorKind::UndefinedVariable(ref name) if name == "missing_constant_value"),
        "{error}"
    );
}

#[test]
fn bootstrap_cannot_turn_a_failed_constant_into_a_nil_return() {
    for source in [
        "const N:Int=([Byte;N]).size;",
        "const N:Int=missing_constant_value;",
    ] {
        let ast = Parser::new(source).parse_module().expect("grammar");
        for policy in [
            ItemFailurePolicy::Strict,
            ItemFailurePolicy::StubAndContinue,
        ] {
            let mut codegen = VbcCodegen::new();
            codegen
                .collect_unit_declarations(&[&ast])
                .expect("declarations");
            assert!(
                codegen.compile_unit_items(&[&ast], policy).is_err(),
                "{source}"
            );
            assert!(
                codegen.ctx_mut().current_function.is_none(),
                "failed constant left an active function"
            );
            assert!(
                codegen.ctx_mut().instructions.is_empty(),
                "failed constant retained a partial body"
            );
        }
    }
}

#[test]
fn valid_scalar_record_and_forward_constants_execute_after_wire_roundtrip() {
    for (source, expected) in [
        (
            "const FIRST:Int=SECOND+2; const SECOND:Int=35; fn probe()->Int {FIRST}",
            37,
        ),
        (
            "type Cell is {value:Int}; const CELL:Cell=Cell{value:37}; fn probe()->Int {CELL.value}",
            37,
        ),
        (
            "const WIDTH:Int=([Byte;16]).size; fn probe()->Int {WIDTH}",
            16,
        ),
        ("const NOTHING:Unit={}; fn probe()->Int {NOTHING;37}", 37),
    ] {
        let ast = Parser::new(source).parse_module().expect("grammar");
        let module = VbcCodegen::new().compile_module(&ast).expect(source);
        let bytes = verum_vbc::serialize::serialize_module(&module).expect("write");
        let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("read");
        let entry = module.find_function_by_name("probe").expect("probe");
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(entry)
                .expect("runtime")
                .as_i64(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn forward_cross_module_constants_keep_their_declared_owner() {
    let consumer =
        Parser::new("module consumer; const VALUE:Int=provider.COUNT+2; fn probe()->Int {VALUE}")
            .parse_module()
            .expect("consumer");
    let provider = Parser::new("module provider; public const COUNT:Int=35;")
        .parse_module()
        .expect("provider");
    for reverse in [false, true] {
        let files = if reverse {
            [&provider, &consumer]
        } else {
            [&consumer, &provider]
        };
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("main"));
        codegen
            .collect_unit_declarations(&files)
            .expect("declarations");
        codegen
            .compile_unit_items(&files, ItemFailurePolicy::Strict)
            .expect("bodies");
        let module = codegen.finalize_module().expect("finalize");
        let entry = module
            .find_function_by_name("consumer.probe")
            .expect("exact consumer");
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(entry)
                .expect("runtime")
                .as_i64(),
            37
        );
    }
}
