// ARCHITECTURE NOTE: This file uses regex-based pseudo-parsing (extract_declarations_from_file)
// for lightweight source scanning. Migration to verum_fast_parser planned for cog system.
//! Stdlib Compilation Cache
//!
//! Industrial-grade caching system for compiled Verum stdlib.
//! Ensures stdlib is compiled exactly once per project, with proper cache
//! invalidation based on compiler version, target configuration, and source hash.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                            CoreCache                                       │
//! │  ┌─────────────────┐    ┌──────────────────┐    ┌─────────────────────────┐ │
//! │  │   CacheKey      │    │   CacheEntry     │    │   CacheStore            │ │
//! │  │ - compiler_ver  │    │ - metadata       │    │ - disk: target/.verum/  │ │
//! │  │ - target_triple │    │ - vbc_modules    │    │ - memory: LRU cache     │ │
//! │  │ - source_hash   │    │ - timestamp      │    │                         │ │
//! │  └─────────────────┘    └──────────────────┘    └─────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Cache Invalidation
//!
//! The cache is invalidated when:
//! - Verum compiler version changes
//! - Target configuration changes (os, arch)
//! - Stdlib source content changes (hash mismatch)
//!
//! # Usage
//!
//! ```ignore
//! let cache = CoreCache::new(project_root)?;
//!
//! // Get compiled stdlib (compiles if needed, uses cache if valid)
//! let stdlib = cache.get_or_compile(&source, &target)?;
//!
//! // Use stdlib for user code compilation
//! pipeline.set_stdlib(stdlib);
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use crate::hash::ContentHash;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use verum_ast::cfg::TargetConfig;

use crate::core_source::{CoreSource, CoreSourceTrait};

// =============================================================================
// CACHE KEY
// =============================================================================

/// Unique identifier for a cached stdlib compilation.
///
/// The cache key is computed from:
/// - Compiler version (ensures cache invalidation on upgrades)
/// - Target triple (os, arch, env)
/// - Stdlib source content hash
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoreCacheKey {
    /// Verum compiler version (e.g., "0.4.0")
    pub compiler_version: String,

    /// Target triple (e.g., "aarch64-apple-darwin")
    pub target_triple: String,

    /// Hash of all stdlib source files (16 hex chars)
    pub source_hash: String,
}

impl CoreCacheKey {
    /// Create a new cache key from components.
    pub fn new(source: &dyn CoreSourceTrait, target: &TargetConfig) -> Self {
        let compiler_version = crate::VERSION.to_string();
        let target_triple = format!(
            "{}-{}-{}",
            target.target_arch.as_str(),
            target.target_vendor.as_str(),
            target.target_os.as_str()
        );
        let source_hash = Self::compute_source_hash(source);

        Self {
            compiler_version,
            target_triple,
            source_hash,
        }
    }

    /// Compute hash of all stdlib source files using Blake3.
    fn compute_source_hash(source: &dyn CoreSourceTrait) -> String {
        let mut hasher = ContentHash::new();

        // Sort file list for deterministic ordering
        let mut files: Vec<&str> = source.list_files();
        files.sort();

        for file in files {
            // Hash file path
            hasher.update_str(file);
            hasher.update(b"\x00");

            // Hash file content
            if let Some(content) = source.read_file(file) {
                hasher.update(content.as_ref().as_bytes());
                hasher.update(b"\x00");
            }
        }

        // Return first 16 hex chars for compatibility with existing cache keys
        hasher.finalize().short().repeat(2)
    }

    /// Get the cache file name for this key.
    pub fn cache_filename(&self) -> String {
        format!(
            "stdlib_{}_{}_{}.cache",
            self.compiler_version.replace('.', "_"),
            self.target_triple.replace('-', "_"),
            &self.source_hash[..8] // Use first 8 chars of hash
        )
    }
}

// =============================================================================
// CACHE ENTRY
// =============================================================================

/// Cached compilation result for stdlib.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreCacheEntry {
    /// The cache key that produced this entry
    pub key: CoreCacheKey,

    /// Compiled stdlib metadata (types, functions, etc.)
    pub metadata: CachedCoreMetadata,

    /// Compilation timestamp
    pub compiled_at: SystemTime,

    /// Compilation duration (for metrics)
    pub compilation_duration_ms: u64,

    /// Number of modules compiled
    pub module_count: usize,

    /// Number of functions compiled
    pub function_count: usize,
}

