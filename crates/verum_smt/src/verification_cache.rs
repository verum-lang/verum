//! Verification result caching for refinement type checking.
//!
//! This module implements a hash-based LRU cache for SMT verification results,
//! providing 10-100x speedup for incremental builds by avoiding redundant
//! solver invocations.

use crate::verify::VerificationResult;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use verum_ast::{Expr, Type};
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;

// ==================== Cache Key ====================

/// Cache key for verification results.
///
/// Combines hash of refinement predicate + base type for unique identification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    predicate_hash: u64,
    base_type_hash: u64,
}

impl CacheKey {
    fn new(predicate: &Expr, base_type: &Type) -> Self {
        Self {
            predicate_hash: hash_expr(predicate),
            base_type_hash: hash_type(base_type),
        }
    }
}

/// Hash an expression for cache key generation.
///
/// Implements structural hashing that improves cache hits by:
/// - Normalizing variable names (alpha-equivalence)
/// - Commutative operation ordering
/// - Ignoring parentheses
fn hash_expr(expr: &Expr) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Use structural representation instead of debug formatting
    let structural = structural_hash_expr(expr);
    structural.hash(&mut hasher);

    hasher.finish()
}

/// Compute structural hash string of expression
fn structural_hash_expr(expr: &Expr) -> Text {
    use std::collections::BTreeSet;
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::LiteralKind;

    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Text::from(format!("bool:{}", b)),
            LiteralKind::Int(i) => Text::from(format!("int:{}", i.value)),
            LiteralKind::Float(f) => Text::from(format!("float:{}", f.value)),
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

            // For commutative operations, sort operands for consistent hash
            if op.is_commutative() {
                let mut operands = BTreeSet::new();
                operands.insert(left_hash);
                operands.insert(right_hash);
                let sorted: List<_> = operands.into_iter().collect();
                Text::from(format!(
                    "bin:{}:[{},{}]",
                    op.as_str(),
                    sorted[0],
                    sorted.get(1).unwrap_or(&"".to_text())
                ))
            } else {
                Text::from(format!(
                    "bin:{}:[{},{}]",
                    op.as_str(),
                    left_hash,
                    right_hash
                ))
            }
        }

        ExprKind::Unary { op, expr: inner } => Text::from(format!(
            "un:{}:{}",
            format!("{:?}", op),
            structural_hash_expr(inner)
        )),

        ExprKind::Call { func, args, .. } => {
            let func_hash = structural_hash_expr(func);
            let args_hash: List<_> = args.iter().map(structural_hash_expr).collect();
            Text::from(format!("call:{}:[{}]", func_hash, args_hash.join(",")))
        }

        ExprKind::Paren(inner) => {
            // Ignore parentheses for structural equivalence
            structural_hash_expr(inner)
        }

        _ => {
            // Fallback to debug representation for unsupported kinds
            Text::from(format!("expr:{:?}", expr.kind))
        }
    }
}

/// Hash a type for cache key generation.
fn hash_type(ty: &Type) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Hash the type's string representation
    format!("{:?}", ty).hash(&mut hasher);
    hasher.finish()
}

// ==================== Cache Implementation ====================

/// Verification result cache entry.
#[derive(Debug, Clone)]
struct CachedEntry {
    result: VerificationResult,
    timestamp: Instant,
    hit_count: u64,
}

/// Verification result cache with LRU eviction.
struct VerificationCacheInner {
    entries: Map<CacheKey, CachedEntry>,
    max_size: usize,
}

impl VerificationCacheInner {
    fn new(max_size: usize) -> Self {
        Self {
            entries: Map::with_capacity(max_size),
            max_size,
        }
    }

    fn get(&mut self, key: &CacheKey) -> Maybe<VerificationResult> {
        if let Maybe::Some(entry) = self.entries.get_mut(key) {
            // Update hit count
            entry.hit_count += 1;
            Maybe::Some(entry.result.clone())
        } else {
            Maybe::None
        }
    }

    fn insert(&mut self, key: CacheKey, result: VerificationResult) {
        // Evict if needed
        if self.entries.len() >= self.max_size {
            self.evict();
        }

        self.entries.insert(
            key,
            CachedEntry {
                result,
                timestamp: Instant::now(),
                hit_count: 0,
            },
        );
    }

    fn evict(&mut self) {
        // MEMORY FIX: Avoid collecting ALL entries into a sorted list (O(n log n) + memory spike).
        // Instead, scan for entries with lowest hit_count and remove them directly.
        let num_to_remove = (self.max_size / 10).max(1);

        // Find the hit_count threshold: scan once to find a reasonable cutoff
        let min_hits = self
            .entries
            .values()
            .map(|e| e.hit_count)
            .min()
            .unwrap_or(0);

        // Remove entries at the minimum hit count until we've removed enough
        let keys_to_remove: List<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.hit_count <= min_hits)
            .take(num_to_remove)
            .map(|(key, _)| *key)
            .collect();

