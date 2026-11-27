//! Execute serialized source bytecode through the complete native runtime.
use std::process::Command;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, serialize::serialize_module,
};

pub fn module(source: &str) -> verum_vbc::VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let original = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let wire =
        deserialize_module(&serialize_module(&original).expect("serialize")).expect("reload");
    verum_vbc::validate::validate_module(&wire).expect("valid declared module");
    wire
}

pub fn run(source: &str) {
    run_with(source, |_| {});
}

/// Set a runtime resource boundary before emitting the ordinary native image.
pub fn run_with(source: &str, configure: impl FnOnce(&verum_llvm::module::Module<'_>)) {
    run_module(module(source), configure);
}

/// Execute an already admitted module through the same complete native path.
pub fn run_module(
    wire: verum_vbc::VbcModule,
    configure: impl FnOnce(&verum_llvm::module::Module<'_>),
) {
    let run = execute_module(wire, configure);
    assert_eq!(
        run.status.code(),
        Some(73),
        "{}",
        Text::from_utf8_lossy(&run.stderr)
    );
}

/// Return the linked program's actual status and diagnostics for refusal tests.
pub fn execute_module(
    wire: verum_vbc::VbcModule,
    configure: impl FnOnce(&verum_llvm::module::Module<'_>),
) -> std::process::Output {
    execute_with_arguments(wire, configure, &[])
}

pub fn execute_with_arguments(
    mut wire: verum_vbc::VbcModule,
    configure: impl FnOnce(&verum_llvm::module::Module<'_>),
    arguments: &[&std::ffi::OsStr],
) -> std::process::Output {
    for function in &mut wire.functions {
        function.is_generic = !function.type_params.is_empty();
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .expect("loaded body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    let graph = verum_vbc::mono::module_instantiations(&wire).expect("declared generic witnesses");
    let wire = verum_vbc::mono::monomorphize_minimal(wire, &graph)
        .expect("specialize")
        .module;
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple)
        .unwrap()
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("source_image").with_debug_info(false),
    );
    lowering.lower_module(&wire).expect("native lowering");
    let native = lowering.module();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    configure(native);
    native.verify().expect("complete native IR");
    let directory = tempfile::tempdir().unwrap();
    let object = directory.path().join("copy.o");
    let executable = directory.path().join("copy");
    machine
        .write_to_file(native, FileType::Object, &object)
        .expect("object");
    let link = Command::new("cc")
        .arg(object)
        .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
        .arg(&executable)
        .output()
        .expect("system linker");
    assert!(
        link.status.success(),
        "{}",
        Text::from_utf8_lossy(&link.stderr)
    );
    Command::new(executable).args(arguments).output().expect("native execution")
}