/// Cached stdlib metadata that can be serialized to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCoreMetadata {
    /// Type registry entries (serialized)
    pub types: Vec<CachedTypeEntry>,

    /// Function registry entries (serialized)
    pub functions: Vec<CachedFunctionEntry>,

    /// Module information
    pub modules: Vec<CachedModuleEntry>,

    /// Context protocol names declared in stdlib (`context Name { ... }`
    /// and `context protocol Name { ... }`). Pre-registered during
    /// NormalBuild so `using [ComputeDevice]` etc. resolve without
    /// requiring the declaring module to be loaded first.
    pub context_declarations: Vec<String>,

    // =========================================================================
    // META-SYSTEM INFORMATION
    // =========================================================================
    // These fields enable macro expansion and meta-function resolution
    // in user code that uses stdlib features.

    /// Meta function registry entries
    ///
    /// Includes all `meta fn` definitions from stdlib, used for
    /// compile-time code generation and metaprogramming.
    #[serde(default)]
    pub meta_functions: Vec<CachedMetaFunctionEntry>,

    /// Macro definitions (derive, attribute, procedural)
    ///
    /// Includes all macro definitions like @derive(Debug), @derive(Clone),
    /// and custom attribute macros.
    #[serde(default)]
    pub macros: Vec<CachedMacroEntry>,

    /// Built-in derive implementations
    ///
    /// Maps derive names to their implementations for stdlib types.
    #[serde(default)]
    pub derives: Vec<CachedDeriveEntry>,
}

/// Cached type information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTypeEntry {
    /// Full type path (e.g., "core.Option")
    pub path: String,

    /// Type definition (serialized AST or simplified form)
    pub definition: String,

    /// Type kind (struct, enum, protocol, etc.)
    pub kind: String,
}

/// Cached function information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFunctionEntry {
    /// Full function path (e.g., "core.Option.Some")
    pub path: String,

    /// Function signature
    pub signature: String,

    /// Is this an intrinsic?
    pub is_intrinsic: bool,

    /// Intrinsic name (if applicable)
    pub intrinsic_name: Option<String>,
}

/// Cached module information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModuleEntry {
    /// Module name (e.g., "core", "sys.linux")
    pub name: String,

    /// Files in this module
    pub files: Vec<String>,

    /// Dependencies
    pub dependencies: Vec<String>,
}

/// Cached meta function information.
///
/// Meta functions execute at compile-time and are used for metaprogramming.
/// This includes functions marked with `meta fn` keyword.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMetaFunctionEntry {
    /// Full function path (e.g., "meta.derive_debug")
    pub path: String,

    /// Function name
    pub name: String,

    /// Module where defined
    pub module: String,

    /// Parameter types (serialized)
    pub params: Vec<CachedMetaParam>,

    /// Return type (serialized)
    pub return_type: String,

    /// Whether this is an async meta function
    pub is_async: bool,

    /// Required meta contexts (e.g., "TypeInfo", "AstAccess")
    pub contexts: Vec<String>,
}

/// Cached meta function parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMetaParam {
    /// Parameter name
    pub name: String,

    /// Parameter type (serialized)
    pub ty: String,

    /// Whether this is a meta parameter (compile-time value)
    pub is_meta: bool,
}

/// Cached macro definition information.
///
/// Macros include derive macros (@derive), attribute macros (@attr),
/// and procedural macros.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMacroEntry {
    /// Macro name (e.g., "Debug", "Clone", "derive_builder")
    pub name: String,

    /// Macro kind: "derive", "attribute", or "procedural"
    pub kind: String,

    /// Function name that performs the expansion
    pub expander: String,

    /// Module where defined
    pub module: String,
}

/// Cached derive implementation information.
///
/// Tracks built-in derives and their implementations for types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDeriveEntry {
    /// Derive name (e.g., "Debug", "Clone", "Eq")
    pub name: String,

    /// Target type path (e.g., "core.Option")
    pub target_type: String,

    /// Module providing the implementation
    pub impl_module: String,
}

// =============================================================================
// CACHE STORE
// =============================================================================

/// Cache storage backend.
///
/// Supports both disk and memory caching for optimal performance.
pub struct CoreCacheStore {
    /// Cache directory on disk
    cache_dir: PathBuf,

    /// In-memory LRU cache for hot entries
    memory_cache: RwLock<HashMap<String, Arc<CoreCacheEntry>>>,

    /// Maximum age for cache entries (default: 30 days)
    max_age: Duration,
}

