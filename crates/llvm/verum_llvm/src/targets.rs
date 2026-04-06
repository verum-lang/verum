use verum_llvm_sys::target::{
    LLVMABIAlignmentOfType, LLVMABISizeOfType, LLVMByteOrder, LLVMByteOrdering, LLVMCallFrameAlignmentOfType,
    LLVMCopyStringRepOfTargetData, LLVMCreateTargetData, LLVMDisposeTargetData, LLVMElementAtOffset,
    LLVMIntPtrTypeForASInContext, LLVMIntPtrTypeInContext, LLVMOffsetOfElement, LLVMPointerSize, LLVMPointerSizeForAS,
    LLVMPreferredAlignmentOfGlobal, LLVMPreferredAlignmentOfType, LLVMSizeOfTypeInBits, LLVMStoreSizeOfType,
    LLVMTargetDataRef,
};
use verum_llvm_sys::target_machine::LLVMCreateTargetDataLayout;
use verum_llvm_sys::target_machine::{
    LLVMAddAnalysisPasses, LLVMCodeGenFileType, LLVMCodeModel, LLVMCreateTargetMachine, LLVMDisposeTargetMachine,
    LLVMGetDefaultTargetTriple, LLVMGetFirstTarget, LLVMGetNextTarget, LLVMGetTargetDescription, LLVMGetTargetFromName,
    LLVMGetTargetFromTriple, LLVMGetTargetMachineCPU, LLVMGetTargetMachineFeatureString, LLVMGetTargetMachineTarget,
    LLVMGetTargetMachineTriple, LLVMGetTargetName, LLVMRelocMode, LLVMSetTargetMachineAsmVerbosity,
    LLVMTargetHasAsmBackend, LLVMTargetHasJIT, LLVMTargetHasTargetMachine, LLVMTargetMachineEmitToFile,
    LLVMTargetMachineEmitToMemoryBuffer, LLVMTargetMachineRef, LLVMTargetRef,
};

use verum_llvm_sys::target_machine::{
    LLVMCreateTargetMachineOptions, LLVMCreateTargetMachineWithOptions, LLVMDisposeTargetMachineOptions,
    LLVMTargetMachineOptionsRef, LLVMTargetMachineOptionsSetABI, LLVMTargetMachineOptionsSetCPU,
    LLVMTargetMachineOptionsSetCodeGenOptLevel, LLVMTargetMachineOptionsSetCodeModel,
    LLVMTargetMachineOptionsSetFeatures, LLVMTargetMachineOptionsSetRelocMode,
};
use once_cell::sync::Lazy;
use std::sync::RwLock;

use crate::context::AsContextRef;
use crate::data_layout::DataLayout;
use crate::memory_buffer::MemoryBuffer;
use crate::module::Module;
use crate::passes::PassManager;
use crate::support::{to_c_str, LLVMString};
use crate::types::{AnyType, AsTypeRef, IntType, StructType};
use crate::values::{AsValueRef, GlobalValue};
use crate::{AddressSpace, OptimizationLevel};

use std::default::Default;
use std::ffi::CStr;
use std::fmt;
use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr;

#[derive(Default, Debug, PartialEq, Eq, Copy, Clone)]
pub enum CodeModel {
    #[default]
    Default,
    JITDefault,
    Small,
    Kernel,
    Medium,
    Large,
}

