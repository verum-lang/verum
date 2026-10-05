//! The x64 MSVC Float ABI marker is owned by generated code, never CRT.
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    module::{Linkage, Module},
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

const WINDOWS: &str = "x86_64-pc-windows-msvc";

fn with_source(triple: &str, name: &str, check: impl FnOnce(&Context, &Module)) {
    let source = format!("fn {name}(value: Float) -> Float {{ value + 1.25 }}");
    let ast = Parser::new(&source).parse_module().unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("float_abi")
            .with_debug_info(false)
            .with_target(triple),
    );
    lower.lower_module(&vbc).unwrap();
    check(&context, lower.module());
}

fn source_object(name: &str, include_provider: bool, path: &Path) {
    with_source(WINDOWS, name, |context, module| {
        // Retain the actual emitted source body, not a hand-written Float IR
        // substitute. Unrelated standalone runtime helpers are outside this
        // closed numeric library's link test. The ABI marker is a backend
        // dependency, so its global/COMDAT must be retained explicitly here.
        let mut ir = Text::new();
        for line in module.print_to_string().to_str().unwrap().lines() {
            if line.starts_with("attributes #")
                || (include_provider && line.starts_with("$_fltused ="))
            {
                ir.push_str(line);
                ir.push('\n');
            }
        }
        if include_provider && let Some(global) = module.get_global("_fltused") {
            ir.push_str(global.print_to_string().to_str().unwrap());
            ir.push('\n');
        }
        let function = module.get_function(name).unwrap();
        function.set_linkage(Linkage::External);
        ir.push_str(function.print_to_string().to_str().unwrap());
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "source_float",
            ))
            .unwrap();
        let triple = TargetTriple::create(WINDOWS);
        let machine = Target::from_triple(&triple)
            .unwrap()
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::Aggressive,
                RelocMode::PIC,
                CodeModel::Default,
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
        module.verify().unwrap();
        let bytes = machine
            .write_to_memory_buffer(&module, FileType::Object)
            .unwrap();
        std::fs::write(path, bytes.as_slice()).unwrap();
    });
}

fn tool(name: &str) -> PathBuf {
    std::env::var_os("VERUM_LLVM_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join("bin").join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn link(objects: &[&Path], output: &Path) -> std::process::Output {
    let mut command = Command::new(tool("lld"));
    command.args([
        "-flavor",
        "link",
        "/dll",
        "/noentry",
        "/nodefaultlib",
        "/export:first",
    ]);
    if objects.len() == 2 {
        command.arg("/export:second");
    }
    command
        .arg(format!("/out:{}", output.display()))
        .args(objects)
        .output()
        .expect("bundled LLD must run")
}

#[test]
fn actual_float_source_links_without_crt_and_duplicate_providers() {
    Target::initialize_all(&InitializationConfig::default());
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.obj");
    let second = temp.path().join("second.obj");
    let missing = temp.path().join("missing.obj");
    source_object("first", true, &first);
    source_object("second", true, &second);
    source_object("first", false, &missing);
    let negative = link(&[&missing], &temp.path().join("missing.dll"));
    assert!(
        !negative.status.success(),
        "real source floating instructions need the marker"
    );
    assert!(Text::from_utf8_lossy(&negative.stderr).contains("_fltused"));
    eprintln!(
        "No provider control: {}",
        Text::from_utf8_lossy(&negative.stderr).trim()
    );
    for (name, objects) in [
        (
            "single.dll",
            [first.as_path()].into_iter().collect::<List<_>>(),
        ),
        (
            "merged.dll",
            [first.as_path(), second.as_path()]
                .into_iter()
                .collect::<List<_>>(),
        ),
    ] {
        let output = temp.path().join(name);
        let result = link(&objects, &output);
        assert!(
            result.status.success(),
            "no-CRT final link {name}: {}",
            Text::from_utf8_lossy(&result.stderr)
        );
        let imports = Command::new(tool("llvm-readobj"))
            .arg("--coff-imports")
            .arg(&output)
            .output()
            .unwrap();
        assert!(imports.status.success());
        let imports = Text::from_utf8_lossy(&imports.stdout);
        assert!(
            !imports.contains("Import {"),
            "closed numeric PE must have no DLL imports: {imports}"
        );
        eprintln!("{name}: /nodefaultlib final link PASS; no DLL imports");
        if let Some(dir) = std::env::var_os("VERUM_FLOAT_ABI_EVIDENCE") {
            std::fs::copy(&output, Path::new(&dir).join(name)).unwrap();
        }
    }
    if let Some(dir) = std::env::var_os("VERUM_FLOAT_ABI_EVIDENCE") {
        for path in [&first, &second, &missing] {
            std::fs::copy(path, Path::new(&dir).join(path.file_name().unwrap())).unwrap();
        }
    }
}

#[test]
fn provider_is_an_owned_x64_msvc_datum_and_not_a_cross_target_requirement() {
    for triple in [
        WINDOWS,
        "aarch64-pc-windows-msvc",
        "x86_64-pc-windows-gnu",
        "x86_64-unknown-linux-gnu",
        "x86_64-apple-darwin",
    ] {
        with_source(triple, "probe", |context, module| {
            let marker = module.get_global("_fltused");
            if triple == WINDOWS {
                let marker = marker.expect("owned x64 MSVC marker");
                assert_eq!(
                    marker.get_initializer(),
                    Some(context.i32_type().const_zero().into())
                );
                assert_eq!(marker.get_linkage(), Linkage::WeakODR);
                assert_eq!(marker.get_alignment(), 4);
                assert!(marker.get_comdat().is_some());
            } else {
                assert!(marker.is_none(), "unrelated target {triple}");
            }
        });
    }
}

#[test]
fn compatible_declaration_is_completed_but_conflicting_abi_data_is_rejected() {
    let ast = Parser::new("").parse_module().unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    for kind in [
        "declaration",
        "wrong_width",
        "nonzero",
        "import",
        "tls",
        "address_space",
        "function",
    ] {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("collision")
                .with_debug_info(false)
                .with_target(WINDOWS),
        );
        if kind == "function" {
            lower
                .module()
                .add_function("_fltused", context.void_type().fn_type(&[], false), None);
        } else {
            let ty = if kind == "wrong_width" {
                context.i64_type()
            } else {
                context.i32_type()
            };
            let global = lower.module().add_global(
                ty,
                if kind == "address_space" {
                    Some(verum_llvm::AddressSpace::from(1u16))
                } else {
                    None
                },
                "_fltused",
            );
            if kind == "nonzero" {
                global.set_initializer(&ty.const_int(7, false));
            }
            if kind == "import" {
                global.set_dll_storage_class(verum_llvm::DLLStorageClass::Import);
            }
            if kind == "tls" {
                global.set_thread_local(true);
            }
        }
        let result = lower.lower_module(&vbc);
        if kind == "declaration" {
            result.unwrap();
            let before = lower.module().get_global("_fltused").unwrap();
            lower.lower_module(&vbc).unwrap();
            assert_eq!(lower.module().get_global("_fltused"), Some(before));
            assert_eq!(
                before.get_initializer(),
                Some(context.i32_type().const_zero().into())
            );
            assert!(lower.module().get_global("_fltused.1").is_none());
        } else {
            let error = result.expect_err(kind).to_string();
            assert!(
                error.contains("reserved Windows Float ABI datum _fltused"),
                "{kind}: {error}"
            );
        }
    }
}
