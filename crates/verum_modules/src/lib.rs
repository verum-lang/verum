#![allow(unexpected_cfgs)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::filter_map_identity)]
#![allow(clippy::manual_filter_map)]
#![allow(clippy::unnecessary_filter_map)]
//! Module system for the Verum programming language.
//!
//! This crate provides comprehensive module resolution, loading, and management
//! for the Verum compiler. It implements the Verum module system with three
//! core responsibilities: namespace management (hierarchical modules), visibility
//! control (private-by-default with public/public(crate)/public(super)/public(in path)),
//! and dependency resolution (deterministic, unambiguous name resolution).
//!
//! # Overview
//!
//! The module system provides:
//!
//! - **Name Resolution**: Resolves identifiers to their definitions across modules
//! - **Module Loading**: Loads modules from the filesystem (.vr files)
//! - **Dependency Management**: Tracks module dependencies and compilation order
//! - **Import/Export**: Manages visibility and re-exports
//! - **Caching**: Caches parsed modules for performance
//! - **Language Profiles**: Profile-aware module control (Application/Systems/Research)
//! - **Protocol Coherence**: Orphan rule validation and overlap detection
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐
//! │ ModuleLoader │ ← Loads .vr files from filesystem
//! └────────┬────────┘
//!  │
//!  v
//! ┌─────────────────┐
//! │ ProfileChecker │ ← Validates language profile compatibility
//! └────────┬────────┘
//!  │
//!  v
//! ┌─────────────────┐
//! │ DependencyGraph│ ← Builds module dependency graph
//! └────────┬────────┘
//!  │
//!  v
//! ┌─────────────────┐
//! │ NameResolver │ ← Resolves names with scope rules
//! └────────┬────────┘
//!  │
//!  v
//! ┌─────────────────┐
//! │VisibilityChecker│ ← Checks access permissions
//! └────────┬────────┘
//!  │
//!  v
//! ┌─────────────────┐
//! │CoherenceChecker │ ← Validates protocol implementations
//! └─────────────────┘
//! ```
//!
//! # Language Profiles
//!
//! Verum supports three language profiles:
//! - **Application**: Safe, productive, async-first (default)
//! - **Systems**: Unsafe allowed, manual memory management
//! - **Research**: Formal verification, dependent types, proofs
//!
//! Modules can declare their profile with `@profile(application)`, `@profile(systems)`,
//! or `@profile(research)`. Child modules inherit parent profile restrictions.
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use verum_modules::{ModuleLoader, NameResolver, ProfileChecker};
//! use std::path::Path;
//!
//! // Create module loader
//! let mut loader = ModuleLoader::new(Path::new("src"));
//!
//! // Load module
//! let module_info = loader.load_and_parse(&ModulePath::root(), ModuleId::new(0))?;
//!
//! // Check profile compatibility
//! let profile_checker = ProfileChecker::new(LanguageProfile::Application);
//! profile_checker.check_module(&module_info)?;
//!
//! // Resolve names
//! let resolver = NameResolver::new();
//! resolver.resolve(&module_info)?;
//! ```
//!
//! # Key Design Principles
//!
//! - Explicit is better than implicit (no magical globals)
//! - File system mirrors module hierarchy
//! - Visibility defaults to private (principle of least privilege)
//! - Name resolution is deterministic and unambiguous

#![deny(missing_debug_implementations)]
#![deny(rust_2018_idioms)]
#![allow(dead_code)]

// v6.0-BALANCED semantic types (MANDATORY)
use verum_common::{List, Map, Maybe, Set, Shared, Text};

pub mod cache;
pub mod cog_resolver;
pub mod coherence;
pub mod dependency;
mod observations;
pub use observations::{ModuleDependencies, ModuleObservations};
pub mod error;
pub mod exports;
pub mod imports;
pub use verum_ast::mount_paths;
pub mod loader;
// File-relative mount resolution (#5 / P1.5) — plugs
// `mount ./helper.vr;` into the existing module-path
// pipeline by synthesising a module name per loaded file.
pub mod file_mount;
pub mod parallel;
pub mod path;
pub mod profile;
pub mod refinement_info;
pub mod resolver;
pub mod suggestions;
pub mod visibility;
pub mod warnings;

