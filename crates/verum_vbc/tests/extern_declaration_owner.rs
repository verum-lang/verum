//! Extern declarations retain lexical ownership during standard-library baking.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, FunctionInfo, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    module::FunctionId,
    serialize::serialize_module,
};

fn check(source: &str, expected_ffi: bool) {
    let ast = Parser::new(source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("owner"));
        codegen.set_prefer_existing_functions(bootstrap);
        let foreign = FunctionInfo {
            id: FunctionId(200),
            param_count: 3,
            param_names: vec!["a".into(), "b".into(), "c".into()],
            return_type_name: Some("Int32".into()),
            ..Default::default()
        };
        codegen.import_functions(
            &[
                ("foreign.bridge".into(), foreign.clone()),
                ("bridge#3".into(), foreign),
                (
                    "bridge".into(),
                    FunctionInfo {
                        id: FunctionId(201),
                        param_count: 0,
                        return_type_name: Some("Int32".into()),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
        );
        let module = if bootstrap {
            codegen.collect_unit_declarations(&[&ast]).unwrap();
            codegen
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            codegen.finalize_module_from_state().unwrap()
        } else {
            codegen.compile_module(&ast).unwrap()
        };
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("owner.probe"))
            .unwrap();
        let start = function.bytecode_offset as usize;
        let instructions =
            decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize])
                .unwrap();
        assert_eq!(
            instructions
                .iter()
                .any(|i| matches!(i, Instruction::FfiExtended { sub_op: 0x10, .. })),
            expected_ffi,
            "bootstrap={bootstrap}: {instructions:?}"
        );
        if !expected_ffi {
            let local = module
                .functions
                .iter()
                .find(|f| module.get_string(f.name) == Some("owner.bridge"))
                .unwrap();
            assert!(
                instructions.iter().any(
                    |i| matches!(i, Instruction::Call { func_id, .. } if *func_id == local.id.0)
                ),
                "the ordinary call must select its own declaration: {instructions:?}"
            );
        }
    }
}

#[test]
fn a_local_variadic_extern_beats_an_imported_exact_arity_wrapper() {
    check(
        "module owner; extern { fn bridge(a: Int32, b: Int32, ...) -> Int32; }
        fn probe() -> Int32 { unsafe { bridge(1, 2, 3) } }",
        true,
    );
}

#[test]
fn a_local_fixed_extern_beats_an_imported_same_arity_wrapper() {
    check(
        "module owner; extern { fn bridge(a: Int32, b: Int32, c: Int32) -> Int32; }
        fn probe() -> Int32 { unsafe { bridge(1, 2, 3) } }",
        true,
    );
}

#[test]
fn an_ordinary_declaration_keeps_its_body_despite_a_foreign_extern() {
    check(
        "module foreign { extern { fn bridge(a: Int32, b: Int32, ...) -> Int32; } }
        module owner {
            fn bridge(a: Int32, b: Int32, c: Int32) -> Int32 { a + b + c }
            fn probe() -> Int32 { bridge(1, 2, 3) }
        }",
        false,
    );
}
