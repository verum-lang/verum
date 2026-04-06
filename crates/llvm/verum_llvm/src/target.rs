//! Target machine configuration and code generation
//!
//! Handles CPU targeting, features, and code model configuration.

use verum_verum_llvm_sys::llvm::target::*;
use verum_verum_llvm_sys::llvm::target_machine::*;
use verum_verum_llvm_sys::llvm::prelude::*;
use parking_lot::RwLock;
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{LlvmError, LlvmResult};
use crate::support::{to_c_string, Triple, host_cpu_name, host_cpu_features, MemoryBuffer};

/// Global lock for target initialization (LLVM is not thread-safe for init)
static TARGET_INIT_LOCK: RwLock<()> = RwLock::new(());
static TARGETS_INITIALIZED: AtomicBool = AtomicBool::new(false);
static NATIVE_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize all LLVM targets
pub fn initialize_all_targets() {
    if TARGETS_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let _guard = TARGET_INIT_LOCK.write();
    if TARGETS_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    // Use verum_verum_llvm_sys wrapper functions
    verum_verum_llvm_sys::initialize_targets();

    TARGETS_INITIALIZED.store(true, Ordering::Release);
    NATIVE_INITIALIZED.store(true, Ordering::Release);
}

/// Initialize native target only
pub fn initialize_native_target() {
    if NATIVE_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let _guard = TARGET_INIT_LOCK.write();
    if NATIVE_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    // Use verum_verum_llvm_sys wrapper functions
    verum_verum_llvm_sys::initialize_native_target();

    NATIVE_INITIALIZED.store(true, Ordering::Release);
}

/// Code generation optimization level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeGenOptLevel {
    None,
    Less,
    #[default]
    Default,
    Aggressive,
}

impl From<CodeGenOptLevel> for LLVMCodeGenOptLevel {
    fn from(level: CodeGenOptLevel) -> Self {
        match level {
            CodeGenOptLevel::None => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            CodeGenOptLevel::Less => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            CodeGenOptLevel::Default => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            CodeGenOptLevel::Aggressive => LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
        }
    }
}

/// Relocation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelocMode {
    #[default]
    Default,
    Static,
    PIC,
    DynamicNoPic,
}

impl From<RelocMode> for LLVMRelocMode {
    fn from(mode: RelocMode) -> Self {
        match mode {
            RelocMode::Default => LLVMRelocMode::LLVMRelocDefault,
            RelocMode::Static => LLVMRelocMode::LLVMRelocStatic,
            RelocMode::PIC => LLVMRelocMode::LLVMRelocPIC,
            RelocMode::DynamicNoPic => LLVMRelocMode::LLVMRelocDynamicNoPic,
        }
    }
}

/// Code model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeModel {
    #[default]
    Default,
    JITDefault,
    Tiny,
    Small,
    Kernel,
    Medium,
    Large,
}

impl From<CodeModel> for LLVMCodeModel {
    fn from(model: CodeModel) -> Self {
        match model {
            CodeModel::Default => LLVMCodeModel::LLVMCodeModelDefault,
            CodeModel::JITDefault => LLVMCodeModel::LLVMCodeModelJITDefault,
            CodeModel::Tiny => LLVMCodeModel::LLVMCodeModelTiny,
            CodeModel::Small => LLVMCodeModel::LLVMCodeModelSmall,
            CodeModel::Kernel => LLVMCodeModel::LLVMCodeModelKernel,
            CodeModel::Medium => LLVMCodeModel::LLVMCodeModelMedium,
            CodeModel::Large => LLVMCodeModel::LLVMCodeModelLarge,
        }
    }
}

/// File type for code generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Assembly,
    Object,
}

impl From<FileType> for LLVMCodeGenFileType {
    fn from(ft: FileType) -> Self {
        match ft {
            FileType::Assembly => LLVMCodeGenFileType::LLVMAssemblyFile,
            FileType::Object => LLVMCodeGenFileType::LLVMObjectFile,
        }
    }
}

/// Target configuration
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub triple: Triple,
    /// CPU name (e.g., "generic", "native", "znver4")
    pub cpu: String,
    /// CPU features (e.g., "+avx512f,+avx512vl")
    pub features: String,
    /// Optimization level
    pub opt_level: CodeGenOptLevel,
    /// Relocation mode
    pub reloc_mode: RelocMode,
    /// Code model
    pub code_model: CodeModel,
}

impl TargetConfig {
    /// Create configuration for native target
    pub fn native() -> Self {
        initialize_native_target();
        Self {
            triple: Triple::host(),
            cpu: host_cpu_name(),
            features: host_cpu_features(),
            opt_level: CodeGenOptLevel::Default,
            reloc_mode: RelocMode::Default,
            code_model: CodeModel::Default,
        }
    }

    /// Create configuration with specific triple
    pub fn for_triple(triple: Triple) -> Self {
        Self {
            triple,
            cpu: "generic".to_string(),
            features: String::new(),
            opt_level: CodeGenOptLevel::Default,
            reloc_mode: RelocMode::Default,
            code_model: CodeModel::Default,
        }
    }