impl CoreCacheStore {
    /// Create a new cache store.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory of the project
    pub fn new(project_root: &Path) -> Result<Self> {
        // Use target/.verum-cache/core/ for project-local caching
        let cache_dir = project_root.join("target").join(".verum-cache").join("stdlib");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create cache dir: {}", cache_dir.display()))?;

        Ok(Self {
            cache_dir,
            memory_cache: RwLock::new(HashMap::new()),
            max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
        })
    }

    /// Get a cache entry by key.
    pub fn get(&self, key: &CoreCacheKey) -> Option<Arc<CoreCacheEntry>> {
        let filename = key.cache_filename();

        // Check memory cache first
        {
            let cache = self.memory_cache.read().ok()?;
            if let Some(entry) = cache.get(&filename) {
                if self.is_entry_valid(entry) {
                    debug!("Stdlib cache hit (memory): {}", filename);
                    return Some(Arc::clone(entry));
                }
            }
        }

        // Check disk cache
        let cache_path = self.cache_dir.join(&filename);
        if cache_path.exists() {
            match self.load_from_disk(&cache_path) {
                Ok(entry) => {
                    if entry.key == *key && self.is_entry_valid(&entry) {
                        debug!("Stdlib cache hit (disk): {}", filename);
                        let entry = Arc::new(entry);

                        // Populate memory cache
                        if let Ok(mut cache) = self.memory_cache.write() {
                            cache.insert(filename, Arc::clone(&entry));
                        }

                        return Some(entry);
                    }
                }
                Err(e) => {
                    warn!("Failed to load cache entry from disk: {}", e);
                }
            }
        }

        debug!("Stdlib cache miss: {}", filename);
        None
    }

    /// Store a cache entry.
    pub fn put(&self, entry: CoreCacheEntry) -> Result<()> {
        let filename = entry.key.cache_filename();
        let entry = Arc::new(entry);

        // Store in memory
        if let Ok(mut cache) = self.memory_cache.write() {
            cache.insert(filename.clone(), Arc::clone(&entry));
        }

        // Store on disk
        let cache_path = self.cache_dir.join(&filename);
        self.save_to_disk(&cache_path, &entry)?;

        debug!("Stdlib cache stored: {}", filename);
        Ok(())
    }

    /// Check if a cache entry is still valid.
    fn is_entry_valid(&self, entry: &CoreCacheEntry) -> bool {
        match entry.compiled_at.elapsed() {
            Ok(age) => age < self.max_age,
            Err(_) => false, // System time went backwards
        }
    }

    /// Load cache entry from disk.
    fn load_from_disk(&self, path: &Path) -> Result<CoreCacheEntry> {
        let data = fs::read(path)
            .with_context(|| format!("Failed to read cache file: {}", path.display()))?;
        let entry: CoreCacheEntry = bincode::deserialize(&data)
            .with_context(|| format!("Failed to deserialize cache: {}", path.display()))?;
        Ok(entry)
    }

    /// Save cache entry to disk.
    fn save_to_disk(&self, path: &Path, entry: &CoreCacheEntry) -> Result<()> {
        let data = bincode::serialize(entry)
            .context("Failed to serialize cache entry")?;
        fs::write(path, &data)
            .with_context(|| format!("Failed to write cache file: {}", path.display()))?;
        Ok(())
    }

