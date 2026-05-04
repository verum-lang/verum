//! Incremental Meta Evaluation Cache
//!

//! This module implements a content-addressed LRU cache for meta-programming
//! evaluation results, providing 2-10x speedup for incremental builds by
//! avoiding redundant meta function executions.
//!

//! ## Cached Items
//!

//! - Meta function call results (pure functions only)
//! - Builtin function call results (for deterministic builtins)
//! - Type definition lookups
//! - AST-to-MetaExpr conversion results
//!

//! ## Cache Invalidation
//!

//! The cache uses content hashing for automatic invalidation:
//! - Source file changes invalidate dependent entries
//! - Type definition changes invalidate type lookup entries
//! - All entries expire after a configurable TTL
//!

//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::hash::ContentHash;

use verum_ast::MetaValue;
use verum_ast::expr::Expr;
use verum_common::{List, Map, Maybe, Text};

// ==================== Cache Key Types ====================

/// Cache key for meta function calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetaCallKey {
    /// Hash of function name
    function_hash: u64,
    /// Hash of arguments
    args_hash: u64,
}

impl MetaCallKey {
    /// Create a cache key for a meta function call.
    pub fn new(function_name: &Text, args: &[MetaValue]) -> Self {
        Self {
            function_hash: hash_text(function_name),
            args_hash: hash_args(args),
        }
    }
}

/// Cache key for builtin function calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinCallKey {
    /// Hash of builtin name
    builtin_hash: u64,
    /// Hash of arguments
    args_hash: u64,
}

impl BuiltinCallKey {
    /// Create a cache key for a builtin function call.
    pub fn new(builtin_name: &Text, args: &[MetaValue]) -> Self {
        Self {
            builtin_hash: hash_text(builtin_name),
            args_hash: hash_args(args),
        }
    }
}

/// Cache key for type lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeLookupKey {
    /// Hash of type name
    type_name_hash: u64,
}

impl TypeLookupKey {
    /// Create a cache key for a type lookup.
    pub fn new(type_name: &Text) -> Self {
        Self {
            type_name_hash: hash_text(type_name),
        }
    }
}

/// Cache key for AST-to-MetaExpr conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprConversionKey {
    /// Hash of expression
    expr_hash: u64,
}

impl ExprConversionKey {
    /// Create a cache key for an expression conversion.
    pub fn new(expr: &Expr) -> Self {
        Self {
            expr_hash: hash_expr(expr),
        }
    }
}

// ==================== Hashing Functions ====================

/// Hash a text value for cache keys using Blake3.
fn hash_text(text: &Text) -> u64 {
    let mut hasher = ContentHash::new();
    hasher.update_str(text.as_str());
    hasher.finalize().to_u64()
}

/// Hash a list of MetaValue arguments using Blake3.
fn hash_args(args: &[MetaValue]) -> u64 {
    let mut hasher = ContentHash::new();
    for arg in args {
        // Encode the hash of each argument as bytes
        let arg_hash = hash_meta_value(arg);
        hasher.update(&arg_hash.to_le_bytes());
    }
    hasher.finalize().to_u64()
}

/// Hash a MetaValue for cache key generation using Blake3.
///

