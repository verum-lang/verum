//! Workspace-wide symbol index for cross-file navigation
//!
//! Maintains a mapping of symbols to their locations across all .vr files
//! in the workspace. Supports cross-file goto definition, find references,
//! and rename operations.

use dashmap::DashMap;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::*;
use verum_ast::{ItemKind, Module};

use crate::document::DocumentStore;
use crate::position_utils::ast_span_to_range;
use crate::references;

/// A symbol's location in the workspace
#[derive(Debug, Clone)]
pub struct SymbolLocation {
    /// The URI of the file containing the symbol
    pub uri: Url,
    /// The range of the symbol definition
    pub range: Range,
    /// The kind of symbol
    pub kind: SymbolExportKind,
    /// The symbol name
    pub name: String,
}

/// What kind of exported symbol this is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolExportKind {
    Function,
    Type,
    Protocol,
    Constant,
    Module,
}

/// A mount (import) statement parsed from a file
#[derive(Debug, Clone)]
pub struct MountInfo {
    /// The module path being imported (e.g., "collections.list")
    pub module_path: String,
    /// Specific symbols imported (empty = import all)
    pub symbols: Vec<String>,
    /// The span range of the mount statement
    pub range: Range,
}

/// Workspace-wide index for cross-file navigation
pub struct WorkspaceIndex {
    /// Map from file URI to module name
    uri_to_module: DashMap<Url, String>,
    /// Map from module name to file URI
    module_to_uri: DashMap<String, Url>,
    /// Exported symbols: symbol name -> locations across workspace
    exports: DashMap<String, Vec<SymbolLocation>>,
    /// Mount graph: file URI -> list of mount statements
    mount_graph: DashMap<Url, Vec<MountInfo>>,
    /// Workspace root path
    workspace_root: parking_lot::RwLock<Option<PathBuf>>,
}

impl WorkspaceIndex {
    /// Create a new empty workspace index
    pub fn new() -> Self {
        Self {
            uri_to_module: DashMap::new(),
            module_to_uri: DashMap::new(),
            exports: DashMap::new(),
            mount_graph: DashMap::new(),
            workspace_root: parking_lot::RwLock::new(None),
        }
    }

    /// Initialize the workspace index by scanning for .vr files
    pub fn initialize(&self, workspace_root: &Path) {
        *self.workspace_root.write() = Some(workspace_root.to_path_buf());
        tracing::info!("Initializing workspace index at: {:?}", workspace_root);

        // Scan for .vr files recursively
        if let Ok(entries) = Self::find_vr_files(workspace_root) {
            for path in entries {
                if let Some(module_name) = self.path_to_module_name(workspace_root, &path) {
                    if let Ok(uri) = Url::from_file_path(&path) {
                        self.uri_to_module.insert(uri.clone(), module_name.clone());
                        self.module_to_uri.insert(module_name, uri);
                    }
                }
            }
        }

        tracing::info!(
            "Workspace index initialized: {} modules",
            self.uri_to_module.len()
        );
    }

    /// Index a document's exports and mount statements
    pub fn index_document(&self, uri: &Url, module: &Module, text: &str) {
        // Extract module name from URI
        let module_name = self
            .uri_to_module
            .get(uri)
            .map(|v| v.clone())
            .unwrap_or_else(|| {
                // Derive module name from file path
                uri.to_file_path()
                    .ok()
                    .and_then(|p| {
                        let root = self.workspace_root.read();
                        root.as_ref()
                            .and_then(|r| self.path_to_module_name(r, &p))
                    })
                    .unwrap_or_else(|| "unknown".to_string())
            });

        // Register URI <-> module mapping
        self.uri_to_module
            .insert(uri.clone(), module_name.clone());
        self.module_to_uri
            .insert(module_name.clone(), uri.clone());

        // Clear old exports for this URI
        self.remove_exports_for_uri(uri);

        // Extract exports (top-level items)
        for item in module.items.iter() {
            let (name, kind) = match &item.kind {
                ItemKind::Function(func) => {
                    (func.name.as_str().to_string(), SymbolExportKind::Function)
                }
                ItemKind::Type(type_decl) => {
                    (type_decl.name.as_str().to_string(), SymbolExportKind::Type)
                }
                ItemKind::Protocol(protocol) => (
                    protocol.name.as_str().to_string(),
                    SymbolExportKind::Protocol,
                ),
                ItemKind::Const(const_decl) => (
                    const_decl.name.as_str().to_string(),
                    SymbolExportKind::Constant,
                ),
                ItemKind::Module(mod_decl) => {
                    (mod_decl.name.as_str().to_string(), SymbolExportKind::Module)
                }
                _ => continue,
            };

            let range = ast_span_to_range(&item.span, text);
            let location = SymbolLocation {
                uri: uri.clone(),
                range,
                kind,
                name: name.clone(),
            };

            self.exports
                .entry(name)
                .or_insert_with(Vec::new)
                .push(location);
        }

        // Extract mount statements
        let mut mounts = Vec::new();
        for item in module.items.iter() {
            if let ItemKind::Mount(_mount_tree) = &item.kind {
                let range = ast_span_to_range(&item.span, text);
                // Extract the mount path from the source text
                let span_start = item.span.start as usize;
                let span_end = (item.span.end as usize).min(text.len());
                let mount_text = &text[span_start..span_end];

                if let Some(info) = parse_mount_text(mount_text, range) {
                    mounts.push(info);
                }
            }
        }
        self.mount_graph.insert(uri.clone(), mounts);
    }

