//! Emit the signal opcode directly as optimizable IR, with only the kernel ABI
//! selected by the output target. No host probes, foreign objects or callbacks
//! into Verum execution are involved.
use super::context::FunctionContext;
use super::error::{BuildExt, LlvmLoweringError, OptionExt, Result};
use super::{syscall_registry::emit_linux_syscall_inline, target_triple::*};
use verum_llvm::{
    AddressSpace,
    attributes::{Attribute, AttributeLoc},
    memory_buffer::MemoryBuffer,
    module::Linkage,
    values::FunctionValue,
};
use verum_vbc::signal_watch::abi;

pub(super) fn runtime<'ctx>(ctx: &FunctionContext<'_, 'ctx>) -> Result<FunctionValue<'ctx>> {
    let cache = ctx
        .signal_runtime
        .or_internal("signal opcode requires module-owned lowering state")?;
    if let Some(&function) = cache.get() {
        return Ok(function);
    }
    let dest = ctx.get_module();
    let triple = dest.get_triple();
    let target = triple.as_str().to_string_lossy();
    let x86 = target_is_x86_64(dest);
    if !target_is_linux(dest)
        || !(x86 || target_is_aarch64(dest))
        || target.contains("x32")
        || target.contains("ilp32")
    {
        return Err(LlvmLoweringError::UnsupportedInstruction(
            format!(
                "Linux signal observation requires Linux x86-64 or AArch64 LP64, target is {target}"
            )
            .into(),
        ));
    }
    let llvm = ctx.llvm_context();
    let buffer = MemoryBuffer::create_from_memory_range_copy(
        include_bytes!("linux_signal.ll"),
        "VBC LinuxSignalWatch",
    );
    let ir = llvm
        .create_module_from_ir(buffer)
        .map_err(|e| LlvmLoweringError::internal(e.to_string()))?;
    ir.set_triple(&triple);
    ir.set_data_layout(&dest.get_data_layout());
    let builder = llvm.create_builder();
    let word = llvm.i64_type();
    let ptr = llvm.ptr_type(AddressSpace::default());
    let (flags, restorer) = if x86 {
        // Entered by RET from the handler with the kernel-provided signal frame
        // on RSP. Even a compiler-generated prologue would corrupt sigreturn.
        let ty = llvm.void_type().fn_type(&[], false);
        let trampoline = ir.add_function("rt_sigreturn", ty, Some(Linkage::Internal));
        trampoline.add_attribute(
            AttributeLoc::Function,
            llvm.create_enum_attribute(Attribute::get_named_enum_kind_id("naked"), 0),
        );
        builder.position_at_end(llvm.append_basic_block(trampoline, "entry"));
        let asm = llvm.create_inline_asm(
            ty,
            format!("movq $${}, %rax; syscall", abi::X86_SIGRETURN),
            "~{rax},~{rcx},~{r11},~{memory}".into(),
            true,
            false,
            None,
            false,
        );
        builder
            .build_indirect_call(ty, asm, &[], "")
            .or_llvm_err()?;
        builder.build_unreachable().or_llvm_err()?;
        (
            abi::SA_RESTART | abi::SA_RESTORER,
            trampoline.as_global_value().as_pointer_value(),
        )
    } else {
        (abi::SA_RESTART, ptr.const_null())
    };
    let observer = ir
        .get_function("count_delivery")
        .or_internal("missing signal handler")?;
    let action = ir
        .get_global("observer_action")
        .or_internal("missing signal action")?;
    let init = action
        .get_value_type()
        .into_struct_type()
        .const_named_struct(&[
            observer.as_global_value().as_pointer_value().into(),
            word.const_int(flags, false).into(),
            restorer.into(),
            word.const_zero().into(),
        ]);
    action.set_initializer(&init);
    action.set_linkage(Linkage::Internal);
    let exchange = ir
        .get_function("exchange_action")
        .or_internal("missing signal exchange")?;
    exchange.set_linkage(Linkage::Internal);
    builder.position_at_end(llvm.append_basic_block(exchange, "entry"));
    let mut args = exchange.get_param_iter();
    let sig = args
        .next()
        .or_internal("missing signal number")?
        .into_int_value();
    let next = args
        .next()
        .or_internal("missing signal action")?
        .into_pointer_value();
    let old = args
        .next()
        .or_internal("missing prior signal action")?
        .into_pointer_value();
    let result = emit_linux_syscall_inline(
        &builder,
        llvm,
        &ir,
        if x86 {
            abi::X86_SIGACTION
        } else {
            abi::ARM_SIGACTION
        },
        &[
            sig,
            builder.build_ptr_to_int(next, word, "next").or_llvm_err()?,
            builder
                .build_ptr_to_int(old, word, "previous")
                .or_llvm_err()?,
            word.const_int(abi::MASK_BYTES, false),
        ],
    )?;
    builder.build_return(Some(&result)).or_llvm_err()?;

    // Never bind an intrinsic to a source declaration merely because its name
    // matches. Pick a free link name, then keep only the emitter's actual value.
    let mut name = "verum#linux.signal.watch".to_owned();
    while dest.get_function(&name).is_some() || dest.get_global(&name).is_some() {
        name.push('_');
    }
    ir.get_function("watch")
        .or_internal("missing signal operation")?
        .as_global_value()
        .set_name(&name);
    dest.link_in_module(ir)
        .map_err(|e| LlvmLoweringError::internal(e.to_string()))?;
    let function = dest
        .get_function(&name)
        .or_internal("signal operation was not linked")?;
    function.set_linkage(Linkage::Internal);
    cache
        .set(function)
        .map_err(|_| LlvmLoweringError::internal("duplicate signal emitter"))?;
    Ok(function)
}