/// MetaValue variants (from verum_ast/src/meta_value.rs):
/// - Primitives: Unit, Bool, Int, UInt, Float, Char, Text, Bytes
/// - Collections: Array, Tuple, Maybe
/// - AST: Expr, Type, Pattern, Item, Items
fn hash_meta_value(value: &MetaValue) -> u64 {
    let mut hasher = ContentHash::new();
    match value {
        MetaValue::Unit => {
            hasher.update_str("unit");
        }
        MetaValue::Bool(b) => {
            hasher.update_str("bool:");
            hasher.update(if *b { b"1" } else { b"0" });
        }
        MetaValue::Int(i) => {
            hasher.update_str("int:");
            hasher.update(&i.to_le_bytes());
        }
        MetaValue::UInt(u) => {
            hasher.update_str("uint:");
            hasher.update(&u.to_le_bytes());
        }
        MetaValue::Float(f) => {
            hasher.update_str("float:");
            hasher.update(&f.to_bits().to_le_bytes());
        }
        MetaValue::Char(c) => {
            hasher.update_str("char:");
            hasher.update(&(*c as u32).to_le_bytes());
        }
        MetaValue::Text(t) => {
            hasher.update_str("text:");
            hasher.update_str(t.as_str());
        }
        MetaValue::Bytes(b) => {
            hasher.update_str("bytes:");
            hasher.update(b);
        }
        MetaValue::Array(items) => {
            hasher.update_str("array:");
            hasher.update(&items.len().to_le_bytes());
            for item in items.iter() {
                hasher.update(&hash_meta_value(item).to_le_bytes());
            }
        }
        MetaValue::Tuple(items) => {
            hasher.update_str("tuple:");
            hasher.update(&items.len().to_le_bytes());
            for item in items.iter() {
                hasher.update(&hash_meta_value(item).to_le_bytes());
            }
        }
        MetaValue::Maybe(m) => {
            hasher.update_str("maybe:");
            match m.as_ref() {
                Maybe::Some(v) => {
                    hasher.update_str("some:");
                    hasher.update(&hash_meta_value(v).to_le_bytes());
                }
                Maybe::None => {
                    hasher.update_str("none");
                }
            }
        }
        MetaValue::Map(map) => {
            hasher.update_str("map:");
            hasher.update(&map.len().to_le_bytes());
            for (k, v) in map.iter() {
                hasher.update_str(k.as_str());
                hasher.update(b"\x00");
                hasher.update(&hash_meta_value(v).to_le_bytes());
            }
        }
        MetaValue::Set(set) => {
            hasher.update_str("set:");
            hasher.update(&set.len().to_le_bytes());
            for item in set.iter() {
                hasher.update_str(item.as_str());
                hasher.update(b"\x00");
            }
        }
        // AST variants - hash their debug representation
        MetaValue::Expr(e) => {
            hasher.update_str("expr:");
            hasher.update(&hash_expr(e).to_le_bytes());
        }
        MetaValue::Type(ty) => {
            hasher.update_str("type:");
            hasher.update_str(&format!("{:?}", ty));
        }
        MetaValue::Pattern(p) => {
            hasher.update_str("pattern:");
            hasher.update_str(&format!("{:?}", p));
        }
        MetaValue::Item(item) => {
            hasher.update_str("item:");
            hasher.update_str(&format!("{:?}", item));
        }
        MetaValue::Items(items) => {
            hasher.update_str("items:");
            hasher.update(&items.len().to_le_bytes());
            for item in items.iter() {
                hasher.update_str(&format!("{:?}", item));
                hasher.update(b"\x00");
            }
        }
    }
    hasher.finalize().to_u64()
}

/// Hash an expression for cache key generation using Blake3.
fn hash_expr(expr: &Expr) -> u64 {
    let mut hasher = ContentHash::new();
    // Use structural representation
    let structural = structural_hash_expr(expr);
    hasher.update_str(structural.as_str());
    hasher.finalize().to_u64()
}

/// Compute structural hash string of expression.
fn structural_hash_expr(expr: &Expr) -> Text {
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::LiteralKind;

    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Text::from(format!("bool:{}", b)),
            LiteralKind::Int(i) => Text::from(format!("int:{}", i.value)),
            LiteralKind::Float(f) => Text::from(format!("float:{}", f.value)),
            LiteralKind::Char(c) => Text::from(format!("char:{}", c)),
            LiteralKind::Text(s) => Text::from(format!("str:{}", s.as_str())),
            _ => Text::from(format!("lit:{:?}", lit.kind)),
        },
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                Text::from(format!("var:{}", ident.as_str()))
            } else {
                Text::from(format!("path:{:?}", path))
            }
        }
        ExprKind::Binary { op, left, right } => {
            let left_hash = structural_hash_expr(left);
            let right_hash = structural_hash_expr(right);
            Text::from(format!(
                "bin:{}:[{},{}]",
                op.as_str(),
                left_hash,
                right_hash
            ))
        }
        ExprKind::Unary { op, expr: inner } => {
            Text::from(format!("un:{:?}:{}", op, structural_hash_expr(inner)))
        }
        ExprKind::Call { func, args, .. } => {
            let func_hash = structural_hash_expr(func);
            let args_hash: List<_> = args.iter().map(structural_hash_expr).collect();
            Text::from(format!("call:{}:[{}]", func_hash, args_hash.join(",")))
        }
        ExprKind::Paren(inner) => structural_hash_expr(inner),
        _ => Text::from(format!("expr:{:?}", std::mem::discriminant(&expr.kind))),
    }
}

