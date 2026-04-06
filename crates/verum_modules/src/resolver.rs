//! Name resolution across modules.
//!
//! Resolves identifiers to their definitions using the path resolution algorithm:
//! 1. Check local scope (explicit bindings)
//! 2. Check explicit imports
//! 3. Check glob imports (lazily resolved)
//! 4. Check prelude (std.prelude.*)
//! 5. Error if ambiguous (no "last import wins")
//!
//! Resolution rules: explicit imports take precedence over glob imports,
//! local bindings shadow all imports, ambiguous names are compile errors,
//! absolute paths (crate.*) bypass local scope, and visibility is checked
//! after name resolution.

use crate::error::{ModuleError, ModuleResult};
use crate::path::{ModuleId, ModulePath};
use crate::refinement_info::RefinementInfo;
use crate::visibility::{Visibility, VisibilityChecker};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use verum_ast::Path;
use verum_common::{List, Map, Maybe, Text};

/// A resolved name - the result of name resolution.
/// Contains the module, path, kind, visibility, and optional refinement
/// information for the resolved item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedName {
    /// The module containing the definition
    pub module_id: ModuleId,
    /// The full path to the item
    pub path: ModulePath,
    /// The kind of item
    pub kind: NameKind,
    /// The local name (as imported/used)
    pub local_name: Text,
    /// Visibility of the item (private/public/internal/public(super)/public(in path))
    pub visibility: Visibility,
    /// Refinement type information for refined types (e.g., Int{> 0}).
    /// When a type with refinements is exported, the refinement becomes part
    /// of the public API contract and is preserved across module boundaries.
    pub refinement: Maybe<RefinementInfo>,
    /// Visibility of the refinement predicate (can differ from type visibility).
    /// Public predicates are reusable, internal predicates are cog-only,
    /// private predicates are implementation details.
    pub predicate_visibility: Visibility,

    /// Origin cog name, if this item was imported from an external cog.
    /// None means the item is from the current cog (local).
    /// Some("http") means it was imported via `mount http.client.Response`.
    pub cog_origin: Maybe<Text>,
}

impl ResolvedName {
    pub fn new(
        module_id: ModuleId,
        path: ModulePath,
        kind: NameKind,
        local_name: impl Into<Text>,
    ) -> Self {
        Self {
            module_id,
            path,
            kind,
            local_name: local_name.into(),
            visibility: Visibility::Public,
            refinement: Maybe::None,
            predicate_visibility: Visibility::Public,
            cog_origin: Maybe::None,
        }
    }

    pub fn with_visibility(
        module_id: ModuleId,
        path: ModulePath,
        kind: NameKind,
        local_name: impl Into<Text>,
        visibility: Visibility,
    ) -> Self {
        Self {
            module_id,
            path,
            kind,
            local_name: local_name.into(),
            visibility: visibility.clone(),
            refinement: Maybe::None,
            predicate_visibility: visibility,
            cog_origin: Maybe::None,
        }
    }

    /// Create a resolved name with refinement information.
    pub fn with_refinement(
        module_id: ModuleId,
        path: ModulePath,
        kind: NameKind,
        local_name: impl Into<Text>,
        visibility: Visibility,
        refinement: RefinementInfo,
        predicate_visibility: Visibility,
    ) -> Self {
        Self {
            module_id,
            path,
            kind,
            local_name: local_name.into(),
            visibility,
            refinement: Maybe::Some(refinement),
            predicate_visibility,
            cog_origin: Maybe::None,
        }
    }

    /// Set the cog origin for this resolved name (external cog import).
    pub fn with_cog_origin(mut self, cog_name: impl Into<Text>) -> Self {
        self.cog_origin = Maybe::Some(cog_name.into());
        self
    }

    /// Check if this resolved name has a refinement
    pub fn has_refinement(&self) -> bool {
        matches!(self.refinement, Maybe::Some(_))
    }

    /// Get the refinement info if present
    pub fn get_refinement(&self) -> Maybe<&RefinementInfo> {
        self.refinement.as_ref()
    }
}

/// The kind of named item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NameKind {
    Function,
    Type,
    Protocol,
    Module,
    Const,
    Static,
    Local,     // Local variable
    Parameter, // Function parameter
}

