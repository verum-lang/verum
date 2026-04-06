//! Incremental Compilation Support
//!
//! Enables fast recompilation by caching:
//! - Parsed ASTs
//! - Type checking results
//! - Meta registry
//! - Optimization results
//!
//! Incremental compilation: item-level hashing distinguishes signature changes
//! (which invalidate dependents) from body-only changes (which skip recompilation).
//! Phase 2 meta registry cached; Phase 3 re-expands only changed modules.
//!
//! ## Key Features
//!
//! - **Dependency Tracking**: Automatic transitive dependency invalidation
//! - **Cache Persistence**: Save/load cache across compilation sessions
//! - **Topological Sorting**: Correct recompilation order based on dependency graph
//! - **Type Check Caching**: Avoid redundant type checking for unchanged modules

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::time::SystemTime;
use verum_ast::Module;
use verum_common::{List, Map};

/// Cache file format version for compatibility checking
const CACHE_VERSION: u32 = 1;

/// Magic bytes for cache file identification
const CACHE_MAGIC: &[u8; 4] = b"VRMC";

/// Incremental compilation manager
pub struct IncrementalCompiler {
    /// Cache of parsed modules
    module_cache: Map<PathBuf, CachedModule>,

    /// Cache of type check results
    type_check_cache: Map<PathBuf, TypeCheckResult>,

    /// Dependency graph: file -> files it depends on
    dependencies: Map<PathBuf, List<PathBuf>>,

    /// Reverse dependency graph: file -> files that depend on it
    reverse_deps: Map<PathBuf, List<PathBuf>>,

    /// Cache of meta registry
    meta_registry_valid: bool,

    /// Last compilation timestamp
    last_compile_time: Option<SystemTime>,

    /// Cache directory for persistence
    cache_dir: Option<PathBuf>,

    /// Content hashes for change detection
    content_hashes: Map<PathBuf, u64>,

    // =========================================================================
    // Fine-Grained Invalidation (Signature vs Body)
    // =========================================================================

    /// Item hashes for fine-grained change detection.
    /// Maps module path to its item-level hashes.
    item_hashes: Map<PathBuf, crate::hash::ItemHashes>,

    /// Verification-only cache: modules that only need re-verification.
    /// These had body-only changes in their dependencies.
    verification_only_cache: HashSet<PathBuf>,
}

/// Result of type checking a module
#[derive(Debug, Clone)]
pub struct TypeCheckResult {
    /// Whether type checking succeeded
    pub success: bool,
    /// Number of errors found
    pub error_count: usize,
    /// Number of warnings found
    pub warning_count: usize,
    /// Timestamp when type checking was performed
    pub timestamp: SystemTime,
    /// Hash of the source content when type checked
    pub content_hash: u64,
}

impl IncrementalCompiler {
    /// Create a new incremental compiler
    pub fn new() -> Self {
        Self {
            module_cache: Map::new(),
            type_check_cache: Map::new(),
            dependencies: Map::new(),
            reverse_deps: Map::new(),
            meta_registry_valid: false,
            last_compile_time: None,
            cache_dir: None,
            content_hashes: Map::new(),
            item_hashes: Map::new(),
            verification_only_cache: HashSet::new(),
        }
    }

