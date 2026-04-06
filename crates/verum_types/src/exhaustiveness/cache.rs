//! Incremental Exhaustiveness Checking
//!
//! This module provides caching for exhaustiveness results to enable
//! incremental checking. When patterns haven't changed, we can reuse
//! previous results instead of re-analyzing.
//!
//! ## Cache Keys
//!
//! Cache entries are keyed by:
//! 1. Pattern structure hash (structural equality)
//! 2. Scrutinee type hash
//! 3. Type environment hash (relevant type definitions)
//!
//! ## Invalidation
//!
//! Cache entries are invalidated when:
//! - Pattern structure changes
//! - Scrutinee type changes
//! - Relevant type definitions change
//! - Cache exceeds size limits (LRU eviction)

use super::{ExhaustivenessResult, PatternColumn};
use crate::ty::Type;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use verum_common::{List, Text};

/// Configuration for the exhaustiveness cache
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cache entries (default: 10,000)
    pub max_entries: usize,
    /// Maximum age of cache entries before eviction (default: 5 minutes)
    pub max_age: Duration,
    /// Whether to enable structural caching (default: true)
    pub enable_structural_cache: bool,
    /// Whether to enable type definition tracking (default: true)
    pub track_type_definitions: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_age: Duration::from_secs(300),
            enable_structural_cache: true,
            track_type_definitions: true,
        }
    }
}

/// Cache key for exhaustiveness results
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Hash of pattern structures
    patterns_hash: u64,
    /// Hash of scrutinee type
    type_hash: u64,
    /// Hash of relevant type definitions
    env_hash: u64,
}

impl CacheKey {
    /// Create a cache key from patterns and type
    pub fn new(patterns: &[PatternColumn], scrutinee_ty: &Type, env_hash: u64) -> Self {
        let mut patterns_hasher = std::collections::hash_map::DefaultHasher::new();
        for pattern in patterns {
            hash_pattern(pattern, &mut patterns_hasher);
        }
        let patterns_hash = patterns_hasher.finish();

        let type_hash = hash_type(scrutinee_ty);

        Self {
            patterns_hash,
            type_hash,
            env_hash,
        }
    }
}

/// Cached exhaustiveness result with metadata
#[derive(Debug, Clone)]
pub struct CachedResult {
    /// The cached result
    pub result: ExhaustivenessResult,
    /// When the entry was created
    pub created_at: Instant,
    /// Number of times this entry has been accessed
    pub access_count: u64,
    /// Last access time
    pub last_accessed: Instant,
}

impl CachedResult {
    /// Create a new cached result
    pub fn new(result: ExhaustivenessResult) -> Self {
        let now = Instant::now();
        Self {
            result,
            created_at: now,
            access_count: 0,
            last_accessed: now,
        }
    }

    /// Check if the entry is expired
    pub fn is_expired(&self, max_age: Duration) -> bool {
        self.created_at.elapsed() > max_age
    }

    /// Record an access
    pub fn record_access(&mut self) {
        self.access_count += 1;
        self.last_accessed = Instant::now();
    }
}

/// Statistics about cache performance
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of entries evicted
    pub evictions: u64,
    /// Number of entries expired
    pub expirations: u64,
    /// Current number of entries
    pub current_entries: usize,
    /// Total time saved by cache hits
    pub time_saved: Duration,
}

impl CacheStats {
    /// Calculate hit rate
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64) / (total as f64)
        }
    }
}

/// Thread-safe exhaustiveness cache
pub struct ExhaustivenessCache {
    config: CacheConfig,
    entries: RwLock<HashMap<CacheKey, CachedResult>>,
    stats: RwLock<CacheStats>,
    type_definition_hashes: RwLock<HashMap<Text, u64>>,
}

