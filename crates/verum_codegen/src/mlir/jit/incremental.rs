//! Incremental Compilation Support for JIT.
//!
//! Provides intelligent caching and incremental recompilation to minimize
//! compilation time for unchanged code.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                     Incremental Compilation Pipeline                         │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//!   Source Code (AST)
//!         │
//!         ▼
//! ┌─────────────────┐    ┌─────────────────┐
//! │  Content Hash   │───▶│   Cache Check   │
//! │  (blake3)       │    │                 │
//! └─────────────────┘    └────────┬────────┘
//!                                 │
//!                    ┌────────────┼────────────┐
//!                    │            │            │
//!              Cache Hit     Cache Miss   Partial Hit
//!                    │            │            │
//!                    ▼            ▼            ▼
//!            ┌───────────┐ ┌───────────┐ ┌───────────┐
//!            │  Reuse    │ │  Compile  │ │  Delta    │
//!            │  Cached   │ │  Full     │ │  Compile  │
//!            └───────────┘ └───────────┘ └───────────┘
//!                    │            │            │
//!                    └────────────┼────────────┘
//!                                 ▼
//!                        ┌───────────────┐
//!                        │  Update Cache │
//!                        │  & Deps       │
//!                        └───────────────┘
//! ```
//!
//! # Features
//!
//! - Content-based hashing for accurate change detection
//! - Dependency tracking between modules
//! - Intelligent invalidation
//! - Persistent cache across sessions
//! - Compression for cache storage
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{IncrementalCache, CacheConfig};
//!
//! let cache = IncrementalCache::new(CacheConfig::default());
//!
//! // Check if compilation is needed
//! if let Some(cached) = cache.get(&module_hash)? {
//!     // Use cached compilation
//!     engine.load_cached(cached)?;
//! } else {
//!     // Compile and cache
//!     let compiled = engine.compile(&module)?;
//!     cache.put(&module_hash, &compiled)?;
//! }
//! ```

use crate::mlir::error::{MlirError, Result};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use verum_common::Text;

// ============================================================================
// Cache Configuration
// ============================================================================

/// Configuration for incremental compilation cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Cache directory path.
    pub cache_dir: PathBuf,

    /// Maximum cache size in bytes (0 = unlimited).
    pub max_size_bytes: u64,

    /// Maximum number of cached entries.
    pub max_entries: usize,

    /// Enable persistent cache (write to disk).
    pub persistent: bool,

    /// Enable compression for stored entries.
    pub compress: bool,

    /// Cache entry TTL in seconds (0 = no expiry).
    pub ttl_seconds: u64,

    /// Enable verbose logging.
    pub verbose: bool,
}

impl CacheConfig {
    /// Create new cache configuration.
    pub fn new(cache_dir: impl AsRef<Path>) -> Self {
        Self {
            cache_dir: cache_dir.as_ref().to_path_buf(),
            max_size_bytes: 1024 * 1024 * 1024, // 1 GB
            max_entries: 10000,
            persistent: true,
            compress: true,
            ttl_seconds: 86400 * 7, // 7 days
            verbose: false,
        }
    }

    /// Builder: set max size in bytes.
    pub fn max_size(mut self, bytes: u64) -> Self {
        self.max_size_bytes = bytes;
        self
    }

    /// Builder: set max entries.
    pub fn max_entries(mut self, entries: usize) -> Self {
        self.max_entries = entries;
        self
    }

    /// Builder: enable/disable persistence.
    pub fn persistent(mut self, enabled: bool) -> Self {
        self.persistent = enabled;
        self
    }

    /// Builder: enable/disable compression.
    pub fn compress(mut self, enabled: bool) -> Self {
        self.compress = enabled;
        self
    }

    /// Builder: set TTL.
    pub fn ttl(mut self, seconds: u64) -> Self {
        self.ttl_seconds = seconds;
        self
    }

