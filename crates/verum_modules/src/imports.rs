//! Import resolution for modules.
//!
//! Resolves import statements (mount/import) to actual items in modules.
//! Supports single imports, glob imports (path.*), nested imports (path.{A, B}),
//! and renamed imports (path.X as Y). Visibility is checked during resolution
//! to ensure only accessible items are imported.

use crate::error::{ModuleError, ModuleResult};
use crate::exports::{ExportKind, ExportTable};
use crate::path::{ModuleId, ModulePath};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use verum_ast::{MountDecl, MountTree, MountTreeKind, Path, PathSegment, Span};
use verum_common::{List, Map, Maybe, Text};

/// Convert an AST Path to a ModulePath.
///
/// This function extracts the path segments from an AST Path and converts
/// them to a ModulePath suitable for module resolution. It properly handles:
/// - Named segments (identifiers)
/// - Generic arguments (ignored for module paths)
/// - Self/super/crate keywords
///
/// Converts an AST Path to a ModulePath. Module paths follow hierarchical
/// structure: absolute paths start from crate root (crate.*), relative paths
/// use self/super/direct names. Path segments are dot-separated identifiers.
pub fn path_to_module_path(path: &Path) -> ModulePath {
    let mut segments = List::new();

    for segment in &path.segments {
        match segment {
            PathSegment::Name(ident) => {
                segments.push(ident.name.clone());
            }
            PathSegment::SelfValue => {
                segments.push(Text::from("self"));
            }
            PathSegment::Super => {
                segments.push(Text::from("super"));
            }
            PathSegment::Cog => {
                segments.push(Text::from("cog"));
            }
            PathSegment::Relative => {
                // Relative path marker - skip
            }
        }
    }

    ModulePath::new(segments)
}

/// Extract the last segment name from a Path.
///
/// Returns the name of the last segment if it's an identifier,
/// or an error if the path is empty or ends with a keyword.
pub fn path_last_segment_name(path: &Path) -> Option<Text> {
    path.segments.last().and_then(|segment| match segment {
        PathSegment::Name(ident) => Some(ident.name.clone()),
        _ => None,
    })
}

/// Get the parent path (all segments except the last).
///
/// Returns None if the path has 0 or 1 segments.
pub fn path_parent(path: &Path) -> Option<ModulePath> {
    if path.segments.len() <= 1 {
        return None;
    }

    let mut segments = List::new();
    for (i, segment) in path.segments.iter().enumerate() {
        if i < path.segments.len() - 1 {
            match segment {
                PathSegment::Name(ident) => {
                    segments.push(ident.name.clone());
                }
                PathSegment::SelfValue => {
                    segments.push(Text::from("self"));
                }
                PathSegment::Super => {
                    segments.push(Text::from("super"));
                }
                PathSegment::Cog => {
                    segments.push(Text::from("cog"));
                }
                PathSegment::Relative => {
                    // Relative path marker - skip
                }
            }
        }
    }

    Some(ModulePath::new(segments))
}

/// A resolved import statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedImport {
    /// The original import declaration
    pub original_path: ModulePath,
    /// The items imported
    pub items: List<ImportedItem>,
    /// Whether this is a glob import
    pub is_glob: bool,
    /// The importing module
    pub importing_module: ModuleId,
    /// Span of the import statement
    pub span: Span,
}

impl ResolvedImport {
    pub fn new(
        original_path: ModulePath,
        items: List<ImportedItem>,
        is_glob: bool,
        importing_module: ModuleId,
        span: Span,
    ) -> Self {
        Self {
            original_path,
            items,
            is_glob,
            importing_module,
            span,
        }
    }

    /// Create a single-item import
    pub fn single(
        path: ModulePath,
        item: ImportedItem,
        importing_module: ModuleId,
        span: Span,
    ) -> Self {
        Self::new(path, vec![item].into(), false, importing_module, span)
    }

    /// Create a glob import
    pub fn glob(
        path: ModulePath,
        items: List<ImportedItem>,
        importing_module: ModuleId,
        span: Span,
    ) -> Self {
        Self::new(path, items, true, importing_module, span)
    }
}

/// An imported item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedItem {
    /// Name as it appears in the importing module
    pub name: Text,
    /// Original name in the source module
    pub original_name: Text,
    /// The module containing the item
    pub source_module: ModuleId,
    /// Kind of item
    pub kind: ExportKind,
    /// Span of the import
    pub span: Span,
}