    /// Set CPU name
    pub fn with_cpu(mut self, cpu: &str) -> Self {
        self.cpu = cpu.to_string();
        self
    }

    /// Set CPU features
    pub fn with_features(mut self, features: &str) -> Self {
        self.features = features.to_string();
        self
    }

    /// Add CPU features (appends to existing)
    pub fn add_features(mut self, features: &str) -> Self {
        if self.features.is_empty() {
            self.features = features.to_string();
        } else {
            self.features = format!("{},{}", self.features, features);
        }
        self
    }

    /// Set optimization level
    pub fn with_opt_level(mut self, level: CodeGenOptLevel) -> Self {
        self.opt_level = level;
        self
    }

    /// Set relocation mode
    pub fn with_reloc_mode(mut self, mode: RelocMode) -> Self {
        self.reloc_mode = mode;
        self
    }

    /// Set code model
    pub fn with_code_model(mut self, model: CodeModel) -> Self {
        self.code_model = model;
        self
    }

    /// Configure for release build (aggressive optimization, PIC)
    pub fn release(mut self) -> Self {
        self.opt_level = CodeGenOptLevel::Aggressive;
        self.reloc_mode = RelocMode::PIC;
        self
    }

    /// Configure for debug build (no optimization)
    pub fn debug(mut self) -> Self {
        self.opt_level = CodeGenOptLevel::None;
        self
    }

    /// Common x86_64 configurations
    pub fn x86_64_v3() -> Self {
        Self::for_triple(Triple::x86_64_linux_gnu())
            .with_cpu("x86-64-v3")
            .with_features("+avx2,+fma,+bmi2,+lzcnt")
    }

    pub fn x86_64_v4() -> Self {
        Self::for_triple(Triple::x86_64_linux_gnu())
            .with_cpu("x86-64-v4")
            .with_features("+avx512f,+avx512vl,+avx512bw,+avx512dq,+avx512cd")
    }

    /// AMD Zen 4 configuration
    pub fn zen4() -> Self {
        Self::for_triple(Triple::x86_64_linux_gnu())
            .with_cpu("znver4")
            .with_features("+avx512f,+avx512vl,+avx512bw,+avx512dq,+avx512cd,+avx512bf16,+avx512vbmi,+avx512vbmi2,+avx512vnni")
    }

    /// Apple M-series configuration
    pub fn apple_m_series() -> Self {
        Self::for_triple(Triple::aarch64_apple_darwin())
            .with_cpu("apple-m1")
            .with_features("+neon,+fp-armv8,+crypto")
    }
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self::native()
    }
}

/// LLVM Target
pub struct Target {
    target: LLVMTargetRef,
}

impl Target {
    /// Get target from triple
    pub fn from_triple(triple: &Triple) -> LlvmResult<Self> {
        let triple_c = triple.as_c_str();
        let mut target: LLVMTargetRef = ptr::null_mut();
        let mut error: *mut libc::c_char = ptr::null_mut();

        let result = unsafe {
            LLVMGetTargetFromTriple(triple_c.as_ptr(), &mut target, &mut error)
        };

        if result != 0 || target.is_null() {
            let msg = if error.is_null() {
                format!("Target not found for triple: {}", triple)
            } else {
                let s = unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() };
                unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(error) };
                s
            };
            return Err(LlvmError::TargetNotFound(msg));
        }

        Ok(Self { target })
    }

    /// Get target name
    pub fn name(&self) -> String {
        let ptr = unsafe { LLVMGetTargetName(self.target) };
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
        }
    }

    /// Get target description
    pub fn description(&self) -> String {
        let ptr = unsafe { LLVMGetTargetDescription(self.target) };
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
        }
    }

    /// Check if target has JIT
    pub fn has_jit(&self) -> bool {
        unsafe { LLVMTargetHasJIT(self.target) != 0 }
    }

    /// Check if target has target machine
    pub fn has_target_machine(&self) -> bool {
        unsafe { LLVMTargetHasTargetMachine(self.target) != 0 }
    }

    /// Check if target has asm backend
    pub fn has_asm_backend(&self) -> bool {
        unsafe { LLVMTargetHasAsmBackend(self.target) != 0 }
    }

    /// Create target machine from this target
    pub fn create_machine(&self, config: &TargetConfig) -> LlvmResult<TargetMachine> {
        let triple_c = config.triple.as_c_str();
        let cpu_c = to_c_string(&config.cpu);
        let features_c = to_c_string(&config.features);

        let machine = unsafe {
            LLVMCreateTargetMachine(
                self.target,
                triple_c.as_ptr(),
                cpu_c.as_ptr(),
                features_c.as_ptr(),
                config.opt_level.into(),
                config.reloc_mode.into(),
                config.code_model.into(),
            )
        };

        if machine.is_null() {
            return Err(LlvmError::TargetMachineError(
                "Failed to create target machine".to_string(),
            ));
        }

        Ok(TargetMachine { machine })
    }
}