impl ExhaustivenessCache {
    /// Create a new cache with default configuration
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create a new cache with custom configuration
    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            config,
            entries: RwLock::new(HashMap::new()),
            stats: RwLock::new(CacheStats::default()),
            type_definition_hashes: RwLock::new(HashMap::new()),
        }
    }

    /// Get a cached result if available and valid
    ///
    /// Performance: Uses read lock first to check existence, only upgrades to
    /// write lock when necessary for updates.
    pub fn get(&self, key: &CacheKey) -> Option<ExhaustivenessResult> {
        // First, check with read lock (fast path for cache misses)
        {
            let entries = self.entries.read().ok()?;
            if let Some(entry) = entries.get(key) {
                // Check expiration without write lock
                if !entry.is_expired(self.config.max_age) {
                    // Found valid entry - clone result while holding read lock
                    let result = entry.result.clone();

                    // Drop read lock before acquiring write lock
                    drop(entries);

                    // Update stats with write lock (can fail, it's just stats)
                    if let (Ok(mut entries_w), Ok(mut stats)) =
                        (self.entries.write(), self.stats.write())
                    {
                        if let Some(entry) = entries_w.get_mut(key) {
                            entry.record_access();
                        }
                        stats.hits += 1;
                    }

                    return Some(result);
                }
            } else {
                // Definitely not in cache - update miss stat
                drop(entries);
                if let Ok(mut stats) = self.stats.write() {
                    stats.misses += 1;
                }
                return None;
            }
        }

        // Entry exists but is expired - need write lock to remove it
        let mut entries = self.entries.write().ok()?;
        let mut stats = self.stats.write().ok()?;

        // Double-check with write lock (another thread might have removed it)
        if let Some(entry) = entries.get(key) {
            if entry.is_expired(self.config.max_age) {
                entries.remove(key);
                stats.expirations += 1;
                stats.misses += 1;
                return None;
            }
            // Not expired after all - return it
            let result = entry.result.clone();
            if let Some(entry) = entries.get_mut(key) {
                entry.record_access();
            }
            stats.hits += 1;
            return Some(result);
        }

        // Cache miss
        stats.misses += 1;
        None
    }

    /// Store a result in the cache
    pub fn put(&self, key: CacheKey, result: ExhaustivenessResult) {
        let mut entries = match self.entries.write() {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut stats = match self.stats.write() {
            Ok(s) => s,
            Err(_) => return,
        };

        // Check if we need to evict entries
        if entries.len() >= self.config.max_entries {
            self.evict_lru(&mut entries, &mut stats);
        }

        entries.insert(key, CachedResult::new(result));
        stats.current_entries = entries.len();
    }

    /// Evict least recently used entries
    fn evict_lru(&self, entries: &mut HashMap<CacheKey, CachedResult>, stats: &mut CacheStats) {
        // Find the LRU entry
        let mut oldest_key = None;
        let mut oldest_time = Instant::now();

        for (key, entry) in entries.iter() {
            if entry.last_accessed < oldest_time {
                oldest_time = entry.last_accessed;
                oldest_key = Some(key.clone());
            }
        }

        if let Some(key) = oldest_key {
            entries.remove(&key);
            stats.evictions += 1;
        }
    }

    /// Clear all entries
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
        if let Ok(mut stats) = self.stats.write() {
            stats.current_entries = 0;
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> Option<CacheStats> {
        self.stats.read().ok().map(|s| s.clone())
    }

    /// Register a type definition for tracking
    pub fn register_type_definition(&self, name: Text, definition_hash: u64) {
        if self.config.track_type_definitions {
            if let Ok(mut hashes) = self.type_definition_hashes.write() {
                hashes.insert(name, definition_hash);
            }
        }
    }

    /// Invalidate cache entries that depend on a changed type
    pub fn invalidate_type(&self, name: &Text) {
        // For now, we clear the entire cache when a type changes
        // A more sophisticated implementation would track dependencies
        if let Ok(mut hashes) = self.type_definition_hashes.write() {
            if hashes.remove(name).is_some() {
                self.clear();
            }
        }
    }

    /// Get the current environment hash for cache keys
    pub fn env_hash(&self) -> u64 {
        let hashes = match self.type_definition_hashes.read() {
            Ok(h) => h,
            Err(_) => return 0,
        };

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (name, hash) in hashes.iter() {
            name.hash(&mut hasher);
            hash.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl Default for ExhaustivenessCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Global cache instance
static GLOBAL_CACHE: std::sync::OnceLock<Arc<ExhaustivenessCache>> = std::sync::OnceLock::new();

/// Get the global exhaustiveness cache
pub fn global_cache() -> Arc<ExhaustivenessCache> {
    GLOBAL_CACHE
        .get_or_init(|| Arc::new(ExhaustivenessCache::new()))
        .clone()
}

/// Clear the global cache
pub fn clear_global_cache() {
    if let Some(cache) = GLOBAL_CACHE.get() {
        cache.clear();
    }
}

/// Get global cache statistics
pub fn global_cache_stats() -> Option<CacheStats> {
    GLOBAL_CACHE.get()?.stats()
}

/// Hash a pattern for caching
fn hash_pattern(pattern: &PatternColumn, hasher: &mut impl Hasher) {
    use std::mem::discriminant;

    discriminant(pattern).hash(hasher);

    match pattern {
        PatternColumn::Wildcard => {}
        PatternColumn::Literal(lit) => {
            use super::matrix::LiteralPattern;
            match lit {
                LiteralPattern::Int(n) => n.hash(hasher),
                LiteralPattern::Float(f) => f.to_bits().hash(hasher),
                LiteralPattern::Bool(b) => b.hash(hasher),
                LiteralPattern::Char(c) => c.hash(hasher),
                LiteralPattern::Text(s) => s.hash(hasher),
            }
        }
        PatternColumn::Constructor { name, args } => {
            name.hash(hasher);
            for arg in args.iter() {
                hash_pattern(arg, hasher);
            }
        }
        PatternColumn::Tuple(elements) => {
            for elem in elements.iter() {
                hash_pattern(elem, hasher);
            }
        }
        PatternColumn::Array(elements) => {
            for elem in elements.iter() {
                hash_pattern(elem, hasher);
            }
        }
        PatternColumn::Record { fields, rest } => {
            for (name, pattern) in fields.iter() {
                name.hash(hasher);
                hash_pattern(pattern, hasher);
            }
            rest.hash(hasher);
        }
        PatternColumn::Range {
            start,
            end,
            inclusive,
        } => {
            start.hash(hasher);
            end.hash(hasher);
            inclusive.hash(hasher);
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                hash_pattern(alt, hasher);
            }
        }
        PatternColumn::And(operands) => {
            for op in operands.iter() {
                hash_pattern(op, hasher);
            }
        }
        PatternColumn::Guarded(inner) => {
            hash_pattern(inner, hasher);
        }
        PatternColumn::Reference { mutable, inner } => {
            mutable.hash(hasher);
            hash_pattern(inner, hasher);
        }
        PatternColumn::Stream { head_patterns, tail } => {
            for pattern in head_patterns.iter() {
                hash_pattern(pattern, hasher);
            }
            if let Some(tail_pattern) = tail {
                hash_pattern(tail_pattern, hasher);
            }
        }
        PatternColumn::TypeTest { type_name, binding } => {
            type_name.hash(hasher);
            if let Some(binding_pattern) = binding {
                hash_pattern(binding_pattern, hasher);
            }
        }
        PatternColumn::Active { name, bindings, is_total } => {
            name.hash(hasher);
            is_total.hash(hasher);
            for binding in bindings.iter() {
                hash_pattern(binding, hasher);
            }
        }
    }
}

/// Hash a type for caching
fn hash_type(ty: &Type) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_type_inner(ty, &mut hasher);
    hasher.finish()
}

fn hash_type_inner(ty: &Type, hasher: &mut impl Hasher) {
    use std::mem::discriminant;

    discriminant(ty).hash(hasher);

    match ty {
        Type::Named { path, args } => {
            // Hash path as string representation
            for segment in path.segments.iter() {
                format!("{:?}", segment).hash(hasher);
            }
            for arg in args.iter() {
                hash_type_inner(arg, hasher);
            }
        }
        Type::Generic { name, args } => {
            name.hash(hasher);
            for arg in args.iter() {
                hash_type_inner(arg, hasher);
            }
        }
        Type::Tuple(elements) => {
            for elem in elements.iter() {
                hash_type_inner(elem, hasher);
            }
        }
        Type::Function {
            params,
            return_type,
            ..
        } => {
            for param in params.iter() {
                hash_type_inner(param, hasher);
            }
            hash_type_inner(return_type, hasher);
        }
        Type::Variant(variants) => {
            for (name, ty) in variants.iter() {
                name.hash(hasher);
                hash_type_inner(ty, hasher);
            }
        }
        Type::Record(fields) => {
            for (name, ty) in fields.iter() {
                name.hash(hasher);
                hash_type_inner(ty, hasher);
            }
        }
        Type::Reference { inner, .. } => {
            hash_type_inner(inner, hasher);
        }
        Type::Array { element, size } => {
            hash_type_inner(element, hasher);
            size.hash(hasher);
        }
        Type::Refined { base, .. } => {
            hash_type_inner(base, hasher);
        }
        // Primitive types - discriminant is enough
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_config_default() {
        let config = CacheConfig::default();
        assert_eq!(config.max_entries, 10_000);
        assert_eq!(config.max_age, Duration::from_secs(300));
        assert!(config.enable_structural_cache);
        assert!(config.track_type_definitions);
    }

    #[test]
    fn test_cache_put_get() {
        let cache = ExhaustivenessCache::new();
        let key = CacheKey {
            patterns_hash: 123,
            type_hash: 456,
            env_hash: 789,
        };
        let result = ExhaustivenessResult {
            is_exhaustive: true,
            uncovered_witnesses: List::new(),
            redundant_patterns: List::new(),
            all_guarded: false,
            range_overlaps: None,
            warnings: List::new(),
        };

        cache.put(key.clone(), result.clone());
        let retrieved = cache.get(&key);

        assert!(retrieved.is_some());
        assert!(retrieved.unwrap().is_exhaustive);
    }

    #[test]
    fn test_cache_miss() {
        let cache = ExhaustivenessCache::new();
        let key = CacheKey {
            patterns_hash: 123,
            type_hash: 456,
            env_hash: 789,
        };

        let result = cache.get(&key);
        assert!(result.is_none());

        let stats = cache.stats().unwrap();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
    }

    #[test]
    fn test_cache_stats() {
        let cache = ExhaustivenessCache::new();
        let key = CacheKey {
            patterns_hash: 1,
            type_hash: 2,
            env_hash: 3,
        };
        let result = ExhaustivenessResult {
            is_exhaustive: false,
            uncovered_witnesses: List::new(),
            redundant_patterns: List::new(),
            all_guarded: false,
            range_overlaps: None,
            warnings: List::new(),
        };

        // Miss
        cache.get(&key);

        // Put
        cache.put(key.clone(), result);

        // Hit
        cache.get(&key);

        let stats = cache.stats().unwrap();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.current_entries, 1);
    }

    #[test]
    fn test_hit_rate() {
        let mut stats = CacheStats::default();
        stats.hits = 80;
        stats.misses = 20;
        assert!((stats.hit_rate() - 0.8).abs() < 0.0001);
    }

    #[test]
    fn test_clear() {
        let cache = ExhaustivenessCache::new();
        let key = CacheKey {
            patterns_hash: 1,
            type_hash: 2,
            env_hash: 3,
        };
        let result = ExhaustivenessResult {
            is_exhaustive: true,
            uncovered_witnesses: List::new(),
            redundant_patterns: List::new(),
            all_guarded: false,
            range_overlaps: None,
            warnings: List::new(),
        };

        cache.put(key.clone(), result);
        assert!(cache.get(&key).is_some());

        cache.clear();
        assert!(cache.get(&key).is_none());
    }
}