    /// Remove all exported symbols from a specific URI
    fn remove_exports_for_uri(&self, uri: &Url) {
        let mut to_remove = Vec::new();
        for mut entry in self.exports.iter_mut() {
            entry.value_mut().retain(|loc| &loc.uri != uri);
            if entry.value().is_empty() {
                to_remove.push(entry.key().clone());
            }
        }
        for key in to_remove {
            self.exports.remove(&key);
        }
    }

    /// Resolve a mount path to a file URI
    pub fn resolve_mount(&self, _from_uri: &Url, mount_path: &str) -> Option<Url> {
        // Try direct module name lookup
        if let Some(entry) = self.module_to_uri.get(mount_path) {
            return Some(entry.clone());
        }

        // Try with dot-separated path segments
        let normalized = mount_path.replace('.', "/");
        for entry in self.module_to_uri.iter() {
            if entry.key().replace('.', "/") == normalized {
                return Some(entry.value().clone());
            }
        }

        // Try file-path based resolution
        let root = self.workspace_root.read();
        if let Some(root) = root.as_ref() {
            let candidate = root.join(format!("{}.vr", normalized));
            if candidate.exists() {
                return Url::from_file_path(&candidate).ok();
            }
            let candidate = root.join(&normalized).join("mod.vr");
            if candidate.exists() {
                return Url::from_file_path(&candidate).ok();
            }
        }

        None
    }

    /// Find a symbol across the entire workspace
    pub fn find_symbol_across_workspace(&self, name: &str) -> Vec<SymbolLocation> {
        self.exports
            .get(name)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// Find all files that import a given module
    pub fn find_files_importing_module(&self, module_name: &str) -> Vec<Url> {
        let mut result = Vec::new();
        for entry in self.mount_graph.iter() {
            for mount in entry.value().iter() {
                if mount.module_path == module_name
                    || mount.module_path.starts_with(&format!("{}.", module_name))
                {
                    result.push(entry.key().clone());
                    break;
                }
            }
        }
        result
    }

    /// Find all files that import a given symbol
    pub fn find_files_importing_symbol(&self, symbol_name: &str) -> Vec<Url> {
        let mut result = Vec::new();
        for entry in self.mount_graph.iter() {
            for mount in entry.value().iter() {
                if mount.symbols.iter().any(|s| s == symbol_name) {
                    result.push(entry.key().clone());
                    break;
                }
            }
        }
        result
    }

    /// Get the module name for a URI
    pub fn get_module_name(&self, uri: &Url) -> Option<String> {
        self.uri_to_module.get(uri).map(|v| v.clone())
    }

    /// Get all indexed module names
    pub fn module_count(&self) -> usize {
        self.uri_to_module.len()
    }

    /// Convert a file path to a module name relative to the workspace root
    fn path_to_module_name(&self, root: &Path, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(root).ok()?;
        let stem = relative.with_extension("");
        let module_name = stem
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join(".");

        // Strip trailing ".mod" for directory modules
        let module_name = module_name
            .strip_suffix(".mod")
            .unwrap_or(&module_name)
            .to_string();

        if module_name.is_empty() {
            None
        } else {
            Some(module_name)
        }
    }

    /// Recursively find all .vr files in a directory
    fn find_vr_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        Self::find_vr_files_recursive(dir, &mut files)?;
        Ok(files)
    }

