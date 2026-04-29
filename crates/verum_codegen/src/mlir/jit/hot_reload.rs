//! Hot Code Replacement for JIT.
//!
//! Enables live code updates without restarting the application.
//! Functions can be replaced at runtime while maintaining state.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                     Hot Code Replacement Pipeline                            │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//!   Updated Source
//!         │
//!         ▼
//! ┌─────────────────┐    ┌─────────────────┐
//! │  Change Detect  │───▶│   Validation    │  Type compatibility check
//! │  (file watch)   │    │   (signature)   │
//! └─────────────────┘    └────────┬────────┘
//!                                 │
//!                                 ▼
//!                        ┌─────────────────┐
//!                        │  Compile New    │  JIT compile new version
//!                        │  Version        │
//!                        └────────┬────────┘
//!                                 │
//!                    ┌────────────┼────────────┐
//!                    │            │            │
//!              ┌─────▼─────┐ ┌────▼────┐ ┌─────▼─────┐
//!              │  Suspend  │ │  Swap   │ │  Resume   │
//!              │  Callers  │ │  Ptr    │ │  Callers  │
//!              └───────────┘ └─────────┘ └───────────┘
//! ```
//!
//! # Safety
//!
//! Hot code replacement is inherently unsafe. The system provides:
//! - Signature validation (parameter/return type checking)
//! - Version tracking
//! - Rollback capability
//! - State migration hooks
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{HotReloader, HotReloadConfig};
//!
//! let mut reloader = HotReloader::new(engine, HotReloadConfig::default())?;
//!
//! // Replace a function
//! reloader.replace_function("process", new_module)?;
//!
//! // Rollback if needed
//! if something_wrong {
//!     reloader.rollback("process")?;
//! }
//! ```

use crate::mlir::error::{MlirError, Result};
use crate::mlir::jit::{JitEngine, JitConfig, CompiledFunction};
use dashmap::DashMap;
use verum_mlir::ir::Module;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use verum_common::Text;

// ============================================================================
// Hot Reload Configuration
// ============================================================================

/// Configuration for hot code replacement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotReloadConfig {
    /// Enable signature validation.
    pub validate_signatures: bool,

    /// Keep N previous versions for rollback.
    pub version_history: usize,

    /// Enable state migration hooks.
    pub enable_migration: bool,

    /// Maximum replacement time (microseconds).
    pub max_replacement_time_us: u64,

    /// Enable verbose logging.
    pub verbose: bool,

    /// Atomic replacement (suspend all callers).
    pub atomic_replacement: bool,
}

impl HotReloadConfig {
    /// Create new configuration.
    pub fn new() -> Self {
        Self {
            validate_signatures: true,
            version_history: 5,
            enable_migration: true,
            max_replacement_time_us: 1_000_000, // 1 second
            verbose: false,
            atomic_replacement: true,
        }
    }

    /// Builder: enable/disable signature validation.
    pub fn validate_signatures(mut self, enabled: bool) -> Self {
        self.validate_signatures = enabled;
        self
    }

    /// Builder: set version history size.
    pub fn version_history(mut self, size: usize) -> Self {
        self.version_history = size;
        self
    }

    /// Builder: enable/disable migration hooks.
    pub fn enable_migration(mut self, enabled: bool) -> Self {
        self.enable_migration = enabled;
        self
    }

    /// Builder: set verbose mode.
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }

    /// Builder: enable/disable atomic replacement.
    pub fn atomic_replacement(mut self, enabled: bool) -> Self {
        self.atomic_replacement = enabled;
        self
    }

    /// Create development configuration.
    pub fn development() -> Self {
        Self::new()
            .verbose(true)
            .version_history(10)
    }

    /// Create production configuration (stricter).
    pub fn production() -> Self {
        Self::new()
            .validate_signatures(true)
            .atomic_replacement(true)
            .version_history(3)
    }
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Function Version
// ============================================================================

/// A version of a function.
#[derive(Debug, Clone)]
pub struct FunctionVersion {
    /// Version number.
    pub version: u64,

    /// Function address.
    pub address: *mut (),

    /// Function signature hash (for validation).
    pub signature_hash: u64,

    /// Creation timestamp.
    pub created_at: instant::Instant,