impl From<CodeModel> for LLVMCodeModel {
    fn from(value: CodeModel) -> Self {
        match value {
            CodeModel::Default => LLVMCodeModel::LLVMCodeModelDefault,
            CodeModel::JITDefault => LLVMCodeModel::LLVMCodeModelJITDefault,
            CodeModel::Small => LLVMCodeModel::LLVMCodeModelSmall,
            CodeModel::Kernel => LLVMCodeModel::LLVMCodeModelKernel,
            CodeModel::Medium => LLVMCodeModel::LLVMCodeModelMedium,
            CodeModel::Large => LLVMCodeModel::LLVMCodeModelLarge,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq, Copy, Clone)]
pub enum RelocMode {
    #[default]
    Default,
    Static,
    PIC,
    DynamicNoPic,
}

impl From<RelocMode> for LLVMRelocMode {
    fn from(value: RelocMode) -> Self {
        match value {
            RelocMode::Default => LLVMRelocMode::LLVMRelocDefault,
            RelocMode::Static => LLVMRelocMode::LLVMRelocStatic,
            RelocMode::PIC => LLVMRelocMode::LLVMRelocPIC,
            RelocMode::DynamicNoPic => LLVMRelocMode::LLVMRelocDynamicNoPic,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FileType {
    Assembly,
    Object,
}

impl FileType {
    fn as_llvm_file_type(&self) -> LLVMCodeGenFileType {
        match *self {
            FileType::Assembly => LLVMCodeGenFileType::LLVMAssemblyFile,
            FileType::Object => LLVMCodeGenFileType::LLVMObjectFile,
        }
    }
}

/// Configuration for LLVM target initialization. `base` enables TargetMachine support;
/// `machine_code` enables the MC layer (assembler backend).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct InitializationConfig {
    pub asm_parser: bool,
    pub asm_printer: bool,
    pub base: bool,
    pub disassembler: bool,
    pub info: bool,
    pub machine_code: bool,
}

impl Default for InitializationConfig {
    fn default() -> Self {
        InitializationConfig {
            asm_parser: true,
            asm_printer: true,
            base: true,
            disassembler: true,
            info: true,
            machine_code: true,
        }
    }
}

#[derive(Eq)]
pub struct TargetTriple {
    pub(crate) triple: LLVMString,
}

impl TargetTriple {
    pub unsafe fn new(triple: LLVMString) -> TargetTriple {
        TargetTriple { triple }
    }

    pub fn create(triple: &str) -> TargetTriple {
        let c_string = to_c_str(triple);

        TargetTriple {
            triple: LLVMString::create_from_c_str(&c_string),
        }
    }

    pub fn as_str(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.as_ptr()) }
    }

    pub fn as_ptr(&self) -> *const ::libc::c_char {
        self.triple.as_ptr()
    }
}

impl PartialEq for TargetTriple {
    fn eq(&self, other: &TargetTriple) -> bool {
        self.triple == other.triple
    }
}

impl fmt::Debug for TargetTriple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TargetTriple({:?})", self.triple)
    }
}

impl fmt::Display for TargetTriple {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.triple)
    }
}

static TARGET_LOCK: Lazy<RwLock<()>> = Lazy::new(|| RwLock::new(()));

// NOTE: Versions verified as target-complete: 3.6, 3.7, 3.8, 3.9, 4.0
#[derive(Debug, Eq, PartialEq)]
pub struct Target {
    target: LLVMTargetRef,
}

impl Target {
    pub unsafe fn new(target: LLVMTargetRef) -> Self {
        assert!(!target.is_null());

        Target { target }
    }

    /// Acquires the underlying raw pointer belonging to this `Target` type.
    pub fn as_mut_ptr(&self) -> LLVMTargetRef {
        self.target
    }