// ==================== Cache Entry ====================

/// A cached evaluation result.
#[derive(Debug, Clone)]
struct CachedEntry<V> {
    /// The cached value
    value: V,
    /// When this entry was created
    timestamp: Instant,
    /// Number of cache hits
    hit_count: u64,
    /// Content hash for invalidation
    content_hash: u64,
}

// ==================== Cache Statistics ====================

/// Statistics about cache usage.
#[derive(Debug, Clone, Default)]
pub struct MetaCacheStats {
    /// Total cache hits
    pub hits: u64,
    /// Total cache misses
    pub misses: u64,
    /// Number of evictions
    pub evictions: u64,
    /// Number of invalidations
    pub invalidations: u64,
    /// Current cache size (entries)
    pub current_size: usize,
    /// Maximum cache size (entries)
    pub max_size: usize,
}

impl MetaCacheStats {
    /// Calculate cache hit rate.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// ==================== Meta Evaluation Cache ====================

/// Cache for meta function call results.
struct MetaCallCacheInner {
    entries: Map<MetaCallKey, CachedEntry<MetaValue>>,
    max_size: usize,
    ttl: Duration,
    stats: MetaCacheStats,
}

impl MetaCallCacheInner {
    fn new(max_size: usize, ttl: Duration) -> Self {
        Self {
            entries: Map::with_capacity(max_size),
            max_size,
            ttl,
            stats: MetaCacheStats {
                max_size,
                ..Default::default()
            },
        }
    }

    fn get(&mut self, key: &MetaCallKey) -> Maybe<MetaValue> {
        if let Maybe::Some(entry) = self.entries.get_mut(key) {
            // Check TTL
            if entry.timestamp.elapsed() > self.ttl {
                self.entries.remove(key);
                self.stats.invalidations += 1;
                self.stats.misses += 1;
                self.stats.current_size = self.entries.len();
                return Maybe::None;
            }
            entry.hit_count += 1;
            self.stats.hits += 1;
            Maybe::Some(entry.value.clone())
        } else {
            self.stats.misses += 1;
            Maybe::None
        }
    }

    fn insert(&mut self, key: MetaCallKey, value: MetaValue, content_hash: u64) {
        if self.entries.len() >= self.max_size {
            self.evict();
        }
        self.entries.insert(
            key,
            CachedEntry {
                value,
                timestamp: Instant::now(),
                hit_count: 0,
                content_hash,
            },
        );
        self.stats.current_size = self.entries.len();
    }

    fn evict(&mut self) {
        // LRU eviction: remove oldest 10% of entries
        let num_to_remove = (self.max_size / 10).max(1);
        let mut entries: List<_> = self.entries.iter().collect();
        entries.sort_by_key(|(_, entry)| (entry.hit_count, entry.timestamp));
        let keys_to_remove: List<_> = entries
            .iter()
            .take(num_to_remove)
            .map(|(key, _)| **key)
            .collect();
        for key in keys_to_remove {
            self.entries.remove(&key);
        }
        self.stats.evictions += num_to_remove as u64;
        self.stats.current_size = self.entries.len();
    }

    fn invalidate_by_content_hash(&mut self, content_hash: u64) {
        let keys_to_remove: List<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.content_hash == content_hash)
            .map(|(key, _)| *key)
            .collect();
        for key in keys_to_remove.iter() {
            self.entries.remove(key);
            self.stats.invalidations += 1;
        }
        self.stats.current_size = self.entries.len();
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.stats.current_size = 0;
    }
}

