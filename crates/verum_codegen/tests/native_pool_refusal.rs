//! T2263: missing work cannot become a successful raw-pool completion.
//! Structural checks cover the actual runtime emitter; Darwin subprocess
//! controls execute it. These do not qualify the complete typed stdlib pool.
use verum_codegen::llvm::vbc_lowering::PanicStrategy;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::List;
use verum_llvm::context::Context;
use verum_llvm::values::AnyValue;
use verum_vbc::module::VbcModule;

#[test]
fn absent_pool_callback_cannot_publish_a_completed_zero() {
    for strategy in [PanicStrategy::Unwind, PanicStrategy::Abort] {
        let context = Context::create();
        let lowering = runtime(
            &context,
            "pool_absent_callback",
            strategy,
            "aarch64-apple-darwin",
        );
        let module = lowering.module();
        let worker = module
            .get_function("pool_worker_entry")
            .expect("worker body");
        assert!(worker.verify(true), "actual worker must have valid IR");
        let missing = worker
            .get_basic_blocks()
            .into_iter()
            .find(|block| block.get_name().to_bytes() == b"exec_null")
            .expect("the missing-callback branch");
        let body = missing
            .get_instructions()
            .map(|i| i.print_to_string().to_string())
            .collect::<List<_>>()
            .join("\n");
        assert!(body.contains("call void @verum_panic("), "{body}");
        assert!(body.contains("unreachable"), "{body}");
        assert!(
            !body.contains("store ") && !body.contains("br label"),
            "{body}"
        );
        let actual = worker.print_to_string().to_string();
        let phi = actual
            .lines()
            .find(|line| line.contains("task_result = phi"))
            .expect("actual callback result merge");
        assert!(!phi.contains("exec_null"), "{phi}");
        assert!(
            phi.contains("%raw_result") && phi.contains("%closure_result"),
            "{phi}"
        );
    }
}

#[test]
fn supported_pool_callback_paths_still_invoke_their_owned_callable() {
    let context = Context::create();
    let lowering = runtime(
        &context,
        "pool_valid_callbacks",
        PanicStrategy::Unwind,
        "aarch64-apple-darwin",
    );
    let module = lowering.module();
    let worker = module
        .get_function("pool_worker_entry")
        .expect("worker body");
    assert!(worker.verify(true));
    let actual = worker.print_to_string().to_string();
    assert!(actual.contains("call i64 %raw_fn_ptr(i64 %arg"), "{actual}");
    assert!(
        actual.contains("call i64 %closure_fn(ptr %closure_env, i64 %arg"),
        "{actual}"
    );
    assert!(
        actual.contains("store atomic i64 1, ptr %dp release"),
        "{actual}"
    );
}

#[test]
fn contended_exchange_that_observes_unlock_acquires_the_mutex() {
    use verum_llvm::{
        AtomicOrdering, AtomicRMWBinOp, IntPredicate,
        values::{BasicValue, InstructionOpcode},
    };

    let context = Context::create();
    let lowering = runtime(
        &context,
        "mutex_race",
        PanicStrategy::Unwind,
        "aarch64-apple-darwin",
    );
    let mutex = lowering.module().get_function("verum_mutex_lock").unwrap();
    let contended = mutex
        .get_basic_blocks()
        .into_iter()
        .find(|block| block.get_name().to_bytes() == b"contended")
        .unwrap();
    let branch = contended.get_terminator().unwrap();
    assert!(
        branch.is_conditional(),
        "first exchange cannot unconditionally sleep"
    );
    let comparison = branch
        .get_operand(0)
        .unwrap()
        .value()
        .unwrap()
        .into_int_value()
        .as_instruction_value()
        .unwrap();
    assert_eq!(comparison.get_icmp_predicate(), Some(IntPredicate::EQ));
    let exchange = comparison
        .get_operand(0)
        .unwrap()
        .value()
        .unwrap()
        .into_int_value()
        .as_instruction_value()
        .unwrap();
    assert_eq!(exchange.get_opcode(), InstructionOpcode::AtomicRMW);
    assert_eq!(exchange.get_atomic_rmw_bin_op(), Some(AtomicRMWBinOp::Xchg));
    assert_eq!(
        exchange.get_atomic_ordering().unwrap(),
        AtomicOrdering::Acquire
    );
    assert_eq!(
        exchange
            .get_operand(1)
            .unwrap()
            .value()
            .unwrap()
            .into_int_value()
            .get_zero_extended_constant(),
        Some(2)
    );
    assert_eq!(exchange.get_parent(), Some(contended));
    assert_eq!(
        comparison
            .get_operand(1)
            .unwrap()
            .value()
            .unwrap()
            .into_int_value()
            .get_zero_extended_constant(),
        Some(0)
    );

    // Causal schedule: CAS observed a holder, that holder unlocks, then
    // exchange(2) observes zero. Its successful acquire must return without
    // waiting for a nonexistent owner. Other observations must still wait.
    for previous in [0, 1, 2] {
        // LLVM conditional branch operands are condition, false, true.
        let destination = branch
            .get_operand(if previous == 0 { 2 } else { 1 })
            .unwrap()
            .block()
            .unwrap();
        if previous == 0 {
            assert_eq!(
                destination.get_terminator().unwrap().get_opcode(),
                InstructionOpcode::Return
            );
            assert!(
                !destination
                    .get_instructions()
                    .any(|i| i.get_opcode() == InstructionOpcode::Call)
            );
        } else {
            assert!(destination.get_instructions().any(|i| {
                i.print_to_string()
                    .to_string()
                    .contains("@verum_futex_wait(")
            }));
        }
    }
}

