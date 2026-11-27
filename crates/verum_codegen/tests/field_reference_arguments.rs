//! T1537: references passed across a call keep the original field address.
use verum_codegen::llvm::{LoweringConfig, RefTier, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::context::Context;
use verum_vbc::codegen::VbcCodegen;

fn ir(source: &str) -> String {
    let ast = Parser::new(source).parse_module().expect("parse");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("VBC");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("field_refs")
            .with_default_tier(RefTier::Tier1)
            .with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("LLVM");
    assert!(lower.module().get_function("probe").unwrap().verify(true));
    lower.get_ir().to_string()
}

#[test]
fn scalar_reference_call_keeps_field_address() {
    let output = ir(r#"
        type Cell is { padding: Int, value: Int };
        fn replace_value(p: &mut Int) { *p = 77; }
        fn probe(c: &mut Cell) { replace_value(&mut c.value); }
    "#);
    assert!(output.contains("refield_arg_addr"), "{output}");
}

#[test]
fn borrowed_field_forwarding_keeps_address_across_alias() {
    let output = ir(r#"
        type Cell is { padding: Int, value: Int };
        fn inspect(p: &Int) -> Int { *p }
        fn forward(p: &Int) -> Int { inspect(p) }
        fn probe(c: &Cell) -> Int { let alias = &c.value; forward(alias) }
    "#);
    assert!(output.contains("refield_arg_addr"), "{output}");
}

#[test]
fn generic_call_reads_the_original_scalar_field_cell() {
    let (value, fields) = execute(r#"
        type Cell is { padding: Int, value: Int };
        fn inspect<T>(marker: T, field: &Int) -> Int { *field }
        fn probe(cell: &Cell, unused: Bool) -> Int { inspect<Int>(0, &cell.value) }
    "#, 11, 73, false);
    assert_eq!(value, 73);
    assert_eq!(&fields[3..], &[11, 73]);
}

#[test]
fn field_reference_cast_has_explicit_raw_address_conversion() {
    let output = ir(r#"
        type Cell is { padding: Int, value: Int };
        fn probe(c: &Cell) -> &unsafe Byte { &c.value as &unsafe Byte }
    "#);
    assert!(output.contains("refield_raw_addr"), "{output}");
}

#[test]
fn returned_call_result_does_not_retain_field_reference_identity() {
    let output = ir(r#"
        type Value is { count: Int };
        type Pair is { left: Value, right: Value };
        implement Value {
            fn equal(&self, other: &Value) -> Bool { self.count == other.count }
        }
        fn compare(left: &Value, right: &Value) -> Bool { left.equal(right) }
        fn probe(pair: &Pair) -> Bool { pair.left.equal(&pair.right) }
        fn direct(pair: &Pair) -> Bool { compare(&pair.left, &pair.right) }
    "#);
    for name in ["probe", "direct"] {
        let start = output.find(&format!("@{name}(")).expect("declared function");
        let body = output[start..].split_once("\n}").unwrap().0;
        assert!(body.contains("refield_loaded"), "{body}");
        assert!(!body.contains("refield_slot_addr"), "{body}");
    }
}

fn execute(source: &str, first: i64, second: i64, choose: bool) -> (i64, [i64; 5]) {
    execute_with_runtime(source, first, second, choose, &[])
}

fn execute_with_runtime(
    source: &str,
    first: i64,
    second: i64,
    choose: bool,
    runtime: &[&str],
) -> (i64, [i64; 5]) {
    use verum_llvm::targets::{InitializationConfig, Target};
    use verum_llvm::{OptimizationLevel, memory_buffer::MemoryBuffer, values::AnyValue};
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let ast = Parser::new(source).parse_module().expect("parse");
    let mut vbc = VbcCodegen::new().compile_module(&ast).expect("VBC");
    // Match native source execution: concrete calls select their declared
    // specialization before lowering, including the callee's typed cleanup.
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &mut vbc.functions {
        function.is_generic = !function.type_params.is_empty();
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &vbc.bytecode[start..start + function.bytecode_length as usize],
        ).expect("loaded source body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for function in &vbc.functions {
        verum_vbc::mono::discover_call_instantiations(
            &vbc, function.instructions.as_ref().unwrap(), function.func_id_base, &mut graph,
        ).expect("declared call witnesses");
    }
    let vbc = verum_vbc::mono::monomorphize_minimal(vbc, &graph)
        .expect("specialized source module").module;
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("field_refs")
            .with_default_tier(RefTier::Tier1)
            .with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("LLVM");
    // Execute the source functions from the real lowerer. Unused standalone
    // platform wrappers are outside this unit's native ABI contract.
    let mut definitions = vbc
        .functions
        .iter()
        // Generic templates are retained in VBC for further specialization;
        // only their selected concrete bodies belong in this executable.
        // Any call still reaching an omitted template fails IR validation.
        .filter(|fd| !fd.is_generic)
        .map(|fd| {
            let name = vbc.get_string(fd.name).unwrap();
            let f = lower.module().get_function(name).expect(name);
            assert!(f.verify(true), "{name}");
            f.print_to_string().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for name in runtime {
        definitions.push_str("\n");
        definitions.push_str(
            &lower
                .module()
                .get_function(name)
                .expect(name)
                .print_to_string()
                .to_string(),
        );
    }
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            definitions.as_bytes(),
            "source_functions",
        ))
        .expect("source IR");
    executable.verify().expect("valid source IR");
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    let mut fields = [0_i64, 0, 0, first, second];
    // SAFETY: probe takes a native record handle and Bool, returning Int. The
    // storage includes the 24-byte object header and both 8-byte field slots.
    let result = unsafe {
        engine
            .get_function::<unsafe extern "C" fn(*mut i64, u64) -> i64>("probe")
            .expect("probe")
            .call(fields.as_mut_ptr(), choose as u64)
    };
    (result, fields)
}

#[test]
fn mutation_through_forwarded_alias_reaches_original_field() {
    let (result, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        fn assign_cell_value(p: &mut Int) { *p = 77; }
        fn forward(p: &mut Int) { assign_cell_value(p); }
        fn probe(c: &mut Cell, unused: Bool) -> Int {
            let alias = &mut c.value;
            let copied_alias = alias;
            forward(copied_alias);
            c.value
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(result, 77);
    assert_eq!(&fields[3..], &[41, 77]);
}

#[test]
fn conditional_reference_preserves_each_runtime_branch() {
    let source = r#"
        type Cell is { padding: Int, value: Int };
        fn assign_cell_value(p: &mut Int) { *p = 77; }
        fn probe(c: &mut Cell, choose: Bool) -> Int {
            let alias = if choose { &mut c.value } else { &mut c.padding };
            assign_cell_value(alias);
            c.value
        }
    "#;
    assert_eq!(&execute(source, 41, 17, true).1[3..], &[41, 77]);
    assert_eq!(&execute(source, 41, 17, false).1[3..], &[77, 17]);
}

#[test]
fn loop_reference_aliases_do_not_keep_the_previous_iteration_address() {
    let (_, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        fn assign_cell_value(p: &mut Int, value: Int) { *p = value; }
        fn probe(c: &mut Cell, unused: Bool) -> Int {
            let mut i = 0;
            while i < 2 {
                let alias = if i == 0 { &mut c.padding } else { &mut c.value };
                assign_cell_value(alias, 70 + i);
                i += 1;
            }
            c.value
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(&fields[3..], &[70, 71]);
}

#[test]
fn dereferenced_by_value_argument_does_not_receive_the_cell_address() {
    let (result, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        fn echo(value: Int) -> Int { value + 1 }
        fn probe(c: &Cell, unused: Bool) -> Int {
            let alias = &c.value;
            echo(*alias)
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(result, 18);
    assert_eq!(&fields[3..], &[41, 17]);
}

#[test]
fn raw_cast_of_borrowed_field_writes_the_original_cell() {
    let (result, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        fn probe(c: &mut Cell, unused: Bool) -> Int {
            let alias = &mut c.value;
            let raw = alias as &unsafe Int;
            unsafe { *raw = 77; }
            c.value
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(result, 77);
    assert_eq!(&fields[3..], &[41, 77]);
}

#[test]
fn method_reference_argument_preserves_cell_and_record_receiver() {
    let (result, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        implement Cell {
            fn assign_other(&self, destination: &mut Int) { *destination = self.padding; }
        }
        fn probe(c: &mut Cell, unused: Bool) -> Int {
            c.assign_other(&mut c.value);
            c.value
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(result, 41);
    assert_eq!(&fields[3..], &[41, 41]);
}

#[test]
fn replacing_a_reference_register_with_a_value_clears_provenance() {
    let (result, fields) = execute(
        r#"
        type Cell is { padding: Int, value: Int };
        fn inspect(p: &Int) -> Int { *p }
        fn echo(value: Int) -> Int { value }
        fn probe(c: &Cell, unused: Bool) -> Int {
            { let alias = &c.value; let seen = inspect(alias); }
            let value = 123;
            echo(value)
        }
    "#,
        41,
        17,
        false,
    );
    assert_eq!(result, 123);
    assert_eq!(&fields[3..], &[41, 17]);
}

#[cfg(target_os = "macos")]
#[test]
fn forwarded_field_reference_reaches_native_futex_mismatch_precheck() {
    let (result, fields) = execute_with_runtime(
        r#"
        type Cell is { padding: Int, value: Int };
        fn wait_once(addr: &Int) -> Int {
            @intrinsic("futex_wait", addr as &unsafe Byte, 8, 0_u64)
        }
        fn probe(c: &Cell, unused: Bool) -> Int { wait_once(&c.value) }
    "#,
        41,
        7,
        false,
        &["verum_futex_wait", "__ulock_wait"],
    );
    assert_eq!(
        result, -11,
        "mismatched value returns EAGAIN without waiting"
    );
    assert_eq!(&fields[3..], &[41, 7]);
}

#[test]
fn provenance_clear_builder_failure_returns_a_lowering_error() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{Instruction, Reg};
    let context = Context::create();
    let module = context.create_module("write_error");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let mut ctx = FunctionContext::new(&context, &module, function, "probe");
    let entry = context.append_basic_block(function, "entry");
    ctx.builder().position_at_end(entry);
    ctx.prepare_field_reference_slots([0]).expect("provenance slot");
    ctx.builder().clear_insertion_position();
    let error = lower_instruction(&mut ctx, &Instruction::LoadI { dst: Reg(0), value: 3 })
        .expect_err("failed provenance clearing must reject lowering");
    assert!(error.to_string().contains("Builder error"), "{error}");
}

#[test]
fn parameter_write_failure_is_checked_before_the_next_instruction() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::Instruction;
    let context = Context::create();
    let module = context.create_module("parameter_error");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let mut ctx = FunctionContext::new(&context, &module, function, "probe");
    let entry = context.append_basic_block(function, "entry");
    ctx.builder().position_at_end(entry);
    ctx.prepare_field_reference_slots([0]).expect("provenance slot");
    ctx.builder().clear_insertion_position();
    ctx.set_register(0, context.i64_type().const_zero().into());
    // Restoring the builder cannot erase the earlier failed register write.
    ctx.builder().position_at_end(entry);
    assert!(lower_instruction(&mut ctx, &Instruction::RetV).is_err());
    assert!(entry.get_terminator().is_none());
}

#[test]
fn narrow_native_references_keep_signedness_and_width() {
    let output = ir(r#"
        fn probe(s: &Int16, u: &UInt8) -> Int { (*s as Int) + (*u as Int) }
    "#);
    for instruction in ["load i16", "sext i16", "load i8", "zext i8"] {
        assert!(output.contains(instruction), "missing {instruction} in {output}");
    }
}

fn attenuate_reference(dst: u16, src: u16) -> verum_vbc::Instruction {
    use verum_vbc::{Instruction, Reg, instruction::CbgrSubOpcode, value::Capabilities};
    let mut operands = verum_common::List::<u8>::new().into();
    verum_vbc::encoding::encode_reg(Reg(dst), &mut operands);
    verum_vbc::encoding::encode_reg(Reg(src), &mut operands);
    operands.extend_from_slice(&Capabilities::READ_ONLY.0.to_le_bytes());
    Instruction::CbgrExtended { sub_op: CbgrSubOpcode::CapAttenuate as u8, operands }
}

#[test]
fn attenuation_keeps_the_field_place_through_aliases_and_clears_replaced_places() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_llvm::{OptimizationLevel, targets::{InitializationConfig, Target}};
    use verum_vbc::{Instruction, Reg, instruction::CbgrSubOpcode};
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    // Keep the field reference, replace it with an ordinary reference, or
    // replace its copied destination with a scalar.
    for overwrite in [None, Some(Reg(0)), Some(Reg(2))] {
        let context = Context::create();
        let module = context.create_module("attenuated_place");
        let word = context.i64_type();
        let function = module.add_function("probe", word.fn_type(&[word.into(), word.into()], false), None);
        let mut ctx = FunctionContext::new(&context, &module, function, "probe");
        ctx.builder().position_at_end(context.append_basic_block(function, "entry"));
        ctx.prepare_field_reference_slots([0, 1, 2]).unwrap();
        ctx.set_register(0, word.const_int(73, false).into());
        ctx.set_field_reference_address(0, function.get_nth_param(0).unwrap().into_int_value()).unwrap();
        ctx.mark_ref_param_register(0);
        ctx.set_sticky_type_hint(0, "Int".into());
        // Reusing dst must replace this different place and its old type facts.
        ctx.set_register(1, word.const_int(19, false).into());
        ctx.set_field_reference_address(1, function.get_nth_param(1).unwrap().into_int_value()).unwrap();
        ctx.set_sticky_type_hint(1, "Bool".into());
        if overwrite == Some(Reg(0)) {
            // A native reference parameter already contains the cell address;
            // it must not inherit the previous field's separate address slot.
            ctx.set_register(0, function.get_nth_param(1).unwrap());
            ctx.mark_ref_param_register(0);
        }
        lower_instruction(&mut ctx, &attenuate_reference(0, 0)).unwrap();
        lower_instruction(&mut ctx, &attenuate_reference(1, 0)).unwrap();
        assert_eq!(ctx.sticky_type_hint(1), Some("Int"));
        assert!(ctx.is_ref_param_register(1));
        lower_instruction(&mut ctx, &Instruction::Mov { dst: Reg(2), src: Reg(1) }).unwrap();
        let result = if overwrite == Some(Reg(2)) {
            lower_instruction(&mut ctx, &Instruction::LoadI { dst: Reg(2), value: 123 }).unwrap();
            assert!(!ctx.is_ref_param_register(2));
            // Inspect the address sidecar directly. A scalar is not an operand
            // for a reference conversion, and must retain no former cell.
            ctx.field_reference_address(2).unwrap().unwrap().into()
        } else {
            lower_instruction(&mut ctx, &Instruction::CbgrExtended {
                sub_op: CbgrSubOpcode::ToRawPtr as u8,
                operands: [3, 2].into(),
            }).unwrap();
            ctx.get_register(3).unwrap()
        };
        ctx.builder().build_return(Some(&result)).unwrap();
        module.verify().unwrap();
        let engine = module.create_jit_execution_engine(OptimizationLevel::None).unwrap();
        // SAFETY: the generated function has exactly this two-word input/result ABI.
        let probe = unsafe { engine.get_function::<unsafe extern "C" fn(u64, u64) -> u64>("probe") }.unwrap();
        let original = 73_i64;
        let replaced = 19_i64;
        let original_address = (&original as *const i64) as u64;
        let replaced_address = (&replaced as *const i64) as u64;
        // SAFETY: these live addresses are only forwarded, never dereferenced by the generated function.
        let result = unsafe { probe.call(original_address, replaced_address) };
        let expected = match overwrite {
            Some(Reg(0)) => replaced_address,
            Some(Reg(2)) => 0,
            None => original_address,
            _ => unreachable!(),
        };
        assert_eq!(result, expected);
        assert_eq!((original, replaced), (73, 19));
    }
}

#[test]
fn distinct_attenuation_destinations_receive_field_provenance_slots() {
    use verum_vbc::{Instruction, Reg, instruction::CbgrSubOpcode};
    for wide in [false, true] {
        let ast = Parser::new(r#"
            type Cell is { padding:Int, value:Int };
            fn inspect(value:&Int)->Int { *value }
            fn probe(cell:&Cell, unused:Bool)->Int { inspect(&cell.value) }
        "#).parse_module().unwrap();
        let mut vbc = VbcCodegen::new().compile_module(&ast).unwrap();
        let id = vbc.find_function_by_name("probe").unwrap();
        let function = vbc.functions.iter_mut().find(|function| function.id == id).unwrap();
        let destination = if wide { function.register_count.max(130) } else { function.register_count };
        function.register_count = destination + 1;
        let body = function.instructions.as_mut().unwrap();
        let index = body.iter().position(|instruction| matches!(instruction,
            Instruction::CbgrExtended { sub_op, .. } if *sub_op == CbgrSubOpcode::CapAttenuate as u8
        )).unwrap();
        let Instruction::CbgrExtended { operands, .. } = &body[index] else { unreachable!() };
        let mut offset = 0;
        let original_destination = verum_vbc::encoding::decode_reg(operands, &mut offset).unwrap();
        let source = verum_vbc::encoding::decode_reg(operands, &mut offset).unwrap();
        assert_eq!(original_destination, source);
        // This low-level control gives the attenuation a distinct result register
        // and forwards it back. The original source currently uses self-aliasing.
        body[index] = attenuate_reference(destination, source.0);
        body.insert(index + 1, Instruction::Mov { dst: original_destination, src: Reg(destination) });
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(&context,
            LoweringConfig::debug("distinct_attenuation")
                .with_default_tier(RefTier::Tier1).with_debug_info(false));
        lower.lower_module(&vbc).unwrap();
        let probe = lower.module().get_function("probe").unwrap();
        assert!(probe.verify(true));
        use verum_llvm::values::AnyValue;
        let output = probe.print_to_string().to_string();
        assert!(output.contains(&format!("%r{destination}_field_addr = alloca i64")), "{output}");
        assert!(output.contains("refield_arg_addr"), "{output}");
    }
}

#[test]
fn inferred_scalar_local_and_forwarded_reference_keep_their_own_storage() {
    use verum_vbc::Instruction;

    let source = r#"
        type Cell is { padding: Int, value: Int };
        fn advance(offset: &mut Int, amount: Int) { *offset = *offset + amount; }
        fn forward(offset: &mut Int, amount: Int) { advance(offset, amount); }
        fn walk(initial: &mut Int) -> Int {
            let mut position = *initial + 4;
            while position < 9 {
                forward(&mut position, 2);
            }
            position
        }
        fn probe(cell: &mut Cell, choose: Bool) -> Int {
            let selected = if choose { &mut cell.value } else { &mut cell.padding };
            let result = walk(selected);
            forward(selected, 3);
            result
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for (name, expected_slot_borrows) in [("walk", 1), ("forward", 0), ("probe", 0)] {
        let function = &wire.functions[wire.find_function_by_name(name).unwrap().0 as usize];
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let instructions =
            verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end]).unwrap();
        let slots = instructions.iter()
            .filter(|op| matches!(op, Instruction::RefLocal { .. }))
            .count();
        assert_eq!(slots, expected_slot_borrows, "{name}: {instructions:?}");
    }
    let (result, fields) = execute(source, 1, 2, false);
    assert_eq!(result, 9);
    assert_eq!(&fields[3..], &[4, 2]);
    let (result, fields) = execute(source, 1, 2, true);
    assert_eq!(result, 10);
    assert_eq!(&fields[3..], &[1, 5]);
}
