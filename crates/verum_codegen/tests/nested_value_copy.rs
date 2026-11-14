//! The original owning branch must remain independent of a copied branch.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/canonical_drop.rs"]
mod canonical_drop;
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
    run_native_module(module(source));
}

fn run_native_module(mut wire: verum_vbc::VbcModule) {
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

#[test]
fn selected_structural_clone_executes_the_declared_native_copy() {
    for source in [r#"
        type Leaf is { value: Int };
        fn duplicate(source: &Leaf)->Leaf { source.clone() }
        fn main()->Int {
            let original = Leaf { value: 0 };
            let mut copied = duplicate(&original);
            copied.value = 73;
            original.value + copied.value
        }
    "#, r#"
        type Leaf is { value: Int };
        type Wrap<T> is { item:T };
        fn duplicate(source: &Wrap<Leaf>)->Wrap<Leaf> { source.clone() }
        fn main()->Int {
            let original = Wrap { item:Leaf { value: 0 } };
            let mut copied = duplicate(&original);
            copied.item.value = 73;
            original.item.value + copied.item.value
        }
    "#] {
        let mut ast = Parser::new(source).parse_module().unwrap();
        let mut checker = verum_types::TypeChecker::with_minimal_context();
        checker.register_builtins();
        let mut errors = checker.prepare_module_declarations(&ast);
        errors.extend(ast.items.iter().filter_map(|item| checker.check_item(item).err()));
        errors.extend(checker.take_deferred_errors());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(checker.diagnostic_sources().is_empty());
        checker.commit_resolved_call_targets(&mut ast);
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        verum_vbc::validate::validate_module(&wire).unwrap();
        run_native_module(wire);
    }
}

#[test]
fn inferred_generic_record_with_sized_fields_executes_in_native_image() {
    run_native(r#"
        type Inner<T> is { strong_count: UInt32, weak_count: UInt32, value: T };
        type Outer<T> is { inner: Inner<T> };
        implement<T> Outer<T> {
            fn new(value: T) -> Outer<T> {
                let inner = Inner { strong_count: 1, weak_count: 0, value: value };
                Outer { inner: inner }
            }
        }
        fn main() -> Int {
            let result = Outer.new(73);
            if result.inner.strong_count != 1 || result.inner.weak_count != 0 { return 0; }
            result.inner.value
        }
    "#);
}

#[test]
fn inferred_record_keeps_caller_and_owner_binders_separate_in_native_image() {
    run_native(r#"
        type Inner<Right, Left> is { fixed: UInt16, left: Left, right: Right };
        type Outer<A, B> is { inner: Inner<B, A> };
        fn build<A, B>(a: A, b: B) -> Outer<A, B> {
            let inner = Inner { right: b, fixed: 65535, left: a };
            Outer { inner }
        }
        fn main() -> Int {
            let result = build(73, true);
            if result.inner.fixed != 65535 || !result.inner.right { return 0; }
            result.inner.left
        }
    "#);
}

#[test]
fn selected_imported_function_executes_the_same_native_declaration() {
    for (mount, name) in [
        ("mount main.provider.operation;", "operation"),
        ("mount main.provider.operation as selected;", "selected"),
    ] {
        for body in [format!("{name}()"), format!("let callback:fn()->Int={name}; callback()")] {
            let source = format!(
                "module main; public module provider {{ public fn operation()->Int {{73}} }} {mount} async fn {name}()->Int {{0}} fn main()->Int {{{body}}}"
            );
            let mut ast = Parser::new(&source).parse_module().unwrap();
            let mut checker = verum_types::TypeChecker::with_minimal_context();
            checker.register_builtins();
            checker.set_current_module_path("main");
            let mut errors = checker.prepare_module_declarations(&ast);
            errors.extend(ast.items.iter().filter_map(|item| checker.check_item(item).err()));
            errors.extend(checker.take_deferred_errors());
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert!(checker.diagnostic_sources().is_empty(), "{:?}", checker.diagnostic_sources());
            checker.commit_resolved_call_targets(&mut ast);
            assert!(!checker.take_resolved_call_targets().is_empty(), "selected import needs a checked target: {source}");
            let module = VbcCodegen::new().compile_module(&ast).unwrap();
            let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
            verum_vbc::validate::validate_module(&wire).unwrap();
            let entry = wire.find_function_by_name("main").unwrap();
            let value = verum_vbc::interpreter::Interpreter::try_new_validated(wire.clone().into()).unwrap()
                .execute_function(entry).unwrap();
            assert_eq!(value.try_as_i64(), Some(73), "selected source must agree before native lowering");
            run_native_module(wire);
        }
    }
}

#[test]
fn a_native_transparent_scalar_copy_keeps_the_inner_carrier() {
    run_native(
        "type Number is (Int); fn main()->Int {let first=Number(73);let second=first;second.0}",
    );
}

#[test]
fn a_native_generic_newtype_copy_owns_its_nested_payload() {
    run_native(
        r#"
        type Leaf is {value:Int}; type Branch is {leaf:Leaf}; type Wrapped<T> is (T);
        fn main()->Int {
            let mut first:Wrapped<Branch> = Wrapped(Branch{leaf:Leaf{value:73}});
            let second=first;first.0.leaf.value=0;second.0.leaf.value
        }
    "#,
    );
}

#[test]
fn a_native_borrowed_newtype_copy_owns_its_nested_payload() {
    run_native(
        r#"
        type Leaf is {value:Int}; type Branch is {leaf:Leaf}; type Wrapped is (Branch);
        fn main()->Int {
            let mut first=Wrapped(Branch{leaf:Leaf{value:73}});
            let second=*(&first);first.0.leaf.value=0;second.0.leaf.value
        }
    "#,
    );
}

#[test]
fn native_typed_destruction_of_a_generic_newtype_reaches_its_actual_payload() {
    // Only the explicit generic disposal operation is a bytecode fixture.
    // Constructors, witnesses and the counter-changing Drop body are source.
    let source = r#"
        module transparent_drop;
        mount core.base.protocols.Drop;
        type Guard is {count:&mut Int};
        implement Drop for Guard {fn drop(&mut self) {*self.count=*self.count+1;}}
        type Wrapped<T> is (T);
        fn dispose<T>(value:T) {}
        fn main()->Int {
            let mut count=0;
            dispose<Wrapped<Guard>>(Wrapped(Guard{count:&mut count}));
            if count==1 {73}else{0}
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut generator =
        VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("transparent_drop"));
    canonical_drop::initialize(&mut generator);
    let mut module = generator.compile_additional_module(&ast).unwrap();
    let dispose = module
        .find_function_by_name("transparent_drop.dispose")
        .unwrap();
    let declared = module
        .get_function(dispose)
        .unwrap()
        .semantic_parameter_types()
        .unwrap()[0]
        .clone()
        .unwrap();
    assert!(declared.is_generic());
    let start = module.bytecode.len() as u32;
    verum_vbc::bytecode::encode_instruction(
        &verum_vbc::Instruction::DropTyped {
            src: verum_vbc::instruction::Reg(0),
            type_ref: declared,
        },
        &mut module.bytecode,
    );
    verum_vbc::bytecode::encode_instruction(&verum_vbc::Instruction::RetV, &mut module.bytecode);
    let end = module.bytecode.len() as u32;
    let descriptor = module
        .functions
        .iter_mut()
        .find(|f| f.id == dispose)
        .unwrap();
    descriptor.bytecode_offset = start;
    descriptor.bytecode_length = end - start;
    descriptor.instructions = None;
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    run_native_module(wire);
}