    /// Source hash (for identifying changes).
    pub source_hash: [u8; 32],

    /// Whether this is the active version.
    pub active: bool,
}

// SAFETY: FunctionVersion can be sent/shared across threads because:
// - `address` points to JIT-compiled machine code that is immutable once compiled
// - The JIT memory region outlives all FunctionVersion instances
// - All other fields are plain data types (u64, [u8; 32], bool, Instant)
unsafe impl Send for FunctionVersion {}
unsafe impl Sync for FunctionVersion {}

impl FunctionVersion {
    /// Create a new function version.
    pub fn new(
        version: u64,
        address: *mut (),
        signature_hash: u64,
        source_hash: [u8; 32],
    ) -> Self {
        Self {
            version,
            address,
            signature_hash,
            created_at: instant::Instant::now(),
            source_hash,
            active: false,
        }
    }

    /// Get age in microseconds.
    pub fn age_us(&self) -> u64 {
        self.created_at.elapsed().as_micros() as u64
    }
}

// ============================================================================
// Function Entry (with versioning)
// ============================================================================

/// Entry for a hot-reloadable function.
pub struct HotFunction {
    /// Function name.
    pub name: Text,

    /// Current version.
    pub current_version: u64,

    /// All versions (for rollback).
    pub versions: Vec<FunctionVersion>,

    /// Indirection pointer (points to current implementation).
    /// Callers go through this for hot swapping.
    /// Uses AtomicPtr for thread-safe pointer swapping during hot reload.
    indirection: AtomicPtr<()>,

    /// Whether function is currently being replaced.
    pub replacing: AtomicBool,

    /// Number of active calls.
    pub active_calls: AtomicU64,

    /// Total replacements.
    pub replacement_count: AtomicU64,
}

// SAFETY: HotFunction can be sent/shared across threads because:
// - `indirection` is an AtomicPtr, inherently thread-safe for pointer swaps
// - `replacing` is AtomicBool, `active_calls` and `replacement_count` are AtomicU64
// - `versions` is only mutated through &mut self (exclusive access)
// - All function pointers point to JIT code that outlives HotFunction
unsafe impl Send for HotFunction {}
unsafe impl Sync for HotFunction {}

impl HotFunction {
    /// Create a new hot function entry.
    ///
    /// # Safety
    ///
    /// The address must point to valid JIT-compiled code.
    pub unsafe fn new(name: impl Into<Text>, initial_address: *mut (), signature_hash: u64) -> Self {
        let version = FunctionVersion::new(0, initial_address, signature_hash, [0u8; 32]);
        let mut versions = Vec::with_capacity(5);
        versions.push(FunctionVersion { active: true, ..version });

        Self {
            name: name.into(),
            current_version: 0,
            versions,
            indirection: AtomicPtr::new(initial_address),
            replacing: AtomicBool::new(false),
            active_calls: AtomicU64::new(0),
            replacement_count: AtomicU64::new(0),
        }
    }

    /// Get the indirection pointer (for callers that need a raw pointer).
    /// Callers should use this instead of direct address.
    pub fn indirection_ptr(&self) -> *const AtomicPtr<()> {
        &self.indirection as *const AtomicPtr<()>
    }

    /// Get current active address (thread-safe atomic load).
    pub fn current_address(&self) -> *mut () {
        self.indirection.load(Ordering::Acquire)
    }

    /// Replace with new version.
    ///
    /// # Safety
    ///
    /// The new address must point to valid JIT-compiled code.
    pub unsafe fn replace(&mut self, new_address: *mut (), signature_hash: u64, source_hash: [u8; 32]) -> Result<()> {
        // Mark as replacing
        if self.replacing.compare_exchange(
            false,
            true,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ).is_err() {
            return Err(MlirError::HotCodeError {
                message: Text::from("Function is already being replaced"),
            });
        }

        // Wait for active calls to complete (with timeout)
        let start = instant::Instant::now();
        while self.active_calls.load(Ordering::Acquire) > 0 {
            if start.elapsed().as_micros() > 100_000 {
                // 100ms timeout
                self.replacing.store(false, Ordering::SeqCst);
                return Err(MlirError::HotCodeError {
                    message: Text::from("Timeout waiting for active calls"),
                });
            }
            std::thread::yield_now();
        }

        // Mark old version as inactive
        if let Some(old) = self.versions.last_mut() {
            old.active = false;
        }

        // Create new version
        let new_version = self.current_version + 1;
        let version = FunctionVersion {
            version: new_version,
            address: new_address,
            signature_hash,
            created_at: instant::Instant::now(),
            source_hash,
            active: true,
        };

        // Swap the indirection pointer atomically for thread-safe hot reload
        self.indirection.store(new_address, Ordering::Release);

        // Store version
        self.versions.push(version);
        self.current_version = new_version;
        self.replacement_count.fetch_add(1, Ordering::Relaxed);

        // Done replacing
        self.replacing.store(false, Ordering::SeqCst);

        Ok(())
    }

