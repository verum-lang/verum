//! Nominal storage positions are local to the declaration, not a name interner.
#![cfg(feature = "codegen")]

use verum_ast::Module;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().unwrap()
}

fn wire(module: &VbcModule) -> VbcModule {
    let module = deserialize_module(&serialize_module(module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    module
}

fn noise() -> Module {
    parse("module elsewhere; type Decoy is { payload:Int, unrelated:Int };")
}

fn offsets(module: &VbcModule) -> List<u32> {
    module.types.iter().find(|ty|
        module.get_string(ty.name) == Some("Datum")
            && ty.origin_module.and_then(|id| module.get_string(id)) == Some("app"))
        .unwrap().fields.iter().map(|field| field.offset).collect()
}

#[test]
fn record_and_tuple_offsets_are_local_declaration_positions() {
    for declaration in [
        "public type Datum is { public left:Int, public right:Int };",
        "public type Datum is (Int, Int);",
        "public type Datum<T> is { public left:T, public right:Int };",
    ] {
        let source = parse(&format!("module app; {declaration}"));
        let noise = noise();
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
        let module = producer.collect_declaration_interface(&[&noise, &source]).unwrap();
        assert_eq!(offsets(&module).as_slice(), [0, 8], "{declaration}");
        assert_eq!(offsets(&wire(&module)).as_slice(), [0, 8]);
    }
}

#[test]
fn a_transparent_tuple_has_one_zero_offset_slot() {
    let source = parse("module app; public type Datum is (Int);");
    let noise = noise();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
    let module = producer.collect_declaration_interface(&[&noise, &source]).unwrap();
    assert_eq!(offsets(&wire(&module)).as_slice(), [0]);
}

#[test]
fn imported_headers_complete_after_unrelated_field_names_are_registered() {
    for declaration in [
        "public type Datum is { public left:Int, public right:Int };",
        "public type Datum is (Int, Int);",
        "public type Datum is (Int);",
    ] {
        let source = parse(&format!("module app; {declaration}"));
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
        let archived = wire(&producer.collect_declaration_interface(&[&source]).unwrap());
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
        consumer.collect_unit_declarations(&[&noise()]).unwrap();
        consumer.import_archive_module_types(&archived);
        let completed = consumer.collect_declaration_interface(&[&source]).unwrap();
        assert_eq!(offsets(&wire(&completed)), offsets(&archived), "{declaration}");
    }
}

#[test]
fn a_conflicting_archive_slot_is_still_rejected() {
    let source = parse("module app; public type Datum is { public left:Int, public right:Int };");
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
    let mut archived = wire(&producer.collect_declaration_interface(&[&source]).unwrap());
    archived.types.iter_mut().find(|ty| ty.fields.len() == 2).unwrap().fields[0].offset = 16;
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    consumer.import_archive_module_types(&archived);
    let error = consumer.collect_declaration_interface(&[&source]).unwrap_err();
    assert!(matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)), "{error}");
    assert!(error.to_string().contains("ABI mismatch"), "{error}");
}

#[test]
fn actual_record_field_access_uses_its_declared_slots_through_wire() {
    let source = parse(r#"
        module app;
        type Datum is { left:Int, right:Int };
        fn main()->Int {
            let value=Datum{left:40,right:33};
            value.left+value.right
        }
    "#);
    let noise = noise();
    let units = [&noise, &source];
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
    producer.collect_unit_declarations(&units).unwrap();
    producer.compile_unit_items(&units, ItemFailurePolicy::Strict).unwrap();
    let module = wire(&producer.finalize_module_from_state().unwrap());
    let entry = module.find_function_by_name("app.main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}