// Re-export main types
pub use cache::{ModuleCache, ModuleCacheEntry};
pub use coherence::{CoherenceChecker, CoherenceError, ImplEntry};
pub use dependency::{
    BackendState, CompilationTier, DependencyGraph, DependencyNode, IncrementalGraph, ModuleState,
    compute_content_hash,
};
pub use error::{
    CycleBreakKind, CycleBreakSuggestion, ModuleError, ModuleResult,
    generate_cycle_break_suggestions,
};
pub use exports::{
    ContextSourceKind, ExportKind, ExportTable, ExportedContext, ExportedItem,
    extract_contexts_from_module, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};
pub use imports::{ImportResolver, ResolvedImport};
pub use loader::{LazyModuleResolver, ModuleLoader, ModuleSource, SharedModuleResolver};
// Re-export cfg types for conditional compilation configuration
pub use parallel::{
    ParallelLoadConfig, ParallelLoadResult, ParallelLoadStats, ParallelLoader, SyncParallelLoader,
};
pub use path::{ModuleId, ModulePath, resolve_import};
pub use profile::{LanguageProfile, ModuleFeature, ModuleProfile, ProfileChecker};
pub use resolver::{NameResolver, ResolvedName, Scope};
pub use suggestions::{
    Suggestion, find_similar, find_similar_items, find_similar_modules, format_module_suggestions,
    format_suggestions, levenshtein_distance, similarity_ratio,
};
pub use verum_ast::cfg::{CfgEvaluator, CfgPredicate, TargetConfig};
pub use visibility::{Visibility, VisibilityChecker};
pub use warnings::{
    ModuleWarning, PreludeShadowingChecker, WarningCollector, WarningKind, WarningSeverity,
};

use verum_ast::{FileId, Item, Module as AstModule};

/// A loaded module with metadata.
///
/// This represents a fully parsed module ready for name resolution.
/// Includes profile information for language profile enforcement.
///
/// Represents a fully loaded, parsed module with its metadata, exports, imports,
/// child relationships, and language profile. This is the primary unit of
/// organization in the Verum module system.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModuleInfo {
    /// Unique module identifier
    pub id: ModuleId,
    /// Module path (e.g., std.collections.List)
    pub path: ModulePath,
    /// The AST of this module
    pub ast: AstModule,
    /// Items exported by this module
    pub exports: ExportTable,
    /// Modules imported by this module
    pub imports: List<ResolvedImport>,
    /// Child modules (inline or file-based)
    pub children: List<ModuleId>,
    /// Parent module (None for root)
    pub parent: Maybe<ModuleId>,
    /// Source file information
    pub file_id: FileId,
    /// Full source code
    pub source: Text,
    /// Language profile for this module (Application/Systems/Research).
    /// Modules declare their profile with @profile(application|systems|research).
    /// Child modules inherit parent profile restrictions but can be more restrictive.
    /// Research is most permissive, Application is most restrictive (default).
    pub profile: ModuleProfile,
    /// MOD-MED-1 — header-validation warnings produced
    /// at parse time. Carries dangling forward-decls and
    /// inline-vs-filesystem overlaps. Drained by the pipeline at
    /// load time and emitted as diagnostics; never blocks module
    /// loading by itself.
    #[serde(skip)]
    pub header_warnings: Vec<crate::loader::ModuleHeaderWarning>,
}

impl ModuleInfo {
    pub fn new(
        id: ModuleId,
        path: ModulePath,
        ast: AstModule,
        file_id: FileId,
        source: Text,
    ) -> Self {
        Self {
            id,
            path,
            ast,
            exports: ExportTable::new(),
            imports: List::new(),
            children: List::new(),
            parent: Maybe::None,
            file_id,
            source,
            profile: ModuleProfile::default(),
            header_warnings: Vec::new(),
        }
    }

