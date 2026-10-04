//! T1536: explicit Panic uses the configured common runtime policy.
use verum_codegen::llvm::context::FunctionContext;
use verum_codegen::llvm::instruction::lower_instruction;
use verum_codegen::llvm::vbc_lowering::{LoweringConfig, PanicStrategy, VbcToLlvmLowering};
use verum_llvm::attributes::{Attribute, AttributeLoc};
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::values::AnyValue;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::module::{FunctionDescriptor, VbcModule};

fn assert_try_begin_returns_twice(triple: &str, callee_name: &str, predeclare: bool) {
    let context = Context::create();
    let mut vbc = VbcModule::new("try_begin_contract".to_owned());
    let mut function = FunctionDescriptor::new(vbc.intern_string("main"));
    function.register_count = 2;
    function.return_type = verum_vbc::types::TypeRef::concrete(verum_vbc::types::TypeId::INT);
    function.instructions = Some(vec![
        Instruction::TryBegin { handler_offset: 4 },
        Instruction::LoadI {
            dst: Reg(0),
            value: 37,
        },
        Instruction::TryEnd,
        Instruction::Ret { value: Reg(0) },
        Instruction::GetException { dst: Reg(1) },
        Instruction::Ret { value: Reg(1) },
    ]);
    vbc.add_function(function);
    let config = LoweringConfig::new("try_begin_contract").with_target(triple);
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering.module().add_function(
        "unmarked_internal",
        context.i32_type().fn_type(&[], false),
        None,
    );
    if predeclare {
        lowering.module().add_function(
            callee_name,
            context
                .i32_type()
                .fn_type(&[context.ptr_type(Default::default()).into()], false),
            None,
        );
    }
    lowering.lower_module(&vbc).expect("lower real TryBegin");
    let module = lowering.module();
    let actual_calls = module.get_functions().any(|function| {
        function
            .print_to_string()
            .to_string()
            .contains(&format!("call i32 @{callee_name}("))
    });
    assert!(
        actual_calls,
        "TryBegin must call {callee_name} for {triple}"
    );
    let callee = module
        .get_function(callee_name)
        .expect("actual TryBegin callee");
    assert_eq!(
        callee.count_basic_blocks(),
        0,
        "{callee_name} must not become a zero-return stub"
    );
    assert_eq!(callee.get_linkage(), verum_llvm::module::Linkage::External);
    let kind = Attribute::get_named_enum_kind_id("returns_twice");
    assert!(
        callee
            .get_enum_attribute(AttributeLoc::Function, kind)
            .is_some(),
        "{triple}: actual callee lacks LLVM returns_twice: {}",
        callee.print_to_string(),
    );
    if callee_name == "_setjmp" {
        let longjmp = module.get_function("longjmp").expect("actual throw callee");
        assert_eq!(
            longjmp.count_basic_blocks(),
            0,
            "longjmp must remain linker-resolved"
        );
        assert_eq!(longjmp.get_linkage(), verum_llvm::module::Linkage::External);
        assert!(
            longjmp
                .get_enum_attribute(
                    AttributeLoc::Function,
                    Attribute::get_named_enum_kind_id("noreturn")
                )
                .is_some()
        );
    } else {
        assert!(module.get_function("_setjmp").is_none());
        assert!(module.get_function("longjmp").is_none());
    }
    assert!(
        module
            .get_function("unmarked_internal")
            .expect("negative control")
            .count_basic_blocks()
            > 0
    );
}

#[test]
fn darwin_try_begin_marks_the_actual_setjmp_callee_returns_twice() {
    for triple in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        assert_try_begin_returns_twice(triple, "_setjmp", false);
        assert_try_begin_returns_twice(triple, "_setjmp", true);
    }
}

#[test]
fn sjlj_try_begin_preserves_the_same_double_return_contract() {
    for triple in ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"] {
        assert_try_begin_returns_twice(triple, "llvm.eh.sjlj.setjmp", false);
    }
}

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
