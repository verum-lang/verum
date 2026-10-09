//! Source body presence is declaration evidence, independent of emitted RetV.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, FunctionInfo, VbcCodegen},
    deserialize::deserialize_module,
    module::{FunctionDescriptor, VbcModule},
    serialize::serialize_module,
};

fn compile(source: &str) -> (VbcCodegen, VbcModule) {
    let ast = Parser::new(source).parse_module().expect("body syntax");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("body_presence"));
    let module = codegen.compile_module(&ast).expect("body compilation");
    (codegen, module)
}

fn descriptor<'a>(module: &'a VbcModule, name: &str) -> &'a FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some(name))
        .unwrap_or_else(|| panic!("missing exact declaration {name}"))
}

#[test]
fn source_declarations_distinguish_empty_expression_and_forward_bodies() {
    let (codegen, module) = compile(
        "fn forward() -> Unit; fn empty() -> Unit {} fn value() -> Int = 7; \
         implement USize { fn forward(self) -> Unit; fn empty(self) -> Unit {} }",
    );
    let functions = codegen.export_functions();
    for (name, expected) in [
        ("body_presence.forward", false),
        ("body_presence.empty", true),
        ("body_presence.value", true),
        ("USize.forward", false),
        ("USize.empty", true),
    ] {
        let function = descriptor(&module, name);
        assert!(function.bytecode_length > 0, "{name} contains emitted code");
        assert_eq!(function.has_source_body, expected, "{name}");
        let mut registrations = 0;
        for info in functions.values().filter(|info| info.id == function.id) {
            assert_eq!(info.has_source_body, expected, "registration {name}");
            registrations += 1;
        }
        assert!(registrations > 0, "registered source declaration {name}");
    }
}

#[test]
fn source_body_presence_survives_wire_without_conflating_forward_stubs() {
    let (_, source) = compile("fn forward() -> Unit; fn empty() -> Unit {}");
    let module = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    assert!(module.header.version_minor >= 24);
    assert!(!descriptor(&module, "body_presence.forward").has_source_body);
    assert!(descriptor(&module, "body_presence.empty").has_source_body);
}

#[test]
fn legacy_reserved_flag_cannot_authorize_a_source_body() {
    let (_, source) = compile("fn empty() -> Unit {}");
    let mut bytes = serialize_module(&source).unwrap();
    // v2.24 uses an existing flag byte and adds no tail. Keep its bit set to
    // prove that a v2.23 descriptor's reserved bit has no source authority.
    bytes[6..8].copy_from_slice(&23_u16.to_le_bytes());
    let module = deserialize_module(&bytes).expect("v2.23 descriptor remains readable");
    let function = descriptor(&module, "body_presence.empty");
    assert!(function.bytecode_length > 0);
    assert!(!function.has_source_body);
}

#[test]
fn unknown_and_synthetic_defaults_have_no_body_provenance() {
    assert!(!FunctionInfo::default().has_source_body);
    let function = FunctionDescriptor {
        bytecode_length: 1,
        ..FunctionDescriptor::default()
    };
    assert!(!function.has_source_body, "bytecode length is not AST evidence");
}