    /// Create a new module with a specific profile.
    pub fn with_profile(mut self, profile: ModuleProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Set the module's profile.
    pub fn set_profile(&mut self, profile: ModuleProfile) {
        self.profile = profile;
    }

    /// Get the module's language profile.
    pub fn language_profile(&self) -> LanguageProfile {
        self.profile.profile
    }

    /// Check if this module has a specific feature enabled.
    pub fn has_feature(&self, feature: ModuleFeature) -> bool {
        self.profile.has_feature(feature)
    }

    /// Check if unsafe code is allowed in this module.
    pub fn allows_unsafe(&self) -> bool {
        self.profile.has_feature(ModuleFeature::Unsafe)
    }

    /// Get all public items in this module
    pub fn public_items(&self) -> impl Iterator<Item = &Item> {
        self.ast.items.iter().filter(|item| {
            use verum_ast::decl::Visibility;
            // Check if item is public
            match &item.kind {
                verum_ast::ItemKind::Function(f) => f.visibility == Visibility::Public,
                verum_ast::ItemKind::Type(t) => t.visibility == Visibility::Public,
                verum_ast::ItemKind::Protocol(p) => p.visibility == Visibility::Public,
                verum_ast::ItemKind::Module(m) => m.visibility == Visibility::Public,
                verum_ast::ItemKind::Const(c) => c.visibility == Visibility::Public,
                verum_ast::ItemKind::Static(s) => s.visibility == Visibility::Public,
                _ => false,
            }
        })
    }

    /// Check if this is a root module (crate root)
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Get the module name
    pub fn name(&self) -> &str {
        self.path
            .segments()
            .last()
            .map(|s| s.as_str())
            .unwrap_or("")
    }
}

/// Declaration data owned by one immutable registry generation.
///
/// A snapshot shares this generation. Every write first obtains a unique copy,
/// so lazy imports cannot change another consumer's names or exports.
#[derive(Debug, Clone, Default)]
struct RegistryData {
    modules: Map<ModuleId, Shared<ModuleInfo>>,
    /// Exact declared owners; lookup aliases never rewrite declaration identity.
    path_to_id: Map<Text, ModuleId>,
    path_aliases: Map<Text, Text>,
    cog_name: Option<Text>,
    external_cogs: Set<Text>,
}

impl RegistryData {
    /// Missing and ambiguous are distinct: an alias must never conceal an
    /// ambiguous direct request by selecting an unrelated third declaration.
    fn resolve_path_id(&self, path: &str) -> Result<Option<ModuleId>, ()> {
        let qualified = path::qualified_module_key(path, self.cog_name.as_deref(), |first| {
            self.external_cogs.contains(&Text::from(first))
        });
        let exact = self.path_to_id.get(&Text::from(path)).copied();
        let relative = self.path_to_id.get(&Text::from(qualified)).copied();
        match (exact, relative) {
            (Some(left), Some(right)) if left != right => Err(()),
            (Some(id), _) | (_, Some(id)) => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    fn lookup_by_path(&self, path: &str) -> Maybe<Shared<ModuleInfo>> {
        self.resolve_path_id(path).ok().flatten()
            .and_then(|id| self.modules.get(&id).cloned())
    }

    fn lookup_by_path_aliased(&self, path: &str) -> Maybe<Shared<ModuleInfo>> {
        match self.resolve_path_id(path) {
            Ok(Some(id)) => self.modules.get(&id).cloned(),
            Ok(None) => self.path_aliases.get(&Text::from(path))
                .and_then(|canonical| self.lookup_by_path(canonical.as_str())),
            Err(()) => Maybe::None,
        }
    }
}

/// Central storage for loaded modules, with constant-time immutable snapshots.
///
/// Clones share declarations until a write; their monotone ID allocator remains
/// shared. Use [`Self::deep_clone`] for an independent allocator as well.
#[derive(Debug, Clone)]
pub struct ModuleRegistry {
    data: Shared<RegistryData>,
    next_id: Shared<std::sync::atomic::AtomicU32>,
    observations: Option<std::sync::Arc<observations::Recorder>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            data: Shared::new(RegistryData::default()),
            next_id: Shared::new(std::sync::atomic::AtomicU32::new(0)),
            observations: None,
        }
    }

    /// The only mutable access to a declaration generation. Invalidation belongs
    /// here so export resolution cannot bypass the same rule as lazy loading.
    fn data_mut(&mut self) -> &mut RegistryData {
        self.observe_mutation();
        if Shared::get_mut(&mut self.data).is_none() {
            self.data = Shared::new((*self.data).clone());
        }
        Shared::get_mut(&mut self.data).expect("detached registry generation is uniquely owned")
    }

    /// Register a path-alias: when callers do `get_by_path_aliased(alias)`
    /// the registry first probes the canonical path-to-id map, then
    /// rewrites via this alias map and probes again. The pipeline /
    /// loader installs the standard set of aliases (`std.foo` →
    /// `core.foo`, semantic shorthand like `core.memory` →
    /// `core.base.memory`, platform-specific `core.thread` resolution)
    /// once at startup so the type-resolver does not re-derive aliases
    /// per call.
    ///
    /// MOD-CRIT-1: this is the funnel point that closes the loader/
    /// resolver path-dedup incoherence — every alias decision lives in
    /// one map, owned by the registry, queried by every consumer
    /// (loader for resolution, type-resolver for import lookup, CLI
    /// audits for path normalisation).
    pub fn register_path_alias(&mut self, alias: impl Into<Text>, canonical: impl Into<Text>) {
        self.data_mut().path_aliases.insert(alias.into(), canonical.into());
    }

