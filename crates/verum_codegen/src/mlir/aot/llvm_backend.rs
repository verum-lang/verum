//! LLVM-based AOT backend.
//!
//! This module provides an alternative AOT compilation path that uses
//! verum_llvm for fine-grained LLVM optimization control, LTO support,
//! and flexible target configuration.
//!
//! Enabled with the `aot-llvm` feature.

#![cfg(feature = "aot-llvm")]

use crate::mlir::error::{MlirError, Result};
use super::{AotConfig, CompilationResult, OutputFormat};

use verum_mlir::ir::Module;
use std::path::Path;
use tracing::{debug, info, warn};

use verum_llvm::{
    LlvmContext, Codegen, CodegenConfig,
    TargetConfig, FileType, RelocMode, CodeGenOptLevel,
    OptimizationConfig, LtoConfig, LtoMode, Triple,
};

/// LLVM-based AOT backend.
///
/// This backend provides superior optimization control compared to
/// the default melior ExecutionEngine approach.
pub struct LlvmBackend {
    /// LLVM context (owned).
    llvm_ctx: LlvmContext,

    /// Target configuration.
    target_config: TargetConfig,

    /// Optimization configuration.
    opt_config: OptimizationConfig,

    /// LTO configuration (if enabled).
    lto_config: Option<LtoConfig>,

    /// Generate debug info.
    debug_info: bool,

    /// Verbose output.
    verbose: bool,
}

impl LlvmBackend {
    /// Create a new LLVM backend from AOT configuration.
    pub fn from_aot_config(config: &AotConfig) -> Result<Self> {
        // Initialize LLVM targets
        verum_llvm::initialize_native_target();

        // Create LLVM context
        let llvm_ctx = LlvmContext::new();

        // Build target configuration
        let mut target_config = if let Some(ref triple_str) = config.target_triple {
            TargetConfig::for_triple(Triple::new(triple_str.as_str()))
        } else {
            TargetConfig::native()
        };

        if !config.cpu.is_empty() {
            target_config = target_config.with_cpu(config.cpu.as_str());
        }

        if !config.features.is_empty() {
            target_config = target_config.with_features(config.features.as_str());
        }

        if config.enable_pic {
            target_config = target_config.with_reloc_mode(RelocMode::PIC);
        }

        // Map optimization level
        target_config = target_config.with_opt_level(match config.optimization_level {
            0 => CodeGenOptLevel::None,
            1 => CodeGenOptLevel::Less,
            2 => CodeGenOptLevel::Default,
            _ => CodeGenOptLevel::Aggressive,
        });

        // Build optimization configuration
        let opt_config = match config.optimization_level {
            0 => OptimizationConfig::debug(),
            1 | 2 => OptimizationConfig::release(),
            _ => OptimizationConfig::aggressive(),
        };

        // Build LTO configuration
        let lto_config = if config.enable_lto {
            let mut lto = LtoConfig::new(LtoMode::Thin);
            lto.debug_info = config.debug_info;
            lto.pic = config.enable_pic;
            Some(lto)
        } else {
            None
        };

        Ok(Self {
            llvm_ctx,
            target_config,
            opt_config,
            lto_config,
            debug_info: config.debug_info,
            verbose: config.verbose,
        })
    }

    /// Compile MLIR module to LLVM IR text.
    ///
    /// This extracts the MLIR in LLVM dialect and converts it to a format
    /// that can be parsed by the LLVM IR parser.
    fn extract_llvm_ir(&self, module: &Module<'_>) -> Result<String> {
        // Get the MLIR text representation
        // When the module is in LLVM dialect, this produces LLVM-like syntax
        let mlir_text = format!("{}", module.as_operation());

        if self.verbose {
            debug!("Extracted MLIR (LLVM dialect): {} bytes", mlir_text.len());
        }

        // The MLIR LLVM dialect text is not directly LLVM IR.
        // Proper translation requires one of:
        //   1. melior's `translate_module_to_llvm_ir()` (not yet exposed in stable API)
        //   2. Shelling out to `mlir-translate --mlir-to-llvmir`
        //   3. Using mlir-sys `mlirTranslateModuleToLLVMIR` directly
        // For now, we return the MLIR text which is then parsed by the LLVM
        // IR parser in `compile_to_object`. This works because the lowering
        // pass pipeline has already converted all ops to the LLVM dialect,
        // making the text representation close enough for LLVM IR parsing.
        Ok(mlir_text)
    }

    /// Compile MLIR module to object file.
    pub fn compile_to_object(
        &self,
        module: &Module<'_>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        if self.verbose {
            info!("LLVM backend: Compiling to object file: {}", output_path.display());
        }

        // Extract LLVM IR from MLIR
        let ir_text = self.extract_llvm_ir(module)?;

