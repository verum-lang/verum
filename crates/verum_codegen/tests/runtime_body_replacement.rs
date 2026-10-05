//! Replacing runtime bodies must release old LLVM uses, retaining call edges.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, platform_ir::PlatformIR};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::{BasicValue, BasicValueEnum}};
use verum_vbc::codegen::VbcCodegen;

#[test]
fn full_module_has_no_detached_runtime_users() {
    let vbc = VbcCodegen::new().compile_module(
        &Parser::new("fn main() {}").parse_module().unwrap()).unwrap();
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(&context,
        LoweringConfig::debug("runtime-replacement").with_debug_info(false));
    lower.lower_module(&vbc).unwrap();
    lower.module().verify().unwrap();
}

#[test]
fn repeated_platform_emission_preserves_tls_calls_without_orphan_users() {
    let context = Context::create();
    let vbc = VbcCodegen::new().compile_module(
        &Parser::new("fn main() {}").parse_module().unwrap()).unwrap();
    let mut lower = VbcToLlvmLowering::new(&context,
        LoweringConfig::debug("tls-replacement").with_debug_info(false));
    lower.lower_module(&vbc).unwrap();
    let module = lower.module();
    let platform = PlatformIR::new(&context);
    let get = module.get_function("verum_tls_get").unwrap();
    let builder = context.create_builder();
    let caller = module.add_function("tls_existing_caller", context.i64_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(caller, "entry"));
    let call = builder.build_call(get, &[context.i64_type().const_zero().into()], "tls").unwrap();
    let result: BasicValueEnum = call.try_as_basic_value().basic().unwrap();
    builder.build_return(Some(&result)).unwrap();
    for _ in 0..2 {
        platform.emit_platform_functions(module).unwrap();
        assert_eq!(get, module.get_function("verum_tls_get").unwrap());
        assert_eq!(call.get_called_fn_value(), Some(get));
        assert!(module.get_global("__verum_tls_slots").unwrap().as_pointer_value().get_first_use().is_none());
        module.verify().unwrap();
    }
}