    /// Read-only access to the alias table. Useful for diagnostics
    /// (e.g. `verum audit --module-aliases`) and for downstream
    /// tooling that needs to render the alias chain.
    pub fn path_aliases(&self) -> &Map<Text, Text> {
        self.observe(|reads| reads.alias_table = true);
        &self.data.path_aliases
    }

    /// Set the primary cog for relative lookup. Declared owners stay exact,
    /// including metadata registered before this context was configured.
    pub fn set_cog_name(&mut self, cog_name: impl Into<String>) {
        self.data_mut().cog_name = Some(Text::from(cog_name.into()));
    }

    /// Current cog name, if set.
    pub fn cog_name(&self) -> Option<&str> {
        self.observe(|reads| reads.cog_name = true);
        self.data.cog_name.as_deref()
    }

    /// Mark a dependency namespace as absolute, rather than cog-relative.
    pub fn register_external_cog(&mut self, name: impl Into<Text>) {
        self.data_mut().external_cogs.insert(name.into());
    }

    /// Allocate a new module ID
    pub fn allocate_id(&self) -> ModuleId {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ModuleId::new(id)
    }

    /// Reserve an externally assigned identity before allocating new modules.
    pub fn reserve_id(&self, id: ModuleId) {
        self.next_id.fetch_max(
            id.as_u32().checked_add(1).expect("module identity space exhausted"),
            std::sync::atomic::Ordering::SeqCst,
        );
    }

    /// Hand out the underlying Arc for the ModuleId allocator so other
    /// subsystems (ModuleLoader, Session, infer) can share the same
    /// monotonic counter. Without this, each subsystem's own
    /// `allocate_id()` / `allocate_module_id()` runs on a *separate*
    /// counter, and two modules can end up with the same numeric ID
    /// — which is the root of a whole class of "phantom duplicate"
    /// bugs the registry has no chance of detecting.
    pub fn id_allocator(&self) -> Shared<std::sync::atomic::AtomicU32> {
        self.next_id.clone()
    }

    /// Register a module.
    ///
    /// Dedupes only the exact declared owner. Loaders resolve request aliases
    /// before constructing ModuleInfo, so local and external declarations
    /// cannot coalesce merely because stripping a prefix gives the same text.
    /// Existing populated entries retain their ID and exports; empty forward
    /// declarations can be completed without changing that identity.
    pub fn register(&mut self, module: ModuleInfo) -> ModuleId {
        self.reserve_id(module.id);
        let path_key = Text::from(module.path.to_string());
        if let Some(&existing_id) = self.data.path_to_id.get(&path_key) {
            let placeholder = self.data.modules.get(&existing_id).is_some_and(|existing| {
                existing.exports.all_exports().count() == 0 && existing.imports.is_empty()
            });
            if placeholder {
                let mut completed = module;
                completed.id = existing_id;
                self.data_mut().modules.insert(existing_id, Shared::new(completed));
            } else {
                // Preserve first-registration-wins and the observation of an
                // attempted write without copying an unchanged generation.
                self.observe_mutation();
            }
            return existing_id;
        }
        let id = module.id;
        let data = self.data_mut();
        data.modules.insert(id, Shared::new(module));
        data.path_to_id.insert(path_key, id);
        id
    }

    /// Get a module by ID
    pub fn get(&self, id: ModuleId) -> Maybe<Shared<ModuleInfo>> {
        self.observe(|reads| { reads.ids.insert(id); });
        self.data.modules.get(&id).cloned()
    }

    /// Resolve an exact or cog-relative request without changing ownership.
    /// A relative spelling with two distinct candidates is unavailable; callers
    /// must qualify it. Namespace and shorthand aliases are explicit policy.
    pub fn get_by_path(&self, path: &str) -> Maybe<Shared<ModuleInfo>> {
        self.observe(|reads| { reads.paths.insert(path.into()); });
        self.data.lookup_by_path(path)
    }