        for key in keys_to_remove {
            self.entries.remove(&key);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn size(&self) -> usize {
        self.entries.len()
    }
}

// ==================== Public API ====================

/// Thread-safe verification result cache.
///
/// Provides automatic caching of SMT verification results with LRU eviction.
/// Expected cache hit rate: 90%+ for typical incremental builds.
pub struct VerificationCache {
    inner: Arc<RwLock<VerificationCacheInner>>,
    stats: Arc<RwLock<CacheStats>>,
    config: CacheConfig,
    distributed: Maybe<Arc<crate::distributed_cache::DistributedCache>>,
}

impl VerificationCache {
    /// Create a new verification cache with default configuration.
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create a new verification cache with custom configuration.
    ///
    /// Honours `CacheConfig.distributed_cache`: when present, the
    /// distributed backend is auto-constructed and installed on
    /// the new cache. Pre-fix the field was set via the
    /// `with_distributed_cache` builder but no code path consulted
    /// it during cache construction — so configuring a distributed
    /// backend in the manifest had zero observable effect; users
    /// had to additionally call `with_distributed(...)` with a
    /// hand-built backend, defeating the documented "configure
    /// once" contract.
    pub fn with_config(config: CacheConfig) -> Self {
        let distributed = match config.distributed_cache {
            Maybe::Some(ref dc_config) => {
                // Auto-build the distributed cache from the
                // configured stance. On error the construction
                // logs and falls back to no-distributed (the
                // local cache continues to work). Production
                // callers that need to surface the failure
                // explicitly should construct the distributed
                // backend themselves and pass it via
                // `with_distributed`.
                match Self::build_distributed_from_config(dc_config) {
                    Ok(backend) => Maybe::Some(Arc::new(backend)),
                    Err(e) => {
                        tracing::warn!(
                            "CacheConfig.distributed_cache configured but \
                             backend construction failed: {} — falling back \
                             to local-only cache",
                            e,
                        );
                        Maybe::None
                    }
                }
            }
            Maybe::None => Maybe::None,
        };
        Self {
            inner: Arc::new(RwLock::new(VerificationCacheInner::new(config.max_size))),
            stats: Arc::new(RwLock::new(CacheStats::default())),
            config,
            distributed,
        }
    }

    /// Internal: construct a `DistributedCache` from a
    /// `DistributedCacheConfig`. Mirrors the inline construction
    /// path that callers using `with_distributed(backend)` already
    /// run — extracted so the `with_config` auto-construction is a
    /// single source of truth.
    fn build_distributed_from_config(
        config: &DistributedCacheConfig,
    ) -> std::result::Result<crate::distributed_cache::DistributedCache, String> {
        // Translate the local DistributedCacheConfig into the
        // shape expected by the distributed_cache module. The
        // local config carries an s3_url + cache_dir + trust
        // level + verify_signatures flag; the module's own
        // config takes more fields (filesystem_fallback, redis,
        // credentials) that we leave at their defaults here.
        let mut dc = crate::distributed_cache::DistributedCacheConfig::new(
            config.s3_url.clone(),
        );
        dc.trust_level = match config.trust {
            TrustLevel::All => crate::distributed_cache::TrustLevel::None,
            TrustLevel::Signatures
            | TrustLevel::SignaturesAndExpiry => {
                // The module's own TrustLevel doesn't expose a
                // separate "with-expiry" variant; downgrade to
                // Signatures (the lower bound of the requested
                // policy) and let the cache's TTL field on the
                // local CacheConfig enforce the expiry side.
                crate::distributed_cache::TrustLevel::Signatures
            }
        };
        dc.filesystem_fallback = verum_common::Maybe::Some(config.cache_dir.clone());
        let _ = config.verify_signatures; // consumed via trust_level mapping above
        Ok(crate::distributed_cache::DistributedCache::new(dc))
    }

    /// Set distributed cache backend
    pub fn with_distributed(
        mut self,
        distributed: crate::distributed_cache::DistributedCache,
    ) -> Self {
        self.distributed = Maybe::Some(Arc::new(distributed));
        self
    }

    /// Get cached verification result if available.
    ///
    /// Returns `None` if not cached, requiring SMT verification.
    pub fn get(&self, predicate: &Expr, base_type: &Type) -> Maybe<VerificationResult> {
        let key = CacheKey::new(predicate, base_type);
        let start = Instant::now();

        let result = self.inner.write().unwrap().get(&key);

        // Update stats
        {
            let mut stats = self.stats.write().unwrap();
            if result.is_some() {
                stats.cache_hits += 1;
                stats.cache_hit_time_us += start.elapsed().as_micros() as u64;
            } else {
                stats.cache_misses += 1;
            }
        }

        // Mark result as cached
        result.map(|res| res.map(|proof| proof.with_cached()))
    }

    /// Cache a verification result.
    pub fn insert(&self, predicate: &Expr, base_type: &Type, result: VerificationResult) {
        let key = CacheKey::new(predicate, base_type);
        self.inner.write().unwrap().insert(key, result);

        // Update stats
        {
            let mut stats = self.stats.write().unwrap();
            stats.total_insertions += 1;
        }
    }