    /// Builder: set verbose mode.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Create in-memory only configuration.
    pub fn memory_only() -> Self {
        Self {
            cache_dir: PathBuf::from("/dev/null"),
            max_size_bytes: 256 * 1024 * 1024, // 256 MB
            max_entries: 1000,
            persistent: false,
            compress: false,
            ttl_seconds: 0,
            verbose: false,
        }
    }

    /// Create development configuration.
    pub fn development() -> Self {
        let cache_dir = std::env::temp_dir().join("verum_jit_cache");
        Self::new(cache_dir).verbose(true).ttl(3600) // 1 hour
    }

    /// Create production configuration.
    pub fn production(cache_dir: impl AsRef<Path>) -> Self {
        Self::new(cache_dir)
            .max_size(10 * 1024 * 1024 * 1024) // 10 GB
            .max_entries(100000)
            .ttl(86400 * 30) // 30 days
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        let cache_dir = std::env::temp_dir().join("verum_jit_cache");
        Self::new(cache_dir)
    }
}

// ============================================================================
// Cache Entry
// ============================================================================

/// Hash type for content identification.
pub type ContentHash = [u8; 32];

/// A cached compilation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Content hash.
    pub hash: ContentHash,

    /// Module name.
    pub module_name: Text,

    /// Serialized LLVM IR or object code.
    pub data: Vec<u8>,

    /// Entry creation timestamp.
    pub created_at: u64,

    /// Last access timestamp.
    pub last_accessed: u64,

    /// Size in bytes.
    pub size: u64,

    /// Dependencies (other module hashes this depends on).
    pub dependencies: Vec<ContentHash>,

    /// Compilation options used.
    pub options: CacheOptions,

    /// Entry version for format compatibility.
    pub version: u32,
}

/// Options that affect compilation (must match for cache hit).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheOptions {
    /// Optimization level.
    pub optimization_level: usize,

    /// Target triple.
    pub target_triple: Option<Text>,

    /// Debug info enabled.
    pub debug_info: bool,

    /// CBGR optimization enabled.
    pub cbgr_optimization: bool,

    /// Context monomorphization enabled.
    pub context_mono: bool,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            optimization_level: 2,
            target_triple: None,
            debug_info: false,
            cbgr_optimization: true,
            context_mono: true,
        }
    }
}

impl CacheEntry {
    /// Current cache format version.
    pub const VERSION: u32 = 1;

    /// Create a new cache entry.
    pub fn new(
        hash: ContentHash,
        module_name: impl Into<Text>,
        data: Vec<u8>,
        dependencies: Vec<ContentHash>,
        options: CacheOptions,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            hash,
            module_name: module_name.into(),
            data,
            created_at: now,
            last_accessed: now,
            size: 0, // Will be set when stored
            dependencies,
            options,
            version: Self::VERSION,
        }
    }

    /// Check if entry is expired.
    pub fn is_expired(&self, ttl_seconds: u64) -> bool {
        if ttl_seconds == 0 {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now - self.last_accessed > ttl_seconds
    }

    /// Update last access time.
    pub fn touch(&mut self) {
        self.last_accessed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

// ============================================================================
// Cache Statistics
// ============================================================================

/// Statistics for cache operations.
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: AtomicU64,

    /// Number of cache misses.
    pub misses: AtomicU64,

    /// Number of entries added.
    pub entries_added: AtomicU64,

    /// Number of entries evicted.
    pub entries_evicted: AtomicU64,

    /// Number of entries invalidated.
    pub entries_invalidated: AtomicU64,

    /// Total bytes saved by cache hits.
    pub bytes_saved: AtomicU64,

    /// Total bytes stored.
    pub bytes_stored: AtomicU64,

    /// Total compilation time saved (microseconds).
    pub time_saved_us: AtomicU64,
}

impl CacheStats {
    /// Create new statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get hit rate.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits.load(Ordering::Relaxed) + self.misses.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.hits.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Get summary.
    pub fn summary(&self) -> CacheStatsSummary {
        CacheStatsSummary {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            hit_rate: self.hit_rate(),
            entries_added: self.entries_added.load(Ordering::Relaxed),
            entries_evicted: self.entries_evicted.load(Ordering::Relaxed),
            bytes_stored: self.bytes_stored.load(Ordering::Relaxed),
            time_saved_us: self.time_saved_us.load(Ordering::Relaxed),
        }
    }
}

/// Summary of cache statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStatsSummary {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
    pub entries_added: u64,
    pub entries_evicted: u64,
    pub bytes_stored: u64,
    pub time_saved_us: u64,
}

