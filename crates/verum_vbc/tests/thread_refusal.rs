//! Native thread creation is unavailable until the interpreter owns isolated
//! concurrent execution. Source and wire routes must refuse before callbacks.
#![cfg(all(feature = "codegen", feature = "ffi"))]

// Interpreter::new owns the existing Arc<VbcModule> API boundary.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    instruction::{Instruction, SystemSubOpcode},
    interpreter::{Interpreter, InterpreterError},
    module::{CallingConvention, VbcModule},
};

fn routes(source: &str) -> [VbcModule; 2] {
    let ast = Parser::new(source)
        .parse_module()
        .expect("actual source grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("ordinary foreign declaration lowering");
    let probe = module.find_function_by_name("probe").unwrap();
    let function = module.get_function(probe).unwrap();
    let ops = verum_vbc::bytecode::decode_instructions(
        &module.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize],
    )
    .unwrap();
    assert!(
        ops.iter().any(|op| matches!(
            op,
            Instruction::FfiExtended { sub_op, .. }
                if *sub_op == SystemSubOpcode::CallFfiC as u8
        )),
        "must exercise actual foreign call: {ops:?}"
    );
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    [
        module,
        verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
    ]
}

fn assert_refusal(source: &str, operation: &str) {
    assert_modules_refuse(routes(source), operation);
}

fn assert_modules_refuse(modules: [VbcModule; 2], operation: &str) {
    for module in modules {
        assert_eq!(
            module.get_string(module.ffi_symbols[0].name),
            Some(operation)
        );
        let probe = module.find_function_by_name("probe").unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(probe);
        assert!(
            matches!(&result, Err(InterpreterError::UnsupportedRuntimeOperation { operation: actual, reason })
                if actual.as_str() == operation
                    && reason.as_str() == "native threads require isolated concurrent VM execution"),
            "must refuse before callback or synthetic success: {result:?}"
        );
    }
}

#[test]
fn pthread_create_does_not_run_child_before_returning_to_parent() {
    assert_refusal(
        r#"
        extern "C" {
            @link_name("pthread_create") fn start(handle: &mut Int, attr: Int, entry: fn(Int) -> Int, arg: Int) -> Int;
        }
        fn child(arg: Int) -> Int { panic("child executed eagerly before parent can release its lock"); 0 }
        fn probe() -> Int { let mut handle = 0; start(&mut handle, 0, child, 7) }
    "#,
        "pthread_create",
    );
}

#[test]
fn unknown_thread_join_is_not_synthetic_success() {
    assert_refusal(
        r#"
        extern "C" { @link_name("pthread_join") fn collect(handle: Int, output: &mut Int) -> Int; }
        fn probe() -> Int { let mut output = 913; collect(999999, &mut output) }
    "#,
        "pthread_join",
    );
}

#[test]
fn unknown_thread_detach_is_not_synthetic_success() {
    assert_refusal(
        r#"
        extern "C" { @link_name("pthread_detach") fn release(handle: Int) -> Int; }
        fn probe() -> Int { release(999999) }
    "#,
        "pthread_detach",
    );
}

#[test]
fn ordinary_source_names_do_not_select_foreign_thread_semantics() {
    let ast = Parser::new(
        r#"
        fn pthread_create() -> Int { 19 }
        fn pthread_join() -> Int { 23 }
        fn pthread_detach() -> Int { 29 }
        fn probe() -> Int { pthread_create() + pthread_join() + pthread_detach() }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    assert!(module.ffi_symbols.is_empty());
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
    ] {
        let probe = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(probe)
                .unwrap()
                .as_i64(),
            71
        );
    }
}

#[cfg(unix)]
#[test]
fn ordinary_foreign_call_preserves_real_native_result() {
    for module in routes(
        r#"
        extern "C" { fn getpid() -> Int32; }
        fn probe() -> Int { getpid() as Int }
    "#,
    ) {
        let probe = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(probe)
                .unwrap()
                .as_i64(),
            std::process::id() as i64
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn raw_clone_syscalls_refuse_before_resuming_the_current_vm_on_another_stack() {
    for (number, operation) in [(libc::SYS_clone, "clone"), (libc::SYS_clone3, "clone3")] {
        let source = format!(r#"fn probe() -> Int {{ @intrinsic("syscall0", {number}) }}"#);
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
        ] {
            let probe = module.find_function_by_name("probe").unwrap();
            let result = Interpreter::new(Arc::new(module)).execute_function(probe);
            assert!(
                matches!(&result, Err(InterpreterError::UnsupportedRuntimeOperation { operation: actual, reason })
                    if actual.as_str() == operation
                        && reason.as_str() == "native threads require isolated concurrent VM execution"),
                "{result:?}"
            );
        }
    }
}

#[test]
fn foreign_calling_conventions_do_not_bypass_thread_refusal() {
    for (abi, convention) in [
        ("stdcall", CallingConvention::Stdcall),
        ("fastcall", CallingConvention::Fastcall),
        ("sysv64", CallingConvention::SysV64),
    ] {
        let source = format!(
            r#"extern "{abi}" {{ @link_name("pthread_detach") fn release(handle: Int) -> Int; }}
            fn probe() -> Int {{ release(999999) }}"#
        );
        let modules = routes(&source);
        for module in &modules {
            assert_eq!(module.ffi_symbols[0].convention, convention);
        }
        // Ordinary lowering carries this convention on the descriptor and
        // emits CallFfiC. This does not claim alternate-opcode coverage.
        assert_modules_refuse(modules, "pthread_detach");
    }
}

#[test]
fn variadic_foreign_declaration_does_not_bypass_thread_refusal() {
    let modules = routes(
        r#"extern "C" { @link_name("pthread_detach") fn release(handle: Int, ...) -> Int; }
        fn probe() -> Int { release(999999, 71) }"#,
    );
    for module in &modules {
        assert!(module.ffi_symbols[0].signature.is_variadic);
    }
    // The descriptor retains varargs; the actual source producer emits
    // CallFfiC. No rewritten bytecode is substituted for the source route.
    assert_modules_refuse(modules, "pthread_detach");
}