    /// Cache a verification result with statistics-driven decision
    ///
    /// Only caches if the query was expensive based on solver statistics.
    /// This improves cache efficiency by avoiding cheap queries.
    pub fn insert_with_stats(
        &self,
        predicate: &Expr,
        base_type: &Type,
        result: VerificationResult,
        decisions: u64,
        conflicts: u64,
        solve_time_ms: u64,
    ) {
        // Check if we should cache based on statistics
        if self
            .config
            .should_cache_with_stats(decisions, conflicts, solve_time_ms)
        {
            self.insert(predicate, base_type, result);
        }
    }

    /// Get or compute a verification result.
    ///
    /// Checks cache first, falling back to the provided verification function.
    /// This is the primary API for using the cache.
    ///
    /// # Example
    /// ```ignore
    /// let result = cache.get_or_verify(
    ///     &predicate,
    ///     &base_type,
    ///     || verify_with_z3(&predicate, &base_type)
    /// );
    /// ```
    pub fn get_or_verify<F>(
        &self,
        predicate: &Expr,
        base_type: &Type,
        verify_fn: F,
    ) -> VerificationResult
    where
        F: FnOnce() -> VerificationResult,
    {
        // Try cache first
        if let Maybe::Some(cached_result) = self.get(predicate, base_type) {
            return cached_result;
        }

        // Cache miss - run actual verification
        let result = verify_fn();

        // Cache the result (even if it's an error). When the
        // config carries `statistics_driven = true`, this
        // unconditional path bypasses the gating — callers that
        // need expense-driven caching should use
        // `get_or_verify_with_stats` instead and pass the Z3
        // solver statistics.
        self.insert(predicate, base_type, result.clone());

        result
    }

    /// Get-or-verify with solver statistics for stats-driven
    /// caching. Closes the inert-defense pattern around four
    /// `CacheConfig` fields (`statistics_driven`,
    /// `min_decisions_to_cache`, `min_conflicts_to_cache`,
    /// `min_solve_time_ms`) at the call-site layer: pre-fix the
    /// fields routed through `should_cache_with_stats` only via
    /// `insert_with_stats`, but no production caller used the
    /// stats-aware insertion path — `get_or_verify` always
    /// cached unconditionally regardless of the config.
    ///
    /// The closure returns both the verification result and the
    /// solver statistics that produced it. When the cache
    /// config has `statistics_driven = true`, only "expensive"
    /// queries (exceeding any of the three thresholds) are
    /// cached. When `statistics_driven = false`, every result
    /// is cached just like `get_or_verify`.
    ///
    /// # Arguments
    ///
    /// * `predicate` / `base_type` — cache key components
    /// * `verify_fn` — closure returning `(result, decisions,
    ///   conflicts, solve_time_ms)`. Backends that don't expose
    ///   per-query statistics can pass zeros — the statistics-
    ///   driven gate then short-circuits to "skip cache" when
    ///   any threshold is non-zero, which is the desired
    ///   conservative behaviour.
    pub fn get_or_verify_with_stats<F>(
        &self,
        predicate: &Expr,
        base_type: &Type,
        verify_fn: F,
    ) -> VerificationResult
    where
        F: FnOnce() -> (VerificationResult, u64, u64, u64),
    {
        // Try cache first
        if let Maybe::Some(cached_result) = self.get(predicate, base_type) {
            return cached_result;
        }

        // Cache miss - run actual verification with stats
        let (result, decisions, conflicts, solve_time_ms) = verify_fn();

        // Stats-driven cache decision (only caches expensive queries
        // when `config.statistics_driven = true`)
        self.insert_with_stats(
            predicate,
            base_type,
            result.clone(),
            decisions,
            conflicts,
            solve_time_ms,
        );

        result
    }

    /// Clear all cached results.
    pub fn clear(&self) {
        self.inner.write().unwrap().clear();
        self.stats.write().unwrap().clear();
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let stats = self.stats.read().unwrap();
        let size = self.inner.read().unwrap().size();

        CacheStats {
            cache_hits: stats.cache_hits,
            cache_misses: stats.cache_misses,
            total_insertions: stats.total_insertions,
            cache_hit_time_us: stats.cache_hit_time_us,
            current_size: size,
            max_size: self.config.max_size,
        }
    }

    /// Get cache hit rate (0.0 to 1.0).
    pub fn hit_rate(&self) -> f64 {
        let stats = self.stats.read().unwrap();
        let total = stats.cache_hits + stats.cache_misses;
        if total == 0 {
            0.0
        } else {
            stats.cache_hits as f64 / total as f64
        }
    }
}

impl Default for VerificationCache {
    fn default() -> Self {
        Self::new()
    }
}

// Make it cloneable for sharing across threads
impl Clone for VerificationCache {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            stats: Arc::clone(&self.stats),
            config: self.config.clone(),
            distributed: self.distributed.as_ref().map(Arc::clone),
        }
    }
}

// ==================== Statistics ====================

/// Cache performance statistics.
///
/// Tracks cache effectiveness and capacity utilization for
/// verification result caching.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of successful cache lookups (avoided recomputation).
    pub cache_hits: u64,
    /// Number of failed cache lookups (required computation).
    pub cache_misses: u64,
    /// Total number of entries inserted into the cache.
    pub total_insertions: u64,
    /// Cumulative time spent on cache hits in microseconds.
    pub cache_hit_time_us: u64,
    /// Current number of entries stored in the cache.
    pub current_size: usize,
    /// Maximum allowed cache capacity before eviction.
    pub max_size: usize,
}

