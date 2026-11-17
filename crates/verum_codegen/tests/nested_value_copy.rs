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
fn native_immutable_text_survives_replacement_of_its_copied_sum_owner() {
    run_native(r#"
        type Choice is Empty | Full(Text);
        fn main() -> Int {
            let mut first = Choice.Full("retained");
            let second = first;
            first = Choice.Empty;
            match second {
                Full(text) => if text == "retained" {73} else {0},
                Empty => 0,
            }
        }
    "#);
}

#[test]
fn native_text_builder_vbc_copy_preserves_field_projection() {
    native_text_builder_vbc_copy(false, false, false);
}

#[test]
fn native_text_builder_vbc_self_copy_preserves_field_projection() {
    native_text_builder_vbc_copy(true, false, false);
}

#[test]
fn native_text_builder_vbc_representation_copy_preserves_the_original_length() {
    native_text_builder_vbc_copy(false, true, false);
}

#[test]
fn native_transparent_text_vbc_copy_keeps_its_resolved_codec() {
    for copy_in_place in [false, true] {
        native_text_builder_vbc_copy(copy_in_place, false, true);
    }
}

fn native_text_builder_vbc_copy(copy_in_place: bool, mutate: bool, transparent: bool) {
    use verum_ast::ItemKind;
    use verum_vbc::{Instruction, Reg, TypeId, TypeRef};
    use verum_vbc::instruction::{BinaryIntOp, MemSubOpcode, TextSubOpcode};

    // Explicit VBC representation gate, not source admission. The real source
    // owns the descriptor; the fixture initializes the {ptr,len,cap} state
    // produced by Text.with_capacity + byte writes. Allocation, New(TEXT) and
    // SetF all use production lowering, without a synthetic host-memory ABI.
    let mut ast = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module().expect("canonical Text source");
    ast.items.retain(|item| matches!(&item.kind, ItemKind::Module(_))
        || matches!(&item.kind, ItemKind::Type(declaration)
            if declaration.name.name.as_str() == "Text"));
    assert_eq!(ast.items.len(), 2, "canonical module and record");
    ast.items.extend(Parser::new("type TextValue is (Text); fn main() -> Int { 0 }")
        .parse_module().expect("entry signature").items);
    let mut compiled = VbcCodegen::with_config(
        verum_vbc::codegen::CodegenConfig::new("core.text.text"),
    ).compile_module(&ast).expect("actual source descriptor");
    let descriptor = compiled.get_type(TypeId::TEXT).expect("canonical Text identity");
    assert_eq!(descriptor.kind, verum_vbc::types::TypeKind::Record);
    assert_eq!(descriptor.fields.len(), 3);
    for (index, name) in ["ptr", "len", "cap"].into_iter().enumerate() {
        assert_eq!(compiled.get_string(descriptor.fields[index].name), Some(name));
        assert_eq!(descriptor.fields[index].offset, index as u32 * 8);
        assert!(descriptor.fields[index].declaration_type.is_some());
    }
    assert_eq!(descriptor.fields[1].declaration_type, Some(TypeRef::Concrete(TypeId::INT)));
    assert_eq!(descriptor.fields[2].declaration_type, Some(TypeRef::Concrete(TypeId::INT)));
    let copy_type = if transparent {
        let wrapper = compiled.types.iter().find(|ty| compiled.get_string(ty.name)
            .is_some_and(|name| name == "TextValue" || name.ends_with(".TextValue"))).unwrap();
        assert!(wrapper.source_declaration.is_some());
        let ty = TypeRef::Concrete(wrapper.id);
        let plan = verum_vbc::copy_plan::build(&compiled, &ty).unwrap();
        let verum_vbc::copy_plan::CopyNode::Alias(inner) = plan.nodes[0] else {
            panic!("source newtype must erase to its Text payload");
        };
        assert!(matches!(plan.nodes[inner.0], verum_vbc::copy_plan::CopyNode::Text));
        ty
    } else {
        TypeRef::Concrete(TypeId::TEXT)
    };

    let mut allocation_operands = verum_common::List::<u8>::new().into();
    for register in [Reg(2), Reg(0), Reg(1)] {
        verum_vbc::encoding::encode_reg(register, &mut allocation_operands);
    }
    let copied = if copy_in_place { Reg(3) } else { Reg(6) };
    let mut instructions = verum_common::List::from_iter([
        Instruction::LoadI { dst: Reg(0), value: 17 },
        Instruction::LoadI { dst: Reg(1), value: 1 },
        // Nine NUL characters are valid UTF-8. Zeroing cap+1 initializes the
        // byte prefix and terminating NUL expected by the source mutators.
        Instruction::MemExtended {
            sub_op: MemSubOpcode::AllocZeroed.to_byte(), operands: allocation_operands,
        },
        Instruction::New { dst: Reg(3), type_id: TypeId::TEXT.0, field_count: 3 },
        Instruction::SetF { obj: Reg(3), field_idx: 0, value: Reg(2) },
        Instruction::LoadI { dst: Reg(4), value: 9 },
        Instruction::SetF { obj: Reg(3), field_idx: 1, value: Reg(4) },
        Instruction::LoadI { dst: Reg(5), value: 16 },
        Instruction::SetF { obj: Reg(3), field_idx: 2, value: Reg(5) },
        Instruction::CloneTyped { dst: copied, src: Reg(3), type_ref: copy_type },
    ]);
    if mutate {
        instructions.push(Instruction::LoadI { dst: Reg(7), value: 3 });
        instructions.push(Instruction::SetF { obj: Reg(3), field_idx: 1, value: Reg(7) });
        let mut operands = verum_common::List::<u8>::new().into();
        for register in [Reg(8), copied] {
            verum_vbc::encoding::encode_reg(register, &mut operands);
        }
        instructions.push(Instruction::TextExtended {
            sub_op: TextSubOpcode::ByteLen as u8, operands,
        });
    } else {
        // Isolate carrier preservation from storage ownership.
        instructions.push(Instruction::GetF { dst: Reg(8), obj: copied, field_idx: 1 });
    }
    instructions.extend([
        Instruction::LoadI { dst: Reg(9), value: 64 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(10), a: Reg(8), b: Reg(9) },
        Instruction::Ret { value: Reg(10) },
    ]);
    let mut bytecode = verum_common::List::<u8>::new().into();
    for instruction in &instructions {
        verum_vbc::bytecode::encode_instruction(instruction, &mut bytecode);
    }
    let offset = compiled.append_bytecode(&bytecode);
    let entry = compiled.find_function_by_name("core.text.text.main").expect("entry");
    let body = compiled.functions.iter_mut().find(|function| function.id == entry).unwrap();
    body.bytecode_offset = offset;
    body.bytecode_length = bytecode.len() as u32;
    body.register_count = 11;
    body.locals_count = 11;
    body.instructions = None;
    body.value_uses = None;
    let wire = deserialize_module(&serialize_module(&compiled).expect("serialize"))
        .expect("reload");
    verum_vbc::validate::validate_module(&wire).expect("valid representation fixture");
    run_native_module(wire);
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
fn native_typed_reference_copy_keeps_the_source_field_address() {
    use verum_vbc::{Instruction, TypeId, TypeRef, instruction::CbgrSubOpcode};
    let mut wire = module(r#"
        type Cell is { padding:Int, value:Int };
        fn main()->Int {
            let owner=Cell{padding:11,value:17};
            let original=&owner.value;
            let copied=original;
            let expected=original as &unsafe Byte as Int;
            let actual=copied as &unsafe Byte as Int;
            if expected==actual {73}else{0}
        }
    "#);
    let entry = wire.find_function_by_name("main").unwrap();
    let descriptor = wire.get_function(entry).unwrap();
    let start = descriptor.bytecode_offset as usize;
    let mut instructions = verum_vbc::bytecode::decode_instructions(
        &wire.bytecode[start..start + descriptor.bytecode_length as usize],
    ).unwrap();
    verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
    assert!(instructions.iter().any(|instruction| matches!(instruction,
        Instruction::CbgrExtended { sub_op, .. } if *sub_op == CbgrSubOpcode::RefField as u8)));
    let conversions: verum_common::List<_> = instructions.iter().enumerate().filter_map(|(index, instruction)| {
        let Instruction::CbgrExtended { sub_op, operands } = instruction else { return None; };
        if *sub_op != CbgrSubOpcode::ToRawPtr as u8 { return None; }
        let mut offset = 0;
        verum_vbc::encoding::decode_reg(operands, &mut offset).unwrap();
        Some((index, verum_vbc::encoding::decode_reg(operands, &mut offset).unwrap()))
    }).collect();
    assert_eq!(conversions.len(), 2, "two actual source field-address observations");
    let (observe, copied) = conversions[1];
    assert_ne!(conversions[0].1, copied, "the expected address keeps its original carrier");
    let (index, source) = instructions[..observe].iter().enumerate().rev().find_map(|(index, instruction)| {
        match instruction { Instruction::Mov { dst, src } if *dst == copied => Some((index, *src)), _ => None }
    }).expect("source alias binding");
    // Replace only the source alias operation. The real field producer and
    // ordinary native sidecar prepass remain responsible for its address.
    instructions[index] = Instruction::CloneTyped {
        dst: copied, src: source,
        type_ref: TypeRef::reference(TypeRef::Concrete(TypeId::INT), Default::default(), Default::default()),
    };
    let mut bytecode = verum_common::List::<u8>::new().into();
    verum_vbc::bytecode::encode_instructions_with_fixup(&instructions, &mut bytecode);
    let offset = wire.append_bytecode(&bytecode);
    let descriptor = wire.functions.iter_mut().find(|function| function.id == entry).unwrap();
    descriptor.bytecode_offset = offset;
    descriptor.bytecode_length = bytecode.len() as u32;
    descriptor.instructions = None;
    descriptor.value_uses = None;
    let wire = deserialize_module(&serialize_module(&wire).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).expect("valid typed field-reference copy");
    run_native_module(wire);
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
fn a_native_const_generic_packed_field_keeps_its_declared_cleanup() {
    run_native(r#"
        type Buffer<const N: Int> is { data: [Byte; N] };
        implement<const N: Int> Buffer<N> {
            fn new()->Self { Self { data: [0; N] } }
            fn stride(&self)->Int {
                (&self.data[3] as *const Byte as Int) - (&self.data[0] as *const Byte as Int)
            }
        }
        fn main()->Int { let buffer = Buffer<8>.new(); 70 + buffer.stride() }
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

#[test]
fn a_generic_receiver_keeps_its_owned_value_inside_the_native_result() {
    for implementation in [
        "implement<T> Cell<T> { fn wrap(self) -> Envelope<Self> { Envelope { item: self } } }",
        "type Base is protocol {}; type Wrap is protocol { fn wrap(self) -> Envelope<Self>; }; implement<T> Base for Cell<T> {} implement<T: Base> Wrap for T { fn wrap(self) -> Envelope<T> { Envelope { item: (self) } } }",
    ] {
        run_native(&format!(r#"
            type Cell<T> is {{ value: T }};
            type Envelope<T> is {{ item: T }};
            {implementation}
            fn main() -> Int {{
                let result = Cell {{ value: 73 }}.wrap();
                result.item.value
            }}
        "#));
    }
}

#[test]
fn a_selected_native_callable_keeps_its_scalar_argument_abi() {
    for body in [
        "let selected:fn(Int,Int)->Int=add; let callback=selected; callback(70,3)",
        "let callback:fn(Int)->Int=|value:Int| value+3; callback(70)",
        "let callback:fn(Float)->Int=convert; callback(3.5)",
    ] {
        run_native(&format!(r#"
            fn add(left:Int,right:Int)->Int {{left+right}}
            fn convert(value:Float)->Int {{(value as Int)+70}}
            fn main()->Int {{{body}}}
        "#));
    }
}

#[test]
fn native_callable_frame_layout_is_independent_of_its_linkage_name() {
    for closure_body in [false, true] {
        let source = if closure_body {
            "fn main()->Int {let callback:fn(Int)->Int=|value:Int| value+1;callback(72)}"
        } else {
            "fn known(value:Int)->Int {value+1} fn main()->Int {let callback:fn(Int)->Int=known;callback(72)}"
        };
        let mut wire = module(source);
        let target = if closure_body {
            wire.functions.iter().find(|f| f.closure_capture_count.is_some()).unwrap().id
        } else { wire.find_function_by_name("known").unwrap() };
        let name = wire.strings.intern(if closure_body { "renamed_callback" } else { "ordinary$closure$spelling" });
        wire.functions.iter_mut().find(|f| f.id == target).unwrap().name = name;
        run_native_module(wire);
    }
}

fn run_native(source: &str) {
    run_native_module(module(source));
}

fn run_native_module(mut wire: verum_vbc::VbcModule) {
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &mut wire.functions {
        assert_eq!(function.is_generic, !function.type_params.is_empty(), "production generic telescope");
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
    "#, r#"
        type Leaf is { value: Int };
        type Wrapped<T> is (T);
        type Outer is { inner: Wrapped<Leaf> };
        fn duplicate(source:&Outer)->Outer { source.clone() }
        fn main()->Int {
            let original=Outer { inner:Wrapped(Leaf {value:0}) };
            let mut copied=duplicate(&original);
            copied.inner.0.value=73;
            original.inner.0.value+copied.inner.0.value
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
        let entry=wire.find_function_by_name("main").unwrap();
        let mut vm=verum_vbc::interpreter::Interpreter::try_new_validated(wire.clone().into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(),Some(73));
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

#[test]
fn transparent_record_fields_preserve_instantiated_carriers() {
    for source in [
        r#"
        type Leaf is {padding:Int,value:Int};
        type Wrapped<T> is (T);
        type Outer<A> is {inner:Wrapped<A>};
        fn build(n:Int)->Outer<Leaf> {Outer{inner:Wrapped(Leaf{padding:9,value:n})}}
        fn main()->Int {
            let original=build(0);
            let mut copied=original;
            copied.inner.0.value=73;
            if original.inner.0.padding!=9 {return 0;}
            original.inner.0.value+copied.inner.0.value
        }
    "#,
        r#"
        type Leaf is {value:Int};
        type One<T> is (T);
        type Two<T> is (One<T>);
        type Holder is {inner:Two<Leaf>};
        fn main()->Int {
            let original=Holder{inner:Two(One(Leaf{value:0}))};
            let mut copied=original;
            copied.inner.0.0.value=73;
            original.inner.0.0.value+copied.inner.0.0.value
        }
    "#,
    ] {
        run_native(source);
    }
}

#[test]
fn transparent_fields_preserve_float_and_borrowed_carriers() {
    for source in [
        r#"
        type Number is (Float);
        type Holder is {inner:Number};
        fn main()->Int {let value=Holder{inner:Number(72.5)};if value.inner.0+0.5==73.0{73}else{0}}
    "#,
        r#"
        type Number is (Float32);
        type Holder is {inner:Number};
        fn main()->Int {let value=Holder{inner:Number(72.5f32)};if value.inner.0+0.5f32==73.0f32{73}else{0}}
    "#,
        r#"
        type Leaf is {value:Int};
        type Wrapped<T> is (T);
        type Holder is {inner:Wrapped<Leaf>};
        fn inspect_payload(value:&Holder)->Int {value.inner.0.value}
        fn main()->Int {let value=Holder{inner:Wrapped(Leaf{value:73})};inspect_payload(&value)}
    "#,
        r#"
        type Leaf is {value:Int};
        type Wrapped<T> is (T);
        fn main()->Int {let leaf=Leaf{value:73};let wrapper=Wrapped(&leaf);wrapper.0.value}
    "#,
    ] {
        run_native(source);
    }
}

#[test]
fn a_float_returning_source_read_keeps_its_declared_identity() {
    run_native(
        r#"
        type Number is (Float);
        type Holder is {inner:Number};
        fn read(value:&Holder)->Float {value.inner.0}
        fn main()->Int {
            let value=Holder{inner:Number(72.5)};
            if read(&value)+0.5==73.0 {73}else{0}
        }
    "#,
    );
}

// Exercise the existing GetF boundary directly: normal source .0 can lower to
// Mov. Source still supplies the exact wrapper declaration and call arguments.
fn direct_field_module(source: &str) -> verum_vbc::VbcModule {
    use verum_vbc::{Instruction, instruction::Reg};
    let mut module = module(source);
    let target = module.find_function_by_name("project").unwrap();
    let start = module.bytecode.len() as u32;
    for instruction in [
        Instruction::GetF {
            dst: Reg(1),
            obj: Reg(0),
            field_idx: 0,
        },
        Instruction::Ret { value: Reg(1) },
    ] {
        verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    let length = module.bytecode.len() as u32 - start;
    let descriptor = module
        .functions
        .iter_mut()
        .find(|function| function.id == target)
        .unwrap();
    descriptor.register_count = descriptor.register_count.max(2);
    descriptor.bytecode_offset = start;
    descriptor.bytecode_length = length;
    descriptor.instructions = None;
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    wire
}

#[test]
fn direct_scalar_getf_preserves_int_float_and_ordinary_record() {
    for source in [
        "type Number is (Int); fn project(value:Number)->Int{value.0} fn main()->Int{project(Number(73))}",
        "type Number is (Float); fn project(value:Number)->Float{value.0} fn main()->Int{if project(Number(72.5))+0.5==73.0{73}else{0}}",
        "type Number is {value:Int}; fn project(value:Number)->Int{value.value} fn main()->Int{project(Number{value:73})}",
    ] {
        let wire = direct_field_module(source);
        let entry = wire.find_function_by_name("main").unwrap();
        let mut vm =
            verum_vbc::interpreter::Interpreter::try_new_validated(wire.clone().into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        run_native_module(wire);
    }
}

#[test]
fn native_field_projection_refuses_forged_transparent_declarations() {
    for (source, forge) in [
        (
            "type Number is {value:Int}; fn project(value:Number)->Int{value.value} fn main()->Int{project(Number{value:73})}",
            0,
        ),
        (
            "type Number is (Int); fn project(value:Number)->Int{value.0} fn main()->Int{project(Number(73))}",
            1,
        ),
        (
            "type Number is (Int); fn project(value:Number)->Int{value.0} fn main()->Int{project(Number(73))}",
            2,
        ),
    ] {
        let mut wire = direct_field_module(source);
        let index = wire
            .types
            .iter()
            .position(|ty| wire.get_string(ty.name) == Some("Number"))
            .unwrap();
        match forge {
            0 => wire.types[index].is_transparent_wrapper = true,
            1 => wire.types[index].is_transparent_wrapper = false,
            _ => wire.types[index].source_declaration = None,
        }
        for function in &mut wire.functions {
            let start = function.bytecode_offset as usize;
            let mut instructions = verum_vbc::bytecode::decode_instructions(
                &wire.bytecode[start..start + function.bytecode_length as usize],
            )
            .unwrap();
            verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
            function.instructions = Some(instructions);
        }
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("forged_field").with_debug_info(false),
        );
        assert!(
            lowering.lower_module(&wire).is_err(),
            "forged transparent carrier {forge}"
        );
    }
}

#[test]
fn native_getf_accepts_a_declared_float_carrier_and_refuses_an_unproved_one() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{Instruction, instruction::Reg};
    let vbc = module("type Number is (Float); fn main()->Int{73}");
    let context = Context::create();
    let native = context.create_module("float_field");
    let function = native.add_function(
        "field",
        context
            .f64_type()
            .fn_type(&[context.f64_type().into()], false),
        None,
    );
    let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, function, "field");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_first_param().unwrap());
    ctx.set_obj_register_type(0, "Number".into());
    lower_instruction(
        &mut ctx,
        &Instruction::GetF {
            dst: Reg(1),
            obj: Reg(0),
            field_idx: 0,
        },
    )
    .unwrap();
    let value = ctx.get_register(1).unwrap();
    assert!(value.is_float_value());
    ctx.builder().build_return(Some(&value)).unwrap();
    native.verify().unwrap();
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let engine = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: the generated definition has exactly the declared f64 -> f64 ABI.
    let function =
        unsafe { engine.get_function::<unsafe extern "C" fn(f64) -> f64>("field") }.unwrap();
    // SAFETY: this call uses that exact ABI and supplies no references.
    assert_eq!(unsafe { function.call(72.5) }, 72.5);

    let other = context.create_module("unproved_field");
    let function = other.add_function(
        "field",
        context
            .f64_type()
            .fn_type(&[context.f64_type().into()], false),
        None,
    );
    let mut ctx = FunctionContext::with_vbc_module(&context, &other, &vbc, function, "field");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_first_param().unwrap());
    assert!(matches!(
        lower_instruction(
            &mut ctx,
            &Instruction::GetF {
                dst: Reg(1),
                obj: Reg(0),
                field_idx: 0
            }
        ),
        Err(verum_codegen::llvm::error::LlvmLoweringError::InvalidType(
            _
        ))
    ));
}

#[test]
fn declared_float32_getf_uses_the_existing_native_slot_contract() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{Instruction, instruction::Reg};
    let vbc = module("type Number is (Float32); fn main()->Int{73}");
    for slot in [false, true] {
        let context = Context::create();
        let native = context.create_module("f32_field");
        let input = if slot {
            context.i64_type().into()
        } else {
            context.f32_type().into()
        };
        let function =
            native.add_function("field", context.f32_type().fn_type(&[input], false), None);
        let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, function, "field");
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(0, function.get_first_param().unwrap());
        ctx.set_obj_register_type(0, "Number".into());
        lower_instruction(
            &mut ctx,
            &Instruction::GetF {
                dst: Reg(1),
                obj: Reg(0),
                field_idx: 0,
            },
        )
        .unwrap();
        let value = ctx.get_register(1).unwrap();
        assert_eq!(value.get_type(), context.f32_type().into());
        ctx.builder().build_return(Some(&value)).unwrap();
        native.verify().unwrap();
        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let engine = native
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        if slot {
            // SAFETY: the generated definition takes one i64 slot and returns f32.
            let function =
                unsafe { engine.get_function::<unsafe extern "C" fn(u64) -> f32>("field") }
                    .unwrap();
            // SAFETY: native float slots contain the f64 encoding before declared narrowing.
            assert_eq!(
                unsafe { function.call(16_777_217.0f64.to_bits()) },
                16_777_216.0f32
            );
        } else {
            // SAFETY: the generated definition takes and returns exactly f32.
            let function =
                unsafe { engine.get_function::<unsafe extern "C" fn(f32) -> f32>("field") }
                    .unwrap();
            // SAFETY: the supplied value uses that exact ABI and has no references.
            assert_eq!(unsafe { function.call(72.5) }, 72.5);
        }
    }
}

#[test]
fn transparent_field_projection_refuses_missing_arguments_and_cycles() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{
        Instruction,
        instruction::Reg,
        types::{TypeId, TypeRef},
    };
    for (source, owner, args) in [
        (
            "type Wrapper<T> is (T); fn main()->Int{73}",
            "Wrapper",
            vec![],
        ),
        (
            "type Wrapper<T> is (T); fn main()->Int{73}",
            "Wrapper",
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::BOOL),
            ],
        ),
        ("type Cycle is (Cycle); fn main()->Int{73}", "Cycle", vec![]),
    ] {
        let vbc = module(source);
        let context = Context::create();
        let native = context.create_module("incomplete_owner");
        let function = native.add_function("field", context.void_type().fn_type(&[], false), None);
        let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, function, "field");
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(0, context.i64_type().const_zero().into());
        ctx.set_obj_register_type(0, owner.into());
        ctx.set_generic_type_args(0, args);
        let error = lower_instruction(
            &mut ctx,
            &Instruction::GetF {
                dst: Reg(1),
                obj: Reg(0),
                field_idx: 0,
            },
        )
        .expect_err("no unproved layout");
        assert!(
            error.to_string().contains("transparent field carrier"),
            "{error}"
        );
    }
}

#[test]
fn erased_record_slot_read_clears_reused_destination_type_hints() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{
        Instruction,
        instruction::Reg,
        types::{TypeId, TypeRef},
    };
    let vbc = module("type Holder<T> is {value:T}; fn main()->Int{73}");
    let context = Context::create();
    let native = context.create_module("erased_record_slot");
    let function = native.add_function(
        "slot",
        context
            .i64_type()
            .fn_type(&[context.ptr_type(Default::default()).into()], false),
        None,
    );
    let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, function, "slot");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_first_param().unwrap());
    ctx.set_obj_register_type(0, "Holder".into());
    ctx.set_register(1, context.i64_type().const_zero().into());
    ctx.set_obj_register_type(1, "Holder".into());
    ctx.set_generic_type_args(1, vec![TypeRef::Concrete(TypeId::FLOAT)]);
    ctx.set_sticky_type_hint(1, "Holder<Float>".into());
    ctx.mark_float_register(1);
    lower_instruction(
        &mut ctx,
        &Instruction::GetF {
            dst: Reg(1),
            obj: Reg(0),
            field_idx: 0,
        },
    )
    .unwrap();
    assert!(ctx.get_obj_register_type(1).is_none());
    assert!(ctx.get_generic_type_args(1).is_none());
    assert!(ctx.sticky_type_hint(1).is_none());
    assert!(!ctx.is_float_register(1));
    let value = ctx.get_register(1).unwrap();
    ctx.builder().build_return(Some(&value)).unwrap();
    native.verify().unwrap();
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let engine = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // The ordinary record ABI has a header followed by fixed Value-word slots.
    let mut words =
        [0u64; verum_codegen::llvm::runtime::RuntimeLowering::OBJECT_HEADER_SIZE as usize / 8 + 1];
    *words.last_mut().unwrap() = 73;
    // SAFETY: this generated function takes a live record allocation pointer and returns its one i64 slot.
    let function =
        unsafe { engine.get_function::<unsafe extern "C" fn(*const u64) -> i64>("slot") }.unwrap();
    // SAFETY: the array covers the exact header and declared field extent for the entire call.
    assert_eq!(unsafe { function.call(words.as_ptr()) }, 73);
}

#[test]
fn transparent_field_projection_refuses_unresolved_and_foreign_payload_arguments() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{
        Instruction,
        instruction::Reg,
        types::{TypeId, TypeParamId, TypeRef},
    };
    for argument in [
        TypeRef::Generic(TypeParamId(0)),
        TypeRef::Concrete(TypeId(u32::MAX)),
    ] {
        let vbc = module("type Wrapper<T> is (T); fn main()->Int{73}");
        let context = Context::create();
        let native = context.create_module("unproved_payload");
        let function = native.add_function("field", context.void_type().fn_type(&[], false), None);
        let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, function, "field");
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(0, context.i64_type().const_zero().into());
        ctx.set_obj_register_type(0, "Wrapper".into());
        ctx.set_generic_type_args(0, vec![argument]);
        let error = lower_instruction(
            &mut ctx,
            &Instruction::GetF {
                dst: Reg(1),
                obj: Reg(0),
                field_idx: 0,
            },
        )
        .expect_err("no payload authority");
        assert!(
            error.to_string().contains("transparent field carrier"),
            "{error}"
        );
    }
}
