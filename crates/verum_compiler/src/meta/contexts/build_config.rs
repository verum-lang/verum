//! Build Configuration Sub-Context
//!
//! Manages runtime information, build assets, project metadata, and
//! staged metaprogramming configuration.
//!
//! ## Responsibility
//!
//! - Runtime/platform information (target_os, target_arch, etc.)
//! - Build assets (file loading, include_bytes, etc.)
//! - Project metadata (name, version, etc.)
//! - Stage configuration for staged metaprogramming
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::Span;
use verum_common::{List, Map, Text};

use crate::meta::{
    BenchResult, BuildAssetsInfo, MacroStateInfo, ProjectInfoData, RuntimeInfo, StageRecord,
};

/// Build configuration context
///
/// Manages build-time configuration including platform info, assets,
/// project metadata, and staged metaprogramming state.
#[derive(Debug, Clone)]
pub struct BuildConfiguration {
    /// Runtime information for MetaRuntime context
    pub runtime_info: RuntimeInfo,

    /// Build assets information for BuildAssets context
    pub build_assets: BuildAssetsInfo,

    /// Macro state information for MacroState context
    pub macro_state: MacroStateInfo,

    /// Project metadata
    pub project_info: ProjectInfoData,

    /// Current execution stage (0 = runtime, 1+ = compile-time)
    current_stage: u32,

    /// Maximum stage level in the compilation
    max_stage: u32,

    /// Function name -> stage level mapping
    function_stages: Map<Text, u32>,

    /// Whether staged metaprogramming is enabled
    staged_enabled: bool,

    /// Stage configuration key-value pairs
    stage_config: Map<Text, Text>,

    /// Chain of generation records (for tracking code provenance)
    generation_chain: List<StageRecord>,

    /// Original source span that generated current code
    generation_origin: Option<Span>,

    /// Function name that generated current code
    generation_source_function: Option<Text>,

    /// String literals in the code (literal text -> span)
    string_literals: List<(Text, Span)>,

    /// Benchmark results (name -> list of results)
    bench_results: Map<Text, List<BenchResult>>,

    /// Memory usage reports (name -> bytes)
    memory_reports: Map<Text, u64>,

    /// Counters (name -> count)
    counters: Map<Text, u64>,

    /// Current memory usage (bytes)
    memory_used: u64,

    /// Peak memory usage (bytes)
    peak_memory: u64,
}

impl Default for BuildConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildConfiguration {
    /// Create a new build configuration with defaults
    pub fn new() -> Self {
        Self {
            runtime_info: RuntimeInfo::default(),
            build_assets: BuildAssetsInfo::default(),
            macro_state: MacroStateInfo::default(),
            project_info: ProjectInfoData::default(),
            current_stage: 1, // Default to compile-time
            max_stage: 1,
            function_stages: Map::new(),
            staged_enabled: true,
            stage_config: Map::new(),
            generation_chain: List::new(),
            generation_origin: None,
            generation_source_function: None,
            string_literals: List::new(),
            bench_results: Map::new(),
            memory_reports: Map::new(),
            counters: Map::new(),
            memory_used: 0,
            peak_memory: 0,
        }
    }

    // ======== Runtime Info ========

    /// Set runtime info
    #[inline]
    pub fn set_runtime_info(&mut self, runtime_info: RuntimeInfo) {
        self.runtime_info = runtime_info;
    }

    /// Get runtime info reference
    #[inline]
    pub fn runtime_info(&self) -> &RuntimeInfo {
        &self.runtime_info
    }

    /// Get mutable runtime info
    #[inline]
    pub fn runtime_info_mut(&mut self) -> &mut RuntimeInfo {
        &mut self.runtime_info
    }

    // ======== Build Assets ========

    /// Set build assets
    #[inline]
    pub fn set_build_assets(&mut self, build_assets: BuildAssetsInfo) {
        self.build_assets = build_assets;
    }

    /// Get build assets reference
    #[inline]
    pub fn build_assets(&self) -> &BuildAssetsInfo {
        &self.build_assets
    }