impl std::fmt::Display for NameKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameKind::Function => write!(f, "function"),
            NameKind::Type => write!(f, "type"),
            NameKind::Protocol => write!(f, "protocol"),
            NameKind::Module => write!(f, "module"),
            NameKind::Const => write!(f, "const"),
            NameKind::Static => write!(f, "static"),
            NameKind::Local => write!(f, "local variable"),
            NameKind::Parameter => write!(f, "parameter"),
        }
    }
}

/// A scope for name resolution.
///
/// Scopes are organized hierarchically with imports and local bindings.
#[derive(Debug, Clone)]
pub struct Scope {
    /// The module this scope belongs to
    module_id: ModuleId,
    /// Parent scope (for nested scopes)
    parent: Maybe<Box<Scope>>,
    /// Names defined in this scope
    bindings: Map<Text, ResolvedName>,
    /// Glob imports in this scope (resolved lazily)
    glob_imports: List<ModuleId>,
}

impl Scope {
    /// Create a new scope for a module.
    pub fn new(module_id: ModuleId) -> Self {
        Self {
            module_id,
            parent: Maybe::None,
            bindings: Map::new(),
            glob_imports: List::new(),
        }
    }

    /// Create a child scope.
    pub fn child(&self) -> Self {
        Self {
            module_id: self.module_id,
            parent: Maybe::Some(Box::new(self.clone())),
            bindings: Map::new(),
            glob_imports: self.glob_imports.clone(),
        }
    }

    /// Add a binding to this scope.
    pub fn add_binding(&mut self, name: impl Into<Text>, resolved: ResolvedName) {
        self.bindings.insert(name.into(), resolved);
    }

    /// Add a glob import to this scope.
    pub fn add_glob_import(&mut self, module_id: ModuleId) {
        if !self.glob_imports.contains(&module_id) {
            self.glob_imports.push(module_id);
        }
    }

    /// Look up a name in this scope.
    pub fn lookup(&self, name: &str) -> Maybe<ResolvedName> {
        let name_text = Text::from(name);

        // Check local bindings
        if let Some(resolved) = self.bindings.get(&name_text) {
            return Maybe::Some(resolved.clone());
        }

        // Check parent scope
        if let Maybe::Some(parent) = &self.parent {
            return parent.lookup(name);
        }

        Maybe::None
    }

    /// Check if a name is defined in this scope (not parent scopes).
    pub fn contains_local(&self, name: &str) -> bool {
        self.bindings.contains_key(&Text::from(name))
    }

    /// Get all bindings in this scope.
    pub fn all_bindings(&self) -> impl Iterator<Item = (&Text, &ResolvedName)> {
        self.bindings.iter()
    }

    /// Get the module ID this scope belongs to.
    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    /// Get the glob imports in this scope.
    pub fn glob_imports(&self) -> &List<ModuleId> {
        &self.glob_imports
    }
}

/// Name resolver - resolves identifiers to definitions.
///
/// Implements the resolution algorithm from Section 6 of the spec:
///
/// 1. Check local scope
/// 2. Check explicit imports
/// 3. Check glob imports
/// 4. Check prelude
/// 5. Error if ambiguous
///
#[derive(Debug)]
pub struct NameResolver {
    /// Scopes by module
    scopes: Map<ModuleId, Scope>,
    /// Prelude items (always available)
    prelude: Scope,
    /// Cache for multi-segment path resolution
    /// Key: (current_scope_module, path_segments)
    /// Value: resolved name
    path_cache: DashMap<(ModuleId, List<Text>), ResolvedName>,
    /// Module exports for path traversal
    module_exports: Map<ModuleId, Map<Text, ResolvedName>>,
    /// Visibility checker for access control (private/public/internal/public(super)/public(in path))
    visibility_checker: VisibilityChecker,
    /// Module paths for visibility checking
    /// Maps module_id -> module_path
    module_paths: Map<ModuleId, ModulePath>,
    /// Module parent relationships for super resolution
    /// Maps module_id -> parent_module_id
    module_parents: Map<ModuleId, ModuleId>,
}

impl NameResolver {
    pub fn new() -> Self {
        Self {
            scopes: Map::new(),
            prelude: Scope::new(ModuleId::ROOT),
            path_cache: DashMap::new(),
            module_exports: Map::new(),
            visibility_checker: VisibilityChecker::new(),
            module_paths: Map::new(),
            module_parents: Map::new(),
        }
    }