/// Cache for builtin function call results.
struct BuiltinCallCacheInner {
    entries: Map<BuiltinCallKey, CachedEntry<MetaValue>>,
    max_size: usize,
    stats: MetaCacheStats,
}

impl BuiltinCallCacheInner {
    fn new(max_size: usize) -> Self {
        Self {
            entries: Map::with_capacity(max_size),
            max_size,
            stats: MetaCacheStats {
                max_size,
                ..Default::default()
            },
        }
    }

    fn get(&mut self, key: &BuiltinCallKey) -> Maybe<MetaValue> {
        if let Maybe::Some(entry) = self.entries.get_mut(key) {
            entry.hit_count += 1;
            self.stats.hits += 1;
            Maybe::Some(entry.value.clone())
        } else {
            self.stats.misses += 1;
            Maybe::None
        }
    }

    fn insert(&mut self, key: BuiltinCallKey, value: MetaValue) {
        if self.entries.len() >= self.max_size {
            self.evict();
        }
        self.entries.insert(
            key,
            CachedEntry {
                value,
                timestamp: Instant::now(),
                hit_count: 0,
                content_hash: 0,
            },
        );
        self.stats.current_size = self.entries.len();
    }

    fn evict(&mut self) {
        let num_to_remove = (self.max_size / 10).max(1);
        let mut entries: List<_> = self.entries.iter().collect();
        entries.sort_by_key(|(_, entry)| (entry.hit_count, entry.timestamp));
        let keys_to_remove: List<_> = entries
            .iter()
            .take(num_to_remove)
            .map(|(key, _)| **key)
            .collect();
        for key in keys_to_remove {
            self.entries.remove(&key);
        }
        self.stats.evictions += num_to_remove as u64;
        self.stats.current_size = self.entries.len();
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.stats.current_size = 0;
    }
}

// ==================== Main Cache Interface ====================

/// Configuration for the meta evaluation cache.
#[derive(Debug, Clone)]
pub struct MetaCacheConfig {
    /// Maximum entries in meta function call cache
    pub meta_call_max_size: usize,
    /// Maximum entries in builtin call cache
    pub builtin_call_max_size: usize,
    /// Time-to-live for meta function call results
    pub meta_call_ttl: Duration,
    /// Enable caching (can be disabled for debugging)
    pub enabled: bool,
}

impl Default for MetaCacheConfig {
    fn default() -> Self {
        Self {
            meta_call_max_size: 10_000,
            builtin_call_max_size: 50_000,
            meta_call_ttl: Duration::from_secs(300), // 5 minutes
            enabled: true,
        }
    }
}

/// Thread-safe incremental meta evaluation cache.
///

/// Caches results of pure meta function calls and deterministic builtin
/// function calls to speed up incremental compilation.
///
/// Optionally backed by a filesystem persistence layer
/// ([`crate::meta::persistence::MetaCachePersistence`]) — Phase 9 of
/// the precompiled-stdlib epic — which preserves the primitive subset
/// of `MetaValue` results across compiler runs. Configure via
/// [`MetaEvalCache::with_persistence`].
pub struct MetaEvalCache {
    /// Meta function call cache
    meta_calls: Arc<RwLock<MetaCallCacheInner>>,
    /// Builtin call cache
    builtin_calls: Arc<RwLock<BuiltinCallCacheInner>>,
    /// Whether caching is enabled
    enabled: bool,
    /// Optional cross-run filesystem persistence layer for the
    /// meta-call cache.  When `Some`, FS reads back-fill in-memory
    /// misses and FS writes happen alongside in-memory inserts.
    persistence: Option<Arc<crate::meta::persistence::MetaCachePersistence>>,
}

impl MetaEvalCache {
    /// Create a new meta evaluation cache with default configuration.
    pub fn new() -> Self {
        Self::with_config(MetaCacheConfig::default())
    }

    /// Create a new meta evaluation cache with custom configuration.
    pub fn with_config(config: MetaCacheConfig) -> Self {
        Self {
            meta_calls: Arc::new(RwLock::new(MetaCallCacheInner::new(
                config.meta_call_max_size,
                config.meta_call_ttl,
            ))),
            builtin_calls: Arc::new(RwLock::new(BuiltinCallCacheInner::new(
                config.builtin_call_max_size,
            ))),
            enabled: config.enabled,
            persistence: None,
        }
    }