    /// Alias-aware variant of [`Self::get_by_path`]. Probes the direct
    /// canonical path first; on miss, consults the registered
    /// path_aliases map and probes the canonical translation.
    ///
    /// MOD-CRIT-1: the type-resolver's `get_module_with_path_aliases`
    /// (infer.rs) used to host a hardcoded alias table, creating two
    /// independent canonical-path maps (loader's `module_path_to_id`
    /// + resolver's hardcoded aliases). With this method, both
    /// subsystems route through one alias map — installed by the
    /// pipeline at startup, queried by every consumer.
    pub fn get_by_path_aliased(&self, path: &str) -> Maybe<Shared<ModuleInfo>> {
        self.observe(|reads| { reads.aliases.insert(path.into()); });
        self.data.lookup_by_path_aliased(path)
    }

    /// Get all modules
    pub fn all_modules(&self) -> impl Iterator<Item = (&ModuleId, &Shared<ModuleInfo>)> {
        self.observe(|reads| reads.all = true);
        self.data.modules.iter()
    }

    /// Update a module's exports by adding items from another export table.
    ///
    /// This is used for glob re-exports (`public import path.*`).
    /// The source exports are copied to the target module's export table.
    ///
    /// Returns the number of *new* exports actually added to the target —
    /// items already present in the target's exports do NOT count. The
    /// closure driver in `resolve_glob_reexports` consumes this count to
    /// detect a fixed point (zero new exports added = stable).
    pub fn add_exports_to_module(
        &mut self,
        target_id: ModuleId,
        source_exports: &exports::ExportTable,
    ) -> usize {
        let mut newly_added = 0usize;
        if let Some(existing) = self.data.modules.get(&target_id) {
            // Clone the existing ModuleInfo
            let mut updated = (**existing).clone();
            let pre_len = updated.exports.len();

            // Re-export the complete declaration payload. Reconstructing a
            // descriptor from its name/kind loses aliases and proof obligations.
            for (_, item) in source_exports.all_exports() {
                let _ = updated.exports.add_export(item.clone());
            }

            newly_added = updated.exports.len().saturating_sub(pre_len);

            // Replace the module in the registry
            let path_str = updated.path.to_string();
            let data = self.data_mut();
            data.modules.insert(target_id, Shared::new(updated));
            data.path_to_id.insert(Text::from(path_str.as_str()), target_id);
        } else {
            self.observe_mutation();
        }
        newly_added
    }

    /// Check if a module exists
    pub fn contains(&self, id: ModuleId) -> bool {
        self.get(id).is_some()
    }

    /// Clear all modules
    pub fn clear(&mut self) {
        let data = self.data_mut();
        data.modules.clear();
        data.path_to_id.clear();
    }

    /// Drop every module whose canonical path is NOT in `keep`.
    ///
    /// Returns `(kept, dropped)` counts. The `path_to_id` map is
    /// rewritten to mirror the surviving module set so subsequent
    /// `get_by_path` lookups are still consistent. Path aliases are
    /// preserved verbatim — they are textual rewrites that resolve
    /// to canonical paths, and a stale alias to a dropped module
    /// simply yields `None` on the next probe (which is the desired
    /// "not loaded" behaviour for the on-demand loader).
    ///
    /// This is the keystone of the on-demand stdlib loader: after the
    /// usual full / cached load populates the registry, the pipeline
    /// computes the user's reachable set and calls this method to
    /// narrow the visible module surface for downstream type-check
    /// passes (which see fewer exports → fewer name candidates →
    /// faster resolution).
    pub fn retain_paths(&mut self, keep: &std::collections::HashSet<String>) -> (usize, usize) {
        let data = self.data_mut();
        let pre_len = data.modules.len();
        let surviving_ids: Set<ModuleId> = data.path_to_id.iter()
            .filter_map(|(path, id)| keep.contains(path.as_str()).then_some(*id))
            .collect();
        data.modules.retain(|id, _| surviving_ids.contains(id));
        data.path_to_id.retain(|path, _| keep.contains(path.as_str()));
        let post_len = data.modules.len();
        (post_len, pre_len.saturating_sub(post_len))
    }