    /// Register a module's path for visibility checking.
    /// Required for internal/public(super)/public(in path) access control.
    pub fn register_module_path(&mut self, module_id: ModuleId, module_path: ModulePath) {
        self.module_paths.insert(module_id, module_path);
    }

    /// Register a module's parent for `super` path resolution.
    /// Enables relative path navigation via `super.sibling_module`.
    pub fn register_module_parent(&mut self, module_id: ModuleId, parent_id: ModuleId) {
        self.module_parents.insert(module_id, parent_id);
    }

    /// Create a scope for a module.
    pub fn create_scope(&mut self, module_id: ModuleId) -> &mut Scope {
        self.scopes
            .entry(module_id)
            .or_insert_with(|| Scope::new(module_id))
    }

    /// Get a scope for a module.
    pub fn get_scope(&self, module_id: ModuleId) -> Maybe<&Scope> {
        match self.scopes.get(&module_id) {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }

    /// Get a mutable scope for a module.
    pub fn get_scope_mut(&mut self, module_id: ModuleId) -> Maybe<&mut Scope> {
        match self.scopes.get_mut(&module_id) {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }

    /// Resolve a name in a module's scope.
    ///
    /// Follows the resolution priority:
    /// 1. Local scope (explicit bindings)
    /// 2. Glob imports (import path.*)
    /// 3. Prelude
    ///
    /// Resolution priority:
    /// 1. Local scope (explicit bindings)
    /// 2. Glob imports (import path.*)
    /// 3. Prelude (std.prelude.*)
    ///
    /// Note: Module resolution is typically not deeply nested, so we rely on
    /// sufficient stack size (RUST_MIN_STACK=16MB) rather than dynamic stack growth.
    pub fn resolve_name(&self, name: &str, in_module: ModuleId) -> ModuleResult<ResolvedName> {
        let name_text = Text::from(name);

        // Step 1: Check local scope (explicit bindings)
        if let Maybe::Some(scope) = self.get_scope(in_module) {
            if let Maybe::Some(resolved) = scope.lookup(name) {
                return Ok(resolved);
            }

            // Step 2: Check glob imports
            // We need to search through all glob-imported modules
            let mut found: Maybe<ResolvedName> = Maybe::None;
            let mut ambiguous_modules: List<ModuleId> = List::new();

            for glob_module_id in scope.glob_imports() {
                if let Some(exports) = self.module_exports.get(glob_module_id) {
                    if let Some(resolved) = exports.get(&name_text) {
                        if found.is_some() {
                            // Found in multiple modules - ambiguous
                            ambiguous_modules.push(*glob_module_id);
                        } else {
                            found = Maybe::Some(resolved.clone());
                            ambiguous_modules.push(*glob_module_id);
                        }
                    }
                }
            }

            // Check for ambiguity
            if ambiguous_modules.len() > 1 {
                return Err(ModuleError::AmbiguousImport {
                    name: name_text,
                    candidates: ambiguous_modules
                        .iter()
                        .filter_map(|id| self.module_paths.get(id).cloned())
                        .collect(),
                    span: None,
                });
            }

            if let Maybe::Some(resolved) = found {
                return Ok(resolved);
            }
        }

        // Step 3: Check prelude
        if let Maybe::Some(resolved) = self.prelude.lookup(name) {
            return Ok(resolved);
        }

        // Step 4: Not found
        Err(ModuleError::ItemNotFound {
            item_name: name_text,
            module_path: ModulePath::from_str("unknown"),
            available_items: List::new(),
            suggestions: List::new(),
            span: None,
        })
    }

    /// Resolve a path (qualified name).
    ///
    /// Implements multi-segment path resolution per Spec 14.6.
    ///
    /// # Examples
    /// - `List` → single segment resolution
    /// - `std.collections.List` → multi-segment resolution
    /// - `cog.module.Type` → absolute path
    /// - `super.sibling.Type` → relative path
    /// - `self.child.Type` → relative path from current module
    ///
    /// # Algorithm
    /// 1. Check cache for previously resolved paths
    /// 2. Resolve first segment through 6-step resolution
    /// 3. For each subsequent segment:
    ///    - Lookup in previous result's module
    ///    - Check visibility
    ///    - Continue chain
    /// 4. Cache result
    ///
    /// Multi-segment path resolution: resolves first segment through the
    /// standard resolution algorithm, then traverses subsequent segments
    /// through module exports, checking visibility at each step. Results
    /// are cached for performance.
    ///
    /// Note: Path resolution is typically not deeply nested, so we rely on
    /// sufficient stack size (RUST_MIN_STACK=16MB) rather than dynamic stack growth.
    pub fn resolve_path(&self, path: &Path, in_module: ModuleId) -> ModuleResult<ResolvedName> {
        if path.segments.is_empty() {
            return Err(ModuleError::InvalidPath {
                path: Text::from("empty path"),
                reason: Text::from("path cannot be empty"),
                span: Some(path.span),
            });
        }

        // If it's a single segment, resolve as a simple name
        if path.segments.len() == 1 {
            return self.resolve_single_segment(&path.segments[0], in_module, path.span);
        }

        // Convert segments to text list for caching
        let segment_texts: List<Text> = path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::PathSegment::Name(ident) => Some(ident.name.clone()),
                verum_ast::PathSegment::Cog => Some(Text::from("cog")),
                verum_ast::PathSegment::Super => Some(Text::from("super")),
                verum_ast::PathSegment::SelfValue => Some(Text::from("self")),
                verum_ast::PathSegment::Relative => Some(Text::from(".")),
            })
            .collect();

        // Check cache
        let cache_key = (in_module, segment_texts.clone());
        if let Some(cached) = self.path_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        // Multi-segment resolution
        let resolved = self.resolve_multi_segment(path, in_module)?;

        // Cache the result
        self.path_cache.insert(cache_key, resolved.clone());

        Ok(resolved)
    }

