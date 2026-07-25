//! Module caching for performance.
//!
//! Caches parsed modules to avoid re-parsing unchanged files.

use crate::ModuleInfo;
use crate::path::ModuleId;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use verum_common::Maybe;

/// A cached module entry.
#[derive(Debug, Clone)]
pub struct ModuleCacheEntry {
    /// The cached module
    pub module: Arc<ModuleInfo>,
    /// File modification time when cached
    pub mtime: SystemTime,
    /// Hash of the source code
    pub source_hash: u64,
}

impl ModuleCacheEntry {
    pub fn new(module: ModuleInfo, mtime: SystemTime, source_hash: u64) -> Self {
        Self {
            module: Arc::new(module),
            mtime,
            source_hash,
        }
    }

    /// Check if this cache entry is still valid.
    pub fn is_valid(&self, current_mtime: SystemTime, current_hash: u64) -> bool {
        self.mtime == current_mtime && self.source_hash == current_hash
    }
}

/// Module cache - stores parsed modules for reuse.
///
/// The cache is thread-safe and can be shared across multiple threads.
/// It uses file modification times and content hashes to detect changes.
#[derive(Debug)]
pub struct ModuleCache {
    /// Cached modules by file path
    by_path: DashMap<PathBuf, ModuleCacheEntry>,
    /// Cached modules by module ID
    by_id: DashMap<ModuleId, ModuleCacheEntry>,
    /// Cache statistics
    hits: Arc<std::sync::atomic::AtomicU64>,
    misses: Arc<std::sync::atomic::AtomicU64>,
}

impl ModuleCache {
    pub fn new() -> Self {
        Self {
            by_path: DashMap::new(),
            by_id: DashMap::new(),
            hits: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            misses: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Insert a module into the cache.
    pub fn insert(&self, file_path: PathBuf, entry: ModuleCacheEntry) {
        let module_id = entry.module.id;
        self.by_path.insert(file_path, entry.clone());
        self.by_id.insert(module_id, entry);
    }

    /// Get a cached module by file path.
    pub fn get_by_path(&self, file_path: &PathBuf) -> Maybe<Arc<ModuleInfo>> {
        if let Some(entry) = self.by_path.get(file_path) {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Maybe::Some(entry.module.clone())
        } else {
            self.misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Maybe::None
        }
    }

    /// Get a cached module by module ID.
    pub fn get_by_id(&self, module_id: ModuleId) -> Maybe<Arc<ModuleInfo>> {
        if let Some(entry) = self.by_id.get(&module_id) {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Maybe::Some(entry.module.clone())
        } else {
            self.misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Maybe::None
        }
    }

    /// Check if a module is cached and still valid.
    pub fn is_valid(&self, file_path: &PathBuf, mtime: SystemTime, hash: u64) -> bool {
        if let Some(entry) = self.by_path.get(file_path) {
            entry.is_valid(mtime, hash)
        } else {
            false
        }
    }

    /// Remove a module from the cache.
    pub fn remove(&self, file_path: &PathBuf) -> Maybe<ModuleCacheEntry> {
        self.by_path
            .remove(file_path)
            .map(|(_, entry)| {
                self.by_id.remove(&entry.module.id);
                Maybe::Some(entry)
            })
            .unwrap_or(Maybe::None)
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        self.by_path.clear();
        self.by_id.clear();
        self.hits.store(0, std::sync::atomic::Ordering::Relaxed);
        self.misses.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get the number of cached modules.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let hits = self.hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.misses.load(std::sync::atomic::Ordering::Relaxed);
        CacheStats { hits, misses }
    }

    /// Compute a simple hash of source code.
    pub fn hash_source(source: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for ModuleCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    /// Get total number of lookups.
    pub fn total(&self) -> u64 {
        self.hits + self.misses
    }
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cache stats: {} hits, {} misses, {:.1}% hit rate",
            self.hits,
            self.misses,
            self.hit_rate()
        )
    }
}
