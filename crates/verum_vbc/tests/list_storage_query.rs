//! The carrier owns storage width; a generic element layout is not its stride.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, module::VbcModule, types::TypeId};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::new().compile_module(&ast).expect("source VBC")
}
fn execute(module: VbcModule) -> Result<i64, verum_vbc::interpreter::InterpreterError> {
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .unwrap()
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .map(|v| v.as_i64())
}
const QUERY: &str = r#"
    fn stride<T>(list: &List<T>) -> Int { @intrinsic("list_storage_stride", list) }
"#;
#[test]
fn value_slots_and_packed_bytes_keep_distinct_encodings() {
    for (element, expected) in [("Int", 8), ("Byte", 1)] {
        let source = format!(
            "{QUERY} fn probe()->Int {{ let values:List<{element}> = List<{element}>.with_capacity(2); stride(&values) }}"
        );
        assert_eq!(execute(compile(&source)).unwrap(), expected);
    }
}
#[test]
fn record_storage_is_not_inline_record_layout() {
    let source = format!(
        r#"{QUERY} type Record is {{ first:Int, second:Int, third:Int }}; fn probe()->Int {{ let values:List<Record> = List<Record>.new(); stride(&values) }}"#
    );
    assert_eq!(execute(compile(&source)).unwrap(), 8);
}
#[test]
fn foreign_record_cannot_supply_a_storage_layout() {
    let module = compile(
        r#"type Foreign is { len:Int, cap:Int, ptr:Int }; fn probe()->Int { let value=Foreign {len:0,cap:0,ptr:0}; @intrinsic("list_storage_stride", value) }"#,
    );
    let failure = execute(module).unwrap_err().to_string();
    assert!(failure.contains("unknown List storage type"), "{failure}");
}
#[test]
fn query_roundtrip_preserves_the_following_call_boundary() {
    let source = format!(
        "{QUERY} fn increment(x:Int)->Int{{x+1}} fn probe()->Int{{let values:List<Int> =List<Int>.new(); increment(stride(&values))}}"
    );
    let module = compile(&source);
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    assert_eq!(decoded.header.version_minor, verum_vbc::format::VERSION_MINOR);
    assert!(decoded.header.version_minor >= 19, "storage query requires v2.19");
    assert_eq!(execute(decoded).unwrap(), 9);
}
#[test]
fn storage_identity_domain_does_not_include_unknown_or_scalar_types() {
    assert_eq!(TypeId::LIST.list_storage_stride(), Some(8));
    assert_eq!(TypeId::BYTE_LIST.list_storage_stride(), Some(1));
    for other in [
        TypeId::U8,
        TypeId::INT,
        TypeId::UNIT,
        TypeId::ARRAY,
        TypeId(99999),
    ] {
        assert_eq!(other.list_storage_stride(), None);
    }
}

#[test]
fn unsupported_memory_wire_tag_is_refused() {
    let source = format!(
        "{QUERY} fn probe()->Int{{let values:List<Int> =List<Int>.new(); stride(&values)}}"
    );
    let mut module = compile(&source);
    let function = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("stride"))
        .unwrap();
    let offset = function.bytecode_offset as usize;
    let body = &module.bytecode[offset..offset + function.bytecode_length as usize];
    let instructions = verum_vbc::bytecode::decode_instructions(body).unwrap();
    let index = instructions
        .iter()
        .position(|i| {
            matches!(
                i,
                verum_vbc::instruction::Instruction::MemExtended { sub_op: 0x07, .. }
            )
        })
        .unwrap();
    let mut prefix = Vec::new();
    verum_vbc::bytecode::encode_instructions(&instructions[..index], &mut prefix);
    module.bytecode[offset + prefix.len() + 1] = 0xff;
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    let failure = execute(decoded).unwrap_err().to_string();
    assert!(failure.contains("mem_extended sub-opcode"), "{failure}");
}
