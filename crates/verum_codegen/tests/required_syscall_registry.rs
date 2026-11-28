//! Required native declarations fail at the registry boundary (T2781).
#[cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

use verum_codegen::llvm::error::LlvmLoweringError;
use verum_codegen::llvm::runtime::RuntimeLowering;
use verum_codegen::llvm::syscall_registry::{
    canonical_fn_type, get_or_declare, lookup_sig, predeclare_all, verum_wrapper_name,
};
use verum_codegen::llvm::vbc_lowering::{LoweringConfig, VbcToLlvmLowering};
use verum_llvm::context::Context;
use verum_llvm::targets::TargetTriple;
use verum_llvm::values::AnyValue;
use verum_vbc::module::VbcModule;

fn assert_registry_error(error: LlvmLoweringError, name: &str, reason: &str) {
    let LlvmLoweringError::Internal(message) = error else {
        panic!("expected registry diagnostic for {name}, got {error}");
    };
    assert!(message.contains("syscall_registry"), "{message}");
    assert!(message.contains(name), "{message}");
    assert!(message.contains(reason), "{message}");
}

#[test]
fn missing_required_symbol_does_not_create_a_declaration() {
    let context = Context::create();
    let module = context.create_module("missing_registry_entry");
    let name = "not_a_registered_platform_operation";
    assert!(lookup_sig(name).is_none());
    assert_registry_error(
        get_or_declare(&module, &context, name).unwrap_err(),
        name,
        "missing",
    );
    assert!(module.get_function(name).is_none());
    assert_eq!(module.get_functions().count(), 0);
}

#[test]
fn a_foreign_same_name_body_does_not_create_registry_membership() {
    let context = Context::create();
    let module = context.create_module("foreign_registry_entry");
    let name = "not_a_registered_platform_operation";
    let foreign = module.add_function(name, context.i64_type().fn_type(&[], false), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(foreign, "entry"));
    builder
        .build_return(Some(&context.i64_type().const_int(73, false)))
        .unwrap();
    let before = module.print_to_string().to_string();

    assert_registry_error(
        get_or_declare(&module, &context, name).unwrap_err(),
        name,
        "missing",
    );
    assert_eq!(module.get_function(name), Some(foreign));
    assert_eq!(module.print_to_string().to_string(), before);
    module.verify().unwrap();
}

#[test]
fn incompatible_registered_symbols_never_escape_as_canonical_handles() {
    for has_body in [false, true] {
        let context = Context::create();
        let module = context.create_module("conflicting_registry_abi");
        let name = "clock_gettime";
        let foreign = module.add_function(name, context.i32_type().fn_type(&[], false), None);
        if has_body {
            let builder = context.create_builder();
            builder.position_at_end(context.append_basic_block(foreign, "entry"));
            builder
                .build_return(Some(&context.i32_type().const_int(73, false)))
                .unwrap();
        }
        let before = module.print_to_string().to_string();
        assert_registry_error(
            get_or_declare(&module, &context, name).unwrap_err(),
            name,
            "ABI mismatch",
        );
        assert_eq!(module.get_function(name), Some(foreign));
        assert_eq!(module.print_to_string().to_string(), before);
        module.verify().unwrap();
    }
}

#[test]
fn registry_abi_checks_include_arguments_and_variadicness() {
    let context = Context::create();
    let ptr = context.ptr_type(Default::default());
    let i64_type = context.i64_type();
    for wrong_type in [
        i64_type.fn_type(&[ptr.into(), ptr.into()], false),
        i64_type.fn_type(&[i64_type.into(), ptr.into()], true),
    ] {
        let module = context.create_module("conflicting_registry_parameter_abi");
        let existing = module.add_function("clock_gettime", wrong_type, None);
        assert_registry_error(
            get_or_declare(&module, &context, "clock_gettime").unwrap_err(),
            "clock_gettime",
            "ABI mismatch",
        );
        assert_eq!(module.get_function("clock_gettime"), Some(existing));
        assert_eq!(module.get_functions().count(), 1);
    }
}