    /// Rollback to previous version.
    pub fn rollback(&mut self) -> Result<()> {
        if self.versions.len() < 2 {
            return Err(MlirError::HotCodeError {
                message: Text::from("No previous version to rollback to"),
            });
        }

        // Mark current as inactive
        if let Some(current) = self.versions.last_mut() {
            current.active = false;
        }

        // Remove current version
        self.versions.pop();

        // Activate previous version
        if let Some(prev) = self.versions.last_mut() {
            prev.active = true;
            self.current_version = prev.version;

            self.indirection.store(prev.address, Ordering::Release);
        }

        Ok(())
    }

    /// Rollback to specific version.
    pub fn rollback_to(&mut self, version: u64) -> Result<()> {
        let index = self.versions.iter().position(|v| v.version == version)
            .ok_or_else(|| MlirError::HotCodeError {
                message: Text::from(format!("Version {} not found", version)),
            })?;

        // Mark all versions after target as inactive
        for v in &mut self.versions[index + 1..] {
            v.active = false;
        }

        // Truncate to target version
        self.versions.truncate(index + 1);

        // Activate target version
        if let Some(target) = self.versions.last_mut() {
            target.active = true;
            self.current_version = target.version;

            self.indirection.store(target.address, Ordering::Release);
        }

        Ok(())
    }

    /// Enter a call (for active call tracking).
    pub fn enter_call(&self) {
        self.active_calls.fetch_add(1, Ordering::AcqRel);
    }

    /// Exit a call.
    pub fn exit_call(&self) {
        self.active_calls.fetch_sub(1, Ordering::AcqRel);
    }

    /// Get version history.
    pub fn history(&self) -> Vec<&FunctionVersion> {
        self.versions.iter().collect()
    }

    /// Prune old versions (keep N most recent).
    pub fn prune(&mut self, keep: usize) {
        if self.versions.len() > keep {
            let remove_count = self.versions.len() - keep;
            self.versions.drain(0..remove_count);
        }
    }
}

impl Drop for HotFunction {
    fn drop(&mut self) {
        // AtomicPtr is inline, no heap allocation to free.
        // JIT code memory is managed by the JIT engine, not by HotFunction.
    }
}

// ============================================================================
// Hot Reload Statistics
// ============================================================================

/// Statistics for hot reload operations.
#[derive(Debug, Default)]
pub struct HotReloadStats {
    /// Number of replacements.
    pub replacements: AtomicU64,

    /// Number of rollbacks.
    pub rollbacks: AtomicU64,

    /// Number of validation failures.
    pub validation_failures: AtomicU64,

    /// Total replacement time (microseconds).
    pub total_replacement_time_us: AtomicU64,

    /// Number of functions registered.
    pub functions_registered: AtomicU64,
}

impl HotReloadStats {
    /// Create new statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get average replacement time.
    pub fn avg_replacement_time_us(&self) -> f64 {
        let count = self.replacements.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            self.total_replacement_time_us.load(Ordering::Relaxed) as f64 / count as f64
        }
    }

    /// Get summary.
    pub fn summary(&self) -> HotReloadStatsSummary {
        HotReloadStatsSummary {
            replacements: self.replacements.load(Ordering::Relaxed),
            rollbacks: self.rollbacks.load(Ordering::Relaxed),
            validation_failures: self.validation_failures.load(Ordering::Relaxed),
            avg_replacement_time_us: self.avg_replacement_time_us(),
            functions: self.functions_registered.load(Ordering::Relaxed),
        }
    }
}

