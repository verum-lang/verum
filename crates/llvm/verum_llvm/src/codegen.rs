//! Code generation and object file emission
//!
//! High-level API for generating object files, assembly, and bitcode.

use std::path::Path;

use crate::context::LlvmContext;
use crate::error::{LlvmError, LlvmResult};
use crate::module::LlvmModule;
use crate::passes::OptimizationConfig;
use crate::target::{FileType, TargetConfig, TargetMachine};

/// Code generation configuration
#[derive(Debug, Clone)]
pub struct CodegenConfig {
    /// Target configuration
    pub target: TargetConfig,
    /// Optimization configuration
    pub optimization: OptimizationConfig,
    /// Generate debug info
    pub debug_info: bool,
    /// Strip symbols
    pub strip: bool,
}

impl CodegenConfig {
    /// Create debug build configuration
    pub fn debug() -> Self {
        Self {
            target: TargetConfig::native().debug(),
            optimization: OptimizationConfig::debug(),
            debug_info: true,
            strip: false,
        }
    }

    /// Create release build configuration
    pub fn release() -> Self {
        Self {
            target: TargetConfig::native().release(),
            optimization: OptimizationConfig::release(),
            debug_info: false,
            strip: true,
        }
    }

    /// Create aggressive optimization configuration
    pub fn aggressive() -> Self {
        Self {
            target: TargetConfig::native().release(),
            optimization: OptimizationConfig::aggressive(),
            debug_info: false,
            strip: true,
        }
    }

    /// Set target
    pub fn with_target(mut self, target: TargetConfig) -> Self {
        self.target = target;
        self
    }

    /// Set optimization
    pub fn with_optimization(mut self, opt: OptimizationConfig) -> Self {
        self.optimization = opt;
        self
    }

    /// Set debug info
    pub fn with_debug_info(mut self, enable: bool) -> Self {
        self.debug_info = enable;
        self
    }

    /// Set strip
    pub fn with_strip(mut self, enable: bool) -> Self {
        self.strip = enable;
        self
    }
}

impl Default for CodegenConfig {
    fn default() -> Self {
        Self::release()
    }
}

/// High-level code generator
///
/// Provides a simple interface for compiling LLVM IR to native code.
pub struct Codegen<'ctx> {
    module: LlvmModule<'ctx>,
    target_machine: TargetMachine,
    config: CodegenConfig,
}

impl<'ctx> Codegen<'ctx> {
    /// Create new code generator from IR text
    pub fn from_ir(ctx: &'ctx LlvmContext, ir: &str, config: CodegenConfig) -> LlvmResult<Self> {
        let module = ctx.parse_ir(ir)?;
        Self::from_module(module, config)
    }

    /// Create new code generator from bitcode
    pub fn from_bitcode(ctx: &'ctx LlvmContext, bc: &[u8], config: CodegenConfig) -> LlvmResult<Self> {
        let module = ctx.parse_bitcode(bc)?;
        Self::from_module(module, config)
    }

    /// Create new code generator from module
    pub fn from_module(module: LlvmModule<'ctx>, config: CodegenConfig) -> LlvmResult<Self> {
        let target_machine = TargetMachine::new(&config.target)?;

        // Set target triple and data layout on module
        module.set_target_triple(&config.target.triple.0);
        module.set_data_layout_from_target(&target_machine);

        Ok(Self {
            module,
            target_machine,
            config,
        })
    }

