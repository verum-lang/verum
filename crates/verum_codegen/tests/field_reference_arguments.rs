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
fn field_reference_cast_has_explicit_raw_address_conversion() {
    let output = ir(r#"
        type Cell is { padding: Int, value: Int };
        fn probe(c: &Cell) -> &unsafe Byte { &c.value as &unsafe Byte }
    "#);
    assert!(output.contains("refield_raw_addr"), "{output}");
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
    let vbc = VbcCodegen::new().compile_module(&ast).expect("VBC");
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
