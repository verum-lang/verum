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

fn check(source: &str, caller_owner: &str, expected_ffi: bool) {
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
            .find(|f| module.get_string(f.name) == Some(format!("{caller_owner}.probe").as_str()))
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
                .find(|f| {
                    module.get_string(f.name) == Some(format!("{caller_owner}.bridge").as_str())
                })
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
        "owner",
        true,
    );
}

#[test]
fn a_local_fixed_extern_beats_an_imported_same_arity_wrapper() {
    check(
        "module owner; extern { fn bridge(a: Int32, b: Int32, c: Int32) -> Int32; }
        fn probe() -> Int32 { unsafe { bridge(1, 2, 3) } }",
        "owner",
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
        "owner.owner",
        false,
    );
}

fn mounted_foreign(
    member: &str,
    decoy: bool,
) -> Result<verum_vbc::module::VbcModule, verum_vbc::codegen::CodegenError> {
    let (library, native_symbol) = if cfg!(target_os = "windows") {
        ("kernel32.dll", "GetCurrentProcessId")
    } else if cfg!(target_os = "macos") {
        ("libSystem.B.dylib", "getpid")
    } else {
        ("libc.so.6", "getpid")
    };
    let provider = Parser::new(&format!(
        r#"
        module core.provider;
        @ffi("{library}") extern {{
            @link_name("{native_symbol}") public fn process_number() -> Int32;
            @link_name("{native_symbol}") fn private_number() -> Int32;
        }}
    "#
    ))
    .parse_module()
    .unwrap();
    let caller = Parser::new(&format!(
        r#"
        module core.consumer;
        mount core.provider.{member} as selected;
        public fn probe() -> Int {{ unsafe {{ selected() as Int }} }}
    "#
    ))
    .parse_module()
    .unwrap();
    let sibling = Parser::new("module core.decoy; public fn process_number() -> Int { 71 }")
        .parse_module()
        .unwrap();
    let mut files = vec![&provider, &caller];
    if decoy {
        files.push(&sibling);
    }
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core"));
    codegen.collect_unit_declarations(&files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn qualified_extern_mounts_preserve_visibility_and_foreign_linkage() {
    let source = mounted_foreign("process_number", true).unwrap();
    let wire = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    for artifact in [source, wire] {
        verum_vbc::validate::validate_module(&artifact).unwrap();
        let entry = artifact
            .find_function_by_name("core.consumer.probe")
            .unwrap();
        let function = artifact.get_function(entry).unwrap();
        let start = function.bytecode_offset as usize;
        let ops = decode_instructions(
            &artifact.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        assert!(
            ops.iter()
                .any(|op| matches!(op, Instruction::FfiExtended { sub_op: 0x10, .. })),
            "{ops:?}"
        );
        #[cfg(all(
            feature = "ffi",
            any(
                target_os = "macos",
                target_os = "windows",
                all(target_os = "linux", target_env = "gnu")
            )
        ))]
        {
            let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(artifact));
            let observed = vm.execute_function(entry).unwrap();
            assert_eq!(observed.try_as_i64(), Some(i64::from(std::process::id())));
        }
    }
    let denied = mounted_foreign("private_number", false)
        .unwrap_err()
        .to_string();
    assert!(
        denied.contains("core.provider.private_number") && denied.contains("inaccessible"),
        "{denied}"
    );
}