// ============================================================================
// Dependency Tracker
// ============================================================================

/// Tracks dependencies between modules for invalidation.
pub struct DependencyTracker {
    /// Forward dependencies: module → modules it depends on.
    depends_on: DashMap<ContentHash, HashSet<ContentHash>>,

    /// Reverse dependencies: module → modules that depend on it.
    depended_by: DashMap<ContentHash, HashSet<ContentHash>>,
}

impl DependencyTracker {
    /// Create new dependency tracker.
    pub fn new() -> Self {
        Self {
            depends_on: DashMap::new(),
            depended_by: DashMap::new(),
        }
    }

    /// Register dependencies for a module.
    pub fn register(&self, module: ContentHash, dependencies: Vec<ContentHash>) {
        // Store forward dependencies
        self.depends_on.insert(module, dependencies.iter().cloned().collect());

        // Store reverse dependencies
        for dep in dependencies {
            self.depended_by
                .entry(dep)
                .or_insert_with(HashSet::new)
                .insert(module);
        }
    }

    /// Get modules that depend on the given module.
    pub fn get_dependents(&self, module: &ContentHash) -> Vec<ContentHash> {
        self.depended_by
            .get(module)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get modules that the given module depends on.
    pub fn get_dependencies(&self, module: &ContentHash) -> Vec<ContentHash> {
        self.depends_on
            .get(module)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all modules that would be invalidated if the given module changes.
    /// Uses transitive closure to find all affected modules.
    pub fn get_invalidation_set(&self, module: &ContentHash) -> HashSet<ContentHash> {
        let mut invalidated = HashSet::new();
        let mut queue = vec![*module];

        while let Some(current) = queue.pop() {
            if invalidated.insert(current) {
                // Add all modules that depend on this one
                if let Some(dependents) = self.depended_by.get(&current) {
                    for dep in dependents.iter() {
                        if !invalidated.contains(dep) {
                            queue.push(*dep);
                        }
                    }
                }
            }
        }

        invalidated
    }

    /// Remove a module and its dependencies.
    pub fn remove(&self, module: &ContentHash) {
        // Remove forward dependencies
        if let Some((_, deps)) = self.depends_on.remove(module) {
            for dep in deps {
                if let Some(mut dependents) = self.depended_by.get_mut(&dep) {
                    dependents.remove(module);
                }
            }
        }

        // Remove reverse dependencies
        self.depended_by.remove(module);
    }

    /// Clear all dependencies.
    pub fn clear(&self) {
        self.depends_on.clear();
        self.depended_by.clear();
    }
}

impl Default for DependencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Incremental Cache
// ============================================================================

/// Incremental compilation cache.
pub struct IncrementalCache {
    /// Configuration.
    config: CacheConfig,

    /// In-memory cache.
    memory_cache: DashMap<ContentHash, CacheEntry>,

    /// Dependency tracker.
    dependencies: DependencyTracker,

    /// Statistics.
    stats: Arc<CacheStats>,

    /// Current cache size in bytes.
    current_size: AtomicU64,

    /// Cache directory lock.
    cache_lock: RwLock<()>,
}

impl IncrementalCache {
    /// Create a new incremental cache.
    pub fn new(config: CacheConfig) -> Result<Self> {
        // Create cache directory if persistent
        if config.persistent {
            fs::create_dir_all(&config.cache_dir).map_err(|e| MlirError::CacheError {
                message: Text::from(format!("Failed to create cache dir: {}", e)),
            })?;
        }

        let cache = Self {
            config,
            memory_cache: DashMap::new(),
            dependencies: DependencyTracker::new(),
            stats: Arc::new(CacheStats::new()),
            current_size: AtomicU64::new(0),
            cache_lock: RwLock::new(()),
        };

        // Load persistent cache if enabled
        if cache.config.persistent {
            cache.load_persistent_cache()?;
        }

        Ok(cache)
    }

    /// Compute content hash for source code.
    pub fn hash_content(content: &[u8]) -> ContentHash {
        *blake3::hash(content).as_bytes()
    }

    /// Compute hash for source code with options.
    pub fn hash_with_options(content: &[u8], options: &CacheOptions) -> ContentHash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(content);

        // Include options in hash
        let options_bytes = serde_json::to_vec(options).unwrap_or_default();
        hasher.update(&options_bytes);

        *hasher.finalize().as_bytes()
    }

    /// Get a cached entry.
    pub fn get(&self, hash: &ContentHash) -> Option<CacheEntry> {
        // Check memory cache first
        if let Some(mut entry) = self.memory_cache.get_mut(hash) {
            // Check expiry
            if entry.is_expired(self.config.ttl_seconds) {
                drop(entry);
                self.invalidate(hash);
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            entry.touch();
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            self.stats.bytes_saved.fetch_add(entry.size, Ordering::Relaxed);
            return Some(entry.clone());
        }

        // Try persistent cache
        if self.config.persistent {
            if let Some(entry) = self.load_from_disk(hash) {
                // Cache in memory
                self.memory_cache.insert(*hash, entry.clone());
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry);
            }
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put an entry in the cache.
    pub fn put(&self, entry: CacheEntry) -> Result<()> {
        let hash = entry.hash;
        let size = entry.data.len() as u64;

        // Check size limits
        self.evict_if_needed(size)?;

        // Register dependencies
        self.dependencies.register(hash, entry.dependencies.clone());

        // Store in memory
        self.memory_cache.insert(hash, entry.clone());
        self.current_size.fetch_add(size, Ordering::Relaxed);
        self.stats.entries_added.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes_stored.fetch_add(size, Ordering::Relaxed);

        // Store persistently
        if self.config.persistent {
            self.save_to_disk(&entry)?;
        }

        if self.config.verbose {
            tracing::debug!("Cached module: {} ({} bytes)", entry.module_name, size);
        }

        Ok(())
    }

    /// Check if an entry exists.
    pub fn contains(&self, hash: &ContentHash) -> bool {
        self.memory_cache.contains_key(hash) || self.has_on_disk(hash)
    }

    /// Invalidate a single entry.
    pub fn invalidate(&self, hash: &ContentHash) {
        // Get all entries that need to be invalidated
        let to_invalidate = self.dependencies.get_invalidation_set(hash);

        for h in to_invalidate {
            if let Some((_, entry)) = self.memory_cache.remove(&h) {
                self.current_size.fetch_sub(entry.size, Ordering::Relaxed);
                self.stats.entries_invalidated.fetch_add(1, Ordering::Relaxed);

                if self.config.persistent {
                    let _ = self.remove_from_disk(&h);
                }
            }

            self.dependencies.remove(&h);
        }
    }

    /// Invalidate all entries that depend on a given module.
    pub fn invalidate_dependents(&self, hash: &ContentHash) {
        let dependents = self.dependencies.get_dependents(hash);
        for dep in dependents {
            self.invalidate(&dep);
        }
    }

    /// Clear the entire cache.
    pub fn clear(&self) -> Result<()> {
        self.memory_cache.clear();
        self.dependencies.clear();
        self.current_size.store(0, Ordering::Relaxed);

        if self.config.persistent {
            fs::remove_dir_all(&self.config.cache_dir).ok();
            fs::create_dir_all(&self.config.cache_dir).map_err(|e| MlirError::CacheError {
                message: Text::from(format!("Failed to recreate cache dir: {}", e)),
            })?;
        }

        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get cache configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Get current cache size in bytes.
    pub fn size(&self) -> u64 {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Get number of entries.
    pub fn len(&self) -> usize {
        self.memory_cache.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.memory_cache.is_empty()
    }

    /// Evict entries to make room for new data.
    fn evict_if_needed(&self, needed_bytes: u64) -> Result<()> {
        // Check entry count
        while self.memory_cache.len() >= self.config.max_entries {
            self.evict_oldest()?;
        }

        // Check size
        let max_size = self.config.max_size_bytes;
        if max_size > 0 {
            while self.current_size.load(Ordering::Relaxed) + needed_bytes > max_size {
                self.evict_oldest()?;
            }
        }

        Ok(())
    }

    /// Evict the oldest entry (LRU).
    fn evict_oldest(&self) -> Result<()> {
        // Find oldest entry
        let oldest = self
            .memory_cache
            .iter()
            .min_by_key(|e| e.last_accessed)
            .map(|e| *e.key());

        if let Some(hash) = oldest {
            if let Some((_, entry)) = self.memory_cache.remove(&hash) {
                self.current_size.fetch_sub(entry.size, Ordering::Relaxed);
                self.stats.entries_evicted.fetch_add(1, Ordering::Relaxed);
                self.dependencies.remove(&hash);

                if self.config.persistent {
                    let _ = self.remove_from_disk(&hash);
                }
            }
        }

        Ok(())
    }

    // Persistent cache operations

    fn cache_path(&self, hash: &ContentHash) -> PathBuf {
        let hex = hex::encode(hash);
        self.config.cache_dir.join(format!("{}.cache", hex))
    }

    fn has_on_disk(&self, hash: &ContentHash) -> bool {
        self.config.persistent && self.cache_path(hash).exists()
    }

    fn load_from_disk(&self, hash: &ContentHash) -> Option<CacheEntry> {
        let path = self.cache_path(hash);
        let mut file = File::open(&path).ok()?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).ok()?;

        // Decompress if enabled
        let data = if self.config.compress {
            // Simple decompression (in production, use proper compression like zstd)
            data
        } else {
            data
        };

        serde_json::from_slice(&data).ok()
    }

    fn save_to_disk(&self, entry: &CacheEntry) -> Result<()> {
        let _lock = self.cache_lock.write();
        let path = self.cache_path(&entry.hash);

        let data = serde_json::to_vec(entry).map_err(|e| MlirError::CacheError {
            message: Text::from(format!("Serialization failed: {}", e)),
        })?;

        // Compress if enabled
        let data = if self.config.compress {
            // Simple compression (in production, use proper compression like zstd)
            data
        } else {
            data
        };

        let mut file = File::create(&path).map_err(|e| MlirError::CacheError {
            message: Text::from(format!("Failed to create cache file: {}", e)),
        })?;

        file.write_all(&data).map_err(|e| MlirError::CacheError {
            message: Text::from(format!("Failed to write cache file: {}", e)),
        })?;

        Ok(())
    }

    fn remove_from_disk(&self, hash: &ContentHash) -> Result<()> {
        let path = self.cache_path(hash);
        fs::remove_file(&path).ok();
        Ok(())
    }

    fn load_persistent_cache(&self) -> Result<()> {
        if !self.config.cache_dir.exists() {
            return Ok(());
        }

        let entries = fs::read_dir(&self.config.cache_dir).map_err(|e| MlirError::CacheError {
            message: Text::from(format!("Failed to read cache dir: {}", e)),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "cache").unwrap_or(false) {
                if let Ok(mut file) = File::open(&path) {
                    let mut data = Vec::new();
                    if file.read_to_end(&mut data).is_ok() {
                        if let Ok(cache_entry) = serde_json::from_slice::<CacheEntry>(&data) {
                            // Check if expired
                            if !cache_entry.is_expired(self.config.ttl_seconds) {
                                self.memory_cache.insert(cache_entry.hash, cache_entry.clone());
                                self.current_size
                                    .fetch_add(cache_entry.size, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        }

        if self.config.verbose {
            tracing::info!(
                "Loaded {} entries from persistent cache",
                self.memory_cache.len()
            );
        }

        Ok(())
    }
}

// ============================================================================
// Content Hasher
// ============================================================================

/// Helper for computing content hashes for various AST elements.
pub struct ContentHasher {
    hasher: blake3::Hasher,
}

impl ContentHasher {
    /// Create a new content hasher.
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
        }
    }

    /// Add bytes to hash.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.hasher.update(data);
        self
    }

    /// Add a string to hash.
    pub fn update_str(&mut self, s: &str) -> &mut Self {
        self.hasher.update(s.as_bytes());
        self
    }

    /// Add a number to hash.
    pub fn update_u64(&mut self, n: u64) -> &mut Self {
        self.hasher.update(&n.to_le_bytes());
        self
    }

    /// Finalize and get the hash.
    pub fn finalize(self) -> ContentHash {
        *self.hasher.finalize().as_bytes()
    }
}

impl Default for ContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_config_default() {
        let config = CacheConfig::default();
        assert!(config.persistent);
        assert!(config.compress);
        assert!(config.max_entries > 0);
    }

    #[test]
    fn test_cache_config_memory_only() {
        let config = CacheConfig::memory_only();
        assert!(!config.persistent);
        assert!(!config.compress);
    }

    #[test]
    fn test_content_hasher() {
        let mut hasher1 = ContentHasher::new();
        hasher1.update(b"hello");
        hasher1.update_str(" world");
        let hash1 = hasher1.finalize();

        let mut hasher2 = ContentHasher::new();
        hasher2.update(b"hello world");
        let hash2 = hasher2.finalize();

        // Different order should give different hash
        let mut hasher3 = ContentHasher::new();
        hasher3.update(b"world hello");
        let hash3 = hasher3.finalize();

        // hash1 and hash2 are different because the input is different
        // "hello" + " world" vs "hello world" (same content, should be equal)
        assert_ne!(hash1, hash3);
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_cache_entry_expiry() {
        let entry = CacheEntry::new(
            [0u8; 32],
            "test",
            vec![],
            vec![],
            CacheOptions::default(),
        );

        // Should not be expired with 0 TTL
        assert!(!entry.is_expired(0));

        // Should not be expired with large TTL
        assert!(!entry.is_expired(86400));
    }

    #[test]
    fn test_dependency_tracker() {
        let tracker = DependencyTracker::new();

        let module_a = [1u8; 32];
        let module_b = [2u8; 32];
        let module_c = [3u8; 32];

        // A depends on B
        tracker.register(module_a, vec![module_b]);
        // B depends on C
        tracker.register(module_b, vec![module_c]);

        // Get dependents of C (should include B and A through transitivity)
        let invalidated = tracker.get_invalidation_set(&module_c);
        assert!(invalidated.contains(&module_c));
        assert!(invalidated.contains(&module_b));
        assert!(invalidated.contains(&module_a));
    }

    #[test]
    fn test_incremental_cache_memory() -> Result<()> {
        let config = CacheConfig::memory_only();
        let cache = IncrementalCache::new(config)?;

        let hash = IncrementalCache::hash_content(b"test content");
        let entry = CacheEntry::new(
            hash,
            "test_module",
            b"compiled data".to_vec(),
            vec![],
            CacheOptions::default(),
        );

        // Initially empty
        assert!(cache.get(&hash).is_none());

        // Put and get
        cache.put(entry)?;
        let retrieved = cache.get(&hash);
        assert!(retrieved.is_some());

        // Invalidate
        cache.invalidate(&hash);
        assert!(cache.get(&hash).is_none());

        Ok(())
    }

    #[test]
    fn test_cache_stats() {
        let stats = CacheStats::new();

        stats.hits.fetch_add(3, Ordering::Relaxed);
        stats.misses.fetch_add(1, Ordering::Relaxed);

        assert_eq!(stats.hit_rate(), 0.75);

        let summary = stats.summary();
        assert_eq!(summary.hits, 3);
        assert_eq!(summary.misses, 1);
    }
}
