//! The original owning branch must remain independent of a copied branch.
#![cfg(target_os = "macos")]
use std::process::Command;
use verum_common::Text;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel, context::Context,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};
use verum_vbc::{codegen::VbcCodegen, deserialize::deserialize_module, serialize::serialize_module};

const SOURCE: &str = r#"
    type Leaf is { value: Int };
    type Branch is { leaf: Leaf };
    fn main() -> Int {
        let mut first = Branch { leaf: Leaf { value: 73 } };
        let second = first;
        first.leaf.value = 0;
        second.leaf.value
    }
"#;

fn module(source: &str) -> verum_vbc::VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let original = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let wire = deserialize_module(&serialize_module(&original).expect("serialize")).expect("reload");
    verum_vbc::validate::validate_module(&wire).expect("valid declared module");
    wire
}

#[test]
fn nested_value_copy_preserves_the_original_owned_branch_in_vm() {
    let wire = module(SOURCE);
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(wire.into()).expect("validated interpreter entry");
    let value = vm.execute_function(entry).expect("VM execution");
    assert_eq!(value.try_as_i64(), Some(73));
}

#[test]
fn nested_value_copy_preserves_the_original_owned_branch_in_native_image() {
    run_native(SOURCE);
}

#[test]
fn a_native_copy_through_a_borrow_owns_its_nested_child() {
    run_native(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn main() -> Int {
            let mut original = Branch { leaf: Leaf { value: 73 } };
            let copied = *(&original);
            original.leaf.value = 0;
            copied.leaf.value
        }
    "#);
}

#[test]
fn a_native_array_copy_owns_its_nested_element() {
    run_native(r#"
        type Leaf is { value: Int };
        fn main() -> Int {
            let mut original = [Leaf { value: 73 }];
            let copied = original;
            original[0].value = 0;
            copied[0].value
        }
    "#);
}

#[test]
fn a_native_mutable_parameter_owns_its_nested_value() {
    run_native(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn alter(mut value: Branch) { value.leaf.value = 0; }
        fn main() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            alter(original);
            original.leaf.value
        }
    "#);
}

fn run_native(source: &str) {
    let mut wire = module(source);
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &mut wire.functions {
        function.is_generic = !function.type_params.is_empty();
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        ).expect("loaded body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for function in &wire.functions {
        verum_vbc::mono::discover_call_instantiations(
            &wire, function.instructions.as_ref().unwrap(), function.func_id_base, &mut graph,
        ).expect("declared generic witnesses");
    }
    let wire = verum_vbc::mono::monomorphize_minimal(wire, &graph).expect("specialize").module;
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple).unwrap().create_target_machine(
        &triple, "generic", "", OptimizationLevel::Default, RelocMode::PIC, CodeModel::Default,
    ).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("nested_value_copy").with_debug_info(false),
    );
    lowering.lower_module(&wire).expect("native lowering");
    let native = lowering.module();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().expect("complete native IR");
    let directory = tempfile::tempdir().unwrap();
    let object = directory.path().join("copy.o");
    let executable = directory.path().join("copy");
    machine.write_to_file(native, FileType::Object, &object).expect("object");
    let link = Command::new("cc").arg(object)
        .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"]).arg(&executable)
        .output().expect("system linker");
    assert!(link.status.success(), "{}", Text::from_utf8_lossy(&link.stderr));
    let run = Command::new(executable).output().expect("native execution");
    assert_eq!(run.status.code(), Some(73), "{}", Text::from_utf8_lossy(&run.stderr));
}