/// Summary of hot reload statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotReloadStatsSummary {
    pub replacements: u64,
    pub rollbacks: u64,
    pub validation_failures: u64,
    pub avg_replacement_time_us: f64,
    pub functions: u64,
}

// ============================================================================
// State Migration
// ============================================================================

/// State migration callback.
pub type MigrationCallback = Box<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;

/// State migration configuration.
pub struct MigrationConfig {
    /// Old state version.
    pub from_version: u64,

    /// New state version.
    pub to_version: u64,

    /// Migration callback.
    pub migrate: MigrationCallback,
}

// ============================================================================
// Hot Reloader
// ============================================================================

/// Hot code replacement manager.
pub struct HotReloader {
    /// Configuration.
    config: HotReloadConfig,

    /// Registered hot functions.
    functions: DashMap<Text, Arc<RwLock<HotFunction>>>,

    /// Statistics.
    stats: Arc<HotReloadStats>,

    /// Migration callbacks.
    migrations: DashMap<Text, Vec<MigrationConfig>>,

    /// Lock for replacement operations.
    replacement_lock: RwLock<()>,
}

impl HotReloader {
    /// Create a new hot reloader.
    pub fn new(config: HotReloadConfig) -> Self {
        Self {
            config,
            functions: DashMap::new(),
            stats: Arc::new(HotReloadStats::new()),
            migrations: DashMap::new(),
            replacement_lock: RwLock::new(()),
        }
    }

