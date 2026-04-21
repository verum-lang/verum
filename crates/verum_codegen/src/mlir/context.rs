//! MLIR context and codegen infrastructure.
//!
//! Provides the main entry points for MLIR-based code generation:
//! - `MlirContext`: Wrapper around melior Context with dialect registration
//! - `MlirCodegen`: High-level codegen interface (VBC → MLIR for GPU path)
//! - `MlirConfig`: Configuration options

use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::VerumDialect;
use crate::mlir::passes::{PassPipeline, PassConfig, GpuPassPipeline, GpuPassConfig, GpuPipelineResult};
use crate::mlir::vbc_lowering::{VbcToMlirGpuLowering, GpuLoweringConfig, GpuLoweringStats, GpuTarget};

use verum_mlir::{
    Context,
    ir::{Module, Location, Block, Region, Type, Value},
    ir::operation::OperationLike,
    dialect::DialectRegistry,
    pass::PassManager,
    utility::{register_all_llvm_translations, register_used_dialects},
};
use verum_common::{List, Map, Text};
use verum_types::TypeRegistry;
use verum_vbc::module::VbcModule;

use indexmap::IndexMap;
use parking_lot::RwLock;
use std::sync::Arc;

/// Configuration for MLIR codegen.
#[derive(Debug, Clone)]
pub struct MlirConfig {
    /// Module name for the generated MLIR module.
    pub module_name: Text,

    /// Enable CBGR optimization pass.
    pub enable_cbgr_optimization: bool,

    /// Enable context monomorphization.
    pub enable_context_mono: bool,

    /// Enable standard MLIR optimizations (CSE, canonicalize, etc.).
    pub enable_standard_opts: bool,

    /// Optimization level (0-3).
    pub optimization_level: u8,

    /// Enable debug information generation.
    pub debug_info: bool,

    /// Enable verbose output for debugging.
    pub verbose: bool,

    /// Target triple for cross-compilation (None = native).
    pub target_triple: Option<Text>,

    /// Shared library paths for JIT symbol resolution.
    pub shared_library_paths: List<Text>,
}

impl Default for MlirConfig {
    fn default() -> Self {
        Self {
            module_name: Text::from("verum_module"),
            enable_cbgr_optimization: true,
            enable_context_mono: true,
            enable_standard_opts: true,
            optimization_level: 2,
            debug_info: false,
            verbose: false,
            target_triple: None,
            shared_library_paths: List::new(),
        }
    }
}

impl MlirConfig {
    /// Create a new configuration with the given module name.
    pub fn new(module_name: impl Into<Text>) -> Self {
        Self {
            module_name: module_name.into(),
            ..Default::default()
        }
    }

    /// Set optimization level.
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Enable or disable CBGR optimization.
    pub fn with_cbgr_optimization(mut self, enable: bool) -> Self {
        self.enable_cbgr_optimization = enable;
        self
    }

    /// Enable or disable context monomorphization.
    pub fn with_context_mono(mut self, enable: bool) -> Self {
        self.enable_context_mono = enable;
        self
    }

    /// Enable or disable debug info.
    pub fn with_debug_info(mut self, enable: bool) -> Self {
        self.debug_info = enable;
        self
    }

    /// Enable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Set target triple for cross-compilation.
    pub fn with_target_triple(mut self, triple: impl Into<Text>) -> Self {
        self.target_triple = Some(triple.into());
        self
    }

    /// Add shared library path for JIT.
    pub fn with_shared_library(mut self, path: impl Into<Text>) -> Self {
        self.shared_library_paths.push(path.into());
        self
    }
}

/// MLIR context wrapper with Verum dialect support.
pub struct MlirContext {
    /// Underlying melior context.
    context: Context,

    /// Registered dialects.
    dialects_loaded: bool,
}