    /// Get mutable build assets
    #[inline]
    pub fn build_assets_mut(&mut self) -> &mut BuildAssetsInfo {
        &mut self.build_assets
    }

    // ======== Macro State ========

    /// Set macro state
    #[inline]
    pub fn set_macro_state(&mut self, macro_state: MacroStateInfo) {
        self.macro_state = macro_state;
    }

    /// Get macro state reference
    #[inline]
    pub fn macro_state(&self) -> &MacroStateInfo {
        &self.macro_state
    }

    /// Get mutable macro state
    #[inline]
    pub fn macro_state_mut(&mut self) -> &mut MacroStateInfo {
        &mut self.macro_state
    }

    /// Enter a macro execution scope
    #[inline]
    pub fn enter_macro(&mut self, name: Text) {
        self.macro_state.enter_macro(name);
    }

    /// Exit the current macro execution scope
    #[inline]
    pub fn exit_macro(&mut self) {
        self.macro_state.exit_macro();
    }

    // ======== Project Info ========

    /// Set project info
    #[inline]
    pub fn set_project_info(&mut self, project_info: ProjectInfoData) {
        self.project_info = project_info;
    }

    /// Get project info reference
    #[inline]
    pub fn project_info(&self) -> &ProjectInfoData {
        &self.project_info
    }

    /// Get mutable project info
    #[inline]
    pub fn project_info_mut(&mut self) -> &mut ProjectInfoData {
        &mut self.project_info
    }

    // ======== Stage Management ========

    /// Get current stage
    #[inline]
    pub fn current_stage(&self) -> u32 {
        self.current_stage
    }

    /// Set current stage
    #[inline]
    pub fn set_current_stage(&mut self, stage: u32) {
        self.current_stage = stage;
    }

    /// Get max stage
    #[inline]
    pub fn max_stage(&self) -> u32 {
        self.max_stage
    }

    /// Set max stage
    #[inline]
    pub fn set_max_stage(&mut self, stage: u32) {
        self.max_stage = stage;
    }

    /// Check if staged metaprogramming is enabled
    #[inline]
    pub fn staged_enabled(&self) -> bool {
        self.staged_enabled
    }

    /// Enable/disable staged metaprogramming
    #[inline]
    pub fn set_staged_enabled(&mut self, enabled: bool) {
        self.staged_enabled = enabled;
    }

    /// Get function stage level
    pub fn get_function_stage(&self, name: &Text) -> Option<u32> {
        self.function_stages.get(name).copied()
    }

    /// Set function stage level
    pub fn set_function_stage(&mut self, name: Text, stage: u32) {
        self.function_stages.insert(name, stage);
    }

    /// Get function stages map
    pub fn function_stages(&self) -> &Map<Text, u32> {
        &self.function_stages
    }

    /// Get mutable function stages
    pub fn function_stages_mut(&mut self) -> &mut Map<Text, u32> {
        &mut self.function_stages
    }

    /// Get stage config value
    pub fn get_stage_config(&self, key: &Text) -> Option<&Text> {
        self.stage_config.get(key)
    }

    /// Set stage config value
    pub fn set_stage_config(&mut self, key: Text, value: Text) {
        self.stage_config.insert(key, value);
    }

    /// Get stage config map
    pub fn stage_config(&self) -> &Map<Text, Text> {
        &self.stage_config
    }

    /// Get mutable stage config
    pub fn stage_config_mut(&mut self) -> &mut Map<Text, Text> {
        &mut self.stage_config
    }

    // ======== Generation Tracking ========

    /// Add a generation record
    pub fn add_generation_record(&mut self, record: StageRecord) {
        self.generation_chain.push(record);
    }

    /// Get generation chain
    pub fn generation_chain(&self) -> &List<StageRecord> {
        &self.generation_chain
    }

    /// Get mutable generation chain
    pub fn generation_chain_mut(&mut self) -> &mut List<StageRecord> {
        &mut self.generation_chain
    }

    /// Set generation origin
    #[inline]
    pub fn set_generation_origin(&mut self, span: Option<Span>) {
        self.generation_origin = span;
    }

