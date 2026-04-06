//! Semantic Query Layer for Content-Addressed Caching
//!
//! This module provides a content-addressed caching layer that uses semantic
//! hashes instead of file paths. This enables:
//!
//! - **Deduplication**: Semantically identical items are cached once
//! - **Cross-project reuse**: Share cached artifacts across projects
//! - **Smart invalidation**: Only invalidate when semantic meaning changes
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │                        Semantic Query Layer                              │
//! │  ┌─────────────────┐   ┌──────────────────┐   ┌───────────────────────┐  │
//! │  │  SemanticKey    │   │  SemanticIndex   │   │  SemanticQueryCache   │  │
//! │  │ - type_hash     │   │ - by_signature   │   │ - content_store       │  │
//! │  │ - signature_hash│   │ - by_protocol    │   │ - semantic_index      │  │
//! │  │ - combined_hash │   │ - by_name        │   │ - lru_eviction        │  │
//! │  └─────────────────┘   └──────────────────┘   └───────────────────────┘  │
//! └──────────────────────────────────────────────────────────────────────────┘
//!                                   │
//!           ┌───────────────────────┼───────────────────────┐
//!           ▼                       ▼                       ▼
//!    ┌─────────────┐       ┌──────────────┐        ┌──────────────┐
//!    │ TypeCache   │       │ FunctionCache│        │ VerifyCache  │
//!    │ (types,     │       │ (signatures, │        │ (verification│
//!    │  protocols) │       │  bodies)     │        │  results)    │
//!    └─────────────┘       └──────────────┘        └──────────────┘
//! ```
//!
//! ## Content-Addressed Keys
//!
//! Unlike file-path based caching, semantic keys are computed from:
//! - Type definition structure (fields, variants, constraints)
//! - Function signature (params, return type, contexts, properties)
//! - Protocol requirements (associated types, methods)
//!
//! This means identical definitions in different files share the same cache entry.
//!
//! ## Semantic Queries
//!
//! The index supports queries like:
//! - Find functions by signature pattern: `fn(Int, _) -> Text`
//! - Find types implementing a protocol: `Iterator<Item = T>`
//! - Find items by name across all modules
//!
//! Multi-pass compilation pipeline: Parse → Meta Registry → Macro Expansion →
//! Contract Verification → Semantic Analysis → HIR → MIR → Optimization → Codegen.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::hash::{ContentHash, FunctionHashes, HashValue};
use verum_common::{List, Map, Maybe, Text};

// ============================================================================
// Semantic Key Types
// ============================================================================

/// A content-addressed key based on semantic hash.
///
/// Unlike file-path keys, semantic keys are computed from the actual content's
/// meaning, enabling deduplication across files and projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticKey {
    /// The primary semantic hash of the item's meaning
    semantic_hash: HashValue,
    /// Kind discriminator to avoid cross-type collisions
    kind: SemanticKind,
}

impl SemanticKey {
    /// Create a new semantic key from a hash and kind.
    pub fn new(semantic_hash: HashValue, kind: SemanticKind) -> Self {
        Self {
            semantic_hash,
            kind,
        }
    }

    /// Create a semantic key for a type definition.
    pub fn for_type(type_def_hash: HashValue) -> Self {
        Self::new(type_def_hash, SemanticKind::Type)
    }

    /// Create a semantic key for a function signature.
    pub fn for_function_signature(sig_hash: HashValue) -> Self {
        Self::new(sig_hash, SemanticKind::FunctionSignature)
    }

    /// Create a semantic key for a function body.
    pub fn for_function_body(body_hash: HashValue) -> Self {
        Self::new(body_hash, SemanticKind::FunctionBody)
    }

    /// Create a semantic key for a protocol definition.
    pub fn for_protocol(proto_hash: HashValue) -> Self {
        Self::new(proto_hash, SemanticKind::Protocol)
    }

    /// Create a semantic key for a verification result.
    pub fn for_verification(verification_hash: HashValue) -> Self {
        Self::new(verification_hash, SemanticKind::Verification)
    }

    /// Create a semantic key for a type-checked item.
    pub fn for_type_check(item_hash: HashValue) -> Self {
        Self::new(item_hash, SemanticKind::TypeCheck)
    }

    /// Get the raw hash value.
    pub fn hash(&self) -> HashValue {
        self.semantic_hash
    }

    /// Get the kind of this semantic key.
    pub fn kind(&self) -> SemanticKind {
        self.kind
    }

    /// Get a combined hash including the kind discriminator.
    pub fn combined(&self) -> HashValue {
        let mut hasher = ContentHash::new();
        hasher.update(&[self.kind as u8]);
        hasher.update(self.semantic_hash.as_bytes());
        hasher.finalize()
    }
}

/// The kind of semantic item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SemanticKind {
    /// Type definition (struct, enum, alias)
    Type = 0,
    /// Function signature (params, return type, contexts)
    FunctionSignature = 1,
    /// Function body (implementation)
    FunctionBody = 2,
    /// Protocol definition (trait-like)
    Protocol = 3,
    /// Verification result
    Verification = 4,
    /// Type check result
    TypeCheck = 5,
    /// Constant value
    Constant = 6,
    /// Module metadata
    Module = 7,
}

// ============================================================================
// Signature Pattern Types
// ============================================================================

/// A pattern for matching function signatures.
///
/// Supports wildcards for flexible querying.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignaturePattern {
    /// Parameter type patterns (None = any type)
    pub params: List<Maybe<Text>>,
    /// Return type pattern (None = any type)
    pub return_type: Maybe<Text>,
    /// Required contexts (empty = any)
    pub contexts: List<Text>,
    /// Required properties (empty = any)
    pub properties: List<Text>,
}

impl SignaturePattern {
    /// Create a new signature pattern.
    pub fn new() -> Self {
        Self {
            params: List::new(),
            return_type: Maybe::None,
            contexts: List::new(),
            properties: List::new(),
        }
    }

