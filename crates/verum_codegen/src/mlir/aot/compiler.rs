//! AOT compiler implementation.
//!
//! Compiles MLIR modules to object files and executables.

use crate::mlir::error::{MlirError, Result};
use crate::mlir::passes::{PassPipeline, PassConfig};
use crate::mlir::context::MlirContext;

use verum_mlir::{
    ir::Module,
    ExecutionEngine,
    pass::PassManager,
};
use verum_common::{List, Text};
use std::path::Path;

/// AOT compilation configuration.
#[derive(Debug, Clone)]
pub struct AotConfig {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu").
    pub target_triple: Option<Text>,

    /// CPU model (e.g., "generic", "native").
    pub cpu: Text,

    /// CPU features (e.g., "+avx2,+fma").
    pub features: Text,

    /// Optimization level (0-3).
    pub optimization_level: usize,

    /// Enable LTO (Link-Time Optimization).
    pub enable_lto: bool,

    /// Enable PIC (Position Independent Code).
    pub enable_pic: bool,

    /// Generate debug information.
    pub debug_info: bool,

    /// Output format.
    pub output_format: OutputFormat,

    /// Verbose output.
    pub verbose: bool,
}

/// Output format for AOT compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Object file (.o).
    Object,

    /// LLVM IR (.ll).
    LlvmIr,

    /// LLVM bitcode (.bc).
    LlvmBitcode,

    /// Assembly (.s).
    Assembly,
}

impl Default for AotConfig {
    fn default() -> Self {
        Self {
            target_triple: None,
            cpu: Text::from("generic"),
            features: Text::new(),
            optimization_level: 2,
            enable_lto: false,
            enable_pic: true,
            debug_info: false,
            output_format: OutputFormat::Object,
            verbose: false,
        }
    }
}

impl AotConfig {
    /// Create a new AOT configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set target triple.
    pub fn with_target_triple(mut self, triple: impl Into<Text>) -> Self {
        self.target_triple = Some(triple.into());
        self
    }

    /// Set CPU model.
    pub fn with_cpu(mut self, cpu: impl Into<Text>) -> Self {
        self.cpu = cpu.into();
        self
    }

    /// Set CPU features.
    pub fn with_features(mut self, features: impl Into<Text>) -> Self {
        self.features = features.into();
        self
    }

    /// Set optimization level.
    pub fn with_optimization_level(mut self, level: usize) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Enable LTO.
    pub fn with_lto(mut self, enable: bool) -> Self {
        self.enable_lto = enable;
        self
    }

    /// Enable PIC.
    pub fn with_pic(mut self, enable: bool) -> Self {
        self.enable_pic = enable;
        self
    }

    /// Enable debug info.
    pub fn with_debug_info(mut self, enable: bool) -> Self {
        self.debug_info = enable;
        self
    }

    /// Set output format.
    pub fn with_output_format(mut self, format: OutputFormat) -> Self {
        self.output_format = format;
        self
    }

    /// Enable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Create release configuration.
    pub fn release() -> Self {
        Self {
            optimization_level: 3,
            enable_lto: true,
            debug_info: false,
            ..Default::default()
        }
    }

    /// Create debug configuration.
    pub fn debug() -> Self {
        Self {
            optimization_level: 0,
            debug_info: true,
            ..Default::default()
        }
    }
}

/// AOT compiler for Verum MLIR.
pub struct AotCompiler<'c> {
    /// Configuration.
    config: AotConfig,

    /// MLIR context.
    mlir_ctx: &'c MlirContext,

    /// LLVM backend (when feature enabled).
    #[cfg(feature = "aot-llvm")]
    llvm_backend: Option<super::llvm_backend::LlvmBackend>,
}

impl<'c> AotCompiler<'c> {
    /// Create a new AOT compiler.
    pub fn new(mlir_ctx: &'c MlirContext, config: AotConfig) -> Self {
        #[cfg(feature = "aot-llvm")]
        let llvm_backend = super::llvm_backend::LlvmBackend::from_aot_config(&config).ok();

        // Phase-not-realised tracing: when the LLVM backend isn't
        // active (the `aot-llvm` feature isn't declared in
        // Cargo.toml at present, so the backend file is gated out
        // entirely; even when the feature is added, `from_aot_config`
        // can fail and yield None), the fallback path uses MLIR's
        // ExecutionEngine which only consumes `optimization_level`,
        // `verbose`, and `output_format`. Surface a warning when the
        // user has set fields that the fallback can't honour, so a
        // `[codegen.aot] target_triple = "..."` setting in
        // verum.toml doesn't silently produce a host-native object.
        #[cfg(not(feature = "aot-llvm"))]
        if config.target_triple.is_some()
            || config.cpu.as_str() != "generic"
            || !config.features.is_empty()
            || config.enable_lto
            || !config.enable_pic
            || config.debug_info
        {
            tracing::warn!(
                "AotConfig surface: target_triple={:?}, cpu={:?}, features={:?}, \
                 enable_lto={}, enable_pic={}, debug_info={} (these fields land \
                 on the MLIR-AOT config but the LLVM backend (`aot-llvm` feature) \
                 is not built — the fallback ExecutionEngine path consumes only \
                 optimization_level, verbose, and output_format)",
                config.target_triple,
                config.cpu,
                config.features,
                config.enable_lto,
                config.enable_pic,
                config.debug_info,
            );
        }

        Self {
            config,
            mlir_ctx,
            #[cfg(feature = "aot-llvm")]
            llvm_backend,
        }
    }

