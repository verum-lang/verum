#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

// T1505: these are the source spellings consumed by the archive's checker
// schemes. A module root is not a nominal type, even inside a generic.
fn assert_signature(source: &str, expected: &str) {
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("signature_names"))
        .compile_module(&ast)
        .expect("compile");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("deserialize");
    let function = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .expect("probe descriptor");
    assert_eq!(
        module.get_string(function.params[0].type_name),
        Some(expected),
        "parameter carry"
    );
    assert_eq!(
        function
            .return_type_name
            .and_then(|id| module.get_string(id)),
        Some(expected),
        "return carry"
    );
}

#[test]
fn nested_qualified_nominal_keeps_every_path_segment() {
    assert_signature(
        "type Carrier<T> is { value: T }; fn probe(x: Carrier<domain.atomic.Flag>) -> Carrier<domain.atomic.Flag> { x }",
        "Carrier<domain.atomic.Flag>",
    );
}

#[test]
fn qualified_generic_head_and_multiple_nominals_keep_their_identity() {
    assert_signature(
        "fn probe(x: domain.container.Pair<domain.atomic.Flag, domain.other.Token>) -> domain.container.Pair<domain.atomic.Flag, domain.other.Token> { x }",
        "domain.container.Pair<domain.atomic.Flag, domain.other.Token>",
    );
}

#[test]
fn bare_nominal_control_keeps_its_existing_spelling() {
    assert_signature(
        "type Flag is (); type Carrier<T> is { value: T }; fn probe(x: Carrier<Flag>) -> Carrier<Flag> { x }",
        "Carrier<Flag>",
    );
}

#[test]
fn nested_reference_keeps_its_tier_and_qualified_nominal() {
    assert_signature(
        "type Carrier<T> is { value: T }; fn probe(x: Carrier<&unsafe domain.atomic.Flag>) -> Carrier<&unsafe domain.atomic.Flag> { x }",
        "Carrier<&unsafe domain.atomic.Flag>",
    );
}

#[test]
fn associated_projection_keeps_its_qualified_base() {
    assert_signature(
        "type Carrier<T> is { value: T }; fn probe(x: Carrier<domain.iterator.Stream.Item>) -> Carrier<domain.iterator.Stream.Item> { x }",
        "Carrier<::Item<domain.iterator.Stream>>",
    );
}