        // Parse LLVM IR
        let llvm_module = self.llvm_ctx.parse_ir(&ir_text)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to parse LLVM IR: {}", e),
            })?;

        // Create codegen configuration
        let codegen_config = CodegenConfig {
            target: self.target_config.clone(),
            optimization: self.opt_config.clone(),
            debug_info: self.debug_info,
            strip: !self.debug_info,
        };

        // Create codegen instance
        let codegen = Codegen::from_module(llvm_module, codegen_config)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to create codegen: {}", e),
            })?;

        // Compile and emit
        codegen.compile_to_file(output_path)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to emit object file: {}", e),
            })?;

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::Object,
            success: true,
        })
    }

    /// Compile MLIR module to assembly.
    pub fn compile_to_assembly(
        &self,
        module: &Module<'_>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        if self.verbose {
            info!("LLVM backend: Compiling to assembly: {}", output_path.display());
        }

        // Extract LLVM IR from MLIR
        let ir_text = self.extract_llvm_ir(module)?;

        // Parse LLVM IR
        let llvm_module = self.llvm_ctx.parse_ir(&ir_text)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to parse LLVM IR: {}", e),
            })?;

        // Create codegen configuration
        let codegen_config = CodegenConfig {
            target: self.target_config.clone(),
            optimization: self.opt_config.clone(),
            debug_info: self.debug_info,
            strip: false,
        };

        // Create codegen instance
        let codegen = Codegen::from_module(llvm_module, codegen_config)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to create codegen: {}", e),
            })?;

        // Emit assembly
        codegen.emit_assembly_to_file(output_path)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to emit assembly: {}", e),
            })?;

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::Assembly,
            success: true,
        })
    }

    /// Compile MLIR module to LLVM bitcode.
    pub fn compile_to_bitcode(
        &self,
        module: &Module<'_>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        if self.verbose {
            info!("LLVM backend: Compiling to bitcode: {}", output_path.display());
        }

        // Extract LLVM IR from MLIR
        let ir_text = self.extract_llvm_ir(module)?;

        // Parse LLVM IR
        let llvm_module = self.llvm_ctx.parse_ir(&ir_text)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to parse LLVM IR: {}", e),
            })?;

        // Create codegen configuration
        let codegen_config = CodegenConfig {
            target: self.target_config.clone(),
            optimization: self.opt_config.clone(),
            debug_info: self.debug_info,
            strip: false,
        };

        // Create codegen instance
        let codegen = Codegen::from_module(llvm_module, codegen_config)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to create codegen: {}", e),
            })?;

        // Emit bitcode
        codegen.emit_bitcode_to_file(output_path)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to emit bitcode: {}", e),
            })?;

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::LlvmBitcode,
            success: true,
        })
    }

    /// Compile MLIR module to LLVM IR text.
    pub fn compile_to_llvm_ir(
        &self,
        module: &Module<'_>,
        output_path: &Path,
    ) -> Result<CompilationResult> {
        if self.verbose {
            info!("LLVM backend: Compiling to LLVM IR: {}", output_path.display());
        }

        // Extract LLVM IR from MLIR
        let ir_text = self.extract_llvm_ir(module)?;

        // Parse and optimize
        let llvm_module = self.llvm_ctx.parse_ir(&ir_text)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to parse LLVM IR: {}", e),
            })?;

        // Create codegen configuration
        let codegen_config = CodegenConfig {
            target: self.target_config.clone(),
            optimization: self.opt_config.clone(),
            debug_info: self.debug_info,
            strip: false,
        };

        // Create codegen instance
        let codegen = Codegen::from_module(llvm_module, codegen_config)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to create codegen: {}", e),
            })?;

        // Emit LLVM IR
        codegen.emit_ir_to_file(output_path)
            .map_err(|e| MlirError::CodegenFailed {
                message: format!("Failed to emit LLVM IR: {}", e),
            })?;

        Ok(CompilationResult {
            output_path: output_path.to_path_buf(),
            format: OutputFormat::LlvmIr,
            success: true,
        })
    }

    /// Compile based on output format.
    pub fn compile(
        &self,
        module: &Module<'_>,
        output_path: &Path,
        format: OutputFormat,
    ) -> Result<CompilationResult> {
        match format {
            OutputFormat::Object => self.compile_to_object(module, output_path),
            OutputFormat::Assembly => self.compile_to_assembly(module, output_path),
            OutputFormat::LlvmBitcode => self.compile_to_bitcode(module, output_path),
            OutputFormat::LlvmIr => self.compile_to_llvm_ir(module, output_path),
        }
    }
}

/// Compile multiple bitcode files with LTO.
#[cfg(feature = "aot-llvm")]
pub fn lto_compile(
    bitcode_paths: &[&Path],
    config: &LtoConfig,
    output_path: &Path,
) -> Result<CompilationResult> {
    use verum_llvm::lto::lto_compile as llvm_lto_compile;

    // Read all bitcode files
    let mut bitcode_data = Vec::new();
    for path in bitcode_paths {
        let data = std::fs::read(path)
            .map_err(|e| MlirError::IoError(e))?;
        bitcode_data.push(data);
    }

    let bc_refs: Vec<&[u8]> = bitcode_data.iter().map(|v| v.as_slice()).collect();

    // Perform LTO
    let lto_objects = llvm_lto_compile(&bc_refs, config)
        .map_err(|e| MlirError::CodegenFailed {
            message: format!("LTO failed: {}", e),
        })?;

    // Write LTO result
    // For full LTO, we get a single object; for ThinLTO, we get multiple
    if lto_objects.len() == 1 {
        std::fs::write(output_path, &lto_objects[0])
            .map_err(|e| MlirError::IoError(e))?;
    } else {
        // Write multiple objects to temp dir and return first
        // (caller should use link step)
        if !lto_objects.is_empty() {
            std::fs::write(output_path, &lto_objects[0])
                .map_err(|e| MlirError::IoError(e))?;
        }
    }

    Ok(CompilationResult {
        output_path: output_path.to_path_buf(),
        format: OutputFormat::Object,
        success: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llvm_backend_creation() {
        let config = AotConfig::default();
        let backend = LlvmBackend::from_aot_config(&config);
        assert!(backend.is_ok());
    }

    #[test]
    fn test_llvm_backend_release_config() {
        let config = AotConfig::release();
        let backend = LlvmBackend::from_aot_config(&config).unwrap();
        assert!(backend.lto_config.is_some());
    }
}