    // REVIEW: Should this just initialize all? Is opt into each a good idea?
    #[cfg(feature = "target-x86")]
    pub fn initialize_x86(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVMInitializeX86AsmParser, LLVMInitializeX86AsmPrinter, LLVMInitializeX86Disassembler,
            LLVMInitializeX86Target, LLVMInitializeX86TargetInfo, LLVMInitializeX86TargetMC,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86Target() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86TargetInfo() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86AsmPrinter() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86AsmParser() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86Disassembler() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeX86TargetMC() };
        }
    }

    #[cfg(feature = "target-arm")]
    pub fn initialize_arm(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVMInitializeARMAsmParser, LLVMInitializeARMAsmPrinter, LLVMInitializeARMDisassembler,
            LLVMInitializeARMTarget, LLVMInitializeARMTargetInfo, LLVMInitializeARMTargetMC,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMTarget() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMTargetInfo() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMAsmPrinter() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMAsmParser() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMDisassembler() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeARMTargetMC() };
        }
    }

    #[cfg(feature = "target-aarch64")]
    pub fn initialize_aarch64(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVMInitializeAArch64AsmParser, LLVMInitializeAArch64AsmPrinter, LLVMInitializeAArch64Disassembler,
            LLVMInitializeAArch64Target, LLVMInitializeAArch64TargetInfo, LLVMInitializeAArch64TargetMC,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64Target() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64TargetInfo() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64AsmPrinter() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64AsmParser() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64Disassembler() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeAArch64TargetMC() };
        }
    }

    #[cfg(feature = "target-wasm")]
    pub fn initialize_webassembly(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVMInitializeWebAssemblyAsmParser, LLVMInitializeWebAssemblyAsmPrinter,
            LLVMInitializeWebAssemblyDisassembler, LLVMInitializeWebAssemblyTarget,
            LLVMInitializeWebAssemblyTargetInfo, LLVMInitializeWebAssemblyTargetMC,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyTarget() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyTargetInfo() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyAsmPrinter() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyAsmParser() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyDisassembler() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeWebAssemblyTargetMC() };
        }
    }

    #[cfg(feature = "target-riscv")]
    pub fn initialize_riscv(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVMInitializeRISCVAsmParser, LLVMInitializeRISCVAsmPrinter, LLVMInitializeRISCVDisassembler,
            LLVMInitializeRISCVTarget, LLVMInitializeRISCVTargetInfo, LLVMInitializeRISCVTargetMC,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVTarget() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVTargetInfo() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVAsmPrinter() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVAsmParser() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVDisassembler() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMInitializeRISCVTargetMC() };
        }
    }

    pub fn initialize_native(config: &InitializationConfig) -> Result<(), String> {
        use verum_llvm_sys::target::{
            LLVM_InitializeNativeAsmParser, LLVM_InitializeNativeAsmPrinter, LLVM_InitializeNativeDisassembler,
            LLVM_InitializeNativeTarget,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            let code = unsafe { LLVM_InitializeNativeTarget() };

            if code == 1 {
                return Err("Unknown error in initializing native target".into());
            }
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            let code = unsafe { LLVM_InitializeNativeAsmPrinter() };

            if code == 1 {
                return Err("Unknown error in initializing native asm printer".into());
            }
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            let code = unsafe { LLVM_InitializeNativeAsmParser() };

            if code == 1 {
                // REVIEW: Does parser need to go before printer?
                return Err("Unknown error in initializing native asm parser".into());
            }
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            let code = unsafe { LLVM_InitializeNativeDisassembler() };

            if code == 1 {
                return Err("Unknown error in initializing native disassembler".into());
            }
        }

        Ok(())
    }

    pub fn initialize_all(config: &InitializationConfig) {
        use verum_llvm_sys::target::{
            LLVM_InitializeAllAsmParsers, LLVM_InitializeAllAsmPrinters, LLVM_InitializeAllDisassemblers,
            LLVM_InitializeAllTargetInfos, LLVM_InitializeAllTargetMCs, LLVM_InitializeAllTargets,
        };

        if config.base {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllTargets() };
        }

        if config.info {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllTargetInfos() };
        }

        if config.asm_parser {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllAsmParsers() };
        }

        if config.asm_printer {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllAsmPrinters() };
        }

        if config.disassembler {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllDisassemblers() };
        }

        if config.machine_code {
            let _guard = TARGET_LOCK.write().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVM_InitializeAllTargetMCs() };
        }
    }

    pub fn create_target_machine(
        &self,
        triple: &TargetTriple,
        cpu: &str,
        features: &str,
        level: OptimizationLevel,
        reloc_mode: RelocMode,
        code_model: CodeModel,
    ) -> Option<TargetMachine> {
        let cpu = to_c_str(cpu);
        let features = to_c_str(features);

        let target_machine = unsafe {
            LLVMCreateTargetMachine(
                self.target,
                triple.as_ptr(),
                cpu.as_ptr(),
                features.as_ptr(),
                level.into(),
                reloc_mode.into(),
                code_model.into(),
            )
        };

        if target_machine.is_null() {
            return None;
        }

        unsafe { Some(TargetMachine::new(target_machine)) }
    }

    /// Create a target machine from given [TargetMachineOptions].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::targets::{InitializationConfig, Target, TargetMachine, TargetTriple, TargetMachineOptions};
    /// use verum_llvm::OptimizationLevel;
    ///
    /// Target::initialize_native(&InitializationConfig::default()).unwrap();
    ///
    /// let triple = TargetMachine::get_default_triple();
    /// let target = Target::from_triple(&triple).unwrap();
    /// let options = TargetMachineOptions::default()
    ///     .set_cpu("generic")
    ///     .set_level(OptimizationLevel::Aggressive);
    ///
    /// let target_machine = target.create_target_machine_from_options(&triple, options).unwrap();
    /// ```
    pub fn create_target_machine_from_options(
        &self,
        triple: &TargetTriple,
        options: TargetMachineOptions,
    ) -> Option<TargetMachine> {
        options.into_target_machine(self.target, triple)
    }

    pub fn get_first() -> Option<Self> {
        let target = {
            let _guard = TARGET_LOCK.read().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMGetFirstTarget() }
        };

        if target.is_null() {
            return None;
        }

        unsafe { Some(Target::new(target)) }
    }

    pub fn get_next(&self) -> Option<Self> {
        let target = unsafe { LLVMGetNextTarget(self.target) };

        if target.is_null() {
            return None;
        }

        unsafe { Some(Target::new(target)) }
    }

    pub fn get_name(&self) -> &CStr {
        unsafe { CStr::from_ptr(LLVMGetTargetName(self.target)) }
    }

    pub fn get_description(&self) -> &CStr {
        unsafe { CStr::from_ptr(LLVMGetTargetDescription(self.target)) }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let c_string = to_c_str(name);

        Self::from_name_raw(c_string.as_ptr())
    }

    pub(crate) fn from_name_raw(c_string: *const ::libc::c_char) -> Option<Self> {
        let target = {
            let _guard = TARGET_LOCK.read().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMGetTargetFromName(c_string) }
        };

        if target.is_null() {
            return None;
        }

        unsafe { Some(Target::new(target)) }
    }

    pub fn from_triple(triple: &TargetTriple) -> Result<Self, LLVMString> {
        let mut target = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let code = {
            let _guard = TARGET_LOCK.read().unwrap_or_else(|e| e.into_inner());
            unsafe { LLVMGetTargetFromTriple(triple.as_ptr(), &mut target, err_string.as_mut_ptr()) }
        };

        if code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        unsafe { Ok(Target::new(target)) }
    }

    pub fn has_jit(&self) -> bool {
        unsafe { LLVMTargetHasJIT(self.target) == 1 }
    }

    pub fn has_target_machine(&self) -> bool {
        unsafe { LLVMTargetHasTargetMachine(self.target) == 1 }
    }

    pub fn has_asm_backend(&self) -> bool {
        unsafe { LLVMTargetHasAsmBackend(self.target) == 1 }
    }
}