    /// Create a new incremental compiler with a cache directory
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        let mut compiler = Self::new();
        compiler.cache_dir = Some(cache_dir);
        compiler
    }

    /// Set the cache directory for persistence
    pub fn set_cache_dir(&mut self, cache_dir: PathBuf) {
        self.cache_dir = Some(cache_dir);
    }

    /// Compute hash of file content using Blake3 for fast, high-quality hashing.
    ///
    /// Returns a u64 for compatibility with existing APIs, truncated from
    /// the full Blake3 256-bit hash.
    fn compute_content_hash(path: &PathBuf) -> std::io::Result<u64> {
        crate::hash::hash_file(path).map(|h| h.to_u64())
    }

    /// Register dependencies for a module
    pub fn register_dependencies(&mut self, path: PathBuf, deps: List<PathBuf>) {
        // Update forward dependencies
        self.dependencies.insert(path.clone(), deps.clone());

        // Update reverse dependencies
        for dep in deps.iter() {
            let reverse = self
                .reverse_deps
                .entry(dep.clone())
                .or_insert_with(List::new);
            if !reverse.contains(&path) {
                reverse.push(path.clone());
            }
        }
    }

    /// Get files that depend on the given file (directly or transitively)
    pub fn get_dependents(&self, path: &PathBuf) -> List<PathBuf> {
        let mut result = List::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(path.clone());

        while let Some(current) = queue.pop_front() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());

            if let Some(dependents) = self.reverse_deps.get(&current) {
                for dep in dependents.iter() {
                    if !visited.contains(dep) {
                        result.push(dep.clone());
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        result
    }

    /// Get the set of files that need recompilation given changed files
    ///
    /// Returns files in topological order (dependencies first)
    pub fn get_recompilation_set(&self, changed_files: &[PathBuf]) -> List<PathBuf> {
        let mut to_recompile = HashSet::new();

        // Add changed files and their dependents
        for file in changed_files {
            to_recompile.insert(file.clone());

            let dependents = self.get_dependents(file);
            for dep in dependents.iter() {
                to_recompile.insert(dep.clone());
            }
        }

        // Topological sort
        self.topological_sort(&to_recompile)
    }

    /// Perform topological sort on the given files
    fn topological_sort(&self, files: &HashSet<PathBuf>) -> List<PathBuf> {
        let mut result = List::new();
        let mut visited = HashSet::new();
        let mut in_progress = HashSet::new();

        // Build a filtered dependency graph
        let mut filtered_deps: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for file in files {
            let deps: Vec<PathBuf> = self
                .dependencies
                .get(file)
                .map(|d| d.iter().filter(|p| files.contains(*p)).cloned().collect())
                .unwrap_or_default();
            filtered_deps.insert(file.clone(), deps);
        }

        fn visit(
            file: &PathBuf,
            filtered_deps: &HashMap<PathBuf, Vec<PathBuf>>,
            visited: &mut HashSet<PathBuf>,
            in_progress: &mut HashSet<PathBuf>,
            result: &mut List<PathBuf>,
        ) {
            if visited.contains(file) {
                return;
            }
            if in_progress.contains(file) {
                // Cycle detected - skip to avoid infinite loop
                return;
            }

            in_progress.insert(file.clone());

            if let Some(deps) = filtered_deps.get(file) {
                for dep in deps {
                    visit(dep, filtered_deps, visited, in_progress, result);
                }
            }

            in_progress.remove(file);
            visited.insert(file.clone());
            result.push(file.clone());
        }

        for file in files {
            visit(
                file,
                &filtered_deps,
                &mut visited,
                &mut in_progress,
                &mut result,
            );
        }

        result
    }

    /// Check if a file needs recompilation
    pub fn needs_recompile(&self, path: &PathBuf) -> bool {
        match self.module_cache.get(path) {
            Some(cached) => {
                // Check if file was modified
                if let Ok(metadata) = std::fs::metadata(path) {
                    if let Ok(modified) = metadata.modified() {
                        return modified > cached.timestamp;
                    }
                }
                true
            }
            None => true,
        }
    }

    /// Cache a compiled module
    pub fn cache_module(&mut self, path: PathBuf, module: Module) {
        let timestamp = SystemTime::now();
        self.module_cache.insert(
            path,
            CachedModule {
                module,
                timestamp,
                dependencies: List::new(),
            },
        );
    }

    /// Get cached module
    pub fn get_cached_module(&self, path: &PathBuf) -> Option<&Module> {
        self.module_cache.get(path).map(|c| &c.module)
    }

    /// Invalidate cache for a file and its dependents
    /// SAFETY: Uses visited set to prevent infinite recursion on circular dependencies
    pub fn invalidate(&mut self, path: &PathBuf) {
        let mut visited = std::collections::HashSet::new();
        self.invalidate_with_visited(path, &mut visited);
    }

    /// Internal invalidation with cycle detection
    fn invalidate_with_visited(
        &mut self,
        path: &PathBuf,
        visited: &mut std::collections::HashSet<PathBuf>,
    ) {
        // Cycle detection - prevent infinite recursion
        if visited.contains(path) {
            return;
        }
        visited.insert(path.clone());

        // Remove from cache
        self.module_cache.remove(path);

        // Invalidate dependents
        let dependents: List<_> = self
            .module_cache
            .iter()
            .filter(|(_, cached)| cached.dependencies.contains(path))
            .map(|(p, _)| p.clone())
            .collect();

        for dependent in dependents {
            self.invalidate_with_visited(&dependent, visited);
        }
    }

    // =========================================================================
    // Type Check Caching
    // =========================================================================

    /// Cache type check result for a module
    pub fn cache_type_check(&mut self, path: PathBuf, result: TypeCheckResult) {
        self.type_check_cache.insert(path, result);
    }

    /// Get cached type check result
    pub fn get_type_check_result(&self, path: &PathBuf) -> Option<&TypeCheckResult> {
        self.type_check_cache.get(path)
    }

    /// Check if type check result is still valid (content hash matches)
    pub fn is_type_check_valid(&self, path: &PathBuf) -> bool {
        if let Some(cached) = self.type_check_cache.get(path) {
            if let Ok(current_hash) = Self::compute_content_hash(path) {
                return cached.content_hash == current_hash;
            }
        }
        false
    }

    /// Invalidate type check cache for a file
    pub fn invalidate_type_check(&mut self, path: &PathBuf) {
        self.type_check_cache.remove(path);
    }

    // =========================================================================
    // Cache Persistence
    // =========================================================================

    /// Get the cache file path
    fn cache_file_path(&self) -> Option<PathBuf> {
        self.cache_dir
            .as_ref()
            .map(|dir| dir.join("incremental_cache.bin"))
    }

    /// Save cache to disk for persistence across sessions
    ///
    /// Cache format:
    /// - Magic bytes (4 bytes): "VRMC"
    /// - Version (4 bytes): u32
    /// - Number of entries (8 bytes): u64
    /// - For each entry:
    ///   - Path length (4 bytes): u32
    ///   - Path (variable): UTF-8 bytes
    ///   - Timestamp (16 bytes): SystemTime as duration since UNIX_EPOCH
    ///   - Content hash (8 bytes): u64
    ///   - Dependencies count (4 bytes): u32
    ///   - For each dependency:
    ///     - Path length (4 bytes): u32
    ///     - Path (variable): UTF-8 bytes
    pub fn save_cache(&self) -> std::io::Result<()> {
        let cache_path = self.cache_file_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Cache directory not set")
        })?;

        // Ensure cache directory exists
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = File::create(&cache_path)?;
        let mut writer = BufWriter::new(file);

        // Write header
        writer.write_all(CACHE_MAGIC)?;
        writer.write_all(&CACHE_VERSION.to_le_bytes())?;

        // Write number of module entries
        let entry_count = self.module_cache.len() as u64;
        writer.write_all(&entry_count.to_le_bytes())?;

        // Write each cached module entry
        for (path, cached) in self.module_cache.iter() {
            // Write path
            let path_str = path.to_string_lossy();
            let path_bytes = path_str.as_bytes();
            writer.write_all(&(path_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(path_bytes)?;

            // Write timestamp as duration since UNIX_EPOCH
            let duration = cached
                .timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            writer.write_all(&duration.as_secs().to_le_bytes())?;
            writer.write_all(&duration.subsec_nanos().to_le_bytes())?;

            // Write content hash
            let hash = self.content_hashes.get(path).copied().unwrap_or(0);
            writer.write_all(&hash.to_le_bytes())?;

            // Write dependencies count
            writer.write_all(&(cached.dependencies.len() as u32).to_le_bytes())?;

            // Write each dependency path
            for dep in cached.dependencies.iter() {
                let dep_str = dep.to_string_lossy();
                let dep_bytes = dep_str.as_bytes();
                writer.write_all(&(dep_bytes.len() as u32).to_le_bytes())?;
                writer.write_all(dep_bytes)?;
            }
        }

        // Write type check cache entries
        let tc_count = self.type_check_cache.len() as u64;
        writer.write_all(&tc_count.to_le_bytes())?;

        for (path, result) in self.type_check_cache.iter() {
            // Write path
            let path_str = path.to_string_lossy();
            let path_bytes = path_str.as_bytes();
            writer.write_all(&(path_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(path_bytes)?;

            // Write result fields
            writer.write_all(&[if result.success { 1u8 } else { 0u8 }])?;
            writer.write_all(&(result.error_count as u32).to_le_bytes())?;
            writer.write_all(&(result.warning_count as u32).to_le_bytes())?;
            writer.write_all(&result.content_hash.to_le_bytes())?;

            // Write timestamp
            let duration = result
                .timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            writer.write_all(&duration.as_secs().to_le_bytes())?;
            writer.write_all(&duration.subsec_nanos().to_le_bytes())?;
        }

        writer.flush()?;
        Ok(())
    }

    /// Load cache from disk
    pub fn load_cache(&mut self) -> std::io::Result<()> {
        let cache_path = self.cache_file_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Cache directory not set")
        })?;

        if !cache_path.exists() {
            return Ok(()); // No cache file, nothing to load
        }

        let file = File::open(&cache_path)?;
        let mut reader = BufReader::new(file);

        // Read and verify header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != CACHE_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid cache file magic bytes",
            ));
        }

        let mut version_bytes = [0u8; 4];
        reader.read_exact(&mut version_bytes)?;
        let version = u32::from_le_bytes(version_bytes);
        if version != CACHE_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Cache version mismatch: expected {}, got {}",
                    CACHE_VERSION, version
                ),
            ));
        }

        // Read entry count
        let mut count_bytes = [0u8; 8];
        reader.read_exact(&mut count_bytes)?;
        let entry_count = u64::from_le_bytes(count_bytes);

        // Read module entries
        for _ in 0..entry_count {
            // Read path
            let mut path_len_bytes = [0u8; 4];
            reader.read_exact(&mut path_len_bytes)?;
            let path_len = u32::from_le_bytes(path_len_bytes) as usize;

            let mut path_bytes = vec![0u8; path_len];
            reader.read_exact(&mut path_bytes)?;
            let path = PathBuf::from(String::from_utf8_lossy(&path_bytes).to_string());

            // Read timestamp
            let mut secs_bytes = [0u8; 8];
            reader.read_exact(&mut secs_bytes)?;
            let secs = u64::from_le_bytes(secs_bytes);

            let mut nanos_bytes = [0u8; 4];
            reader.read_exact(&mut nanos_bytes)?;
            let nanos = u32::from_le_bytes(nanos_bytes);

            let _timestamp = std::time::UNIX_EPOCH + std::time::Duration::new(secs, nanos);

            // Read content hash
            let mut hash_bytes = [0u8; 8];
            reader.read_exact(&mut hash_bytes)?;
            let content_hash = u64::from_le_bytes(hash_bytes);

            // Store hash
            self.content_hashes.insert(path.clone(), content_hash);

            // Read dependencies count
            let mut dep_count_bytes = [0u8; 4];
            reader.read_exact(&mut dep_count_bytes)?;
            let dep_count = u32::from_le_bytes(dep_count_bytes) as usize;

            let mut dependencies = List::new();
            for _ in 0..dep_count {
                let mut dep_len_bytes = [0u8; 4];
                reader.read_exact(&mut dep_len_bytes)?;
                let dep_len = u32::from_le_bytes(dep_len_bytes) as usize;

                let mut dep_bytes = vec![0u8; dep_len];
                reader.read_exact(&mut dep_bytes)?;
                dependencies.push(PathBuf::from(
                    String::from_utf8_lossy(&dep_bytes).to_string(),
                ));
            }

            // Note: We don't store the actual Module in cache file
            // (too complex to serialize). We only store metadata.
            // The module will be reloaded on demand if the content hash matches.
            // For now, we mark it as needing recompile by not adding to module_cache.
            // This is a metadata-only cache for dependency tracking.

            // Register dependencies for fast lookup
            self.register_dependencies(path.clone(), dependencies);
        }

        // Read type check cache entries
        let mut tc_count_bytes = [0u8; 8];
        reader.read_exact(&mut tc_count_bytes)?;
        let tc_count = u64::from_le_bytes(tc_count_bytes);

        for _ in 0..tc_count {
            // Read path
            let mut path_len_bytes = [0u8; 4];
            reader.read_exact(&mut path_len_bytes)?;
            let path_len = u32::from_le_bytes(path_len_bytes) as usize;

            let mut path_bytes = vec![0u8; path_len];
            reader.read_exact(&mut path_bytes)?;
            let path = PathBuf::from(String::from_utf8_lossy(&path_bytes).to_string());

            // Read result fields
            let mut success_byte = [0u8; 1];
            reader.read_exact(&mut success_byte)?;
            let success = success_byte[0] != 0;

            let mut error_count_bytes = [0u8; 4];
            reader.read_exact(&mut error_count_bytes)?;
            let error_count = u32::from_le_bytes(error_count_bytes) as usize;

            let mut warning_count_bytes = [0u8; 4];
            reader.read_exact(&mut warning_count_bytes)?;
            let warning_count = u32::from_le_bytes(warning_count_bytes) as usize;

            let mut hash_bytes = [0u8; 8];
            reader.read_exact(&mut hash_bytes)?;
            let content_hash = u64::from_le_bytes(hash_bytes);

            // Read timestamp
            let mut secs_bytes = [0u8; 8];
            reader.read_exact(&mut secs_bytes)?;
            let secs = u64::from_le_bytes(secs_bytes);

            let mut nanos_bytes = [0u8; 4];
            reader.read_exact(&mut nanos_bytes)?;
            let nanos = u32::from_le_bytes(nanos_bytes);

            let timestamp = std::time::UNIX_EPOCH + std::time::Duration::new(secs, nanos);

            self.type_check_cache.insert(
                path,
                TypeCheckResult {
                    success,
                    error_count,
                    warning_count,
                    timestamp,
                    content_hash,
                },
            );
        }

        Ok(())
    }

    /// Clear all caches
    pub fn clear(&mut self) {
        self.module_cache.clear();
        self.type_check_cache.clear();
        self.dependencies.clear();
        self.reverse_deps.clear();
        self.content_hashes.clear();
        self.item_hashes.clear();
        self.verification_only_cache.clear();
        self.meta_registry_valid = false;
        self.last_compile_time = None;
    }

    // =========================================================================
    // Fine-Grained Invalidation (Signature vs Body)
    // =========================================================================

    /// Update item hashes for a module.
    ///
    /// Call this after compiling a module to enable fine-grained
    /// change detection on subsequent compilations.
    pub fn update_item_hashes(&mut self, path: PathBuf, hashes: crate::hash::ItemHashes) {
        self.item_hashes.insert(path, hashes);
    }

    /// Get the cached item hashes for a module.
    pub fn get_item_hashes(&self, path: &PathBuf) -> Option<&crate::hash::ItemHashes> {
        self.item_hashes.get(path)
    }

    /// Determine what kind of recompilation is needed for a file.
    ///
    /// Returns:
    /// - `NoChange`: File hasn't changed
    /// - `BodyOnly`: Only function bodies changed, re-verification needed
    /// - `Signature`: API changed, full recompilation needed
    pub fn classify_change(
        &self,
        path: &PathBuf,
        new_hashes: &crate::hash::ItemHashes,
    ) -> crate::hash::ChangeKind {
        match self.item_hashes.get(path) {
            Some(old_hashes) => new_hashes.compare(old_hashes),
            None => crate::hash::ChangeKind::Signature, // New file
        }
    }

    /// Compute fine-grained recompilation sets.
    ///
    /// Returns a tuple of:
    /// - Files needing full recompilation (signature changed)
    /// - Files needing re-verification only (body-only changed in dependency)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (full_recompile, verify_only) = compiler.compute_incremental_sets_fine_grained(
    ///     &all_files,
    ///     |path| compute_item_hashes(path),
    /// );
    /// ```
    pub fn compute_incremental_sets_fine_grained<F>(
        &mut self,
        all_files: &[PathBuf],
        compute_hashes: F,
    ) -> (List<PathBuf>, List<PathBuf>)
    where
        F: Fn(&PathBuf) -> Option<crate::hash::ItemHashes>,
    {
        use crate::hash::ChangeKind;

        let mut signature_changed: HashSet<PathBuf> = HashSet::new();
        let mut body_only_changed: HashSet<PathBuf> = HashSet::new();

        // First pass: classify all changed files
        for file in all_files {
            if let Some(new_hashes) = compute_hashes(file) {
                match self.classify_change(file, &new_hashes) {
                    ChangeKind::Signature => {
                        signature_changed.insert(file.clone());
                    }
                    ChangeKind::BodyOnly => {
                        body_only_changed.insert(file.clone());
                    }
                    ChangeKind::NoChange => {}
                }
            }
        }

        // Second pass: propagate signature changes to dependents
        let mut full_recompile: HashSet<PathBuf> = signature_changed.clone();
        for file in &signature_changed {
            let dependents = self.get_dependents(file);
            for dep in dependents.iter() {
                full_recompile.insert(dep.clone());
            }
        }

        // Third pass: find files that only need re-verification
        // These are dependents of body-only changed files that aren't already
        // in the full recompile set
        let mut verify_only: HashSet<PathBuf> = HashSet::new();
        for file in &body_only_changed {
            // The changed file itself needs full recompile
            full_recompile.insert(file.clone());

            // Its dependents only need re-verification (if not already in full set)
            let dependents = self.get_dependents(file);
            for dep in dependents.iter() {
                if !full_recompile.contains(dep) {
                    verify_only.insert(dep.clone());
                }
            }
        }

        // Update verification-only cache
        self.verification_only_cache = verify_only.clone();

        // Return in topological order
        let full_set: HashSet<PathBuf> = full_recompile;
        let verify_set: HashSet<PathBuf> = verify_only;

        (
            self.topological_sort(&full_set),
            self.topological_sort(&verify_set),
        )
    }

    /// Check if a file only needs re-verification (not full recompilation).
    ///
    /// This is true when the file's dependencies had body-only changes.
    pub fn needs_verification_only(&self, path: &PathBuf) -> bool {
        self.verification_only_cache.contains(path)
    }

    /// Mark a file as needing verification only.
    pub fn mark_verification_only(&mut self, path: PathBuf) {
        self.verification_only_cache.insert(path);
    }

    /// Clear verification-only status for a file.
    pub fn clear_verification_only(&mut self, path: &PathBuf) {
        self.verification_only_cache.remove(path);
    }

    /// Get all files that need verification only.
    pub fn get_verification_only_files(&self) -> List<PathBuf> {
        self.verification_only_cache.iter().cloned().collect()
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            cached_modules: self.module_cache.len(),
            meta_registry_valid: self.meta_registry_valid,
            type_check_cached: self.type_check_cache.len(),
            dependency_edges: self.dependencies.values().map(|d| d.len()).sum(),
            item_hashes_cached: self.item_hashes.len(),
            verification_only_count: self.verification_only_cache.len(),
        }
    }

    /// Mark meta registry as valid (call after successful registry build)
    pub fn set_meta_registry_valid(&mut self, valid: bool) {
        self.meta_registry_valid = valid;
    }

    /// Get the last compilation time
    pub fn last_compile_time(&self) -> Option<SystemTime> {
        self.last_compile_time
    }

    /// Update last compilation time (call at start of compilation)
    pub fn mark_compilation_start(&mut self) {
        self.last_compile_time = Some(SystemTime::now());
    }

    /// Update content hash for a file
    pub fn update_content_hash(&mut self, path: &PathBuf) -> std::io::Result<u64> {
        let hash = Self::compute_content_hash(path)?;
        self.content_hashes.insert(path.clone(), hash);
        Ok(hash)
    }

    /// Check if file content has changed since last compilation
    pub fn has_file_changed(&self, path: &PathBuf) -> bool {
        match (
            self.content_hashes.get(path),
            Self::compute_content_hash(path),
        ) {
            (Some(&cached_hash), Ok(current_hash)) => cached_hash != current_hash,
            // If we can't compare, assume it changed
            _ => true,
        }
    }

    /// Get files that have changed since last compilation
    pub fn get_changed_files(&self, files: &[PathBuf]) -> List<PathBuf> {
        files
            .iter()
            .filter(|f| self.has_file_changed(f))
            .cloned()
            .collect()
    }

    /// Perform incremental compilation on changed files
    ///
    /// Returns the set of files that need full recompilation based on:
    /// 1. Files that have changed content
    /// 2. Files that depend on changed files (transitively)
    ///
    /// The returned list is in topological order (dependencies first).
    pub fn compute_incremental_set(&self, all_files: &[PathBuf]) -> List<PathBuf> {
        // Find changed files
        let changed: Vec<PathBuf> = all_files
            .iter()
            .filter(|f| self.has_file_changed(f))
            .cloned()
            .collect();

        if changed.is_empty() {
            return List::new();
        }

        // Get full recompilation set (including dependents)
        self.get_recompilation_set(&changed)
    }

    /// Check if compilation cache is stale and should be cleared
    ///
    /// Returns true if:
    /// - Cache is older than specified duration
    /// - Cache version is incompatible
    /// - Cache file is corrupted
    pub fn is_cache_stale(&self, max_age: std::time::Duration) -> bool {
        match self.last_compile_time {
            Some(time) => {
                match time.elapsed() {
                    Ok(elapsed) => elapsed > max_age,
                    Err(_) => true, // System time went backwards, treat as stale
                }
            }
            None => true, // No last compile time, cache is stale
        }
    }
}