    /// Attach a cross-run filesystem persistence layer.  Reads use it
    /// to back-fill in-memory misses; writes go through to disk
    /// alongside in-memory inserts.  Phase 9 V0 — primitive
    /// `MetaValue` subset only; AST-bearing variants stay in-memory
    /// only.
    ///
    /// Use this in conjunction with
    /// [`get_meta_call_with_source_hash`](Self::get_meta_call_with_source_hash)
    /// — the plain [`get_meta_call`](Self::get_meta_call) path bypasses
    /// the persistence layer because it has no source_hash to validate
    /// against.
    pub fn with_persistence(
        mut self,
        persistence: Arc<crate::meta::persistence::MetaCachePersistence>,
    ) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Look up a cached meta function call result.
    pub fn get_meta_call(&self, function_name: &Text, args: &[MetaValue]) -> Maybe<MetaValue> {
        if !self.enabled {
            return Maybe::None;
        }
        let key = MetaCallKey::new(function_name, args);
        match self.meta_calls.write() {
            Ok(mut cache) => cache.get(&key),
            Err(_) => Maybe::None,
        }
    }

    /// Cache a meta function call result.
    pub fn insert_meta_call(
        &self,
        function_name: &Text,
        args: &[MetaValue],
        result: MetaValue,
        source_hash: u64,
    ) {
        if !self.enabled {
            return;
        }
        let key = MetaCallKey::new(function_name, args);
        // Write to filesystem persistence (best-effort) BEFORE
        // moving `result` into the in-memory cache.  Skipped for
        // AST-bearing MetaValue variants by `MetaCachePersistence::put`.
        if let Some(p) = self.persistence.as_ref() {
            p.put(
                function_name.as_str(),
                key.function_hash,
                key.args_hash,
                source_hash,
                &result,
            );
        }
        if let Ok(mut cache) = self.meta_calls.write() {
            cache.insert(key, result, source_hash);
        }
    }

    /// Source-hash-aware lookup that consults the cross-run
    /// filesystem persistence layer on in-memory miss.  Returns
    /// `Maybe::Some(value)` for in-memory hit, on-disk hit (with
    /// matching source hash), or `Maybe::None` otherwise.
    ///
    /// On disk hit, the value is also re-populated into the in-memory
    /// cache so subsequent lookups within the same run hit the fast
    /// path.
    pub fn get_meta_call_with_source_hash(
        &self,
        function_name: &Text,
        args: &[MetaValue],
        source_hash: u64,
    ) -> Maybe<MetaValue> {
        if !self.enabled {
            return Maybe::None;
        }
        let key = MetaCallKey::new(function_name, args);
        // 1. In-memory fast path.
        if let Ok(mut cache) = self.meta_calls.write() {
            if let Maybe::Some(value) = cache.get(&key) {
                return Maybe::Some(value);
            }
        }
        // 2. Filesystem fallback.
        let persistence = match self.persistence.as_ref() {
            Some(p) => p,
            None => return Maybe::None,
        };
        let value = match persistence.get(key.function_hash, key.args_hash, source_hash) {
            Some(v) => v,
            None => return Maybe::None,
        };
        // 3. Back-fill in-memory cache so the second lookup of the
        //    same call within this run takes the hot path.
        if let Ok(mut cache) = self.meta_calls.write() {
            cache.insert(key, value.clone(), source_hash);
        }
        Maybe::Some(value)
    }

    /// Look up a cached builtin function call result.
    pub fn get_builtin_call(&self, builtin_name: &Text, args: &[MetaValue]) -> Maybe<MetaValue> {
        if !self.enabled {
            return Maybe::None;
        }
        let key = BuiltinCallKey::new(builtin_name, args);
        match self.builtin_calls.write() {
            Ok(mut cache) => cache.get(&key),
            Err(_) => Maybe::None,
        }
    }

    /// Cache a builtin function call result.
    ///

