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
//! │   ModuleLoader  │ ← Loads .vr files from filesystem
//! └────────┬────────┘
//!          │
//!          v
//! ┌─────────────────┐
//! │  ProfileChecker │ ← Validates language profile compatibility
//! └────────┬────────┘
//!          │
//!          v
//! ┌─────────────────┐
//! │  DependencyGraph│ ← Builds module dependency graph
//! └────────┬────────┘
//!          │
//!          v
//! ┌─────────────────┐
//! │   NameResolver  │ ← Resolves names with scope rules
//! └────────┬────────┘
//!          │
//!          v
//! ┌─────────────────┐
//! │VisibilityChecker│ ← Checks access permissions
//! └────────┬────────┘
//!          │
//!          v
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
use verum_common::{List, Map, Maybe, Shared, Text};

pub mod cache;
pub mod cog_resolver;
pub mod coherence;
pub mod dependency;
pub mod error;
pub mod exports;
pub mod imports;
pub mod loader;
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
pub use verum_ast::cfg::{CfgEvaluator, CfgPredicate, TargetConfig};
pub use path::{ModuleId, ModulePath, resolve_import};
pub use profile::{LanguageProfile, ModuleFeature, ModuleProfile, ProfileChecker};
pub use resolver::{NameResolver, ResolvedName, Scope};
pub use suggestions::{
    Suggestion, find_similar, find_similar_items, find_similar_modules,
    format_module_suggestions, format_suggestions, levenshtein_distance, similarity_ratio,
};
pub use parallel::{
    ParallelLoadConfig, ParallelLoadResult, ParallelLoadStats, ParallelLoader, SyncParallelLoader,
};
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

