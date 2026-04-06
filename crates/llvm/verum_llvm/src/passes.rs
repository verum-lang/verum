//! LLVM Pass Infrastructure
//!
//! LLVM 17+ removed the legacy pass manager. This module provides the new pass builder API.

use verum_llvm_sys::core::{
    LLVMCreateFunctionPassManagerForModule, LLVMCreatePassManager, LLVMDisposePassManager,
    LLVMFinalizeFunctionPassManager, LLVMInitializeFunctionPassManager, LLVMRunFunctionPassManager,
    LLVMRunPassManager,
};
use verum_llvm_sys::prelude::LLVMPassManagerRef;
use verum_llvm_sys::transforms::pass_builder::{
    LLVMCreatePassBuilderOptions, LLVMDisposePassBuilderOptions, LLVMPassBuilderOptionsRef,
    LLVMPassBuilderOptionsSetCallGraphProfile, LLVMPassBuilderOptionsSetDebugLogging,
    LLVMPassBuilderOptionsSetForgetAllSCEVInLoopUnroll, LLVMPassBuilderOptionsSetLicmMssaNoAccForPromotionCap,
    LLVMPassBuilderOptionsSetLicmMssaOptCap, LLVMPassBuilderOptionsSetLoopInterleaving,
    LLVMPassBuilderOptionsSetLoopUnrolling, LLVMPassBuilderOptionsSetLoopVectorization,
    LLVMPassBuilderOptionsSetMergeFunctions, LLVMPassBuilderOptionsSetSLPVectorization,
    LLVMPassBuilderOptionsSetVerifyEach,
};

use crate::module::Module;
use crate::values::{AsValueRef, FunctionValue};

use std::borrow::Borrow;
use std::marker::PhantomData;

// This is an ugly privacy hack so that PassManagerSubType can stay private
// to this module and so that super traits using this trait will be not be
// implementable outside this library
pub trait PassManagerSubType {
    type Input;

    unsafe fn create<I: Borrow<Self::Input>>(input: I) -> LLVMPassManagerRef;
    unsafe fn run_in_pass_manager(&self, pass_manager: &PassManager<Self>) -> bool
    where
        Self: Sized;
}

impl PassManagerSubType for Module<'_> {
    type Input = ();

    unsafe fn create<I: Borrow<Self::Input>>(_: I) -> LLVMPassManagerRef {
        LLVMCreatePassManager()
    }

    unsafe fn run_in_pass_manager(&self, pass_manager: &PassManager<Self>) -> bool {
        LLVMRunPassManager(pass_manager.pass_manager, self.module.get()) == 1
    }
}

impl<'ctx> PassManagerSubType for FunctionValue<'ctx> {
    type Input = Module<'ctx>;

    unsafe fn create<I: Borrow<Self::Input>>(input: I) -> LLVMPassManagerRef {
        LLVMCreateFunctionPassManagerForModule(input.borrow().module.get())
    }

    unsafe fn run_in_pass_manager(&self, pass_manager: &PassManager<Self>) -> bool {
        LLVMRunFunctionPassManager(pass_manager.pass_manager, self.as_value_ref()) == 1
    }
}

/// A manager for running optimization and simplification passes.
///
/// Note: In LLVM 17+, the legacy pass manager was removed. Use `PassBuilderOptions`
/// with `Module::run_passes` for the new pass pipeline.
#[derive(Debug)]
pub struct PassManager<T> {
    pub(crate) pass_manager: LLVMPassManagerRef,
    sub_type: PhantomData<T>,
}

impl PassManager<FunctionValue<'_>> {
    /// Acquires the underlying raw pointer belonging to this `PassManager<T>` type.
    pub fn as_mut_ptr(&self) -> LLVMPassManagerRef {
        self.pass_manager
    }

    // return true means some pass modified the module, not an error occurred
    pub fn initialize(&self) -> bool {
        unsafe { LLVMInitializeFunctionPassManager(self.pass_manager) == 1 }
    }

    pub fn finalize(&self) -> bool {
        unsafe { LLVMFinalizeFunctionPassManager(self.pass_manager) == 1 }
    }
}

impl<T: PassManagerSubType> PassManager<T> {
    pub unsafe fn new(pass_manager: LLVMPassManagerRef) -> Self {
        assert!(!pass_manager.is_null());

        PassManager {
            pass_manager,
            sub_type: PhantomData,
        }
    }

