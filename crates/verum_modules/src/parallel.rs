//! Parallel module loading infrastructure.
//!
//! Provides async parallel loading of independent modules using tokio.
//! Modules at the same dependency level can be loaded concurrently,
//! significantly improving compilation times for large projects.
//!
//! # Architecture
//!
//! ```text
//! Level 0: [A, B, C]  ──── Parallel Load ────┐
//!                                            │
//! Level 1: [D, E]     ──── Parallel Load ────┤──► Results
//!                                            │
//! Level 2: [F]        ──── Sequential Load ──┘
//! ```
//!
//! Modules at the same dependency level (determined by the dependency graph's
//! independent_groups algorithm) can be loaded concurrently, significantly
//! improving compilation times for large projects.

use crate::dependency::DependencyGraph;
use crate::error::{ModuleError, ModuleResult};
use crate::loader::ModuleLoader;
use crate::path::{ModuleId, ModulePath};
use crate::ModuleInfo;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use verum_common::{List, Map, Text};

/// Result of loading a single module in parallel.
#[derive(Debug)]
pub struct ParallelLoadResult {
    /// The module ID
    pub module_id: ModuleId,
    /// The module path
    pub module_path: ModulePath,
    /// The loaded module info (if successful)
    pub result: ModuleResult<ModuleInfo>,
}

/// Statistics for parallel loading operations.
#[derive(Debug, Clone, Default)]
pub struct ParallelLoadStats {
    /// Total modules loaded
    pub total_modules: usize,
    /// Modules loaded successfully
    pub successful: usize,
    /// Modules that failed to load
    pub failed: usize,
    /// Number of parallel batches executed
    pub batches: usize,
    /// Maximum parallelism achieved (largest batch)
    pub max_parallelism: usize,
}

/// Configuration for parallel module loading.
#[derive(Debug, Clone)]
pub struct ParallelLoadConfig {
    /// Maximum number of concurrent module loads
    pub max_concurrency: usize,
    /// Whether to continue loading after failures
    pub continue_on_error: bool,
    /// Whether to collect detailed statistics
    pub collect_stats: bool,
}

impl Default for ParallelLoadConfig {
    fn default() -> Self {
        Self {
            max_concurrency: num_cpus::get(),
            continue_on_error: false,
            collect_stats: true,
        }
    }
}

impl ParallelLoadConfig {
    /// Create config for maximum parallelism.
    pub fn max_parallel() -> Self {
        Self {
            max_concurrency: num_cpus::get() * 2,
            continue_on_error: false,
            collect_stats: false,
        }
    }

    /// Create config for development (continue on errors).
    pub fn development() -> Self {
        Self {
            max_concurrency: num_cpus::get(),
            continue_on_error: true,
            collect_stats: true,
        }
    }
}

/// Parallel module loader using tokio for async concurrent loading.
///
/// Loads modules in dependency order, processing independent modules
/// in parallel within each dependency level.
///
/// # Example
///
/// ```ignore
/// use verum_modules::{ParallelLoader, ModuleLoader, DependencyGraph};
///
/// let loader = ModuleLoader::new("src/");
/// let mut parallel = ParallelLoader::new(loader);
///
/// // Load all modules in parallel based on dependency graph
/// let results = parallel.load_all(&graph).await?;
/// ```
pub struct ParallelLoader {
    /// The underlying module loader (thread-safe)
    loader: Arc<Mutex<ModuleLoader>>,
    /// Configuration
    config: ParallelLoadConfig,
    /// Loading statistics
    stats: ParallelLoadStats,
}