    fn find_vr_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                // Skip hidden directories and common non-source directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    Self::find_vr_files_recursive(&path, files)?;
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("vr") {
                files.push(path);
            }
        }
        Ok(())
    }
}

impl Default for WorkspaceIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse mount statement text into MountInfo
fn parse_mount_text(text: &str, range: Range) -> Option<MountInfo> {
    let text = text.trim();

    // "mount foo.bar.{baz, qux}" or "mount foo.bar"
    let text = text.strip_prefix("mount")?.trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim();

    // Check for selective imports: "foo.bar.{a, b, c}"
    if let Some(brace_start) = text.find('{') {
        let module_path = text[..brace_start]
            .trim()
            .trim_end_matches('.')
            .to_string();
        let symbols_text = text[brace_start + 1..]
            .trim_end_matches('}')
            .trim();
        let symbols: Vec<String> = symbols_text
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Some(MountInfo {
            module_path,
            symbols,
            range,
        })
    } else {
        // Simple mount: "mount foo.bar"
        let module_path = text.to_string();
        Some(MountInfo {
            module_path,
            symbols: Vec::new(),
            range,
        })
    }
}

// ==================== Cross-File Navigation ====================

/// Find definition across files, falling back to workspace index
pub fn goto_definition_cross_file(
    document_store: &DocumentStore,
    workspace_index: &WorkspaceIndex,
    uri: &Url,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    // First try single-file lookup
    let single_file_result = document_store
        .with_document(uri, |doc| {
            crate::goto_definition::goto_definition(doc, position, uri)
        })
        .flatten();

    if single_file_result.is_some() {
        return single_file_result;
    }

    // Get the word at position
    let word = document_store
        .with_document(uri, |doc| doc.word_at_position(position))
        .flatten()?;

    // Check mount statements to find which module the symbol might come from
    if let Some(mounts) = workspace_index.mount_graph.get(uri) {
        for mount in mounts.value().iter() {
            // Check if this mount imports the symbol
            let imports_symbol = mount.symbols.is_empty() // wildcard import
                || mount.symbols.iter().any(|s| s == &word);

            if imports_symbol {
                // Try to resolve the mount to a file
                if let Some(target_uri) = workspace_index.resolve_mount(uri, &mount.module_path) {
                    // Look for the symbol in the target file
                    let found = document_store
                        .with_document(&target_uri, |doc| {
                            crate::goto_definition::goto_definition(doc, Position::default(), &target_uri)
                                .or_else(|| {
                                    // Direct symbol table lookup
                                    let symbol = doc.get_symbol(&word)?;
                                    let range = ast_span_to_range(&symbol.def_span, &doc.text);
                                    Some(GotoDefinitionResponse::Scalar(Location {
                                        uri: target_uri.clone(),
                                        range,
                                    }))
                                })
                        })
                        .flatten();

                    if found.is_some() {
                        return found;
                    }
                }
            }
        }
    }

    // Fall back to workspace-wide symbol search
    let locations = workspace_index.find_symbol_across_workspace(&word);
    if locations.is_empty() {
        return None;
    }

    // Filter out the current file's own definitions
    let external: Vec<_> = locations
        .iter()
        .filter(|loc| &loc.uri != uri)
        .collect();

    if external.len() == 1 {
        Some(GotoDefinitionResponse::Scalar(Location {
            uri: external[0].uri.clone(),
            range: external[0].range,
        }))
    } else if external.len() > 1 {
        Some(GotoDefinitionResponse::Array(
            external
                .iter()
                .map(|loc| Location {
                    uri: loc.uri.clone(),
                    range: loc.range,
                })
                .collect(),
        ))
    } else if locations.len() == 1 {
        Some(GotoDefinitionResponse::Scalar(Location {
            uri: locations[0].uri.clone(),
            range: locations[0].range,
        }))
    } else {
        Some(GotoDefinitionResponse::Array(
            locations
                .iter()
                .map(|loc| Location {
                    uri: loc.uri.clone(),
                    range: loc.range,
                })
                .collect(),
        ))
    }
}

