//! T1564: count-sized raw spawn packs are read only within their allocation.
use super::PlatformIR;
use crate::llvm::{context::FunctionContext, instruction::lower_instruction};
use verum_common::{List, Text};
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{
    instruction::{Instruction, Reg, RegRange},
    module::{FunctionDescriptor, VbcModule},
};

fn producer_ir(count: u8) -> Result<Text, Text> {
    let context = Context::create();
    let module = context.create_module("spawn_pack_producer");
    let int = context.i64_type();
    let params: List<_> = (0..count).map(|_| int.into()).collect();
    module.add_function("callee", int.fn_type(params.as_slice(), false), None);
    let entry = module.add_function("probe", int.fn_type(&[], false), None);
    let mut vbc = VbcModule::new("spawn_pack_producer".into());
    let callee = FunctionDescriptor::new(vbc.intern_string("callee"));
    let fid = vbc.add_function(callee);
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, entry, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(entry, "entry"));
    for i in 0..count {
        ctx.set_register(i as u16, int.const_int(i as u64 + 37, false).into());
    }
    lower_instruction(
        &mut ctx,
        &Instruction::Spawn {
            dst: Reg(count as u16),
            func_id: fid.0,
            args: RegRange::new(Reg(0), count),
        },
    )
    .map_err(|error| Text::from(error.to_string()))?;
    ctx.builder()
        .build_return(Some(&ctx.get_register(count as u16).unwrap()))
        .unwrap();
    module.verify().expect("producer IR verifies");
    Ok(entry.print_to_string().to_string().into())
}

#[test]
fn producer_allocates_only_declared_arguments() {
    for count in [2_u8, 8] {
        let ir = producer_ir(count).expect("supported arity");
        assert!(
            ir.contains(&format!("@verum_alloc(i64 {})", (u64::from(count) + 2) * 8)),
            "{ir}"
        );
        assert_eq!(
            ir.as_str()
                .lines()
                .filter(|line| line.trim().starts_with("store i64 "))
                .count(),
            count as usize + 2
        );
        assert!(ir.contains("@verum_pool_global_submit"));
        assert!(ir.contains("@verum_thread_spawn_multi"));
    }
}

#[test]
fn zero_and_one_argument_fast_paths_do_not_allocate_packs() {
    for count in [0, 1] {
        let ir = producer_ir(count).expect("existing fast path");
        assert!(ir.contains("@verum_pool_global_submit"));
        assert!(!ir.contains("@verum_alloc"));
        assert!(!ir.contains("@verum_thread_spawn_multi"));
    }
}

#[test]
fn producer_rejects_unsupported_arity_before_emitting_a_pack() {
    let error = producer_ir(9).expect_err("nine arguments must not silently call with two");
    assert!(
        error.contains("Spawn") && error.contains("9") && error.contains("8"),
        "{error}"
    );
}

#[test]
fn consumer_loads_arguments_only_in_the_selected_arity_block() {
    let context = Context::create();
    let module = context.create_module("spawn_pack_consumer");
    PlatformIR::new(&context)
        .emit_thread_spawn_multi_ir(&module)
        .unwrap();
    module.verify().expect("consumer IR verifies");
    let function = module.get_function("verum_thread_spawn_multi").unwrap();
    for block in function.get_basic_blocks() {
        let name = block.get_name().to_str().unwrap();
        let loads = block
            .get_instructions()
            .filter(|instruction| {
                instruction.get_opcode() == verum_llvm::values::InstructionOpcode::Load
            })
            .count();
        if name == "entry" {
            assert_eq!(
                loads, 2,
                "entry may read only fn and count, not argument slots"
            );
        } else if let Some(arity) = name.strip_prefix("call_") {
            assert_eq!(
                loads,
                arity.parse::<usize>().unwrap(),
                "{name}: load exactly count arguments"
            );
        }
    }
    let ir = function.print_to_string().to_string();
    assert!(
        ir.contains("@verum_panic"),
        "unsupported count must fail explicitly"
    );
    assert!(ir.contains("unreachable"));
}