    /// Resolve a single path segment.
    fn resolve_single_segment(
        &self,
        segment: &verum_ast::PathSegment,
        in_module: ModuleId,
        span: verum_ast::Span,
    ) -> ModuleResult<ResolvedName> {
        match segment {
            verum_ast::PathSegment::Name(ident) => {
                self.resolve_name(ident.name.as_str(), in_module)
            }
            verum_ast::PathSegment::Cog => {
                // Resolve to cog root module
                Ok(ResolvedName::new(
                    ModuleId::ROOT,
                    ModulePath::from_str("cog"),
                    NameKind::Module,
                    "cog",
                ))
            }
            verum_ast::PathSegment::SelfValue => {
                // Resolve to current module
                Ok(ResolvedName::new(
                    in_module,
                    ModulePath::from_str("self"),
                    NameKind::Module,
                    "self",
                ))
            }
            verum_ast::PathSegment::Super => {
                // Resolve to parent module
                match self.module_parents.get(&in_module) {
                    Some(parent_id) => {
                        // Get parent module path
                        let parent_path = self
                            .module_paths
                            .get(parent_id)
                            .cloned()
                            .unwrap_or_else(|| ModulePath::from_str("super"));

                        Ok(ResolvedName::new(
                            *parent_id,
                            parent_path,
                            NameKind::Module,
                            "super",
                        ))
                    }
                    None => {
                        // Root module has no parent
                        Err(ModuleError::InvalidPath {
                            path: Text::from("super"),
                            reason: Text::from("cannot use `super` from root module"),
                            span: Some(span),
                        })
                    }
                }
            }
            verum_ast::PathSegment::Relative => {
                // Relative import - resolve to parent module (like super)
                match self.module_parents.get(&in_module) {
                    Some(parent_id) => {
                        // Get parent module path
                        let parent_path = self
                            .module_paths
                            .get(parent_id)
                            .cloned()
                            .unwrap_or_else(|| ModulePath::from_str("."));

                        Ok(ResolvedName::new(
                            *parent_id,
                            parent_path,
                            NameKind::Module,
                            ".",
                        ))
                    }
                    None => {
                        // Root module has no parent
                        Err(ModuleError::InvalidPath {
                            path: Text::from("."),
                            reason: Text::from("cannot use relative import from root module"),
                            span: Some(span),
                        })
                    }
                }
            }
        }
    }