/// Find references across all workspace files
pub fn find_references_cross_file(
    document_store: &DocumentStore,
    workspace_index: &WorkspaceIndex,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let mut all_locations = Vec::new();

    // Get the symbol name at position
    let word = match document_store
        .with_document(uri, |doc| doc.word_at_position(position))
        .flatten()
    {
        Some(w) => w,
        None => return all_locations,
    };

    // Find references in the current file
    let local_refs = document_store
        .with_document(uri, |doc| {
            references::find_references(doc, position, uri, include_declaration)
        })
        .unwrap_or_default();
    all_locations.extend(local_refs);

    // Find references in other files that import this symbol
    let module_name = workspace_index.get_module_name(uri);

    // Search files that mount the module containing this symbol
    if let Some(ref mod_name) = module_name {
        let importing_files = workspace_index.find_files_importing_module(mod_name);
        for file_uri in importing_files {
            if &file_uri == uri {
                continue; // Already searched
            }

            let refs = document_store
                .with_document(&file_uri, |doc| {
                    if let Some(module) = &doc.module {
                        let refs = references::find_ast_references(module, &word, &file_uri, &doc.text);
                        let locations: Vec<Location> = refs
                            .into_iter()
                            .filter(|r| {
                                include_declaration
                                    || r.kind != references::ReferenceKind::Definition
                            })
                            .map(|r| r.location)
                            .collect();
                        locations
                    } else {
                        Vec::new()
                    }
                })
                .unwrap_or_default();

            all_locations.extend(refs);
        }
    }

    // Also search files that import the symbol by name
    let importing_by_name = workspace_index.find_files_importing_symbol(&word);
    for file_uri in importing_by_name {
        if &file_uri == uri {
            continue;
        }
        // Skip if already searched via module import
        if module_name.is_some()
            && workspace_index
                .find_files_importing_module(module_name.as_ref().unwrap())
                .contains(&file_uri)
        {
            continue;
        }

        let refs = document_store
            .with_document(&file_uri, |doc| {
                if let Some(module) = &doc.module {
                    let refs = references::find_ast_references(module, &word, &file_uri, &doc.text);
                    let locations: Vec<Location> = refs
                        .into_iter()
                        .filter(|r| {
                            include_declaration
                                || r.kind != references::ReferenceKind::Definition
                        })
                        .map(|r| r.location)
                        .collect();
                    locations
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();

        all_locations.extend(refs);
    }

    all_locations
}

/// Perform cross-file rename across the workspace
pub fn rename_cross_file(
    document_store: &DocumentStore,
    workspace_index: &WorkspaceIndex,
    uri: &Url,
    position: Position,
    new_name: String,
) -> Option<WorkspaceEdit> {
    use std::collections::HashMap;

    // Verify there's a symbol at position
    let _word = document_store
        .with_document(uri, |doc| doc.word_at_position(position))
        .flatten()?;

    // Validate new name
    if !crate::rename::is_valid_identifier(&new_name) || crate::rename::is_keyword(&new_name) {
        return None;
    }

    // Find all references across the workspace
    let all_refs = find_references_cross_file(document_store, workspace_index, uri, position, true);

    if all_refs.is_empty() {
        return None;
    }

    // Group edits by URI
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for location in all_refs {
        changes
            .entry(location.uri)
            .or_default()
            .push(TextEdit {
                range: location.range,
                new_text: new_name.clone(),
            });
    }

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mount_text_simple() {
        let range = Range::default();
        let info = parse_mount_text("mount foo.bar;", range).unwrap();
        assert_eq!(info.module_path, "foo.bar");
        assert!(info.symbols.is_empty());
    }

    #[test]
    fn test_parse_mount_text_selective() {
        let range = Range::default();
        let info = parse_mount_text("mount foo.bar.{baz, qux};", range).unwrap();
        assert_eq!(info.module_path, "foo.bar");
        assert_eq!(info.symbols, vec!["baz", "qux"]);
    }

    #[test]
    fn test_workspace_index_new() {
        let index = WorkspaceIndex::new();
        assert_eq!(index.module_count(), 0);
    }

    #[test]
    fn test_find_symbol_empty() {
        let index = WorkspaceIndex::new();
        assert!(index.find_symbol_across_workspace("foo").is_empty());
    }
}