impl MlirContext {
    /// Create a new MLIR context with all required dialects.
    ///
    /// This properly registers all dialects (including arith, func, scf, etc.)
    /// to enable type inference in MLIR operations.
    pub fn new() -> Result<Self> {
        let context = Context::new();

        // Create a dialect registry and register only the dialects Verum
        // actually targets. The explicit list lets link-time DCE drop
        // unused dialects (OpenMP, SparseTensor, Async, Shape, Quant,
        // PDL/IRDL, EmitC, …) — see `register_used_dialects` docstring.
        let registry = DialectRegistry::new();
        register_used_dialects(&registry);
        context.append_dialect_registry(&registry);

        // Now load all available dialects
        context.load_all_available_dialects();

        // Register LLVM translations for JIT compilation
        register_all_llvm_translations(&context);

        Ok(Self {
            context,
            dialects_loaded: true,
        })
    }

    /// Get a reference to the underlying melior context.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// Create an unknown location.
    pub fn unknown_location(&self) -> Location<'_> {
        Location::unknown(&self.context)
    }

    /// Create a file location.
    pub fn file_location(&self, filename: &str, line: u32, column: u32) -> Location<'_> {
        Location::new(&self.context, filename, line as usize, column as usize)
    }

    /// Parse an MLIR type from string.
    pub fn parse_type(&self, type_str: &str) -> Result<Type<'_>> {
        Type::parse(&self.context, type_str)
            .ok_or_else(|| MlirError::type_translation(type_str, "failed to parse type"))
    }

    /// Get integer type with given bit width.
    pub fn integer_type(&self, bits: u32) -> Type<'_> {
        verum_mlir::ir::r#type::IntegerType::new(&self.context, bits).into()
    }

    /// Get index type.
    pub fn index_type(&self) -> Type<'_> {
        Type::index(&self.context)
    }

    /// Get f32 type.
    pub fn f32_type(&self) -> Type<'_> {
        Type::float32(&self.context)
    }

    /// Get f64 type.
    pub fn f64_type(&self) -> Type<'_> {
        Type::float64(&self.context)
    }
}

impl Default for MlirContext {
    fn default() -> Self {
        Self::new().expect("Failed to create MLIR context")
    }
}

/// Symbol table entry for tracking defined values.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    /// The MLIR value.
    pub value_index: usize,

    /// The Verum type (as string for now).
    pub verum_type: Text,

    /// Whether this is mutable.
    pub is_mutable: bool,
}

/// Main MLIR code generator for GPU path.
///
/// This provides the GPU compilation path:
/// VBC → MLIR → GPU binaries (CUDA/ROCm/Vulkan/Metal)
pub struct MlirCodegen<'ctx> {
    /// Configuration.
    config: MlirConfig,

    /// MLIR context.
    mlir_ctx: &'ctx MlirContext,

    /// The MLIR module being generated.
    module: Option<Module<'ctx>>,

    /// Symbol table for variables.
    symbols: IndexMap<Text, SymbolEntry>,

    /// Function registry.
    functions: IndexMap<Text, FunctionEntry>,

    /// Type registry from verum_types (optional).
    type_registry: Option<TypeRegistry>,

    /// GPU lowering statistics.
    gpu_stats: Option<GpuLoweringStats>,

    /// Whether optimization has been run.
    optimized: bool,
}

/// Entry for a registered function.
#[derive(Debug, Clone)]
pub struct FunctionEntry {
    /// Function name.
    pub name: Text,

    /// Parameter types.
    pub param_types: List<Text>,

    /// Return type.
    pub return_type: Text,

    /// Whether this is async.
    pub is_async: bool,

    /// Context requirements.
    pub contexts: List<Text>,
}

impl<'ctx> MlirCodegen<'ctx> {
    /// Create a new MLIR codegen instance.
    pub fn new(mlir_ctx: &'ctx MlirContext, config: MlirConfig) -> Result<Self> {
        let location = mlir_ctx.unknown_location();
        let module = Module::new(location);

        Ok(Self {
            config,
            mlir_ctx,
            module: Some(module),
            symbols: IndexMap::new(),
            functions: IndexMap::new(),
            type_registry: None,
            gpu_stats: None,
            optimized: false,
        })
    }