    /// Resolve a multi-segment path.
    ///
    /// Resolves a multi-segment path by resolving the first segment, then
    /// traversing remaining segments through module exports.
    fn resolve_multi_segment(
        &self,
        path: &Path,
        in_module: ModuleId,
    ) -> ModuleResult<ResolvedName> {
        // Step 1: Resolve first segment
        let first_segment = &path.segments[0];
        let mut current = self.resolve_first_segment(first_segment, in_module, path.span)?;

        // Step 2: Traverse remaining segments
        for segment in &path.segments[1..] {
            let seg_name = match segment {
                verum_ast::PathSegment::Name(ident) => &ident.name,
                verum_ast::PathSegment::Super => {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from("super in path"),
                        reason: Text::from("super can only appear at the start of a path"),
                        span: Some(path.span),
                    });
                }
                verum_ast::PathSegment::SelfValue => {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from("self in path"),
                        reason: Text::from("self can only appear at the start of a path"),
                        span: Some(path.span),
                    });
                }
                verum_ast::PathSegment::Cog => {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from("crate in path"),
                        reason: Text::from("crate can only appear at the start of a path"),
                        span: Some(path.span),
                    });
                }
                verum_ast::PathSegment::Relative => {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from(". in path"),
                        reason: Text::from(
                            "relative marker can only appear at the start of a path",
                        ),
                        span: Some(path.span),
                    });
                }
            };

            // Resolve in the context of the current module
            current = self.resolve_in_module(current.module_id, seg_name, in_module, path.span)?;
        }

        Ok(current)
    }

    /// Resolve the first segment of a path.
    ///
    /// Uses the 6-step resolution algorithm:
    /// 1. Local scope
    /// 2. Explicit imports
    /// 3. Glob imports
    /// 4. Prelude
    /// 5. Parent modules (if enabled)
    /// 6. Error if not found
    ///
    /// Uses the resolution algorithm: local scope, explicit imports, glob
    /// imports, prelude. Special segments: `cog` -> root, `self` -> current,
    /// `super` -> parent.
    fn resolve_first_segment(
        &self,
        segment: &verum_ast::PathSegment,
        in_module: ModuleId,
        span: verum_ast::Span,
    ) -> ModuleResult<ResolvedName> {
        match segment {
            verum_ast::PathSegment::Name(ident) => {
                // Use standard name resolution
                self.resolve_name(ident.name.as_str(), in_module)
            }
            verum_ast::PathSegment::Cog => {
                // Start from cog root
                Ok(ResolvedName::new(
                    ModuleId::ROOT,
                    ModulePath::from_str("cog"),
                    NameKind::Module,
                    "cog",
                ))
            }
            verum_ast::PathSegment::Super => {
                // Resolve to parent module
                match self.module_parents.get(&in_module) {
                    Some(parent_id) => {
                        // Get parent module path
                        let parent_path = self
                            .module_paths
                            .get(parent_id)
                            .cloned()
                            .unwrap_or_else(|| ModulePath::from_str("super"));

                        Ok(ResolvedName::new(
                            *parent_id,
                            parent_path,
                            NameKind::Module,
                            "super",
                        ))
                    }
                    None => {
                        // Root module has no parent
                        Err(ModuleError::InvalidPath {
                            path: Text::from("super"),
                            reason: Text::from("cannot use `super` from root module"),
                            span: Some(span),
                        })
                    }
                }
            }
            verum_ast::PathSegment::SelfValue => {
                // Start from current module
                Ok(ResolvedName::new(
                    in_module,
                    ModulePath::from_str("self"),
                    NameKind::Module,
                    "self",
                ))
            }
            verum_ast::PathSegment::Relative => {
                // Relative import - resolve to parent module (like super)
                match self.module_parents.get(&in_module) {
                    Some(parent_id) => {
                        // Get parent module path
                        let parent_path = self
                            .module_paths
                            .get(parent_id)
                            .cloned()
                            .unwrap_or_else(|| ModulePath::from_str("."));

                        Ok(ResolvedName::new(
                            *parent_id,
                            parent_path,
                            NameKind::Module,
                            ".",
                        ))
                    }
                    None => {
                        // Root module has no parent
                        Err(ModuleError::InvalidPath {
                            path: Text::from("."),
                            reason: Text::from("cannot use relative import from root module"),
                            span: Some(span),
                        })
                    }
                }
            }
        }
    }

    /// Resolve a name within a specific module's exports.
    ///
    /// This looks up the name in the module's exported items and checks
    /// visibility from the accessing module.
    ///
    /// Looks up a name in a module's exported items and checks visibility
    /// from the accessing module using the five-level visibility system.
    fn resolve_in_module(
        &self,
        target_module: ModuleId,
        name: &Text,
        accessing_module: ModuleId,
        span: verum_ast::Span,
    ) -> ModuleResult<ResolvedName> {
        // Get exports for the target module
        let exports = match self.module_exports.get(&target_module) {
            Some(exports) => exports,
            None => {
                return Err(ModuleError::ItemNotFound {
                    item_name: name.clone(),
                    module_path: ModulePath::from_str(&format!("module_{}", target_module.get())),
                    available_items: List::new(),
                    suggestions: List::new(),
                    span: Some(span),
                });
            }
        };

        // Look up the name
        let resolved = match exports.get(name) {
            Some(item) => item.clone(),
            None => {
                let available: List<Text> = exports.keys().cloned().collect();
                let suggestions = crate::suggestions::find_similar_items(name.as_str(), &available);
                return Err(ModuleError::ItemNotFound {
                    item_name: name.clone(),
                    module_path: ModulePath::from_str(&format!("module_{}", target_module.get())),
                    available_items: available,
                    suggestions,
                    span: Some(span),
                });
            }
        };

        // Check visibility using the five-level visibility algorithm
        let unknown_path = ModulePath::from_str("unknown");
        let item_module_path = self
            .module_paths
            .get(&resolved.module_id)
            .unwrap_or(&unknown_path);
        let accessing_module_path = self
            .module_paths
            .get(&accessing_module)
            .unwrap_or(&unknown_path);

        self.visibility_checker.check_visibility(
            name.as_str(),
            resolved.visibility.clone(),
            item_module_path,
            accessing_module_path,
        )?;

        Ok(resolved)
    }

    /// Register a module's exports for path resolution.
    ///
    /// This must be called after a module is loaded to enable path resolution
    /// through that module.
    pub fn register_module_exports(
        &mut self,
        module_id: ModuleId,
        exports: Map<Text, ResolvedName>,
    ) {
        self.module_exports.insert(module_id, exports);
    }

    /// Clear the path resolution cache.
    ///
    /// This should be called when modules are reloaded or modified.
    pub fn clear_path_cache(&self) {
        self.path_cache.clear();
    }

    /// Add a prelude item (available in all modules).
    pub fn add_prelude_item(&mut self, name: impl Into<Text>, resolved: ResolvedName) {
        self.prelude.add_binding(name, resolved);
    }

    /// Clear all scopes.
    pub fn clear(&mut self) {
        self.scopes.clear();
    }

    /// Validate that a refined type's predicate is accessible from the
    /// accessing module. All three refinement syntaxes (inline, declarative,
    /// sigma-type) work equivalently across module boundaries. Predicate
    /// visibility can be public (reusable), internal (cog-only), or
    /// private (implementation detail). The exporting module validates
    /// refinements; the importing module trusts them.
    pub fn validate_refinement_predicate_access(
        &self,
        resolved: &ResolvedName,
        from_module: ModuleId,
    ) -> ModuleResult<()> {
        // Only check if the resolved name has a refinement
        if resolved.refinement.is_some() {
            // Get module paths for visibility checking
            let unknown_path = ModulePath::from_str("unknown");
            let item_module_path = self
                .module_paths
                .get(&resolved.module_id)
                .unwrap_or(&unknown_path);
            let from_module_path = self.module_paths.get(&from_module).unwrap_or(&unknown_path);

            // Check if predicate is accessible using standard visibility rules
            let is_accessible = match &resolved.predicate_visibility {
                Visibility::Public => true,
                Visibility::PublicCrate => {
                    item_module_path.segments().first() == from_module_path.segments().first()
                }
                Visibility::PublicSuper => match item_module_path.parent() {
                    Some(parent) => &parent == from_module_path,
                    None => false,
                },
                Visibility::PublicIn(path) => {
                    let path_str = path
                        .segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(".");
                    let target_path = ModulePath::from_str(&path_str);
                    target_path.is_prefix_of(from_module_path) || from_module_path == &target_path
                }
                Visibility::Private | Visibility::Internal | Visibility::Protected => {
                    item_module_path == from_module_path
                }
            };

            if !is_accessible {
                return Err(ModuleError::Other {
                    message: Text::from(format!(
                        "Refinement predicate for '{}' is not accessible from this module",
                        resolved.local_name
                    )),
                    span: None,
                });
            }
        }

        Ok(())
    }
}

impl Default for NameResolver {
    fn default() -> Self {
        Self::new()
    }
}