#[test]
fn pool_binding_refuses_foreign_abi_or_unsupported_target() {
    use verum_fast_parser::Parser;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    for (triple, callback, expected) in [
        ("aarch64-apple-darwin", "Int", "incompatible worker pool signature"),
        ("aarch64-unknown-linux-gnu", "fn(Int) -> Int", "pool_global_submit"),
    ] {
        let source = format!(r#"
            module app;
            @intrinsic("pool_global_submit") fn queue(callback: {callback}, value: Int) -> Int;
            public fn probe(callback: {callback}) -> Int {{ queue(callback, 27) }}
        "#);
        let ast = Parser::new(&source).parse_module().unwrap();
        let vbc = VbcCodegen::with_config(CodegenConfig::new("app")).compile_module(&ast).unwrap();
        let context = Context::create();
        let mut config = LoweringConfig::debug("app").with_debug_info(false);
        config.target_triple = Some(triple.into());
        let mut lower = VbcToLlvmLowering::new(&context, config);
        let error = lower.lower_module(&vbc).expect_err("unsupported binding must refuse");
        assert!(error.to_string().contains(expected), "{triple}: {error}");
    }
}

fn runtime<'ctx>(
    context: &'ctx Context,
    name: &str,
    strategy: PanicStrategy,
    triple: &str,
) -> VbcToLlvmLowering<'ctx> {
    let mut config = LoweringConfig::debug(name).with_debug_info(false);
    config.target_triple = Some(triple.into());
    config.panic_strategy = strategy;
    let mut lowering = VbcToLlvmLowering::new(context, config);
    lowering
        .lower_module(&VbcModule::new(name.into()))
        .expect("ordinary lowering emits the complete platform runtime");
    lowering.module().verify().expect("complete runtime IR");
    lowering
}