// Host-only protected-memory control. Production does not acquire a libc
// dependency: these macOS test symbols are provided by required libSystem.
#[cfg(target_os = "macos")]
mod protected {
    use super::*;
    use std::cell::Cell;
    use verum_llvm::{
        OptimizationLevel,
        targets::{InitializationConfig, Target},
    };
    thread_local! {
        static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        static DEALLOCATED_BYTES: Cell<usize> = const { Cell::new(0) };
    }
    unsafe extern "C" {
        fn getpagesize() -> i32;
        fn mmap(
            address: *mut u8,
            length: usize,
            protection: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut u8;
        fn mprotect(address: *mut u8, length: usize, protection: i32) -> i32;
        fn munmap(address: *mut u8, length: usize) -> i32;
    }
    unsafe extern "C" fn release_pack(address: *mut u8, size: u64) {
        DEALLOCATIONS.with(|cell| cell.set(cell.get() + 1));
        DEALLOCATED_BYTES.with(|cell| cell.set(size as usize));
        // Model invalidation at free: every argument must have been loaded
        // before releasing storage, while the actual mapping stays owned here.
        unsafe {
            std::ptr::write_bytes(address, 0, size as usize);
        }
    }
    unsafe extern "C" fn report_bad_arity(message: *const u8, length: u64, _: *const u8, _: u32) {
        let bytes = unsafe { std::slice::from_raw_parts(message, length as usize) };
        let message = std::str::from_utf8(bytes).unwrap_or("");
        std::process::exit(if message.contains("spawn") && message.contains("arity") {
            73
        } else {
            74
        });
    }
    unsafe extern "C" fn sum2(a: u64, b: u64) -> u64 {
        a + b
    }
    unsafe extern "C" fn sum3(a: u64, b: u64, c: u64) -> u64 {
        a + b + c
    }
    unsafe extern "C" fn sum4(a: u64, b: u64, c: u64, d: u64) -> u64 {
        a + b + c + d
    }
    unsafe extern "C" fn sum5(a: u64, b: u64, c: u64, d: u64, e: u64) -> u64 {
        a + b + c + d + e
    }
    unsafe extern "C" fn sum6(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> u64 {
        a + b + c + d + e + f
    }
    unsafe extern "C" fn sum7(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64) -> u64 {
        a + b + c + d + e + f + g
    }
    unsafe extern "C" fn sum8(
        a: u64,
        b: u64,
        c: u64,
        d: u64,
        e: u64,
        f: u64,
        g: u64,
        h: u64,
    ) -> u64 {
        a + b + c + d + e + f + g + h
    }

    fn child(count: usize) {
        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let context = Context::create();
        let module = context.create_module("guarded_spawn_pack");
        PlatformIR::new(&context)
            .emit_thread_spawn_multi_ir(&module)
            .unwrap();
        module.verify().expect("JIT IR verifies");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(
            &module.get_function("verum_dealloc").unwrap(),
            release_pack as *const () as usize,
        );
        if let Some(panic) = module.get_function("verum_panic") {
            engine.add_global_mapping(&panic, report_bad_arity as *const () as usize);
        }
        let valid = (2..=8).contains(&count);
        let bytes = if valid { (count + 2) * 8 } else { 16 };
        // SAFETY: a two-page mapping owns the valid pack; the immediately
        // following page is PROT_NONE. Any extra argument read must fault.
        unsafe {
            let page = getpagesize() as usize;
            let mapping = mmap(std::ptr::null_mut(), page * 2, 3, 0x1002, -1, 0);
            assert_ne!(mapping as usize, usize::MAX);
            assert_eq!(mprotect(mapping.add(page), page, 0), 0);
            let pack = mapping.add(page - bytes).cast::<u64>();
            let target = match count {
                3 => sum3 as *const (),
                4 => sum4 as *const (),
                5 => sum5 as *const (),
                6 => sum6 as *const (),
                7 => sum7 as *const (),
                8 => sum8 as *const (),
                _ => sum2 as *const (),
            };
            pack.write(target as u64);
            pack.add(1).write(count as u64);
            if valid {
                for i in 0..count {
                    pack.add(i + 2).write(37 + i as u64);
                }
            }
            let result = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("verum_thread_spawn_multi")
                .unwrap()
                .call(pack as u64);
            assert!(valid, "unsupported arity unexpectedly returned {result}");
            assert_eq!(result, (0..count).map(|i| 37 + i as u64).sum::<u64>());
            assert_eq!(DEALLOCATIONS.with(Cell::get), 1);
            assert_eq!(DEALLOCATED_BYTES.with(Cell::get), bytes);
            assert_eq!(munmap(mapping, page * 2), 0);
        }
    }

    #[test]
    fn protected_pack_child() {
        if let Ok(count) = std::env::var("VERUM_SPAWN_PACK_CHILD") {
            child(count.parse().unwrap());
        }
    }
    fn run_child(count: usize, expected: i32) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "llvm::platform_ir::spawn_pack_bounds_tests::protected::protected_pack_child",
                "--nocapture",
            ])
            .env("VERUM_SPAWN_PACK_CHILD", count.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected),
            "count={count}, status={}, stdout={}, stderr={}",
            output.status,
            Text::from(std::str::from_utf8(&output.stdout).unwrap_or("non-UTF8 stdout")),
            Text::from(std::str::from_utf8(&output.stderr).unwrap_or("non-UTF8 stderr"))
        );
    }
    #[test]
    fn all_supported_argument_packs_stop_at_the_guard_page() {
        for count in 2..=8 {
            run_child(count, 0);
        }
    }
    #[test]
    fn unsupported_packs_panic_before_reading_any_arguments() {
        for count in [0, 1, 9] {
            run_child(count, 73);
        }
    }
}
