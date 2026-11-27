//! Each archive entry owns its native symbol operands, signatures, layouts and library ids.
#![cfg(all(feature = "ffi", any(target_os = "macos", target_os = "linux")))]
use std::sync::Arc;
use verum_vbc::{
    archive::ArchiveBuilder,
    bytecode,
    instruction::{Instruction, Reg, SystemSubOpcode},
    interpreter::Interpreter,
    linker::VbcLinker,
    module::{
        CType, FfiLibrary, FfiPlatform, FfiSignature, FfiSymbol, FunctionDescriptor, VbcModule,
    },
    types::{TypeId, TypeRef},
};

fn native_entry(name: &str, native: &str, operand: u32) -> VbcModule {
    let mut module = VbcModule::new(name.to_owned());
    let (library, platform) = if cfg!(target_os = "macos") {
        ("libSystem.B.dylib", FfiPlatform::Darwin)
    } else {
        ("libc.so.6", FfiPlatform::Linux)
    };
    let lib = module.intern_string(library);
    module.add_ffi_library(FfiLibrary::new(lib, platform));
    let symbol = module.intern_string(native);
    let mut foreign = FfiSymbol::new(symbol, FfiSignature::new(CType::I32, Default::default()));
    foreign.library_idx = 0;
    module.add_ffi_symbol(foreign);
    let mut target = FunctionDescriptor::new(module.intern_string(&format!("{name}.read")));
    target.origin_module = Some(module.intern_string(name));
    target.has_source_body = true;
    target.return_type = TypeRef::Concrete(TypeId::INT);
    target.register_count = 1;
    let mut operands = operand.to_le_bytes().to_vec();
    operands.extend([0, 0, 0]);
    for instruction in [
        Instruction::FfiExtended {
            sub_op: SystemSubOpcode::CallFfiC as u8,
            operands,
        },
        Instruction::Ret { value: Reg(0) },
    ] {
        bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    target.bytecode_length = module.bytecode.len() as u32;
    module.add_function(target);
    module
}

#[test]
fn native_call_after_another_entrys_symbol_zero_executes_the_named_c_function() {
    let mut archive = ArchiveBuilder::new();
    for module in [
        native_entry("fixture.parent", "getppid", 0),
        native_entry("fixture.child", "getpid", 0),
    ] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    let mut linker = VbcLinker::new(std::env::consts::ARCH);
    linker.add_archive(&archive.finish()).unwrap();
    let linked = linker.finalize();
    let bytes = verum_vbc::serialize::serialize_module(&linked).unwrap();
    let linked = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&linked).unwrap();
    let child = linked.find_function_by_name("fixture.child.read").unwrap();
    let parent = linked.find_function_by_name("fixture.parent.read").unwrap();
    let mut vm = Interpreter::new(Arc::new(linked));
    let pid = i64::from(std::process::id());
    assert_eq!(vm.execute_function(child).unwrap().try_as_i64(), Some(pid));
    assert_ne!(vm.execute_function(parent).unwrap().try_as_i64(), Some(pid));
}

#[test]
fn native_operand_must_belong_to_its_own_entry() {
    let mut archive = ArchiveBuilder::new();
    for module in [
        native_entry("fixture.front", "getpid", 0),
        native_entry("fixture.damaged", "getpid", 1),
    ] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    assert!(
        VbcLinker::new(std::env::consts::ARCH)
            .add_archive(&archive.finish())
            .is_err()
    );
}

#[cfg(feature = "codegen")]
#[test]
fn selected_foreign_return_keeps_its_verum_declaration_through_an_unsafe_block() {
    use verum_fast_parser::Parser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
    use verum_vbc::{deserialize::deserialize_module, serialize::serialize_module};

    let native = Parser::new("module native_ids; extern { public fn getpid() -> Int32; }")
        .parse_module()
        .unwrap();
    let local = Parser::new("module local_ids; public fn getpid() -> Int32 { -7_i32 }")
        .parse_module()
        .unwrap();
    let app = Parser::new(
        r#"
        module inspect;
        mount native_ids.getpid as host_id;
        mount local_ids.getpid as local_id;
        type Observation is Measured(Int32) | Missing;
        fn probe() -> Int {
            let first = unsafe { host_id() };
            let local = local_id();
            if local != -7_i32 { return -1; }
            let observation = Observation.Measured(first);
            match observation {
                Observation.Measured(id) => id as Int,
                Observation.Missing => -2,
            }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&native, &local, &app];
    let mut codegen = VbcCodegen::new();
    codegen.initialize();
    codegen.collect_unit_declarations(&units).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module().unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("serialized", decoded)] {
        let probe = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("inspect.probe"))
            .unwrap()
            .id;
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(probe).unwrap().as_i64(),
            i64::from(std::process::id()),
            "{route}"
        );
    }
}
