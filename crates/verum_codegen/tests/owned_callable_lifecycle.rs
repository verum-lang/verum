//! Execute exact native producer lifecycles through the ordinary OS runtime.
#![cfg(target_os = "macos")]
use std::process::Command;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{OptimizationLevel, context::Context,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}};
use verum_vbc::{VbcModule, codegen::VbcCodegen,
    deserialize::deserialize_module_validated, serialize::serialize_module};

fn source_module(source: &str, wire: bool) -> VbcModule {
    let module = VbcCodegen::new().compile_module(&Parser::new(source).parse_module().unwrap()).unwrap();
    let mut module = if wire {
        deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap()
    } else { module };
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    module
}

fn run(source: &str, wire: bool, check_counter: bool) {
    let module = source_module(source, wire);
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(&context,
        LoweringConfig::debug("owned_callables").with_debug_info(false));
    lowering.lower_module(&module).unwrap();
    let native = lowering.module();
    // Source-private functions retain their linkage. Publish a separate exact
    // call adapter for the test process, without changing the source body.
    let selected = native.get_function("probe").expect("actual source entry");
    let entry = native.add_function("invoke_probe", selected.get_type(), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(entry, "entry"));
    let call = builder.build_call(selected, &[], "result").unwrap();
    call.set_call_convention(selected.get_call_conventions());
    builder.build_return(Some(&call.try_as_basic_value().basic().unwrap())).unwrap();
    if check_counter {
        let selected = native.get_function("verum_release_owner_u32").expect("owned release emitted");
        let wrapper = native.add_function("release_one", selected.get_type(), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(wrapper, "entry"));
        let result = builder.build_call(selected, &[wrapper.get_first_param().unwrap().into()], "previous")
            .unwrap().try_as_basic_value().basic().unwrap();
        builder.build_return(Some(&result)).unwrap();
    }
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple).unwrap().create_target_machine(&triple, "generic", "",
        OptimizationLevel::Default, RelocMode::PIC, CodeModel::Default).unwrap();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().unwrap();
    let directory = tempfile::Builder::new().prefix("owned-callable-native-")
        .tempdir_in(std::env::var_os("CARGO_TARGET_DIR").expect("session-private external target")).unwrap();
    let object = directory.path().join("callable.o");
    let image = directory.path().join("callable.dylib");
    machine.write_to_file(native, FileType::Object, &object).unwrap();
    let mut linker = Command::new("cc");
    linker.arg(&object).args(["-dynamiclib", "-nostdlib", "-lSystem", "-Wl,-dead_strip", "-Wl,-exported_symbol,_invoke_probe"]);
    if check_counter { linker.arg("-Wl,-exported_symbol,_release_one"); }
    let output = linker.arg("-o").arg(&image).output().unwrap();
    assert!(output.status.success(), "{}", verum_common::Text::from_utf8_lossy(&output.stderr));
    // SAFETY: this exact generated image owns the declared exported signatures;
    // it remains loaded through execution. The harness replaces no runtime operation.
    unsafe {
        let library = libloading::Library::new(&image).unwrap();
        let probe = library.get::<unsafe extern "C" fn() -> i64>(b"invoke_probe").unwrap();
        assert_eq!(probe(), 73, "wire={wire}");
        if check_counter {
            let release = library.get::<unsafe extern "C" fn(*mut u32) -> u32>(b"release_one").unwrap();
            let mut count = 3;
            for expected in [3, 2, 1, 0] {
                assert_eq!(release(&mut count), expected);
                assert_eq!(count, expected.saturating_sub(1));
            }
        }
    }
}

#[test]
fn native_owned_record_captures_and_shared_copies_survive_source_and_wire() {
    let source = "type Payload is {value:Int}; fn probe()->Int {let value=Payload{value:73}; let callback=||value.value; let copied=callback; copied()}";
    for wire in [false, true] { run(source, wire, true); }
}

#[test]
fn native_capture_telescope_copies_multiple_roots_in_one_transaction() {
    let source = "type Payload is {value:Int}; fn probe()->Int {let first=Payload{value:36}; let second=Payload{value:37}; let callback=||first.value+second.value; let copied=callback; copied()}";
    for wire in [false, true] { run(source, wire, false); }
}

#[test]
fn native_empty_owned_environment_releases_its_final_owner() {
    for wire in [false, true] { run("fn probe()->Int {let callback=||73; let copied=callback; copied()}", wire, true); }
}