#[derive(Debug)]
pub struct TargetMachine {
    pub(crate) target_machine: LLVMTargetMachineRef,
}

impl TargetMachine {
    pub unsafe fn new(target_machine: LLVMTargetMachineRef) -> Self {
        assert!(!target_machine.is_null());

        TargetMachine { target_machine }
    }

    /// Acquires the underlying raw pointer belonging to this `TargetMachine` type.
    pub fn as_mut_ptr(&self) -> LLVMTargetMachineRef {
        self.target_machine
    }

    pub fn get_target(&self) -> Target {
        unsafe { Target::new(LLVMGetTargetMachineTarget(self.target_machine)) }
    }

    pub fn get_triple(&self) -> TargetTriple {
        let str = unsafe { LLVMString::new(LLVMGetTargetMachineTriple(self.target_machine)) };

        unsafe { TargetTriple::new(str) }
    }

    /// Gets the default triple for the current system.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::targets::TargetMachine;
    ///
    /// let default_triple = TargetMachine::get_default_triple();
    ///
    /// assert_eq!(default_triple.as_str().to_str(), Ok("x86_64-pc-linux-gnu"));
    /// ```
    pub fn get_default_triple() -> TargetTriple {
        let llvm_string = unsafe { LLVMString::new(LLVMGetDefaultTargetTriple()) };

        unsafe { TargetTriple::new(llvm_string) }
    }

    pub fn normalize_triple(triple: &TargetTriple) -> TargetTriple {
        use verum_llvm_sys::target_machine::LLVMNormalizeTargetTriple;

        let normalized = unsafe { LLVMString::new(LLVMNormalizeTargetTriple(triple.as_ptr())) };

        unsafe { TargetTriple::new(normalized) }
    }

    /// Gets a string containing the host CPU's name (triple).
    ///
    /// # Example Output
    ///
    /// `x86_64-pc-linux-gnu`
    pub fn get_host_cpu_name() -> LLVMString {
        use verum_llvm_sys::target_machine::LLVMGetHostCPUName;

        unsafe { LLVMString::new(LLVMGetHostCPUName()) }
    }

    /// Gets a comma separated list of supported features by the host CPU.
    ///
    /// # Example Output
    ///
    /// `+sse2,+cx16,+sahf,-tbm`
    pub fn get_host_cpu_features() -> LLVMString {
        use verum_llvm_sys::target_machine::LLVMGetHostCPUFeatures;

        unsafe { LLVMString::new(LLVMGetHostCPUFeatures()) }
    }