    /// Add a parameter type constraint.
    pub fn with_param(mut self, ty: impl Into<Text>) -> Self {
        self.params.push(Maybe::Some(ty.into()));
        self
    }

    /// Add a wildcard parameter (matches any type).
    pub fn with_any_param(mut self) -> Self {
        self.params.push(Maybe::None);
        self
    }

    /// Set the return type constraint.
    pub fn with_return(mut self, ty: impl Into<Text>) -> Self {
        self.return_type = Maybe::Some(ty.into());
        self
    }

    /// Add a context requirement.
    pub fn with_context(mut self, ctx: impl Into<Text>) -> Self {
        self.contexts.push(ctx.into());
        self
    }

    /// Add a property requirement.
    pub fn with_property(mut self, prop: impl Into<Text>) -> Self {
        self.properties.push(prop.into());
        self
    }

    /// Compute a hash for this pattern (for index lookup).
    pub fn hash(&self) -> HashValue {
        let mut hasher = ContentHash::new();
        hasher.update_str("sig_pattern:");
        for param in self.params.iter() {
            match param {
                Maybe::Some(ty) => {
                    hasher.update_str("param:");
                    hasher.update_str(ty.as_str());
                }
                Maybe::None => {
                    hasher.update_str("param:*");
                }
            }
            hasher.update(b"\x00");
        }
        if let Maybe::Some(ref ret) = self.return_type {
            hasher.update_str("ret:");
            hasher.update_str(ret.as_str());
        } else {
            hasher.update_str("ret:*");
        }
        hasher.update(b"\x00");
        for ctx in self.contexts.iter() {
            hasher.update_str("ctx:");
            hasher.update_str(ctx.as_str());
            hasher.update(b"\x00");
        }
        for prop in self.properties.iter() {
            hasher.update_str("prop:");
            hasher.update_str(prop.as_str());
            hasher.update(b"\x00");
        }
        hasher.finalize()
    }
}

impl Default for SignaturePattern {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Semantic Index
// ============================================================================

/// An index mapping semantic properties to cached items.
///
/// Supports efficient queries by:
/// - Function signature (exact or pattern)
/// - Protocol implementation
/// - Name (across all modules)
#[derive(Debug, Clone)]
pub struct SemanticIndex {
    /// Index by name → semantic keys
    by_name: Map<Text, HashSet<SemanticKey>>,

    /// Index by type name → function keys that use it
    by_type_usage: Map<Text, HashSet<SemanticKey>>,

    /// Index by protocol → implementing types
    by_protocol_impl: Map<Text, HashSet<SemanticKey>>,

    /// Index by return type → functions
    by_return_type: Map<Text, HashSet<SemanticKey>>,

    /// Index by context → functions requiring it
    by_context: Map<Text, HashSet<SemanticKey>>,

    /// Reverse index: semantic key → location info
    key_locations: Map<SemanticKey, ItemLocation>,
}

/// Location information for a semantic item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemLocation {
    /// Module path (e.g., "core.collections")
    pub module: Text,
    /// Item name
    pub name: Text,
    /// Source file path (optional, for file-based lookups)
    pub file_path: Maybe<PathBuf>,
    /// Line number (optional)
    pub line: Maybe<u32>,
}

