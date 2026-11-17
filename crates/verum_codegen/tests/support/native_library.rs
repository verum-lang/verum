//! Link and execute the complete generated native module without runtime mocks.
use std::process::Command;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Text};
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};

pub fn with_library(
    module: &verum_vbc::VbcModule,
    exports: &[(&str, &str)],
    check: impl FnOnce(&libloading::Library),
) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("native_source_library").with_debug_info(false),
    );
    lowering.lower_module(module).expect("lower source and runtime");
    let native = lowering.module();
    for &(source, exported) in exports {
        let selected = native.get_function(source).expect("exact generated entry");
        let entry = native.add_function(exported, selected.get_type(), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(entry, "entry"));
        let arguments: List<_> = entry.get_param_iter().map(Into::into).collect();
        let call = builder.build_call(selected, &arguments, "result").unwrap();
        call.set_call_convention(selected.get_call_conventions());
        if let Some(value) = call.try_as_basic_value().basic() {
            builder.build_return(Some(&value)).unwrap();
        } else {
            builder.build_return(None).unwrap();
        }
    }
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple).unwrap().create_target_machine(
        &triple, "generic", "", OptimizationLevel::Default, RelocMode::PIC, CodeModel::Default,
    ).unwrap();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().expect("complete generated module");
    if let Some(path) = std::env::var_os("TYPED_IR_DUMP") {
        native.print_to_file(path).unwrap();
    }
    let directory = tempfile::Builder::new().prefix("native-source-library-").tempdir_in(
        std::env::var_os("CARGO_TARGET_DIR").expect("session-private build directory"),
    ).unwrap();
    let object = directory.path().join("source.o");
    let image = directory.path().join("source.library");
    machine.write_to_file(native, FileType::Object, &object).unwrap();
    let mut linker = Command::new("cc");
    linker.arg(&object);
    if cfg!(target_os = "macos") {
        linker.args(["-dynamiclib", "-nostdlib", "-lSystem", "-Wl,-dead_strip"]);
        for &(_, exported) in exports {
            linker.arg(format!("-Wl,-exported_symbol,_{exported}"));
        }
    } else {
        linker.args(["-shared", "-nostdlib"]);
    }
    let output = linker.arg("-o").arg(&image).output().unwrap();
    assert!(output.status.success(), "{}", Text::from_utf8_lossy(&output.stderr));
    // SAFETY: the harness loads only the exact image generated and linked above.
    // The image remains loaded for every invocation made by the typed callback.
    let library = unsafe { libloading::Library::new(&image) }.unwrap();
    check(&library);
}