    pub fn get_cpu(&self) -> LLVMString {
        unsafe { LLVMString::new(LLVMGetTargetMachineCPU(self.target_machine)) }
    }

    pub fn get_feature_string(&self) -> &CStr {
        unsafe { CStr::from_ptr(LLVMGetTargetMachineFeatureString(self.target_machine)) }
    }

    /// Create TargetData from this target machine
    pub fn get_target_data(&self) -> TargetData {
        unsafe { TargetData::new(LLVMCreateTargetDataLayout(self.target_machine)) }
    }

    pub fn set_asm_verbosity(&self, verbosity: bool) {
        unsafe { LLVMSetTargetMachineAsmVerbosity(self.target_machine, verbosity as i32) }
    }

    /// Adds target-specific analysis passes to the pass manager. Kept on TargetMachine
    /// because it requires target-specific knowledge.
    pub fn add_analysis_passes<T>(&self, pass_manager: &PassManager<T>) {
        unsafe { LLVMAddAnalysisPasses(self.target_machine, pass_manager.pass_manager) }
    }

    /// Writes a `TargetMachine` to a `MemoryBuffer`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::OptimizationLevel;
    /// use verum_llvm::context::Context;
    /// use verum_llvm::targets::{CodeModel, RelocMode, FileType, Target, TargetMachine, TargetTriple, InitializationConfig};
    ///
    /// Target::initialize_native(&InitializationConfig::default()).unwrap();
    ///
    /// let opt = OptimizationLevel::Default;
    /// let reloc = RelocMode::Default;
    /// let model = CodeModel::Default;
    /// let triple = TargetMachine::get_default_triple();
    /// let target = Target::from_triple(&triple).unwrap();
    /// let target_machine = target.create_target_machine(
    ///     &triple,
    ///     "generic",
    ///     "",
    ///     opt,
    ///     reloc,
    ///     model
    /// )
    /// .unwrap();
    ///
    /// let context = Context::create();
    /// let module = context.create_module("my_module");
    /// let void_type = context.void_type();
    /// let fn_type = void_type.fn_type(&[], false);
    ///
    /// module.add_function("my_fn", fn_type, None);
    ///
    /// let buffer = target_machine.write_to_memory_buffer(&module, FileType::Assembly).unwrap();
    /// ```
    pub fn write_to_memory_buffer(&self, module: &Module, file_type: FileType) -> Result<MemoryBuffer, LLVMString> {
        let mut memory_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();
        let return_code = unsafe {
            let module_ptr = module.module.get();
            let file_type_ptr = file_type.as_llvm_file_type();

            LLVMTargetMachineEmitToMemoryBuffer(
                self.target_machine,
                module_ptr,
                file_type_ptr,
                err_string.as_mut_ptr(),
                &mut memory_buffer,
            )
        };

        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        unsafe { Ok(MemoryBuffer::new(memory_buffer)) }
    }

    /// Saves a `TargetMachine` to a file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::OptimizationLevel;
    /// use verum_llvm::context::Context;
    /// use verum_llvm::targets::{CodeModel, RelocMode, FileType, Target, TargetMachine, TargetTriple, InitializationConfig};
    ///
    /// use std::path::Path;
    ///
    /// Target::initialize_native(&InitializationConfig::default()).unwrap();
    ///
    /// let opt = OptimizationLevel::Default;
    /// let reloc = RelocMode::Default;
    /// let model = CodeModel::Default;
    /// let path = Path::new("/tmp/some/path/main.o");
    /// let triple = TargetMachine::get_default_triple();
    /// let target = Target::from_triple(&triple).unwrap();
    /// let target_machine = target.create_target_machine(
    ///     &triple,
    ///     "generic",
    ///     "",
    ///     opt,
    ///     reloc,
    ///     model
    /// )
    /// .unwrap();
    ///
    /// let context = Context::create();
    /// let module = context.create_module("my_module");
    /// let void_type = context.void_type();
    /// let fn_type = void_type.fn_type(&[], false);
    ///
    /// module.add_function("my_fn", fn_type, None);
    ///
    /// assert!(target_machine.write_to_file(&module, FileType::Object, &path).is_ok());
    /// ```
    pub fn write_to_file(&self, module: &Module, file_type: FileType, path: &Path) -> Result<(), LLVMString> {
        let path = path.to_str().expect("Did not find a valid Unicode path string");
        let path_c_string = to_c_str(path);
        let mut err_string = MaybeUninit::uninit();
        let return_code = unsafe {
            // REVIEW: Why does LLVM need a mutable ptr to path...?
            let module_ptr = module.module.get();
            let path_ptr = path_c_string.as_ptr() as *mut _;
            let file_type_ptr = file_type.as_llvm_file_type();

            LLVMTargetMachineEmitToFile(
                self.target_machine,
                module_ptr,
                path_ptr,
                file_type_ptr,
                err_string.as_mut_ptr(),
            )
        };

        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        Ok(())
    }
}

