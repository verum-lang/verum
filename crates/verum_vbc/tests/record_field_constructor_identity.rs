#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str) -> i64 {
    let ast = Parser::new(source).parse_module().expect("parse");
    run_module(&ast)
}

fn run_module(ast: &verum_ast::Module) -> i64 {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("codec"));
    let module = codegen
        .compile_module(ast)
        .expect("declared field owner resolves");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute")
        .as_i64()
}

#[test]
fn record_field_owns_bare_unit_constructor_in_a_named_module() {
    for types in [
        "type ErrorKind is InvalidByte | InvalidLength; type Other is InvalidLength | Valid;",
        "type Other is InvalidLength | Valid; type ErrorKind is InvalidByte | InvalidLength;",
    ] {
        assert_eq!(
            run(&format!(
                r#"
module codec;
{types}
type Error is {{ kind: ErrorKind }};
fn make() -> Result<Int, Error> {{ Result.Err(Error {{ kind: InvalidLength }}) }}
fn probe() -> Int {{
    match make() {{
        Result.Err(error) => match error.kind {{ ErrorKind.InvalidLength => 42, _ => -1 }},
        _ => -2,
    }}
}}
"#
            )),
            42
        );
    }
}

#[test]
fn record_field_owns_bare_tuple_constructor_in_a_named_module() {
    assert_eq!(
        run(r#"
module codec;
type ErrorKind is InvalidByte | InvalidLength(Int);
type Other is InvalidLength(Int) | Valid;
type Error is { kind: ErrorKind };
fn probe() -> Int {
    let error = Error { kind: InvalidLength(42) };
    match error.kind { ErrorKind.InvalidLength(value) => value, _ => -1 }
}
"#),
        42
    );
}

#[test]
fn qualified_record_field_does_not_select_another_modules_same_leaf_sum() {
    assert_eq!(
        run(r#"
module codec;
module alpha {
    public type Kind is Padding | InvalidLength;
    public type Error is { public kind: Kind };
    public fn make() -> Error { Error { kind: InvalidLength } }
}
module beta {
    public type Kind is InvalidLength | Padding;
    public type Error is { public kind: Kind };
    public fn make() -> Error { Error { kind: InvalidLength } }
}
fn probe() -> Int {
    let a = alpha.make();
    let b = beta.make();
    let first = match a.kind { alpha.Kind.InvalidLength => 40, _ => -100 };
    first + match b.kind { beta.Kind.InvalidLength => 2, _ => -100 }
}
"#),
        42
    );
}

#[test]
fn real_base64_decoder_survives_unrelated_error_constructors() {
    let source = format!(
        r#"{}
// Import registries can contain unrelated errors with these variant names.
type OtherEncodingError is InvalidLength | InvalidPadding | InvalidByte;
fn probe() -> Int {{
    let bytes = decode(&"c2FsdA==").unwrap();
    assert_eq(bytes.len(), 4);
    assert_eq(bytes[0], 115);
    assert_eq(bytes[1], 97);
    assert_eq(bytes[2], 108);
    assert_eq(bytes[3], 116);
    let error = match decode(&"a") {{ Result.Err(error) => error, _ => panic("accepted invalid length") }};
    match error.kind {{ Base64ErrorKind.InvalidLength => 42, _ => -1 }}
}}
"#,
        include_str!("../../../core/encoding/base64.vr")
    );
    let mut ast = Parser::new(&source)
        .parse_module()
        .expect("real decoder parses");
    // Encoder allocation APIs are outside this decoder's dependency closure.
    ast.items.retain(|item| {
        !matches!(&item.kind,
            verum_ast::decl::ItemKind::Function(function)
                if function.name.name.starts_with("encode")
        )
    });
    assert_eq!(run_module(&ast), 42);
}
