//! Lower the public context facade, including the scope delimiters, into one linked program.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/context_slots.rs"]
mod fixture;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn slots_and_lexical_providers_share_native_inherited_context_frames() {
    let bytes = verum_vbc::serialize::serialize_module(&fixture::module()).unwrap();
    let module = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(module, |_| {});
}

#[test]
fn existing_named_handler_images_keep_native_scope_order() {
    let mut bytes = verum_vbc::serialize::serialize_module(&fixture::legacy_module()).unwrap();
    bytes[6..8].copy_from_slice(&70u16.to_le_bytes());
    source_image::run_module(
        verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap(),
        |_| {},
    );
}

#[test]
fn elf_context_lookups_address_tls_without_a_libc_resolver() {
    use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
    use verum_llvm::{
        OptimizationLevel, ThreadLocalMode,
        context::Context,
        memory_buffer::MemoryBuffer,
        targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple},
        values::AnyValue,
    };
    Target::initialize_all(&InitializationConfig::default());
    let vbc = source_image::module("fn main() -> Int { 73 }");
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-freebsd",
    ] {
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("context_tls")
                .with_target(target)
                .with_debug_info(false),
        );
        lowering.lower_module(&vbc).unwrap();
        let module = lowering.module();
        let mut ir = String::new();
        for line in module.print_to_string().to_str().unwrap().lines() {
            if line.starts_with("%context.entry") && line.contains(" = type ") {
                ir.push_str(line);
                ir.push('\n');
            }
        }
        for name in ["context.head", "context.depth"] {
            let global = module.get_global(name).unwrap();
            assert_eq!(
                global.get_thread_local_mode(),
                Some(ThreadLocalMode::InitialExecTLSModel),
                "{target}"
            );
            ir.push_str(global.print_to_string().to_str().unwrap());
            ir.push('\n');
        }
        for name in ["context.find", "context.read", "verum_ctx_slot_get"] {
            ir.push_str(
                module
                    .get_function(name)
                    .unwrap()
                    .print_to_string()
                    .to_str()
                    .unwrap(),
            );
            ir.push('\n');
        }
        let lookup = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "context_lookup",
            ))
            .unwrap();
        let triple = TargetTriple::create(target);
        let machine = Target::from_triple(&triple)
            .unwrap()
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::Default,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .unwrap();
        lookup.set_triple(&triple);
        lookup.set_data_layout(&machine.get_target_data().get_data_layout());
        lookup.verify().unwrap();
        let object = machine
            .write_to_memory_buffer(&lookup, FileType::Object)
            .unwrap()
            .create_object_file()
            .unwrap();
        let mut found_lookup = false;
        for symbol in object.get_symbols().filter_map(|symbol| {
            symbol
                .get_name()
                .map(|name| name.to_string_lossy().into_owned())
        }) {
            found_lookup |= symbol == "verum_ctx_slot_get";
            assert!(
                !symbol.contains("tls_get_addr"),
                "{target} imports {symbol}"
            );
        }
        assert!(found_lookup, "{target}");
    }
}