    /// Register a function for hot reloading.
    ///
    /// # Safety
    ///
    /// The address must point to valid JIT-compiled code.
    pub unsafe fn register(
        &self,
        name: impl Into<Text>,
        address: *mut (),
        signature_hash: u64,
    ) -> Result<()> {
        let name = name.into();
        // SAFETY: Caller guarantees address points to valid JIT-compiled code
        let hot_fn = unsafe { HotFunction::new(name.clone(), address, signature_hash) };

        self.functions.insert(name, Arc::new(RwLock::new(hot_fn)));
        self.stats.functions_registered.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Get indirection pointer for a function.
    ///
    /// Callers should use this pointer for all calls to support hot reloading.
    pub fn get_indirection(&self, name: &str) -> Option<*const AtomicPtr<()>> {
        self.functions
            .get(&Text::from(name))
            .map(|f| f.read().indirection_ptr())
    }

    /// Check if a function is registered.
    pub fn is_registered(&self, name: &str) -> bool {
        self.functions.contains_key(&Text::from(name))
    }

    /// Replace a function with new implementation.
    ///
    /// # Safety
    ///
    /// The new address must point to valid JIT-compiled code that is
    /// compatible with the existing signature.
    pub unsafe fn replace(
        &self,
        name: &str,
        new_address: *mut (),
        signature_hash: u64,
        source_hash: [u8; 32],
    ) -> Result<()> {
        // Acquire the global replacement lock when atomic-replacement
        // mode is configured (the default). Atomic mode serialises
        // every replacement so concurrent callers always observe a
        // consistent function pointer; non-atomic mode trades that
        // guarantee for less head-of-line blocking when many
        // independent functions reload in parallel. The per-function
        // RwLock acquired below still serialises mutators of the
        // same hot-fn, so non-atomic mode is safe for *distinct*
        // function names — it only relaxes the cross-function
        // ordering. Without this gate the field was inert.
        let _atomic_guard = if self.config.atomic_replacement {
            Some(self.replacement_lock.write())
        } else {
            None
        };
        let start = instant::Instant::now();

        let name_text = Text::from(name);
        let hot_fn = self.functions.get(&name_text).ok_or_else(|| MlirError::HotCodeError {
            message: Text::from(format!("Function '{}' not registered", name)),
        })?;

        // Validate signature if enabled
        if self.config.validate_signatures {
            let current = hot_fn.read();
            if let Some(current_ver) = current.versions.last() {
                if current_ver.signature_hash != signature_hash {
                    self.stats.validation_failures.fetch_add(1, Ordering::Relaxed);
                    return Err(MlirError::HotCodeError {
                        message: Text::from("Signature mismatch - function parameters or return type changed"),
                    });
                }
            }
        }

        // Perform replacement
        let mut hot_fn = hot_fn.write();
        // SAFETY: Caller guarantees new_address points to valid JIT-compiled code
        unsafe { hot_fn.replace(new_address, signature_hash, source_hash)?; }

        // Prune old versions
        hot_fn.prune(self.config.version_history);

        // Record statistics
        let elapsed = start.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;
        self.stats.replacements.fetch_add(1, Ordering::Relaxed);
        self.stats.total_replacement_time_us.fetch_add(elapsed_us, Ordering::Relaxed);

        // Honour `HotReloadConfig.max_replacement_time_us` budget:
        // a replacement that exceeds the configured ceiling is
        // surfaced as a warning (the replacement itself has already
        // succeeded — we don't roll back, since the new code is
        // already live and rolling back would mean swapping in
        // a third version mid-call). Callers that want strict
        // budget enforcement can react to the warning by calling
        // `rollback`. Without this gate the field was inert.
        if elapsed_us > self.config.max_replacement_time_us {
            tracing::warn!(
                "Hot-reload of '{}' took {}µs, exceeding configured budget {}µs",
                name,
                elapsed_us,
                self.config.max_replacement_time_us
            );
        }

        if self.config.verbose {
            tracing::info!(
                "Replaced function '{}' (v{}) in {:?}",
                name,
                hot_fn.current_version,
                elapsed
            );
        }

        Ok(())
    }

    /// Rollback a function to previous version.
    pub fn rollback(&self, name: &str) -> Result<()> {
        let name_text = Text::from(name);
        let hot_fn = self.functions.get(&name_text).ok_or_else(|| MlirError::HotCodeError {
            message: Text::from(format!("Function '{}' not registered", name)),
        })?;

        let mut hot_fn = hot_fn.write();
        hot_fn.rollback()?;

        self.stats.rollbacks.fetch_add(1, Ordering::Relaxed);

        if self.config.verbose {
            tracing::info!("Rolled back function '{}' to v{}", name, hot_fn.current_version);
        }

        Ok(())
    }

    /// Rollback a function to specific version.
    pub fn rollback_to(&self, name: &str, version: u64) -> Result<()> {
        let name_text = Text::from(name);
        let hot_fn = self.functions.get(&name_text).ok_or_else(|| MlirError::HotCodeError {
            message: Text::from(format!("Function '{}' not registered", name)),
        })?;

        let mut hot_fn = hot_fn.write();
        hot_fn.rollback_to(version)?;

        self.stats.rollbacks.fetch_add(1, Ordering::Relaxed);

        if self.config.verbose {
            tracing::info!("Rolled back function '{}' to v{}", name, version);
        }

        Ok(())
    }

    /// Get version history for a function.
    pub fn get_history(&self, name: &str) -> Option<Vec<u64>> {
        self.functions
            .get(&Text::from(name))
            .map(|f| f.read().versions.iter().map(|v| v.version).collect())
    }

    /// Get current version for a function.
    pub fn get_version(&self, name: &str) -> Option<u64> {
        self.functions
            .get(&Text::from(name))
            .map(|f| f.read().current_version)
    }

    /// Get all registered function names.
    pub fn registered_functions(&self) -> Vec<Text> {
        self.functions.iter().map(|e| e.key().clone()).collect()
    }

    /// Get statistics.
    pub fn stats(&self) -> &HotReloadStats {
        &self.stats
    }

    /// Get configuration.
    pub fn config(&self) -> &HotReloadConfig {
        &self.config
    }

    /// Register a migration callback.
    ///
    /// Honours `HotReloadConfig.enable_migration`: when `false`,
    /// the registration is rejected with a structured error so
    /// callers can detect the policy and fall back to a different
    /// upgrade strategy. Without this gate the field was inert —
    /// migration callbacks would always register and fire on
    /// upgrade regardless of configuration.
    pub fn register_migration(
        &self,
        name: impl Into<Text>,
        from_version: u64,
        to_version: u64,
        migrate: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static,
    ) -> Result<()> {
        if !self.config.enable_migration {
            return Err(MlirError::HotCodeError {
                message: Text::from(
                    "migration is disabled by HotReloadConfig.enable_migration = false",
                ),
            });
        }

        let name = name.into();
        let config = MigrationConfig {
            from_version,
            to_version,
            migrate: Box::new(migrate),
        };

        self.migrations
            .entry(name)
            .or_insert_with(Vec::new)
            .push(config);

        Ok(())
    }

    /// Call wrapper that tracks active calls.
    ///
    /// Use this to wrap calls to hot-reloadable functions.
    pub fn with_tracking<F, R>(&self, name: &str, f: F) -> Option<R>
    where
        F: FnOnce(*mut ()) -> R,
    {
        let name_text = Text::from(name);
        let hot_fn = self.functions.get(&name_text)?;

        let fn_ref = hot_fn.read();
        fn_ref.enter_call();

        let addr = fn_ref.current_address();
        drop(fn_ref);

        let result = f(addr);

        hot_fn.read().exit_call();

        Some(result)
    }

    /// Unregister a function.
    pub fn unregister(&self, name: &str) {
        self.functions.remove(&Text::from(name));
    }

    /// Clear all registered functions.
    pub fn clear(&self) {
        self.functions.clear();
        self.migrations.clear();
    }

    /// Apply registered state-migration callbacks to transform
    /// `state` from `from_version` to `to_version`. Closes the
    /// inert-defense pattern around `MigrationConfig.migrate`:
    /// pre-fix the callback was registered via
    /// `register_migration` but no production code path ever
    /// invoked it, so the field was a write-only side channel.
    ///
    /// The function chains callbacks: if the user registered
    /// `1 → 2` and `2 → 3` migrations, calling `migrate_state(name, 1, 3, state)`
    /// applies them in order. When no chain reaches the target
    /// version, returns `None` so callers can fall back to a
    /// fresh-start strategy. Honours `enable_migration` — when
    /// `false` the lookup returns `None` even for registered
    /// callbacks (matching the registration-time gate).
    ///
    /// # Arguments
    ///
    /// * `name` — the hot function the migrations were
    ///   registered for
    /// * `from_version` — the version stamp on the input `state`
    /// * `to_version` — the version stamp the caller wants to
    ///   reach (must be ≥ `from_version`; descending chains are
    ///   not supported)
    /// * `state` — the serialised state to transform
    ///
    /// Returns `Some(transformed)` when a contiguous chain of
    /// callbacks from `from_version` to `to_version` exists,
    /// `None` otherwise.
    pub fn migrate_state(
        &self,
        name: &str,
        from_version: u64,
        to_version: u64,
        mut state: Vec<u8>,
    ) -> Option<Vec<u8>> {
        if !self.config.enable_migration {
            return None;
        }
        if from_version == to_version {
            // Identity migration — no callbacks needed.
            return Some(state);
        }
        if from_version > to_version {
            // Downgrade not supported by the chained-forward
            // model; callers that need rollback should keep the
            // pre-replacement bytes themselves.
            return None;
        }

        let migrations = self.migrations.get(&Text::from(name))?;
        let mut current = from_version;
        // Chain forward: at each step, find the migration whose
        // `from_version` matches the current version. Linear scan
        // — the migration list is bounded by `version_history` so
        // O(N²) is acceptable for the typical N ≤ 5.
        while current < to_version {
            let next = migrations
                .iter()
                .find(|m| m.from_version == current)?;
            state = (next.migrate)(&state);
            current = next.to_version;
        }
        if current != to_version {
            // Chain over-shot or under-shot — no exact path.
            return None;
        }
        Some(state)
    }
}

// ============================================================================
// Function Signature Hasher
// ============================================================================

/// Helper to compute function signature hashes.
pub struct SignatureHasher {
    hasher: std::collections::hash_map::DefaultHasher,
}

impl SignatureHasher {
    /// Create new signature hasher.
    pub fn new() -> Self {
        use std::hash::Hasher;
        Self {
            hasher: std::collections::hash_map::DefaultHasher::new(),
        }
    }

