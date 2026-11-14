//! Execute the actual configuration identifier predicate and Text byte producer.
#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module, decl::ImplItemKind};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::{Interpreter, InterpreterError},
    serialize::serialize_module,
};

fn source(text: &str) -> Module {
    Parser::new(text).parse_module().expect("actual library source grammar")
}

fn execute(input: &str) -> Result<i64, InterpreterError> {
    let mut configuration = source(include_str!("../../../core/configuration/error.vr"));
    configuration.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.name.as_str() == "FormatId",
        ItemKind::Function(decl) => decl.name.name.as_str() == "is_format_id",
        _ => false,
    });
    assert_eq!(configuration.items.len(), 3, "unchanged owner, predicate and validator");
    let mut text = source(include_str!("../../../core/text/text.vr"));
    text.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.name.as_str() == "Text",
        ItemKind::Mount(_) => true,
        ItemKind::Impl(decl) => {
            decl.items.retain(|member| matches!(&member.kind,
                ImplItemKind::Function(function) if function.name.name.as_str() == "as_bytes"));
            !decl.items.is_empty()
        }
        _ => false,
    });
    let mut intrinsic = source(include_str!("../../../core/intrinsics/runtime/text.vr"));
    intrinsic.items.retain(|item| matches!(&item.kind,
        ItemKind::Module(decl) if decl.items.is_none()) || matches!(&item.kind,
        ItemKind::Function(decl) if decl.name.name.as_str() == "text_as_bytes"));
    let consumer = source(&format!(r#"
        module consumer;
        mount core.configuration.error.FormatId;
        fn accept(value: Text) -> FormatId {{ value }}
        fn main() -> Int {{ let identifier = accept({input:?}); 73 }}
    "#));
    let units: List<_> = [&configuration, &text, &intrinsic, &consumer].into_iter().collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    compiler.collect_unit_declarations(&units).expect("all declarations");
    compiler.resolve_pending_imports();
    compiler.compile_unit_items(&units, ItemFailurePolicy::Strict).expect("all bodies");
    let mut module = compiler.finalize_module_from_state().expect("finalize");
    assert!(module.resolve_external_bands().is_empty(), "no missing source body");
    module.synthesize_intrinsic_band_wrappers();
    module.resolve_protocol_dispatch();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).expect("wire contract");
    let entry = module.find_function_by_name("consumer.main").expect("actual consumer entry");
    Interpreter::new(module.into()).execute_function(entry).map(|value|
        value.try_as_i64().expect("consumer Int result"))
}

#[test]
fn actual_format_identifier_accepts_the_declared_ascii_language() {
    for input in ["a", "json", "toml", "a0-_z", "abcdefghijklmnopqrstuvwxyz012345"] {
        assert_eq!(execute(input).unwrap_or_else(|error| panic!("{input:?}: {error:?}")), 73);
    }
}

#[test]
fn actual_format_identifier_refuses_each_invalid_boundary() {
    for input in ["", "0abc", "_abc", "-abc", "Json", "aZ", "a b", "a.b",
        "a/b", "a\n", "a\0", "я", "aé", "abcdefghijklmnopqrstuvwxyz0123456"]
    {
        let error = execute(input).expect_err("invalid identifier must fail its predicate");
        assert!(matches!(&error, InterpreterError::AssertionFailed { message, .. }
            if message.contains("refinement violation")), "{input:?}: {error:?}");
    }
}