impl ImportedItem {
    pub fn new(
        name: impl Into<Text>,
        original_name: impl Into<Text>,
        source_module: ModuleId,
        kind: ExportKind,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            original_name: original_name.into(),
            source_module,
            kind,
            span,
        }
    }

    /// Create an imported item without renaming
    pub fn direct(
        name: impl Into<Text>,
        source_module: ModuleId,
        kind: ExportKind,
        span: Span,
    ) -> Self {
        let name_text = name.into();
        Self {
            name: name_text.clone(),
            original_name: name_text,
            source_module,
            kind,
            span,
        }
    }

    /// Check if this import has renaming
    pub fn is_renamed(&self) -> bool {
        self.name != self.original_name
    }
}

/// Filter for glob imports.
///
/// Glob filters allow fine-grained control over which items are imported
/// from a glob import. Supports two modes:
/// 1. Hiding mode: `import std.io.* hiding Read` - imports everything except Read
/// 2. Selection mode: `import std.io.* only [Read, Write]` - imports only Read and Write
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobFilter {
    /// Items to exclude from glob import (hiding mode)
    hidden: HashSet<Text>,
    /// Items to include in glob import (selection mode)
    selected: Option<HashSet<Text>>,
}

impl GlobFilter {
    /// Create a new filter with no restrictions.
    pub fn new() -> Self {
        Self {
            hidden: HashSet::new(),
            selected: None,
        }
    }

    /// Create a filter that hides specific items.
    pub fn hiding<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        Self {
            hidden: items.into_iter().map(|s| s.into()).collect(),
            selected: None,
        }
    }

    /// Create a filter that only allows specific items.
    pub fn only<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        Self {
            hidden: HashSet::new(),
            selected: Some(items.into_iter().map(|s| s.into()).collect()),
        }
    }

    /// Add an item to the hidden list.
    pub fn hide(&mut self, item: impl Into<Text>) {
        self.hidden.insert(item.into());
    }

    /// Add an item to the selection list.
    pub fn select(&mut self, item: impl Into<Text>) {
        let selected = self.selected.get_or_insert_with(HashSet::new);
        selected.insert(item.into());
    }

    /// Check if an item is allowed by this filter.
    pub fn allows(&self, name: &str) -> bool {
        let name_text = Text::from(name);
        if let Some(ref selected) = self.selected {
            return selected.contains(&name_text) && !self.hidden.contains(&name_text);
        }
        !self.hidden.contains(&name_text)
    }

    /// Check if an item is allowed by this filter (Text version).
    pub fn allows_text(&self, name: &Text) -> bool {
        if let Some(ref selected) = self.selected {
            return selected.contains(name) && !self.hidden.contains(name);
        }
        !self.hidden.contains(name)
    }

    /// Check if this filter is empty (no restrictions).
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty() && self.selected.is_none()
    }

    /// Get the hidden items.
    pub fn hidden_items(&self) -> impl Iterator<Item = &Text> {
        self.hidden.iter()
    }

    /// Get the selected items (if in selection mode).
    pub fn selected_items(&self) -> Option<impl Iterator<Item = &Text>> {
        self.selected.as_ref().map(|s| s.iter())
    }
}

impl Default for GlobFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Import resolver - resolves import statements to actual items.
#[derive(Debug)]
pub struct ImportResolver {
    /// Cache of resolved imports
    cache: Map<(ModuleId, Text), ResolvedImport>,
}

impl ImportResolver {
    pub fn new() -> Self {
        Self { cache: Map::new() }
    }

    /// Resolve an import declaration.
    ///
    /// This performs the following steps:
    /// 1. Parse the import path
    /// 2. Find the target module
    /// 3. Check visibility
    /// 4. Resolve items
    /// 5. Handle renaming
    ///
    /// Resolves an import declaration by: (1) parsing the import path,
    /// (2) finding the target module, (3) checking visibility, (4) resolving
    /// items, and (5) handling renaming.
    pub fn resolve_import(
        &mut self,
        import: &MountDecl,
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
    ) -> ModuleResult<ResolvedImport> {
        let cache_key = (importing_module, Text::from(format!("{:?}", import)));

        // Check cache
        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let resolved = self.resolve_import_tree(
            &import.tree,
            importing_module,
            module_exports,
            module_paths,
            import.span,
        )?;

        // Cache the result
        self.cache.insert(cache_key, resolved.clone());
        Ok(resolved)
    }

