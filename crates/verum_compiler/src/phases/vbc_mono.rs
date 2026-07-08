//! VBC Monomorphization Phase
//!

//! This phase specializes generic VBC functions with concrete type arguments.
//! It is Phase 6 in the VBC-first pipeline.
//!

//! # Architecture
//!

//! ```text
//! VBC Module (with generics)
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │ MONOMORPHIZATION PIPELINE │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │ │
//! │ ┌─────────────────────────────────────────────────────────────────────┐ │
//! │ │ INSTANTIATION GRAPH │ │
//! │ │ (from type checking) │ │
//! │ │ │ │
//! │ │ [(List.new, [Int]), (List.push, [Int]), (List.new, [MyStruct]), ...]│ │
//! │ └─────────────────────────────────────────────────────────────────────┘ │
//! │ │ │
//! │ ▼ │
//! │ ┌─────────────────────────────────────────────────────────────────────┐ │
//! │ │ RESOLUTION PHASE │ │
//! │ │ │ │
//! │ │ For each (fn_id, type_args): │ │
//! │ │ 1. Check stdlib precompiled → FOUND → use │ │
//! │ │ 2. Check persistent cache → VALID → use │ │
//! │ │ 3. MISS → schedule for specialization │ │
//! │ └─────────────────────────────────────────────────────────────────────┘ │
//! │ │ │
//! │ ▼ │
//! │ ┌─────────────────────────────────────────────────────────────────────┐ │
//! │ │ SPECIALIZATION PHASE │ │
//! │ │ │ │
//! │ │ For each unresolved (fn_id, type_args): │ │
//! │ │ 1. Load generic VBC │ │
//! │ │ 2. Apply type substitution │ │
//! │ │ 3. Optimize specialized VBC │ │
//! │ │ 4. Cache result │ │
//! │ └─────────────────────────────────────────────────────────────────────┘ │
//! │ │ │
//! │ ▼ │
//! │ ┌─────────────────────────────────────────────────────────────────────┐ │
//! │ │ MERGE PHASE │ │
//! │ │ │ │
//! │ │ Combine: │ │
//! │ │ - User module VBC │ │
//! │ │ - Stdlib precompiled specializations │ │
//! │ │ - Newly monomorphized functions │ │
//! │ │ → Final monomorphized VBC module │ │
//! │ └─────────────────────────────────────────────────────────────────────┘ │
//! │ │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!

//! # Implementation
//!

//! This phase delegates to the industrial-grade monomorphization implementation
//! in `verum_vbc::mono`, which provides:
//! - `InstantiationGraph` - dependency tracking with topological ordering
//! - `MonomorphizationResolver` - three-level resolution (core/cache/pending)
//! - `BytecodeSpecializer` - full opcode coverage with type substitution
//! - `SpecializationOptimizer` - constant folding, DCE, peephole optimization
//! - `ModuleMerger` - final module assembly
//!

//! VBC monomorphization: specializes generic functions for concrete type
//! arguments, producing type-specific VBC bytecode for efficient execution.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput, VbcModuleData};
use verum_common::{List, Text};
use verum_diagnostics::Diagnostic;
use verum_vbc::module::{FunctionId, VbcModule};
use verum_vbc::mono::{
    InstantiationGraph, MonoPhaseConfig, MonomorphizationPhase as VbcMonoPhase, SourceLocation,
};
use verum_vbc::types::TypeRef;

// ============================================================================
// Monomorphization Phase
// ============================================================================

/// VBC monomorphization phase.
///

/// Specializes generic VBC functions with concrete type arguments.
/// This is a required step before interpretation or further lowering to MLIR.
///

/// Uses the industrial-grade implementation from `verum_vbc::mono`.
pub struct VbcMonomorphizationPhase {
    /// Monomorphization cache directory.
    cache_dir: Option<PathBuf>,
    /// Enable persistent caching.
    enable_cache: bool,
    /// Enable parallel specialization.
    enable_parallel: bool,
    /// Enable post-specialization optimization.
    enable_optimize: bool,
    /// Stdlib module for precompiled specializations.
    stdlib: Option<Arc<VbcModule>>,
    /// Performance metrics.
    metrics: PhaseMetricsData,
}

