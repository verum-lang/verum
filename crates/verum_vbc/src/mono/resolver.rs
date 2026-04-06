//! Monomorphization resolver for resolving generic instantiations.
//!
//! The resolver implements a three-level resolution strategy:
//! 1. Stdlib precompiled specializations
//! 2. Persistent cache (validated)
//! 3. Schedule for specialization
//!
//! Three-level resolution: (1) stdlib precompiled specializations, (2) persistent
//! disk cache with validity checking, (3) schedule for fresh specialization.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::module::{SpecializationEntry, VbcModule};
use crate::types::TypeRef;

use super::cache::MonomorphizationCache;
use super::graph::{InstantiationGraph, InstantiationRequest};

// ============================================================================
// Resolved Specialization
// ============================================================================

/// Resolved specialization status.
#[derive(Debug, Clone)]
pub enum ResolvedSpecialization {
    /// Found in stdlib precompiled.
    StdlibPrecompiled {
        /// Bytecode offset in stdlib.
        bytecode_offset: u32,
        /// Bytecode length.
        bytecode_length: u32,
        /// Register count.
        register_count: u16,
    },

    /// Found in persistent cache (validated).
    Cached {
        /// Cache file path.
        cache_file: PathBuf,
        /// Bytecode offset in cache file.
        bytecode_offset: u32,
        /// Bytecode length.
        bytecode_length: u32,
    },

    /// Needs to be specialized (scheduled as pending).
    Pending,
}

// ============================================================================
// Cache Metadata
// ============================================================================

/// Metadata for cached specializations (for validation).
#[derive(Debug, Clone)]
pub struct CacheMetadata {
    /// Compiler version that created this cache.
    pub compiler_version: Version,
    /// Hash of type definitions (for invalidation).
    pub type_hash: u64,
    /// Hash of generic function definition.
    pub function_hash: u64,
    /// Creation timestamp.
    pub created_at: SystemTime,
}

impl CacheMetadata {
    /// Creates new metadata.
    pub fn new(type_hash: u64, function_hash: u64) -> Self {
        Self {
            compiler_version: Version::current(),
            type_hash,
            function_hash,
            created_at: SystemTime::now(),
        }
    }

    /// Loads metadata from a file.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        if data.len() < 32 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid cache metadata",
            ));
        }

        // Parse: [version_major:u16][version_minor:u16][version_patch:u16][pad:u16]
        //        [type_hash:u64][function_hash:u64][timestamp:u64]
        let version_major = u16::from_le_bytes([data[0], data[1]]);
        let version_minor = u16::from_le_bytes([data[2], data[3]]);
        let version_patch = u16::from_le_bytes([data[4], data[5]]);
        // data[6..8] is padding

        let type_hash = u64::from_le_bytes([
            data[8], data[9], data[10], data[11],
            data[12], data[13], data[14], data[15],
        ]);
        let function_hash = u64::from_le_bytes([
            data[16], data[17], data[18], data[19],
            data[20], data[21], data[22], data[23],
        ]);
        let timestamp_secs = u64::from_le_bytes([
            data[24], data[25], data[26], data[27],
            data[28], data[29], data[30], data[31],
        ]);

        Ok(Self {
            compiler_version: Version {
                major: version_major,
                minor: version_minor,
                patch: version_patch,
            },
            type_hash,
            function_hash,
            created_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp_secs),
        })
    }

    /// Saves metadata to a file.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut data = Vec::with_capacity(32);

        // Version
        data.extend_from_slice(&self.compiler_version.major.to_le_bytes());
        data.extend_from_slice(&self.compiler_version.minor.to_le_bytes());
        data.extend_from_slice(&self.compiler_version.patch.to_le_bytes());
        data.extend_from_slice(&[0u8, 0u8]); // padding

        // Hashes
        data.extend_from_slice(&self.type_hash.to_le_bytes());
        data.extend_from_slice(&self.function_hash.to_le_bytes());

        // Timestamp
        let timestamp_secs = self.created_at
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        data.extend_from_slice(&timestamp_secs.to_le_bytes());

        std::fs::write(path, &data)
    }
}

// ============================================================================
// Version
// ============================================================================

/// Compiler version for cache validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
}

impl Version {
    /// Current compiler version.
    pub fn current() -> Self {
        Self {
            major: 0,
            minor: 4,
            patch: 0,
        }
    }

    /// Checks if this version is compatible with another.
    ///
    /// Compatible means: same major version and other.minor >= self.minor.
    pub fn compatible_with(&self, other: &Version) -> bool {
        self.major == other.major && self.minor <= other.minor
    }
}