impl Drop for TargetMachine {
    fn drop(&mut self) {
        unsafe { LLVMDisposeTargetMachine(self.target_machine) }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum ByteOrdering {
    BigEndian,
    LittleEndian,
}

#[derive(PartialEq, Eq, Debug)]
pub struct TargetData {
    pub(crate) target_data: LLVMTargetDataRef,
}

impl TargetData {
    pub unsafe fn new(target_data: LLVMTargetDataRef) -> TargetData {
        assert!(!target_data.is_null());

        TargetData { target_data }
    }

    /// Acquires the underlying raw pointer belonging to this `TargetData` type.
    pub fn as_mut_ptr(&self) -> LLVMTargetDataRef {
        self.target_data
    }

    /// Gets the `IntType` representing a bit width of a pointer. It will be assigned the referenced context.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::OptimizationLevel;
    /// use verum_llvm::context::Context;
    /// use verum_llvm::targets::{InitializationConfig, Target};
    ///
    /// Target::initialize_native(&InitializationConfig::default()).expect("Failed to initialize native target");
    ///
    /// let context = Context::create();
    /// let module = context.create_module("sum");
    /// let execution_engine = module.create_jit_execution_engine(OptimizationLevel::None).unwrap();
    /// let target_data = execution_engine.get_target_data();
    /// let int_type = target_data.ptr_sized_int_type_in_context(&context, None);
    /// ```
    #[deprecated(note = "This method will be removed in the future. Please use Context::ptr_sized_int_type instead.")]
    pub fn ptr_sized_int_type_in_context<'ctx>(
        &self,
        context: impl AsContextRef<'ctx>,
        address_space: Option<AddressSpace>,
    ) -> IntType<'ctx> {
        let int_type_ptr = match address_space {
            Some(address_space) => unsafe {
                LLVMIntPtrTypeForASInContext(context.as_ctx_ref(), self.target_data, address_space.0)
            },
            None => unsafe { LLVMIntPtrTypeInContext(context.as_ctx_ref(), self.target_data) },
        };

        unsafe { IntType::new(int_type_ptr) }
    }

    pub fn get_data_layout(&self) -> DataLayout {
        unsafe { DataLayout::new_owned(LLVMCopyStringRepOfTargetData(self.target_data)) }
    }

    // REVIEW: Does this only work if Sized?
    pub fn get_bit_size(&self, type_: &dyn AnyType) -> u64 {
        unsafe { LLVMSizeOfTypeInBits(self.target_data, type_.as_type_ref()) }
    }

    /// Creates a TargetData from a target data layout string. LLVM may call `exit()` on
    /// invalid input; there is no way to recover from this via the C API.
    pub fn create(str_repr: &str) -> TargetData {
        let c_string = to_c_str(str_repr);

        unsafe { TargetData::new(LLVMCreateTargetData(c_string.as_ptr())) }
    }

    pub fn get_byte_ordering(&self) -> ByteOrdering {
        let byte_ordering = unsafe { LLVMByteOrder(self.target_data) };

        match byte_ordering {
            LLVMByteOrdering::LLVMBigEndian => ByteOrdering::BigEndian,
            LLVMByteOrdering::LLVMLittleEndian => ByteOrdering::LittleEndian,
        }
    }

    pub fn get_pointer_byte_size(&self, address_space: Option<AddressSpace>) -> u32 {
        match address_space {
            Some(address_space) => unsafe { LLVMPointerSizeForAS(self.target_data, address_space.0) },
            None => unsafe { LLVMPointerSize(self.target_data) },
        }
    }

    pub fn get_store_size(&self, type_: &dyn AnyType) -> u64 {
        unsafe { LLVMStoreSizeOfType(self.target_data, type_.as_type_ref()) }
    }