    /// Resolve an import tree (handles nested imports).
    fn resolve_import_tree(
        &self,
        tree: &MountTree,
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
        span: Span,
    ) -> ModuleResult<ResolvedImport> {
        match &tree.kind {
            MountTreeKind::Path(path) => self.resolve_simple_import(
                path,
                importing_module,
                module_exports,
                module_paths,
                span,
            ),
            MountTreeKind::Glob(path) => {
                self.resolve_glob_import(path, importing_module, module_exports, module_paths, span)
            }
            MountTreeKind::Nested { prefix, trees } => self.resolve_nested_import(
                prefix,
                trees,
                importing_module,
                module_exports,
                module_paths,
                span,
            ),
        }
    }

    /// Resolve a simple import: `import std.io.File`
    fn resolve_simple_import(
        &self,
        path: &Path,
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
        span: Span,
    ) -> ModuleResult<ResolvedImport> {
        // Convert Path to ModulePath using proper segment extraction
        // The module path is the parent of the full path (without the item name)
        let module_path = path_parent(path).unwrap_or_else(|| path_to_module_path(path));

        // Find the target module
        let target_module = self.find_module_by_path(&module_path, module_paths)?;

        // Get the last segment as the item name
        let item_name_segment = path
            .segments
            .last()
            .ok_or_else(|| ModuleError::InvalidPath {
                path: Text::from(format!("{:?}", path)),
                reason: Text::from("empty path"),
                span: Some(span),
            })?;

        // Extract the name from the PathSegment
        let item_name_text = match item_name_segment {
            verum_ast::PathSegment::Name(ident) => &ident.name,
            _ => {
                return Err(ModuleError::InvalidPath {
                    path: Text::from(format!("{:?}", path)),
                    reason: Text::from("expected identifier, got keyword"),
                    span: Some(span),
                });
            }
        };

        // Look up the item in exports
        let exports = match module_exports.get(&target_module) {
            Some(e) => e,
            None => {
                return Err(ModuleError::ModuleNotFound {
                    path: module_path.clone(),
                    searched_paths: List::new(),
                    suggestions: List::new(),
                    span: Some(span),
                });
            }
        };

        let exported_item = match exports.get(item_name_text) {
            Maybe::Some(item) => item,
            Maybe::None => {
                let available_items: List<Text> = exports
                    .all_exports()
                    .map(|(name, _)| name.clone())
                    .collect::<List<_>>();
                let suggestions = crate::suggestions::find_similar_items(
                    item_name_text.as_str(),
                    &available_items,
                );
                return Err(ModuleError::ItemNotFound {
                    item_name: item_name_text.clone(),
                    module_path: module_path.clone(),
                    available_items,
                    suggestions,
                    span: Some(span),
                });
            }
        };

        // Check visibility using path-based API for proper PublicCrate/PublicSuper/PublicIn checks
        let importing_path = module_paths
            .get(&importing_module)
            .cloned()
            .unwrap_or_else(|| ModulePath::from_str("unknown"));
        if !exports.is_visible_from_path(item_name_text, &importing_path) {
            return Err(ModuleError::PrivateAccess {
                item_name: item_name_text.clone(),
                item_module: module_path,
                accessing_module: importing_path,
                span: Some(span),
            });
        }

        let imported_item = ImportedItem::direct(
            item_name_text.clone(),
            target_module,
            exported_item.kind,
            span,
        );

        Ok(ResolvedImport::single(
            module_path,
            imported_item,
            importing_module,
            span,
        ))
    }