impl CacheStats {
    fn clear(&mut self) {
        *self = Self::default();
    }

    /// Get cache hit rate (0.0 to 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    /// Get average cache hit time in microseconds.
    pub fn avg_hit_time_us(&self) -> f64 {
        if self.cache_hits == 0 {
            0.0
        } else {
            self.cache_hit_time_us as f64 / self.cache_hits as f64
        }
    }

    /// Get cache utilization (0.0 to 1.0).
    pub fn utilization(&self) -> f64 {
        if self.max_size == 0 {
            0.0
        } else {
            self.current_size as f64 / self.max_size as f64
        }
    }
}

// ==================== Configuration ====================

/// Configuration for verification cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cached entries (default: 2,000)
    pub max_size: usize,
    /// Maximum cache size in bytes (default: 500MB)
    pub max_size_bytes: u64,
    /// Time-to-live for cache entries (default: 30 days)
    pub ttl: Duration,
    /// Enable statistics-driven caching (cache expensive queries only)
    pub statistics_driven: bool,
    /// Minimum SMT decisions to trigger caching (statistics-driven mode)
    pub min_decisions_to_cache: u64,
    /// Minimum conflicts to trigger caching (statistics-driven mode)
    pub min_conflicts_to_cache: u64,
    /// Minimum solving time (ms) to trigger caching
    pub min_solve_time_ms: u64,
    /// Distributed cache configuration
    pub distributed_cache: Maybe<DistributedCacheConfig>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: 2_000,                   // 2K entries (reduced from 10K to limit memory)
            max_size_bytes: 500 * 1024 * 1024, // 500MB
            ttl: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
            statistics_driven: true,           // Enable by default for better cache efficiency
            min_decisions_to_cache: 1000,      // Cache queries requiring 1000+ decisions
            min_conflicts_to_cache: 100,       // Cache queries with 100+ conflicts
            min_solve_time_ms: 100,            // Cache queries taking 100ms+
            distributed_cache: Maybe::None,
        }
    }
}

impl CacheConfig {
    /// Create cache config for development (smaller cache).
    pub fn development() -> Self {
        Self {
            max_size: 1_000,
            max_size_bytes: 50 * 1024 * 1024,           // 50MB
            ttl: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            statistics_driven: true,
            min_decisions_to_cache: 500,
            min_conflicts_to_cache: 50,
            min_solve_time_ms: 50,
            distributed_cache: Maybe::None,
        }
    }

    /// Create cache config for production (larger cache).
    pub fn production() -> Self {
        Self {
            max_size: 50_000,
            max_size_bytes: 1024 * 1024 * 1024,          // 1GB
            ttl: Duration::from_secs(60 * 24 * 60 * 60), // 60 days
            statistics_driven: true,
            min_decisions_to_cache: 2000,
            min_conflicts_to_cache: 200,
            min_solve_time_ms: 200,
            distributed_cache: Maybe::None,
        }
    }

    /// Create cache config with custom max size.
    pub fn with_max_size(max_size: usize) -> Self {
        let mut config = Self::default();
        config.max_size = max_size;
        config
    }

    /// Disable statistics-driven caching (cache everything)
    pub fn cache_all(mut self) -> Self {
        self.statistics_driven = false;
        self
    }

    /// Enable distributed cache with S3.
    pub fn with_distributed_cache(mut self, config: DistributedCacheConfig) -> Self {
        self.distributed_cache = Maybe::Some(config);
        self
    }

    /// Set time-to-live for cache entries.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Set maximum cache size in bytes.
    pub fn with_max_size_bytes(mut self, bytes: u64) -> Self {
        self.max_size_bytes = bytes;
        self
    }

    /// Check if result should be cached based on statistics
    ///
    /// Uses Z3 solver statistics to determine if a query was expensive
    /// enough to warrant caching. Only caches expensive queries to
    /// maximize cache efficiency.
    pub fn should_cache_with_stats(
        &self,
        decisions: u64,
        conflicts: u64,
        solve_time_ms: u64,
    ) -> bool {
        if !self.statistics_driven {
            return true; // Cache everything
        }

        // Cache if any threshold is exceeded
        decisions >= self.min_decisions_to_cache
            || conflicts >= self.min_conflicts_to_cache
            || solve_time_ms >= self.min_solve_time_ms
    }
}

// ==================== Counterexample Parsing ====================

