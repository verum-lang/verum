//! T1536: explicit Panic uses the configured common runtime policy.
use verum_codegen::llvm::context::FunctionContext;
use verum_codegen::llvm::instruction::lower_instruction;
use verum_codegen::llvm::vbc_lowering::{LoweringConfig, PanicStrategy, VbcToLlvmLowering};
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::values::AnyValue;
use verum_vbc::instruction::Instruction;
use verum_vbc::module::VbcModule;

#[test]
fn explicit_panic_calls_canonical_runtime_abi() {
    let context = Context::create();
    let module = context.create_module("panic_policy");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let mut vbc = VbcModule::new("panic_policy".to_owned());
    let message_id = vbc.intern_string("exact panic message");
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    lower_instruction(
        &mut ctx,
        &Instruction::Panic {
            message_id: message_id.0,
        },
    )
    .expect("lower panic");
    module.verify().expect("valid IR");
    let ir = function.print_to_string().to_string();
    assert!(ir.contains("call void @verum_panic(ptr"), "{ir}");
    assert!(ir.contains("i64 19, ptr null, i32 0"), "{ir}");
    assert!(!ir.contains("exit") && !ir.contains("puts"), "{ir}");
}

#[test]
fn runtime_unwind_throws_an_owned_message_packet_while_abort_terminates() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    for policy in [PanicStrategy::Unwind, PanicStrategy::Abort] {
        let context = Context::create();
        let mut config = LoweringConfig::new("runtime_panic_policy");
        config.panic_strategy = policy;
        let mut lowering = VbcToLlvmLowering::new(&context, config);
        lowering
            .lower_module(&VbcModule::new("runtime_panic_policy".to_owned()))
            .expect("ordinary runtime setup");
        let module = lowering.module();
        let panic = module.get_function("verum_panic").expect("canonical panic");
        assert_eq!(panic.count_params(), 4);
        let ir = panic.print_to_string().to_string();
        match policy {
            PanicStrategy::Unwind => {
                assert!(ir.contains("@verum_text_from_stride"), "{ir}");
                assert!(
                    ir.contains("@verum_exception_throw(i64 %panic_packet)"),
                    "{ir}"
                );
                assert!(!ir.contains("@_exit("), "{ir}");
                assert!(!ir.contains("@verum_exception_throw(i64 0)"), "{ir}");
            }
            PanicStrategy::Abort => {
                assert!(ir.contains("@_exit(i32 1)"), "{ir}");
                assert!(!ir.contains("@verum_exception_throw"), "{ir}");
            }
        }
    }
}