    /// Set type registry for type inference during codegen.
    pub fn set_type_registry(&mut self, registry: TypeRegistry) {
        self.type_registry = Some(registry);
    }

    /// Lower a VBC module to MLIR (GPU compilation path).
    ///
    /// This is the primary method for GPU code generation:
    /// AST → VBC → MLIR → GPU binaries.
    ///
    /// # Arguments
    ///
    /// * `vbc_module` - The VBC bytecode module to lower
    /// * `gpu_target` - The GPU target platform (CUDA, ROCm, Vulkan, Metal)
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_codegen::mlir::{MlirCodegen, MlirConfig, GpuTarget};
    ///
    /// let config = MlirConfig::default();
    /// let mut codegen = MlirCodegen::new(&mlir_ctx, config)?;
    ///
    /// // Lower VBC to MLIR for GPU
    /// codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda)?;
    ///
    /// // Run GPU-specific optimization passes
    /// codegen.optimize()?;
    /// ```
    pub fn lower_vbc_module(&mut self, vbc_module: &VbcModule, gpu_target: GpuTarget) -> Result<()> {
        let gpu_config = GpuLoweringConfig {
            target: gpu_target,
            opt_level: self.config.optimization_level,
            enable_tensor_cores: true,
            max_shared_memory: 48 * 1024, // 48KB default
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: self.config.debug_info,
        };

        let _module = self.module.as_ref()
            .ok_or_else(|| MlirError::internal("Module not initialized"))?;

        let mut lowering = VbcToMlirGpuLowering::new(
            self.mlir_ctx.context(),
            gpu_config,
        );

        // Lower the VBC module
        lowering.lower_module(vbc_module)
            .map_err(|e| MlirError::lowering(None, format!("VBC → MLIR lowering failed: {:?}", e)))?;

        // Store statistics
        self.gpu_stats = Some(lowering.stats().clone());

        // Verify the module
        self.verify()?;

        Ok(())
    }

    /// Run optimization passes.
    pub fn optimize(&mut self) -> Result<()> {
        if self.optimized {
            return Ok(());
        }

        let module = self.module.as_mut()
            .ok_or_else(|| MlirError::internal("Module not initialized"))?;

        let pass_config = PassConfig {
            enable_cbgr_elimination: self.config.enable_cbgr_optimization,
            enable_context_mono: self.config.enable_context_mono,
            enable_refinement_propagation: true,
            enable_standard_opts: self.config.enable_standard_opts,
            enable_early_opts: true,
            enable_late_opts: self.config.optimization_level >= 2,
            optimization_level: self.config.optimization_level,
            cbgr_aggressive: false,
            verbose: self.config.verbose,
            debug_ir_printing: false,
            verify_after_each_pass: true,
        };

        let pipeline = PassPipeline::new(self.mlir_ctx.context(), pass_config);
        pipeline.run(module)?;

        self.optimized = true;
        self.verify()?;

        Ok(())
    }

    /// Run the GPU pass pipeline (tensor → linalg → scf → gpu → target binary).
    pub fn optimize_gpu(&mut self, gpu_target: GpuTarget) -> Result<GpuPipelineResult> {
        let module = self.module.as_mut()
            .ok_or_else(|| MlirError::internal("Module not initialized"))?;

        let gpu_config = GpuPassConfig {
            target: gpu_target,
            optimization_level: self.config.optimization_level.min(3),
            enable_async: false,
            enable_tensor_cores: true,
            verbose: self.config.verbose,
            verify_after_each_phase: true,
        };

        let pipeline = GpuPassPipeline::new(self.mlir_ctx.context(), gpu_config);
        let result = pipeline.run(module)?;
        Ok(result)
    }