/// Internal metrics storage.
#[derive(Debug, Clone, Default)]
struct PhaseMetricsData {
    /// Time spent in monomorphization.
    duration: Duration,
    /// Number of specializations from stdlib.
    stdlib_hits: usize,
    /// Number of cache hits.
    cache_hits: usize,
    /// Number of new specializations generated.
    new_specializations: usize,
    /// Total generic functions processed.
    generic_functions: usize,
    /// Total instantiations discovered.
    total_instantiations: usize,
    /// Bytes of specialized bytecode generated.
    bytecode_generated: usize,
}

impl VbcMonomorphizationPhase {
    /// Creates a new VBC monomorphization phase with default settings.
    pub fn new() -> Self {
        Self {
            cache_dir: None,
            enable_cache: true,
            enable_parallel: true,
            enable_optimize: true,
            stdlib: None,
            metrics: PhaseMetricsData::default(),
        }
    }

    /// Sets the cache directory for persistent monomorphization cache.
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = Some(dir);
        self
    }

    /// Disables persistent caching.
    pub fn without_cache(mut self) -> Self {
        self.enable_cache = false;
        self
    }

    /// Disables parallel specialization.
    pub fn without_parallel(mut self) -> Self {
        self.enable_parallel = false;
        self
    }

    /// Disables post-specialization optimization.
    pub fn without_optimize(mut self) -> Self {
        self.enable_optimize = false;
        self
    }

    /// Sets the stdlib module for precompiled specializations.
    pub fn with_core(mut self, stdlib: Arc<VbcModule>) -> Self {
        self.stdlib = Some(stdlib);
        self
    }

    /// Monomorphize a VBC module, specializing generic functions with concrete types.
    ///

    /// This is the public entry point for the AOT compilation path.
    /// Wraps the internal `process_module()` logic with a `VbcModuleData` wrapper.
    ///

    /// Returns the monomorphized module on success, or diagnostic errors on failure.
    pub fn monomorphize(&mut self, module: &VbcModule) -> Result<VbcModule, List<Diagnostic>> {
        // Fast path: with no seeded instantiations there is nothing to
        // specialize, so skip the whole pass — including the per-function
        // `is_generic` recovery below, which walks all ~42k descriptors and is
        // pure waste when no specialization will run.  Codegen seeds
        // `module.specializations` (VBC-GENERIC-INSTANTIATION) only for
        // programs that actually instantiate a generic function needing
        // monomorphization, so the common case pays a single clone and returns.
        if module.specializations.is_empty() {
            return Ok(module.clone());
        }
        let mut cloned = module.clone();
        // VBC-GENERIC-INSTANTIATION: recover the `is_generic` flag from each
        // descriptor's parameter/return `TypeRef`s.  The codegen never sets
        // `descriptor.is_generic` (it is a dead flag defaulting to false), so
        // this pass historically counted zero generic functions and returned
        // early — generic AOT monomorphization never ran.  The param/return
        // TypeRefs DO encode `Generic(TypeParamId)` (they survive the archive
        // round-trip), so `TypeRef::is_generic()` recovers the flag exactly.
        // Doing this IN-MEMORY here — not by setting the flag in the codegen —
        // deliberately avoids regenerating the precompiled stdlib archive,
        // whose byte-layout perturbation shifts the global field-intern order
        // and trips a latent positional-field-layout non-determinism
        // (defect-class §40).  Self-consistent: the interpreter never runs
        // this pass, so nothing observes the recovered flag except the AOT
        // monomorphizer.
        let mut recovered = 0usize;
        for f in cloned.functions.iter_mut() {
            if !f.is_generic
                && (f.params.iter().any(|p| p.type_ref.is_generic())
                    || f.return_type.is_generic())
            {
                f.is_generic = true;
                recovered += 1;
            }
        }
        if recovered > 0 {
            tracing::debug!(
                "VBC monomorphization: recovered is_generic on {} descriptor(s) from TypeRefs",
                recovered
            );
        }
        // Dead-invariant guard (defect-class: consulted-but-never-set). This
        // whole pass keys on `is_generic`; if a stdlib-sized module ends up with
        // ZERO generic functions, the recovery itself has regressed (the
        // TypeRefs no longer carry `Generic`, or the flag was consulted before
        // recovery) — the exact silent failure that kept generic AOT
        // monomorphization dead for the project's entire history. Warn so it
        // cannot recur unnoticed: any module that links the stdlib has generic
        // functions, so 0 is implausible.
        let total_generic = cloned.functions.iter().filter(|f| f.is_generic).count();
        if total_generic == 0 && cloned.functions.len() > 1000 {
            tracing::warn!(
                "VBC monomorphization: 0 generic functions across {} descriptors — the \
                 is_generic recovery has likely REGRESSED (a stdlib-linked module must have \
                 generics). generic-AOT-monomorphization is silently disabled; see the \
                 dead-invariant audit + defect-class consulted-but-never-set.",
                cloned.functions.len(),
            );
        }
        let module_data = VbcModuleData {
            module: cloned,
            tier_stats: super::VbcTierStats {
                tier0_refs: 0,
                tier1_refs: 0,
                tier2_refs: 0,
                promotion_rate: 0.0,
            },
        };
        let result = self.process_module(&module_data)?;
        Ok(result.module)
    }

    /// Process a single VBC module.
    fn process_module(
        &mut self,
        module_data: &VbcModuleData,
    ) -> Result<VbcModuleData, List<Diagnostic>> {
        let module = &module_data.module;

        // Count generic functions
        let generic_count = module.functions.iter().filter(|f| f.is_generic).count();
        self.metrics.generic_functions = generic_count;

        if std::env::var_os("VERUM_TRACE_MONO").is_some() {
            let sample: Vec<String> = module
                .functions
                .iter()
                .filter(|f| f.is_generic)
                .filter_map(|f| module.strings.get(f.name).map(|s| s.to_string()))
                .filter(|n| n.contains("poll") || n.contains("future"))
                .take(6)
                .collect();
            eprintln!(
                "[mono-pass] total_fns={} generic_fns={} poll/future-generics={:?}",
                module.functions.len(),
                generic_count,
                sample
            );
        }

        // Skip the (expensive) instantiation-graph scan unless there is
        // actual monomorphization work.  `build_instantiation_graph` walks the
        // bytecode of EVERY function in the module — including the whole
        // precompiled stdlib (~42k functions) — so running it per compilation
        // when there is nothing to specialize is a severe, pure-waste perf
        // regression.  Instantiations are seeded into `module.specializations`
        // by the codegen (VBC-GENERIC-INSTANTIATION); with no seeds there is
        // no work, so short-circuit.  (Historically `is_generic` was never set
        // so `generic_count` was always 0 and this pass was always a no-op —
        // gating on the seed set preserves that fast path exactly while
        // enabling real specialization when the codegen records an
        // instantiation.)
        if generic_count == 0 || module.specializations.is_empty() {
            tracing::debug!(
                "VBC monomorphization: module '{}' — nothing to specialize \
                 ({} generic fns, {} seeded instantiations)",
                module.name,
                generic_count,
                module.specializations.len(),
            );
            return Ok(module_data.clone());
        }

        tracing::debug!(
            "VBC monomorphization: module '{}' with {} generic functions",
            module.name,
            generic_count
        );

        // Build instantiation graph from bytecode analysis
        let graph = self.build_instantiation_graph(module);
        self.metrics.total_instantiations = graph.len();

        // Create configuration
        let config = MonoPhaseConfig {
            use_stdlib: self.stdlib.is_some(),
            use_cache: self.enable_cache,
            parallel: self.enable_parallel,
            num_threads: 0, // Auto-detect
            optimize: self.enable_optimize,
            cache_dir: self.cache_dir.clone(),
        };

        // Execute monomorphization
        let mut phase = VbcMonoPhase::new(config);
        if let Some(ref stdlib) = self.stdlib {
            phase = phase.with_core(stdlib.clone());
        }

        match phase.execute(module.clone(), &graph) {
            Ok(result) => {
                // Update metrics from result
                self.metrics.stdlib_hits = result.metrics.stdlib_hits;
                self.metrics.cache_hits = result.metrics.cache_hits;
                self.metrics.new_specializations = result.metrics.new_specializations;
                self.metrics.bytecode_generated = result.metrics.bytecode_generated;

                tracing::info!(
                    "VBC monomorphization: module '{}' - {} generic fns, {} new specs, {} cache hits, {} bytes generated",
                    module.name,
                    self.metrics.generic_functions,
                    self.metrics.new_specializations,
                    self.metrics.cache_hits,
                    self.metrics.bytecode_generated
                );

                // Log warnings
                for warning in result.warnings {
                    tracing::warn!("Monomorphization warning: {}", warning);
                }

                Ok(VbcModuleData {
                    module: result.module,
                    tier_stats: module_data.tier_stats.clone(),
                })
            }
            Err(e) => {
                let diagnostic = verum_diagnostics::DiagnosticBuilder::error()
                    .code("E0801")
                    .message(format!("Monomorphization failed: {}", e))
                    .build();
                Err({
                    let mut list = List::new();
                    list.push(diagnostic);
                    list
                })
            }
        }
    }

    /// Builds the instantiation graph by analyzing bytecode.
    fn build_instantiation_graph(&self, module: &VbcModule) -> InstantiationGraph {
        let mut graph = InstantiationGraph::new();

        // Collect generic function IDs
        let generic_fns: Vec<FunctionId> = module
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.is_generic)
            .map(|(i, _)| FunctionId(i as u32))
            .collect();

        // Add instantiations from existing specialization entries
        for spec in &module.specializations {
            graph.record_instantiation(
                spec.generic_fn,
                spec.type_args.clone(),
                SourceLocation::default(),
            );
        }

        // Analyze bytecode for CALL_G instructions
        for (func_idx, func) in module.functions.iter().enumerate() {
            let start = func.bytecode_offset as usize;
            let end = start + func.bytecode_length as usize;

            if let Some(bytecode) = module.bytecode.get(start..end) {
                self.analyze_function_bytecode(
                    bytecode,
                    FunctionId(func_idx as u32),
                    &generic_fns,
                    &mut graph,
                );
            }
        }

        graph
    }

    /// Analyzes a function's bytecode for generic calls.
    fn analyze_function_bytecode(
        &self,
        bytecode: &[u8],
        _caller: FunctionId,
        generic_fns: &[FunctionId],
        graph: &mut InstantiationGraph,
    ) {
        use verum_vbc::instruction::Instruction;

        let generic_set: std::collections::HashSet<FunctionId> =
            generic_fns.iter().copied().collect();
        let mut pc = 0;

        // Decode with the CANONICAL decoder so the scan stays ALIGNED to real
        // instruction boundaries.  The previous hand-rolled
        // `skip_instruction_operands` length table was incomplete (a
        // "simplified" copy of the same class of bug as the specializer's
        // get_operand_bytes) — one wrong length desynchronised the stream and
        // made a later operand byte (e.g. a 0x80 register byte) decode as a
        // phantom CALL_G whose garbage callee happened to land in the generic
        // set, injecting instantiations for completely unrelated functions.
        while pc < bytecode.len() {
            let instr_start = pc;
            match verum_vbc::bytecode::decode_instruction(bytecode, &mut pc) {
                Ok(Instruction::CallG {
                    func_id, type_args, ..
                }) => {
                    let callee = FunctionId(func_id);
                    if generic_set.contains(&callee) && !type_args.is_empty() {
                        graph.record_instantiation(callee, type_args, SourceLocation::default());
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
            if pc <= instr_start {
                break;
            }
        }
    }

}

impl Default for VbcMonomorphizationPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for VbcMonomorphizationPhase {
    fn name(&self) -> &str {
        "VBC Monomorphization"
    }

    fn description(&self) -> &str {
        "Specializes generic VBC functions with concrete type arguments"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract VBC modules from input
        let vbc_modules = match input.data {
            PhaseData::Vbc(modules) => modules,
            _ => {
                let diagnostic = verum_diagnostics::DiagnosticBuilder::error()
                    .code("E0800")
                    .message("VBC monomorphization phase requires VBC modules as input")
                    .build();
                return Err({
                    let mut list = List::new();
                    list.push(diagnostic);
                    list
                });
            }
        };

        // Process each module
        let mut phase = Self::new();
        phase.cache_dir = self.cache_dir.clone();
        phase.enable_cache = self.enable_cache;
        phase.enable_parallel = self.enable_parallel;
        phase.enable_optimize = self.enable_optimize;
        phase.stdlib = self.stdlib.clone();

        let mut processed_modules = List::new();

        for module_data in vbc_modules.iter() {
            let processed = phase.process_module(module_data)?;
            processed_modules.push(processed);
        }

        let duration = start.elapsed();
        phase.metrics.duration = duration;

        // Log statistics
        tracing::info!(
            "VBC monomorphization complete: {} modules, {} generic fns, {} stdlib hits, {} cache hits, {} new specs",
            processed_modules.len(),
            phase.metrics.generic_functions,
            phase.metrics.stdlib_hits,
            phase.metrics.cache_hits,
            phase.metrics.new_specializations
        );

        Ok(PhaseOutput {
            data: PhaseData::Vbc(processed_modules),
            warnings: List::new(),
            metrics: phase.phase_metrics(),
        })
    }

    fn can_parallelize(&self) -> bool {
        true
    }

    fn metrics(&self) -> PhaseMetrics {
        self.phase_metrics()
    }
}

impl VbcMonomorphizationPhase {
    fn phase_metrics(&self) -> PhaseMetrics {
        let mut custom_metrics = List::new();
        custom_metrics.push((
            Text::from("stdlib_hits"),
            Text::from(self.metrics.stdlib_hits.to_string()),
        ));
        custom_metrics.push((
            Text::from("cache_hits"),
            Text::from(self.metrics.cache_hits.to_string()),
        ));
        custom_metrics.push((
            Text::from("new_specializations"),
            Text::from(self.metrics.new_specializations.to_string()),
        ));
        custom_metrics.push((
            Text::from("generic_functions"),
            Text::from(self.metrics.generic_functions.to_string()),
        ));
        custom_metrics.push((
            Text::from("total_instantiations"),
            Text::from(self.metrics.total_instantiations.to_string()),
        ));
        custom_metrics.push((
            Text::from("bytecode_generated"),
            Text::from(self.metrics.bytecode_generated.to_string()),
        ));

        PhaseMetrics {
            phase_name: Text::from("VBC Monomorphization"),
            duration: self.metrics.duration,
            items_processed: self.metrics.generic_functions,
            memory_allocated: self.metrics.bytecode_generated,
            custom_metrics,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vbc_mono_phase_creation() {
        let phase = VbcMonomorphizationPhase::new();
        assert_eq!(phase.name(), "VBC Monomorphization");
        assert!(phase.enable_cache);
        assert!(phase.enable_parallel);
        assert!(phase.enable_optimize);
    }

    #[test]
    fn test_vbc_mono_phase_without_cache() {
        let phase = VbcMonomorphizationPhase::new().without_cache();
        assert!(!phase.enable_cache);
    }

    #[test]
    fn test_vbc_mono_phase_without_parallel() {
        let phase = VbcMonomorphizationPhase::new().without_parallel();
        assert!(!phase.enable_parallel);
    }

    #[test]
    fn test_vbc_mono_phase_with_cache_dir() {
        let phase =
            VbcMonomorphizationPhase::new().with_cache_dir(PathBuf::from("/tmp/mono_cache"));
        assert_eq!(phase.cache_dir, Some(PathBuf::from("/tmp/mono_cache")));
    }

    #[test]
    fn test_vbc_mono_phase_can_parallelize() {
        let phase = VbcMonomorphizationPhase::new();
        assert!(phase.can_parallelize());
    }
}