    /// Resolve a glob import: `import std.io.*`
    fn resolve_glob_import(
        &self,
        path: &Path,
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
        span: Span,
    ) -> ModuleResult<ResolvedImport> {
        // Convert Path to ModulePath using proper segment extraction
        let module_path = path_to_module_path(path);
        let target_module = self.find_module_by_path(&module_path, module_paths)?;

        let exports = match module_exports.get(&target_module) {
            Some(e) => e,
            None => {
                return Err(ModuleError::ModuleNotFound {
                    path: module_path.clone(),
                    searched_paths: List::new(),
                    suggestions: List::new(),
                    span: Some(span),
                });
            }
        };

        // Import all visible items using path-based visibility checks
        let importing_path = module_paths
            .get(&importing_module)
            .cloned()
            .unwrap_or_else(|| ModulePath::from_str("unknown"));
        let mut items = List::new();
        for (name, exported_item) in exports.public_exports().map(|e| (e.name.clone(), e)) {
            if exports.is_visible_from_path(&name, &importing_path) {
                items.push(ImportedItem::direct(
                    name,
                    target_module,
                    exported_item.kind,
                    span,
                ));
            }
        }

        Ok(ResolvedImport::glob(
            module_path,
            items,
            importing_module,
            span,
        ))
    }

    /// Resolve a glob import with filtering: `import std.io.* hiding Read`
    ///
    /// This method supports filtering glob imports in two modes:
    /// 1. Hiding mode: Import all items except those in the filter
    /// 2. Selection mode: Import only items in the filter
    pub fn resolve_glob_import_filtered(
        &self,
        path: &Path,
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
        span: Span,
        filter: &GlobFilter,
    ) -> ModuleResult<ResolvedImport> {
        let module_path = path_to_module_path(path);
        let target_module = self.find_module_by_path(&module_path, module_paths)?;

        let exports = match module_exports.get(&target_module) {
            Some(e) => e,
            None => {
                return Err(ModuleError::ModuleNotFound {
                    path: module_path.clone(),
                    searched_paths: List::new(),
                    suggestions: List::new(),
                    span: Some(span),
                });
            }
        };

        // Import all visible items that pass the filter using path-based visibility checks
        let importing_path = module_paths
            .get(&importing_module)
            .cloned()
            .unwrap_or_else(|| ModulePath::from_str("unknown"));
        let mut items = List::new();
        for (name, exported_item) in exports.public_exports().map(|e| (e.name.clone(), e)) {
            if filter.allows_text(&name) && exports.is_visible_from_path(&name, &importing_path) {
                items.push(ImportedItem::direct(
                    name,
                    target_module,
                    exported_item.kind,
                    span,
                ));
            }
        }

        // If in selection mode, verify that all selected items were found
        if let Some(selected) = filter.selected_items() {
            for selected_name in selected {
                if !exports.contains(selected_name.as_str()) {
                    let available_items: List<Text> = exports
                        .all_exports()
                        .map(|(name, _)| name.clone())
                        .collect::<List<_>>();
                    let suggestions = crate::suggestions::find_similar_items(
                        selected_name.as_str(),
                        &available_items,
                    );
                    return Err(ModuleError::ItemNotFound {
                        item_name: selected_name.clone(),
                        module_path: module_path.clone(),
                        available_items,
                        suggestions,
                        span: Some(span),
                    });
                }
            }
        }

        Ok(ResolvedImport::glob(
            module_path,
            items,
            importing_module,
            span,
        ))
    }

    /// Resolve nested imports: `import std.io.{File, Read, Write}`
    fn resolve_nested_import(
        &self,
        prefix: &Path,
        trees: &[MountTree],
        importing_module: ModuleId,
        module_exports: &Map<ModuleId, ExportTable>,
        module_paths: &Map<ModuleId, ModulePath>,
        span: Span,
    ) -> ModuleResult<ResolvedImport> {
        // Convert Path to ModulePath using proper segment extraction
        let prefix_path = path_to_module_path(prefix);
        let mut all_items = List::new();

        for tree in trees {
            let resolved = self.resolve_import_tree(
                tree,
                importing_module,
                module_exports,
                module_paths,
                span,
            )?;
            all_items.extend(resolved.items);
        }

        Ok(ResolvedImport::new(
            prefix_path,
            all_items,
            false,
            importing_module,
            span,
        ))
    }

    /// Find a module by its path
    fn find_module_by_path(
        &self,
        path: &ModulePath,
        module_paths: &Map<ModuleId, ModulePath>,
    ) -> ModuleResult<ModuleId> {
        for (id, mod_path) in module_paths.iter() {
            if mod_path == path {
                return Ok(*id);
            }
        }

        Err(ModuleError::module_not_found(path.clone(), List::new()))
    }

    /// Clear the cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for ImportResolver {
    fn default() -> Self {
        Self::new()
    }
}