    /// Add parameter type to signature.
    pub fn add_param_type(&mut self, type_name: &str) {
        use std::hash::Hash;
        type_name.hash(&mut self.hasher);
    }

    /// Set return type.
    pub fn set_return_type(&mut self, type_name: &str) {
        use std::hash::Hash;
        "->".hash(&mut self.hasher);
        type_name.hash(&mut self.hasher);
    }

    /// Finalize and get signature hash.
    pub fn finish(self) -> u64 {
        use std::hash::Hasher;
        self.hasher.finish()
    }
}

impl Default for SignatureHasher {
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
    fn test_hot_reload_config() {
        let config = HotReloadConfig::default();
        assert!(config.validate_signatures);
        assert!(config.atomic_replacement);
        assert!(config.version_history > 0);
    }

    /// Pin: `enable_migration` defaults to `true` and gates
    /// `register_migration`. Closes the inert-defense pattern:
    /// before this wire-up the field had no effect — migration
    /// callbacks would always register regardless of policy.
    #[test]
    fn enable_migration_default_is_true() {
        let config = HotReloadConfig::default();
        assert!(config.enable_migration);
    }

    #[test]
    fn register_migration_succeeds_when_enabled() {
        let reloader = HotReloader::new(HotReloadConfig::default());
        let result =
            reloader.register_migration("fn1", 1, 2, |bytes| bytes.to_vec());
        assert!(
            result.is_ok(),
            "registration should succeed under default enable_migration=true"
        );
    }

