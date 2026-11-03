//! The HTTP connection passes the stream and handler witnesses to each message reader.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::Instruction;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::types::TypeRef;

#[test]
fn http_connection_forwards_both_witnesses_to_its_reader() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    let path = root.join("net/weft/connection.vr");
    let source = std::fs::read_to_string(&path).unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(
        CodegenConfig::new("core.net.weft.connection").with_strict_codegen(),
    )
    .compile_module_with_mounts(&ast, path.to_str().unwrap(), root.to_str().unwrap())
    .unwrap();
    let caller = module
        .find_function_by_name("core.net.weft.connection.serve_http1")
        .unwrap();
    let callee = module
        .find_function_by_name("core.net.weft.connection.serve_one_message")
        .unwrap();
    let caller = module.get_function(caller).unwrap();
    let expected: Vec<_> = caller
        .type_params
        .iter()
        .map(|p| TypeRef::Generic(p.id))
        .collect();
    assert_eq!(expected.len(), 2);
    let start = caller.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + caller.bytecode_length as usize])
            .unwrap();
    let calls: Vec<_> = instructions
        .iter()
        .filter(|op| match op {
            Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } => {
                module.resolved_function_id(*func_id) == callee
            }
            _ => false,
        })
        .collect();
    assert!(
        matches!(calls.as_slice(), [Instruction::CallG { type_args, .. }] if type_args.as_slice() == expected),
        "serve_http1 → serve_one_message: {calls:?}"
    );
}