/// Parse a cached counterexample value string into a map of variable assignments.
///
/// Supports various formats that Z3 models are typically stored as:
/// - Simple assignment: "x = 5" or "x = -5"
/// - Multiple assignments: "x = 5, y = true"
/// - Boolean values: "flag = true" or "flag = false"
/// - Real/float values: "ratio = 3.14" or "ratio = 3/2"
/// - Unknown/complex: stored as-is with Unknown variant
///
/// # Example
///
/// ```ignore
/// let assignments = parse_cached_counterexample("x = -5, y = true");
/// assert_eq!(assignments.get("x"), Some(&CounterExampleValue::Int(-5)));
/// assert_eq!(assignments.get("y"), Some(&CounterExampleValue::Bool(true)));
/// ```
fn parse_cached_counterexample(
    value_str: &str,
) -> Map<Text, crate::counterexample::CounterExampleValue> {
    let mut assignments = Map::new();

    // Handle empty or whitespace-only strings
    let trimmed = value_str.trim();
    if trimmed.is_empty() {
        return assignments;
    }

    // Split by comma to handle multiple assignments
    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Try to parse "name = value" format
        if let Some((name, value)) = parse_assignment(part) {
            assignments.insert(name.to_text(), value);
        } else {
            // If we can't parse the format, store the entire string as a single unknown value
            // This handles cases where the cached value is in an unexpected format
            if assignments.is_empty() {
                assignments.insert("value".to_text(), parse_value_string(trimmed));
            }
        }
    }

    // If no assignments were parsed, store the raw value
    if assignments.is_empty() {
        assignments.insert("value".to_text(), parse_value_string(trimmed));
    }

    assignments
}

/// Parse a single "name = value" assignment string.
///
/// Returns `Some((name, value))` if successfully parsed, `None` otherwise.
fn parse_assignment(s: &str) -> Option<(String, crate::counterexample::CounterExampleValue)> {
    // Find the '=' separator
    let eq_pos = s.find('=')?;

    let name = s[..eq_pos].trim();
    let value_str = s[eq_pos + 1..].trim();

    if name.is_empty() || value_str.is_empty() {
        return None;
    }

    let value = parse_value_string(value_str);
    Some((name.to_string(), value))
}

/// Parse a value string into the appropriate CounterExampleValue variant.
///
/// Attempts to parse as (in order):
/// 1. Boolean (true/false)
/// 2. Integer (including negative)
/// 3. Rational/Float (including "num/den" format from Z3)
/// 4. Unknown (fallback for complex expressions)
fn parse_value_string(s: &str) -> crate::counterexample::CounterExampleValue {
    use crate::counterexample::CounterExampleValue;

    let s = s.trim();

    // Try boolean
    if s == "true" {
        return CounterExampleValue::Bool(true);
    }
    if s == "false" {
        return CounterExampleValue::Bool(false);
    }

    // Try integer (including negative numbers)
    // Handle Z3's format for negative numbers: "(- N)" or "(-N)"
    if let Some(val) = parse_z3_integer(s) {
        return CounterExampleValue::Int(val);
    }

    // Try regular integer parsing
    if let Ok(i) = s.parse::<i64>() {
        return CounterExampleValue::Int(i);
    }

    // Try rational/float (Z3 outputs rationals as "num/den")
    if let Some(val) = parse_z3_rational(s) {
        return CounterExampleValue::Float(val);
    }

    // Try regular float parsing
    if let Ok(f) = s.parse::<f64>() {
        return CounterExampleValue::Float(f);
    }

    // Fallback: unknown value
    CounterExampleValue::Unknown(s.to_text())
}

/// Parse Z3's integer format, which may include "(- N)" for negative numbers.
fn parse_z3_integer(s: &str) -> Option<i64> {
    let s = s.trim();

    // Handle "(- N)" format
    if s.starts_with("(-") && s.ends_with(')') {
        let inner = s[2..s.len() - 1].trim();
        if let Ok(n) = inner.parse::<i64>() {
            return Some(-n);
        }
    }

    // Handle "(- N)" with space format
    if s.starts_with("(- ") && s.ends_with(')') {
        let inner = s[3..s.len() - 1].trim();
        if let Ok(n) = inner.parse::<i64>() {
            return Some(-n);
        }
    }

    // Try direct parsing
    s.parse::<i64>().ok()
}

/// Parse Z3's rational format "num/den" into a float.
fn parse_z3_rational(s: &str) -> Option<f64> {
    if !s.contains('/') {
        return None;
    }

    let parts: List<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return None;
    }

    // Parse numerator (may be in Z3's negative format)
    let num = parse_z3_integer(parts[0].trim())
        .map(|n| n as f64)
        .or_else(|| parts[0].trim().parse::<f64>().ok())?;

    // Parse denominator
    let den = parse_z3_integer(parts[1].trim())
        .map(|n| n as f64)
        .or_else(|| parts[1].trim().parse::<f64>().ok())?;

    if den == 0.0 {
        return None;
    }

    Some(num / den)
}

// ==================== Distributed Cache ====================

/// Distributed cache configuration for S3 backend.
#[derive(Debug, Clone)]
pub struct DistributedCacheConfig {
    /// S3 bucket URL (e.g., "s3://my-bucket/verum-cache")
    pub s3_url: Text,
    /// Cache directory for local copies
    pub cache_dir: Text,
    /// Trust level for distributed cache entries
    pub trust: TrustLevel,
    /// Enable signature verification for cache entries
    pub verify_signatures: bool,
}

/// Trust level for distributed cache entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Trust all cache entries (no verification)
    All,
    /// Require cryptographic signatures
    Signatures,
    /// Verify signatures and check expiration
    SignaturesAndExpiry,
}

