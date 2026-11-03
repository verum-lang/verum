//! A module's extern retains its native ABI through public mounts and archive linking.
use std::fs;
use verum_compiler::archive_ctx_loader::ArchiveCtxCache;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
use verum_vbc::{
    Instruction, archive::read_archive_from_file, bytecode::decode_instructions, module::VbcModule,
};

fn instructions(module: &VbcModule, name: &str) -> Vec<Instruction> {
    let id = module.find_function_by_name(name).expect(name);
    let function = module.get_function(id).unwrap();
    let start = function.bytecode_offset as usize;
    decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize]).unwrap()
}

#[test]
fn public_wrapper_reexport_does_not_capture_the_local_or_explicitly_mounted_extern() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let platform = core.join("platform");
    fs::create_dir_all(&platform).unwrap();
    let input = core.join("mod.vr");
    fs::write(
        &input,
        "module core; public module platform; public module independent;",
    )
    .unwrap();
    fs::write(
        platform.join("mod.vr"),
        "module core.platform;
        public module native; public module caller; public module other;
        public mount .native.{wrapped as query};
        public fn through_wrapper() -> Int32 { query() }",
    )
    .unwrap();
    fs::write(
        platform.join("native.vr"),
        r#"module core.platform.native;
        @ffi("libprovider") extern { @ffi_name("provider_query") public fn query() -> Int32; public fn relay(n: Int32, ...) -> Int32; }
        public fn wrapped() -> Int32 { unsafe { query() } + 1 }"#,
    )
    .unwrap();
    fs::write(
        platform.join("caller.vr"),
        "module core.platform.caller;
        mount core.platform.native.{query as raw_query};
        public fn direct() -> Int32 { unsafe { raw_query() } }",
    )
    .unwrap();
    fs::write(
        platform.join("other.vr"),
        r#"module core.platform.other;
        @ffi("libother") extern { @ffi_name("provider_query") public fn query(n: Int32) -> Int32; }
        public fn separate() -> Int32 { unsafe { query(4) } }"#,
    )
    .unwrap();

    let independent = core.join("independent");
    fs::create_dir_all(&independent).unwrap();
    fs::write(
        independent.join("mod.vr"),
        "module core.independent;
        mount core.platform.native.{query as raw, relay as send};
        public fn cross_bundle() -> Int32 { unsafe { raw() + send(1, 2, 3) } }",
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
        .unwrap();
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core.platform").unwrap();
    check_bindings(&module);

    let independent = archive.load_module("core.independent").unwrap();
    let cross = instructions(&independent, "core.independent.cross_bundle");
    let calls: Vec<_> = cross
        .iter()
        .filter_map(|op| match op {
            Instruction::FfiExtended {
                sub_op: 0x10,
                operands,
            } => {
                let id = u32::from_le_bytes(operands[..4].try_into().unwrap()) as usize;
                let symbol = &independent.ffi_symbols[id];
                Some((
                    independent.get_string(symbol.name).unwrap(),
                    operands[4],
                    symbol.signature.is_variadic,
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        [("provider_query", 0, false), ("relay", 3, true)],
        "{cross:?}"
    );

    let app = Parser::new(
        "module app; mount core.platform.{through_wrapper};
        mount core.platform.caller.{direct}; mount core.platform.other.{separate};
        fn main() -> Int32 { through_wrapper() + direct() + separate() }",
    )
    .parse_module()
    .unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new().apply_lazy_with_types(&archive, &mut consumer, &app, &[]).expect("valid archive");
    consumer.collect_unit_declarations(&[&app]).unwrap();
    consumer.resolve_pending_imports();
    consumer
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    check_bindings(&consumer.finalize_module_from_state().unwrap());
}

fn check_bindings(module: &VbcModule) {
    for (function, symbol_name, library, arity) in [
        (
            "core.platform.native.wrapped",
            "provider_query",
            "libprovider",
            0,
        ),
        (
            "core.platform.caller.direct",
            "provider_query",
            "libprovider",
            0,
        ),
        (
            "core.platform.other.separate",
            "provider_query",
            "libother",
            1,
        ),
    ] {
        let body = instructions(module, function);
        let ids: Vec<_> = body
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::FfiExtended {
                    sub_op: 0x10,
                    operands,
                } => Some(u32::from_le_bytes(operands[..4].try_into().unwrap()) as usize),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 1, "{function}: {body:?}");
        let symbol = &module.ffi_symbols[ids[0]];
        assert_eq!(module.get_string(symbol.name), Some(symbol_name));
        assert_eq!(
            module.get_string(module.ffi_libraries[symbol.library_idx as usize].name),
            Some(library)
        );
        assert_eq!(symbol.signature.param_types.len(), arity);
    }
    let wrapper = module
        .find_function_by_name("core.platform.native.wrapped")
        .unwrap();
    let body = instructions(module, "core.platform.through_wrapper");
    assert!(
        body.iter()
            .any(|op| matches!(op, Instruction::Call { func_id, .. } if *func_id == wrapper.0)),
        "{body:?}"
    );
}