#[test]
fn canonical_declarations_keep_their_types_and_handles() {
    let context = Context::create();
    let module = context.create_module("canonical_registry_abi");
    let i64_type = context.i64_type();
    let ptr = context.ptr_type(Default::default());
    for (name, expected) in [
        (
            "clock_gettime",
            i64_type.fn_type(&[i64_type.into(), ptr.into()], false),
        ),
        (
            "nanosleep",
            i64_type.fn_type(&[ptr.into(), ptr.into()], false),
        ),
        ("sched_yield", i64_type.fn_type(&[], false)),
        (
            "fcntl",
            context.i32_type().fn_type(&[context.i32_type().into(), context.i32_type().into()], true),
        ),
    ] {
        let first = get_or_declare(&module, &context, name).unwrap();
        assert_eq!(first.get_type(), expected);
        assert_eq!(get_or_declare(&module, &context, name).unwrap(), first);
    }
    predeclare_all(&module, &context).unwrap();
    let count = module.get_functions().count();
    predeclare_all(&module, &context).unwrap();
    assert_eq!(module.get_functions().count(), count);
    for function in module.get_functions() {
        let name = function.get_name().to_str().unwrap();
        assert_eq!(
            function.get_type(),
            canonical_fn_type(&context, lookup_sig(name).unwrap())
        );
        assert_eq!(get_or_declare(&module, &context, name).unwrap(), function);
    }
    module.verify().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn darwin_fcntl_uses_the_variadic_argument_and_returns_signed_errors() {
    let module = source_image::module(r#"
        extern "C" {
            fn fcntl(fd: Int32, command: Int32, ...) -> Int32;
            fn close(fd: Int32) -> Int32;
        }
        fn main() -> Int {
            unsafe {
                let duplicate = fcntl(1 as Int32, 0 as Int32, 79);
                if duplicate < 79 { return 1; }
                if close(duplicate) != 0 { return 2; }
                if fcntl(duplicate, 1 as Int32) != -1 { return 3; }
            }
            73
        }
    "#);
    source_image::run_module(module, |_| {});
}

#[test]
fn public_lowering_stops_at_an_incompatible_registry_predeclaration() {
    let context = Context::create();
    let config =
        LoweringConfig::new("required_registry_boundary").with_target("aarch64-apple-darwin");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    let wrong = lowering.module().add_function(
        "clock_gettime",
        context.i32_type().fn_type(&[], false),
        None,
    );
    let module = VbcModule::new("required_registry_boundary".into());
    assert_registry_error(
        lowering.lower_module(&module).unwrap_err(),
        "clock_gettime",
        "ABI mismatch",
    );
    assert_eq!(lowering.module().get_function("clock_gettime"), Some(wrong));
    assert_eq!(lowering.module().get_functions().count(), 1);
}

#[test]
fn a_missing_required_wrapper_cannot_fall_back_to_a_direct_symbol() {
    assert!(
        lookup_sig("read").is_some(),
        "registered syscall without a time wrapper"
    );
    for name in ["read", "not_a_registered_platform_operation"] {
        assert_registry_error(
            verum_wrapper_name(name).unwrap_err(),
            name,
            "required wrapper",
        );
    }
}

#[test]
fn actual_time_wrappers_preserve_the_declared_target_boundary() {
    for (triple, linux_instruction) in [
        ("x86_64-unknown-linux-gnu", Some("syscall")),
        ("aarch64-unknown-linux-gnu", Some("svc #0")),
        ("x86_64-apple-darwin", None),
        ("aarch64-apple-darwin", None),
    ] {
        let context = Context::create();
        let module = context.create_module("required_registry_wrappers");
        module.set_triple(&TargetTriple::create(triple));
        let runtime = RuntimeLowering::new(&context);
        let wrappers = [
            runtime.emit_verum_clock_gettime_wrapper(&module).unwrap(),
            runtime.emit_verum_nanosleep_wrapper(&module).unwrap(),
            runtime.emit_verum_sched_yield_wrapper(&module).unwrap(),
        ];
        let repeated = [
            runtime.emit_verum_clock_gettime_wrapper(&module).unwrap(),
            runtime.emit_verum_nanosleep_wrapper(&module).unwrap(),
            runtime.emit_verum_sched_yield_wrapper(&module).unwrap(),
        ];
        assert_eq!(wrappers, repeated);
        for (wrapper, (symbol, expected_name)) in wrappers.into_iter().zip([
            ("clock_gettime", "__verum_clock_gettime"),
            ("nanosleep", "__verum_nanosleep"),
            ("sched_yield", "__verum_sched_yield"),
        ]) {
            assert_eq!(verum_wrapper_name(symbol).unwrap(), expected_name);
            assert_eq!(wrapper.get_name().to_str().unwrap(), expected_name);
            assert_eq!(
                wrapper.get_type(),
                canonical_fn_type(&context, lookup_sig(symbol).unwrap())
            );
            let ir = wrapper.print_to_string().to_string();
            if let Some(instruction) = linux_instruction {
                assert!(ir.contains(instruction), "{triple}: {ir}");
                assert!(
                    module.get_function(symbol).is_none(),
                    "{triple}: unexpected external {symbol}"
                );
            } else {
                let external = module.get_function(symbol).unwrap();
                assert_eq!(external.get_type(), wrapper.get_type());
                assert_eq!(external.count_basic_blocks(), 0);
                assert!(
                    ir.contains(&format!("call i64 @{symbol}(")),
                    "{triple}: {ir}"
                );
            }
        }
        module.verify().unwrap();
    }
}

#[test]
fn actual_wrappers_propagate_a_conflicting_required_syscall() {
    for symbol in ["clock_gettime", "nanosleep", "sched_yield"] {
        let context = Context::create();
        let module = context.create_module("required_registry_wrapper_conflict");
        module.set_triple(&TargetTriple::create("aarch64-apple-darwin"));
        let foreign = module.add_function(symbol, context.i32_type().fn_type(&[], false), None);
        let runtime = RuntimeLowering::new(&context);
        let result = match symbol {
            "clock_gettime" => runtime.emit_verum_clock_gettime_wrapper(&module),
            "nanosleep" => runtime.emit_verum_nanosleep_wrapper(&module),
            "sched_yield" => runtime.emit_verum_sched_yield_wrapper(&module),
            _ => unreachable!(),
        };
        assert_registry_error(result.unwrap_err(), symbol, "ABI mismatch");
        assert_eq!(module.get_function(symbol), Some(foreign));
    }
}

#[test]
fn actual_wrapper_reuse_checks_abi_before_accepting_an_existing_body() {
    for symbol in ["clock_gettime", "nanosleep", "sched_yield"] {
        for has_body in [true, false] {
            let context = Context::create();
            let module = context.create_module("required_registry_wrapper_reuse");
            module.set_triple(&TargetTriple::create("aarch64-apple-darwin"));
            let name = verum_wrapper_name(symbol).unwrap();
            let foreign = module.add_function(name, context.i32_type().fn_type(&[], false), None);
            if has_body {
                let builder = context.create_builder();
                builder.position_at_end(context.append_basic_block(foreign, "entry"));
                builder
                    .build_return(Some(&context.i32_type().const_int(73, false)))
                    .unwrap();
            }
            let before = module.print_to_string().to_string();
            let runtime = RuntimeLowering::new(&context);
            let result = match symbol {
                "clock_gettime" => runtime.emit_verum_clock_gettime_wrapper(&module),
                "nanosleep" => runtime.emit_verum_nanosleep_wrapper(&module),
                "sched_yield" => runtime.emit_verum_sched_yield_wrapper(&module),
                _ => unreachable!(),
            };
            assert_registry_error(result.unwrap_err(), name, "ABI mismatch");
            assert_eq!(module.get_function(name), Some(foreign));
            assert_eq!(module.print_to_string().to_string(), before);
            module.verify().unwrap();
        }
    }
}

#[test]
fn platform_socket_reuse_checks_the_registered_abi_before_accepting_a_body() {
    use verum_codegen::llvm::platform_ir::PlatformIR;
    let context = Context::create();
    let module = context.create_module("required_registry_socket_reuse");
    module.set_triple(&TargetTriple::create("aarch64-apple-darwin"));
    let foreign = module.add_function("socket", context.i32_type().fn_type(&[], false), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(foreign, "entry"));
    builder
        .build_return(Some(&context.i32_type().const_int(73, false)))
        .unwrap();
    verum_codegen::llvm::runtime::define_text_ir_helpers(&context, &module).unwrap();
    verum_codegen::llvm::runtime::define_list_ir_helpers(&context, &module).unwrap();
    verum_codegen::llvm::runtime::define_map_set_ir_helpers(&context, &module).unwrap();
    let runtime = RuntimeLowering::new(&context);
    runtime.emit_text_ir_functions(&module).unwrap();
    runtime.emit_misc_ir_functions(&module).unwrap();
    let platform = PlatformIR::new(&context);
    let error = platform.emit_platform_functions(&module).unwrap_err();
    assert_registry_error(error, "socket", "ABI mismatch");
    assert_eq!(module.get_function("socket"), Some(foreign));
}