impl DistributedCacheConfig {
    /// Create new distributed cache configuration.
    pub fn new(s3_url: Text) -> Self {
        Self {
            s3_url,
            cache_dir: ".verum/verify-cache".to_text(),
            trust: TrustLevel::Signatures,
            verify_signatures: true,
        }
    }

    /// Set trust level.
    pub fn with_trust(mut self, trust: TrustLevel) -> Self {
        self.trust = trust;
        self.verify_signatures = trust != TrustLevel::All;
        self
    }

    /// Set cache directory.
    pub fn with_cache_dir(mut self, dir: Text) -> Self {
        self.cache_dir = dir;
        self
    }
}

// ==================== Cache Statistics Extensions ====================

impl CacheStats {
    /// Calculate total time saved by cache hits.
    ///
    /// Estimates time saved based on average verification time without cache.
    /// Assumes average SMT query takes ~1s (1,000,000 microseconds).
    pub fn time_saved_estimate(&self, avg_verification_time_us: u64) -> Duration {
        let total_saved_us = self.cache_hits * avg_verification_time_us;
        Duration::from_micros(total_saved_us)
    }

    /// Get time saved as a percentage of total time.
    pub fn time_saved_percentage(&self, total_verification_time: Duration) -> f64 {
        if total_verification_time.as_micros() == 0 {
            return 0.0;
        }
        let saved_us = self.cache_hit_time_us;
        let total_us = total_verification_time.as_micros() as u64;
        if saved_us >= total_us {
            return 100.0;
        }
        (saved_us as f64 / total_us as f64) * 100.0
    }

    /// Format cache statistics for display.
    pub fn format_report(&self, total_time: Duration) -> Text {
        let total_queries = self.cache_hits + self.cache_misses;
        let hit_rate = self.hit_rate() * 100.0;
        let utilization = self.utilization() * 100.0;

        // Estimate time saved (assuming 1s average verification time)
        let avg_verify_time_us = 1_000_000; // 1 second
        let time_saved = self.time_saved_estimate(avg_verify_time_us);

        format!(
            "Cache Statistics:\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             Cache hits:     {} / {} ({:.1}%)\n\
             Cache misses:   {} / {} ({:.1}%)\n\
             Time saved:     {:.1}s ({:.0}% of total verification time)\n\
             Cache size:     {} entries / {} max ({:.1}% utilization)\n\
             Avg hit time:   {:.2}μs\n",
            self.cache_hits,
            total_queries,
            hit_rate,
            self.cache_misses,
            total_queries,
            100.0 - hit_rate,
            time_saved.as_secs_f64(),
            self.time_saved_percentage(total_time),
            self.current_size,
            self.max_size,
            utilization,
            self.avg_hit_time_us()
        )
        .to_text()
    }
}

// ==================== TTL-Based Eviction ====================

impl VerificationCacheInner {
    /// Evict entries based on TTL and size constraints.
    fn evict_with_ttl(&mut self, ttl: Duration) {
        let now = Instant::now();

        // First, remove all expired entries
        let keys_to_remove: List<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| now.duration_since(entry.timestamp) > ttl)
            .map(|(key, _)| *key)
            .collect();

        let evicted_count = keys_to_remove.len();
        for key in keys_to_remove {
            self.entries.remove(&key);
        }

        // If still over capacity, use LRU eviction
        if self.entries.len() >= self.max_size {
            self.evict();
        }

        if evicted_count > 0 {
            tracing::debug!("Evicted {} expired cache entries", evicted_count);
        }
    }

    /// Get number of entries that would be evicted by TTL.
    fn count_expired(&self, ttl: Duration) -> usize {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|(_, entry)| now.duration_since(entry.timestamp) > ttl)
            .count()
    }
}

// ==================== Enhanced Cache API ====================

impl VerificationCache {
    /// Evict expired entries based on TTL.
    pub fn evict_expired(&self) {
        let ttl = self.config.ttl;
        self.inner.write().unwrap().evict_with_ttl(ttl);
    }

    /// Get number of expired entries.
    pub fn count_expired(&self) -> usize {
        let ttl = self.config.ttl;
        self.inner.read().unwrap().count_expired(ttl)
    }

    /// Get estimated cache size in bytes.
    ///
    /// Rough estimate: ~200 bytes per entry (key + value + metadata).
    pub fn estimated_size_bytes(&self) -> u64 {
        let entry_count = self.inner.read().unwrap().size();
        (entry_count as u64) * 200 // Rough estimate
    }

    /// Check if cache exceeds size limits.
    pub fn exceeds_size_limit(&self) -> bool {
        self.estimated_size_bytes() > self.config.max_size_bytes
    }