    pub fn get_abi_size(&self, type_: &dyn AnyType) -> u64 {
        unsafe { LLVMABISizeOfType(self.target_data, type_.as_type_ref()) }
    }

    pub fn get_abi_alignment(&self, type_: &dyn AnyType) -> u32 {
        unsafe { LLVMABIAlignmentOfType(self.target_data, type_.as_type_ref()) }
    }

    pub fn get_call_frame_alignment(&self, type_: &dyn AnyType) -> u32 {
        unsafe { LLVMCallFrameAlignmentOfType(self.target_data, type_.as_type_ref()) }
    }

    pub fn get_preferred_alignment(&self, type_: &dyn AnyType) -> u32 {
        unsafe { LLVMPreferredAlignmentOfType(self.target_data, type_.as_type_ref()) }
    }

    pub fn get_preferred_alignment_of_global(&self, value: &GlobalValue) -> u32 {
        unsafe { LLVMPreferredAlignmentOfGlobal(self.target_data, value.as_value_ref()) }
    }

    pub fn element_at_offset(&self, struct_type: &StructType, offset: u64) -> u32 {
        unsafe { LLVMElementAtOffset(self.target_data, struct_type.as_type_ref(), offset) }
    }

    pub fn offset_of_element(&self, struct_type: &StructType, element: u32) -> Option<u64> {
        if element > struct_type.count_fields() - 1 {
            return None;
        }

        unsafe {
            Some(LLVMOffsetOfElement(
                self.target_data,
                struct_type.as_type_ref(),
                element,
            ))
        }
    }
}

impl Drop for TargetData {
    fn drop(&mut self) {
        unsafe { LLVMDisposeTargetData(self.target_data) }
    }
}

/// LLVM target machine options provide another way to create target machines,
/// used with [Target::create_target_machine_from_options].
///
/// The option structure exposes an additional setting (i.e., the target ABI)
/// and provides default values for unspecified settings.

#[derive(Default, Debug)]
pub struct TargetMachineOptions(Option<LLVMTargetMachineOptionsRef>);


impl TargetMachineOptions {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn set_cpu(mut self, cpu: &str) -> Self {
        let cpu = to_c_str(cpu);
        unsafe { LLVMTargetMachineOptionsSetCPU(self.inner(), cpu.as_ptr()) };

        self
    }

    pub fn set_features(mut self, features: &str) -> Self {
        let features = to_c_str(features);
        unsafe { LLVMTargetMachineOptionsSetFeatures(self.inner(), features.as_ptr()) };

        self
    }

    pub fn set_abi(mut self, abi: &str) -> Self {
        let abi = to_c_str(abi);
        unsafe { LLVMTargetMachineOptionsSetABI(self.inner(), abi.as_ptr()) };

        self
    }

    pub fn set_level(mut self, level: OptimizationLevel) -> Self {
        unsafe { LLVMTargetMachineOptionsSetCodeGenOptLevel(self.inner(), level.into()) };

        self
    }

    pub fn set_reloc_mode(mut self, reloc_mode: RelocMode) -> Self {
        unsafe { LLVMTargetMachineOptionsSetRelocMode(self.inner(), reloc_mode.into()) }

        self
    }

    pub fn set_code_model(mut self, code_model: CodeModel) -> Self {
        unsafe { LLVMTargetMachineOptionsSetCodeModel(self.inner(), code_model.into()) };

        self
    }

    fn into_target_machine(mut self, target: LLVMTargetRef, triple: &TargetTriple) -> Option<TargetMachine> {
        let target_machine = unsafe { LLVMCreateTargetMachineWithOptions(target, triple.as_ptr(), self.inner()) };

        if target_machine.is_null() {
            return None;
        }

        unsafe { Some(TargetMachine::new(target_machine)) }
    }

    /// SAFETY:
    /// - The internal `LLVMCreateTargetMachineOptionsRef` structure leaks memory
    ///   if not disposed via `fn LLVMCreateTargetMachineWithOptions()`.
    /// - The only way to access it is via this private method.
    /// - Disposal is taken care of automatically in `Drop::drop`.
    unsafe fn inner(&mut self) -> LLVMTargetMachineOptionsRef {
        *self.0.get_or_insert_with(|| LLVMCreateTargetMachineOptions())
    }
}


impl Drop for TargetMachineOptions {
    fn drop(&mut self) {
        if let Some(inner) = self.0 {
            unsafe { LLVMDisposeTargetMachineOptions(inner) };
        }
    }
}
