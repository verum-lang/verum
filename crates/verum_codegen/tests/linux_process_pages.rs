#![cfg(unix)]
#[path = "support/linux_elf.rs"]
mod linux_elf;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, memory_buffer::MemoryBuffer};
use verum_vbc::codegen::VbcCodegen;

#[test]
fn kernel_error_is_null_and_writable_pages_can_be_released() {
    let vbc = VbcCodegen::new()
        .compile_module(
            &Parser::new("fn main() -> Int { 1 }")
                .parse_module()
                .unwrap(),
        )
        .unwrap();
    for arch in ["x86_64", "aarch64"] {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("pages")
                .with_debug_info(false)
                .with_target(format!("{arch}-unknown-linux-gnu")),
        );
        lower.lower_module(&vbc).unwrap();
        let module = lower.module();
        let probe = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                br#"
            declare ptr @verum_os_alloc(i64)
            declare void @verum_os_free(ptr, i64)
            define i32 @probe_pages() {
              %invalid = call ptr @verum_os_alloc(i64 0)
              %rejected = icmp eq ptr %invalid, null
              br i1 %rejected, label %valid, label %fail
            valid:
              %page = call ptr @verum_os_alloc(i64 4096)
              %allocated = icmp ne ptr %page, null
              br i1 %allocated, label %use, label %fail
            use:
              store volatile i32 73, ptr %page
              %value = load volatile i32, ptr %page
              call void @verum_os_free(ptr %page, i64 4096)
              ret i32 %value
            fail:
              ret i32 1
            }
        "#,
                "kernel page probe",
            ))
            .unwrap();
        probe.set_triple(&module.get_triple());
        probe.set_data_layout(&module.get_data_layout());
        module.link_in_module(probe).unwrap();
        linux_elf::check(module, arch, "probe_pages", "pages", 73, b"");
    }
}

#[test]
fn unavailable_kernel_abi_cannot_emit_an_empty_asm_result() {
    use verum_codegen::llvm::syscall_registry::emit_linux_syscall_inline;
    use verum_llvm::targets::TargetTriple;
    for target in [
        "riscv64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnux32",
        "aarch64-apple-darwin",
    ] {
        let ctx = Context::create();
        let module = ctx.create_module("unsupported_kernel");
        module.set_triple(&TargetTriple::create(target));
        let err =
            emit_linux_syscall_inline(&ctx.create_builder(), &ctx, &module, 0, &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("no direct Linux syscall ABI for {target}")),
            "{err}"
        );
    }
}
