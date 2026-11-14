//! Packed local allocations retain their source array declaration beside a real List.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(body: &str) -> Result<VbcModule, verum_vbc::codegen::CodegenError> {
    let mut list = FastParser::new()
        .parse_module_str(
            include_str!("../../../core/collections/list.vr"),
            FileId::new(1),
        )
        .unwrap();
    list.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "List")
    });
    assert_eq!(
        list.items
            .iter()
            .filter(|item| matches!(&item.kind, ItemKind::Type(_)))
            .count(),
        1
    );
    let source = FastParser::new()
        .parse_module_str(&format!("module app; {body}"), FileId::new(2))
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app").with_validation());
    let units = [&list, &source];
    codegen.collect_unit_declarations(&units)?;
    codegen.compile_unit_items(&units, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}
fn execute(body: &str) {
    let module = compile(body).expect("declared fixed array must not become a List");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("app.main").unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}
#[test]
fn annotated_byte_array_repeats_keep_the_selected_array() {
    execute("fn main()->Int { let data:[Byte;4]=[73_u8;4]; let part=&data[..]; part[3] as Int }");
}
#[test]
fn annotated_byte_array_literals_keep_the_selected_array() {
    execute(
        "fn main()->Int { let data:[Byte;4]=[1_u8,2_u8,3_u8,73_u8]; let part=&data[..]; part[3] as Int }",
    );
}
#[test]
fn nonconstant_byte_array_repeats_keep_the_selected_array() {
    execute(
        "fn byte()->Byte {73_u8} fn main()->Int {let value=byte(); let data:[Byte;4]=[value;4]; let part=&data[..]; part[3] as Int}",
    );
}
#[test]
fn annotated_wide_array_repeats_keep_the_selected_array() {
    execute("fn main()->Int {let data:[Int;4]=[73;4]; let part=&data[..]; part[3]}");
}
#[test]
fn annotated_wide_array_literals_keep_the_selected_array() {
    execute("fn main()->Int {let data:[Int;4]=[1,2,3,73]; let part=&data[..]; part[3]}");
}
#[test]
fn shadowed_array_bindings_retain_their_own_element_types() {
    execute(
        "fn main()->Int {let data:[Int;2]=[31,42]; {let data:[Byte;1]=[5_u8]; assert((&data[..])[0]==5_u8);} let part=&data[..]; part[0]+part[1]}",
    );
}
#[test]
fn a_foreign_same_leaf_record_does_not_gain_array_projection() {
    let error = compile(
        "type List<T> is { value: T }; fn probe(data: &List<Int>) -> Int { data[0] } fn main() -> Int { 73 }",
    )
    .expect_err("an unrelated List declaration is not indexed storage");
    assert!(
        error.to_string().contains("no declared read projection"),
        "{error}"
    );
}