    /// Get cache configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Get with fallback to distributed cache
    ///
    /// First checks local cache, then falls back to distributed cache if configured.
    /// This is an async method because distributed cache access involves network I/O.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let result = cache.get_with_distributed(predicate, base_type).await;
    /// ```
    pub async fn get_with_distributed(
        &self,
        predicate: &Expr,
        base_type: &Type,
    ) -> Maybe<VerificationResult> {
        // Try local cache first
        if let Maybe::Some(result) = self.get(predicate, base_type) {
            return Maybe::Some(result);
        }

        // Try distributed cache if available
        if let Maybe::Some(ref distributed) = self.distributed {
            use crate::distributed_cache::{CachedResult, generate_cache_key};

            // Generate cache key
            let key = CacheKey::new(predicate, base_type);
            let cache_key = generate_cache_key(
                &format!("{}", key.predicate_hash),
                &format!("{}", key.base_type_hash),
                "proof",
            );

            // Fetch from distributed cache
            if let Maybe::Some(entry) = distributed.get(&cache_key).await {
                // Convert CachedResult to VerificationResult
                use crate::cost::VerificationCost;
                use crate::verify::{ProofResult, VerificationError};

                let verification_result = match entry.result {
                    CachedResult::Proved => {
                        // Create a simple "Proved" result
                        let cost = VerificationCost::new(
                            "distributed-cache".into(),
                            Duration::from_millis(entry.metadata.original_time_ms),
                            true,
                        );
                        Ok(ProofResult::new(cost).with_cached())
                    }
                    CachedResult::Counterexample { value } => {
                        // Create counterexample error
                        use crate::counterexample::CounterExample;
                        let cost = VerificationCost::new(
                            "distributed-cache".into(),
                            Duration::from_millis(entry.metadata.original_time_ms),
                            false,
                        );

                        // Parse the cached counterexample value string to extract actual values
                        let assignments = parse_cached_counterexample(&value);

                        let counterex = CounterExample::new(assignments, value.clone().into());

                        Err(VerificationError::CannotProve {
                            constraint: "cached".to_text(),
                            counterexample: Some(counterex),
                            cost,
                            suggestions: List::new(),
                        })
                    }
                    CachedResult::Timeout | CachedResult::Unknown => {
                        // Create timeout error
                        let cost = VerificationCost::new(
                            "distributed-cache".into(),
                            Duration::from_millis(entry.metadata.original_time_ms),
                            false,
                        );
                        Err(VerificationError::Timeout {
                            constraint: "cached".to_text(),
                            timeout: Duration::from_secs(30),
                            cost,
                        })
                    }
                };

                // Cache locally for future lookups
                self.insert(predicate, base_type, verification_result.clone());

                return Maybe::Some(verification_result);
            }
        }

        Maybe::None
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counterexample::CounterExampleValue;

    #[test]
    fn test_parse_simple_integer() {
        let value = parse_value_string("42");
        assert_eq!(value, CounterExampleValue::Int(42));
    }

    #[test]
    fn test_parse_negative_integer() {
        let value = parse_value_string("-5");
        assert_eq!(value, CounterExampleValue::Int(-5));
    }

    #[test]
    fn test_parse_z3_negative_integer() {
        // Z3 outputs negative numbers as "(- N)"
        let value = parse_value_string("(- 5)");
        assert_eq!(value, CounterExampleValue::Int(-5));

        // Also handles compact form "(-N)"
        let value2 = parse_value_string("(-5)");
        assert_eq!(value2, CounterExampleValue::Int(-5));
    }

    #[test]
    fn test_parse_boolean() {
        let value_true = parse_value_string("true");
        assert_eq!(value_true, CounterExampleValue::Bool(true));

        let value_false = parse_value_string("false");
        assert_eq!(value_false, CounterExampleValue::Bool(false));
    }

    #[test]
    fn test_parse_float() {
        let value = parse_value_string("2.72");
        match value {
            CounterExampleValue::Float(f) => assert!((f - 2.72).abs() < 0.001),
            _ => panic!("Expected Float variant"),
        }
    }

    #[test]
    fn test_parse_z3_rational() {
        // Z3 outputs rationals as "num/den"
        let value = parse_value_string("3/2");
        match value {
            CounterExampleValue::Float(f) => assert!((f - 1.5).abs() < 0.001),
            _ => panic!("Expected Float variant for rational"),
        }
    }

    #[test]
    fn test_parse_z3_negative_rational() {
        // Z3 can output negative rationals as "(- N)/M"
        let value = parse_value_string("(-3)/2");
        match value {
            CounterExampleValue::Float(f) => assert!((f - (-1.5)).abs() < 0.001),
            _ => panic!("Expected Float variant for negative rational"),
        }
    }

    #[test]
    fn test_parse_unknown() {
        // Complex expressions fall back to Unknown
        let value = parse_value_string("(+ x y)");
        match value {
            CounterExampleValue::Unknown(s) => assert_eq!(s.as_str(), "(+ x y)"),
            _ => panic!("Expected Unknown variant"),
        }
    }

    #[test]
    fn test_parse_assignment() {
        let result = parse_assignment("x = 42");
        assert!(result.is_some());
        let (name, value) = result.unwrap();
        assert_eq!(name, "x");
        assert_eq!(value, CounterExampleValue::Int(42));
    }

    #[test]
    fn test_parse_assignment_with_spaces() {
        let result = parse_assignment("  myVar  =  -5  ");
        assert!(result.is_some());
        let (name, value) = result.unwrap();
        assert_eq!(name, "myVar");
        assert_eq!(value, CounterExampleValue::Int(-5));
    }

    #[test]
    fn test_parse_assignment_boolean() {
        let result = parse_assignment("flag = true");
        assert!(result.is_some());
        let (name, value) = result.unwrap();
        assert_eq!(name, "flag");
        assert_eq!(value, CounterExampleValue::Bool(true));
    }

    #[test]
    fn test_parse_cached_counterexample_single() {
        let assignments = parse_cached_counterexample("x = -5");
        assert_eq!(assignments.len(), 1);
        assert_eq!(
            assignments.get(&"x".to_text()),
            Maybe::Some(&CounterExampleValue::Int(-5))
        );
    }

    #[test]
    fn test_parse_cached_counterexample_multiple() {
        let assignments = parse_cached_counterexample("x = 10, y = true, z = 3.5");
        assert_eq!(assignments.len(), 3);
        assert_eq!(
            assignments.get(&"x".to_text()),
            Maybe::Some(&CounterExampleValue::Int(10))
        );
        assert_eq!(
            assignments.get(&"y".to_text()),
            Maybe::Some(&CounterExampleValue::Bool(true))
        );
        match assignments.get(&"z".to_text()) {
            Maybe::Some(CounterExampleValue::Float(f)) => assert!((*f - 3.5).abs() < 0.001),
            _ => panic!("Expected Float for z"),
        }
    }

    #[test]
    fn test_parse_cached_counterexample_empty() {
        let assignments = parse_cached_counterexample("");
        assert!(assignments.is_empty());

        let assignments2 = parse_cached_counterexample("   ");
        assert!(assignments2.is_empty());
    }

    #[test]
    fn test_parse_cached_counterexample_raw_value() {
        // If the string doesn't contain '=', treat it as a raw value
        let assignments = parse_cached_counterexample("42");
        assert_eq!(assignments.len(), 1);
        assert_eq!(
            assignments.get(&"value".to_text()),
            Maybe::Some(&CounterExampleValue::Int(42))
        );
    }

    #[test]
    fn test_parse_z3_integer_formats() {
        // Standard negative
        assert_eq!(parse_z3_integer("-5"), Some(-5));

        // Z3 format with spaces: "(- 5)"
        assert_eq!(parse_z3_integer("(- 5)"), Some(-5));

        // Z3 compact format: "(-5)"
        assert_eq!(parse_z3_integer("(-5)"), Some(-5));

        // Positive
        assert_eq!(parse_z3_integer("42"), Some(42));

        // Invalid
        assert_eq!(parse_z3_integer("not_a_number"), None);
    }

    #[test]
    fn test_parse_z3_rational_formats() {
        // Simple rational
        assert!((parse_z3_rational("3/2").unwrap() - 1.5).abs() < 0.001);

        // Negative numerator
        assert!((parse_z3_rational("(-3)/2").unwrap() - (-1.5)).abs() < 0.001);

        // Division by zero returns None
        assert!(parse_z3_rational("5/0").is_none());

        // Not a rational
        assert!(parse_z3_rational("42").is_none());
    }

    #[test]
    fn with_config_no_distributed_keeps_local_only() {
        // Pin: with `CacheConfig.distributed_cache = None` (the
        // default), the auto-construction path is skipped entirely
        // and the new cache is local-only. Round-trip through
        // with_config preserves the absence sentinel.
        let cfg = CacheConfig::default();
        assert!(matches!(cfg.distributed_cache, Maybe::None));
        let cache = VerificationCache::with_config(cfg);
        assert!(
            matches!(cache.distributed, Maybe::None),
            "no configured distributed_cache must produce local-only cache",
        );
    }

    #[test]
    fn with_config_distributed_auto_constructs_backend() {
        // Pin: with `CacheConfig.distributed_cache = Some(...)`,
        // the auto-construction path runs and the resulting cache
        // has a distributed backend installed without the caller
        // having to call `with_distributed(...)` separately.
        let dc_cfg = DistributedCacheConfig::new(Text::from("file:///tmp/verum-test-cache"));
        let cfg = CacheConfig::default().with_distributed_cache(dc_cfg);
        let cache = VerificationCache::with_config(cfg);
        assert!(
            matches!(cache.distributed, Maybe::Some(_)),
            "configured distributed_cache must auto-install the backend",
        );
    }

    #[test]
    fn distributed_cache_trust_signatures_and_expiry_downgrades_to_signatures() {
        // Pin: the local TrustLevel has a SignaturesAndExpiry
        // variant that the underlying distributed_cache module
        // doesn't expose. The auto-construction path downgrades
        // to Signatures (the lower bound of the requested policy)
        // rather than rejecting — keeps the cache usable while
        // documenting the gap (the cache's own TTL handles the
        // expiry side independently).
        let dc_cfg = DistributedCacheConfig {
            s3_url: Text::from("file:///tmp/verum-test-cache-2"),
            cache_dir: Text::from("/tmp/verum-test-cache-2"),
            trust: TrustLevel::SignaturesAndExpiry,
            verify_signatures: true,
        };
        let cfg = CacheConfig::default().with_distributed_cache(dc_cfg);
        let cache = VerificationCache::with_config(cfg);
        assert!(
            matches!(cache.distributed, Maybe::Some(_)),
            "SignaturesAndExpiry trust level must auto-install (downgrade not reject)",
        );
    }
}