    /// Clean up expired cache entries.
    pub fn cleanup_expired(&self) -> Result<usize> {
        let mut removed = 0;

        // Clean memory cache
        if let Ok(mut cache) = self.memory_cache.write() {
            cache.retain(|_, entry| {
                let valid = self.is_entry_valid(entry);
                if !valid {
                    removed += 1;
                }
                valid
            });
        }

        // Clean disk cache
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "cache") {
                    if let Ok(metadata) = fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(age) = modified.elapsed() {
                                if age > self.max_age {
                                    if fs::remove_file(&path).is_ok() {
                                        removed += 1;
                                        debug!("Removed expired cache: {}", path.display());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(removed)
    }
}

// =============================================================================
// STDLIB CACHE
// =============================================================================

/// Main stdlib caching interface.
///
/// Provides automatic compilation and caching of the Verum standard library.
/// Ensures stdlib is compiled exactly once per project, with proper cache
/// invalidation.
pub struct CoreCache {
    /// Cache storage backend
    store: CoreCacheStore,

    /// Compiled entry (loaded/computed)
    entry: RwLock<Option<Arc<CoreCacheEntry>>>,
}

impl CoreCache {
    /// Create a new stdlib cache for a project.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory of the project
    pub fn new(project_root: &Path) -> Result<Self> {
        let store = CoreCacheStore::new(project_root)?;
        Ok(Self {
            store,
            entry: RwLock::new(None),
        })
    }

    /// Get compiled stdlib, compiling if necessary.
    ///
    /// This is the main entry point for stdlib compilation caching.
    /// It will:
    /// 1. Check if cache is valid
    /// 2. Return cached result if valid
    /// 3. Compile stdlib if cache is invalid/missing
    /// 4. Store result in cache
    ///
    /// # Arguments
    ///
    /// * `source` - Stdlib source (embedded or local)
    /// * `target` - Target configuration
    ///
    /// # Returns
    ///
    /// Arc to cached stdlib entry (can be shared across threads)
    pub fn get_or_compile(
        &self,
        source: &CoreSource,
        target: &TargetConfig,
    ) -> Result<Arc<CoreCacheEntry>> {
        let key = CoreCacheKey::new(source, target);

        // Check if we already have a valid entry loaded
        {
            let entry = self.entry.read().unwrap();
            if let Some(ref cached) = *entry {
                if cached.key == key {
                    debug!("Using already-loaded stdlib cache");
                    return Ok(Arc::clone(cached));
                }
            }
        }

        // Check store for cached entry
        if let Some(cached) = self.store.get(&key) {
            info!(
                "Loaded stdlib from cache ({} modules, compiled in {}ms)",
                cached.module_count, cached.compilation_duration_ms
            );

            // Store in local entry for fast access
            *self.entry.write().unwrap() = Some(Arc::clone(&cached));
            return Ok(cached);
        }

        // Need to compile
        info!(
            "Compiling stdlib ({} files from {} source)...",
            source.list_files().len(),
            source.source_name()
        );

        let start = std::time::Instant::now();
        let metadata = self.compile_core(source, target)?;
        let duration = start.elapsed();

        let entry = CoreCacheEntry {
            key: key.clone(),
            module_count: metadata.modules.len(),
            function_count: metadata.functions.len(),
            compilation_duration_ms: duration.as_millis() as u64,
            compiled_at: SystemTime::now(),
            metadata,
        };

        // Store in cache
        self.store.put(entry.clone())?;

        let entry = Arc::new(entry);
        *self.entry.write().unwrap() = Some(Arc::clone(&entry));

        info!(
            "Stdlib compiled and cached: {} modules, {} functions in {:?}",
            entry.module_count, entry.function_count, duration
        );

        Ok(entry)
    }

    /// Force recompilation of stdlib (ignores cache).
    pub fn force_recompile(
        &self,
        source: &CoreSource,
        target: &TargetConfig,
    ) -> Result<Arc<CoreCacheEntry>> {
        info!("Force recompiling stdlib...");

        let key = CoreCacheKey::new(source, target);
        let start = std::time::Instant::now();
        let metadata = self.compile_core(source, target)?;
        let duration = start.elapsed();

        let entry = CoreCacheEntry {
            key,
            module_count: metadata.modules.len(),
            function_count: metadata.functions.len(),
            compilation_duration_ms: duration.as_millis() as u64,
            compiled_at: SystemTime::now(),
            metadata,
        };

        // Store in cache (overwriting)
        self.store.put(entry.clone())?;

        let entry = Arc::new(entry);
        *self.entry.write().unwrap() = Some(Arc::clone(&entry));

        Ok(entry)
    }

    /// Check if stdlib is already cached (without compiling).
    pub fn is_cached(&self, source: &CoreSource, target: &TargetConfig) -> bool {
        let key = CoreCacheKey::new(source, target);
        self.store.get(&key).is_some()
    }

    /// Get cache entry if available (without compiling).
    pub fn get_cached(&self, source: &CoreSource, target: &TargetConfig) -> Option<Arc<CoreCacheEntry>> {
        let key = CoreCacheKey::new(source, target);
        self.store.get(&key)
    }

    /// Compile stdlib from source.
    ///
    /// This is the internal compilation logic. It uses the CoreSourceResolver
    /// to discover modules and compiles them in dependency order.
    fn compile_core(
        &self,
        source: &CoreSource,
        _target: &TargetConfig,
    ) -> Result<CachedCoreMetadata> {
        use crate::core_source::CoreSourceResolver;

        // Discover modules
        let mut resolver = CoreSourceResolver::new(source);
        resolver.discover().map_err(|e| anyhow::anyhow!("Module discovery failed: {}", e))?;

        let modules = resolver.modules_in_order();

        let mut cached_modules = Vec::new();
        let mut cached_types = Vec::new();
        let mut cached_functions = Vec::new();
        let mut context_declarations = Vec::new();

        // Process each module
        for module in modules {
            debug!("Processing module: {}", module.name);

            cached_modules.push(CachedModuleEntry {
                name: module.name.clone(),
                files: module.files.clone(),
                dependencies: module.dependencies.clone(),
            });

            // Parse and extract types/functions from each file
            for file in &module.files {
                if let Some(content) = resolver.read_file(file) {
                    // Extract types and functions from the file
                    // This is a simplified extraction - the real implementation
                    // would use the full parser
                    self.extract_declarations_from_file(
                        file,
                        &module.name,
                        content.as_ref(),
                        &mut cached_types,
                        &mut cached_functions,
                    );

                    // Extract context declarations:
                    //   `public context Name {`
                    //   `public context protocol Name {`
                    // These are pre-registered during NormalBuild so
                    // `using [ComputeDevice]` etc. resolve without
                    // requiring module loading order.
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("public context ") {
                            let rest = trimmed.strip_prefix("public context ").unwrap_or("");
                            let name = if rest.starts_with("protocol ") {
                                rest.strip_prefix("protocol ")
                                    .and_then(|s| s.split_whitespace().next())
                            } else {
                                rest.split_whitespace().next()
                            };
                            if let Some(n) = name {
                                let clean = n.trim_end_matches('{').trim().to_string();
                                if !clean.is_empty() && !context_declarations.contains(&clean) {
                                    context_declarations.push(clean);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(
            "Stdlib compilation complete: {} modules, {} types, {} functions",
            cached_modules.len(),
            cached_types.len(),
            cached_functions.len()
        );

        // Extract meta-system information
        let (cached_meta_functions, cached_macros, cached_derives) =
            self.extract_meta_system_info(source, &cached_modules);

        info!(
            "Meta-system: {} meta functions, {} macros, {} derives",
            cached_meta_functions.len(),
            cached_macros.len(),
            cached_derives.len()
        );

        if !context_declarations.is_empty() {
            tracing::debug!(
                "Stdlib cache: extracted {} context declarations: {:?}",
                context_declarations.len(),
                context_declarations
            );
        }

        Ok(CachedCoreMetadata {
            types: cached_types,
            functions: cached_functions,
            modules: cached_modules,
            meta_functions: cached_meta_functions,
            macros: cached_macros,
            derives: cached_derives,
            context_declarations,
        })
    }

    /// Extract type and function declarations from a file.
    fn extract_declarations_from_file(
        &self,
        _file_path: &str,
        module_name: &str,
        content: &str,
        types: &mut Vec<CachedTypeEntry>,
        functions: &mut Vec<CachedFunctionEntry>,
    ) {
        // Parse the file to extract declarations
        // Using a simple regex-based extraction for now
        // Full implementation would use verum_parser

        // Extract type declarations
        for line in content.lines() {
            let trimmed = line.trim();

            // Type declarations: "type Name is ..."
            if trimmed.starts_with("public type ") || trimmed.starts_with("type ") {
                if let Some(name) = self.extract_type_name(trimmed) {
                    let kind = if trimmed.contains("protocol") {
                        "protocol"
                    } else if trimmed.contains("|") {
                        "enum"
                    } else {
                        "struct"
                    };

                    types.push(CachedTypeEntry {
                        path: format!("{}.{}", module_name, name),
                        definition: trimmed.to_string(),
                        kind: kind.to_string(),
                    });
                }
            }

            // Function declarations with @intrinsic
            if trimmed.starts_with("@intrinsic") {
                // Next non-empty line should be the function
                // (simplified - real impl would track state)
            }

            // Function declarations: "fn name(...)"
            if trimmed.starts_with("public fn ") || trimmed.starts_with("fn ") {
                if let Some(name) = self.extract_function_name(trimmed) {
                    // Check if previous line had @intrinsic
                    let is_intrinsic = content
                        .lines()
                        .take_while(|l| !l.contains(&name))
                        .last()
                        .map_or(false, |l| l.trim().starts_with("@intrinsic"));

                    let intrinsic_name = if is_intrinsic {
                        self.extract_intrinsic_name(content, &name)
                    } else {
                        None
                    };

                    functions.push(CachedFunctionEntry {
                        path: format!("{}.{}", module_name, name),
                        signature: trimmed.to_string(),
                        is_intrinsic,
                        intrinsic_name,
                    });
                }
            }
        }
    }

    /// Extract type name from a type declaration line.
    fn extract_type_name(&self, line: &str) -> Option<String> {
        // "public type Name is ..." or "type Name is ..."
        let start = if line.starts_with("public ") {
            "public type ".len()
        } else {
            "type ".len()
        };

        let rest = &line[start..];
        let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '<' && c != '>')?;
        let name = &rest[..end];

        // Handle generics: "Name<T>" -> "Name"
        let name = name.split('<').next().unwrap_or(name);
        Some(name.to_string())
    }

    /// Extract function name from a function declaration line.
    fn extract_function_name(&self, line: &str) -> Option<String> {
        // "public fn name(...)" or "fn name(...)"
        let fn_idx = line.find("fn ")?;
        let rest = &line[fn_idx + 3..];
        let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
        Some(rest[..end].to_string())
    }

    /// Extract intrinsic name from @intrinsic attribute.
    fn extract_intrinsic_name(&self, content: &str, fn_name: &str) -> Option<String> {
        // Find @intrinsic("name") before the function
        for (i, line) in content.lines().enumerate() {
            if line.contains(&format!("fn {}", fn_name)) {
                // Look backwards for @intrinsic - collect first since Lines doesn't impl ExactSizeIterator
                let prev_lines: Vec<&str> = content.lines().take(i).collect();
                for prev_line in prev_lines.iter().rev() {
                    if prev_line.trim().starts_with("@intrinsic") {
                        // Extract name from @intrinsic("name")
                        if let Some(start) = prev_line.find('"') {
                            if let Some(end) = prev_line.rfind('"') {
                                if end > start {
                                    return Some(prev_line[start + 1..end].to_string());
                                }
                            }
                        }
                        // @intrinsic without explicit name - use function name
                        return Some(fn_name.to_string());
                    }
                    // Stop if we hit another declaration
                    if prev_line.trim().starts_with("fn ")
                        || prev_line.trim().starts_with("public fn ")
                        || prev_line.trim().starts_with("type ")
                    {
                        break;
                    }
                }
                break;
            }
        }
        None
    }

    /// Extract meta-system information from stdlib source.
    ///
    /// This includes:
    /// - Meta functions (`meta fn` declarations)
    /// - Macro definitions (@derive, @attr, procedural)
    /// - Derive implementations
    fn extract_meta_system_info(
        &self,
        source: &dyn CoreSourceTrait,
        modules: &[CachedModuleEntry],
    ) -> (Vec<CachedMetaFunctionEntry>, Vec<CachedMacroEntry>, Vec<CachedDeriveEntry>) {
        let mut meta_functions = Vec::new();
        let mut macros = Vec::new();
        let mut derives = Vec::new();

        // Process each module's files
        for module in modules {
            for file_path in &module.files {
                if let Some(content) = source.read_file(file_path) {
                    self.extract_meta_from_file(
                        file_path,
                        &module.name,
                        content.as_ref(),
                        &mut meta_functions,
                        &mut macros,
                        &mut derives,
                    );
                }
            }
        }

        // Add built-in derives that are always available
        self.add_builtin_derives(&mut macros);

        (meta_functions, macros, derives)
    }

    /// Extract meta-system declarations from a single file.
    fn extract_meta_from_file(
        &self,
        _file_path: &str,
        module_name: &str,
        content: &str,
        meta_functions: &mut Vec<CachedMetaFunctionEntry>,
        macros: &mut Vec<CachedMacroEntry>,
        derives: &mut Vec<CachedDeriveEntry>,
    ) {
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Meta function declarations: "meta fn name(...)"
            if trimmed.starts_with("meta fn ") || trimmed.starts_with("public meta fn ") {
                if let Some(meta_fn) = self.parse_meta_function(trimmed, module_name, &lines, i) {
                    meta_functions.push(meta_fn);
                }
            }

            // Macro attribute: @macro(kind = "derive", ...)
            if trimmed.starts_with("@macro") {
                if let Some(macro_entry) = self.parse_macro_attribute(trimmed, module_name, &lines, i) {
                    macros.push(macro_entry);
                }
            }

            // Derive implementation: implement Debug for Type
            if trimmed.starts_with("implement ") && trimmed.contains(" for ") {
                if let Some(derive_entry) = self.parse_derive_impl(trimmed, module_name) {
                    derives.push(derive_entry);
                }
            }
        }
    }

    /// Parse a meta function declaration.
    fn parse_meta_function(
        &self,
        line: &str,
        module_name: &str,
        lines: &[&str],
        _line_idx: usize,
    ) -> Option<CachedMetaFunctionEntry> {
        // Extract function name
        let fn_idx = line.find("meta fn ")?;
        let rest = &line[fn_idx + 8..];
        let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
        let name = rest[..name_end].to_string();

        // Check for async
        let is_async = line.contains("async meta fn");

        // Extract contexts from "using" clause
        let contexts = self.extract_using_contexts(line, lines);

        // Extract parameters (simplified - just the signature string)
        let params = self.extract_meta_params(line);

        // Extract return type (simplified)
        let return_type = self.extract_return_type(line);

        Some(CachedMetaFunctionEntry {
            path: format!("{}.{}", module_name, name),
            name,
            module: module_name.to_string(),
            params,
            return_type,
            is_async,
            contexts,
        })
    }

    /// Extract contexts from "using" clause.
    fn extract_using_contexts(&self, line: &str, _lines: &[&str]) -> Vec<String> {
        let mut contexts = Vec::new();

        if let Some(using_idx) = line.find("using ") {
            let rest = &line[using_idx + 6..];
            // Handle "using [A, B, C]" or "using Context"
            if rest.starts_with('[') {
                if let Some(end) = rest.find(']') {
                    let ctx_list = &rest[1..end];
                    for ctx in ctx_list.split(',') {
                        let ctx = ctx.trim();
                        if !ctx.is_empty() {
                            contexts.push(ctx.to_string());
                        }
                    }
                }
            } else {
                // Single context
                let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
                    .unwrap_or(rest.len());
                let ctx = rest[..end].trim();
                if !ctx.is_empty() {
                    contexts.push(ctx.to_string());
                }
            }
        }

        contexts
    }

    /// Extract meta function parameters (simplified).
    fn extract_meta_params(&self, line: &str) -> Vec<CachedMetaParam> {
        let mut params = Vec::new();

        if let Some(paren_start) = line.find('(') {
            if let Some(paren_end) = line.find(')') {
                let params_str = &line[paren_start + 1..paren_end];
                for param in params_str.split(',') {
                    let param = param.trim();
                    if param.is_empty() {
                        continue;
                    }
                    // Parse "name: Type" or "meta name: Type"
                    let is_meta = param.starts_with("meta ");
                    let param = if is_meta { &param[5..] } else { param };

                    if let Some(colon_idx) = param.find(':') {
                        let name = param[..colon_idx].trim().to_string();
                        let ty = param[colon_idx + 1..].trim().to_string();
                        params.push(CachedMetaParam { name, ty, is_meta });
                    }
                }
            }
        }

        params
    }

    /// Extract return type from function signature.
    fn extract_return_type(&self, line: &str) -> String {
        if let Some(arrow_idx) = line.find("->") {
            let rest = &line[arrow_idx + 2..];
            // Find end of type (before { or using or end of line)
            let end = rest.find(['{', ';'])
                .or_else(|| rest.find("using"))
                .unwrap_or(rest.len());
            rest[..end].trim().to_string()
        } else {
            "()".to_string() // Unit return type
        }
    }

    /// Parse a macro attribute declaration.
    fn parse_macro_attribute(
        &self,
        line: &str,
        module_name: &str,
        lines: &[&str],
        line_idx: usize,
    ) -> Option<CachedMacroEntry> {
        // @macro(kind = "derive", name = "Debug", ...)
        // Extract kind
        let kind = if line.contains("\"derive\"") {
            "derive"
        } else if line.contains("\"attribute\"") {
            "attribute"
        } else {
            "procedural"
        };

        // Extract macro name
        let name = self.extract_attribute_value(line, "name")?;

        // Find expander function (next function declaration)
        let expander = self.find_next_function_name(lines, line_idx)?;

        Some(CachedMacroEntry {
            name,
            kind: kind.to_string(),
            expander,
            module: module_name.to_string(),
        })
    }

    /// Extract attribute value from @macro(..., key = "value", ...)
    fn extract_attribute_value(&self, line: &str, key: &str) -> Option<String> {
        let search = format!("{} = \"", key);
        if let Some(start_idx) = line.find(&search) {
            let rest = &line[start_idx + search.len()..];
            if let Some(end_idx) = rest.find('"') {
                return Some(rest[..end_idx].to_string());
            }
        }
        None
    }

    /// Find the next function name after the current line.
    fn find_next_function_name(&self, lines: &[&str], start_idx: usize) -> Option<String> {
        for line in lines.iter().skip(start_idx + 1) {
            let trimmed = line.trim();
            if trimmed.starts_with("fn ") || trimmed.starts_with("public fn ")
                || trimmed.starts_with("meta fn ") || trimmed.starts_with("public meta fn ")
            {
                return self.extract_function_name(trimmed);
            }
            // Stop if we hit another attribute or declaration
            if trimmed.starts_with('@') || trimmed.starts_with("type ") {
                break;
            }
        }
        None
    }

    /// Parse a derive implementation.
    fn parse_derive_impl(&self, line: &str, module_name: &str) -> Option<CachedDeriveEntry> {
        // implement Debug for Option
        let impl_idx = line.find("implement ")?;
        let rest = &line[impl_idx + 10..];

        let for_idx = rest.find(" for ")?;
        let derive_name = rest[..for_idx].trim().to_string();
        let target_type = rest[for_idx + 5..].trim()
            .split(['{', '<'])
            .next()?
            .trim()
            .to_string();

        // Only track common derives
        let common_derives = ["Debug", "Clone", "Copy", "Eq", "PartialEq", "Hash", "Default"];
        if !common_derives.contains(&derive_name.as_str()) {
            return None;
        }

        Some(CachedDeriveEntry {
            name: derive_name,
            target_type,
            impl_module: module_name.to_string(),
        })
    }

    /// Add built-in derives that are always available.
    fn add_builtin_derives(&self, macros: &mut Vec<CachedMacroEntry>) {
        // These are the core derives available in any Verum program
        let builtin = [
            ("Debug", "core"),
            ("Clone", "core"),
            ("Copy", "core"),
            ("Eq", "core"),
            ("PartialEq", "core"),
            ("Hash", "core"),
            ("Default", "core"),
            ("Ord", "core"),
            ("PartialOrd", "core"),
        ];

        for (name, module) in builtin {
            macros.push(CachedMacroEntry {
                name: name.to_string(),
                kind: "derive".to_string(),
                expander: format!("derive_{}", name.to_lowercase()),
                module: module.to_string(),
            });
        }
    }
}

// =============================================================================
// GLOBAL CACHE
// =============================================================================

/// Global stdlib cache instance.
///
/// Initialized once per process and shared across all compilations.
static GLOBAL_CORE_CACHE: std::sync::OnceLock<CoreCache> = std::sync::OnceLock::new();

/// Initialize the global stdlib cache.
///
/// Should be called once at process startup with the project root.
pub fn init_global_cache(project_root: &Path) -> Result<()> {
    let cache = CoreCache::new(project_root)?;
    GLOBAL_CORE_CACHE
        .set(cache)
        .map_err(|_| anyhow::anyhow!("Global stdlib cache already initialized"))
}

/// Get the global stdlib cache.
///
/// Panics if not initialized.
pub fn global_cache() -> &'static CoreCache {
    GLOBAL_CORE_CACHE
        .get()
        .expect("Global stdlib cache not initialized. Call init_global_cache() first.")
}

/// Get the global stdlib cache, initializing if needed.
pub fn global_cache_or_init(project_root: &Path) -> &'static CoreCache {
    GLOBAL_CORE_CACHE.get_or_init(|| {
        CoreCache::new(project_root).expect("Failed to initialize stdlib cache")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_deterministic() {
        use crate::core_source::CoreSource;

        let source = CoreSource::auto_detect();
        let target = TargetConfig::host();

        let key1 = CoreCacheKey::new(&source, &target);
        let key2 = CoreCacheKey::new(&source, &target);

        assert_eq!(key1, key2);
        assert_eq!(key1.source_hash, key2.source_hash);
    }

    #[test]
    fn test_cache_filename() {
        let key = CoreCacheKey {
            compiler_version: "0.4.0".to_string(),
            target_triple: "aarch64-apple-macos".to_string(),
            source_hash: "abcdef1234567890".to_string(),
        };

        let filename = key.cache_filename();
        assert!(filename.contains("0_4_0"));
        assert!(filename.contains("aarch64"));
        assert!(filename.ends_with(".cache"));
    }
}