    /// Create a new AOT compiler with explicit backend choice.
    #[cfg(feature = "aot-llvm")]
    pub fn with_llvm_backend(mlir_ctx: &'c MlirContext, config: AotConfig) -> Result<Self> {
        let llvm_backend = super::llvm_backend::LlvmBackend::from_aot_config(&config)?;

        Ok(Self {
            config,
            mlir_ctx,
            llvm_backend: Some(llvm_backend),
        })
    }

    /// Check if LLVM backend is available and enabled.
    #[cfg(feature = "aot-llvm")]
    fn use_llvm_backend(&self) -> bool {
        self.llvm_backend.is_some()
    }

    #[cfg(not(feature = "aot-llvm"))]
    fn use_llvm_backend(&self) -> bool {
        false
    }

    /// Compile a module to an object file.
    pub fn compile_to_object(
        &self,
        module: &Module<'c>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        #[cfg(feature = "aot-llvm")]
        if let Some(ref backend) = self.llvm_backend {
            return backend.compile_to_object(module, output_path);
        }

        // Fallback: use verum_mlir ExecutionEngine
        if self.config.verbose {
            tracing::info!("Compiling to object file: {}", output_path.display());
        }

        // Create execution engine (used for object dump)
        let engine = ExecutionEngine::new(
            module,
            self.config.optimization_level,
            &[],
            true, // Enable object cache for dump
        );

        // Dump to object file
        engine.dump_to_object_file(output_path.to_str().unwrap_or("output.o"));

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::Object,
            success: true,
        })
    }

    /// Compile a module to LLVM IR.
    pub fn compile_to_llvm_ir(
        &self,
        module: &Module<'c>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        #[cfg(feature = "aot-llvm")]
        if let Some(ref backend) = self.llvm_backend {
            return backend.compile_to_llvm_ir(module, output_path);
        }

        // Fallback: dump MLIR text
        if self.config.verbose {
            tracing::info!("Compiling to LLVM IR: {}", output_path.display());
        }

        // Get MLIR as string (which is already in LLVM dialect)
        let ir = format!("{}", module.as_operation());

        // Write to file
        std::fs::write(output_path, ir)
            .map_err(|e| MlirError::IoError(e))?;

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::LlvmIr,
            success: true,
        })
    }

    /// Compile a module to assembly.
    pub fn compile_to_assembly(
        &self,
        module: &Module<'c>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        #[cfg(feature = "aot-llvm")]
        if let Some(ref backend) = self.llvm_backend {
            return backend.compile_to_assembly(module, output_path);
        }

        Err(MlirError::not_implemented("Assembly output requires aot-llvm feature"))
    }

    /// Compile a module to LLVM bitcode.
    pub fn compile_to_bitcode(
        &self,
        module: &Module<'c>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        #[cfg(feature = "aot-llvm")]
        if let Some(ref backend) = self.llvm_backend {
            return backend.compile_to_bitcode(module, output_path);
        }

        Err(MlirError::not_implemented("Bitcode output requires aot-llvm feature"))
    }

    /// Compile based on configuration.
    pub fn compile(
        &self,
        module: &Module<'c>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        match self.config.output_format {
            OutputFormat::Object => self.compile_to_object(module, output_path),
            OutputFormat::LlvmIr => self.compile_to_llvm_ir(module, output_path),
            OutputFormat::LlvmBitcode => self.compile_to_bitcode(module, output_path),
            OutputFormat::Assembly => self.compile_to_assembly(module, output_path),
        }
    }

    /// Get configuration.
    pub fn config(&self) -> &AotConfig {
        &self.config
    }

    /// Check if advanced features are available (LLVM backend).
    pub fn has_advanced_features(&self) -> bool {
        self.use_llvm_backend()
    }
}

/// Result of AOT compilation.
#[derive(Debug)]
pub struct CompilationResult {
    /// Output file path.
    pub output_path: std::path::PathBuf,

    /// Output format.
    pub format: OutputFormat,

    /// Whether compilation was successful.
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aot_config_default() {
        let config = AotConfig::default();
        assert_eq!(config.optimization_level, 2);
        assert!(!config.enable_lto);
        assert!(config.enable_pic);
        assert!(!config.debug_info);
    }

    #[test]
    fn test_aot_config_release() {
        let config = AotConfig::release();
        assert_eq!(config.optimization_level, 3);
        assert!(config.enable_lto);
        assert!(!config.debug_info);
    }

    #[test]
    fn test_aot_config_debug() {
        let config = AotConfig::debug();
        assert_eq!(config.optimization_level, 0);
        assert!(config.debug_info);
    }

    #[test]
    fn test_aot_config_builder() {
        let config = AotConfig::new()
            .with_target_triple("x86_64-unknown-linux-gnu")
            .with_cpu("native")
            .with_optimization_level(3)
            .with_lto(true);

        assert_eq!(config.target_triple.as_ref().unwrap().as_str(), "x86_64-unknown-linux-gnu");
        assert_eq!(config.cpu.as_str(), "native");
        assert_eq!(config.optimization_level, 3);
        assert!(config.enable_lto);
    }
}