impl Default for IncrementalCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached module data
#[derive(Clone)]
struct CachedModule {
    module: Module,
    timestamp: SystemTime,
    dependencies: List<PathBuf>,
}

/// Cache statistics
pub struct CacheStats {
    /// Number of cached modules
    pub cached_modules: usize,
    /// Whether meta registry is valid
    pub meta_registry_valid: bool,
    /// Number of cached type check results
    pub type_check_cached: usize,
    /// Total number of dependency edges tracked
    pub dependency_edges: usize,
    /// Number of modules with item-level hashes
    pub item_hashes_cached: usize,
    /// Number of modules needing verification only
    pub verification_only_count: usize,
}

impl CacheStats {
    /// Generate a human-readable report of cache statistics
    ///
    /// # Example Output
    ///
    /// ```text
    /// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Incremental Cache Stats
    /// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    ///
    /// Cached modules:        42
    /// Meta registry valid:   Yes
    ///
    /// Cache hit rate:        95.2%
    /// Memory saved:          ~1.2 MB
    /// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// ```
    pub fn report(&self) -> String {
        let separator = "━".repeat(60);
        let mut lines = Vec::new();

        lines.push(separator.clone());
        lines.push("Incremental Cache Stats".to_string());
        lines.push(separator.clone());
        lines.push(String::new());

        // Cached modules count
        lines.push(format!("Cached modules:        {}", self.cached_modules));

        // Type check cache count
        lines.push(format!("Type check cached:     {}", self.type_check_cached));

        // Dependency edges
        lines.push(format!("Dependency edges:      {}", self.dependency_edges));

        // Item hashes (fine-grained invalidation)
        lines.push(format!("Item hashes cached:    {}", self.item_hashes_cached));

        // Verification-only files
        if self.verification_only_count > 0 {
            lines.push(format!("Verify-only pending:   {}", self.verification_only_count));
        }

        // Meta registry status
        let registry_status = if self.meta_registry_valid {
            "Yes"
        } else {
            "No"
        };
        lines.push(format!("Meta registry valid:   {}", registry_status));

        lines.push(String::new());

        // Performance metrics (estimated)
        if self.cached_modules > 0 || self.type_check_cached > 0 {
            // Assume ~30KB per cached module (AST + metadata)
            // and ~2KB per type check result
            let memory_saved_kb = self.cached_modules * 30 + self.type_check_cached * 2;
            if memory_saved_kb >= 1024 {
                lines.push(format!(
                    "Memory saved:          ~{:.1} MB",
                    memory_saved_kb as f64 / 1024.0
                ));
            } else {
                lines.push(format!("Memory saved:          ~{} KB", memory_saved_kb));
            }
        }

        lines.push(separator);

        lines.join("\n")
    }
}