    /// Get generation origin
    #[inline]
    pub fn generation_origin(&self) -> Option<Span> {
        self.generation_origin
    }

    /// Set generation source function
    #[inline]
    pub fn set_generation_source_function(&mut self, func: Option<Text>) {
        self.generation_source_function = func;
    }

    /// Get generation source function
    #[inline]
    pub fn generation_source_function(&self) -> Option<&Text> {
        self.generation_source_function.as_ref()
    }

    // ======== String Literals ========

    /// Add a string literal
    pub fn add_string_literal(&mut self, literal: Text, span: Span) {
        self.string_literals.push((literal, span));
    }

    /// Get string literals
    pub fn string_literals(&self) -> &List<(Text, Span)> {
        &self.string_literals
    }

    /// Get mutable string literals
    pub fn string_literals_mut(&mut self) -> &mut List<(Text, Span)> {
        &mut self.string_literals
    }

    // ======== Benchmarking ========

    /// Add a benchmark result
    pub fn add_bench_result(&mut self, name: Text, result: BenchResult) {
        self.bench_results
            .entry(name)
            .or_insert_with(List::new)
            .push(result);
    }

    /// Get benchmark results
    pub fn bench_results(&self) -> &Map<Text, List<BenchResult>> {
        &self.bench_results
    }

    /// Get mutable benchmark results
    pub fn bench_results_mut(&mut self) -> &mut Map<Text, List<BenchResult>> {
        &mut self.bench_results
    }

    /// Add a memory report
    pub fn add_memory_report(&mut self, name: Text, bytes: u64) {
        self.memory_reports.insert(name, bytes);
    }

    /// Get memory reports
    pub fn memory_reports(&self) -> &Map<Text, u64> {
        &self.memory_reports
    }

    /// Increment a counter
    pub fn increment_counter(&mut self, name: Text) {
        *self.counters.entry(name).or_insert(0) += 1;
    }

    /// Get counter value
    pub fn get_counter(&self, name: &Text) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    /// Get all counters
    pub fn counters(&self) -> &Map<Text, u64> {
        &self.counters
    }

    // ======== Memory Tracking ========

    /// Update memory usage
    pub fn update_memory_used(&mut self, bytes: u64) {
        self.memory_used = bytes;
        if bytes > self.peak_memory {
            self.peak_memory = bytes;
        }
    }

    /// Get current memory usage
    #[inline]
    pub fn memory_used(&self) -> u64 {
        self.memory_used
    }

    /// Get peak memory usage
    #[inline]
    pub fn peak_memory(&self) -> u64 {
        self.peak_memory
    }

    /// Reset memory tracking
    pub fn reset_memory_tracking(&mut self) {
        self.memory_used = 0;
        self.peak_memory = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_management() {
        let mut config = BuildConfiguration::new();
        assert_eq!(config.current_stage(), 1);

        config.set_current_stage(2);
        assert_eq!(config.current_stage(), 2);

        config.set_function_stage(Text::from("my_macro"), 3);
        assert_eq!(config.get_function_stage(&Text::from("my_macro")), Some(3));
    }

    #[test]
    fn test_memory_tracking() {
        let mut config = BuildConfiguration::new();
        assert_eq!(config.memory_used(), 0);
        assert_eq!(config.peak_memory(), 0);

        config.update_memory_used(1000);
        assert_eq!(config.memory_used(), 1000);
        assert_eq!(config.peak_memory(), 1000);

        config.update_memory_used(500);
        assert_eq!(config.memory_used(), 500);
        assert_eq!(config.peak_memory(), 1000); // Peak unchanged

        config.update_memory_used(2000);
        assert_eq!(config.peak_memory(), 2000); // New peak
    }

    #[test]
    fn test_counters() {
        let mut config = BuildConfiguration::new();
        assert_eq!(config.get_counter(&Text::from("calls")), 0);

        config.increment_counter(Text::from("calls"));
        config.increment_counter(Text::from("calls"));
        assert_eq!(config.get_counter(&Text::from("calls")), 2);
    }
}