/// The module registry - central storage for all loaded modules.
///
/// This is the single source of truth for module information during compilation.
#[derive(Debug, Clone)]
pub struct ModuleRegistry {
    /// All loaded modules by ID
    modules: Map<ModuleId, Shared<ModuleInfo>>,
    /// Canonical (cog-name-stripped) module path → ID mapping.
    path_to_id: Map<Text, ModuleId>,
    /// Next module ID to allocate
    next_id: Shared<std::sync::atomic::AtomicU32>,
    /// Name of the primary cog under compilation. When set, module
    /// paths beginning with this segment are canonicalised by stripping
    /// the prefix before being used as a lookup key. Mirrors
    /// `ModuleLoader::cog_name` so that loader and registry agree on
    /// the same canonical form.
    cog_name: Option<String>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: Map::new(),
            path_to_id: Map::new(),
            next_id: Shared::new(std::sync::atomic::AtomicU32::new(0)),
            cog_name: None,
        }
    }

    /// Set the primary cog name (for path canonicalisation on
    /// register / get_by_path).
    pub fn set_cog_name(&mut self, cog_name: impl Into<String>) {
        self.cog_name = Some(cog_name.into());
    }

    /// Current cog name, if set.
    pub fn cog_name(&self) -> Option<&str> {
        self.cog_name.as_deref()
    }

    /// Canonicalise a dotted module path by stripping the cog-name
    /// prefix when present.
    fn canonical_key(&self, path: &str) -> String {
        if let Some(cog) = &self.cog_name {
            if path == cog.as_str() {
                return String::new();
            }
            if let Some(rest) = path.strip_prefix(cog.as_str()) {
                if let Some(tail) = rest.strip_prefix('.') {
                    return tail.to_string();
                }
            }
        }
        path.to_string()
    }

    /// Allocate a new module ID
    pub fn allocate_id(&self) -> ModuleId {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ModuleId::new(id)
    }

    /// Register a module.
    ///
    /// Dedupes by canonical module path: if a module with the same
    /// path is already registered, the existing entry's ModuleId is
    /// returned and the incoming `module.id` is ignored. This is
    /// essential for correctness — without it, calling `register`
    /// twice for the same path (via any combination of the loader /
    /// pipeline / type-inference lazy paths) leaves two entries in
    /// `modules` with distinct ModuleIds, which then appear as
    /// "different source_modules" in `ExportTable::add_export` and
    /// raise spurious "Conflicting export" errors.
    ///
    /// The existing-entry's exports are merged with the incoming
    /// module's exports so that partial registrations (e.g. entry
    /// created by the loader, exports filled in by a later pass)
    /// coalesce into a single authoritative record.
    pub fn register(&mut self, module: ModuleInfo) -> ModuleId {
        let path_str = module.path.to_string();
        let canonical = self.canonical_key(&path_str);
        let path_key = Text::from(canonical);
        if let Some(&existing_id) = self.path_to_id.get(&path_key) {
            // Merge exports from the incoming registration into the
            // already-stored entry. Keep the original ModuleId so that
            // downstream source_module comparisons stay stable.
            if let Some(existing_shared) = self.modules.get(&existing_id) {
                // If the existing entry is empty (e.g. a placeholder
                // from the first lazy-load call), replace it with
                // the fuller incoming one but keep the ID.
                let existing_empty = existing_shared.exports.all_exports().count() == 0
                    && existing_shared.imports.is_empty();
                if existing_empty {
                    let mut replaced = module;
                    replaced.id = existing_id;
                    self.modules.insert(existing_id, Shared::new(replaced));
                }
                // Otherwise, drop the duplicate silently — the first
                // registration wins. Callers that want to refresh
                // must go through `clear` + re-register.
            }
            return existing_id;
        }
        let id = module.id;
        self.modules.insert(id, Shared::new(module));
        self.path_to_id.insert(path_key, id);
        id
    }

    /// Get a module by ID
    pub fn get(&self, id: ModuleId) -> Maybe<Shared<ModuleInfo>> {
        self.modules.get(&id).cloned()
    }

    /// Get a module by path. Accepts either the absolute form
    /// (`core.foo.bar`) or the cog-relative form (`foo.bar`) —
    /// both canonicalise to the same key.
    pub fn get_by_path(&self, path: &str) -> Maybe<Shared<ModuleInfo>> {
        let canonical = self.canonical_key(path);
        match self.path_to_id.get(&Text::from(canonical)) {
            Some(id) => self.modules.get(id).cloned(),
            None => Maybe::None,
        }
    }

    /// Get all modules
    pub fn all_modules(&self) -> impl Iterator<Item = (&ModuleId, &Shared<ModuleInfo>)> {
        self.modules.iter()
    }

    /// Update a module's exports by adding items from another export table.
    ///
    /// This is used for glob re-exports (`public import path.*`).
    /// The source exports are copied to the target module's export table.
    pub fn add_exports_to_module(
        &mut self,
        target_id: ModuleId,
        source_exports: &exports::ExportTable,
    ) {
        if let Some(existing) = self.modules.get(&target_id) {
            // Clone the existing ModuleInfo
            let mut updated = (**existing).clone();

            // Add each export from source to target
            for (name, item) in source_exports.all_exports() {
                // Re-export with the source's info but from the target module
                let reexport = exports::ExportedItem::new(
                    name.as_str(),
                    item.kind,
                    item.visibility.clone(),
                    item.source_module,
                    item.span,
                );
                let _ = updated.exports.add_export(reexport);
            }

            // Replace the module in the registry
            let path_str = updated.path.to_string();
            self.modules.insert(target_id, Shared::new(updated));
            self.path_to_id.insert(Text::from(path_str.as_str()), target_id);
        }
    }

    /// Check if a module exists
    pub fn contains(&self, id: ModuleId) -> bool {
        self.modules.contains_key(&id)
    }

    /// Clear all modules
    pub fn clear(&mut self) {
        self.modules.clear();
        self.path_to_id.clear();
    }

    /// Get the number of registered modules
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    /// Create a deep clone of the registry with a fresh ID counter.
    ///
    /// This is used for test performance optimization: the stdlib modules are
    /// registered once, then each test gets a deep clone of the registry with
    /// its own ID counter for user modules.
    ///
    /// The `Shared<ModuleInfo>` entries are shared (reference count incremented)
    /// but the ID counter is fresh, starting at max_existing_id + 1.
    pub fn deep_clone(&self) -> Self {
        // Find the max existing module ID to set the counter properly
        let max_id = self
            .modules
            .keys()
            .map(|id| id.as_u32())
            .max()
            .unwrap_or(0);

        Self {
            modules: self.modules.clone(),
            path_to_id: self.path_to_id.clone(),
            next_id: Shared::new(std::sync::atomic::AtomicU32::new(max_id + 1)),
            cog_name: self.cog_name.clone(),
        }
    }

    /// Create a registry pre-populated with another registry's contents.
    ///
    /// This is an alias for `deep_clone` that makes the intent clearer.
    pub fn with_base(base: &Self) -> Self {
        base.deep_clone()
    }
}
