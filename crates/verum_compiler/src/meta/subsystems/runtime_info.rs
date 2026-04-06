//! Runtime information for meta functions
//!
//! Provides compile-time and build-time information that can be
//! queried by meta functions via the MetaRuntime context.

use verum_common::{List, Map, Text};

/// Runtime information available to meta functions via MetaRuntime context
///
/// This struct holds all the compile-time and build-time information
/// that can be queried by meta functions.
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    /// Current crate name (from Verum.toml [cog].name)
    pub crate_name: Option<Text>,
    /// Current module path (e.g., "my_crate::my_module")
    pub module_path: Option<Text>,
    /// Crate version as (major, minor, patch)
    pub crate_version: Option<(i64, i64, i64)>,
    /// List of enabled features
    pub enabled_features: List<Text>,
    /// Whether building in debug mode
    pub is_debug: bool,
    /// Optimization level (0-3)
    pub opt_level: u8,
    /// Runtime configuration ("full", "single_thread", "no_async", etc.)
    pub runtime_config: Option<Text>,
    /// Recursion limit for meta functions
    pub recursion_limit: usize,
    /// Iteration limit for loops in meta functions
    pub iteration_limit: usize,
    /// Memory limit in bytes for meta function execution
    pub memory_limit: usize,
    /// Timeout in milliseconds for meta function execution
    pub timeout_ms: u64,
    /// Configuration values from Verum.toml (string values)
    pub config: Map<Text, Text>,
    /// Configuration arrays from Verum.toml
    pub config_arrays: Map<Text, List<Text>>,
}

impl Default for RuntimeInfo {
    fn default() -> Self {
        Self {
            crate_name: None,
            module_path: None,
            crate_version: None,
            enabled_features: List::new(),
            is_debug: cfg!(debug_assertions),
            opt_level: if cfg!(debug_assertions) { 0 } else { 2 },
            runtime_config: Some(Text::from("full")),
            recursion_limit: 128,
            iteration_limit: 1_000_000,
            memory_limit: 256 * 1024 * 1024, // 256 MB
            timeout_ms: 30_000,              // 30 seconds
            config: Map::new(),
            config_arrays: Map::new(),
        }
    }
}

impl RuntimeInfo {
    /// Create a new RuntimeInfo with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the crate name
    #[inline]
    pub fn with_crate_name(mut self, name: impl Into<Text>) -> Self {
        self.crate_name = Some(name.into());
        self
    }

    /// Set the module path
    #[inline]
    pub fn with_module_path(mut self, path: impl Into<Text>) -> Self {
        self.module_path = Some(path.into());
        self
    }

    /// Set the crate version
    #[inline]
    pub fn with_crate_version(mut self, major: i64, minor: i64, patch: i64) -> Self {
        self.crate_version = Some((major, minor, patch));
        self
    }

    /// Add an enabled feature
    #[inline]
    pub fn with_feature(mut self, feature: impl Into<Text>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    /// Set debug mode
    #[inline]
    pub fn with_debug(mut self, is_debug: bool) -> Self {
        self.is_debug = is_debug;
        self
    }

    /// Set optimization level
    #[inline]
    pub fn with_opt_level(mut self, level: u8) -> Self {
        self.opt_level = level;
        self
    }

    /// Set a config value
    #[inline]
    pub fn with_config(mut self, key: impl Into<Text>, value: impl Into<Text>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Set a config array
    #[inline]
    pub fn with_config_array(mut self, key: impl Into<Text>, values: List<Text>) -> Self {
        self.config_arrays.insert(key.into(), values);
        self
    }
}