impl ParallelLoader {
    /// Create a new parallel loader.
    pub fn new(loader: ModuleLoader) -> Self {
        Self {
            loader: Arc::new(Mutex::new(loader)),
            config: ParallelLoadConfig::default(),
            stats: ParallelLoadStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(loader: ModuleLoader, config: ParallelLoadConfig) -> Self {
        Self {
            loader: Arc::new(Mutex::new(loader)),
            config,
            stats: ParallelLoadStats::default(),
        }
    }

    /// Get current loading statistics.
    pub fn stats(&self) -> &ParallelLoadStats {
        &self.stats
    }

    /// Reset statistics.
    pub fn reset_stats(&mut self) {
        self.stats = ParallelLoadStats::default();
    }

    /// Load all modules in the dependency graph in parallel.
    ///
    /// Processes modules level by level, loading modules at the same
    /// dependency level concurrently.
    ///
    /// # Arguments
    ///
    /// * `graph` - The dependency graph defining module order
    ///
    /// # Returns
    ///
    /// Map of module IDs to loaded ModuleInfo, or error if any load fails
    /// (when continue_on_error is false).
    pub async fn load_all(
        &mut self,
        graph: &DependencyGraph,
    ) -> ModuleResult<Map<ModuleId, ModuleInfo>> {
        let groups = graph.independent_groups();
        let mut results: Map<ModuleId, ModuleInfo> = Map::new();

        self.stats.batches = groups.len();

        for group in groups.iter() {
            if self.config.collect_stats && group.len() > self.stats.max_parallelism {
                self.stats.max_parallelism = group.len();
            }

            let batch_results = self.load_batch(group, graph).await?;

            for (id, info) in batch_results {
                results.insert(id, info);
            }
        }

        Ok(results)
    }

    /// Load a batch of independent modules in parallel.
    async fn load_batch(
        &mut self,
        module_ids: &List<ModuleId>,
        graph: &DependencyGraph,
    ) -> ModuleResult<Vec<(ModuleId, ModuleInfo)>> {
        use tokio::sync::Semaphore;

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let mut handles = Vec::with_capacity(module_ids.len());

        for &module_id in module_ids.iter() {
            let path = match graph.module_path(module_id) {
                Some(p) => p.clone(),
                None => continue,
            };

            let loader = Arc::clone(&self.loader);
            let sem = Arc::clone(&semaphore);

            let handle = tokio::spawn(async move {
                // Semaphore is never closed; acquire only fails if closed
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return (module_id, path, Err(crate::error::ModuleError::Other { message: "semaphore closed".into(), span: None })),
                };
                let mut loader_guard = loader.lock().await;
                let result = loader_guard.load_and_parse(&path, module_id);
                (module_id, path, result)
            });

            handles.push(handle);
        }

        let mut batch_results = Vec::new();
        let mut errors = Vec::new();

        for handle in handles {
            match handle.await {
                Ok((id, path, result)) => {
                    self.stats.total_modules += 1;
                    match result {
                        Ok(info) => {
                            self.stats.successful += 1;
                            batch_results.push((id, info));
                        }
                        Err(e) => {
                            self.stats.failed += 1;
                            if self.config.continue_on_error {
                                // Log but continue
                                eprintln!("Warning: Failed to load module {}: {}", path, e);
                            } else {
                                errors.push(e);
                            }
                        }
                    }
                }
                Err(e) => {
                    self.stats.failed += 1;
                    if !self.config.continue_on_error {
                        return Err(ModuleError::Other {
                            message: Text::from(format!("Task join error: {}", e)),
                            span: None,
                        });
                    }
                }
            }
        }

        if !errors.is_empty() && !self.config.continue_on_error {
            return Err(errors.remove(0));
        }

        Ok(batch_results)
    }

    /// Load specific modules in parallel (not using dependency graph).
    ///
    /// Use this when you have a known set of independent modules.
    pub async fn load_modules(
        &mut self,
        modules: &[(ModuleId, ModulePath)],
    ) -> ModuleResult<Map<ModuleId, ModuleInfo>> {
        use tokio::sync::Semaphore;

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let mut handles = Vec::with_capacity(modules.len());

        for (module_id, path) in modules {
            let id = *module_id;
            let path = path.clone();
            let loader = Arc::clone(&self.loader);
            let sem = Arc::clone(&semaphore);

            let handle = tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return (id, Err(crate::error::ModuleError::Other { message: "semaphore closed".into(), span: None })),
                };
                let mut loader_guard = loader.lock().await;
                let result = loader_guard.load_and_parse(&path, id);
                (id, result)
            });

