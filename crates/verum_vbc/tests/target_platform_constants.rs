#![cfg(feature = "codegen")]
//! Platform constants describe the compilation target, independent of the host.
use std::sync::Arc;
use verum_ast::{ItemKind, Module, cfg::TargetConfig};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn platform_constants() -> Module {
    let source = include_str!("../../../core/intrinsics/platform.vr");
    let mut module = Parser::new(source).parse_module().expect("platform grammar");
    // This test covers the six implemented target/build constant operations.
    // Capability queries and timing operations are separate runtime contracts.
    let mut count = 0;
    module.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Function(function) if matches!(function.name.name.as_str(),
            "is_debug" | "is_release" | "target_os" | "target_arch"
            | "target_pointer_width" | "target_is_little_endian") => {
                count += 1;
                true
            },
        _ => false,
    });
    assert_eq!(count, 6);
    module
}

#[test]
fn configured_target_constants_survive_vbc_serialization() {
    for (target, os, arch, bits, little) in [
        (TargetConfig::for_target("x86_64-unknown-linux-gnu"), 1, 1, 64, true),
        (TargetConfig::for_target("aarch64-apple-darwin"), 2, 2, 64, true),
        (TargetConfig::for_target("x86_64-pc-windows-msvc"), 3, 1, 64, true),
        (TargetConfig::for_target("riscv64-unknown-freebsd"), 5, 3, 64, true),
        (TargetConfig::wasm32_wasi(), 4, 4, 32, true),
        (TargetConfig::for_target("powerpc-unknown-linux-gnu"), 1, 0, 32, false),
        (TargetConfig::bare_metal("custom_arch", 24), 0, 0, 24, true),
    ] {
        for debug in [false, true] {
            let mut target = target.clone();
            target.debug_assertions = debug;
            let module = VbcCodegen::with_config(CodegenConfig::new("core").with_target(target.clone()))
                .compile_module(&platform_constants()).expect("compile target constants");
            let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
            for module in [module, restored] {
                let module = Arc::new(module);
                for (name, expected) in [("target_os", os), ("target_arch", arch), ("target_pointer_width", bits)] {
                    let entry = module.find_function_by_name(&format!("core.intrinsics.platform.{name}")).unwrap();
                    let actual = Interpreter::new(module.clone()).execute_function(entry).unwrap();
                    assert_eq!(actual.try_as_i64(), Some(expected), "{target:?}: {name}");
                }
                for (name, expected) in [("is_debug", debug), ("is_release", !debug), ("target_is_little_endian", little)] {
                    let entry = module.find_function_by_name(&format!("core.intrinsics.platform.{name}")).unwrap();
                    let actual = Interpreter::new(module.clone()).execute_function(entry).unwrap();
                    assert_eq!(actual.try_as_bool(), Some(expected), "{target:?}: {name}");
                }
            }
        }
    }
}

#[test]
fn malformed_target_geometry_is_not_replaced_with_host_defaults() {
    for (key, value, diagnostic) in [
        ("width", "", "positive bit width"),
        ("width", "0", "positive bit width"),
        ("width", "not-a-width", "positive bit width"),
        ("endian", "unknown", "unknown target byte order"),
    ] {
        let mut target = TargetConfig::host();
        if key == "width" { target.target_pointer_width = value.into(); }
        else { target.target_endian = value.into(); }
        let error = VbcCodegen::with_config(CodegenConfig::new("core").with_target(target))
            .compile_module(&platform_constants()).expect_err("invalid target accepted");
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}
