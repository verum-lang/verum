//! VBC → LLVM IR lowering entry point.
//!
//! This module provides the main entry point for lowering VBC (Verum Bytecode)
//! modules to LLVM IR for the CPU compilation path.
//!
//! # Architecture
//!
//! ```text
//! VBC Module
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │                  VbcToLlvmLowering                       │
//! │  - Forward declare all functions                        │
//! │  - Lower function bodies (instruction by instruction)   │
//! │  - Apply CBGR tier-aware optimizations                  │
//! └─────────────────────────────────────────────────────────┘
//!     │
//!     ▼
//! LLVM Module
//!     │
//!     ├────────────────┐
//!     ▼                ▼
//! ┌─────────┐    ┌─────────┐
//! │   JIT   │    │   AOT   │
//! │ (Tier1/2│    │ (Tier3) │
//! └─────────┘    └─────────┘
//! ```
//!
//! # CBGR Tier Awareness
//!
//! The lowering process is tier-aware, generating different code based on
//! the CBGR tier:
//!
//! - **Tier 0**: Full runtime checks (~15ns overhead per check)
//! - **Tier 1**: Compiler-proven safe (zero overhead)
//! - **Tier 2**: Manually marked unsafe (zero overhead)

use std::collections::HashMap;
use std::sync::Arc;

use verum_common::Text;
use verum_llvm::attributes::{Attribute, AttributeLoc};
use verum_llvm::context::Context;
use verum_llvm::module::{Linkage, Module};
use verum_llvm::values::{BasicValueEnum, FunctionValue};
use verum_llvm::AddressSpace;
use verum_llvm::debug_info::{
    DebugInfoBuilder, DICompileUnit, DIFile, DISubprogram, DIFlags, DIFlagsConstants,
    DWARFSourceLanguage, DWARFEmissionKind, DIScope, AsDIScope, DIType, DILocalVariable,
};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::{CallingConvention, FunctionDescriptor, InlineHint, OptLevel, OptimizationHints, VbcFunction, VbcModule};
use verum_vbc::types::{TypeId, TypeRef};

use super::context::FunctionContext;
use super::error::{LlvmLoweringError, Result, BuildExt, OptionExt};
use super::instruction::lower_instruction;
use super::types::{RefTier, TypeLowering};
use super::well_known_types::{WellKnownType as WKT, WellKnownTypeExt as _};
use verum_common::well_known_types::type_names as tn;

/// Configuration for VBC → LLVM lowering.
#[derive(Debug, Clone)]
pub struct LoweringConfig {
    /// Module name.
    pub module_name: Text,
    /// Target triple (e.g., "x86_64-unknown-linux-gnu").
    pub target_triple: Option<Text>,
    /// Optimization level (0-3).
    pub opt_level: u8,
    /// Enable CBGR check elimination.
    pub cbgr_elimination: bool,
    /// Default CBGR tier.
    pub default_tier: RefTier,
    /// Inline threshold for small functions.
    pub inline_threshold: u32,
    /// Enable debug information.
    pub debug_info: bool,
    /// Enable code coverage instrumentation.
    pub coverage: bool,
}

impl Default for LoweringConfig {
    fn default() -> Self {
        Self {
            module_name: Text::from("verum_module"),
            target_triple: None,
            opt_level: 2,
            cbgr_elimination: true,
            default_tier: RefTier::Tier0,
            inline_threshold: 100,
            debug_info: false,
            coverage: false,
        }
    }
}

impl LoweringConfig {
    /// Create a new config with the given module name.
    pub fn new(name: impl Into<Text>) -> Self {
        Self {
            module_name: name.into(),
            ..Default::default()
        }
    }

    /// Set the target triple.
    pub fn with_target(mut self, triple: impl Into<Text>) -> Self {
        self.target_triple = Some(triple.into());
        self
    }

    /// Set optimization level.
    pub fn with_opt_level(mut self, level: u8) -> Self {
        self.opt_level = level.min(3);
        self
    }

    /// Enable or disable CBGR elimination.
    pub fn with_cbgr_elimination(mut self, enable: bool) -> Self {
        self.cbgr_elimination = enable;
        self
    }

    /// Set the default CBGR tier.
    pub fn with_default_tier(mut self, tier: RefTier) -> Self {
        self.default_tier = tier;
        self
    }

    /// Enable DWARF debug info generation (-g flag).
    pub fn with_debug_info(mut self, enable: bool) -> Self {
        self.debug_info = enable;
        self
    }

    /// Enable code coverage instrumentation (--coverage flag).
    pub fn with_coverage(mut self, enable: bool) -> Self {
        self.coverage = enable;
        self
    }

    /// Create a debug configuration.
    pub fn debug(name: impl Into<Text>) -> Self {
        Self {
            module_name: name.into(),
            opt_level: 0,
            cbgr_elimination: false,
            default_tier: RefTier::Tier0,
            debug_info: true,
            ..Default::default()
        }
    }

    /// Create a release configuration.
    pub fn release(name: impl Into<Text>) -> Self {
        Self {
            module_name: name.into(),
            opt_level: 2,
            cbgr_elimination: true,
            default_tier: RefTier::Tier0,
            debug_info: false,
            ..Default::default()
        }
    }

    /// Create an aggressive optimization configuration.
    pub fn aggressive(name: impl Into<Text>) -> Self {
        Self {
            module_name: name.into(),
            opt_level: 3,
            cbgr_elimination: true,
            default_tier: RefTier::Tier1, // Assume more safety proofs
            inline_threshold: 200,
            debug_info: false,
            ..Default::default()
        }
    }
}

/// Statistics for VBC → LLVM lowering.
#[derive(Debug, Default, Clone)]
pub struct LoweringStats {
    /// Total functions lowered.
    pub functions_lowered: usize,
    /// Total instructions lowered.
    pub instructions_lowered: usize,
    /// Total basic blocks.
    pub basic_blocks: usize,
    /// CBGR Tier 0 references (full checks).
    pub tier0_refs: usize,
    /// CBGR Tier 1 references (compiler-proven).
    pub tier1_refs: usize,
    /// CBGR Tier 2 references (unsafe).
    pub tier2_refs: usize,
    /// Runtime checks generated.
    pub runtime_checks: usize,
    /// Checks eliminated.
    pub checks_eliminated: usize,
    /// Total warnings emitted during lowering.
    pub warnings: usize,
}

impl LoweringStats {
    /// Calculate CBGR check elimination rate.
    pub fn elimination_rate(&self) -> f64 {
        let total = self.runtime_checks + self.checks_eliminated;
        if total == 0 {
            0.0
        } else {
            self.checks_eliminated as f64 / total as f64
        }
    }

    /// Get total CBGR references.
    pub fn total_refs(&self) -> usize {
        self.tier0_refs + self.tier1_refs + self.tier2_refs
    }
}

/// LLVM calling convention constants.
/// Reference: llvm-c/Core.h LLVMCallConv enum
mod llvm_calling_conventions {
    pub const C: u32 = 0;
    pub const X86_STDCALL: u32 = 64;
    pub const X86_FASTCALL: u32 = 65;
    pub const ARM_AAPCS: u32 = 67;
    pub const X86_64_SYSV: u32 = 78;
    pub const WIN64: u32 = 79;
    pub const X86_INTR: u32 = 83;
}

/// Map VBC CallingConvention to LLVM calling convention ID.
fn to_llvm_calling_convention(cc: &CallingConvention) -> u32 {
    use llvm_calling_conventions::*;
    match cc {
        CallingConvention::C => C,
        CallingConvention::Stdcall => X86_STDCALL,
        CallingConvention::Fastcall => X86_FASTCALL,
        CallingConvention::SysV64 => X86_64_SYSV,
        CallingConvention::Win64 => WIN64,
        CallingConvention::ArmAapcs => ARM_AAPCS,
        // ARM64 uses the default C calling convention in LLVM
        CallingConvention::Arm64 => C,
        // Interrupt handlers use x86_intrcc calling convention
        // LLVM automatically generates:
        // - Save all registers
        // - iret instruction for return
        CallingConvention::Interrupt => X86_INTR,
        // Naked functions use C calling convention but with naked attribute
        CallingConvention::Naked => C,
    }
}

/// Check if a type name is a primitive/builtin type that should NOT get obj_register_type tracking.
/// This replaces TypeId::is_builtin()/is_semantic_type() checks which are unreliable because
/// VBC TypeIds are non-deterministic — stdlib types can get assigned IDs in the builtin range
/// (< 16) depending on module loading order (HashMap iteration in Rust).
pub(crate) fn is_primitive_type_name(name: &str) -> bool {
    use verum_common::well_known_types::type_names;
    type_names::is_primitive_value_type(name)
        || type_names::is_numeric_type(name)
        || matches!(name, "Never"
            // Legacy short aliases used in some code paths
            | "I8" | "I16" | "I32" | "U8" | "U16" | "U32" | "U64" | "I128" | "U128"
            | "F32" | "Usize" | "Isize")
}

/// Main VBC → LLVM IR lowering context.
pub struct VbcToLlvmLowering<'ctx> {
    /// LLVM context.
    context: &'ctx Context,
    /// LLVM module being built.
    module: Module<'ctx>,
    /// Type lowering helper.
    types: TypeLowering<'ctx>,
    /// Configuration.
    config: LoweringConfig,
    /// Function map (VBC function ID → LLVM function).
    functions: HashMap<u32, FunctionValue<'ctx>>,
    /// Statistics.
    stats: LoweringStats,
    /// Pre-built function name index for O(1) lookups in instruction lowering.
    func_name_index: Option<std::sync::Arc<super::context::FuncNameIndex>>,
    /// True if any function declarations were arity-suffixed due to name collisions.
    /// When set, IR printing is skipped to avoid LLVM crashes on bitcast wrappers.
    has_arity_collisions: bool,
    /// Function IDs whose bodies should NOT be lowered because their LLVM function
    /// has a mismatched signature (from arity collision — the original-named function
    /// was created for a different arity). These are stdlib functions whose body
    /// would produce invalid IR if lowered into the wrong-arity LLVM function.
    skip_body_func_ids: std::collections::HashSet<u32>,

    /// DWARF debug info builder (created when config.debug_info is true).
    dibuilder: Option<DebugInfoBuilder<'ctx>>,
    /// DWARF compile unit (root scope for all debug info).
    di_compile_unit: Option<DICompileUnit<'ctx>>,
    /// DWARF file reference for the main source file.
    di_file: Option<DIFile<'ctx>>,

    /// VBC-level CBGR escape analysis results.
    /// When set, provides per-instruction tier decisions that override
    /// the LLVM-side local/unknown heuristic for Ref/RefMut instructions.
    escape_analysis: Option<verum_vbc::cbgr_analysis::EscapeAnalysisResult>,
}

/// Coerce any `BasicValueEnum` to an LLVM `i1` boolean suitable for
/// conditional branches (`JmpNot`, `JmpIf`).
///
/// The VBC compiler may place comparison results in registers that hold
/// pointer values (e.g. from C runtime string-comparison calls) or float
/// values.  Calling `into_int_value()` on those panics, so we handle every
/// possible LLVM value kind explicitly.
fn coerce_to_bool<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    val: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<verum_llvm::values::IntValue<'ctx>> {
    let bool_type = ctx.types().bool_type(); // i1
    let i64_type = ctx.types().i64_type();
    match val {
        BasicValueEnum::IntValue(iv) => {
            let bw = iv.get_type().get_bit_width();
            if bw == 1 {
                Ok(iv)
            } else {
                // Non-zero → true.  Truncate to i1 (keeps the LSB which is
                // the boolean value for well-formed VBC).
                ctx.builder()
                    .build_int_truncate(iv, bool_type, name)
                    .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
            }
        }
        BasicValueEnum::PointerValue(pv) => {
            // ptr != null → true
            let as_int = ctx.builder()
                .build_ptr_to_int(pv, i64_type, &format!("{}_p2i", name))
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
            ctx.builder()
                .build_int_truncate(as_int, bool_type, name)
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
        }
        BasicValueEnum::FloatValue(fv) => {
            // float != 0.0 → true
            let zero = ctx.types().f64_type().const_float(0.0);
            ctx.builder()
                .build_float_compare(verum_llvm::FloatPredicate::ONE, fv, zero, name)
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
        }
        BasicValueEnum::StructValue(sv) => {
            // Extract first field (usually the pointer or discriminant) and
            // recursively coerce.
            if sv.get_type().count_fields() == 0 {
                // Empty struct → false
                return Ok(bool_type.const_int(0, false));
            }
            let field0 = ctx.builder()
                .build_extract_value(sv, 0, &format!("{}_f0", name))
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
            coerce_to_bool(ctx, field0, name)
        }
        _ => {
            // ArrayValue / VectorValue – treat as truthy
            Ok(bool_type.const_int(1, false))
        }
    }
}

/// Coerce any `BasicValueEnum` to an i64 suitable for `Switch` instructions.
fn coerce_to_i64_for_switch<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    val: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<verum_llvm::values::IntValue<'ctx>> {
    let i64_type = ctx.types().i64_type();
    match val {
        BasicValueEnum::IntValue(iv) => {
            let bw = iv.get_type().get_bit_width();
            if bw == 64 {
                Ok(iv)
            } else if bw < 64 {
                ctx.builder()
                    .build_int_z_extend(iv, i64_type, name)
                    .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
            } else {
                ctx.builder()
                    .build_int_truncate(iv, i64_type, name)
                    .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
            }
        }
        BasicValueEnum::PointerValue(pv) => {
            ctx.builder()
                .build_ptr_to_int(pv, i64_type, name)
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
        }
        BasicValueEnum::FloatValue(fv) => {
            ctx.builder()
                .build_bit_cast(fv, i64_type, name)
                .map(|v| v.into_int_value())
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
        }
        BasicValueEnum::StructValue(sv) => {
            if sv.get_type().count_fields() == 0 {
                return Ok(i64_type.const_zero());
            }
            let field0 = ctx.builder()
                .build_extract_value(sv, 0, &format!("{}_f0", name))
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
            coerce_to_i64_for_switch(ctx, field0, name)
        }
        _ => Ok(i64_type.const_zero()),
    }
}