    pub fn create<I: Borrow<T::Input>>(input: I) -> PassManager<T> {
        let pass_manager = unsafe { T::create(input) };

        unsafe { PassManager::new(pass_manager) }
    }

    /// This method returns true if any of the passes modified the function or module
    /// and false otherwise.
    pub fn run_on(&self, input: &T) -> bool {
        unsafe { input.run_in_pass_manager(self) }
    }
}

impl<T> Drop for PassManager<T> {
    fn drop(&mut self) {
        unsafe { LLVMDisposePassManager(self.pass_manager) }
    }
}

/// Options for the new pass builder (LLVM 13+).
///
/// This is the recommended way to run optimization passes in LLVM 17+.
/// Use with `Module::run_passes()`.
///
/// # Example
///
/// ```no_run
/// use verum_llvm::context::Context;
/// use verum_llvm::passes::PassBuilderOptions;
/// use verum_llvm::targets::{CodeModel, RelocMode, Target, TargetMachine, TargetTriple, InitializationConfig};
/// use verum_llvm::OptimizationLevel;
///
/// let context = Context::create();
/// let module = context.create_module("my_module");
///
/// Target::initialize_native(&InitializationConfig::default()).unwrap();
/// let triple = TargetMachine::get_default_triple();
/// let target = Target::from_triple(&triple).unwrap();
/// let target_machine = target.create_target_machine(
///     &triple,
///     "generic",
///     "",
///     OptimizationLevel::Default,
///     RelocMode::Default,
///     CodeModel::Default,
/// ).unwrap();
///
/// let options = PassBuilderOptions::create();
/// options.set_verify_each(true);
/// options.set_loop_vectorization(true);
///
/// // Run optimization passes
/// module.run_passes("default<O2>", &target_machine, options).unwrap();
/// ```
#[derive(Debug)]
pub struct PassBuilderOptions {
    pub(crate) options_ref: LLVMPassBuilderOptionsRef,
}

impl PassBuilderOptions {
    /// Create a new set of options for a PassBuilder
    pub fn create() -> Self {
        unsafe {
            PassBuilderOptions {
                options_ref: LLVMCreatePassBuilderOptions(),
            }
        }
    }

    /// Acquires the underlying raw pointer belonging to this `PassBuilderOptions` type.
    pub fn as_mut_ptr(&self) -> LLVMPassBuilderOptionsRef {
        self.options_ref
    }

    /// Toggle adding the VerifierPass for the PassBuilder, ensuring all functions
    /// inside the module is valid.
    pub fn set_verify_each(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetVerifyEach(self.options_ref, value as i32);
        }
    }

    /// Toggle debug logging when running the PassBuilder.
    pub fn set_debug_logging(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetDebugLogging(self.options_ref, value as i32);
        }
    }

    pub fn set_loop_interleaving(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetLoopInterleaving(self.options_ref, value as i32);
        }
    }

    pub fn set_loop_vectorization(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetLoopVectorization(self.options_ref, value as i32);
        }
    }

    pub fn set_loop_slp_vectorization(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetSLPVectorization(self.options_ref, value as i32);
        }
    }

    pub fn set_loop_unrolling(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetLoopUnrolling(self.options_ref, value as i32);
        }
    }

    pub fn set_forget_all_scev_in_loop_unroll(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetForgetAllSCEVInLoopUnroll(self.options_ref, value as i32);
        }
    }

    pub fn set_licm_mssa_opt_cap(&self, value: u32) {
        unsafe {
            LLVMPassBuilderOptionsSetLicmMssaOptCap(self.options_ref, value);
        }
    }

    pub fn set_licm_mssa_no_acc_for_promotion_cap(&self, value: u32) {
        unsafe {
            LLVMPassBuilderOptionsSetLicmMssaNoAccForPromotionCap(self.options_ref, value);
        }
    }

    pub fn set_call_graph_profile(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetCallGraphProfile(self.options_ref, value as i32);
        }
    }

    pub fn set_merge_functions(&self, value: bool) {
        unsafe {
            LLVMPassBuilderOptionsSetMergeFunctions(self.options_ref, value as i32);
        }
    }
}

impl Drop for PassBuilderOptions {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposePassBuilderOptions(self.options_ref);
        }
    }
}

// Note: Module::run_passes is implemented in module.rs