// The existing native worker/OS-thread implementation is Darwin-only. The
// cross-target futex defect is independently tracked as T2264.
#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::{
        path::Path,
        process::{Command, Stdio},
        sync::Mutex,
        time::{Duration, Instant},
    };
    use verum_fast_parser::Parser;
    use verum_llvm::{
        OptimizationLevel,
        builder::Builder,
        module::Module,
        targets::{CodeModel, FileType, RelocMode, Target, TargetMachine},
        values::{IntValue, PointerValue},
    };
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    static NATIVE: Mutex<()> = Mutex::new(());

    fn run_binary(binary: &Path) -> std::process::Output {
        let mut child = Command::new(binary)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("native subprocess");
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().expect("native child status").is_none() {
            if Instant::now() >= deadline {
                child.kill().expect("stop timed-out native child");
                let output = child.wait_with_output().expect("reap native child");
                panic!(
                    "native pool timed out: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().expect("native child output")
    }

    fn emit_binary(module: &Module<'_>, directory: &Path) -> std::path::PathBuf {
        let triple = module.get_triple();
        let machine = Target::from_triple(&triple)
            .expect("native target")
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .expect("native target machine");
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        module.verify().expect("complete native module verifies");
        let object = directory.join("pool.o");
        machine
            .write_to_file(module, FileType::Object, &object)
            .expect("actual AOT object emission");
        let binary = directory.join("pool");
        // The compiler's Darwin cc-driver path uses the same object emitter
        // and dead stripping. All runtime calls resolve against libSystem;
        // there is no JIT symbol mapping or test replacement implementation.
        let linked = Command::new("cc")
            .arg(&object)
            .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
            .arg(&binary)
            .output()
            .expect("Darwin AOT linker");
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        binary
    }

    fn array<'ctx>(
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        values: &[IntValue<'ctx>],
        name: &str,
    ) -> PointerValue<'ctx> {
        let ty = context.i64_type().array_type(values.len() as u32);
        let address = builder
            .build_alloca(ty, name)
            .expect("runtime input storage");
        for (index, value) in values.iter().enumerate() {
            // SAFETY: index is within the allocated i64 array, and both
            // queue and pool use the emitter's documented 64-bit layout.
            let slot = unsafe {
                builder.build_in_bounds_gep(
                    ty,
                    address,
                    &[
                        context.i64_type().const_zero(),
                        context.i64_type().const_int(index as u64, false),
                    ],
                    "slot",
                )
            }
            .expect("runtime input slot");
            builder
                .build_store(slot, *value)
                .expect("runtime input field");
        }
        address
    }

    #[test]
    fn null_native_work_refuses_before_completion() {
        let _serial = NATIVE.lock().unwrap();
        let context = Context::create();
        let lowering = runtime(
            &context,
            "missing_pool_work",
            PanicStrategy::Abort,
            TargetMachine::get_default_triple()
                .as_str()
                .to_str()
                .unwrap(),
        );
        let module = lowering.module();
        let main = module.add_function("main", context.i32_type().fn_type(&[], false), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(main, "entry"));
        let i64_type = context.i64_type();
        let zero = i64_type.const_zero();
        let one = i64_type.const_int(1, false);
        let receipt = array(
            &context,
            &builder,
            &[i64_type.const_int(-713_i64 as u64, true), zero],
            "receipt",
        );
        let receipt_address = builder
            .build_ptr_to_int(receipt, i64_type, "receipt_address")
            .unwrap();
        // Deliberately malformed low-level input, not a well-typed source
        // program: one null callable, argument 37, live completion slots.
        let queue = array(
            &context,
            &builder,
            &[
                zero,
                i64_type.const_int(37, false),
                receipt_address,
            ],
            "queue",
        );
        let queue_address = builder
            .build_ptr_to_int(queue, i64_type, "queue_address")
            .unwrap();
        // queue, capacity, head, tail, len, mutex/available (two u32),
        // threads, worker_count, shutdown, not_full/padding (two u32).
        let pool = array(
            &context,
            &builder,
            &[
                queue_address,
                one,
                zero,
                zero,
                one,
                zero,
                zero,
                zero,
                one,
                zero,
            ],
            "pool",
        );
        let pool_address = builder
            .build_ptr_to_int(pool, i64_type, "pool_address")
            .unwrap();
        builder
            .build_call(
                module
                    .get_function("pool_worker_entry")
                    .expect("actual worker"),
                &[pool_address.into()],
                "work",
            )
            .unwrap();
        builder
            .build_return(Some(&context.i32_type().const_int(72, false)))
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let child = run_binary(&emit_binary(module, directory.path()));
        let stderr = String::from_utf8_lossy(&child.stderr);
        assert_eq!(child.status.code(), Some(1), "{stderr}");
        assert_eq!(
            stderr.trim(),
            "PANIC: Thread pool task has no callable",
            "{stderr}"
        );
    }

    fn source_module(wire: bool) -> VbcModule {
        let ast = Parser::new(
            "fn compute(value: Int) -> Int { value + 5 }\n\
             async fn probe() -> Int { let work = spawn compute(37); work.await }",
        )
        .parse_module()
        .expect("ordinary spawn grammar");
        let source = VbcCodegen::with_config(CodegenConfig::new("pool_positive"))
            .compile_module(&ast)
            .expect("ordinary spawn compilation");
        if !wire {
            return source;
        }
        let mut loaded = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&source).expect("source serialization"),
        )
        .expect("wire decode");
        for function in &mut loaded.functions {
            let start = function.bytecode_offset as usize;
            let end = start + function.bytecode_length as usize;
            let mut body = verum_vbc::bytecode::decode_instructions(&loaded.bytecode[start..end])
                .expect("wire body decode");
            verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
            function.instructions = Some(body);
        }
        loaded
    }

    fn run_declared_pool(callback: &str) -> std::process::Output {
        let source = format!(r#"
            module app;
            @intrinsic("pool_create") fn open(n: Int) -> Int;
            @intrinsic("pool_submit") fn schedule(p: Int, callback: fn(Int) -> Int, value: Int) -> Int;
            @intrinsic("pool_global_submit") fn schedule_global(callback: fn(Int) -> Int, value: Int) -> Int;
            @intrinsic("pool_await") fn take(id: Int) -> Int;
            @intrinsic("pool_destroy") fn close(id: Int);
            fn times_three(x: Int) -> Int {{ x * 3 }}
            fn queue(f: fn(Int) -> Int) -> Int {{ schedule_global(f, 9) }}
            {callback}
            fn probe() -> Int {{
                let pool = open(2);
                let first = schedule(pool, times_three, 15);
                let second = submit_from_caller();
                close(pool);
                take(second) + take(first) + 1
            }}
        "#);
        run_pool_program(&source)
    }

    fn run_pool_program(source: &str) -> std::process::Output {
        let source = Parser::new(source).parse_module().unwrap();
        let vbc = VbcCodegen::with_config(CodegenConfig::new("app")).compile_module(&source).unwrap();
        let context = Context::create();
        let mut config = LoweringConfig::debug("app").with_debug_info(false);
        config.panic_strategy = PanicStrategy::Abort;
        let mut lowering = VbcToLlvmLowering::new(&context, config);
        lowering.lower_module(&vbc).expect("declaration-owned callback ABI");
        let module = lowering.module();
        let main = module.add_function("main", context.i32_type().fn_type(&[], false), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(main, "entry"));
        let result = builder.build_call(module.get_function("app.probe").unwrap(), &[], "result")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let status = builder.build_int_truncate(result, context.i32_type(), "status").unwrap();
        builder.build_return(Some(&status)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        run_binary(&emit_binary(module, directory.path()))
    }

    #[test]
    fn declared_blocking_pool_operations_run_the_selected_callbacks() {
        let _serial = NATIVE.lock().unwrap();
        let child = run_declared_pool("fn submit_from_caller() -> Int { let f = times_three; queue(f) }");
        assert_eq!(child.status.code(), Some(73), "{}", String::from_utf8_lossy(&child.stderr));
    }

    #[test]
    fn dispatch_returns_before_the_worker_is_permitted_to_finish() {
        let _serial = NATIVE.lock().unwrap();
        for submit in ["dispatch(pool, work, 73);", "global_dispatch(work, 73);"] {
            let source = format!(r#"
                module app;
                @intrinsic("pool_create") fn open(n: Int) -> Int;
                @intrinsic("pool_destroy") fn close(pool: Int);
                @intrinsic("pool_dispatch") fn dispatch(pool: Int, f: fn(Int), n: Int);
                @intrinsic("pool_global_dispatch") fn global_dispatch(f: fn(Int), n: Int);
                fn read(p: *const Int64, order: UInt8) -> Int64 {{ @intrinsic("atomic_load_i64", p, order) }}
                fn write(p: *mut Int64, n: Int64, order: UInt8) {{ @intrinsic("atomic_store_i64", p, n, order); }}
                static mut RELEASE: Int64 = 0;
                static mut DONE: Int64 = 0;
                fn work(n: Int) {{
                    unsafe {{
                        while read(&RELEASE as *const Int64, 1) == 0 {{}}
                        write(&mut DONE as *mut Int64, n as Int64, 2);
                    }}
                }}
                fn probe() -> Int {{
                    let pool = open(1);
                    {submit}
                    unsafe {{
                        write(&mut RELEASE as *mut Int64, 1, 2);
                        while read(&DONE as *const Int64, 1) == 0 {{}}
                    }}
                    close(pool);
                    unsafe {{ read(&DONE as *const Int64, 1) as Int }}
                }}
            "#);
            let child = run_pool_program(&source);
            assert_eq!(child.status.code(), Some(73), "{submit}: {}", String::from_utf8_lossy(&child.stderr));
        }
    }

    #[test]
    fn dispatch_refuses_an_indirect_borrowed_capture() {
        let _serial = NATIVE.lock().unwrap();
        let child = run_pool_program(r#"
            module app;
            @intrinsic("pool_global_dispatch") fn enqueue(f: fn(Int), n: Int);
            fn delegate(f: fn(Int)) { enqueue(f, 73); }
            fn probe() -> Int {
                let mut value = 3;
                let reference = &mut value;
                delegate(|n| { *reference = n; });
                value
            }
        "#);
        assert_eq!(child.status.code(), Some(1));
        assert_eq!(String::from_utf8_lossy(&child.stderr).trim(),
            format!("PANIC: {}", verum_vbc::intrinsics::registry::PoolOperation::CAPTURE_REFUSAL));
    }

    #[test]
    fn indirect_and_escaping_captures_refuse_before_native_worker_admission() {
        let _serial = NATIVE.lock().unwrap();
        for (owner, body) in [
            ("let bias = 3;", "x + bias"),
            ("let mut local = 2; let reference = &mut local;", "*reference += x; *reference"),
        ] {
            let callback = format!("fn submit_from_caller() -> Int {{ {owner} let f = |x| {{ {body} }}; queue(f) }}");
            let child = run_declared_pool(&callback);
            assert_eq!(child.status.code(), Some(1), "{}", String::from_utf8_lossy(&child.stderr));
            assert_eq!(String::from_utf8_lossy(&child.stderr).trim(),
                format!("PANIC: {}", verum_vbc::intrinsics::registry::PoolOperation::CAPTURE_REFUSAL));
        }
    }

    #[test]
    fn ordinary_spawn_await_keeps_its_real_nonzero_result() {
        let _serial = NATIVE.lock().unwrap();
        for wire in [false, true] {
            let context = Context::create();
            let source = source_module(wire);
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("pool_positive").with_debug_info(false),
            );
            lowering
                .lower_module(&source)
                .expect("ordinary native spawn lowering");
            let module = lowering.module();
            let main = module.add_function("main", context.i32_type().fn_type(&[], false), None);
            let builder = context.create_builder();
            builder.position_at_end(context.append_basic_block(main, "entry"));
            // This entry harness calls the unchanged source function. Both
            // spawn and await execute the ordinary generated pool runtime.
            let value = builder
                .build_call(
                    module
                        .get_function("pool_positive.probe")
                        .expect("source entry"),
                    &[],
                    "value",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let pool = builder
                .build_call(
                    module
                        .get_function("verum_pool_global")
                        .expect("global owner"),
                    &[],
                    "pool",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap();
            builder
                .build_call(
                    module
                        .get_function("verum_pool_destroy")
                        .expect("pool shutdown"),
                    &[pool.into()],
                    "",
                )
                .unwrap();
            let correct = builder
                .build_int_compare(
                    verum_llvm::IntPredicate::EQ,
                    value,
                    context.i64_type().const_int(42, false),
                    "correct_result",
                )
                .unwrap();
            let status = builder
                .build_select(
                    correct,
                    context.i32_type().const_int(42, false),
                    context.i32_type().const_int(72, false),
                    "status",
                )
                .unwrap()
                .into_int_value();
            builder.build_return(Some(&status)).unwrap();
            let directory = tempfile::tempdir().unwrap();
            let binary = emit_binary(module, directory.path());
            // T2268: real startup/teardown contention exercised the mutex's
            // CAS-to-exchange window. Keep the same per-process deadline.
            for iteration in 0..16 {
                let child = run_binary(&binary);
                assert_eq!(
                    child.status.code(),
                    Some(42),
                    "wire={wire}, iteration={iteration}: {}",
                    String::from_utf8_lossy(&child.stderr)
                );
            }
        }
    }
}
