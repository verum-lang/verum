//! Macro state context for meta functions
//!
//! Provides caching, memoization, and invocation tracking for meta functions.

use verum_ast::MetaValue;
use verum_common::{List, Map, Text};

/// Cache statistics for debugging
///
/// Matches: core/meta/contexts.vr CacheStats
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of entries in cache
    pub entry_count: usize,
    /// Number of cache hits
    pub hits: usize,
    /// Number of cache misses
    pub misses: usize,
    /// Approximate memory usage in bytes
    pub memory_bytes: usize,
}

impl CacheStats {
    /// Convert to MetaValue tuple representation
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(List::from(vec![
            MetaValue::Int(self.entry_count as i128),
            MetaValue::Int(self.hits as i128),
            MetaValue::Int(self.misses as i128),
            MetaValue::Int(self.memory_bytes as i128),
        ]))
    }

    /// Alias for to_meta_value for backward compatibility
    #[inline]
    pub fn to_const_value(&self) -> MetaValue {
        self.to_meta_value()
    }
}

/// Macro state context configuration
///
/// Provides caching, memoization, and invocation tracking for meta functions.
/// Matches: core/meta/contexts.vr MacroState
#[derive(Debug, Clone)]
pub struct MacroStateInfo {
    /// Cache for values (key -> value)
    cache: Map<Text, MetaValue>,
    /// Cache statistics
    stats: CacheStats,
    /// Current macro name
    current_macro: Option<Text>,
    /// Current call depth
    call_depth: usize,
    /// Invocation counter (monotonically increasing)
    invocation_counter: u64,
    /// Invocation count per macro name
    invocation_counts: Map<Text, usize>,
    /// File dependencies
    file_dependencies: List<Text>,
    /// Type dependencies (by type name)
    type_dependencies: List<Text>,
    /// Environment variable dependencies
    env_dependencies: List<Text>,
}

impl Default for MacroStateInfo {
    fn default() -> Self {
        Self {
            cache: Map::new(),
            stats: CacheStats::default(),
            current_macro: None,
            call_depth: 0,
            invocation_counter: 0,
            invocation_counts: Map::new(),
            file_dependencies: List::new(),
            type_dependencies: List::new(),
            env_dependencies: List::new(),
        }
    }
}

impl MacroStateInfo {
    /// Create new MacroStateInfo
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current macro name and increment invocation count
    pub fn enter_macro(&mut self, name: Text) {
        self.call_depth += 1;
        self.invocation_counter += 1;

        // Update invocation count for this macro
        let count = self.invocation_counts.get(&name).copied().unwrap_or(0);
        self.invocation_counts.insert(name.clone(), count + 1);

        self.current_macro = Some(name);
    }

    /// Exit the current macro
    pub fn exit_macro(&mut self) {
        if self.call_depth > 0 {
            self.call_depth -= 1;
        }
        if self.call_depth == 0 {
            self.current_macro = None;
        }
    }

    /// Get value from cache
    pub fn cache_get(&mut self, key: &Text) -> Option<MetaValue> {
        match self.cache.get(key) {
            Some(value) => {
                self.stats.hits += 1;
                Some(value.clone())
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Set value in cache
    pub fn cache_set(&mut self, key: Text, value: MetaValue) {
        // Approximate memory size (rough estimate)
        let approx_size = std::mem::size_of::<MetaValue>() + key.len();

        if !self.cache.contains_key(&key) {
            self.stats.entry_count += 1;
            self.stats.memory_bytes += approx_size;
        }

        self.cache.insert(key, value);
    }

    /// Check if cache contains key
    #[inline]
    pub fn cache_has(&self, key: &Text) -> bool {
        self.cache.contains_key(key)
    }

    /// Remove value from cache
    pub fn cache_remove(&mut self, key: &Text) -> Option<MetaValue> {
        if let Some(value) = self.cache.remove(key) {
            self.stats.entry_count = self.stats.entry_count.saturating_sub(1);
            self.stats.memory_bytes = self
                .stats
                .memory_bytes
                .saturating_sub(std::mem::size_of::<MetaValue>() + key.len());
            Some(value)
        } else {
            None
        }
    }

    /// Clear all cached values
    pub fn cache_clear(&mut self) {
        self.cache.clear();
        self.stats.entry_count = 0;
        self.stats.memory_bytes = 0;
    }

    /// Get all cache keys
    pub fn cache_keys(&self) -> List<Text> {
        self.cache.keys().cloned().collect()
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        self.stats.clone()
    }

    /// Get invocation count for current macro
    pub fn invocation_count(&self) -> usize {
        self.current_macro
            .as_ref()
            .and_then(|name| self.invocation_counts.get(name).copied())
            .unwrap_or(0)
    }

    /// Get unique invocation ID
    #[inline]
    pub fn invocation_id(&self) -> u64 {
        self.invocation_counter
    }

    /// Get current macro name
    pub fn current_macro_name(&self) -> Text {
        self.current_macro.clone().unwrap_or_else(|| Text::from(""))
    }

    /// Get current call depth
    #[inline]
    pub fn call_depth(&self) -> usize {
        self.call_depth
    }

    /// Register file dependency
    pub fn depend_on_file(&mut self, path: Text) {
        if !self.file_dependencies.contains(&path) {
            self.file_dependencies.push(path);
        }
    }

    /// Register type dependency
    pub fn depend_on_type(&mut self, type_name: Text) {
        if !self.type_dependencies.contains(&type_name) {
            self.type_dependencies.push(type_name);
        }
    }

    /// Register environment variable dependency
    pub fn depend_on_env(&mut self, var: Text) {
        if !self.env_dependencies.contains(&var) {
            self.env_dependencies.push(var);
        }
    }

    /// Get file dependencies
    #[inline]
    pub fn file_dependencies(&self) -> &List<Text> {
        &self.file_dependencies
    }

    /// Get type dependencies
    #[inline]
    pub fn type_dependencies(&self) -> &List<Text> {
        &self.type_dependencies
    }

    /// Get environment variable dependencies
    #[inline]
    pub fn env_dependencies(&self) -> &List<Text> {
        &self.env_dependencies
    }
}