    /// Only deterministic builtins should be cached. Non-deterministic
    /// builtins (e.g., current_time, random) should NOT be cached.
    pub fn insert_builtin_call(&self, builtin_name: &Text, args: &[MetaValue], result: MetaValue) {
        if !self.enabled {
            return;
        }
        let key = BuiltinCallKey::new(builtin_name, args);
        if let Ok(mut cache) = self.builtin_calls.write() {
            cache.insert(key, result);
        }
    }

    /// Invalidate all cache entries that depend on a specific source file.
    pub fn invalidate_by_source(&self, source_hash: u64) {
        if let Ok(mut cache) = self.meta_calls.write() {
            cache.invalidate_by_content_hash(source_hash);
        }
    }

    /// Clear all caches.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.meta_calls.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.builtin_calls.write() {
            cache.clear();
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> MetaCacheStats {
        let mut stats = MetaCacheStats::default();

        if let Ok(meta_cache) = self.meta_calls.read() {
            stats.hits += meta_cache.stats.hits;
            stats.misses += meta_cache.stats.misses;
            stats.evictions += meta_cache.stats.evictions;
            stats.invalidations += meta_cache.stats.invalidations;
            stats.current_size += meta_cache.stats.current_size;
            stats.max_size += meta_cache.stats.max_size;
        }

        if let Ok(builtin_cache) = self.builtin_calls.read() {
            stats.hits += builtin_cache.stats.hits;
            stats.misses += builtin_cache.stats.misses;
            stats.evictions += builtin_cache.stats.evictions;
            stats.invalidations += builtin_cache.stats.invalidations;
            stats.current_size += builtin_cache.stats.current_size;
            stats.max_size += builtin_cache.stats.max_size;
        }

        stats
    }

    /// Check if a builtin is deterministic and should be cached.
    ///

    /// Non-deterministic builtins are NOT cached:
    /// - Time-related: current_time, timestamp, etc.
    /// - Random: random_int, random_float, etc.
    /// - Environment: env_var (may change between compilations)
    pub fn is_cacheable_builtin(name: &str) -> bool {
        !matches!(
            name,
            "current_time"
                | "timestamp"
                | "now"
                | "random_int"
                | "random_float"
                | "random"
                | "env_var"
                | "get_env"
                | "uuid"
                | "generate_id"
        )
    }
}

impl Default for MetaEvalCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MetaEvalCache {
    fn clone(&self) -> Self {
        Self {
            meta_calls: Arc::clone(&self.meta_calls),
            builtin_calls: Arc::clone(&self.builtin_calls),
            enabled: self.enabled,
            persistence: self.persistence.as_ref().map(Arc::clone),
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_cache_basic() {
        let cache = MetaEvalCache::new();

        let func_name = Text::from("test_func");
        let args = vec![MetaValue::Int(42)];
        let result = MetaValue::Text(Text::from("result"));

        // Initially empty
        assert!(cache.get_meta_call(&func_name, &args).is_none());

        // Insert and retrieve
        cache.insert_meta_call(&func_name, &args, result.clone(), 0);
        let cached = cache.get_meta_call(&func_name, &args);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap(), result);
    }

    /// Phase 9 V0: cross-run filesystem persistence. Two separate
    /// `MetaEvalCache` instances pointing at the same on-disk
    /// directory simulate two compiler runs.  The first writes; the
    /// second reads back (in-memory empty, FS hit).
    #[test]
    fn fs_persistence_survives_cache_drop() {
        use crate::meta::persistence::MetaCachePersistence;

        let dir = tempfile::tempdir().unwrap();
        let persistence = Arc::new(MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "compiler-test-1.0.0".to_string(),
        ));
        let func_name = Text::from("compute_layout");
        let args = vec![MetaValue::Int(64)];
        let result = MetaValue::Int(128);
        let source_hash: u64 = 0xdead_beef;

        // Run #1: insert.
        let cache1 = MetaEvalCache::new().with_persistence(Arc::clone(&persistence));
        cache1.insert_meta_call(&func_name, &args, result.clone(), source_hash);

        // Run #2: fresh cache, same persistence layer.  In-memory
        // is empty; FS fallback should hit.
        drop(cache1);
        let cache2 = MetaEvalCache::new().with_persistence(persistence);
        let got = cache2.get_meta_call_with_source_hash(&func_name, &args, source_hash);
        assert!(got.is_some(), "FS fallback should hit on second run");
        assert_eq!(got.unwrap(), result);

        // Subsequent same-key lookup hits in-memory directly (back-fill).
        let got2 = cache2.get_meta_call(&func_name, &args);
        assert!(got2.is_some(), "in-memory back-fill should serve repeat lookup");
    }