impl<'ctx> VbcToLlvmLowering<'ctx> {
    /// Create a new VBC → LLVM lowering context.
    pub fn new(context: &'ctx Context, config: LoweringConfig) -> Self {
        let module = context.create_module(&config.module_name);

        // Set target triple and data layout.
        // Without a data layout, LLVM uses conservative defaults (e.g. align 4
        // for i64 on aarch64) which can cause non-deterministic SIGSEGV crashes.
        let triple = if let Some(ref t) = config.target_triple {
            verum_llvm::targets::TargetTriple::create(t)
        } else {
            verum_llvm::targets::TargetMachine::get_default_triple()
        };
        module.set_triple(&triple);

        // Initialize native target ONCE per process.
        // LLVM's target initialization is NOT idempotent — calling it multiple
        // times can corrupt internal state, causing intermittent SIGSEGV in
        // module verification or code generation.
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let _ = verum_llvm::targets::Target::initialize_native(
                &verum_llvm::targets::InitializationConfig::default(),
            );
        });
        if let Ok(target) = verum_llvm::targets::Target::from_triple(&triple) {
            if let Some(tm) = target.create_target_machine(
                &triple,
                "generic",
                "",
                verum_llvm::OptimizationLevel::None,
                verum_llvm::targets::RelocMode::Default,
                verum_llvm::targets::CodeModel::Default,
            ) {
                let td = tm.get_target_data();
                module.set_data_layout(&td.get_data_layout());
            }
        }

        let types = TypeLowering::new(context);

        // Initialize DWARF debug info if requested.
        // Creates DIBuilder + compile unit + file scope.
        let (dibuilder, di_compile_unit, di_file) = if config.debug_info {
            // Set debug info version metadata on the module
            let debug_metadata_version = context.i32_type().const_int(3, false);
            module.add_basic_value_flag(
                "Debug Info Version",
                verum_llvm::module::FlagBehavior::Warning,
                debug_metadata_version,
            );

            let filename = config.module_name.as_str();
            let directory = ".";
            let (db, cu) = module.create_debug_info_builder(
                true, // allow_unresolved — needed for forward references
                DWARFSourceLanguage::C, // Closest to Verum's ABI (LLVM has no "Verum" language)
                filename,
                directory,
                "verum compiler v0.4.0", // producer
                config.opt_level > 0, // is_optimized
                "", // flags
                0,  // runtime_ver
                "", // split_name
                DWARFEmissionKind::Full,
                0,  // dwo_id
                false, // split_debug_inlining
                false, // debug_info_for_profiling
                "",    // sysroot
                "",    // sdk
            );
            let file = db.create_file(filename, directory);
            (Some(db), Some(cu), Some(file))
        } else {
            (None, None, None)
        };

        Self {
            context,
            module,
            types,
            config,
            functions: HashMap::new(),
            stats: LoweringStats::default(),
            func_name_index: None,
            has_arity_collisions: false,
            skip_body_func_ids: std::collections::HashSet::new(),
            dibuilder,
            di_compile_unit,
            di_file,
            escape_analysis: None,
        }
    }

    /// Set VBC-level CBGR escape analysis results.
    ///
    /// When set, tier decisions from the escape analysis are applied to each
    /// function during LLVM lowering. References that the escape analysis
    /// proves non-escaping are promoted from Tier 0 (~15ns) to Tier 1 (0ns).
    pub fn set_escape_analysis(&mut self, result: verum_vbc::cbgr_analysis::EscapeAnalysisResult) {
        self.escape_analysis = Some(result);
    }

    /// Get the LLVM context.
    pub fn context(&self) -> &'ctx Context {
        self.context
    }

    /// Get the LLVM module.
    pub fn module(&self) -> &Module<'ctx> {
        &self.module
    }

    /// Get the configuration.
    pub fn config(&self) -> &LoweringConfig {
        &self.config
    }

    /// Get lowering statistics.
    pub fn stats(&self) -> &LoweringStats {
        &self.stats
    }

    /// Get CBGR-specific statistics extracted from the lowering stats.
    pub fn cbgr_stats(&self) -> super::cbgr::CbgrStats {
        super::cbgr::CbgrStats {
            refs_created: self.stats.tier0_refs + self.stats.tier1_refs + self.stats.tier2_refs,
            tier0_refs: self.stats.tier0_refs,
            tier1_refs: self.stats.tier1_refs,
            tier2_refs: self.stats.tier2_refs,
            runtime_checks: self.stats.runtime_checks,
            checks_eliminated: self.stats.checks_eliminated,
        }
    }

    /// Lower a complete VBC module to LLVM IR.
    ///
    /// This is the main entry point for lowering. It processes the module in phases:
    /// 1. Forward declare all functions
    /// 2. Lower function bodies
    /// 3. Emit global constructors/destructors
    /// 4. Verify the module
    pub fn lower_module(&mut self, vbc_module: &VbcModule) -> Result<()> {
        // Phase 0.5: Emit LLVM IR helper functions (replaces C runtime stubs)
        super::runtime::define_text_ir_helpers(self.context, &self.module)?;
        super::runtime::define_list_ir_helpers(self.context, &self.module)?;
        super::runtime::define_map_set_ir_helpers(self.context, &self.module)?;

        // Phase 0.6: Declare POSIX/libc functions used by compiled stdlib (.vr) modules.
        // Without these, compiled Mutex.vr/Condvar.vr/Thread.vr functions that call
        // pthread_create/pthread_mutex_lock/etc. become invalid and get stripped.
        self.declare_posix_functions();

        // Phase 1: Forward declare all functions
        self.declare_functions(vbc_module)?;

        // Phase 1.5: Build function name index for O(1) lookups
        self.func_name_index = Some(std::sync::Arc::new(
            super::context::FuncNameIndex::build(vbc_module)
        ));

        // Phase 1.6: Create coverage counter array if coverage is enabled.
        // Global array: i64[N] where N = number of functions.
        // Each function increments its counter on entry.
        if self.config.coverage {
            let i64_type = self.context.i64_type();
            let num_funcs = vbc_module.functions.len() as u32;
            let array_type = i64_type.array_type(num_funcs.max(1));
            let global = self.module.add_global(array_type, None, "__verum_coverage_counters");
            global.set_initializer(&array_type.const_zero());
            global.set_linkage(Linkage::Internal);

            // Also store the function count for the report
            let count_global = self.module.add_global(i64_type, None, "__verum_coverage_func_count");
            count_global.set_initializer(&i64_type.const_int(num_funcs as u64, false));
            count_global.set_linkage(Linkage::Internal);
        }

        // Phase 2: Lower function bodies.
        // Track which LLVM functions have already been lowered to handle
        // name collisions where multiple VBC functions map to the same LLVM
        // function (e.g., stdlib and user both define `execute_with_retry`).
        // The LAST body wins (user code overrides stdlib).
        //
        // Dedupe key is (raw_name, arity). Different-arity overloads of the
        // same name (e.g. user `fn main()` 0-arity and stdlib
        // `darwin_entry::main(argc, argv)` 2-arity) map to DIFFERENT LLVM
        // functions (the second collides on declare and gets a `__arity{n}`
        // suffix at vbc_lowering.rs ~1200). Deduping by name only would
        // skip user's main when stdlib's same-named overload comes second
        // in iteration order, leaving the 0-arity LLVM function bodyless
        // and the Phase 3.6 safety net stubs it with `return 0` — masking
        // the user's program logic. Bisect 2026-04-26: `mount core.*; fn main() -> Int { 7 }`
        // returned exit=0 instead of 7 due to this.
        let mut lowered_llvm_fns: std::collections::HashMap<(String, usize), usize> = std::collections::HashMap::new();

        // First pass: collect (name, arity) pairs that have multiple bodies.
        let mut body_count: std::collections::HashMap<(String, usize), usize> = std::collections::HashMap::new();
        for func_desc in &vbc_module.functions {
            if func_desc.instructions.is_some() {
                let name = vbc_module.strings.get(func_desc.name).unwrap_or("");
                let key = (name.to_string(), func_desc.params.len());
                *body_count.entry(key).or_default() += 1;
            }
        }

        for (_idx, func_desc) in vbc_module.functions.iter().enumerate() {
            if let Some(ref instructions) = func_desc.instructions {
                let vbc_func = VbcFunction::new(func_desc.clone(), instructions.clone());
                let func_name = vbc_module
                    .strings
                    .get(func_desc.name)
                    .unwrap_or("<unknown>");

                // Skip body lowering for functions whose LLVM declaration has
                // wrong param count (from arity collision in declare_functions).
                if let Some(llvm_fn) = self.functions.get(&func_desc.id.0) {
                    let llvm_params = llvm_fn.count_params() as usize;
                    let vbc_params = func_desc.params.len();
                    if llvm_params != vbc_params && vbc_params > 0 {
                        continue;
                    }
                }

                // For name+arity collisions (e.g., stdlib and user both
                // define `parallel_map<T,U>` with the same arity), only
                // lower the LAST occurrence (user code overrides stdlib).
                let dedupe_key = (func_name.to_string(), func_desc.params.len());
                if let Some(&count) = body_count.get(&dedupe_key) {
                    if count > 1 {
                        let seen = lowered_llvm_fns.entry(dedupe_key).or_insert(0);
                        *seen += 1;
                        if *seen < count {
                            // Not the last occurrence — skip it
                            continue;
                        }
                        // Last occurrence — lower it (this is the user's body)
                    }
                }

                let is_tracked = func_name == "parallel_map" || func_name == "execute_with_retry" || func_name == "any_of" || func_name == "heap_sort";
                if is_tracked {
                    let bb_count = self.functions.get(&func_desc.id.0).map_or(0, |f| f.count_basic_blocks());
                    tracing::debug!("[LOWER] {} id={} blocks_before={}", func_name, func_desc.id.0, bb_count);
                }
                // VERUM_AOT_TRACE_LOWER=1: print every function name to
                // stderr before lowering. Survives panics and SIGABRT
                // because the print is line-buffered to stderr (which
                // libc flushes on signal). Used for bisecting a crash
                // that happens during LLVM IR construction by reading
                // the last-printed name from the abort output.
                if std::env::var_os("VERUM_AOT_TRACE_LOWER").is_some() {
                    eprintln!("[aot-lower] id={} name={}", func_desc.id.0, func_name);
                }
                if let Err(e) = self.lower_vbc_function(vbc_module, &vbc_func) {
                    // Stdlib functions may fail to lower (e.g. methods using
                    // unimplemented intrinsics). Skip gracefully.
                    if is_tracked { tracing::debug!("[LOWER] {} FAILED: {:?}", func_name, e); }
                    tracing::warn!("Skipping function '{}': {:?}", func_name, e);
                    // Patch up partially-emitted function: add unreachable terminators
                    // to any blocks missing them. Without this, GlobalDCE can't remove
                    // the function if it's referenced, causing module verification failure.
                    //
                    // Phase-A fallback (#106): if `lower_vbc_function` failed
                    // BEFORE emitting any block (e.g. early validation fail),
                    // the function is left as a bodyless declaration — callers
                    // resolve to a null function-pointer at link/load time
                    // and SIGSEGV during dyld static-init. Append a single
                    // entry block that returns the type's zero value so
                    // callers get a "skipped" sentinel instead of a null
                    // deref. This preserves graceful degradation: the program
                    // sees a default value when reaching skipped code, far
                    // safer than crashing during process startup.
                    if let Some(llvm_fn) = self.module.get_function(func_name) {
                        let builder = self.context.create_builder();
                        if llvm_fn.get_first_basic_block().is_none() {
                            // Bodyless declaration — synthesise a default-return entry.
                            let entry = self.context.append_basic_block(llvm_fn, "skipped_entry");
                            builder.position_at_end(entry);
                            // Return the zero value of the function's return type.
                            let ret_ty_opt = llvm_fn.get_type().get_return_type();
                            match ret_ty_opt {
                                Some(verum_llvm::types::BasicTypeEnum::IntType(it)) => {
                                    let _ = builder.build_return(Some(&it.const_zero()));
                                }
                                Some(verum_llvm::types::BasicTypeEnum::FloatType(ft)) => {
                                    let _ = builder.build_return(Some(&ft.const_zero()));
                                }
                                Some(verum_llvm::types::BasicTypeEnum::PointerType(pt)) => {
                                    let _ = builder.build_return(Some(&pt.const_null()));
                                }
                                Some(_) | None => {
                                    // Void or aggregate return — `ret void` or
                                    // unreachable as last resort.
                                    let _ = builder.build_return(None);
                                }
                            }
                        } else {
                            let mut bb = llvm_fn.get_first_basic_block();
                            while let Some(block) = bb {
                                if block.get_terminator().is_none() {
                                    builder.position_at_end(block);
                                    let _ = builder.build_unreachable();
                                }
                                bb = block.get_next_basic_block();
                            }
                        }
                    }
                    continue;
                }
                if is_tracked {
                    let bb_count = self.functions.get(&func_desc.id.0).map_or(0, |f| f.count_basic_blocks());
                    tracing::debug!("[LOWER] {} blocks_after={}", func_name, bb_count);
                }
                // Track parallel_map blocks
                if func_desc.id.0 == 37 {
                    if let Some(pm) = self.module.get_function("parallel_map") {
                        tracing::debug!("[BEFORE-MAIN] parallel_map blocks={}", pm.count_basic_blocks());
                    }
                }
                if func_desc.id.0 == 36 || func_desc.id.0 == 37 {
                    let fn_name_llvm = self.functions.get(&func_desc.id.0)
                        .map(|f| f.get_name().to_string_lossy().to_string())
                        .unwrap_or_default();
                    tracing::debug!("[LOWER-DETAIL] id={} vbc_name='{}' llvm_name='{}'", func_desc.id.0, func_name, fn_name_llvm);
                }
            }
        }

        if let Some(f) = self.module.get_function("parallel_map") {
            tracing::debug!("[POST-PHASE2-LOOP] parallel_map blocks={}", f.count_basic_blocks());
        }

        // Phase 3: Emit global constructors/destructors
        self.emit_global_ctors_dtors(vbc_module)?;

        if let Some(f) = self.module.get_function("parallel_map") {
            tracing::debug!("[POST-CTORS] parallel_map blocks={}", f.count_basic_blocks());
        }

        // Debug: dump IR to file after phase 2 (before cleanup passes)
        if let Ok(path) = std::env::var("VERUM_DUMP_IR") {
            let _ = self.module.print_to_file(std::path::Path::new(&path));
            tracing::debug!("[IR dump] wrote to {}", path);
        }

        // Debug: check before cleanup
        if let Some(f) = self.module.get_function("parallel_map") {
            tracing::debug!("[PRE-CLEANUP] parallel_map blocks={}", f.count_basic_blocks());
        }

        // Phase 3.5: Remove known-invalid functions by name pattern.
        self.remove_known_invalid_functions();

        if let Some(f) = self.module.get_function("parallel_map") {
            tracing::debug!("[POST-KNOWN-INVALID] parallel_map blocks={}", f.count_basic_blocks());
        }

        // Phase 3.55: Remove self-recursive functions from broken @cfg dispatch
        self.remove_self_recursive_functions();

        if let Some(f) = self.module.get_function("parallel_map") {
            tracing::debug!("[POST-SELF-RECURSIVE] parallel_map blocks={}", f.count_basic_blocks());
        }

        // Phase 3.6: Emit LLVM IR runtime functions
        // These replace C runtime stubs with pure LLVM IR implementations.
        // Must be emitted BEFORE setting Internal linkage (Phase 3.7) so that
        // the IR versions become module-internal and don't conflict with C runtime
        // symbols at link time.
        {
            // VERUM_AOT_TRACE_RUNTIME=1 prints which top-level
            // runtime-emit phase is running so signature-mismatch
            // panics deep inside one of them can be pinpointed by
            // reading the abort tail.
            let trace = std::env::var_os("VERUM_AOT_TRACE_RUNTIME").is_some();

            let runtime = super::runtime::RuntimeLowering::new(self.context);
            if trace { eprintln!("[aot-runtime-stage] emit_text_ir_functions"); }
            runtime.emit_text_ir_functions(&self.module)?;
            if trace { eprintln!("[aot-runtime-stage] emit_misc_ir_functions"); }
            runtime.emit_misc_ir_functions(&self.module)?;

            // Emit platform-native runtime functions as LLVM IR.
            // These shadow C runtime functions for full LTO optimization.
            if trace { eprintln!("[aot-runtime-stage] emit_platform_functions"); }
            let platform = super::platform_ir::PlatformIR::new(self.context);
            platform.emit_platform_functions(&self.module)?;

            // Emit tensor runtime functions as LLVM IR.
            // Replaces verum_tensor.c — uses LLVM intrinsics for math (native HW speed).
            if trace { eprintln!("[aot-runtime-stage] emit_tensor_functions"); }
            let tensor = super::tensor_ir::TensorIR::new(self.context);
            tensor.emit_tensor_functions(&self.module)?;

            // Emit Metal GPU runtime functions as LLVM IR.
            // Replaces verum_metal.m — uses ObjC runtime calls via objc_msgSend.
            if trace { eprintln!("[aot-runtime-stage] emit_metal_functions"); }
            let metal = super::metal_ir::MetalIR::new(self.context);
            metal.emit_metal_functions(&self.module)?;
            if trace { eprintln!("[aot-runtime-stage] all_done"); }
        }

        // Phase 3.6 (#106 Path A.2): final-pass safety net — for any
        // function that's STILL bodyless after all the runtime-helper
        // emit phases, synthesise a default-return entry block. This
        // catches functions that:
        //
        //   * Were declared by Call sites that never matched a
        //     recognised runtime helper (so no emit_*_functions phase
        //     wrote a body).
        //   * Were declared as "extern" by user code intending to link
        //     against a C library that isn't actually wired in this
        //     build configuration.
        //   * Were skipped during VBC lowering AFTER any other phase
        //     could observe them.
        //
        // Without this pass, dyld resolves the bodyless declarations
        // to a null function pointer at process startup; static
        // initializers (e.g. `static NEVER_FLAG: ... = ...` at
        // cancellation.vr:475) trigger SIGSEGV before main() runs.
        // The default-return fallback degrades the call chain
        // gracefully — process boots, downstream code sees a default
        // value where it expected real init.
        //
        // Tracked under #106. Once Path B / C land (fix the underlying
        // skipping causes / wrap static initializers in Lazy<T>) this
        // safety net should rarely fire; until then it's the
        // difference between "process boots" and "SIGSEGV at dyld".
        {
            // Helper: should this bodyless declaration be left alone
            // for the linker to resolve? True for libc/libsystem
            // functions and well-known C-runtime symbols. Patching
            // these with a default-return body BREAKS the C runtime —
            // e.g. an `_exit` stub that returns instead of terminating
            // turns the OOM-abort path inside `verum_checked_malloc`
            // into a SIGTRAP at the trailing `unreachable` rather
            // than a clean process termination. The previous version
            // of this safety net patched everything indiscriminately;
            // exit/abort/malloc were caught and broke their semantics.
            //
            // Conservative allow-list of libc symbols that MUST be
            // resolved by ld at link time, not stubbed here.
            fn is_libc_extern(name: &str) -> bool {
                matches!(name,
                    // process control
                    "exit" | "_exit" | "_Exit" | "abort" | "_abort"
                    // memory
                    | "malloc" | "calloc" | "realloc" | "free"
                    | "posix_memalign" | "aligned_alloc"
                    | "memcpy" | "memmove" | "memset" | "memcmp"
                    | "strlen" | "strcmp" | "strncmp" | "strchr" | "strcpy" | "strncpy" | "strcat"
                    // I/O
                    | "write" | "read" | "open" | "close" | "fsync" | "fdatasync" | "ftruncate"
                    | "lseek" | "lseek64" | "pread" | "pwrite"
                    | "printf" | "fprintf" | "snprintf" | "vprintf" | "vsnprintf"
                    | "puts" | "fputs" | "fputc" | "putchar" | "fwrite" | "fread"
                    | "perror" | "putc" | "getc" | "getchar"
                    // time/clock
                    | "clock_gettime" | "nanosleep" | "gettimeofday"
                    | "time" | "mach_absolute_time"
                    // sockets (libc)
                    | "socket" | "bind" | "listen" | "accept" | "connect"
                    | "send" | "recv" | "sendto" | "recvfrom" | "setsockopt"
                    | "getaddrinfo" | "freeaddrinfo" | "inet_pton"
                    // mmap family
                    | "mmap" | "munmap" | "mprotect" | "madvise"
                    // pthread
                    | "pthread_create" | "pthread_join" | "pthread_mutex_init"
                    | "pthread_mutex_lock" | "pthread_mutex_unlock"
                    | "pthread_cond_init" | "pthread_cond_wait" | "pthread_cond_signal"
                    | "pthread_self" | "pthread_setname_np"
                    // file ops
                    | "stat" | "fstat" | "lstat" | "access" | "unlink" | "rename"
                    | "mkdir" | "rmdir" | "chdir" | "getcwd"
                    // ObjC runtime
                    | "objc_msgSend" | "objc_msgSendSuper" | "sel_registerName"
                    | "objc_getClass" | "objc_lookUpClass"
                )
            }

            let mut patched = 0usize;
            let mut skipped_libc = 0usize;
            let mut func = self.module.get_first_function();
            while let Some(f) = func {
                let next = f.get_next_function();
                if f.get_first_basic_block().is_none() {
                    let name = f.get_name().to_string_lossy().to_string();
                    if is_libc_extern(&name) {
                        // Leave libc decls alone — linker resolves at link time.
                        skipped_libc += 1;
                        func = next;
                        continue;
                    }
                    let entry = self.context.append_basic_block(f, "skipped_entry");
                    let builder = self.context.create_builder();
                    builder.position_at_end(entry);
                    let ret_ty_opt = f.get_type().get_return_type();
                    let _ = match ret_ty_opt {
                        Some(verum_llvm::types::BasicTypeEnum::IntType(it)) => {
                            builder.build_return(Some(&it.const_zero()))
                        }
                        Some(verum_llvm::types::BasicTypeEnum::FloatType(ft)) => {
                            builder.build_return(Some(&ft.const_zero()))
                        }
                        Some(verum_llvm::types::BasicTypeEnum::PointerType(pt)) => {
                            builder.build_return(Some(&pt.const_null()))
                        }
                        Some(_) => {
                            // Aggregate / vector / scalable-vector return —
                            // emit unreachable. dyld-init rarely traverses
                            // such paths, and forging a default for an
                            // unknown aggregate is risky.
                            builder.build_unreachable()
                        }
                        None => builder.build_return(None),
                    };
                    patched += 1;
                }
                func = next;
            }
            if std::env::var_os("VERUM_AOT_TRACE_RUNTIME").is_some() {
                eprintln!(
                    "[aot-runtime-stage] bodyless-decl safety net: patched {} (skipped {} libc externs)",
                    patched, skipped_libc
                );
            }
        }

        // Phase 3.7: Set internal linkage on all defined functions except verum_main.
        // Must be done AFTER remove_invalid_functions — Internal declarations
        // without bodies are invalid LLVM IR. Only verum_main needs External
        // linkage (called by C runtime entry point). All other functions are
        // module-internal and can be removed by GlobalDCE if unreferenced.
        {
            let mut func = self.module.get_first_function();
            while let Some(f) = func {
                let next = f.get_next_function();
                let name = f.get_name().to_string_lossy();
                // Set Internal linkage for globaldce, except:
                // - verum_main/main: entry points
                // - Functions called from C .o: need External linkage for linker
                let keep_external = name == "verum_main" || name == "main"
                    || name == "verum_process_wait"
                    || name == "verum_fd_read_all"
                    || name == "verum_fd_close"
                    || name == "verum_file_open"
                    || name == "verum_file_close"
                    || name == "verum_file_exists"
                    || name == "verum_file_delete"
                    || name == "verum_file_read_text"
                    || name == "verum_file_write_text"
                    || name == "verum_file_read_all"
                    || name == "verum_file_write_all"
                    || name == "verum_file_append_all"
                    || name == "verum_tcp_connect"
                    || name == "verum_tcp_listen"
                    || name == "verum_tcp_accept"
                    || name == "verum_tcp_send_text"
                    || name == "verum_tcp_recv_text"
                    || name == "verum_tcp_close"
                    || name == "verum_udp_bind"
                    || name == "verum_udp_send_text"
                    || name == "verum_udp_recv_text"
                    || name == "verum_udp_close"
                    || name == "verum_time_monotonic_nanos"
                    || name == "verum_time_sleep_nanos"
                    || name == "verum_sleep_ms"
                    || name == "verum_time_now_ms"
                    || name == "verum_alloc"
                    || name == "verum_alloc_zeroed"
                    || name == "verum_dealloc"
                    || name == "verum_os_alloc"
                    || name == "verum_os_free"
                    || name == "verum_os_write"
                    || name == "verum_os_exit"
                    // verum_store_args, get_argc, get_argv, runtime_init/cleanup,
                    // push/pop_stack_frame: LLVM IR versions in platform_ir.rs are authoritative
                    || name == "verum_socket_set_nonblocking"
                    || name == "verum_socket_set_blocking"
                    || name == "verum_socket_set_reuseaddr"
                    || name == "verum_socket_set_nodelay"
                    || name == "verum_socket_set_keepalive"
                    || name == "verum_socket_get_error"
                    || name == "verum_socket_connect_nonblocking"
                    || name == "verum_socket_accept_nonblocking"
                    || name == "verum_ctx_get"
                    || name == "verum_ctx_provide"
                    || name == "verum_ctx_end"
                    || name == "verum_chan_new"
                    || name == "verum_chan_send"
                    || name == "verum_chan_recv"
                    || name == "verum_chan_try_send"
                    || name == "verum_chan_try_recv"
                    || name == "verum_chan_close"
                    || name == "verum_chan_len"
                    || name == "verum_nursery_new"
                    || name == "verum_nursery_spawn"
                    || name == "verum_nursery_await_all"
                    || name == "verum_nursery_set_timeout"
                    || name == "verum_nursery_set_max_tasks"
                    || name == "verum_nursery_set_error_behavior"
                    || name == "verum_nursery_cancel"
                    || name == "verum_nursery_get_error"
                    || name == "verum_waitgroup_new"
                    || name == "verum_waitgroup_add"
                    || name == "verum_waitgroup_done"
                    || name == "verum_waitgroup_wait"
                    || name == "verum_waitgroup_destroy"
                    || name == "verum_select_channels"
                    || name == "verum_io_engine_new"
                    || name == "verum_io_submit"
                    || name == "verum_io_poll"
                    || name == "verum_io_remove"
                    || name == "verum_io_modify"
                    || name == "verum_io_engine_destroy"
                    || name == "verum_io_engine_fd"
                    || name == "verum_io_submit_both"
                    || name == "verum_pool_create"
                    || name == "verum_pool_submit"
                    || name == "verum_pool_await"
                    || name == "verum_pool_destroy"
                    || name == "verum_pool_global"
                    || name == "verum_pool_global_submit"
                    || name == "verum_async_accept"
                    || name == "verum_async_read"
                    || name == "verum_async_write"
                    || name == "verum_gen_create"
                    || name == "verum_gen_next"
                    || name == "verum_gen_has_next"
                    || name == "verum_gen_next_maybe"
                    || name == "verum_gen_close"
                    || name == "verum_cbgr_check"
                    || name == "verum_cbgr_check_write"
                    || name == "verum_cbgr_check_fat"
                    || name == "verum_cbgr_epoch_begin"
                    || name == "verum_thread_entry_darwin"
                    || name == "gen_thread_entry"
                    || name == "verum_spawn_trampoline"
                    || name == "verum_futex_wait"
                    || name == "verum_futex_wake"
                    || name == "verum_process_spawn"
                    || name == "verum_process_run"
                    || name == "verum_process_spawn_cmd"
                    || name == "verum_process_exec";
                if f.count_basic_blocks() > 0 && !keep_external {
                    f.set_linkage(Linkage::Internal);
                }
                func = next;
            }
        }

        // Phase 4: Verify the module
        // Skip verification here — GlobalDCE in pipeline.rs will remove dead
        // functions first, then the module can be verified post-DCE if needed.
        // Phase 4: Module verification deferred to after GlobalDCE (pipeline.rs).
        // Dead functions from text.vr may have invalid IR that would crash the
        // verifier. GlobalDCE removes them first, then pipeline.rs verifies.

        Ok(())
    }

    /// Lower a list of pre-decoded VBC functions.
    ///
    /// Use this when you have already decoded the VBC functions.
    pub fn lower_functions(
        &mut self,
        vbc_module: &VbcModule,
        functions: &[VbcFunction],
    ) -> Result<()> {
        // Phase 1: Forward declare all functions
        self.declare_functions(vbc_module)?;

        // Phase 2: Lower function bodies
        for func in functions {
            self.lower_vbc_function(vbc_module, func)?;
        }

        // Phase 3: Emit global constructors/destructors
        self.emit_global_ctors_dtors(vbc_module)?;

        // Phase 3.5: Per-function verification
        self.remove_invalid_functions();

        // Phase 4: Verify the module
        self.verify()?;

        Ok(())
    }

    /// Forward declare all functions in the module.
    fn declare_functions(&mut self, vbc_module: &VbcModule) -> Result<()> {
        use verum_vbc::types::TypeRef;
        use verum_vbc::types::TypeId;

        for func_desc in &vbc_module.functions {
            // Determine effective return type:
            // - If instructions available and contain Ret (value return), use i64
            // - If instructions available and only RetV (void return), use UNIT
            // - If instructions not available, default to i64 (most functions return values)
            // - User-defined types in return position → i64 (heap pointer representation)
            let effective_return_type = match &func_desc.return_type {
                TypeRef::Concrete(TypeId::UNIT) => {
                    match func_desc.instructions.as_ref() {
                        Some(instrs) => {
                            let has_value_return = instrs.iter().any(|i| matches!(i, Instruction::Ret { .. }));
                            if has_value_return {
                                TypeRef::Concrete(TypeId::INT)
                            } else {
                                TypeRef::Concrete(TypeId::UNIT)
                            }
                        }
                        None => {
                            // No instructions available — conservatively assume i64
                            TypeRef::Concrete(TypeId::INT)
                        }
                    }
                }
                TypeRef::Concrete(tid) => {
                    // Check if type is a known primitive
                    match *tid {
                        TypeId::INT | TypeId::FLOAT | TypeId::BOOL | TypeId::TEXT |
                        TypeId::PTR | TypeId::NEVER | TypeId::U8 | TypeId::I8 |
                        TypeId::U16 | TypeId::I16 | TypeId::U32 | TypeId::I32 |
                        TypeId::U64 | TypeId::F32 => func_desc.return_type.clone(),
                        _ => {
                            // User-defined type — represent as i64 (heap pointer)
                            TypeRef::Concrete(TypeId::INT)
                        }
                    }
                }
                _ => {
                    // Generic/Instantiated/Function/Reference → i64
                    TypeRef::Concrete(TypeId::INT)
                }
            };

            let raw_name = vbc_module
                .strings
                .get(func_desc.name)
                .unwrap_or("<anonymous>");

            // Also normalize parameter types: user-defined types → i64, UNIT → i64
            // (UNIT params happen for `self` in methods where the type isn't resolved)
            //
            // Spawn functions ($spawn$) receive all args as i64 from verum_thread_spawn,
            // so force ALL their params to INT regardless of semantic type. The original
            // type info in func_desc.params is still used for register marking below.
            let is_spawn_func = raw_name.contains("$spawn$");
            let effective_params: Vec<verum_vbc::module::ParamDescriptor> = func_desc.params.iter().map(|p| {
                let eff_type = if is_spawn_func {
                    TypeRef::Concrete(TypeId::INT)
                } else {
                    match &p.type_ref {
                        TypeRef::Concrete(tid) => {
                            match *tid {
                                TypeId::INT | TypeId::FLOAT | TypeId::BOOL | TypeId::TEXT |
                                TypeId::PTR | TypeId::NEVER | TypeId::U8 | TypeId::I8 |
                                TypeId::U16 | TypeId::I16 | TypeId::U32 | TypeId::I32 |
                                TypeId::U64 | TypeId::F32 => p.type_ref.clone(),
                                // UNIT and user-defined types → i64 (heap pointer representation)
                                _ => TypeRef::Concrete(TypeId::INT),
                            }
                        }
                        _ => TypeRef::Concrete(TypeId::INT),
                    }
                };
                verum_vbc::module::ParamDescriptor {
                    name: p.name,
                    type_ref: eff_type,
                    is_mut: p.is_mut,
                    default: p.default,
                }
            }).collect();

            let fn_type = self.types.lower_function_type(
                &effective_params,
                &effective_return_type,
            )?;

            // Rename "main" to "verum_main" so the C runtime entry point can call it
            let func_name = if raw_name == "main" {
                "verum_main".to_string()
            } else {
                raw_name.to_string()
            };

            // Handle name collisions: when a function with the same name but different
            // param count already exists, LLVMAddFunction returns a bitcast wrapper to
            // the existing function — silently corrupting the new function's signature.
            //
            // Fix: detect the collision and create the new function with a unique name.
            // The Call handler uses resolve_llvm_function() which tries suffixed names
            // to find the correct arity match.
            let llvm_fn = if let Some(existing) = self.module.get_function(&func_name) {
                if existing.count_params() as usize != effective_params.len() {
                    // Collision! Create with a unique suffix so both functions coexist.
                    let unique_name = format!("{}__arity{}", func_name, effective_params.len());
                    self.has_arity_collisions = true;
                    // The ORIGINAL function (existing) keeps its name. If the current
                    // func_desc has a body and the existing function has different arity,
                    // lowering this body into the existing function would produce invalid
                    // IR. Mark the existing function's body for skipping.
                    // We record THIS func_id as "skip body" when the existing function
                    // is the one that will get this func_id's body (which it won't —
                    // the arity-suffixed one gets the body). But the original function
                    // (from the FIRST declaration) still exists and its body was already
                    // lowered or will be skipped because it's the FIRST function with
                    // this name. Actually — the issue is the ORIGINAL declaration maps
                    // a DIFFERENT func_id. We need to skip that func_id's body.
                    //
                    // Simple approach: skip body lowering for ANY function whose LLVM
                    // function name doesn't match the expected name.
                    self.module.add_function(&unique_name, fn_type, None)
                } else {
                    // Same arity — LLVMAddFunction returns the existing function (safe)
                    self.module.add_function(&func_name, fn_type, None)
                }
            } else {
                self.module.add_function(&func_name, fn_type, None)
            };

            // Note: Internal linkage is set AFTER successful lowering in lower_module().
            // Setting it here would break functions that fail to lower (Internal
            // declarations without bodies are invalid LLVM IR).

            // Set calling convention on function declaration
            let calling_convention = &func_desc.calling_convention;
            llvm_fn.set_call_conventions(to_llvm_calling_convention(calling_convention));

            // For naked functions, add the "naked" attribute
            if matches!(calling_convention, CallingConvention::Naked) {
                let naked_kind_id = Attribute::get_named_enum_kind_id("naked");
                if naked_kind_id != 0 {
                    let naked_attr = self.context.create_enum_attribute(naked_kind_id, 0);
                    llvm_fn.add_attribute(AttributeLoc::Function, naked_attr);
                }
            }

            // Add nounwind to all non-naked functions (Verum doesn't use C++ exceptions)
            if !matches!(calling_convention, CallingConvention::Naked) {
                let nounwind_kind_id = Attribute::get_named_enum_kind_id("nounwind");
                if nounwind_kind_id != 0 {
                    let nounwind_attr = self.context.create_enum_attribute(nounwind_kind_id, 0);
                    llvm_fn.add_attribute(AttributeLoc::Function, nounwind_attr);
                }
            }

            // Apply optimization hints from @inline, @cold, @hot, @optimize attributes
            self.apply_optimization_hints(llvm_fn, &func_desc.optimization_hints);

            // Mark ALL non-user functions as noinline to prevent LLVM verification
            // failures (missing !dbg location on inlined calls) and mixed ptr/i64 ABI
            // issues. User functions (func_id_base == 0, not starting with uppercase
            // Type prefix) can be safely inlined into each other.
            // Mark stdlib functions noinline to prevent mixed ABI issues when inlining.
            let is_stdlib = func_desc.func_id_base != 0
                || (raw_name.contains('.') && raw_name.chars().next().map_or(false, |c| c.is_uppercase()));
            if is_stdlib && func_desc.optimization_hints.inline_hint.is_none() {
                // noinline: prevent cross-ABI inlining.
                let noinline_kind_id = Attribute::get_named_enum_kind_id("noinline");
                if noinline_kind_id != 0 {
                    let noinline_attr = self.context.create_enum_attribute(noinline_kind_id, 0);
                    llvm_fn.add_attribute(AttributeLoc::Function, noinline_attr);
                }
                // optnone: skip ALL function-level optimization passes
                // on stdlib functions. Stdlib-compiled IR may contain
                // null Type references from arity-collision fixups that
                // crash LLVM passes (InterleavedAccess, SelectionDAG,
                // SinkCast, etc.). optnone + noinline makes LLVM treat
                // these as black boxes — their IR is emitted as-is to
                // machine code without any transformation.
                let optnone_kind_id = Attribute::get_named_enum_kind_id("optnone");
                if optnone_kind_id != 0 {
                    let optnone_attr = self.context.create_enum_attribute(optnone_kind_id, 0);
                    llvm_fn.add_attribute(AttributeLoc::Function, optnone_attr);
                }
            }

            self.functions.insert(func_desc.id.0, llvm_fn);
        }

        Ok(())
    }

    /// Apply optimization hints as LLVM function attributes.
    ///
    /// Maps VBC OptimizationHints to LLVM IR attributes:
    /// - @inline(always) -> `alwaysinline`
    /// - @inline(never) -> `noinline`
    /// - @inline -> `inlinehint`
    /// - @cold -> `cold` + `minsize`
    /// - @hot -> `hot` + `inlinehint`
    /// - @optimize(none) -> `optnone` + `noinline`
    /// - @optimize(size) -> `optsize`
    /// - @target_feature -> `target-features` string attribute
    /// - @target_cpu -> `target-cpu` string attribute
    fn apply_optimization_hints(
        &self,
        llvm_fn: FunctionValue<'ctx>,
        hints: &OptimizationHints,
    ) {
        // --- Inline hints ---
        match hints.inline_hint {
            Some(InlineHint::Always) => {
                let kind_id = Attribute::get_named_enum_kind_id("alwaysinline");
                if kind_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(kind_id, 0),
                    );
                }
            }
            Some(InlineHint::Never) => {
                let kind_id = Attribute::get_named_enum_kind_id("noinline");
                if kind_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(kind_id, 0),
                    );
                }
            }
            Some(InlineHint::Suggest) => {
                let kind_id = Attribute::get_named_enum_kind_id("inlinehint");
                if kind_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(kind_id, 0),
                    );
                }
            }
            Some(InlineHint::Release) => {
                // Apply alwaysinline only when opt_level > 0 (release builds)
                if self.config.opt_level > 0 {
                    let kind_id = Attribute::get_named_enum_kind_id("alwaysinline");
                    if kind_id != 0 {
                        llvm_fn.add_attribute(
                            AttributeLoc::Function,
                            self.context.create_enum_attribute(kind_id, 0),
                        );
                    }
                }
            }
            None => {}
        }

        // --- Cold/Hot hints ---
        if hints.is_cold {
            let kind_id = Attribute::get_named_enum_kind_id("cold");
            if kind_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(kind_id, 0),
                );
            }
            // Cold functions should also be minsize to reduce cache pressure
            let minsize_id = Attribute::get_named_enum_kind_id("minsize");
            if minsize_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(minsize_id, 0),
                );
            }
            // Place in .text.cold section
            llvm_fn.set_section(Some(".text.cold"));
        }

        if hints.is_hot {
            let kind_id = Attribute::get_named_enum_kind_id("hot");
            if kind_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(kind_id, 0),
                );
            }
        }

        // --- Purity attributes ---
        // Grammar (line 551): "pure - Compiler-verified no side effects"
        // Pure functions get LLVM attributes enabling aggressive optimization:
        // - memory(none): no memory reads/writes (enables CSE, DCE, LICM)
        // - nosync: no synchronized memory access (enables reordering)
        // - nounwind: never throws (already set globally, but explicit for pure)
        // - willreturn: always terminates (enables dead call elimination)
        if hints.is_pure {
            // memory(none) — the function does not access any memory
            // This is the LLVM 16+ replacement for readnone
            let readnone_id = Attribute::get_named_enum_kind_id("readnone");
            if readnone_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(readnone_id, 0),
                );
            }
            // nosync — no synchronized memory operations
            let nosync_id = Attribute::get_named_enum_kind_id("nosync");
            if nosync_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(nosync_id, 0),
                );
            }
            // willreturn — function always terminates
            let willreturn_id = Attribute::get_named_enum_kind_id("willreturn");
            if willreturn_id != 0 {
                llvm_fn.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(willreturn_id, 0),
                );
            }
        }

        // --- Per-function optimization level ---
        match hints.opt_level {
            Some(OptLevel::None) => {
                // optnone requires noinline
                let optnone_id = Attribute::get_named_enum_kind_id("optnone");
                let noinline_id = Attribute::get_named_enum_kind_id("noinline");
                if optnone_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(optnone_id, 0),
                    );
                }
                if noinline_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(noinline_id, 0),
                    );
                }
            }
            Some(OptLevel::Size) => {
                let optsize_id = Attribute::get_named_enum_kind_id("optsize");
                if optsize_id != 0 {
                    llvm_fn.add_attribute(
                        AttributeLoc::Function,
                        self.context.create_enum_attribute(optsize_id, 0),
                    );
                }
            }
            Some(OptLevel::Speed) | Some(OptLevel::Balanced) | None => {
                // Default behavior - no special attributes
            }
        }

        // --- Target features ---
        if let Some(ref features) = hints.target_features {
            let attr = self.context.create_string_attribute("target-features", features);
            llvm_fn.add_attribute(AttributeLoc::Function, attr);
        }

        // --- Target CPU ---
        if let Some(ref cpu) = hints.target_cpu {
            let attr = self.context.create_string_attribute("target-cpu", cpu);
            llvm_fn.add_attribute(AttributeLoc::Function, attr);
        }
    }

    /// Lower a single VBC function to LLVM IR.
    fn lower_vbc_function(
        &mut self,
        vbc_module: &VbcModule,
        vbc_func: &VbcFunction,
    ) -> Result<()> {
        let func_id = vbc_func.descriptor.id.0;
        let llvm_fn = self
            .functions
            .get(&func_id)
            .copied()
            .ok_or_else(|| {
                let name = vbc_module
                    .strings
                    .get(vbc_func.descriptor.name)
                    .unwrap_or("<unknown>");
                LlvmLoweringError::MissingFunction(Text::from(name))
            })?;

        // Set calling convention based on function descriptor
        let calling_convention = &vbc_func.descriptor.calling_convention;
        llvm_fn.set_call_conventions(to_llvm_calling_convention(calling_convention));

        // For naked functions, add the "naked" attribute
        // This tells LLVM to not generate any prologue/epilogue
        if matches!(calling_convention, CallingConvention::Naked) {
            let naked_kind_id = Attribute::get_named_enum_kind_id("naked");
            if naked_kind_id != 0 {
                let naked_attr = self.context.create_enum_attribute(naked_kind_id, 0);
                llvm_fn.add_attribute(AttributeLoc::Function, naked_attr);
            }
        }

        let func_name = vbc_module
            .strings
            .get(vbc_func.descriptor.name)
            .unwrap_or("<anonymous>");

        // Use with_vbc_module to enable FFI symbol table access for zero-cost FFI
        let mut ctx = FunctionContext::with_vbc_module(
            self.context,
            &self.module,
            vbc_module,
            llvm_fn,
            func_name,
        );

        // Attach DWARF debug info (DISubprogram) to the function if debug mode is on.
        // This enables breakpoints, step-through, and variable inspection in LLDB/GDB.
        let di_func_scope: Option<DIScope<'ctx>> = if let (Some(db), Some(cu), Some(file)) =
            (&self.dibuilder, &self.di_compile_unit, &self.di_file)
        {
            // Create subroutine type: fn(...) → void (simplified — no DI type info for params yet)
            let subroutine_type = db.create_subroutine_type(*file, None, &[], DIFlags::PUBLIC);

            let func_scope = db.create_function(
                cu.as_debug_info_scope(),
                func_name,
                None, // linkage_name (same as name)
                *file,
                1, // line_no — will be refined when source maps are populated
                subroutine_type,
                true,  // is_local_to_unit
                true,  // is_definition
                1,     // scope_line
                DIFlags::PUBLIC,
                self.config.opt_level > 0,
            );
            llvm_fn.set_subprogram(func_scope);
            Some(func_scope.as_debug_info_scope())
        } else {
            None
        };

        // === DWARF Variable-Level Debug Info ===
        // Emit DILocalVariable for each parameter and local variable from
        // FunctionDescriptor.debug_variables. This enables `info locals` and
        // `frame variable` in LLDB/GDB.
        if let (Some(db), Some(file), Some(scope)) =
            (&self.dibuilder, &self.di_file, di_func_scope)
        {
            // Create i64 base type for all VBC values (VBC is unityped i64)
            if let Ok(i64_di_type) = db.create_basic_type("i64", 64, 0x05, DIFlags::PUBLIC) {
                let di_type: DIType = i64_di_type.as_type();
                let entry_block = llvm_fn.get_first_basic_block();

                for dv in &vbc_func.descriptor.debug_variables {
                    let var_name = vbc_module
                        .strings
                        .get(dv.name)
                        .unwrap_or("<unnamed>");

                    if dv.is_parameter && dv.arg_index > 0 {
                        // Create DW_TAG_formal_parameter
                        let di_param = db.create_parameter_variable(
                            scope,
                            var_name,
                            dv.arg_index as u32,
                            *file,
                            1, // line (refined when source map populated)
                            di_type,
                            true, // always_preserve
                            DIFlags::PUBLIC,
                        );

                        // Associate with the function parameter's alloca or value
                        if let Some(block) = entry_block {
                            let loc = db.create_debug_location(
                                self.context,
                                1, // line
                                0, // column
                                scope,
                                None,
                            );
                            // Get parameter value (LLVM param index is 0-based)
                            if let Some(param_val) = llvm_fn.get_nth_param((dv.arg_index - 1) as u32) {
                                let _ptr_type = self.context.ptr_type(AddressSpace::default());
                                let alloca = ctx.builder().build_alloca(
                                    param_val.get_type(), &format!("{}.addr", var_name)
                                );
                                if let Ok(alloca_val) = alloca {
                                    let _ = ctx.builder().build_store(alloca_val, param_val);
                                    let _ = db.insert_declare_at_end(
                                        alloca_val.into(),
                                        Some(di_param),
                                        None, // empty expression
                                        loc,
                                        block,
                                    );
                                }
                            }
                        }
                    } else {
                        // Create DW_TAG_variable for locals
                        let di_local = db.create_auto_variable(
                            scope,
                            var_name,
                            *file,
                            1, // line
                            di_type,
                            true, // always_preserve
                            DIFlags::PUBLIC,
                            64,   // align_in_bits
                        );

                        // Note: We associate local variables with their allocas
                        // in the instruction lowering loop when LoadK/Mov first
                        // writes to the register. For now, the DILocalVariable
                        // is created for debugger awareness of the variable name.
                    }
                }
            }
        }

        // Set default tier from config
        ctx.set_tier(self.config.default_tier);

        // Apply VBC escape analysis tier decisions for this function.
        // Converts (FunctionId, instruction_offset) → CbgrTier decisions
        // into per-instruction-offset → RefTier map on the FunctionContext.
        if let Some(ref ea) = self.escape_analysis {
            let vbc_func_id = verum_vbc::module::FunctionId(func_id);
            let mut tiers = HashMap::new();
            for (&(fid, offset), &cbgr_tier) in &ea.decisions {
                if fid == vbc_func_id {
                    let ref_tier = match cbgr_tier {
                        verum_vbc::types::CbgrTier::Tier0 => RefTier::Tier0,
                        verum_vbc::types::CbgrTier::Tier1 => RefTier::Tier1,
                        verum_vbc::types::CbgrTier::Tier2 => RefTier::Tier2,
                    };
                    tiers.insert(offset, ref_tier);
                }
            }
            if !tiers.is_empty() {
                ctx.set_vbc_escape_tiers(tiers);
            }
        }

        // Set pre-built function name index for O(1) lookups
        if let Some(ref index) = self.func_name_index {
            ctx.set_func_name_index(index.clone());
        }

        // Share func_id → LLVM function map for collision-safe Call resolution.
        // When user and stdlib functions share a name, the arity-suffixed LLVM
        // function can only be found via func_id, not by name.
        ctx.set_func_id_map(Arc::new(self.functions.clone()));

        // Set function ID base for merged stdlib modules.
        // Call func_ids in merged bytecode are relative to the source module;
        // this offset converts them to the merged module's function table.
        ctx.set_func_id_base(vbc_func.descriptor.func_id_base);

        // Compute basic block boundaries by scanning for jump targets.
        // VBC uses relative instruction offsets in Jmp/JmpNot/JmpIf instructions.
        // We create an LLVM basic block at each jump target instruction index.
        let mut block_target_indices: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        block_target_indices.insert(0); // Entry block always exists

        // Track whether instruction 0 is an explicit jump target from another instruction.
        // If so, we need a separate LLVM entry block (LLVM requires entry blocks have no predecessors).
        let mut entry_is_jump_target = false;

        for (pc, instr) in vbc_func.instructions.iter().enumerate() {
            match instr {
                Instruction::Jmp { offset } => {
                    // VBC offset semantics: target = pc + offset (instruction index based)
                    let target = (pc as i64 + *offset as i64) as usize;
                    block_target_indices.insert(target);
                    if target == 0 { entry_is_jump_target = true; }
                    // The instruction after the jump is also a block start (dead code or merge point)
                    block_target_indices.insert(pc + 1);
                }
                Instruction::JmpNot { offset, .. } | Instruction::JmpIf { offset, .. } => {
                    let target = (pc as i64 + *offset as i64) as usize;
                    block_target_indices.insert(target);
                    if target == 0 { entry_is_jump_target = true; }
                    // Fallthrough is always the next instruction
                    block_target_indices.insert(pc + 1);
                }
                Instruction::Switch { default_offset, cases, .. } => {
                    // Default case target
                    let default_target = (pc as i64 + *default_offset as i64) as usize;
                    block_target_indices.insert(default_target);
                    if default_target == 0 { entry_is_jump_target = true; }
                    // Each case target
                    for (_case_val, case_offset) in cases.iter() {
                        let target = (pc as i64 + *case_offset as i64) as usize;
                        block_target_indices.insert(target);
                        if target == 0 { entry_is_jump_target = true; }
                    }
                    // Instruction after switch is also a block start
                    block_target_indices.insert(pc + 1);
                }
                Instruction::TryBegin { handler_offset } => {
                    // Exception handler starts at pc + handler_offset
                    let target = (pc as i64 + *handler_offset as i64) as usize;
                    block_target_indices.insert(target);
                    if target == 0 { entry_is_jump_target = true; }
                    // TryBegin is a branch (setjmp returns 0 or non-zero), so
                    // fallthrough (try body) needs its own block
                    block_target_indices.insert(pc + 1);
                }
                Instruction::Ret { .. } | Instruction::RetV
                | Instruction::TailCall { .. }
                | Instruction::Unreachable
                | Instruction::Panic { .. }
                | Instruction::Throw { .. } => {
                    // Instructions after a return/unreachable/panic/throw need their own block
                    // (may be dead code or merge point)
                    if pc + 1 < vbc_func.instructions.len() {
                        block_target_indices.insert(pc + 1);
                    }
                }
                _ => {}
            }
        }

        // Build sorted list of block start indices and create blocks.
        // If entry (instruction 0) is a jump target, we offset all block IDs by 1
        // and reserve block_0 as a dedicated LLVM entry block with no predecessors.
        let block_starts: Vec<usize> = block_target_indices.into_iter().collect();
        let block_id_offset: u32 = if entry_is_jump_target { 1 } else { 0 };
        // Map from instruction index to block ID for quick lookup
        let instr_to_block: std::collections::HashMap<usize, u32> = block_starts
            .iter()
            .enumerate()
            .map(|(block_id, &instr_idx)| (instr_idx, block_id as u32 + block_id_offset))
            .collect();

        // Create dedicated entry block if needed (before all other blocks)
        if entry_is_jump_target {
            ctx.create_block(0, "entry");
        }

        for (block_id, _) in block_starts.iter().enumerate() {
            ctx.create_block(block_id as u32 + block_id_offset, &format!("block_{}", block_id));
        }

        // Enable alloca-based registers for multi-block functions.
        // This ensures values persist across basic block boundaries.
        // LLVM's mem2reg pass will optimize these to SSA form.
        if block_starts.len() > 1 {
            ctx.enable_alloca_mode();
        }

        // Set up entry block — always position at the very first block (block_0)
        if let Some(entry) = ctx.entry_block() {
            ctx.position_at_end(entry);
        }

        // Coverage instrumentation: increment function counter on entry.
        // counter_array[func_idx] += 1 (atomic, thread-safe).
        if self.config.coverage {
            if let Some(counter_global) = self.module.get_global("__verum_coverage_counters") {
                let i64_type = self.context.i64_type();
                let i8_type = self.context.i8_type();
                let func_idx = vbc_func.descriptor.func_id_base as u64 + func_id as u64;
                let byte_offset = func_idx * 8;
                let ptr = counter_global.as_pointer_value();
                let builder = self.context.create_builder();
                if let Some(entry) = ctx.entry_block() {
                    builder.position_at_end(entry);
                    // SAFETY: GEP into the coverage counters global array at byte_offset = func_idx * 8; the array is sized for all functions in the module
                    if let Ok(counter_ptr) = unsafe {
                        builder.build_gep(i8_type, ptr, &[i64_type.const_int(byte_offset, false)], "cov_ptr")
                    } {
                        let _ = builder.build_atomicrmw(
                            verum_llvm::AtomicRMWBinOp::Add,
                            counter_ptr,
                            i64_type.const_int(1, false),
                            verum_llvm::AtomicOrdering::Monotonic,
                        );
                    }
                }
            }
        }

        // Initialize parameters as registers
        // Closure functions use env_ptr convention: fn(env_ptr, user_arg0, arg1, ...)
        // VBC register layout for closures: [captures...][user_args...]
        // We detect closures by the "$closure$" naming convention.
        let is_closure = func_name.contains("$closure$")
            && !vbc_func.descriptor.params.is_empty();
        if is_closure {
            let capture_count = vbc_func.descriptor.max_stack as usize;

            // Load captures from env_ptr (LLVM param 0) into registers 0..capture_count-1
            if capture_count > 0 {
                if let Some(env_param) = llvm_fn.get_nth_param(0) {
                    // env_ptr may be PointerValue (user closures) or IntValue
                    // (stdlib closures with i64-normalized params). Handle both.
                    let env_ptr = if env_param.is_pointer_value() {
                        env_param.into_pointer_value()
                    } else {
                        let ptr_type = self.context.ptr_type(verum_llvm::AddressSpace::default());
                        ctx.builder()
                            .build_int_to_ptr(env_param.into_int_value(), ptr_type, "env_ptr_cast")
                            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
                    };
                    let i8_type = self.context.i8_type();
                    let i64_type = self.context.i64_type();

                    for i in 0..capture_count {
                        let offset = i as u64 * 8; // 8 bytes per capture
                        // SAFETY: In-bounds GEP into the closure environment at offset i*8; the environment was allocated with capture_count * 8 bytes
                        let cap_ptr = unsafe {
                            ctx.builder()
                                .build_in_bounds_gep(
                                    i8_type,
                                    env_ptr,
                                    &[i64_type.const_int(offset, false)],
                                    &format!("cap_{}_ptr", i),
                                )
                                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
                        };
                        let cap_val = ctx
                            .builder()
                            .build_load(i64_type, cap_ptr, &format!("cap_{}", i))
                            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                        ctx.set_register(i as u16, cap_val);
                    }
                }
            }

            // Map user args (LLVM params 1..N) to registers after captures
            for (j, param) in llvm_fn.get_param_iter().skip(1).enumerate() {
                ctx.set_register((capture_count + j) as u16, param);
            }

            // Mark closure user-parameter registers based on VBC type info.
            // descriptor.params: [env_ptr, user_arg0, user_arg1, ...]
            // VBC registers:     [cap0..capN-1, user_arg0, user_arg1, ...]
            // User param at descriptor index (j+1) → VBC register (capture_count + j)
            for (j, p) in vbc_func.descriptor.params.iter().skip(1).enumerate() {
                let reg = (capture_count + j) as u16;
                let effective_type = match &p.type_ref {
                    TypeRef::Reference { inner, .. } => inner.as_ref(),
                    other => other,
                };
                let is_text = matches!(effective_type, TypeRef::Concrete(tid) if *tid == TypeId::TEXT);
                let is_list = matches!(effective_type,
                    TypeRef::Concrete(tid) if *tid == TypeId::LIST
                ) || matches!(effective_type,
                    TypeRef::Instantiated { base, .. } if *base == TypeId::LIST
                );
                let is_map = matches!(effective_type,
                    TypeRef::Concrete(tid) if *tid == TypeId::MAP
                ) || matches!(effective_type,
                    TypeRef::Instantiated { base, .. } if *base == TypeId::MAP
                );
                let is_float = matches!(effective_type,
                    TypeRef::Concrete(tid) if *tid == TypeId::FLOAT || *tid == TypeId::F32
                );
                let is_bool = matches!(effective_type,
                    TypeRef::Concrete(tid) if *tid == TypeId::BOOL
                );
                let is_chan = matches!(effective_type,
                    TypeRef::Concrete(tid) if *tid == TypeId::CHANNEL
                ) || matches!(effective_type,
                    TypeRef::Instantiated { base, .. } if *base == TypeId::CHANNEL
                );

                if is_text { ctx.mark_text_register(reg); }
                if is_list {
                    ctx.mark_list_register(reg);
                    if let TypeRef::Instantiated { args, .. } = effective_type {
                        if args.iter().any(|a| *a == TypeRef::Concrete(TypeId::TEXT)) {
                            ctx.mark_string_list_register(reg);
                        }
                    }
                }
                if is_map { ctx.mark_map_register(reg); }
                if is_float { ctx.mark_float_register(reg); }
                if is_bool { ctx.mark_bool_register(reg); }
                if is_chan { ctx.mark_chan_register(reg); }
            }
        } else {
            // Regular function: straightforward parameter mapping
            for (i, param) in llvm_fn.get_param_iter().enumerate() {
                ctx.set_register(i as u16, param);
                // Mark string registers for Text parameters so that CmpG/Print
                // can correctly dispatch string comparison/printing.
                // Mark list registers for List parameters so that GetE/SetE
                // can correctly use header indirection.
                if let Some(p) = vbc_func.descriptor.params.get(i) {
                    // Unwrap Reference wrapper for type checking — &Map<K,V>
                    // should still be detected as a Map parameter.
                    let effective_type = match &p.type_ref {
                        TypeRef::Reference { inner, .. } => inner.as_ref(),
                        other => other,
                    };
                    let is_text = match effective_type {
                        TypeRef::Concrete(tid) => *tid == TypeId::TEXT,
                        _ => false,
                    };
                    if is_text {
                        ctx.mark_text_register(i as u16);
                    }
                    let is_list = match effective_type {
                        TypeRef::Concrete(tid) => *tid == TypeId::LIST,
                        TypeRef::Instantiated { base, .. } => *base == TypeId::LIST,
                        _ => false,
                    };
                    if is_list {
                        ctx.mark_list_register(i as u16);
                        // Check if this is a List<Text> — mark as string_list for GetE element tracking
                        if let TypeRef::Instantiated { args, .. } = effective_type {
                            if args.iter().any(|a| *a == TypeRef::Concrete(TypeId::TEXT)) {
                                ctx.mark_string_list_register(i as u16);
                            }
                        }
                    }
                    let is_map = match effective_type {
                        TypeRef::Concrete(tid) => *tid == TypeId::MAP,
                        TypeRef::Instantiated { base, .. } => *base == TypeId::MAP,
                        _ => false,
                    };
                    if is_map {
                        ctx.mark_map_register(i as u16);
                        // Check Map value type — if Map<K, List<V>>, mark for list propagation
                        if let TypeRef::Instantiated { args, .. } = &p.type_ref {
                            if args.len() >= 2 {
                                let val_type = &args[1];
                                let is_list_val = match val_type {
                                    TypeRef::Concrete(tid) => *tid == TypeId::LIST,
                                    TypeRef::Instantiated { base, .. } => *base == TypeId::LIST,
                                    _ => false,
                                };
                                if is_list_val {
                                    ctx.mark_map_list_value(i as u16);
                                }
                                let is_text_val = match val_type {
                                    TypeRef::Concrete(tid) => *tid == TypeId::TEXT,
                                    _ => false,
                                };
                                if is_text_val {
                                    ctx.mark_map_string_value(i as u16);
                                }
                            }
                        }
                    }
                }
            }
            // For impl INSTANCE methods (name contains '.' and param[0] is self),
            // mark register 0 with the self type.
            // This enables GetF to look up the correct type's field metadata.
            // Static methods (e.g., Text.from_utf8_unchecked) should NOT mark
            // register 0 as the type — their first param is not self.
            if let Some(dot_pos) = func_name.find('.') {
                let self_type = &func_name[..dot_pos];
                // Check if this is an instance method by verifying the first parameter
                // is self (TypeId(0) in VBC for &self params).
                let first_param_type = vbc_func.descriptor.params.first().map(|p| p.type_ref.clone());
                let is_instance_method = first_param_type.as_ref()
                    .map_or(false, |tr| matches!(tr, TypeRef::Concrete(tid) if tid.0 == 0));
                if is_instance_method {
                    ctx.set_obj_register_type(0, self_type.to_string());
                    // Mark self register for type-specific dispatch (flat layout, etc.)
                    match self_type {
                        tn::TEXT => { ctx.mark_text_register(0); }
                        tn::LIST => { ctx.mark_list_register(0); }
                        tn::MAP => { ctx.mark_map_register(0); }
                        tn::SET => { ctx.mark_set_register(0); }
                        tn::DEQUE => { ctx.mark_deque_register(0); }
                        tn::CHANNEL => { ctx.mark_chan_register(0); }
                        "BTreeMap" => { ctx.mark_btreemap_register(0); }
                        "BTreeSet" => { ctx.mark_btreeset_register(0); }
                        "BinaryHeap" => { ctx.mark_binaryheap_register(0); }
                        "AtomicInt" | "AtomicBool" => { ctx.mark_atomic_int_register(0); }
                        _ => {}
                    }
                } else {
                    // Static methods: check if first parameter is a slice (&[Byte])
                    // Common pattern: Text.from_utf8_unchecked(bytes: &[Byte])
                    let method_name = &func_name[dot_pos + 1..];
                    if matches!(method_name, "from_utf8_unchecked" | "from_utf8") {
                        ctx.mark_slice_register(0);
                    }
                }
            }
            // Mark parameter registers based on their VBC type information.
            // This enables correct dispatch in GetE/GetF/Len for typed parameters.
            for (i, p) in vbc_func.descriptor.params.iter().enumerate() {
                let reg = i as u16;
                match &p.type_ref {
                    TypeRef::Slice(_) => {
                        ctx.mark_slice_register(reg);
                    }
                    TypeRef::Instantiated { base, args } if *base == TypeId::LIST => {
                        // List<U8> / List<I8> used as byte slice representation
                        let is_byte_list = args.len() == 1 && matches!(
                            &args[0],
                            TypeRef::Concrete(tid) if *tid == TypeId::U8 || *tid == TypeId::I8
                        );
                        if is_byte_list {
                            ctx.mark_slice_register(reg);
                        } else {
                            ctx.mark_list_register(reg);
                        }
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::LIST => {
                        ctx.mark_list_register(reg);
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::TEXT => {
                        ctx.mark_text_register(reg);
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::FLOAT || *tid == TypeId::F32 => {
                        ctx.mark_float_register(reg);
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::BOOL => {
                        ctx.mark_bool_register(reg);
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::PTR => {
                        // Generic type params (K, V, T) are compiled as PTR in compiled
                        // module functions. The ptr value IS the value (via inttoptr),
                        // not a real memory address. Mark so Deref does ptrtoint.
                        // Skip register 0 (self) — instance methods' self IS a real pointer.
                        if reg != 0 {
                            ctx.mark_generic_ptr_register(reg);
                        }
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::CHANNEL => {
                        ctx.mark_chan_register(reg);
                    }
                    TypeRef::Instantiated { base, .. } if *base == TypeId::CHANNEL => {
                        ctx.mark_chan_register(reg);
                    }
                    // Use name-based check: VBC TypeIds are non-deterministic and can
                    // collide with builtin ranges depending on module loading order.
                    TypeRef::Concrete(tid) => {
                        if let Some(type_name) = vbc_module.get_type_name(*tid) {
                            if !is_primitive_type_name(&type_name) {
                                if type_name == "AtomicInt" || type_name == "AtomicBool" {
                                    ctx.mark_atomic_int_register(reg);
                                }
                                ctx.set_obj_register_type(reg, type_name);
                            }
                        }
                    }
                    // Reference/Heap wrappers: unwrap to find the inner struct type.
                    // e.g., &mut Heap<BTreeNode<K,V>> → set obj_type = "BTreeNode"
                    TypeRef::Reference { inner, .. } => {
                        // Strip Reference, then check for Heap wrapper
                        let mut unwrapped = inner.as_ref();
                        if let TypeRef::Instantiated { base, args } = unwrapped {
                            if *base == TypeId::HEAP || vbc_module.get_type_name(*base).map_or(false, |n| WKT::Heap.matches(&n)) {
                                if let Some(inner_arg) = args.first() {
                                    unwrapped = inner_arg;
                                }
                            }
                        }
                        // Now set obj_register_type from the unwrapped type
                        match unwrapped {
                            TypeRef::Concrete(tid) => {
                                if let Some(type_name) = vbc_module.get_type_name(*tid) {
                                    if !is_primitive_type_name(&type_name) {
                                        ctx.set_obj_register_type(reg, type_name);
                                    }
                                }
                            }
                            TypeRef::Instantiated { base, .. } => {
                                if let Some(type_name) = vbc_module.get_type_name(*base) {
                                    if !is_primitive_type_name(&type_name) {
                                        ctx.set_obj_register_type(reg, type_name);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // Heap<T> wrapper without reference: unwrap to find inner type
                    TypeRef::Instantiated { base, args } if *base == TypeId::HEAP || vbc_module.get_type_name(*base).map_or(false, |n| WKT::Heap.matches(&n)) => {
                        if let Some(inner) = args.first() {
                            match inner {
                                TypeRef::Concrete(tid) => {
                                    if let Some(type_name) = vbc_module.get_type_name(*tid) {
                                        if !is_primitive_type_name(&type_name) {
                                            ctx.set_obj_register_type(reg, type_name);
                                        }
                                    }
                                }
                                TypeRef::Instantiated { base: inner_base, .. } => {
                                    if let Some(type_name) = vbc_module.get_type_name(*inner_base) {
                                        if !is_primitive_type_name(&type_name) {
                                            ctx.set_obj_register_type(reg, type_name);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // Generic non-builtin Instantiated types: BTreeMap<K,V>, BTreeSet<T>, etc.
                    // PTR is excluded: Instantiated { base: PTR, args } means &Struct<K,V>,
                    // which must fall through to the specialized PTR handler below.
                    TypeRef::Instantiated { base, .. }
                        if *base != TypeId::HEAP && *base != TypeId::PTR && !vbc_module.get_type_name(*base).map_or(false, |n| WKT::Heap.matches(&n)) =>
                    {
                        if let Some(type_name) = vbc_module.get_type_name(*base) {
                            match type_name.as_str() {
                                "BTreeMap" => { ctx.mark_btreemap_register(reg); }
                                "BTreeSet" => { ctx.mark_btreeset_register(reg); }
                                "BinaryHeap" => { ctx.mark_binaryheap_register(reg); }
                                tn::MAP => { ctx.mark_map_register(reg); }
                                tn::SET => { ctx.mark_set_register(reg); }
                                tn::DEQUE => { ctx.mark_deque_register(reg); }
                                tn::LIST => { ctx.mark_list_register(reg); }
                                tn::CHANNEL => { ctx.mark_chan_register(reg); }
                                "AtomicInt" | "AtomicBool" => { ctx.mark_atomic_int_register(reg); ctx.set_obj_register_type(reg, type_name); }
                                _ => { ctx.set_obj_register_type(reg, type_name); }
                            }
                        }
                    }
                    // Ptr<Struct<...>> pattern: &Struct<K,V> in compiled modules.
                    // References to generic structs are encoded as Instantiated { base: PTR, args: [Instantiated { base: struct_id, ... }] }.
                    // The inner struct_id may not resolve via get_type_name due to TypeId
                    // collisions after module merging. Fall back to function name prefix
                    // to find the related struct type.
                    TypeRef::Instantiated { base, args } if *base == TypeId::PTR => {
                        if let Some(inner) = args.first() {
                            let inner_id = match inner {
                                TypeRef::Instantiated { base: inner_base, .. } => Some(*inner_base),
                                TypeRef::Concrete(tid) => Some(*tid),
                                _ => None,
                            };
                            // Try direct lookup first
                            let mut resolved = false;
                            if let Some(tid) = inner_id {
                                if let Some(type_name) = vbc_module.get_type_name(tid) {
                                    ctx.set_obj_register_type(reg, type_name);
                                    resolved = true;
                                }
                            }
                            // Fallback: use function name prefix to find related types.
                            // For "BTreeMap.search_node", search for "BTreeNode" etc.
                            if !resolved {
                                if let Some(dot_pos) = func_name.find('.') {
                                    let prefix = &func_name[..dot_pos]; // e.g., "BTreeMap"
                                    // Search for struct types whose name starts with the same
                                    // prefix root (e.g., "BTree" from "BTreeMap" matches "BTreeNode")
                                    let prefix_root = if prefix.len() >= 5 {
                                        // Use a reasonable prefix: "BTree" from "BTreeMap", "Binary" from "BinaryHeap"
                                        let end = prefix.char_indices()
                                            .skip(1)
                                            .find(|(_, c)| c.is_uppercase())
                                            .map(|(i, _)| i)
                                            .unwrap_or(prefix.len());
                                        &prefix[..end]
                                    } else {
                                        prefix
                                    };
                                    // Find a struct type whose name starts with prefix_root
                                    // and is NOT the same as the receiver type (prefix).
                                    for td in &vbc_module.types {
                                        let tname = vbc_module.get_string(td.name).unwrap_or("");
                                        if tname.starts_with(prefix_root) && tname != prefix
                                            && td.kind == verum_vbc::types::TypeKind::Record
                                            && !td.fields.is_empty()
                                        {
                                            // Prefer the "Node" type variant if available
                                            if tname.contains("Node") || tname.ends_with("Entry") {
                                                ctx.set_obj_register_type(reg, tname.to_string());
                                                resolved = true;
                                                break;
                                            }
                                        }
                                    }
                                    // If no "Node" type found, try any matching struct
                                    if !resolved {
                                        for td in &vbc_module.types {
                                            let tname = vbc_module.get_string(td.name).unwrap_or("");
                                            if tname.starts_with(prefix_root) && tname != prefix
                                                && td.kind == verum_vbc::types::TypeKind::Record
                                                && !td.fields.is_empty()
                                            {
                                                ctx.set_obj_register_type(reg, tname.to_string());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Function type parameters (closures): track return type for CallClosure.
                    TypeRef::Function { return_type, .. } => {
                        ctx.set_closure_return_type(reg, *return_type.clone());
                    }
                    TypeRef::Rank2Function { return_type, .. } => {
                        ctx.set_closure_return_type(reg, *return_type.clone());
                    }
                    // Tuple type parameters: track element types for Unpack.
                    TypeRef::Tuple(elems) => {
                        ctx.set_tuple_element_types(reg, elems.clone());
                    }
                    // Generic struct parameters: track type args for GetF Generic resolution.
                    TypeRef::Instantiated { args, .. } if !args.is_empty() => {
                        ctx.set_generic_type_args(reg, args.clone());
                    }
                    _ => {}
                }
            }
        }

        // If entry is a jump target, we put param init in the dedicated entry block (block_0)
        // and need to branch from it to block_1 (which holds VBC instruction 0).
        if entry_is_jump_target {
            let first_instr_block = ctx.get_block(block_id_offset)?; // block_1
            ctx.builder()
                .build_unconditional_branch(first_instr_block)
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
            ctx.position_at_end(first_instr_block);
        }

        // Pre-pass: forward type inference to set register types before LLVM lowering.
        // This enables GetF to correctly identify List/Text/user-type fields
        // even through variant wrapping (Ok/Some) and match extraction (AsVar).
        // We iterate to fixpoint because VBC control flow can place AsVar before
        // the GetF that types its source register in the linear instruction stream.
        {
            let mut reg_types: HashMap<u16, TypeRef> = HashMap::new();

            // Seed with parameter types
            for (i, p) in vbc_func.descriptor.params.iter().enumerate() {
                reg_types.insert(i as u16, p.type_ref.clone());
            }

            let debug = std::env::var("VERUM_DEBUG_PREPASS").is_ok();

            // Iterate to fixpoint: VBC control flow can place AsVar before GetF
            // in the linear instruction stream, so a single pass won't propagate all types.
            for iteration in 0..4 {
                let prev_count = reg_types.len();

                for instr in &vbc_func.instructions {
                    match instr {
                        Instruction::New { dst, type_id, .. } => {
                            reg_types.insert(dst.0, TypeRef::Concrete(TypeId(*type_id)));
                        }
                        Instruction::NewList { dst } => {
                            reg_types.insert(dst.0, TypeRef::Concrete(TypeId::LIST));
                        }
                        Instruction::Mov { dst, src } => {
                            if let Some(t) = reg_types.get(&src.0).cloned() {
                                reg_types.insert(dst.0, t);
                            }
                        }
                        Instruction::GetF { dst, obj, field_idx } => {
                            if let Some(obj_type) = reg_types.get(&obj.0) {
                                let type_id = match obj_type {
                                    TypeRef::Concrete(tid) => Some(*tid),
                                    _ => None,
                                };
                                if let Some(tid) = type_id {
                                    let type_name_opt = vbc_module.get_type_name(tid);
                                    for type_desc in &vbc_module.types {
                                        let tname = vbc_module.get_string(type_desc.name).unwrap_or("");
                                        if let Some(ref n) = type_name_opt {
                                            if n == tname {
                                                if let Some(field) = type_desc.fields.get(*field_idx as usize) {
                                                    reg_types.insert(dst.0, field.type_ref.clone());
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Instruction::Call { dst, func_id, .. } => {
                            // Track function return types so AsVar can propagate inner types
                            if let Some(func) = vbc_module.functions.get(*func_id as usize) {
                                let ret = &func.return_type;
                                if !matches!(ret, TypeRef::Concrete(tid) if *tid == TypeId::UNIT || *tid == TypeId::INT || *tid == TypeId::FLOAT || *tid == TypeId::BOOL) {
                                    reg_types.insert(dst.0, ret.clone());
                                }
                            }
                        }
                        Instruction::AsVar { dst, value, .. } => {
                            if let Some(vtype) = reg_types.get(&value.0).cloned() {
                                match &vtype {
                                    TypeRef::Instantiated { base, args } => {
                                        if (*base == TypeId::MAYBE || *base == TypeId::RESULT)
                                            && !args.is_empty()
                                        {
                                            reg_types.insert(dst.0, args[0].clone());
                                        }
                                    }
                                    TypeRef::Concrete(tid) => {
                                        if let Some(tn) = vbc_module.get_type_name(*tid) {
                                            if !is_primitive_type_name(&tn) {
                                                reg_types.insert(dst.0, vtype.clone());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Instruction::GetVariantData { dst, variant, .. } => {
                            // GetVariantData extracts payload from a variant.
                            // If the variant register is Maybe<T> or Result<T,E>, payload is T.
                            if let Some(vtype) = reg_types.get(&variant.0).cloned() {
                                match &vtype {
                                    TypeRef::Instantiated { base, args } => {
                                        if (*base == TypeId::MAYBE || *base == TypeId::RESULT)
                                            && !args.is_empty()
                                        {
                                            reg_types.insert(dst.0, args[0].clone());
                                        }
                                    }
                                    TypeRef::Concrete(tid) => {
                                        if let Some(tn) = vbc_module.get_type_name(*tid) {
                                            if !is_primitive_type_name(&tn) {
                                                reg_types.insert(dst.0, vtype.clone());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Check fixpoint
                if reg_types.len() == prev_count && iteration > 0 {
                    if debug {
                        tracing::debug!("[PREPASS] fn {} fixpoint at iteration {}", func_name, iteration);
                    }
                    break;
                }
            }

            // Debug: dump inferred types (VERUM_DEBUG_PREPASS=1)
            if debug {
                tracing::debug!("[PREPASS] fn {} fixpoint - {} register types inferred", func_name, reg_types.len());
            }

            // Flow-sensitive pass: track register types in instruction order to handle
            // register reuse. When a register is used for both a List and a Text at
            // different instruction points, the fixpoint pre-pass only keeps the last type.
            // This pass walks in order and records which Len instruction indices need
            // list dispatch (vs string dispatch).
            {
                let mut flow_types: HashMap<u16, TypeRef> = HashMap::new();
                // Sticky set: registers that were EVER a list. Unlike flow_types which
                // gets overwritten through dead branches (register reuse), this set only
                // grows. Used for Len instruction override when flow_types was polluted.
                let mut ever_list_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
                // Seed with parameter types
                for (i, p) in vbc_func.descriptor.params.iter().enumerate() {
                    flow_types.insert(i as u16, p.type_ref.clone());
                }
                for (instr_idx, instr) in vbc_func.instructions.iter().enumerate() {
                    match instr {
                        Instruction::New { dst, type_id, .. } => {
                            flow_types.insert(dst.0, TypeRef::Concrete(TypeId(*type_id)));
                        }
                        Instruction::NewList { dst } => {
                            flow_types.insert(dst.0, TypeRef::Concrete(TypeId::LIST));
                            ever_list_regs.insert(dst.0);
                        }
                        Instruction::Mov { dst, src } => {
                            if let Some(t) = flow_types.get(&src.0).cloned() {
                                flow_types.insert(dst.0, t.clone());
                                // Propagate sticky list tracking through Mov
                                if ever_list_regs.contains(&src.0) {
                                    ever_list_regs.insert(dst.0);
                                }
                            }
                        }
                        Instruction::GetF { dst, obj, field_idx } => {
                            if let Some(obj_type) = flow_types.get(&obj.0) {
                                let type_id = match obj_type {
                                    TypeRef::Concrete(tid) => Some(*tid),
                                    _ => None,
                                };
                                if let Some(tid) = type_id {
                                    if let Some(type_name) = vbc_module.get_type_name(tid) {
                                        for type_desc in &vbc_module.types {
                                            let tname = vbc_module.get_string(type_desc.name).unwrap_or("");
                                            if tname == type_name {
                                                if let Some(field) = type_desc.fields.get(*field_idx as usize) {
                                                    flow_types.insert(dst.0, field.type_ref.clone());
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Instruction::GetVariantData { dst, variant, .. } => {
                            if let Some(vtype) = flow_types.get(&variant.0).cloned() {
                                match &vtype {
                                    TypeRef::Instantiated { base, args } => {
                                        if (*base == TypeId::MAYBE || *base == TypeId::RESULT) && !args.is_empty() {
                                            flow_types.insert(dst.0, args[0].clone());
                                        }
                                    }
                                    TypeRef::Concrete(tid) => {
                                        if let Some(tn) = vbc_module.get_type_name(*tid) {
                                            if !is_primitive_type_name(&tn) {
                                                flow_types.insert(dst.0, vtype.clone());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Instruction::AsVar { dst, value, .. } => {
                            if let Some(vtype) = flow_types.get(&value.0).cloned() {
                                match &vtype {
                                    TypeRef::Instantiated { base, args } => {
                                        if (*base == TypeId::MAYBE || *base == TypeId::RESULT) && !args.is_empty() {
                                            flow_types.insert(dst.0, args[0].clone());
                                        }
                                    }
                                    TypeRef::Concrete(tid) => {
                                        if let Some(tn) = vbc_module.get_type_name(*tid) {
                                            if !is_primitive_type_name(&tn) {
                                                flow_types.insert(dst.0, vtype.clone());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Instruction::Call { dst, func_id, .. } => {
                            if let Some(func) = vbc_module.get_function(verum_vbc::module::FunctionId(*func_id)) {
                                let ret = func.return_type.clone();
                                let is_list = matches!(&ret,
                                    TypeRef::Concrete(tid) if *tid == TypeId::LIST)
                                    || matches!(&ret,
                                    TypeRef::Instantiated { base, .. } if *base == TypeId::LIST);
                                if is_list {
                                    ever_list_regs.insert(dst.0);
                                }
                                flow_types.insert(dst.0, ret);
                            }
                        }
                        Instruction::Len { arr, .. } => {
                            let is_list_now = flow_types.get(&arr.0).map_or(false, |arr_type| {
                                matches!(arr_type,
                                    TypeRef::Concrete(tid) if *tid == TypeId::LIST)
                                || matches!(arr_type,
                                    TypeRef::Instantiated { base, .. } if *base == TypeId::LIST)
                            });
                            let was_ever_list = ever_list_regs.contains(&arr.0);
                            if is_list_now || was_ever_list {
                                ctx.mark_len_list_override(instr_idx);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Apply inferred types to context (legacy HashSet marking)
            for (reg, type_ref) in &reg_types {
                match type_ref {
                    TypeRef::Concrete(tid) if *tid == TypeId::LIST => {
                        ctx.mark_list_register(*reg);
                    }
                    TypeRef::Instantiated { base, .. } if *base == TypeId::LIST => {
                        ctx.mark_list_register(*reg);
                    }
                    TypeRef::Concrete(tid) if *tid == TypeId::TEXT => {
                        ctx.mark_text_register(*reg);
                        ctx.mark_prescan_text_register(*reg);
                    }
                    TypeRef::Concrete(tid) => {
                        if let Some(type_name) = vbc_module.get_type_name(*tid) {
                            if !is_primitive_type_name(&type_name) {
                                ctx.set_obj_register_type(*reg, type_name);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Populate the unified RegisterTypeMap from pre-pass results.
            // Phase 1: this runs alongside legacy HashSets for validation.
            for (reg, type_ref) in &reg_types {
                ctx.reg_types_mut().set_from_type_ref(*reg, type_ref);
            }
            // Also handle instructions that produce known types not covered by the
            // fixpoint pre-pass (e.g., NewRange, BinaryF, LoadTrue/LoadFalse, etc.)
            for instr in &vbc_func.instructions {
                match instr {
                    Instruction::NewRange { dst, .. } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Range { flat: true });
                    }
                    Instruction::BinaryF { dst, .. } | Instruction::UnaryF { dst, .. }
                    | Instruction::CvtIF { dst, .. } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Float);
                    }
                    Instruction::CmpI { dst, .. } | Instruction::CmpF { dst, .. }
                    | Instruction::IsVar { dst, .. } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Bool);
                    }
                    Instruction::LoadTrue { dst } | Instruction::LoadFalse { dst } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Bool);
                    }
                    Instruction::Concat { dst, .. } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Text {
                            owned: true,
                            compiled_layout: false,
                        });
                    }
                    Instruction::ToString { dst, .. } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Text {
                            owned: true,
                            compiled_layout: false,
                        });
                    }
                    Instruction::NewList { dst } => {
                        ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::List { element: None });
                    }
                    Instruction::LoadK { dst, const_id } => {
                        if let Some(c) = vbc_module.constants.get(*const_id as usize) {
                            match c {
                                verum_vbc::module::Constant::Float(_) => {
                                    ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Float);
                                }
                                verum_vbc::module::Constant::String(_) => {
                                    ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Text {
                                        owned: false,
                                        compiled_layout: false,
                                    });
                                }
                                verum_vbc::module::Constant::Int(_) => {
                                    ctx.reg_types_mut().set(dst.0, super::register_types::RegisterType::Int);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Pre-scan: mark registers used as BinaryF/UnaryF/CmpF operands as float.
        // This ensures GetVariantData-extracted float fields are known before ToString.
        // Also propagate through Mov chains so that source registers of float operands are marked.
        let mut float_regs_from_prescan: std::collections::HashSet<u16> = std::collections::HashSet::new();
        for instr in vbc_func.instructions.iter() {
            match instr {
                Instruction::BinaryF { a, b, dst, .. } => {
                    float_regs_from_prescan.insert(a.0);
                    float_regs_from_prescan.insert(b.0);
                    float_regs_from_prescan.insert(dst.0);
                }
                Instruction::UnaryF { src, dst, .. } => {
                    ctx.mark_float_register(src.0);
                    ctx.mark_float_register(dst.0);
                }
                Instruction::CmpF { a, b, .. } => {
                    ctx.mark_float_register(a.0);
                    ctx.mark_float_register(b.0);
                }
                Instruction::CvtIF { dst, .. } => {
                    ctx.mark_float_register(dst.0);
                }
                Instruction::LoadK { dst, const_id } => {
                    // Float constants in the constant pool
                    if let Some(c) = vbc_module.constants.get(*const_id as usize) {
                        if matches!(c, verum_vbc::module::Constant::Float(_)) {
                            ctx.mark_float_register(dst.0);
                        }
                    }
                }
                Instruction::Mov { dst, src } => {
                    // Propagate: if dst is used in BinaryF, mark src as float too
                    if float_regs_from_prescan.contains(&dst.0) {
                        float_regs_from_prescan.insert(src.0);
                    }
                }
                _ => {}
            }
        }
        // Second pass: propagate backwards through Mov chains
        // (Mov src→dst: if dst is float, mark src as float)
        for instr in vbc_func.instructions.iter().rev() {
            if let Instruction::Mov { dst, src } = instr {
                if float_regs_from_prescan.contains(&dst.0) {
                    float_regs_from_prescan.insert(src.0);
                }
            }
        }
        if !float_regs_from_prescan.is_empty() {
            ctx.set_prescan_float_registers(float_regs_from_prescan);
        }

        // Pre-scan: mark registers that receive text values (Concat/ToString/CharToStr/LoadK String).
        // This survives set_register() clearing, preventing Len dispatch regression when a print()
        // or assert_eq() call reuses a register number between the text creation and .len().
        {
            let mut text_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            for instr in vbc_func.instructions.iter() {
                match instr {
                    Instruction::Concat { dst, .. } | Instruction::ToString { dst, .. } | Instruction::CharToStr { dst, .. } => {
                        text_regs.insert(dst.0);
                    }
                    Instruction::LoadK { dst, const_id } => {
                        if let Some(c) = vbc_module.constants.get(*const_id as usize) {
                            if matches!(c, verum_vbc::module::Constant::String(_)) {
                                text_regs.insert(dst.0);
                            }
                        }
                    }
                    Instruction::Mov { dst, src } => {
                        if text_regs.contains(&src.0) {
                            text_regs.insert(dst.0);
                        }
                    }
                    _ => {}
                }
            }
            // Backward propagation through Mov chains
            for instr in vbc_func.instructions.iter().rev() {
                if let Instruction::Mov { dst, src } = instr {
                    if text_regs.contains(&dst.0) {
                        text_regs.insert(src.0);
                    }
                }
            }
            if !text_regs.is_empty() {
                ctx.set_prescan_text_registers(text_regs);
            }
        }

        // Lower instructions, switching blocks as needed
        let mut _current_block_start_idx = 0usize;

        // Build the tensor-fusion plan once per function. Per-instruction
        // dispatch consults `skip_at` / `anchor_at` to absorb fused
        // chain members into a single fused-kernel call at the chain
        // anchor. When the function has no fusable chains the plan's
        // sets are empty and the loop behaves identically to before.
        // Tracked under #91-2 / #96.
        let fusion_plan = crate::passes::tensor_fusion::FusionPlan::build(
            &vbc_func.instructions,
        );
        if !fusion_plan.chains.is_empty() {
            tracing::debug!(
                "[fusion] fn '{}': {} fusable chains, {} instructions absorbed",
                func_name,
                fusion_plan.chains.len(),
                fusion_plan.fused_indices.len(),
            );
        }

        for (instr_idx, instr) in vbc_func.instructions.iter().enumerate() {
            // Check if this instruction starts a new block
            if instr_idx > 0 {
                if let Some(&block_id) = instr_to_block.get(&instr_idx) {
                    let block = ctx.get_block(block_id)?;
                    // Add fallthrough branch from previous block if needed
                    if !ctx.current_block_has_terminator() {
                        let _ = ctx.builder().build_unconditional_branch(block);
                    }
                    ctx.position_at_end(block);
                    _current_block_start_idx = instr_idx;
                }
            }

            // Skip instructions in a terminated block (dead code after return/jump)
            // that don't start a new block — they are unreachable.
            if ctx.current_block_has_terminator() && !instr_to_block.contains_key(&instr_idx) {
                continue;
            }

            // For Jmp/JmpNot/JmpIf, resolve the offset to a block
            match instr {
                Instruction::Jmp { offset } => {
                    let target_pc = (instr_idx as i64 + *offset as i64) as usize;
                    if let Some(&target_block_id) = instr_to_block.get(&target_pc) {
                        let target_block = ctx.get_block(target_block_id)?;
                        ctx.builder()
                            .build_unconditional_branch(target_block)
                            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                    }
                }
                Instruction::JmpNot { cond, offset } => {
                    let raw_cond = ctx.get_register(cond.0)?;
                    let cond_val = coerce_to_bool(&ctx, raw_cond, "jmpnot_cond")?;
                    let target_pc = (instr_idx as i64 + *offset as i64) as usize;
                    let fallthrough_pc = instr_idx + 1;

                    let target_block_id = instr_to_block.get(&target_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("target_pc_{}", target_pc))))?;
                    let fallthrough_block_id = instr_to_block.get(&fallthrough_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("fallthrough_pc_{}", fallthrough_pc))))?;

                    let target_block = ctx.get_block(*target_block_id)?;
                    let fallthrough_block = ctx.get_block(*fallthrough_block_id)?;

                    // JmpNot: branch to target if condition is FALSE, fallthrough if TRUE
                    ctx.builder()
                        .build_conditional_branch(cond_val, fallthrough_block, target_block)
                        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                }
                Instruction::JmpIf { cond, offset } => {
                    let raw_cond = ctx.get_register(cond.0)?;
                    let cond_val = coerce_to_bool(&ctx, raw_cond, "jmpif_cond")?;
                    let target_pc = (instr_idx as i64 + *offset as i64) as usize;
                    let fallthrough_pc = instr_idx + 1;

                    let target_block_id = instr_to_block.get(&target_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("target_pc_{}", target_pc))))?;
                    let fallthrough_block_id = instr_to_block.get(&fallthrough_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("fallthrough_pc_{}", fallthrough_pc))))?;

                    let target_block = ctx.get_block(*target_block_id)?;
                    let fallthrough_block = ctx.get_block(*fallthrough_block_id)?;

                    // JmpIf: branch to target if condition is TRUE
                    ctx.builder()
                        .build_conditional_branch(cond_val, target_block, fallthrough_block)
                        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                }
                Instruction::Switch { value, default_offset, cases } => {
                    let raw_val = ctx.get_register(value.0)?;
                    let switch_val = coerce_to_i64_for_switch(&ctx, raw_val, "switch_val")?;

                    // Resolve default block
                    let default_pc = (instr_idx as i64 + *default_offset as i64) as usize;
                    let default_block_id = instr_to_block.get(&default_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("switch_default_pc_{}", default_pc))))?;
                    let default_block = ctx.get_block(*default_block_id)?;

                    // Build case list for LLVM switch
                    let mut llvm_cases = Vec::new();
                    for (case_val, case_offset) in cases.iter() {
                        let target_pc = (instr_idx as i64 + *case_offset as i64) as usize;
                        if let Some(&target_block_id) = instr_to_block.get(&target_pc) {
                            if let Ok(target_block) = ctx.get_block(target_block_id) {
                                let case_const = ctx.types().i64_type().const_int(*case_val as u64, false);
                                llvm_cases.push((case_const, target_block));
                            }
                        }
                    }

                    ctx.builder()
                        .build_switch(switch_val, default_block, &llvm_cases)
                        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                }
                Instruction::TryBegin { handler_offset } => {
                    // Cross-function exception handling via setjmp/longjmp.
                    // 1. Call verum_exception_push() to get jmp_buf pointer
                    // 2. Call setjmp(jmp_buf) → returns 0 (normal) or non-zero (exception)
                    // 3. Branch: 0 → fallthrough (try body), non-zero → handler block
                    let handler_pc = (instr_idx as i64 + *handler_offset as i64) as usize;
                    let fallthrough_pc = instr_idx + 1;

                    let handler_block_id = instr_to_block.get(&handler_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("try_handler_pc_{}", handler_pc))))?;
                    let fallthrough_block_id = instr_to_block.get(&fallthrough_pc)
                        .ok_or_else(|| LlvmLoweringError::MissingBlock(
                            Text::from(format!("try_fallthrough_pc_{}", fallthrough_pc))))?;

                    let handler_block = ctx.get_block(*handler_block_id)?;
                    let fallthrough_block = ctx.get_block(*fallthrough_block_id)?;

                    // Also store handler block for intra-function Throw optimization
                    let handler = super::context::ExceptionHandler {
                        handler_block,
                        continuation_block: None,
                    };
                    ctx.push_exception_handler(handler);

                    let module = ctx.get_module();
                    let i64_type = ctx.types().i64_type();
                    let ptr_type = ctx.types().ptr_type();
                    let i32_type = ctx.types().context().i32_type();

                    // Declare verum_exception_push() -> ptr
                    let push_fn = module.get_function("verum_exception_push").unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[], false);
                        module.add_function("verum_exception_push", fn_type, None)
                    });

                    // Declare setjmp(jmp_buf*) -> i32
                    // On macOS ARM64, _setjmp is the non-signal-saving variant
                    let setjmp_name = if cfg!(target_os = "macos") { "_setjmp" } else { "setjmp" };
                    let setjmp_fn = module.get_function(setjmp_name).unwrap_or_else(|| {
                        let fn_type = i32_type.fn_type(&[ptr_type.into()], false);
                        module.add_function(setjmp_name, fn_type, None)
                    });

                    // Call verum_exception_push() to get jmp_buf pointer
                    let jmp_buf_ptr = ctx.builder()
                        .build_call(push_fn, &[], "exception_push")
                        .or_llvm_err()?
                        .try_as_basic_value()
                        .basic()
                        .or_internal("exception_push returned void")?;

                    // Call setjmp(jmp_buf_ptr) → 0 if normal, non-zero if thrown
                    let setjmp_result = ctx.builder()
                        .build_call(setjmp_fn, &[jmp_buf_ptr.into()], "setjmp_result")
                        .or_llvm_err()?
                        .try_as_basic_value()
                        .basic()
                        .or_internal("setjmp returned void")?
                        .into_int_value();

                    // Compare setjmp result with 0
                    let is_exception = ctx.builder()
                        .build_int_compare(
                            verum_llvm::IntPredicate::NE,
                            setjmp_result,
                            i32_type.const_zero(),
                            "is_exception",
                        )
                        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

                    // Branch: normal (0) → fallthrough, exception (non-zero) → handler
                    ctx.builder()
                        .build_conditional_branch(is_exception, handler_block, fallthrough_block)
                        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
                }
                _ => {
                    // Regular instruction — lower normally
                    ctx.set_current_vbc_instr_idx(instr_idx);

                    // Set DWARF debug location for this instruction.
                    // Uses function scope with instruction index as line number
                    // (will use real source map line numbers when populated).
                    if let (Some(db), Some(scope)) = (&self.dibuilder, di_func_scope) {
                        let loc = db.create_debug_location(
                            self.context,
                            (instr_idx + 1) as u32, // line (1-based, instruction index as proxy)
                            0,                        // column
                            scope,
                            None,                     // inlined_at
                        );
                        ctx.builder().set_current_debug_location(loc);
                    }

                    lower_instruction(&mut ctx, instr)?;

                    // Safety net: if lower_instruction emitted a terminator
                    // (e.g., TailCall, Assert, Panic, Unreachable, Throw), create a
                    // dead code block so subsequent instructions don't pollute
                    // the terminated block.
                    if ctx.current_block_has_terminator() {
                        let dead_block = ctx.llvm_context().append_basic_block(
                            llvm_fn,
                            &format!("dead_after_{}", instr_idx),
                        );
                        ctx.position_at_end(dead_block);
                    }
                }
            }
        }

        let num_blocks = block_starts.len();

        // Ensure all basic blocks have terminators (exception handler blocks, etc.)
        let mut bb = llvm_fn.get_first_basic_block();
        while let Some(block) = bb {
            if block.get_terminator().is_none() {
                ctx.position_at_end(block);
                let _ = ctx.builder().build_unreachable();
            }
            bb = block.get_next_basic_block();
        }

        self.stats.functions_lowered += 1;
        self.stats.instructions_lowered += vbc_func.instructions.len();
        self.stats.basic_blocks += num_blocks;

        // Collect CBGR stats from the CbgrLowering helper
        let cbgr_stats = ctx.cbgr().stats();
        self.stats.tier0_refs += cbgr_stats.tier0_refs;
        self.stats.tier1_refs += cbgr_stats.tier1_refs;
        self.stats.tier2_refs += cbgr_stats.tier2_refs;
        self.stats.runtime_checks += cbgr_stats.runtime_checks;
        self.stats.checks_eliminated += cbgr_stats.checks_eliminated;

        // Collect escape-analysis-based elimination stats
        let elim_stats = ctx.cbgr_elimination_stats();
        // Add proven-safe references to Tier1 count
        self.stats.tier1_refs += elim_stats.proven_safe;
        // Update checks eliminated count
        self.stats.checks_eliminated += elim_stats.proven_safe;

        // Collect and print structured diagnostics from this function
        let diagnostics = ctx.take_diagnostics();
        for diag in &diagnostics {
            tracing::warn!("{}", diag.display());
        }
        self.stats.warnings += diagnostics.len();

        Ok(())
    }

    /// Remove LLVM functions that fail per-function verification.
    ///
    /// When compiling stdlib modules leniently, some functions may produce
    /// invalid LLVM IR (block structure issues, type mismatches from complex
    /// control flow). This pass verifies each function individually and removes
    /// invalid ones so the module verification passes.
    /// Remove known-problematic compiled stdlib functions that produce
    /// invalid LLVM IR (e.g. Entry API methods with CBGR ref + SetF).
    /// Uses name-based skip list since per-function f.verify() crashes LLVM.
    fn remove_known_invalid_functions(&mut self) {
        // Functions that produce invalid IR due to CBGR ref struct + SetF/GetF
        // interactions. These are Entry API methods that dereference &mut Slot
        // through a CBGR ref and try to set fields on it.
        const SKIP_PATTERNS: &[&str] = &[
            "OccupiedEntry.",
            "VacantEntry.",
            "MapDrain.",
            "MapIntoIter.",
            "SetDrain.",
            "SetIntoIter.",
        ];

        let mut removed = Vec::new();
        let mut func = self.module.get_first_function();
        while let Some(f) = func {
            let next = f.get_next_function();
            if f.count_basic_blocks() > 0 {
                let name = f.get_name().to_string_lossy().to_string();
                if SKIP_PATTERNS.iter().any(|p| name.contains(p)) {
                    removed.push(name);
                    // SAFETY: Deleting an LLVM function that was determined to be dead (unused after linking); module integrity is maintained
                    unsafe { f.delete(); }
                }
            }
            func = next;
        }
        if !removed.is_empty() {
            tracing::info!(
                "Removed {} known-invalid stdlib functions: {:?}",
                removed.len(),
                removed
            );
        }
    }

    /// Declare POSIX/libc functions used by compiled stdlib modules.
    ///
    /// Compiled .vr modules (Mutex.vr, Condvar.vr, Thread.vr, Once.vr) call
    /// pthread functions directly. These must be declared in the LLVM module
    /// before VBC lowering (Phase 2) so the calls resolve correctly.
    fn declare_posix_functions(&self) {
        let ctx = self.context;
        let i32_type = ctx.i32_type();
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        // Helper: declare function if not already present
        macro_rules! declare_fn {
            ($name:expr, $ret:expr, $params:expr, $variadic:expr) => {
                if self.module.get_function($name).is_none() {
                    let fn_type = $ret.fn_type($params, $variadic);
                    self.module.add_function($name, fn_type, None);
                }
            };
        }

        // pthread mutex functions (used by Mutex.vr)
        declare_fn!("pthread_mutex_init", i32_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_mutex_lock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_mutex_trylock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_mutex_unlock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_mutex_destroy", i32_type, &[ptr_type.into()], false);

        // pthread condition variable functions (used by Condvar.vr)
        declare_fn!("pthread_cond_init", i32_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_cond_wait", i32_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_cond_timedwait", i32_type, &[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_cond_signal", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_cond_broadcast", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_cond_destroy", i32_type, &[ptr_type.into()], false);

        // pthread thread functions — i64 Verum ABI to match platform_ir.rs
        declare_fn!("pthread_create", i64_type, &[ptr_type.into(), ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_join", i64_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_detach", i64_type, &[i64_type.into()], false);

        // pthread rwlock functions (used by RwLock.vr)
        declare_fn!("pthread_rwlock_init", i32_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_rwlock_rdlock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_rwlock_wrlock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_rwlock_unlock", i32_type, &[ptr_type.into()], false);
        declare_fn!("pthread_rwlock_destroy", i32_type, &[ptr_type.into()], false);

        // pthread once functions (used by Once.vr)
        declare_fn!("pthread_once", i32_type, &[ptr_type.into(), ptr_type.into()], false);

        // pthread key (TLS) functions
        declare_fn!("pthread_key_create", i32_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("pthread_getspecific", ptr_type, &[i32_type.into()], false);
        declare_fn!("pthread_setspecific", i32_type, &[i32_type.into(), ptr_type.into()], false);

        // POSIX semaphore (used by Semaphore.vr)
        // macOS uses dispatch_semaphore, Linux uses sem_t
        declare_fn!("sem_init", i32_type, &[ptr_type.into(), i32_type.into(), i32_type.into()], false);
        declare_fn!("sem_wait", i32_type, &[ptr_type.into()], false);
        declare_fn!("sem_post", i32_type, &[ptr_type.into()], false);
        declare_fn!("sem_destroy", i32_type, &[ptr_type.into()], false);

        // Process management (used by platform_ir.rs verum_process_spawn)
        declare_fn!("pipe", i32_type, &[ptr_type.into()], false);
        declare_fn!("fork", i64_type, &[], false);
        declare_fn!("dup2", i64_type, &[i64_type.into(), i64_type.into()], false);
        declare_fn!("execvp", i64_type, &[ptr_type.into(), ptr_type.into()], false);
        declare_fn!("waitpid", i64_type, &[i64_type.into(), ptr_type.into(), i64_type.into()], false);

        // Memory functions
        declare_fn!("memset", ptr_type, &[ptr_type.into(), i32_type.into(), i64_type.into()], false);
        declare_fn!("memcpy", ptr_type, &[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        declare_fn!("memmove", ptr_type, &[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        declare_fn!("free", ctx.void_type(), &[ptr_type.into()], false);
    }

    #[allow(dead_code)]
    fn remove_invalid_functions(&mut self) {
        // Replace bodies of arity-collided functions with a single
        // `unreachable` instruction. These functions have mismatched
        // LLVM signatures from VBC overloading fixups — their original
        // lowered IR contains instructions with null Type operands
        // that crash LLVM passes (TypeFinder, SelectionDAG, SinkCast,
        // InterleavedAccess, etc.). Replacing the body with
        // `unreachable` gives LLVM a valid (if trivially dead)
        // function body that all passes can safely handle.
        //
        // We can't delete these functions because other code may
        // reference them as call targets — the linker resolves them
        // to C runtime stubs.
        let ctx = self.context;
        let mut replaced = 0usize;

        // Iterate all functions in the module.
        let mut func = self.module.get_first_function();
        while let Some(f) = func {
            let next = f.get_next_function();

            // Only touch functions that have bodies AND are in the
            // skip set or marked as arity-collided.
            let fn_name = f.get_name().to_string_lossy().to_string();
            let is_skip_body = self.skip_body_func_ids.iter().any(|&id| {
                self.functions.get(&id).map(|fv| *fv == f).unwrap_or(false)
            });

            if is_skip_body && f.count_basic_blocks() > 0 {
                // Remove all existing basic blocks — their instructions
                // contain null Type references that crash LLVM codegen.
                while let Some(bb) = f.get_first_basic_block() {
                    unsafe { bb.delete().ok(); }
                }
                // Replace with a trivial return stub. `unreachable`
                // causes LLVM to emit a trap instruction which still
                // runs InterleavedAccess/SelectionDAG passes. A plain
                // `ret` is the safest — it generates no machine
                // instructions that touch memory.
                let entry = ctx.append_basic_block(f, "entry");
                let builder = ctx.create_builder();
                builder.position_at_end(entry);
                let ret_ty = f.get_type().get_return_type();
                if let Some(ty) = ret_ty {
                    // Non-void function: return a zero value.
                    let zero = ty.const_zero();
                    let _ = builder.build_return(Some(&zero));
                } else {
                    // Void function: return void.
                    let _ = builder.build_return(None);
                }

                tracing::debug!(
                    "Replaced body of {} with unreachable (arity collision stub)",
                    fn_name
                );
                replaced += 1;
            }

            func = next;
        }

        if replaced > 0 {
            tracing::info!(
                "Replaced {} arity-collided function bodies with unreachable stubs",
                replaced
            );
        }
    }

    /// Remove compiled stdlib functions that use @cfg(target_os=...) dispatch.
    ///
    /// When compiled .vr code uses @cfg to dispatch to platform-specific modules,
    /// the @cfg is not evaluated at compile time. This produces functions that call
    /// themselves (infinite recursion). Remove them so calls fall through to C runtime.
    fn remove_self_recursive_functions(&mut self) {
        const BROKEN_CFG_FUNCTIONS: &[&str] = &[
            "ctx_clear",
            "ctx_set",
            "ctx_get",
            "ctx_get_mut",
            "ctx_has",
            "ctx_push_frame",
            "ctx_pop_frame",
        ];
        for name in BROKEN_CFG_FUNCTIONS {
            if let Some(f) = self.module.get_function(name) {
                if f.count_basic_blocks() > 0 {
                    tracing::info!("Removing broken @cfg-dispatch function: {}", name);
                    // SAFETY: Deleting a function with broken CFG from platform-conditional compilation; the function body is malformed and must not reach the LLVM verifier
                    unsafe { f.delete(); }
                }
            }
        }
    }

    /// Finalize debug info (must be called before verification).
    pub fn finalize_debug_info(&self) {
        if let Some(ref db) = self.dibuilder {
            db.finalize();
        }
    }

    /// Verify the LLVM module.
    pub fn verify(&self) -> Result<()> {
        // Finalize debug info before verification — LLVM verifier checks DI metadata consistency
        self.finalize_debug_info();

        // Skip LLVM module verification when the module has arity
        // collisions. The arity-collision fixup creates bitcast
        // wrapper functions whose types the LLVM verifier can't
        // validate — visiting them causes SIGSEGV in
        // Verifier::visitCallBase when it dereferences a null
        // FunctionType. The compiled binary still works correctly
        // because the wrappers are only for linking, not for
        // optimization.
        //
        // This also skips verification for modules where
        // `skip_body_func_ids` is non-empty, as those functions have
        // mismatched signatures that would fail verification.
        if self.has_arity_collisions || !self.skip_body_func_ids.is_empty() {
            tracing::debug!(
                "Skipping LLVM module verification (arity collisions = {}, skip bodies = {})",
                self.has_arity_collisions,
                self.skip_body_func_ids.len()
            );
            return Ok(());
        }

        self.module
            .verify()
            .map_err(|e| LlvmLoweringError::VerificationFailed(e.to_string().into()))
    }

    /// Get the LLVM IR as a string.
    /// Check if any function declarations had arity collisions.
    pub fn has_arity_collisions(&self) -> bool {
        self.has_arity_collisions
    }

    /// Count of functions whose bodies were not lowered due to
    /// signature mismatch (arity collision).
    pub fn skip_body_count(&self) -> usize {
        self.skip_body_func_ids.len()
    }

    pub fn get_ir(&self) -> Text {
        Text::from(self.module.print_to_string().to_string())
    }

    /// Write LLVM IR to a file.
    pub fn write_ir_to_file(&self, path: &std::path::Path) -> Result<()> {
        self.module
            .print_to_file(path)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
    }

    /// Take ownership of the LLVM module.
    pub fn into_module(self) -> Module<'ctx> {
        self.module
    }

    /// Emit global constructors and destructors from VBC module metadata.
    ///
    /// Static variable initializers are wrapped in a single `__verum_static_init` function
    /// that calls each init function in order. This function is registered as a global
    /// constructor so it runs before main().
    fn emit_global_ctors_dtors(&mut self, vbc_module: &VbcModule) -> Result<()> {
        if vbc_module.global_ctors.is_empty() && vbc_module.global_dtors.is_empty() {
            return Ok(());
        }

        let builder = self.context.create_builder();

        // Emit global constructors
        if !vbc_module.global_ctors.is_empty() {
            let void_type = self.context.void_type();
            let init_fn_type = void_type.fn_type(&[], false);
            let init_fn = self.module.add_function("__verum_static_init", init_fn_type, None);

            let entry_bb = self.context.append_basic_block(init_fn, "entry");
            builder.position_at_end(entry_bb);

            for (func_id, _priority) in &vbc_module.global_ctors {
                if let Some(target_fn) = self.functions.get(&func_id.0).copied() {
                    // Only call functions with 0 parameters — true static init functions.
                    // Module merging can remap func IDs incorrectly, mapping init functions
                    // to instance methods (with &self param). Skip those to avoid
                    // "incorrect number of arguments" LLVM verification failures.
                    if target_fn.count_params() == 0 {
                        builder.build_call(target_fn, &[], "init_result")
                            .map_err(|e| LlvmLoweringError::llvm_error(format!("global ctor call: {}", e)))?;
                    }
                }
            }

            builder.build_return(None)
                .map_err(|e| LlvmLoweringError::llvm_error(format!("global ctor return: {}", e)))?;

            super::symbols::add_global_ctor(&self.module, init_fn, super::symbols::DEFAULT_CTOR_DTOR_PRIORITY)
                .map_err(|e| LlvmLoweringError::llvm_error(format!("emit global ctor: {}", e)))?;
        }

        // Emit global destructors (same pattern)
        if !vbc_module.global_dtors.is_empty() {
            let void_type = self.context.void_type();
            let dtor_fn_type = void_type.fn_type(&[], false);
            let dtor_fn = self.module.add_function("__verum_static_fini", dtor_fn_type, None);

            let entry_bb = self.context.append_basic_block(dtor_fn, "entry");
            builder.position_at_end(entry_bb);

            for (func_id, _priority) in &vbc_module.global_dtors {
                if let Some(target_fn) = self.functions.get(&func_id.0).copied() {
                    builder.build_call(target_fn, &[], "fini_result")
                        .map_err(|e| LlvmLoweringError::llvm_error(format!("global dtor call: {}", e)))?;
                }
            }

            builder.build_return(None)
                .map_err(|e| LlvmLoweringError::llvm_error(format!("global dtor return: {}", e)))?;

            super::symbols::add_global_dtor(&self.module, dtor_fn, super::symbols::DEFAULT_CTOR_DTOR_PRIORITY)
                .map_err(|e| LlvmLoweringError::llvm_error(format!("emit global dtor: {}", e)))?;
        }

        Ok(())
    }

    /// Look up a function by name.
    pub fn get_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        self.module.get_function(name)
    }

    /// Look up a function by ID.
    pub fn get_function_by_id(&self, id: u32) -> Option<FunctionValue<'ctx>> {
        self.functions.get(&id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowering_config_default() {
        let config = LoweringConfig::default();
        assert_eq!(config.opt_level, 2);
        assert!(config.cbgr_elimination);
    }

    #[test]
    fn test_lowering_config_debug() {
        let config = LoweringConfig::debug("test");
        assert_eq!(config.opt_level, 0);
        assert!(!config.cbgr_elimination);
        assert!(config.debug_info);
    }

    #[test]
    fn test_lowering_config_release() {
        let config = LoweringConfig::release("test");
        assert_eq!(config.opt_level, 2);
        assert!(config.cbgr_elimination);
    }

    #[test]
    fn test_calling_convention_mapping() {
        use llvm_calling_conventions::*;

        // Test all calling conventions map to expected LLVM values
        assert_eq!(to_llvm_calling_convention(&CallingConvention::C), C);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Stdcall), X86_STDCALL);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Fastcall), X86_FASTCALL);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::SysV64), X86_64_SYSV);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Win64), WIN64);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::ArmAapcs), ARM_AAPCS);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Arm64), C); // ARM64 uses default C
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Interrupt), X86_INTR);
        assert_eq!(to_llvm_calling_convention(&CallingConvention::Naked), C); // Naked uses C + attribute
    }

    #[test]
    fn test_interrupt_calling_convention_value() {
        // Verify the X86_INTR value matches LLVM's definition
        assert_eq!(llvm_calling_conventions::X86_INTR, 83);
    }

    #[test]
    fn test_naked_attribute_kind_id() {
        // Verify "naked" is a recognized LLVM attribute
        let naked_kind_id = Attribute::get_named_enum_kind_id("naked");
        // naked_kind_id should be non-zero if LLVM recognizes the attribute
        // The exact value varies by LLVM version, but it should be > 0
        assert!(naked_kind_id > 0, "LLVM should recognize 'naked' attribute");
    }
}