/// LLVM Target Machine
pub struct TargetMachine {
    machine: LLVMTargetMachineRef,
}

impl TargetMachine {
    /// Create from configuration
    pub fn new(config: &TargetConfig) -> LlvmResult<Self> {
        // Ensure targets are initialized
        if config.triple.0.contains("native") || config.triple == Triple::host() {
            initialize_native_target();
        } else {
            initialize_all_targets();
        }

        let target = Target::from_triple(&config.triple)?;
        target.create_machine(config)
    }

    /// Create for native host
    pub fn native() -> LlvmResult<Self> {
        Self::new(&TargetConfig::native())
    }

    /// Get raw pointer
    pub fn as_ptr(&self) -> LLVMTargetMachineRef {
        self.machine
    }

    /// Get target triple
    pub fn triple(&self) -> String {
        let ptr = unsafe { LLVMGetTargetMachineTriple(self.machine) };
        let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
        unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(ptr) };
        s
    }

    /// Get CPU
    pub fn cpu(&self) -> String {
        let ptr = unsafe { LLVMGetTargetMachineCPU(self.machine) };
        let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
        unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(ptr) };
        s
    }

    /// Get features
    pub fn features(&self) -> String {
        let ptr = unsafe { LLVMGetTargetMachineFeatureString(self.machine) };
        let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
        unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(ptr) };
        s
    }

    /// Get data layout string
    pub fn data_layout(&self) -> std::ffi::CString {
        let data_layout = unsafe { LLVMCreateTargetDataLayout(self.machine) };
        let ptr = unsafe { LLVMCopyStringRepOfTargetData(data_layout) };
        let s = unsafe { CStr::from_ptr(ptr).to_owned() };
        unsafe {
            verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(ptr);
            LLVMDisposeTargetData(data_layout);
        }
        s
    }

    /// Emit code to memory buffer
    pub fn emit_to_buffer(
        &self,
        module: LLVMModuleRef,
        file_type: FileType,
    ) -> LlvmResult<MemoryBuffer> {
        let mut buffer: LLVMMemoryBufferRef = ptr::null_mut();
        let mut error: *mut libc::c_char = ptr::null_mut();

        let result = unsafe {
            LLVMTargetMachineEmitToMemoryBuffer(
                self.machine,
                module,
                file_type.into(),
                &mut error,
                &mut buffer,
            )
        };

        if result != 0 || buffer.is_null() {
            let msg = if error.is_null() {
                "Code generation failed".to_string()
            } else {
                let s = unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() };
                unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(error) };
                s
            };
            return Err(LlvmError::CodegenError(msg));
        }

        Ok(unsafe { MemoryBuffer::from_raw(buffer) })
    }

    /// Emit code to file
    pub fn emit_to_file(
        &self,
        module: LLVMModuleRef,
        path: &str,
        file_type: FileType,
    ) -> LlvmResult<()> {
        let path_c = to_c_string(path);
        let mut error: *mut libc::c_char = ptr::null_mut();

        let result = unsafe {
            LLVMTargetMachineEmitToFile(
                self.machine,
                module,
                path_c.as_ptr() as *mut _,
                file_type.into(),
                &mut error,
            )
        };

        if result != 0 {
            let msg = if error.is_null() {
                format!("Failed to emit to {}", path)
            } else {
                let s = unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() };
                unsafe { verum_verum_llvm_sys::llvm::core::LLVMDisposeMessage(error) };
                s
            };
            return Err(LlvmError::CodegenError(msg));
        }

        Ok(())
    }

}

impl Drop for TargetMachine {
    fn drop(&mut self) {
        if !self.machine.is_null() {
            unsafe {
                LLVMDisposeTargetMachine(self.machine);
            }
        }
    }
}

// Safety: TargetMachine doesn't share mutable state
unsafe impl Send for TargetMachine {}
unsafe impl Sync for TargetMachine {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_target() {
        initialize_native_target();
        let config = TargetConfig::native();
        assert!(!config.triple.0.is_empty());
        assert!(!config.cpu.is_empty());
    }

    #[test]
    fn test_target_machine_creation() {
        initialize_native_target();
        let machine = TargetMachine::native();
        assert!(machine.is_ok());
        let machine = machine.unwrap();
        assert!(!machine.triple().is_empty());
    }

    #[test]
    fn test_target_config_builder() {
        let config = TargetConfig::for_triple(Triple::x86_64_linux_gnu())
            .with_cpu("skylake")
            .with_features("+avx2")
            .with_opt_level(CodeGenOptLevel::Aggressive)
            .release();

        assert_eq!(config.cpu, "skylake");
        assert!(config.features.contains("avx2"));
        assert_eq!(config.opt_level, CodeGenOptLevel::Aggressive);
        assert_eq!(config.reloc_mode, RelocMode::PIC);
    }
}