    /// Stale source_hash on the FS layer must miss — the cache
    /// can't return a value computed against an obsolete source.
    #[test]
    fn fs_persistence_stale_source_hash_misses() {
        use crate::meta::persistence::MetaCachePersistence;

        let dir = tempfile::tempdir().unwrap();
        let persistence = Arc::new(MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "compiler-test-1.0.0".to_string(),
        ));
        let func_name = Text::from("f");
        let args = vec![MetaValue::Int(1)];

        let cache1 = MetaEvalCache::new().with_persistence(Arc::clone(&persistence));
        cache1.insert_meta_call(&func_name, &args, MetaValue::Int(99), 1);

        let cache2 = MetaEvalCache::new().with_persistence(persistence);
        // Different source hash — source has been edited.
        assert!(
            cache2
                .get_meta_call_with_source_hash(&func_name, &args, 2)
                .is_none(),
            "stale source_hash must miss"
        );
    }

    #[test]
    fn test_builtin_cache_basic() {
        let cache = MetaEvalCache::new();

        let builtin_name = Text::from("list_len");
        let args = vec![MetaValue::Array(List::from_iter([
            MetaValue::Int(1),
            MetaValue::Int(2),
        ]))];
        let result = MetaValue::Int(2);

        // Initially empty
        assert!(cache.get_builtin_call(&builtin_name, &args).is_none());

        // Insert and retrieve
        cache.insert_builtin_call(&builtin_name, &args, result.clone());
        let cached = cache.get_builtin_call(&builtin_name, &args);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap(), result);
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = MetaEvalCache::new();

        let func_name = Text::from("test_func");
        let args = vec![MetaValue::Int(1)];
        let result = MetaValue::Bool(true);
        let source_hash = 12345u64;

        cache.insert_meta_call(&func_name, &args, result.clone(), source_hash);
        assert!(cache.get_meta_call(&func_name, &args).is_some());

        // Invalidate by source hash
        cache.invalidate_by_source(source_hash);
        assert!(cache.get_meta_call(&func_name, &args).is_none());
    }

    #[test]
    fn test_cacheable_builtin_detection() {
        assert!(MetaEvalCache::is_cacheable_builtin("list_len"));
        assert!(MetaEvalCache::is_cacheable_builtin("text_concat"));
        assert!(MetaEvalCache::is_cacheable_builtin("type_name"));

        assert!(!MetaEvalCache::is_cacheable_builtin("current_time"));
        assert!(!MetaEvalCache::is_cacheable_builtin("random_int"));
        assert!(!MetaEvalCache::is_cacheable_builtin("env_var"));
    }

    #[test]
    fn test_cache_stats() {
        let cache = MetaEvalCache::new();

        let func_name = Text::from("test");
        let args = vec![MetaValue::Int(1)];

        // Miss
        cache.get_meta_call(&func_name, &args);

        // Insert
        cache.insert_meta_call(&func_name, &args, MetaValue::Bool(true), 0);

        // Hit
        cache.get_meta_call(&func_name, &args);
        cache.get_meta_call(&func_name, &args);

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_disabled_cache() {
        let config = MetaCacheConfig {
            enabled: false,
            ..Default::default()
        };
        let cache = MetaEvalCache::with_config(config);

        let func_name = Text::from("test");
        let args = vec![MetaValue::Int(1)];

        cache.insert_meta_call(&func_name, &args, MetaValue::Bool(true), 0);
        assert!(cache.get_meta_call(&func_name, &args).is_none());
    }
}