    #[test]
    fn register_migration_rejects_when_disabled() {
        let mut config = HotReloadConfig::default();
        config.enable_migration = false;
        let reloader = HotReloader::new(config);

        let result =
            reloader.register_migration("fn1", 1, 2, |bytes| bytes.to_vec());
        match result {
            Err(MlirError::HotCodeError { message }) => {
                assert!(
                    message.as_str().contains("enable_migration"),
                    "diagnostic should name the flag, got: {}",
                    message
                );
            }
            other => panic!(
                "expected HotCodeError under enable_migration=false, got: {:?}",
                other
            ),
        }
    }

    /// Pin: `max_replacement_time_us` defaults to 1 second
    /// (1_000_000µs). Drift here would silently change the
    /// budget for every caller relying on `Default::default()`.
    #[test]
    fn max_replacement_time_us_default_is_one_second() {
        let config = HotReloadConfig::default();
        assert_eq!(config.max_replacement_time_us, 1_000_000);
    }

    /// Pin: `atomic_replacement` defaults to `true`. The flag
    /// gates the global cross-function replacement lock; with
    /// `false`, distinct functions can reload concurrently.
    #[test]
    fn atomic_replacement_default_is_true() {
        let config = HotReloadConfig::default();
        assert!(config.atomic_replacement);

        let mut relaxed = config.clone();
        relaxed.atomic_replacement = false;
        // Constructor must accept either configuration without
        // panic — the flag is interpreted lazily inside `replace`.
        let _reloader = HotReloader::new(relaxed);
    }

    #[test]
    fn test_hot_reloader_create() {
        let reloader = HotReloader::new(HotReloadConfig::default());
        assert!(reloader.registered_functions().is_empty());
    }

    #[test]
    fn test_function_registration() {
        let reloader = HotReloader::new(HotReloadConfig::default());
        let addr = 0x1000 as *mut ();

        // SAFETY: Using dummy address for test
        unsafe {
            reloader.register("test_fn", addr, 12345).unwrap();
        }

        assert!(reloader.is_registered("test_fn"));
        assert!(!reloader.is_registered("nonexistent"));

        let indirection = reloader.get_indirection("test_fn");
        assert!(indirection.is_some());
    }