    /// Verify the MLIR module.
    pub fn verify(&self) -> Result<()> {
        let module = self.module.as_ref()
            .ok_or_else(|| MlirError::internal("Module not initialized"))?;

        if !module.as_operation().verify() {
            return Err(MlirError::verification("Module verification failed"));
        }

        Ok(())
    }

    /// Get the MLIR module as a string.
    pub fn get_mlir_string(&self) -> Result<Text> {
        let module = self.module.as_ref()
            .ok_or_else(|| MlirError::internal("Module not initialized"))?;

        Ok(Text::from(format!("{}", module.as_operation())))
    }

    /// Get GPU lowering statistics.
    pub fn gpu_stats(&self) -> Option<&GpuLoweringStats> {
        self.gpu_stats.as_ref()
    }

    /// Get a reference to the module.
    pub fn module(&self) -> Result<&Module<'ctx>> {
        self.module.as_ref()
            .ok_or_else(|| MlirError::internal("Module not initialized"))
    }

    /// Take ownership of the module (consumes the codegen).
    pub fn into_module(mut self) -> Result<Module<'ctx>> {
        self.module.take()
            .ok_or_else(|| MlirError::internal("Module already taken"))
    }

    /// Get configuration.
    pub fn config(&self) -> &MlirConfig {
        &self.config
    }

    /// Get the MLIR context.
    pub fn mlir_context(&self) -> &'ctx MlirContext {
        self.mlir_ctx
    }

    /// Define a symbol.
    pub fn define_symbol(&mut self, name: Text, entry: SymbolEntry) {
        self.symbols.insert(name, entry);
    }

    /// Look up a symbol.
    pub fn lookup_symbol(&self, name: &str) -> Option<&SymbolEntry> {
        self.symbols.get(name)
    }

    /// Register a function.
    pub fn register_function(&mut self, entry: FunctionEntry) {
        self.functions.insert(entry.name.clone(), entry);
    }

    /// Look up a function.
    pub fn lookup_function(&self, name: &str) -> Option<&FunctionEntry> {
        self.functions.get(name)
    }
}

/// Builder for MlirCodegen with fluent API.
pub struct MlirCodegenBuilder {
    config: MlirConfig,
}

impl MlirCodegenBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: MlirConfig::default(),
        }
    }

    /// Set module name.
    pub fn module_name(mut self, name: impl Into<Text>) -> Self {
        self.config.module_name = name.into();
        self
    }

    /// Set optimization level.
    pub fn optimization_level(mut self, level: u8) -> Self {
        self.config.optimization_level = level.min(3);
        self
    }

    /// Enable CBGR optimization.
    pub fn cbgr_optimization(mut self, enable: bool) -> Self {
        self.config.enable_cbgr_optimization = enable;
        self
    }

    /// Enable context monomorphization.
    pub fn context_mono(mut self, enable: bool) -> Self {
        self.config.enable_context_mono = enable;
        self
    }

    /// Enable debug info.
    pub fn debug_info(mut self, enable: bool) -> Self {
        self.config.debug_info = enable;
        self
    }

    /// Enable verbose output.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.config.verbose = verbose;
        self
    }

    /// Build the codegen instance.
    pub fn build(self, mlir_ctx: &MlirContext) -> Result<MlirCodegen<'_>> {
        MlirCodegen::new(mlir_ctx, self.config)
    }
}

impl Default for MlirCodegenBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mlir_context_creation() {
        let ctx = MlirContext::new().unwrap();
        assert!(ctx.dialects_loaded);
    }

    #[test]
    fn test_config_builder() {
        let config = MlirConfig::new("test_module")
            .with_optimization_level(3)
            .with_cbgr_optimization(true)
            .with_debug_info(true);

        assert_eq!(config.module_name.as_str(), "test_module");
        assert_eq!(config.optimization_level, 3);
        assert!(config.enable_cbgr_optimization);
        assert!(config.debug_info);
    }
}