    /// Get reference to module
    pub fn module(&self) -> &LlvmModule<'ctx> {
        &self.module
    }

    /// Get mutable reference to module
    pub fn module_mut(&mut self) -> &mut LlvmModule<'ctx> {
        &mut self.module
    }

    /// Verify the module
    pub fn verify(&self) -> LlvmResult<()> {
        self.module.verify()
    }

    /// Run optimization passes
    pub fn optimize(&self) -> LlvmResult<()> {
        self.module.optimize(&self.target_machine, &self.config.optimization)
    }

    /// Generate object file to memory
    pub fn emit_object(&self) -> LlvmResult<Vec<u8>> {
        let buffer = self.target_machine.emit_to_buffer(
            self.module.as_ptr(),
            FileType::Object,
        )?;
        Ok(buffer.as_slice().to_vec())
    }

    /// Generate object file to disk
    pub fn emit_object_to_file(&self, path: impl AsRef<Path>) -> LlvmResult<()> {
        self.target_machine.emit_to_file(
            self.module.as_ptr(),
            path.as_ref().to_str().ok_or_else(|| {
                LlvmError::InvalidConfig("Invalid path".to_string())
            })?,
            FileType::Object,
        )
    }

    /// Generate assembly to memory
    pub fn emit_assembly(&self) -> LlvmResult<String> {
        let buffer = self.target_machine.emit_to_buffer(
            self.module.as_ptr(),
            FileType::Assembly,
        )?;
        String::from_utf8(buffer.as_slice().to_vec())
            .map_err(|_| LlvmError::InvalidUtf8)
    }

    /// Generate assembly to disk
    pub fn emit_assembly_to_file(&self, path: impl AsRef<Path>) -> LlvmResult<()> {
        self.target_machine.emit_to_file(
            self.module.as_ptr(),
            path.as_ref().to_str().ok_or_else(|| {
                LlvmError::InvalidConfig("Invalid path".to_string())
            })?,
            FileType::Assembly,
        )
    }

    /// Generate bitcode to memory
    pub fn emit_bitcode(&self) -> Vec<u8> {
        self.module.write_bitcode_to_buffer().as_slice().to_vec()
    }

    /// Generate bitcode to disk
    pub fn emit_bitcode_to_file(&self, path: impl AsRef<Path>) -> LlvmResult<()> {
        self.module.write_bitcode_to_file(
            path.as_ref().to_str().ok_or_else(|| {
                LlvmError::InvalidConfig("Invalid path".to_string())
            })?
        )
    }

    /// Generate LLVM IR text to memory
    pub fn emit_ir(&self) -> String {
        self.module.print_to_string()
    }

    /// Generate LLVM IR text to disk
    pub fn emit_ir_to_file(&self, path: impl AsRef<Path>) -> LlvmResult<()> {
        self.module.print_to_file(
            path.as_ref().to_str().ok_or_else(|| {
                LlvmError::InvalidConfig("Invalid path".to_string())
            })?
        )
    }

    /// Full compilation pipeline: verify, optimize, emit
    pub fn compile(&self) -> LlvmResult<Vec<u8>> {
        self.verify()?;
        self.optimize()?;
        self.emit_object()
    }

    /// Full compilation pipeline to file
    pub fn compile_to_file(&self, path: impl AsRef<Path>) -> LlvmResult<()> {
        self.verify()?;
        self.optimize()?;
        self.emit_object_to_file(path)
    }
}

/// Compile LLVM IR to object file (convenience function)
pub fn compile_ir_to_object(ir: &str, config: CodegenConfig) -> LlvmResult<Vec<u8>> {
    let ctx = LlvmContext::new();
    let codegen = Codegen::from_ir(&ctx, ir, config)?;
    codegen.compile()
}

/// Compile LLVM bitcode to object file (convenience function)
pub fn compile_bitcode_to_object(bc: &[u8], config: CodegenConfig) -> LlvmResult<Vec<u8>> {
    let ctx = LlvmContext::new();
    let codegen = Codegen::from_bitcode(&ctx, bc, config)?;
    codegen.compile()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initialize_native_target;

    #[test]
    fn test_simple_compilation() {
        initialize_native_target();

        let ir = r#"
            define i32 @main() {
            entry:
                ret i32 0
            }
        "#;

        let result = compile_ir_to_object(ir, CodegenConfig::debug());
        assert!(result.is_ok());
        let object = result.unwrap();
        assert!(!object.is_empty());
    }

    #[test]
    fn test_codegen_pipeline() {
        initialize_native_target();

        let ctx = LlvmContext::new();
        let ir = r#"
            define i32 @add(i32 %a, i32 %b) {
                %sum = add i32 %a, %b
                ret i32 %sum
            }
        "#;

        let codegen = Codegen::from_ir(&ctx, ir, CodegenConfig::release()).unwrap();
        assert!(codegen.verify().is_ok());
        assert!(codegen.optimize().is_ok());

        let asm = codegen.emit_assembly().unwrap();
        assert!(asm.contains("add"));
    }
}
