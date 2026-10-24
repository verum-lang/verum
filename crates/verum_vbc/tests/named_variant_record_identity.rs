#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

const DECLARATIONS: &str = r#"
module wire;
type Envelope<T> is Wrap(T);
type Request is Empty | Initial { value: Int, code: Int } | Final { value: Int };
"#;

fn execute(source: &str, import_shapes: bool) -> i64 {
    let declarations = Parser::new(DECLARATIONS).parse_module().unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("wire"));
    let archive = producer.compile_module(&declarations).unwrap();
    let archive = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&archive).unwrap(),
    )
    .unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("wire"));
    consumer.import_functions(&producer.export_functions());
    if import_shapes {
        // Bootstrap's nominal import registers named payload layouts as well
        // as records. A payload layout must not invent a separate record type.
        consumer.import_archive_module_types(&archive);
    }
    let ast = Parser::new(&format!("{DECLARATIONS}\n{source}"))
        .parse_module()
        .unwrap();
    let compiled = consumer.compile_module(&ast).unwrap();
    let compiled = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
    )
    .unwrap();
    let entry = compiled
        .functions
        .iter()
        .find(|f| compiled.get_string(f.name) == Some("wire.probe"))
        .unwrap()
        .id;
    Interpreter::new(Arc::new(compiled))
        .execute_function(entry)
        .unwrap()
        .as_i64()
}

#[test]
fn bare_named_variants_inside_generic_returns_keep_their_sum_identity() {
    for imported in [false, true] {
        assert_eq!(
            execute(
                r#"
fn make() -> Envelope<Request> { Envelope.Wrap(Initial { code: 2, value: 40 }) }
fn probe() -> Int {
    match make() {
        Envelope.Wrap(Request.Initial { value, code }) => value * 100 + code,
        _ => -1,
    }
}
"#,
                imported
            ),
            4002
        );
    }
}

#[test]
fn qualified_named_variants_keep_their_declared_payload_order() {
    assert_eq!(
        execute(
            r#"
fn make() -> Envelope<Request> { Envelope.Wrap(Request.Initial { code: 2, value: 40 }) }
fn probe() -> Int {
    match make() {
        Envelope.Wrap(Request.Initial { value, code }) => value * 100 + code,
        _ => -1,
    }
}
"#,
            true
        ),
        4002
    );
}

#[test]
fn a_declared_record_is_not_replaced_by_an_imported_variant_payload() {
    assert_eq!(
        execute(
            r#"
module application {
    type Initial is { value: Int, code: Int };
    fn make() -> Initial { Initial { code: 2, value: 40 } }
    public fn answer() -> Int { let record = make(); record.value * 100 + record.code }
}
fn probe() -> Int { application.answer() }
"#,
            true
        ),
        4002
    );
}
