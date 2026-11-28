//! Execute the public descriptor functions at their VBC entries and observe their OS results.
#![cfg(all(feature = "codegen", any(target_os = "linux", target_os = "macos")))]

#[path = "support/selected_stdlib.rs"]
mod selected_stdlib;

use selected_stdlib::source_unit;
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_vbc::{
    Value,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn process_descriptors_use_the_real_pipe_and_return_kernel_errors() {
    let sources = [
        ("mod.vr", ""),
        ("base/mod.vr", "Maybe Result"),
        (
            "base/result.vr",
            "Result Result<T,E>.is_ok Result<T,E>.is_err",
        ),
        ("base/maybe.vr", "Maybe"),
        ("io/process.vr", "close_fd"),
        ("text/text.vr", "Text Text.len Text.as_bytes"),
        ("intrinsics/runtime/text.vr", "text_as_bytes text_byte_len"),
        (
            "base/protocols.vr",
            "Display Formatter FormatError FormatSpec TextAlignment Sign",
        ),
        ("base/primitives.vr", "Int.min"),
        (
            "collections/list.vr",
            "List List<T>.new List<T>.with_capacity List<T>.push List<T>.len List<T>.as_slice",
        ),
        (
            "intrinsics/memory.vr",
            "null_ptr ptr_offset ptr_is_null list_storage_write slice_from_raw_parts",
        ),
        (
            "sys/common.vr",
            "FileDesc FileDesc.from_raw OSError OSError.new OSError.interrupted OSError.kind OSErrorKind OSError.from_code FileDesc.as_raw",
        ),
        #[cfg(target_os = "macos")]
        (
            "sys/darwin/libsystem.vr",
            "safe_read safe_write safe_close read write close kill pipe errno __error errno_to_message",
        ),
        #[cfg(target_os = "macos")]
        ("sys/darwin/errno.vr", "errno_name *const"),
        (
            "sys/process_native.vr",
            "make_pipe native_kill PIPE_CHUNK_SIZE native_fd_write_all native_fd_read_chunk",
        ),
        (
            "sys/error.vr",
            "OSErrorKind native_error_kind native_error_message",
        ),
        #[cfg(target_os = "linux")]
        ("sys/linux/errno.vr", "errno_name *const"),
        #[cfg(target_os = "linux")]
        (
            "sys/linux/arch.vr",
            "SYS_READ SYS_WRITE SYS_CLOSE SYS_PIPE SYS_PIPE2 SYS_KILL SYS_DUP3",
        ),
        #[cfg(target_os = "linux")]
        (
            "sys/linux/syscall.vr",
            "read write close pipe kill syscall_raw",
        ),
        #[cfg(target_os = "linux")]
        ("intrinsics/runtime/syscall.vr", "syscall6"),
        #[cfg(target_os = "linux")]
        ("intrinsics/mod.vr", "syscall6"),
    ];
    let units: verum_common::List<_> = sources
        .iter()
        .enumerate()
        .map(|(index, (path, names))| {
            let mut unit = source_unit(path, names, index);
            if *path == "sys/process_native.vr" {
                let fixture = FastParser::new()
                    .parse_module_str(
                        include_str!("fixtures/stdlib_process_descriptors.vr"),
                        FileId::new(1000),
                    )
                    .unwrap();
                unit.items.extend(fixture.items);
            }
            unit
        })
        .collect();
    let sources: verum_common::List<_> = units.iter().collect();
    let mut generator = VbcCodegen::with_config(CodegenConfig::new(""));
    generator.initialize();
    generator.collect_unit_declarations(&sources).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let module = std::sync::Arc::new(module);
        let mut vm = Interpreter::try_new_validated(module.clone()).unwrap();
        vm.run_global_ctors().unwrap();
        let mut call = |name: &str, args: &[Value]| {
            let name = if name.contains('.') {
                name.to_owned()
            } else {
                format!("core.sys.process_native.{name}")
            };
            let entry = module.find_function_by_name(&name).unwrap();
            vm.execute_function_with_args(entry, args).unwrap()
        };
        // Enter each declared body directly so a host call interceptor cannot supply the result.
        let descriptors = call("make_real_pipe", &[]).try_as_i64().unwrap();
        assert!(descriptors >= 0);
        let reader = Value::from_i64(descriptors / 65536);
        let writer = Value::from_i64(descriptors % 65536);
        let bytes = call("fixture_bytes", &[]);
        let written = call("native_fd_write_all", &[writer, bytes]);
        assert_eq!(call("wrote_four", &[written]).try_as_i64(), Some(1));
        call("core.io.process.close_fd", &[writer]);
        for (size, check) in [(3, "read_three"), (99, "read_one"), (99, "read_eof")] {
            let result = call("native_fd_read_chunk", &[reader, Value::from_i64(size)]);
            assert_eq!(call(check, &[result]).try_as_i64(), Some(1), "{check}");
        }
        assert_eq!(call("close_port", &[reader]).try_as_i64(), Some(1));
        let read = call("native_fd_read_chunk", &[reader, Value::from_i64(1)]);
        assert_eq!(call("read_error", &[read]).try_as_i64(), Some(1));
        let write = call("native_fd_write_all", &[writer, bytes]);
        assert_eq!(call("write_error", &[write]).try_as_i64(), Some(1));
        for (pid, check) in [(0, "signal_ok"), (i32::MAX as i64, "signal_error")] {
            let result = call("native_kill", &[Value::from_i64(pid), Value::from_i64(0)]);
            assert_eq!(call(check, &[result]).try_as_i64(), Some(1), "{check}");
        }
    }
}
