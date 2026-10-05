//! T1591: real generated writer logic; only Windows OS entry points are mocked.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    module::Linkage,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

fn with_module(target: Option<&str>, check: impl FnOnce(&Context, &verum_llvm::module::Module)) {
    let ast = Parser::new("fn trigger() { print(1.25); }")
        .parse_module()
        .unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    let context = Context::create();
    let mut config = LoweringConfig::debug("writer").with_debug_info(false);
    if let Some(target) = target {
        config = config.with_target(target);
    }
    let mut lower = VbcToLlvmLowering::new(&context, config);
    lower.lower_module(&vbc).unwrap();
    check(&context, lower.module());
}

fn reachable(module: &verum_llvm::module::Module, roots: &[&str]) -> Text {
    let mut output = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().unwrap().lines() {
        if line.starts_with("attributes #") {
            output.push_str(line);
            output.push('\n');
        }
    }
    let mut pending: List<Text> = roots.iter().map(|name| Text::from(*name)).collect();
    let mut seen = Set::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let text = if let Some(function) = module.get_function(&name) {
            function.print_to_string()
        } else {
            module.get_global(&name).unwrap().print_to_string()
        };
        let text = text.to_str().unwrap();
        output.push_str(text);
        output.push('\n');
        for tail in text.split('@').skip(1) {
            let name = tail
                .split(|c: char| !c.is_ascii_alphanumeric() && !"_.$".contains(c))
                .next()
                .unwrap();
            if module.get_function(name).is_some() || module.get_global(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    output
}

#[derive(Default)]
struct WindowsState {
    handle: usize,
    selected: List<u32>,
    writes: List<(usize, usize, u32)>,
    results: List<(i32, u32)>,
}
thread_local! { static WINDOWS: RefCell<WindowsState> = RefCell::new(WindowsState::default()); }
extern "C" fn get_std_handle(kind: u32) -> *mut u8 {
    WINDOWS.with(|state| {
        let mut state = state.borrow_mut();
        state.selected.push(kind);
        state.handle as *mut u8
    })
}
extern "C" fn write_file(
    handle: *mut u8,
    bytes: *const u8,
    count: u32,
    written: *mut u32,
    overlapped: *mut u8,
) -> i32 {
    WINDOWS.with(|state| {
        let mut state = state.borrow_mut();
        let index = state.writes.len();
        state.writes.push((handle as usize, bytes as usize, count));
        let (ok, amount) = state.results.get(index).copied().unwrap_or((1, count));
        // SAFETY: the emitted synchronous wrapper passes a live i32 output
        // slot; the mock never reads the possibly very large logical buffer.
        unsafe {
            written.write(amount);
        }
        if !overlapped.is_null() {
            return 0;
        }
        ok
    })
}
fn reset(handle: usize, results: &[(i32, u32)]) {
    WINDOWS.with(|state| {
        *state.borrow_mut() = WindowsState {
            handle,
            results: results.iter().copied().collect(),
            ..WindowsState::default()
        }
    });
}

#[test]
fn windows_writer_is_common_and_reports_only_actual_progress() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    with_module(Some("aarch64-pc-windows-msvc"), |context, full| {
        let platform = full
            .get_function("verum_os_write")
            .unwrap()
            .print_to_string();
        assert!(
            platform
                .to_str()
                .unwrap()
                .contains("@verum_internal_write("),
            "platform must share the writer: {platform}"
        );
        for api in ["GetStdHandle", "WriteFile"] {
            assert_eq!(
                full.get_function(api).unwrap().count_basic_blocks(),
                0,
                "platform import must survive bodyless-stub pass"
            );
        }
        let ir = reachable(
            full,
            &[
                "verum_internal_write",
                "verum_os_write",
                "verum_internal_write_all",
                "verum_internal_puts",
            ],
        );
        assert!(!ir.contains("@write("), "{ir}");
        assert!(ir.contains("@WriteFile("), "{ir}");
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "windows_writer_logic",
            ))
            .unwrap();
        module.verify().unwrap();
        // The test host does not load Windows DLL import-address tables.
        // Adapt only the two mocked OS declarations, not the generated body.
        for api in ["GetStdHandle", "WriteFile"] {
            module
                .get_function(api)
                .unwrap()
                .as_global_value()
                .set_dll_storage_class(verum_llvm::DLLStorageClass::Default);
        }
        // Retarget only the isolated platform-neutral control-flow helper to
        // this test host. Windows execution/linking is not claimed by this JIT.
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(
            &module.get_function("GetStdHandle").unwrap(),
            get_std_handle as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("WriteFile").unwrap(),
            write_file as *const () as usize,
        );
        // SAFETY: exact emitted ABI, live engine. OS mocks only record byte
        // pointers/counts and write the local result slot, without dereferencing
        // the logical multi-gigabyte buffer used to test DWORD chunking.
        unsafe {
            let write = engine
                .get_function::<unsafe extern "C" fn(i64, *const u8, u64) -> i64>(
                    "verum_internal_write",
                )
                .unwrap();
            let os_write = engine
                .get_function::<unsafe extern "C" fn(i64, *const u8, u64) -> i64>("verum_os_write")
                .unwrap();
            let write_all = engine
                .get_function::<unsafe extern "C" fn(i64, *const u8, u64) -> i64>(
                    "verum_internal_write_all",
                )
                .unwrap();
            let puts = engine
                .get_function::<unsafe extern "C" fn(*const u8) -> i32>("verum_internal_puts")
                .unwrap();
            let buffer = 0x10000_usize as *const u8;
            for (fd, selector) in [(0, -10_i32), (1, -11), (2, -12)] {
                reset(0x4000, &[]);
                assert_eq!(write.call(fd, buffer, 7), 7);
                WINDOWS.with(|s| {
                    let s = s.borrow();
                    assert_eq!(s.selected.as_slice(), &[selector as u32]);
                    assert_eq!(s.writes.as_slice(), &[(0x4000, 0x10000, 7)]);
                });
            }
            for handle in [0, usize::MAX] {
                reset(handle, &[]);
                assert_eq!(write.call(1, buffer, 7), -1);
                WINDOWS.with(|s| assert!(s.borrow().writes.is_empty()));
            }
            reset(0x4000, &[]);
            assert_eq!(write.call(9, buffer, 7), -1);
            WINDOWS
                .with(|s| assert!(s.borrow().selected.is_empty() && s.borrow().writes.is_empty()));
            reset(0x4000, &[]);
            assert_eq!(write.call(1, buffer, 0), 0);
            WINDOWS.with(|s| assert!(s.borrow().writes.is_empty()));
            for (result, expected) in [((0, 7), -1), ((1, 0), 0), ((1, 3), 3)] {
                reset(0x4000, &[result]);
                assert_eq!(os_write.call(2, buffer, 7), expected);
            }
            let chunk = u32::MAX;
            reset(0x4000, &[]);
            assert_eq!(
                write_all.call(1, buffer, u64::from(chunk) + 5),
                i64::from(chunk) + 5
            );
            WINDOWS.with(|s| {
                assert_eq!(
                    s.borrow().writes.as_slice(),
                    &[
                        (0x4000, 0x10000, chunk),
                        (0x4000, 0x10000 + chunk as usize, 5)
                    ]
                )
            });
            reset(0x4000, &[(1, chunk), (0, 0)]);
            assert_eq!(write_all.call(1, buffer, u64::from(chunk) + 5), -1);
            // A single low-level write clamps, while puts retries short
            // progress through the common write-all before emitting newline.
            reset(0x4000, &[]);
            assert_eq!(
                write.call(1, buffer, u64::from(chunk) + 5),
                i64::from(chunk)
            );
            let text = b"1234567\0";
            reset(0x4000, &[(1, 3), (1, 4), (1, 1)]);
            assert_eq!(puts.call(text.as_ptr()), 0);
            WINDOWS.with(|s| {
                let state = s.borrow();
                assert_eq!(state.writes.len(), 3);
                assert_eq!(state.writes[0], (0x4000, text.as_ptr() as usize, 7));
                assert_eq!(state.writes[1], (0x4000, text.as_ptr() as usize + 3, 4));
                assert_eq!(state.writes[2].2, 1);
            });
            for results in [
                &[(1, 0)][..],
                &[(0, 0)][..],
                &[(1, 3), (0, 0)][..],
                &[(1, 8)][..],
                &[(1, 7), (0, 0)][..],
            ] {
                reset(0x4000, results);
                assert_eq!(puts.call(text.as_ptr()), -1);
            }
            reset(0x4000, &[]);
            assert_eq!(puts.call(b"\0".as_ptr()), 0);
            WINDOWS.with(|s| assert_eq!(s.borrow().writes.len(), 1));
            reset(0x4000, &[]);
            assert_eq!(write.call(1, buffer, u64::MAX), -1);
            WINDOWS.with(|s| assert!(s.borrow().writes.is_empty()));
        }
    });
}