// ============================================================================
// Monomorphization Resolver
// ============================================================================

/// Resolver for generic instantiations.
///
/// Implements a three-level resolution strategy:
/// 1. Check stdlib precompiled specializations
/// 2. Check persistent cache (with validation)
/// 3. Schedule for specialization
pub struct MonomorphizationResolver {
    /// Stdlib VBC module (contains precompiled specializations).
    stdlib: Option<Arc<VbcModule>>,

    /// Persistent cache.
    cache: Option<MonomorphizationCache>,

    /// Resolved specializations: hash -> resolution.
    resolved: HashMap<u64, ResolvedSpecialization>,

    /// Pending requests (need to be specialized).
    pending: Vec<InstantiationRequest>,

    /// Statistics.
    stats: ResolverStats,
}

/// Resolver statistics.
#[derive(Debug, Clone, Default)]
pub struct ResolverStats {
    /// Total requests processed.
    pub total_requests: usize,
    /// Stdlib precompiled hits.
    pub stdlib_hits: usize,
    /// Cache hits.
    pub cache_hits: usize,
    /// Cache misses (scheduled for specialization).
    pub pending_count: usize,
    /// Cache validation failures.
    pub cache_invalidations: usize,
}

impl Default for MonomorphizationResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl MonomorphizationResolver {
    /// Creates a new resolver without stdlib or cache.
    pub fn new() -> Self {
        Self {
            stdlib: None,
            cache: None,
            resolved: HashMap::new(),
            pending: Vec::new(),
            stats: ResolverStats::default(),
        }
    }

    /// Sets the stdlib module.
    pub fn with_core(mut self, stdlib: Arc<VbcModule>) -> Self {
        self.stdlib = Some(stdlib);
        self
    }