    /// Get the number of registered modules
    pub fn len(&self) -> usize {
        self.observe(|reads| reads.length = true);
        self.data.modules.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.observe(|reads| reads.empty = true);
        self.data.modules.is_empty()
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A reference-counted, interior-mutable handle to a [`ModuleRegistry`].
///
/// This is the canonical shape for sharing a registry across the compiler's
/// concurrently-compiled module set. Most components — `verum_types`,
/// `verum_compiler`, `verum_lsp` — store one of these and call [`Self::read`]
/// / [`Self::write`] to access the inner registry.
///
/// The newtype encapsulates the locking choice (currently
/// `Shared<parking_lot::RwLock<ModuleRegistry>>`); future refactors can switch
/// to a different concurrent map (e.g. `DashMap`) without touching call sites.
///
/// # Why this exists
///
/// Earlier code spelled out the wrapping inline at every construction site:
///
/// ```ignore
/// let reg = Shared::new(parking_lot::RwLock::new(ModuleRegistry::new()));
/// ```
///
/// That repetition is both verbose and a foot-gun: tests and dev tools
/// frequently constructed `Shared::new(ModuleRegistry::new())` (no `RwLock`)
/// and silently passed a `Shared<ModuleRegistry>` into APIs expecting a
/// `Shared<RwLock<ModuleRegistry>>` — producing 30+ `mismatched types` build
/// failures when the API tightened. This newtype makes such drift impossible
/// at the type level: the inner shape is private.
#[derive(Clone)]
pub struct SharedModuleRegistry {
    inner: Shared<parking_lot::RwLock<ModuleRegistry>>,
}

impl SharedModuleRegistry {
    /// Create a new shared registry containing an empty [`ModuleRegistry`].
    pub fn empty() -> Self {
        Self {
            inner: Shared::new(parking_lot::RwLock::new(ModuleRegistry::new())),
        }
    }

    /// Wrap an existing [`ModuleRegistry`] into a shared handle.
    pub fn from_registry(registry: ModuleRegistry) -> Self {
        Self {
            inner: Shared::new(parking_lot::RwLock::new(registry)),
        }
    }

    /// Construct from an already-wrapped registry. Accepts any
    /// `Shared<RwLock<ModuleRegistry>>`-compatible handle.
    ///
    /// Useful at module boundaries where another component has already done
    /// the wrapping. Avoids double-locking.
    pub fn from_raw(inner: Shared<parking_lot::RwLock<ModuleRegistry>>) -> Self {
        Self { inner }
    }

    /// Acquire a read guard. Multiple read guards may exist concurrently;
    /// they exclude write guards.
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, ModuleRegistry> {
        self.inner.read()
    }

    /// Acquire a write guard. Excludes all other guards.
    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, ModuleRegistry> {
        self.inner.write()
    }

    /// Borrow the inner `Shared<RwLock<ModuleRegistry>>` for legacy APIs that
    /// still take it directly. Prefer [`Self::read`] / [`Self::write`].
    pub fn as_inner(&self) -> &Shared<parking_lot::RwLock<ModuleRegistry>> {
        &self.inner
    }

    /// Return the inner handle, consuming this wrapper. Symmetric with
    /// [`Self::from_raw`].
    pub fn into_inner(self) -> Shared<parking_lot::RwLock<ModuleRegistry>> {
        self.inner
    }

    /// Strong reference count for the inner handle. For diagnostic and test
    /// use only.
    pub fn strong_count(&self) -> usize {
        Shared::strong_count(&self.inner)
    }
}

impl Default for SharedModuleRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for SharedModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedModuleRegistry")
            .field("strong_count", &self.strong_count())
            .finish_non_exhaustive()
    }
}

/// Convenience: any `ModuleRegistry` can be `.into()`-converted into a shared
/// handle. Equivalent to [`SharedModuleRegistry::from_registry`].
impl From<ModuleRegistry> for SharedModuleRegistry {
    fn from(registry: ModuleRegistry) -> Self {
        Self::from_registry(registry)
    }
}

impl ModuleRegistry {
    /// Snapshot the declarations with an independent ID allocator.
    ///
    /// The allocator starts after the maximum registered ID, preserving the
    /// independent-analysis contract. Declaration indexes remain shared until
    /// either registry is mutated; allocation alone never changes a declaration.
    pub fn deep_clone(&self) -> Self {
        let max_id = self.data.modules.keys().map(|id| id.as_u32()).max().unwrap_or(0);
        Self {
            data: self.data.clone(),
            next_id: Shared::new(std::sync::atomic::AtomicU32::new(max_id + 1)),
            observations: self.observations.clone(),
        }
    }

    /// Create a registry pre-populated with another registry's contents.
    ///
    /// This is an alias for `deep_clone` that makes the intent clearer.
    pub fn with_base(base: &Self) -> Self {
        base.deep_clone()
    }
}