#[test]
fn optimized_common_strlen_keeps_its_own_byte_scan() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    with_module(None, |context, full| {
        let ir = reachable(full, &["verum_internal_strlen"]);
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "strlen_logic",
            ))
            .unwrap();
        module
            .get_function("verum_internal_strlen")
            .unwrap()
            .set_linkage(Linkage::External);
        let triple = verum_llvm::targets::TargetMachine::get_default_triple();
        let machine = Target::from_triple(&triple)
            .unwrap()
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::Aggressive,
                verum_llvm::targets::RelocMode::PIC,
                verum_llvm::targets::CodeModel::Default,
            )
            .unwrap();
        module.set_triple(&triple);
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        module
            .run_passes(
                "default<O2>",
                &machine,
                verum_llvm::passes::PassBuilderOptions::create(),
            )
            .unwrap();
        assert!(
            module.get_function("strlen").is_none(),
            "{}",
            module.print_to_string()
        );
        module.verify().unwrap();
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        // SAFETY: all inputs are live NUL-terminated byte arrays; function ABI
        // is exactly ptr→i64 and the engine stays alive through every call.
        unsafe {
            let len = engine
                .get_function::<unsafe extern "C" fn(*const u8) -> u64>("verum_internal_strlen")
                .unwrap();
            assert_eq!(len.call(b"\0".as_ptr()), 0);
            assert_eq!(len.call(b"abc\0after".as_ptr()), 3);
            assert_eq!(len.call(b"\xff\x80\x01\0".as_ptr()), 3);
            let mut long = [b'x'; 329];
            long[328] = 0;
            assert_eq!(len.call(long.as_ptr()), 328);
        }
    });
}