    /// Sets the persistent cache.
    pub fn with_cache(mut self, cache: MonomorphizationCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Resolves all instantiations in the graph.
    pub fn resolve(&mut self, graph: &InstantiationGraph) -> Result<(), ResolverError> {
        // Process in topological order for better cache utilization
        let order = graph.topological_order();

        for idx in order {
            if let Some(request) = graph.all_instantiations().get(idx) {
                self.resolve_one(request)?;
            }
        }

        Ok(())
    }

    /// Resolves a single instantiation request.
    fn resolve_one(&mut self, request: &InstantiationRequest) -> Result<(), ResolverError> {
        self.stats.total_requests += 1;

        // Already resolved?
        if self.resolved.contains_key(&request.hash) {
            return Ok(());
        }

        // Step 1: Check stdlib precompiled
        if let Some(ref stdlib) = self.stdlib
            && let Some(spec) = self.find_stdlib_precompiled(stdlib, request) {
                self.resolved.insert(request.hash, ResolvedSpecialization::StdlibPrecompiled {
                    bytecode_offset: spec.bytecode_offset,
                    bytecode_length: spec.bytecode_length,
                    register_count: spec.register_count,
                });
                self.stats.stdlib_hits += 1;
                return Ok(());
            }

        // Step 2: Check persistent cache
        if let Some(ref mut cache) = self.cache {
            // Extract cache_dir first to avoid borrow conflicts
            let cache_dir = cache.cache_dir().clone();
            let metadata_path = cache_dir.join(format!("{:016x}.meta", request.hash));
            let cache_file = cache_dir.join(format!("{:016x}.vbc", request.hash));

            if let Some(cached_bytecode) = cache.get(request.hash) {
                let bytecode_len = cached_bytecode.len() as u32;

                // Validate cache
                if let Ok(metadata) = CacheMetadata::load(&metadata_path) {
                    if self.validate_cache(&metadata, request) {
                        self.resolved.insert(request.hash, ResolvedSpecialization::Cached {
                            cache_file,
                            bytecode_offset: 0,
                            bytecode_length: bytecode_len,
                        });
                        self.stats.cache_hits += 1;
                        return Ok(());
                    } else {
                        self.stats.cache_invalidations += 1;
                    }
                }
            }
        }

        // Step 3: Schedule for specialization
        self.resolved.insert(request.hash, ResolvedSpecialization::Pending);
        self.pending.push(request.clone());
        self.stats.pending_count += 1;

        Ok(())
    }

    /// Finds a stdlib precompiled specialization.
    fn find_stdlib_precompiled(
        &self,
        stdlib: &VbcModule,
        request: &InstantiationRequest,
    ) -> Option<SpecializationEntry> {
        stdlib.specializations.iter().find(|entry| {
            entry.generic_fn == request.function_id &&
            entry.type_args == request.type_args
        }).cloned()
    }

    /// Validates a cached specialization.
    ///
    /// Full validation includes:
    /// 1. Compiler version compatibility
    /// 2. Type definition hash match
    /// 3. Function bytecode hash match
    fn validate_cache(&self, metadata: &CacheMetadata, request: &InstantiationRequest) -> bool {
        // Check compiler version compatibility
        let current = Version::current();
        if !metadata.compiler_version.compatible_with(&current) {
            return false;
        }

        // Check type definitions haven't changed
        let current_type_hash = self.compute_type_hash(&request.type_args);
        if metadata.type_hash != current_type_hash {
            return false;
        }

        // Check function definition hasn't changed
        // Look up the generic function's bytecode
        if let Some(ref stdlib) = self.stdlib
            && let Some(func) = stdlib.functions.iter().find(|f| f.id == request.function_id) {
                let start = func.bytecode_offset as usize;
                let end = start + func.bytecode_length as usize;
                if end <= stdlib.bytecode.len() {
                    let bytecode = &stdlib.bytecode[start..end];
                    let current_func_hash = self.compute_function_hash(bytecode);
                    if metadata.function_hash != current_func_hash {
                        return false;
                    }
                }
            }

        true
    }

    /// Returns the resolution for a request hash.
    pub fn get_resolution(&self, hash: u64) -> Option<&ResolvedSpecialization> {
        self.resolved.get(&hash)
    }

    /// Takes pending requests (consumes them).
    pub fn take_pending(&mut self) -> Vec<InstantiationRequest> {
        std::mem::take(&mut self.pending)
    }

    /// Returns pending requests without consuming.
    pub fn pending(&self) -> &[InstantiationRequest] {
        &self.pending
    }

    /// Returns statistics.
    pub fn stats(&self) -> &ResolverStats {
        &self.stats
    }

    /// Returns the stdlib module.
    pub fn stdlib(&self) -> Option<&Arc<VbcModule>> {
        self.stdlib.as_ref()
    }

    /// Computes type hash for cache validation.
    pub fn compute_type_hash(&self, type_args: &[TypeRef]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        for type_ref in type_args {
            type_ref.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Computes function hash for cache validation.
    pub fn compute_function_hash(&self, bytecode: &[u8]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        bytecode.hash(&mut hasher);
        hasher.finish()
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Resolver error.
#[derive(Debug)]
pub enum ResolverError {
    /// IO error.
    Io(std::io::Error),
    /// Invalid cache.
    InvalidCache(String),
}

impl From<std::io::Error> for ResolverError {
    fn from(err: std::io::Error) -> Self {
        ResolverError::Io(err)
    }
}

impl std::fmt::Display for ResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolverError::Io(e) => write!(f, "IO error: {}", e),
            ResolverError::InvalidCache(msg) => write!(f, "Invalid cache: {}", msg),
        }
    }
}

impl std::error::Error for ResolverError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::FunctionId;
    use crate::types::TypeId;

    #[test]
    fn test_version_compatibility() {
        let v1 = Version { major: 0, minor: 4, patch: 0 };
        let v2 = Version { major: 0, minor: 4, patch: 1 };
        let v3 = Version { major: 0, minor: 5, patch: 0 };
        let v4 = Version { major: 1, minor: 0, patch: 0 };

        assert!(v1.compatible_with(&v2)); // Same major/minor
        assert!(v1.compatible_with(&v3)); // Same major, newer minor
        assert!(!v1.compatible_with(&v4)); // Different major
    }

    #[test]
    fn test_resolver_empty() {
        let mut resolver = MonomorphizationResolver::new();
        let graph = InstantiationGraph::new();

        assert!(resolver.resolve(&graph).is_ok());
        assert_eq!(resolver.stats().total_requests, 0);
    }

    #[test]
    fn test_resolver_pending() {
        use super::super::graph::SourceLocation;

        let mut resolver = MonomorphizationResolver::new();
        let mut graph = InstantiationGraph::new();

        graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        assert!(resolver.resolve(&graph).is_ok());
        assert_eq!(resolver.stats().total_requests, 1);
        assert_eq!(resolver.stats().pending_count, 1);
        assert_eq!(resolver.pending().len(), 1);
    }

    #[test]
    fn test_cache_metadata_roundtrip() {
        let dir = std::env::temp_dir().join("verum-test-meta");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.meta");

        let original = CacheMetadata::new(0x123456789ABCDEF0, 0xFEDCBA9876543210);
        original.save(&path).unwrap();

        let loaded = CacheMetadata::load(&path).unwrap();
        assert_eq!(loaded.type_hash, original.type_hash);
        assert_eq!(loaded.function_hash, original.function_hash);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