impl SemanticIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            by_name: Map::new(),
            by_type_usage: Map::new(),
            by_protocol_impl: Map::new(),
            by_return_type: Map::new(),
            by_context: Map::new(),
            key_locations: Map::new(),
        }
    }

    /// Index a function by its properties.
    pub fn index_function(
        &mut self,
        key: SemanticKey,
        name: &Text,
        param_types: &[Text],
        return_type: &Maybe<Text>,
        contexts: &[Text],
        location: ItemLocation,
    ) {
        // Index by name
        self.by_name
            .entry(name.clone())
            .or_insert_with(HashSet::new)
            .insert(key);

        // Index by parameter types used
        for ty in param_types {
            self.by_type_usage
                .entry(ty.clone())
                .or_insert_with(HashSet::new)
                .insert(key);
        }

        // Index by return type
        if let Maybe::Some(ret_ty) = return_type {
            self.by_return_type
                .entry(ret_ty.clone())
                .or_insert_with(HashSet::new)
                .insert(key);
        }

        // Index by contexts
        for ctx in contexts {
            self.by_context
                .entry(ctx.clone())
                .or_insert_with(HashSet::new)
                .insert(key);
        }

        // Store location
        self.key_locations.insert(key, location);
    }

    /// Index a type definition.
    pub fn index_type(&mut self, key: SemanticKey, name: &Text, location: ItemLocation) {
        self.by_name
            .entry(name.clone())
            .or_insert_with(HashSet::new)
            .insert(key);
        self.key_locations.insert(key, location);
    }

    /// Index a protocol implementation.
    pub fn index_protocol_impl(
        &mut self,
        key: SemanticKey,
        protocol_name: &Text,
        impl_type: &Text,
        location: ItemLocation,
    ) {
        self.by_protocol_impl
            .entry(protocol_name.clone())
            .or_insert_with(HashSet::new)
            .insert(key);

        // Also index by the implementing type name
        self.by_name
            .entry(impl_type.clone())
            .or_insert_with(HashSet::new)
            .insert(key);

        self.key_locations.insert(key, location);
    }

    /// Find items by name.
    pub fn find_by_name(&self, name: &Text) -> List<SemanticKey> {
        self.by_name
            .get(name)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Find functions that use a specific type.
    pub fn find_by_type_usage(&self, type_name: &Text) -> List<SemanticKey> {
        self.by_type_usage
            .get(type_name)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Find types implementing a protocol.
    pub fn find_implementations(&self, protocol_name: &Text) -> List<SemanticKey> {
        self.by_protocol_impl
            .get(protocol_name)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Find functions by return type.
    pub fn find_by_return_type(&self, return_type: &Text) -> List<SemanticKey> {
        self.by_return_type
            .get(return_type)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Find functions requiring a specific context.
    pub fn find_by_context(&self, context: &Text) -> List<SemanticKey> {
        self.by_context
            .get(context)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get location info for a semantic key.
    pub fn get_location(&self, key: &SemanticKey) -> Maybe<&ItemLocation> {
        self.key_locations.get(key).into()
    }

    /// Remove an item from the index.
    pub fn remove(&mut self, key: &SemanticKey) {
        // Remove from all indices
        for set in self.by_name.values_mut() {
            set.remove(key);
        }
        for set in self.by_type_usage.values_mut() {
            set.remove(key);
        }
        for set in self.by_protocol_impl.values_mut() {
            set.remove(key);
        }
        for set in self.by_return_type.values_mut() {
            set.remove(key);
        }
        for set in self.by_context.values_mut() {
            set.remove(key);
        }
        self.key_locations.remove(key);
    }

    /// Clear the entire index.
    pub fn clear(&mut self) {
        self.by_name.clear();
        self.by_type_usage.clear();
        self.by_protocol_impl.clear();
        self.by_return_type.clear();
        self.by_context.clear();
        self.key_locations.clear();
    }

    /// Get statistics about the index.
    pub fn stats(&self) -> SemanticIndexStats {
        SemanticIndexStats {
            names_indexed: self.by_name.len(),
            types_indexed: self.by_type_usage.len(),
            protocols_indexed: self.by_protocol_impl.len(),
            return_types_indexed: self.by_return_type.len(),
            contexts_indexed: self.by_context.len(),
            total_items: self.key_locations.len(),
        }
    }
}

impl Default for SemanticIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the semantic index.
#[derive(Debug, Clone, Default)]
pub struct SemanticIndexStats {
    /// Number of unique names indexed
    pub names_indexed: usize,
    /// Number of type usages tracked
    pub types_indexed: usize,
    /// Number of protocols with implementations
    pub protocols_indexed: usize,
    /// Number of return types tracked
    pub return_types_indexed: usize,
    /// Number of contexts tracked
    pub contexts_indexed: usize,
    /// Total number of items in the index
    pub total_items: usize,
}

// ============================================================================
// Cached Entry Types
// ============================================================================

/// A cached entry with metadata.
#[derive(Debug, Clone)]
pub struct CachedEntry<T> {
    /// The cached value
    pub value: T,
    /// When this entry was cached
    pub cached_at: Instant,
    /// Number of times this entry was accessed
    pub access_count: u64,
    /// The source file(s) this entry depends on
    pub dependencies: List<PathBuf>,
}

impl<T> CachedEntry<T> {
    /// Create a new cached entry.
    pub fn new(value: T, dependencies: List<PathBuf>) -> Self {
        Self {
            value,
            cached_at: Instant::now(),
            access_count: 0,
            dependencies,
        }
    }

    /// Check if this entry is older than the given duration.
    pub fn is_older_than(&self, duration: Duration) -> bool {
        self.cached_at.elapsed() > duration
    }
}

// ============================================================================
// Semantic Query Cache
// ============================================================================

/// Configuration for the semantic query cache.
#[derive(Debug, Clone)]
pub struct SemanticCacheConfig {
    /// Maximum entries in the type cache
    pub max_type_entries: usize,
    /// Maximum entries in the function cache
    pub max_function_entries: usize,
    /// Maximum entries in the verification cache
    pub max_verification_entries: usize,
    /// Time-to-live for cached entries
    pub ttl: Duration,
    /// Whether to enable cross-project sharing
    pub enable_cross_project: bool,
    /// Enable persistent storage (CAS backend)
    pub enable_persistence: bool,
    /// Path to CAS directory (if persistence enabled)
    pub cas_dir: Maybe<PathBuf>,
}

impl Default for SemanticCacheConfig {
    fn default() -> Self {
        Self {
            max_type_entries: 10_000,
            max_function_entries: 50_000,
            max_verification_entries: 20_000,
            ttl: Duration::from_secs(3600), // 1 hour
            enable_cross_project: false,
            enable_persistence: false,
            cas_dir: Maybe::None,
        }
    }
}

impl SemanticCacheConfig {
    /// Enable persistence with the given CAS directory.
    pub fn with_persistence(mut self, cas_dir: PathBuf) -> Self {
        self.enable_persistence = true;
        self.cas_dir = Maybe::Some(cas_dir);
        self
    }
}

/// Thread-safe semantic query cache.
///
/// Provides content-addressed caching with semantic indexing for efficient
/// queries across the codebase.
///
/// ## Persistence Integration
///
/// When configured with persistence, the cache automatically loads from and saves
/// to a content-addressed store (CAS). This enables:
/// - Cross-session caching: Restart compilation without losing cache
/// - Cross-project sharing: Share cached artifacts between projects
/// - Deduplication: Identical artifacts stored only once
pub struct SemanticQueryCache {
    /// Semantic index for lookups
    index: RwLock<SemanticIndex>,

    /// Type definition cache: semantic key → type info
    type_cache: RwLock<HashMap<SemanticKey, CachedEntry<CachedTypeInfo>>>,

    /// Function cache: semantic key → function info
    function_cache: RwLock<HashMap<SemanticKey, CachedEntry<CachedFunctionInfo>>>,

    /// Verification results cache: semantic key → verification result
    verification_cache: RwLock<HashMap<SemanticKey, CachedEntry<VerificationResult>>>,

    /// Configuration
    config: SemanticCacheConfig,

    /// Statistics
    stats: RwLock<SemanticCacheStats>,

    /// Optional persistent store for CAS backing
    persistent_store: Option<std::sync::Arc<crate::content_addressed_storage::ArtifactStore>>,
}

/// Cached type information.
#[derive(Debug, Clone)]
pub struct CachedTypeInfo {
    /// Type name
    pub name: Text,
    /// Type kind (struct, enum, protocol, etc.)
    pub kind: Text,
    /// Generic parameters
    pub generics: List<Text>,
    /// Type body hash
    pub body_hash: HashValue,
    /// Serialized type representation for quick lookup
    pub serialized: Text,
}

/// Cached function information.
#[derive(Debug, Clone)]
pub struct CachedFunctionInfo {
    /// Function name
    pub name: Text,
    /// Signature hash
    pub signature_hash: HashValue,
    /// Body hash (if present)
    pub body_hash: Maybe<HashValue>,
    /// Parameter types
    pub param_types: List<Text>,
    /// Return type
    pub return_type: Maybe<Text>,
    /// Required contexts
    pub contexts: List<Text>,
    /// Computational properties
    pub properties: List<Text>,
    /// Whether this is a meta function
    pub is_meta: bool,
}

/// Verification result for caching.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Whether verification passed
    pub success: bool,
    /// Error messages (if any)
    pub errors: List<Text>,
    /// Warning messages
    pub warnings: List<Text>,
    /// Proof obligations satisfied
    pub obligations_satisfied: u32,
    /// Proof obligations total
    pub obligations_total: u32,
}

/// Statistics about the semantic cache.
#[derive(Debug, Clone, Default)]
pub struct SemanticCacheStats {
    /// Type cache hits
    pub type_hits: u64,
    /// Type cache misses
    pub type_misses: u64,
    /// Function cache hits
    pub function_hits: u64,
    /// Function cache misses
    pub function_misses: u64,
    /// Verification cache hits
    pub verification_hits: u64,
    /// Verification cache misses
    pub verification_misses: u64,
    /// Total evictions
    pub evictions: u64,
    /// Cross-project cache hits (if enabled)
    pub cross_project_hits: u64,
}

impl SemanticCacheStats {
    /// Calculate overall hit rate.
    pub fn hit_rate(&self) -> f64 {
        let total_hits = self.type_hits + self.function_hits + self.verification_hits;
        let total_misses = self.type_misses + self.function_misses + self.verification_misses;
        let total = total_hits + total_misses;
        if total == 0 {
            0.0
        } else {
            total_hits as f64 / total as f64
        }
    }
}

impl SemanticQueryCache {
    /// Create a new semantic query cache with default configuration.
    pub fn new() -> Self {
        Self::with_config(SemanticCacheConfig::default())
    }

    /// Create a new semantic query cache with custom configuration.
    pub fn with_config(config: SemanticCacheConfig) -> Self {
        // Initialize persistent store if configured
        let persistent_store = if config.enable_persistence {
            if let Maybe::Some(ref cas_dir) = config.cas_dir {
                match crate::content_addressed_storage::ArtifactStore::new(cas_dir) {
                    Ok(store) => Some(std::sync::Arc::new(store)),
                    Err(e) => {
                        tracing::warn!("Failed to initialize CAS: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        Self {
            index: RwLock::new(SemanticIndex::new()),
            type_cache: RwLock::new(HashMap::with_capacity(config.max_type_entries)),
            function_cache: RwLock::new(HashMap::with_capacity(config.max_function_entries)),
            verification_cache: RwLock::new(HashMap::with_capacity(
                config.max_verification_entries,
            )),
            config,
            stats: RwLock::new(SemanticCacheStats::default()),
            persistent_store,
        }
    }

    /// Create with an existing ArtifactStore (for sharing across caches).
    pub fn with_persistent_store(
        config: SemanticCacheConfig,
        store: std::sync::Arc<crate::content_addressed_storage::ArtifactStore>,
    ) -> Self {
        Self {
            index: RwLock::new(SemanticIndex::new()),
            type_cache: RwLock::new(HashMap::with_capacity(config.max_type_entries)),
            function_cache: RwLock::new(HashMap::with_capacity(config.max_function_entries)),
            verification_cache: RwLock::new(HashMap::with_capacity(
                config.max_verification_entries,
            )),
            config,
            stats: RwLock::new(SemanticCacheStats::default()),
            persistent_store: Some(store),
        }
    }

    // ========================================================================
    // Type Cache Operations
    // ========================================================================

    /// Look up a cached type by its semantic key.
    pub fn get_type(&self, key: &SemanticKey) -> Maybe<CachedTypeInfo> {
        if let Ok(mut cache) = self.type_cache.write() {
            if let Some(entry) = cache.get_mut(key) {
                if !entry.is_older_than(self.config.ttl) {
                    entry.access_count += 1;
                    if let Ok(mut stats) = self.stats.write() {
                        stats.type_hits += 1;
                    }
                    return Maybe::Some(entry.value.clone());
                }
            }
        }
        if let Ok(mut stats) = self.stats.write() {
            stats.type_misses += 1;
        }
        Maybe::None
    }

    /// Cache a type definition.
    pub fn cache_type(
        &self,
        key: SemanticKey,
        info: CachedTypeInfo,
        location: ItemLocation,
        dependencies: List<PathBuf>,
    ) {
        // Add to index
        if let Ok(mut index) = self.index.write() {
            index.index_type(key, &info.name, location);
        }

        // Add to cache
        if let Ok(mut cache) = self.type_cache.write() {
            // Evict if at capacity
            if cache.len() >= self.config.max_type_entries {
                self.evict_type_cache(&mut cache);
            }
            cache.insert(key, CachedEntry::new(info, dependencies));
        }
    }

    /// Evict least-recently-used type entries.
    fn evict_type_cache(&self, cache: &mut HashMap<SemanticKey, CachedEntry<CachedTypeInfo>>) {
        let num_to_remove = (self.config.max_type_entries / 10).max(1);
        let mut entries: Vec<_> = cache.iter().collect();
        entries.sort_by_key(|(_, e)| (e.access_count, e.cached_at));

        let keys_to_remove: Vec<_> = entries.iter().take(num_to_remove).map(|(k, _)| **k).collect();

        for key in keys_to_remove {
            cache.remove(&key);
            // Also remove from index
            if let Ok(mut index) = self.index.write() {
                index.remove(&key);
            }
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.evictions += num_to_remove as u64;
        }
    }

    // ========================================================================
    // Function Cache Operations
    // ========================================================================

    /// Look up a cached function by its semantic key.
    pub fn get_function(&self, key: &SemanticKey) -> Maybe<CachedFunctionInfo> {
        if let Ok(mut cache) = self.function_cache.write() {
            if let Some(entry) = cache.get_mut(key) {
                if !entry.is_older_than(self.config.ttl) {
                    entry.access_count += 1;
                    if let Ok(mut stats) = self.stats.write() {
                        stats.function_hits += 1;
                    }
                    return Maybe::Some(entry.value.clone());
                }
            }
        }
        if let Ok(mut stats) = self.stats.write() {
            stats.function_misses += 1;
        }
        Maybe::None
    }

    /// Cache a function definition.
    pub fn cache_function(
        &self,
        key: SemanticKey,
        info: CachedFunctionInfo,
        location: ItemLocation,
        dependencies: List<PathBuf>,
    ) {
        // Add to index
        if let Ok(mut index) = self.index.write() {
            index.index_function(
                key,
                &info.name,
                &info.param_types.iter().cloned().collect::<Vec<_>>(),
                &info.return_type,
                &info.contexts.iter().cloned().collect::<Vec<_>>(),
                location,
            );
        }

        // Add to cache
        if let Ok(mut cache) = self.function_cache.write() {
            if cache.len() >= self.config.max_function_entries {
                self.evict_function_cache(&mut cache);
            }
            cache.insert(key, CachedEntry::new(info, dependencies));
        }
    }

    /// Evict least-recently-used function entries.
    fn evict_function_cache(
        &self,
        cache: &mut HashMap<SemanticKey, CachedEntry<CachedFunctionInfo>>,
    ) {
        let num_to_remove = (self.config.max_function_entries / 10).max(1);
        let mut entries: Vec<_> = cache.iter().collect();
        entries.sort_by_key(|(_, e)| (e.access_count, e.cached_at));

        let keys_to_remove: Vec<_> = entries.iter().take(num_to_remove).map(|(k, _)| **k).collect();

        for key in keys_to_remove {
            cache.remove(&key);
            if let Ok(mut index) = self.index.write() {
                index.remove(&key);
            }
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.evictions += num_to_remove as u64;
        }
    }

    // ========================================================================
    // Verification Cache Operations
    // ========================================================================

    /// Look up a cached verification result.
    pub fn get_verification(&self, key: &SemanticKey) -> Maybe<VerificationResult> {
        if let Ok(mut cache) = self.verification_cache.write() {
            if let Some(entry) = cache.get_mut(key) {
                if !entry.is_older_than(self.config.ttl) {
                    entry.access_count += 1;
                    if let Ok(mut stats) = self.stats.write() {
                        stats.verification_hits += 1;
                    }
                    return Maybe::Some(entry.value.clone());
                }
            }
        }
        if let Ok(mut stats) = self.stats.write() {
            stats.verification_misses += 1;
        }
        Maybe::None
    }

    /// Cache a verification result.
    pub fn cache_verification(
        &self,
        key: SemanticKey,
        result: VerificationResult,
        dependencies: List<PathBuf>,
    ) {
        if let Ok(mut cache) = self.verification_cache.write() {
            if cache.len() >= self.config.max_verification_entries {
                self.evict_verification_cache(&mut cache);
            }
            cache.insert(key, CachedEntry::new(result, dependencies));
        }
    }

    /// Evict least-recently-used verification entries.
    fn evict_verification_cache(
        &self,
        cache: &mut HashMap<SemanticKey, CachedEntry<VerificationResult>>,
    ) {
        let num_to_remove = (self.config.max_verification_entries / 10).max(1);
        let mut entries: Vec<_> = cache.iter().collect();
        entries.sort_by_key(|(_, e)| (e.access_count, e.cached_at));

        let keys_to_remove: Vec<_> = entries.iter().take(num_to_remove).map(|(k, _)| **k).collect();

        for key in keys_to_remove {
            cache.remove(&key);
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.evictions += num_to_remove as u64;
        }
    }

    // ========================================================================
    // Semantic Queries
    // ========================================================================

    /// Find items by name.
    pub fn query_by_name(&self, name: &Text) -> List<SemanticKey> {
        if let Ok(index) = self.index.read() {
            index.find_by_name(name)
        } else {
            List::new()
        }
    }

    /// Find functions that return a specific type.
    pub fn query_by_return_type(&self, return_type: &Text) -> List<SemanticKey> {
        if let Ok(index) = self.index.read() {
            index.find_by_return_type(return_type)
        } else {
            List::new()
        }
    }

    /// Find types that implement a protocol.
    pub fn query_implementations(&self, protocol_name: &Text) -> List<SemanticKey> {
        if let Ok(index) = self.index.read() {
            index.find_implementations(protocol_name)
        } else {
            List::new()
        }
    }

    /// Find functions requiring a specific context.
    pub fn query_by_context(&self, context: &Text) -> List<SemanticKey> {
        if let Ok(index) = self.index.read() {
            index.find_by_context(context)
        } else {
            List::new()
        }
    }

    /// Find functions that use a specific type.
    pub fn query_by_type_usage(&self, type_name: &Text) -> List<SemanticKey> {
        if let Ok(index) = self.index.read() {
            index.find_by_type_usage(type_name)
        } else {
            List::new()
        }
    }

    /// Get location info for a semantic key.
    pub fn get_location(&self, key: &SemanticKey) -> Maybe<ItemLocation> {
        if let Ok(index) = self.index.read() {
            index.get_location(key).cloned().into()
        } else {
            Maybe::None
        }
    }

    // ========================================================================
    // Invalidation
    // ========================================================================

    /// Invalidate all entries that depend on a specific file.
    pub fn invalidate_by_file(&self, file_path: &PathBuf) {
        // Invalidate type cache
        if let Ok(mut cache) = self.type_cache.write() {
            let keys_to_remove: Vec<_> = cache
                .iter()
                .filter(|(_, entry)| entry.dependencies.contains(file_path))
                .map(|(k, _)| *k)
                .collect();
            for key in keys_to_remove {
                cache.remove(&key);
                if let Ok(mut index) = self.index.write() {
                    index.remove(&key);
                }
            }
        }

        // Invalidate function cache
        if let Ok(mut cache) = self.function_cache.write() {
            let keys_to_remove: Vec<_> = cache
                .iter()
                .filter(|(_, entry)| entry.dependencies.contains(file_path))
                .map(|(k, _)| *k)
                .collect();
            for key in keys_to_remove {
                cache.remove(&key);
                if let Ok(mut index) = self.index.write() {
                    index.remove(&key);
                }
            }
        }

        // Invalidate verification cache
        if let Ok(mut cache) = self.verification_cache.write() {
            let keys_to_remove: Vec<_> = cache
                .iter()
                .filter(|(_, entry)| entry.dependencies.contains(file_path))
                .map(|(k, _)| *k)
                .collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }
    }

    /// Invalidate a specific semantic key.
    pub fn invalidate(&self, key: &SemanticKey) {
        if let Ok(mut cache) = self.type_cache.write() {
            cache.remove(key);
        }
        if let Ok(mut cache) = self.function_cache.write() {
            cache.remove(key);
        }
        if let Ok(mut cache) = self.verification_cache.write() {
            cache.remove(key);
        }
        if let Ok(mut index) = self.index.write() {
            index.remove(key);
        }
    }

    /// Clear all caches.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.type_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.function_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.verification_cache.write() {
            cache.clear();
        }
        if let Ok(mut index) = self.index.write() {
            index.clear();
        }
        if let Ok(mut stats) = self.stats.write() {
            *stats = SemanticCacheStats::default();
        }
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    /// Get cache statistics.
    pub fn stats(&self) -> SemanticCacheStats {
        if let Ok(stats) = self.stats.read() {
            stats.clone()
        } else {
            SemanticCacheStats::default()
        }
    }

    /// Get index statistics.
    pub fn index_stats(&self) -> SemanticIndexStats {
        if let Ok(index) = self.index.read() {
            index.stats()
        } else {
            SemanticIndexStats::default()
        }
    }

    /// Get current cache sizes.
    pub fn cache_sizes(&self) -> (usize, usize, usize) {
        let type_size = self
            .type_cache
            .read()
            .map(|c| c.len())
            .unwrap_or(0);
        let function_size = self
            .function_cache
            .read()
            .map(|c| c.len())
            .unwrap_or(0);
        let verification_size = self
            .verification_cache
            .read()
            .map(|c| c.len())
            .unwrap_or(0);
        (type_size, function_size, verification_size)
    }

    // ========================================================================
    // Persistence Operations
    // ========================================================================

    /// Check if persistence is enabled and available.
    pub fn has_persistence(&self) -> bool {
        self.persistent_store.is_some()
    }

    /// Save all cached entries to persistent storage.
    ///
    /// Returns the number of entries saved.
    pub fn save_to_persistent(&self) -> std::io::Result<PersistenceResult> {
        let store = match &self.persistent_store {
            Some(store) => store,
            None => {
                return Ok(PersistenceResult {
                    types_saved: 0,
                    functions_saved: 0,
                    verifications_saved: 0,
                })
            }
        };

        let mut types_saved = 0;
        let mut functions_saved = 0;
        let mut verifications_saved = 0;

        // Save types
        if let Ok(cache) = self.type_cache.read() {
            for (key, entry) in cache.iter() {
                if let Ok(_) = store.store_type(entry.value.name.as_str(), &entry.value) {
                    types_saved += 1;
                }
                // Log key usage for debugging
                let _ = key.kind();
            }
        }

        // Save functions
        if let Ok(cache) = self.function_cache.read() {
            for (key, entry) in cache.iter() {
                if let Ok(_) = store.store_function(entry.value.name.as_str(), &entry.value) {
                    functions_saved += 1;
                }
                let _ = key.kind();
            }
        }

        // Save verification results
        if let Ok(cache) = self.verification_cache.read() {
            for (key, entry) in cache.iter() {
                if let Ok(_) = store.store_verification(*key, &entry.value) {
                    verifications_saved += 1;
                }
            }
        }

        // Save indices
        store.save_indices()?;

        Ok(PersistenceResult {
            types_saved,
            functions_saved,
            verifications_saved,
        })
    }

    /// Load type from persistent storage by name.
    ///
    /// Falls back to persistent storage if not in memory cache.
    pub fn get_type_with_fallback(&self, key: &SemanticKey, name: &str) -> Maybe<CachedTypeInfo> {
        // First check memory cache
        if let result @ Maybe::Some(_) = self.get_type(key) {
            return result;
        }

        // Fall back to persistent storage
        if let Some(store) = &self.persistent_store {
            if let Ok(Some(info)) = store.load_type(name) {
                // Promote to memory cache
                let location = ItemLocation {
                    module: Text::from("unknown"),
                    name: Text::from(name),
                    file_path: Maybe::None,
                    line: Maybe::None,
                };
                self.cache_type(*key, info.clone(), location, List::new());

                // Update cross-project hit counter
                if let Ok(mut stats) = self.stats.write() {
                    stats.cross_project_hits += 1;
                }

                return Maybe::Some(info);
            }
        }

        Maybe::None
    }

    /// Load function from persistent storage by name.
    ///
    /// Falls back to persistent storage if not in memory cache.
    pub fn get_function_with_fallback(
        &self,
        key: &SemanticKey,
        name: &str,
    ) -> Maybe<CachedFunctionInfo> {
        // First check memory cache
        if let result @ Maybe::Some(_) = self.get_function(key) {
            return result;
        }

        // Fall back to persistent storage
        if let Some(store) = &self.persistent_store {
            if let Ok(Some(info)) = store.load_function(name) {
                // Promote to memory cache
                let location = ItemLocation {
                    module: Text::from("unknown"),
                    name: Text::from(name),
                    file_path: Maybe::None,
                    line: Maybe::None,
                };
                self.cache_function(*key, info.clone(), location, List::new());

                if let Ok(mut stats) = self.stats.write() {
                    stats.cross_project_hits += 1;
                }

                return Maybe::Some(info);
            }
        }

        Maybe::None
    }

    /// Load verification result from persistent storage.
    ///
    /// Falls back to persistent storage if not in memory cache.
    pub fn get_verification_with_fallback(
        &self,
        key: &SemanticKey,
    ) -> Maybe<VerificationResult> {
        // First check memory cache
        if let result @ Maybe::Some(_) = self.get_verification(key) {
            return result;
        }

        // Fall back to persistent storage
        if let Some(store) = &self.persistent_store {
            if let Ok(Some(result)) = store.load_verification(key) {
                // Promote to memory cache
                self.cache_verification(*key, result.clone(), List::new());

                if let Ok(mut stats) = self.stats.write() {
                    stats.cross_project_hits += 1;
                }

                return Maybe::Some(result);
            }
        }

        Maybe::None
    }

    /// Run garbage collection on persistent storage.
    pub fn gc_persistent(&self, max_age: Duration) -> std::io::Result<u64> {
        if let Some(store) = &self.persistent_store {
            let result = store.gc(max_age)?;
            Ok(result.deleted)
        } else {
            Ok(0)
        }
    }

    /// Get persistent storage statistics.
    pub fn persistent_stats(
        &self,
    ) -> Option<crate::content_addressed_storage::ArtifactStoreStats> {
        self.persistent_store.as_ref().map(|s| s.stats())
    }

    /// Get persistent storage hit rate.
    pub fn persistent_hit_rate(&self) -> f64 {
        self.persistent_store
            .as_ref()
            .map(|s| s.hit_rate())
            .unwrap_or(0.0)
    }
}

/// Result of a persistence save operation.
#[derive(Debug, Clone, Default)]
pub struct PersistenceResult {
    /// Number of types saved
    pub types_saved: u64,
    /// Number of functions saved
    pub functions_saved: u64,
    /// Number of verification results saved
    pub verifications_saved: u64,
}

impl PersistenceResult {
    /// Total entries saved
    pub fn total(&self) -> u64 {
        self.types_saved + self.functions_saved + self.verifications_saved
    }
}

impl Default for SemanticQueryCache {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Key Computation Helpers
// ============================================================================

/// Compute a semantic key for a type definition from ItemHashes.
pub fn compute_type_key(name: &str, type_hash: &HashValue) -> SemanticKey {
    let mut hasher = ContentHash::new();
    hasher.update_str("type:");
    hasher.update_str(name);
    hasher.update(b"\x00");
    hasher.update(type_hash.as_bytes());
    SemanticKey::for_type(hasher.finalize())
}

/// Compute a semantic key for a function from FunctionHashes.
pub fn compute_function_key(name: &str, hashes: &FunctionHashes) -> SemanticKey {
    let mut hasher = ContentHash::new();
    hasher.update_str("fn:");
    hasher.update_str(name);
    hasher.update(b"\x00");
    hasher.update(hashes.signature.as_bytes());
    SemanticKey::for_function_signature(hasher.finalize())
}

/// Compute a semantic key for a verification result.
pub fn compute_verification_key(item_key: &SemanticKey, verification_level: u8) -> SemanticKey {
    let mut hasher = ContentHash::new();
    hasher.update_str("verify:");
    hasher.update(item_key.semantic_hash.as_bytes());
    hasher.update(&[verification_level]);
    SemanticKey::for_verification(hasher.finalize())
}

/// Compute a semantic key for a protocol implementation.
pub fn compute_impl_key(protocol_name: &str, impl_type: &str, impl_hash: &HashValue) -> SemanticKey {
    let mut hasher = ContentHash::new();
    hasher.update_str("impl:");
    hasher.update_str(protocol_name);
    hasher.update(b" for ");
    hasher.update_str(impl_type);
    hasher.update(b"\x00");
    hasher.update(impl_hash.as_bytes());
    SemanticKey::for_type(hasher.finalize())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_str;

    #[test]
    fn test_semantic_key_creation() {
        let hash = hash_str("test_type");
        let key = SemanticKey::for_type(hash);
        assert_eq!(key.kind(), SemanticKind::Type);
        assert_eq!(key.hash(), hash);
    }

    #[test]
    fn test_semantic_key_combined() {
        let hash = hash_str("test_fn");
        let key1 = SemanticKey::for_function_signature(hash);
        let key2 = SemanticKey::for_function_body(hash);
        // Different kinds should produce different combined hashes
        assert_ne!(key1.combined(), key2.combined());
    }

    #[test]
    fn test_signature_pattern() {
        let pattern = SignaturePattern::new()
            .with_param("Int")
            .with_any_param()
            .with_return("Bool")
            .with_context("Database");

        assert_eq!(pattern.params.len(), 2);
        assert!(pattern.return_type.is_some());
        assert_eq!(pattern.contexts.len(), 1);
    }

    #[test]
    fn test_semantic_index_basic() {
        let mut index = SemanticIndex::new();

        let key = SemanticKey::for_type(hash_str("MyType"));
        let location = ItemLocation {
            module: Text::from("core"),
            name: Text::from("MyType"),
            file_path: Maybe::None,
            line: Maybe::None,
        };

        index.index_type(key, &Text::from("MyType"), location);

        let found = index.find_by_name(&Text::from("MyType"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], key);
    }

    #[test]
    fn test_semantic_index_function() {
        let mut index = SemanticIndex::new();

        let key = SemanticKey::for_function_signature(hash_str("my_func"));
        let location = ItemLocation {
            module: Text::from("core"),
            name: Text::from("my_func"),
            file_path: Maybe::None,
            line: Maybe::None,
        };

        index.index_function(
            key,
            &Text::from("my_func"),
            &[Text::from("Int"), Text::from("Text")],
            &Maybe::Some(Text::from("Bool")),
            &[Text::from("Database")],
            location,
        );

        // Find by name
        let by_name = index.find_by_name(&Text::from("my_func"));
        assert_eq!(by_name.len(), 1);

        // Find by return type
        let by_return = index.find_by_return_type(&Text::from("Bool"));
        assert_eq!(by_return.len(), 1);

        // Find by context
        let by_context = index.find_by_context(&Text::from("Database"));
        assert_eq!(by_context.len(), 1);

        // Find by type usage
        let by_type = index.find_by_type_usage(&Text::from("Int"));
        assert_eq!(by_type.len(), 1);
    }

    #[test]
    fn test_semantic_cache_type_operations() {
        let cache = SemanticQueryCache::new();

        let key = SemanticKey::for_type(hash_str("MyType"));
        let info = CachedTypeInfo {
            name: Text::from("MyType"),
            kind: Text::from("struct"),
            generics: List::new(),
            body_hash: hash_str("body"),
            serialized: Text::from("type MyType is { x: Int };"),
        };
        let location = ItemLocation {
            module: Text::from("core"),
            name: Text::from("MyType"),
            file_path: Maybe::None,
            line: Maybe::None,
        };

        cache.cache_type(key, info.clone(), location, List::new());

        // Should be found
        let found = cache.get_type(&key);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name.as_str(), "MyType");

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.type_hits, 1);
        assert_eq!(stats.type_misses, 0);
    }

    #[test]
    fn test_semantic_cache_function_operations() {
        let cache = SemanticQueryCache::new();

        let key = SemanticKey::for_function_signature(hash_str("my_func"));
        let info = CachedFunctionInfo {
            name: Text::from("my_func"),
            signature_hash: hash_str("sig"),
            body_hash: Maybe::Some(hash_str("body")),
            param_types: List::from_iter([Text::from("Int")]),
            return_type: Maybe::Some(Text::from("Bool")),
            contexts: List::new(),
            properties: List::new(),
            is_meta: false,
        };
        let location = ItemLocation {
            module: Text::from("core"),
            name: Text::from("my_func"),
            file_path: Maybe::None,
            line: Maybe::None,
        };

        cache.cache_function(key, info.clone(), location, List::new());

        let found = cache.get_function(&key);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name.as_str(), "my_func");
    }

    #[test]
    fn test_semantic_cache_queries() {
        let cache = SemanticQueryCache::new();

        let key = SemanticKey::for_function_signature(hash_str("process"));
        let info = CachedFunctionInfo {
            name: Text::from("process"),
            signature_hash: hash_str("sig"),
            body_hash: Maybe::None,
            param_types: List::from_iter([Text::from("Request")]),
            return_type: Maybe::Some(Text::from("Response")),
            contexts: List::from_iter([Text::from("Logger")]),
            properties: List::new(),
            is_meta: false,
        };
        let location = ItemLocation {
            module: Text::from("handlers"),
            name: Text::from("process"),
            file_path: Maybe::None,
            line: Maybe::None,
        };

        cache.cache_function(key, info, location, List::new());

        // Query by name
        let by_name = cache.query_by_name(&Text::from("process"));
        assert_eq!(by_name.len(), 1);

        // Query by return type
        let by_return = cache.query_by_return_type(&Text::from("Response"));
        assert_eq!(by_return.len(), 1);

        // Query by context
        let by_context = cache.query_by_context(&Text::from("Logger"));
        assert_eq!(by_context.len(), 1);
    }

    #[test]
    fn test_semantic_cache_invalidation() {
        let cache = SemanticQueryCache::new();

        let key = SemanticKey::for_type(hash_str("MyType"));
        let info = CachedTypeInfo {
            name: Text::from("MyType"),
            kind: Text::from("struct"),
            generics: List::new(),
            body_hash: hash_str("body"),
            serialized: Text::from("type MyType is { x: Int };"),
        };
        let location = ItemLocation {
            module: Text::from("core"),
            name: Text::from("MyType"),
            file_path: Maybe::Some(PathBuf::from("/src/types.vr")),
            line: Maybe::None,
        };

        cache.cache_type(
            key,
            info,
            location,
            List::from_iter([PathBuf::from("/src/types.vr")]),
        );

        // Should be found
        assert!(cache.get_type(&key).is_some());

        // Invalidate by file
        cache.invalidate_by_file(&PathBuf::from("/src/types.vr"));

        // Should no longer be found
        assert!(cache.get_type(&key).is_none());
    }

    #[test]
    fn test_compute_keys() {
        let type_hash = hash_str("type_def");
        let type_key = compute_type_key("MyType", &type_hash);
        assert_eq!(type_key.kind(), SemanticKind::Type);

        let fn_hashes = FunctionHashes::new(hash_str("sig"), hash_str("body"));
        let fn_key = compute_function_key("my_func", &fn_hashes);
        assert_eq!(fn_key.kind(), SemanticKind::FunctionSignature);

        let verify_key = compute_verification_key(&fn_key, 2);
        assert_eq!(verify_key.kind(), SemanticKind::Verification);
    }
}