    #[test]
    fn test_signature_hasher() {
        let mut hasher1 = SignatureHasher::new();
        hasher1.add_param_type("i64");
        hasher1.add_param_type("i64");
        hasher1.set_return_type("i64");
        let hash1 = hasher1.finish();

        let mut hasher2 = SignatureHasher::new();
        hasher2.add_param_type("i64");
        hasher2.add_param_type("i64");
        hasher2.set_return_type("i64");
        let hash2 = hasher2.finish();

        let mut hasher3 = SignatureHasher::new();
        hasher3.add_param_type("i32"); // Different type
        hasher3.add_param_type("i64");
        hasher3.set_return_type("i64");
        let hash3 = hasher3.finish();

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_hot_reload_stats() {
        let stats = HotReloadStats::new();

        stats.replacements.fetch_add(5, Ordering::Relaxed);
        stats.total_replacement_time_us.fetch_add(1000, Ordering::Relaxed);

        assert_eq!(stats.avg_replacement_time_us(), 200.0);
    }

    #[test]
    fn test_function_version() {
        let version = FunctionVersion::new(1, 0x1000 as *mut (), 12345, [0u8; 32]);

        assert_eq!(version.version, 1);
        assert_eq!(version.signature_hash, 12345);
        assert!(!version.active);
    }

    // =========================================================================
    // MigrationConfig.migrate wiring tests
    // =========================================================================
    //
    // Pin: `MigrationConfig.migrate` reaches a public consumer via
    // `migrate_state`. Pre-wire the callback was registered via
    // `register_migration` but no production code path ever invoked
    // it — the field was a write-only side channel.

    #[test]
    fn migrate_state_identity_returns_unchanged() {
        // Pin: from == to is a valid no-op. The chain-forward
        // model returns the input unchanged without consulting
        // any registered migration.
        let reloader = HotReloader::new(HotReloadConfig::development());
        let state = vec![1, 2, 3, 4];
        let result = reloader.migrate_state("any_fn", 5, 5, state.clone());
        assert_eq!(result, Some(state));
    }

    #[test]
    fn migrate_state_chains_forward_through_registered_callbacks() {
        // Pin: registered callbacks chain in version order.
        // 1 → 2 doubles the bytes; 2 → 3 appends [99].
        // migrate_state(name, 1, 3, [10]) → callback chain → [10, 10, 99].
        let reloader = HotReloader::new(HotReloadConfig::development());
        reloader
            .register_migration("f", 1, 2, |state: &[u8]| {
                let mut out = state.to_vec();
                out.extend_from_slice(state);
                out
            })
            .expect("register 1→2");
        reloader
            .register_migration("f", 2, 3, |state: &[u8]| {
                let mut out = state.to_vec();
                out.push(99);
                out
            })
            .expect("register 2→3");
        let state = vec![10];
        let result = reloader.migrate_state("f", 1, 3, state);
        assert_eq!(result, Some(vec![10, 10, 99]));
    }

    #[test]
    fn migrate_state_returns_none_when_chain_breaks() {
        // Pin: gap in the chain → None. 1 → 2 is registered but
        // 2 → 3 is missing, so reaching 3 from 1 fails.
        let reloader = HotReloader::new(HotReloadConfig::development());
        reloader
            .register_migration("f", 1, 2, |s: &[u8]| s.to_vec())
            .expect("register 1→2");
        let result = reloader.migrate_state("f", 1, 3, vec![1, 2, 3]);
        assert_eq!(result, None);
    }

    #[test]
    fn migrate_state_returns_none_for_downgrade() {
        // Pin: from > to is unsupported (no descending chains).
        // Callers that want rollback should keep the pre-
        // replacement bytes themselves.
        let reloader = HotReloader::new(HotReloadConfig::development());
        reloader
            .register_migration("f", 1, 2, |s: &[u8]| s.to_vec())
            .expect("register 1→2");
        let result = reloader.migrate_state("f", 2, 1, vec![1, 2, 3]);
        assert_eq!(result, None);
    }

    #[test]
    fn migrate_state_returns_none_when_migration_disabled() {
        // Pin: `enable_migration = false` short-circuits to None
        // even for would-be-valid chains. Mirrors the
        // registration-time gate at `register_migration`.
        let cfg = HotReloadConfig::new()
            .enable_migration(false);
        let reloader = HotReloader::new(cfg);
        // register_migration would itself error here, but pin
        // the chain-application gate independently by calling
        // migrate_state with an empty migration table — the
        // disabled gate fires before the lookup.
        let result = reloader.migrate_state("f", 1, 2, vec![]);
        assert_eq!(result, None);
    }

    #[test]
    fn migrate_state_returns_none_when_no_migrations_registered() {
        // Pin: no migrations registered → None for any non-
        // identity chain. Identity (from == to) still returns
        // Some(state) per the dedicated test above.
        let reloader = HotReloader::new(HotReloadConfig::development());
        let result = reloader.migrate_state("f", 1, 2, vec![1, 2, 3]);
        assert_eq!(result, None);
    }
}