            handles.push(handle);
        }

        let mut results = Map::new();

        for handle in handles {
            match handle.await {
                Ok((id, result)) => {
                    self.stats.total_modules += 1;
                    match result {
                        Ok(info) => {
                            self.stats.successful += 1;
                            results.insert(id, info);
                        }
                        Err(e) => {
                            self.stats.failed += 1;
                            if !self.config.continue_on_error {
                                return Err(e);
                            }
                        }
                    }
                }
                Err(e) => {
                    self.stats.failed += 1;
                    if !self.config.continue_on_error {
                        return Err(ModuleError::Other {
                            message: Text::from(format!("Task join error: {}", e)),
                            span: None,
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}

impl std::fmt::Debug for ParallelLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelLoader")
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

// =============================================================================
// SYNCHRONOUS PARALLEL LOADING (using rayon)
// =============================================================================

/// Synchronous parallel module loader using rayon.
///
/// For environments where async is not available or preferred,
/// this provides parallel loading using rayon's thread pool.
///
/// # Example
///
/// ```ignore
/// use verum_modules::{SyncParallelLoader, ModuleLoader, DependencyGraph};
///
/// let loader = ModuleLoader::new("src/");
/// let mut parallel = SyncParallelLoader::new(loader);
///
/// let results = parallel.load_all(&graph)?;
/// ```
pub struct SyncParallelLoader {
    /// Root path for loading
    root_path: PathBuf,
    /// Configuration
    config: ParallelLoadConfig,
    /// Loading statistics
    stats: ParallelLoadStats,
}

impl SyncParallelLoader {
    /// Create a new sync parallel loader.
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            config: ParallelLoadConfig::default(),
            stats: ParallelLoadStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(root_path: impl Into<PathBuf>, config: ParallelLoadConfig) -> Self {
        Self {
            root_path: root_path.into(),
            config,
            stats: ParallelLoadStats::default(),
        }
    }

    /// Get current loading statistics.
    pub fn stats(&self) -> &ParallelLoadStats {
        &self.stats
    }

    /// Load all modules in the dependency graph using rayon.
    pub fn load_all(
        &mut self,
        graph: &DependencyGraph,
    ) -> ModuleResult<Map<ModuleId, ModuleInfo>> {
        use rayon::prelude::*;

        let groups = graph.independent_groups();
        let mut results: Map<ModuleId, ModuleInfo> = Map::new();

        self.stats.batches = groups.len();

        for group in groups.iter() {
            if self.config.collect_stats && group.len() > self.stats.max_parallelism {
                self.stats.max_parallelism = group.len();
            }

            // Collect paths for this batch
            let batch: Vec<_> = group
                .iter()
                .filter_map(|&id| graph.module_path(id).map(|p| (id, p.clone())))
                .collect();

            // Load in parallel using rayon
            let batch_results: Vec<_> = batch
                .par_iter()
                .map(|(id, path)| {
                    let mut loader = ModuleLoader::new(&self.root_path);
                    let result = loader.load_and_parse(path, *id);
                    (*id, result)
                })
                .collect();

            // Process results
            for (id, result) in batch_results {
                self.stats.total_modules += 1;
                match result {
                    Ok(info) => {
                        self.stats.successful += 1;
                        results.insert(id, info);
                    }
                    Err(e) => {
                        self.stats.failed += 1;
                        if !self.config.continue_on_error {
                            return Err(e);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Load specific modules in parallel using rayon.
    pub fn load_modules(
        &mut self,
        modules: &[(ModuleId, ModulePath)],
    ) -> ModuleResult<Map<ModuleId, ModuleInfo>> {
        use rayon::prelude::*;

        let batch_results: Vec<_> = modules
            .par_iter()
            .map(|(id, path)| {
                let mut loader = ModuleLoader::new(&self.root_path);
                let result = loader.load_and_parse(path, *id);
                (*id, result)
            })
            .collect();

        let mut results = Map::new();

        for (id, result) in batch_results {
            self.stats.total_modules += 1;
            match result {
                Ok(info) => {
                    self.stats.successful += 1;
                    results.insert(id, info);
                }
                Err(e) => {
                    self.stats.failed += 1;
                    if !self.config.continue_on_error {
                        return Err(e);
                    }
                }
            }
        }

        Ok(results)
    }
}

impl std::fmt::Debug for SyncParallelLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncParallelLoader")
            .field("root_path", &self.root_path)
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parallel_load_config_default() {
        let config = ParallelLoadConfig::default();
        assert!(config.max_concurrency > 0);
        assert!(!config.continue_on_error);
        assert!(config.collect_stats);
    }

    #[test]
    fn test_parallel_load_config_development() {
        let config = ParallelLoadConfig::development();
        assert!(config.continue_on_error);
        assert!(config.collect_stats);
    }

    #[test]
    fn test_parallel_load_stats_default() {
        let stats = ParallelLoadStats::default();
        assert_eq!(stats.total_modules, 0);
        assert_eq!(stats.successful, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.batches, 0);
        assert_eq!(stats.max_parallelism, 0);
    }

    #[test]
    fn test_sync_parallel_loader_new() {
        let loader = SyncParallelLoader::new("/tmp");
        assert_eq!(loader.root_path, PathBuf::from("/tmp"));
    }
}
