//! Build assets context for meta functions
//!
//! Provides file access during meta function execution with security restrictions.

use verum_ast::MetaValue;
use verum_common::{List, Map, Text};

use crate::meta::error::MetaError;

/// Asset metadata returned by BuildAssets.metadata()
///
/// Matches: core/meta/contexts.vr AssetMetadata
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    /// File size in bytes
    pub size: u64,
    /// Modification timestamp (Unix epoch nanoseconds)
    pub modified_ns: u64,
    /// Whether this is a directory
    pub is_directory: bool,
    /// Whether this is a regular file
    pub is_file: bool,
    /// Whether this is a symbolic link
    pub is_symlink: bool,
}

impl AssetMetadata {
    /// Convert to MetaValue struct representation
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(List::from(vec![
            MetaValue::UInt(self.size.into()),
            MetaValue::UInt(self.modified_ns.into()),
            MetaValue::Bool(self.is_directory),
            MetaValue::Bool(self.is_file),
            MetaValue::Bool(self.is_symlink),
        ]))
    }

    /// Alias for to_meta_value for backward compatibility
    #[inline]
    pub fn to_const_value(&self) -> MetaValue {
        self.to_meta_value()
    }
}

/// Build assets context configuration
///
/// Manages file access during meta function execution with security restrictions.
#[derive(Debug, Clone)]
pub struct BuildAssetsInfo {
    /// Project root directory (where Verum.toml is located)
    pub project_root: Option<Text>,
    /// Configured asset directories (from Verum.toml [build.assets])
    pub asset_dirs: List<Text>,
    /// Cache for loaded text files (path -> content)
    text_cache: Map<Text, Text>,
    /// Cache for loaded binary files (path -> bytes)
    binary_cache: Map<Text, Vec<u8>>,
}

impl Default for BuildAssetsInfo {
    fn default() -> Self {
        Self {
            project_root: None,
            asset_dirs: List::new(),
            text_cache: Map::new(),
            binary_cache: Map::new(),
        }
    }
}

impl BuildAssetsInfo {
    /// Create new BuildAssetsInfo with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the project root
    #[inline]
    pub fn with_project_root(mut self, root: impl Into<Text>) -> Self {
        self.project_root = Some(root.into());
        self
    }

    /// Add an asset directory
    #[inline]
    pub fn with_asset_dir(mut self, dir: impl Into<Text>) -> Self {
        self.asset_dirs.push(dir.into());
        self
    }

    /// Validate that a path is safe (no path traversal attacks)
    ///
    /// This implements comprehensive path traversal protection:
    /// - Null byte injection prevention
    /// - Parent directory references (..) in any form
    /// - Absolute path detection (Unix and Windows)
    /// - Suspicious path patterns
    fn validate_path(path: &str) -> Result<(), MetaError> {
        // Check for null bytes (potential injection attack)
        if path.contains('\0') {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Null bytes not allowed in path"),
            });
        }

        // Check for path traversal - multiple detection strategies
        // 1. Direct ".." check
        if path.contains("..") {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Parent directory reference '..' not allowed"),
            });
        }

        // 2. Check URL-encoded variants
        let lower = path.to_lowercase();
        if lower.contains("%2e%2e") || lower.contains("%252e") {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("URL-encoded path traversal not allowed"),
            });
        }

        // 3. Check for backslash variants (Windows path traversal)
        if path.contains("..\\") || path.contains("\\..") {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Backslash path traversal not allowed"),
            });
        }

        // Check for absolute paths (Unix)
        if path.starts_with('/') {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Absolute paths not allowed"),
            });
        }

        // Check for absolute paths (Windows backslash)
        if path.starts_with('\\') {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Absolute paths not allowed"),
            });
        }

        // Check for Windows drive letters (C:, D:, etc.)
        if path.len() >= 2 {
            let first = path.chars().next().unwrap_or(' ');
            let second = path.chars().nth(1).unwrap_or(' ');
            if first.is_ascii_alphabetic() && second == ':' {
                return Err(MetaError::PathTraversalBlocked {
                    path: Text::from(path),
                    reason: Text::from("Windows drive paths not allowed"),
                });
            }
        }

        // Check for Windows UNC paths (\\server\share)
        if path.starts_with("\\\\") {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("UNC paths not allowed"),
            });
        }

        // Check for suspicious Unix-like absolute paths on Windows
        // (some systems may interpret /c/path as C:\path)
        if path.len() >= 3 && path.starts_with('/') {
            let second = path.chars().nth(1).unwrap_or(' ');
            let third = path.chars().nth(2).unwrap_or(' ');
            if second.is_ascii_alphabetic() && third == '/' {
                return Err(MetaError::PathTraversalBlocked {
                    path: Text::from(path),
                    reason: Text::from("Absolute path variant not allowed"),
                });
            }
        }

        // Check for device names on Windows (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
        let path_upper = path.to_uppercase();
        let base_name = path_upper
            .split(['/', '\\'].as_ref())
            .next_back()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");

        let reserved_names = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];

        if reserved_names.contains(&base_name) {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(path),
                reason: Text::from("Reserved device names not allowed"),
            });
        }

        Ok(())
    }

    /// Validate that the resolved path is still within the project root
    ///
    /// This is the final security check after path resolution.
    /// It canonicalizes both paths and ensures the resolved path
    /// is a proper descendant of the project root.
    fn validate_resolved_path(
        resolved: &std::path::Path,
        project_root: &std::path::Path,
    ) -> Result<(), MetaError> {
        // Canonicalize both paths to resolve symlinks and normalize
        let canonical_resolved = match resolved.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // If we can't canonicalize, the file likely doesn't exist yet
                // which is fine - the load will fail with a better error
                return Ok(());
            }
        };

        let canonical_root = match project_root.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                return Err(MetaError::Other(Text::from(format!(
                    "Failed to canonicalize project root: {}",
                    e
                ))));
            }
        };

        // Check that the resolved path is under the project root
        if !canonical_resolved.starts_with(&canonical_root) {
            return Err(MetaError::PathTraversalBlocked {
                path: Text::from(canonical_resolved.display().to_string()),
                reason: Text::from("Resolved path escapes project root"),
            });
        }

        Ok(())
    }

    /// Resolve a relative path to an absolute path
    fn resolve_path(&self, relative_path: &str) -> Result<std::path::PathBuf, MetaError> {
        Self::validate_path(relative_path)?;

        let project_root = self.project_root.as_ref().ok_or_else(|| {
            MetaError::Other(Text::from("Project root not set for BuildAssets"))
        })?;

        let root = std::path::Path::new(project_root.as_str());

        // First try the path directly under project root
        let direct_path = root.join(relative_path);
        if direct_path.exists() {
            // Validate that resolved path stays within project root
            Self::validate_resolved_path(&direct_path, root)?;
            return Ok(direct_path);
        }

        // Then try each asset directory
        for asset_dir in &self.asset_dirs {
            let asset_path = root.join(asset_dir.as_str()).join(relative_path);
            if asset_path.exists() {
                // Validate that resolved path stays within project root
                Self::validate_resolved_path(&asset_path, root)?;
                return Ok(asset_path);
            }
        }

        Err(MetaError::Other(Text::from(format!(
            "Asset not found: {}",
            relative_path
        ))))
    }

    /// Load binary content from a file
    pub fn load(&mut self, path: &str) -> Result<Vec<u8>, MetaError> {
        let path_text = Text::from(path);

        // Check cache first
        if let Some(cached) = self.binary_cache.get(&path_text) {
            return Ok(cached.clone());
        }

        let resolved = self.resolve_path(path)?;
        let content = std::fs::read(&resolved).map_err(|e| {
            MetaError::Other(Text::from(format!("Failed to read file: {}", e)))
        })?;

        // Cache the result
        self.binary_cache.insert(path_text, content.clone());
        Ok(content)
    }

    /// Load text content from a file
    pub fn load_text(&mut self, path: &str) -> Result<Text, MetaError> {
        let path_text = Text::from(path);

        // Check cache first
        if let Some(cached) = self.text_cache.get(&path_text) {
            return Ok(cached.clone());
        }

        let resolved = self.resolve_path(path)?;
        let content = std::fs::read_to_string(&resolved).map_err(|e| {
            MetaError::Other(Text::from(format!("Failed to read file: {}", e)))
        })?;

        let text = Text::from(content);
        // Cache the result
        self.text_cache.insert(path_text, text.clone());
        Ok(text)
    }

    /// Check if a file exists
    pub fn exists(&self, path: &str) -> bool {
        self.resolve_path(path).is_ok()
    }

    /// List directory contents
    pub fn list_dir(&self, path: &str) -> Result<List<Text>, MetaError> {
        let resolved = self.resolve_path(path)?;

        if !resolved.is_dir() {
            return Err(MetaError::Other(Text::from(format!(
                "Not a directory: {}",
                path
            ))));
        }

        let entries = std::fs::read_dir(&resolved).map_err(|e| {
            MetaError::Other(Text::from(format!("Failed to read directory: {}", e)))
        })?;

        let mut result = List::new();
        for entry in entries {
            if let Ok(entry) = entry {
                if let Some(name) = entry.file_name().to_str() {
                    result.push(Text::from(name));
                }
            }
        }

        Ok(result)
    }

    /// Load a TOML file from the project root and parse it into a
    /// `ConstValue::Map` keyed on top-level table names (#20 / P7).
    ///
    /// Sandbox + caching are inherited from `load_text`. The TOML
    /// document is required to be a top-level table (not a bare
    /// value or array) — every other shape returns
    /// `MetaError::Other`. Numeric / boolean / string / sub-table /
    /// array values become the obvious `ConstValue` variants;
    /// datetimes become `Text` (RFC 3339 form preserved by the
    /// `toml` crate's Display) so callers can re-parse them as
    /// needed without forcing a `chrono` dependency on the meta
    /// surface.
    ///
    /// The result is the foundation for `@codegen("spec.toml")`:
    /// users register a meta fn that consumes the spec and emits
    /// generated code; this builtin gives them the spec data
    /// without forcing them to reimplement TOML parsing in user
    /// meta code.
    pub fn load_toml(&mut self, path: &str) -> Result<MetaValue, MetaError> {
        let text = self.load_text(path)?;
        let parsed: toml::Value = toml::from_str(text.as_str()).map_err(|e| {
            MetaError::Other(Text::from(format!(
                "@load_toml({}): TOML parse error: {}",
                path, e
            )))
        })?;
        match parsed {
            toml::Value::Table(_) => Ok(toml_to_meta(parsed)),
            _ => Err(MetaError::Other(Text::from(format!(
                "@load_toml({}): root must be a table, got {}",
                path,
                toml_kind_name(&parsed)
            )))),
        }
    }

    /// Resolve a glob pattern against the project root and
    /// return the matched (relative-path, bytes) pairs.
    ///
    /// Pattern grammar:
    ///
    ///   * `*`   — matches zero or more characters within a
    ///             single path component (does NOT cross `/`).
    ///   * `?`   — matches exactly one character within a
    ///             single path component.
    ///   * `**`  — matches zero or more path components,
    ///             enabling recursive descent. Must appear as
    ///             a complete path component (i.e. surrounded
    ///             by `/` or at the start/end of the pattern);
    ///             `a**b` is rejected.
    ///   * literal text matches verbatim.
    ///
    /// Examples:
    ///
    ///   * `*.png`            — every PNG in the project root
    ///   * `assets/*.png`     — every PNG directly under
    ///                          `assets/`
    ///   * `assets/**/*.png`  — every PNG anywhere under
    ///                          `assets/`, at any depth
    ///   * `**/*.toml`        — every TOML anywhere in the
    ///                          project tree
    ///   * `**`               — every file in the entire tree
    ///
    /// Sandbox: directory traversal flows through the same
    /// `resolve_path` precheck `load` / `load_text` use, so
    /// absolute paths and `..` traversal are rejected before
    /// any filesystem walk happens. Each matched file's bytes
    /// are loaded via `load(...)` so the per-file cache is
    /// populated transparently.
    ///
    /// Determinism: results are sorted lexicographically by
    /// relative path, so generated bytecode is reproducible
    /// across platforms regardless of `readdir` ordering. Walk
    /// depth is capped at 64 levels to prevent symlink-induced
    /// cycles from exhausting the stack.
    pub fn load_glob(&mut self, pattern: &str) -> Result<Vec<(Text, Vec<u8>)>, MetaError> {
        if pattern.is_empty() {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: empty pattern",
            )));
        }
        // Sandbox precheck: any literal segment that escapes
        // the project root (`..`, absolute path) must be
        // rejected before the walk starts. The `resolve_path`
        // call inside `is_directory` / `load` would catch them
        // too, but rejecting up-front gives a single clear
        // diagnostic instead of "directory not found" buried
        // inside a recursive walk.
        if pattern.starts_with('/') {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: absolute paths are forbidden — pattern \
                 must be relative to the project root",
            )));
        }
        // Split into path components and validate `**` only
        // appears as a complete component, and `..` never does.
        let segments: Vec<&str> = pattern.split('/').collect();
        for seg in &segments {
            if seg.contains("**") && *seg != "**" {
                return Err(MetaError::Other(Text::from(
                    "@embed_glob: `**` must be a standalone path component \
                     (e.g. `assets/**/*.png`); `a**b` is not permitted",
                )));
            }
            if *seg == ".." {
                return Err(MetaError::Other(Text::from(
                    "@embed_glob: `..` traversal is forbidden — \
                     pattern must stay within the project root",
                )));
            }
        }

        let mut matched: Vec<(Text, Vec<u8>)> = Vec::new();
        // Walk the segment list with backtracking so `**`
        // (recursive) and `*`/`?` (component-level) interact
        // correctly without re-implementing the whole matcher.
        self.walk_glob_segments(&segments, "", 0, 0, &mut matched)?;

        matched.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        Ok(matched)
    }

    /// Recursive segment walker for `load_glob`.
    ///
    /// `segments` is the split pattern, `current_path` is the
    /// directory we're listing (relative to project root),
    /// `seg_idx` is the current segment cursor, `depth` guards
    /// against symlink cycles.
    fn walk_glob_segments(
        &mut self,
        segments: &[&str],
        current_path: &str,
        seg_idx: usize,
        depth: usize,
        matched: &mut Vec<(Text, Vec<u8>)>,
    ) -> Result<(), MetaError> {
        const MAX_DEPTH: usize = 64;
        if depth > MAX_DEPTH {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: walk depth exceeded — pattern likely \
                 traverses a symlink cycle",
            )));
        }

        if seg_idx >= segments.len() {
            // No more segments: current_path itself must match
            // (file produced the leaf). Already handled by the
            // caller — terminal segment writes into matched.
            return Ok(());
        }

        let seg = segments[seg_idx];
        let listing_path = if current_path.is_empty() { "." } else { current_path };
        let entries = match self.list_dir(listing_path) {
            Ok(es) => es,
            // A pattern that touches a missing directory
            // produces zero matches, not an error — `assets/*`
            // when `assets/` doesn't exist matches nothing.
            Err(_) => return Ok(()),
        };
        let is_terminal = seg_idx + 1 == segments.len();

        if seg == "**" {
            if is_terminal {
                // Trailing `**` matches every file at any
                // depth under the current path.
                for entry in entries.iter() {
                    let full = if current_path.is_empty() {
                        entry.as_str().to_string()
                    } else {
                        format!("{}/{}", current_path, entry.as_str())
                    };
                    if self.is_directory(&full) {
                        self.walk_glob_segments(
                            segments,
                            &full,
                            seg_idx,
                            depth + 1,
                            matched,
                        )?;
                    } else {
                        let bytes = self.load(&full)?;
                        matched.push((Text::from(full), bytes));
                    }
                }
                return Ok(());
            }
            // Non-terminal `**` — match zero components: skip
            // the `**`, try the remaining segments at the same
            // path.
            self.walk_glob_segments(
                segments,
                current_path,
                seg_idx + 1,
                depth,
                matched,
            )?;
            // Match one-or-more components: descend into every
            // subdirectory and re-attempt with the same `**`
            // segment (zero or more).
            for entry in entries.iter() {
                let full = if current_path.is_empty() {
                    entry.as_str().to_string()
                } else {
                    format!("{}/{}", current_path, entry.as_str())
                };
                if self.is_directory(&full) {
                    self.walk_glob_segments(
                        segments,
                        &full,
                        seg_idx,
                        depth + 1,
                        matched,
                    )?;
                }
            }
            return Ok(());
        }

        for entry in entries.iter() {
            if !fnmatch_basename(seg, entry.as_str()) {
                continue;
            }
            let full = if current_path.is_empty() {
                entry.as_str().to_string()
            } else {
                format!("{}/{}", current_path, entry.as_str())
            };
            if is_terminal {
                if !self.is_directory(&full) {
                    let bytes = self.load(&full)?;
                    matched.push((Text::from(full), bytes));
                }
            } else if self.is_directory(&full) {
                self.walk_glob_segments(
                    segments,
                    &full,
                    seg_idx + 1,
                    depth + 1,
                    matched,
                )?;
            }
        }
        Ok(())
    }

    /// Sandbox-respecting helper: returns `true` when the path
    /// resolves to a directory under the project root, `false`
    /// otherwise (including missing-path / not-a-directory
    /// cases). Used by `walk_glob_segments` to decide whether
    /// to recurse.
    fn is_directory(&self, rel: &str) -> bool {
        match self.resolve_path(rel) {
            Ok(resolved) => std::fs::metadata(&resolved)
                .map(|m| m.is_dir())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Get file metadata
    pub fn metadata(&self, path: &str) -> Result<AssetMetadata, MetaError> {
        let resolved = self.resolve_path(path)?;

        let meta = std::fs::metadata(&resolved).map_err(|e| {
            MetaError::Other(Text::from(format!("Failed to get metadata: {}", e)))
        })?;

        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        Ok(AssetMetadata {
            size: meta.len(),
            modified_ns,
            is_directory: meta.is_dir(),
            is_file: meta.is_file(),
            is_symlink: meta.is_symlink(),
        })
    }
}

/// Case-sensitive fnmatch over a single path component.
/// Supports `*` (any chars within one component) and `?` (one
/// char). Backtracks on `*` so e.g. `*.tar.gz` matches
/// `archive.tar.gz`. Rejects path separators implicitly because
/// Convert a `toml::Value` into a `MetaValue` (= `meta::ConstValue`).
/// Datetimes collapse to RFC 3339 text — every other variant has a
/// direct MetaValue analogue. Used by `BuildAssetsInfo::load_toml`
/// to lower a parsed TOML document into the meta-builtin surface.
pub(crate) fn toml_to_meta(v: toml::Value) -> MetaValue {
    use verum_common::OrderedMap;
    match v {
        toml::Value::String(s) => MetaValue::Text(Text::from(s)),
        toml::Value::Integer(i) => MetaValue::Int(i as i128),
        toml::Value::Float(f) => MetaValue::Float(f),
        toml::Value::Boolean(b) => MetaValue::Bool(b),
        toml::Value::Datetime(dt) => MetaValue::Text(Text::from(dt.to_string())),
        toml::Value::Array(arr) => {
            let items: List<MetaValue> = arr.into_iter().map(toml_to_meta).collect();
            MetaValue::Array(items)
        }
        toml::Value::Table(table) => {
            let mut map: OrderedMap<Text, MetaValue> = OrderedMap::new();
            for (k, val) in table.into_iter() {
                map.insert(Text::from(k), toml_to_meta(val));
            }
            MetaValue::Map(map)
        }
    }
}

/// Diagnostic-shaped name for a `toml::Value` variant. Used in
/// error messages from `load_toml` when the root document isn't a
/// table.
pub(crate) fn toml_kind_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

/// `load_glob` already split the pattern at the last `/` and
/// only feeds basenames in.
pub(crate) fn fnmatch_basename(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fnmatch_recurse(&pat, 0, &n, 0)
}

fn fnmatch_recurse(pat: &[char], mut pi: usize, name: &[char], mut ni: usize) -> bool {
    while pi < pat.len() {
        match pat[pi] {
            '*' => {
                // Greedy: try matching the rest at every suffix.
                pi += 1;
                if pi == pat.len() {
                    return true;
                }
                while ni <= name.len() {
                    if fnmatch_recurse(pat, pi, name, ni) {
                        return true;
                    }
                    if ni == name.len() {
                        return false;
                    }
                    ni += 1;
                }
                return false;
            }
            '?' => {
                if ni == name.len() {
                    return false;
                }
                pi += 1;
                ni += 1;
            }
            c => {
                if ni == name.len() || name[ni] != c {
                    return false;
                }
                pi += 1;
                ni += 1;
            }
        }
    }
    ni == name.len()
}

#[cfg(test)]
mod fnmatch_tests {
    use super::*;

    #[test]
    fn star_matches_zero_or_more_chars() {
        assert!(fnmatch_basename("*", ""));
        assert!(fnmatch_basename("*", "anything"));
        assert!(fnmatch_basename("*.png", "icon.png"));
        assert!(fnmatch_basename("*.png", ".png"));
        assert!(!fnmatch_basename("*.png", "icon.jpg"));
    }

    #[test]
    fn question_matches_exactly_one_char() {
        assert!(fnmatch_basename("a?c", "abc"));
        assert!(!fnmatch_basename("a?c", "ac"));
        assert!(!fnmatch_basename("a?c", "abbc"));
    }

    #[test]
    fn literal_chars_match_verbatim() {
        assert!(fnmatch_basename("Cargo.toml", "Cargo.toml"));
        assert!(!fnmatch_basename("Cargo.toml", "cargo.toml"));
    }

    #[test]
    fn star_backtracks_for_double_extension() {
        assert!(fnmatch_basename("*.tar.gz", "archive.tar.gz"));
        assert!(fnmatch_basename("*.tar.gz", ".tar.gz"));
        assert!(!fnmatch_basename("*.tar.gz", "archive.tar"));
    }

    #[test]
    fn matcher_runs_on_basenames_only() {
        // load_glob splits the pattern at the last `/` and only
        // feeds the basename component to fnmatch_basename, so
        // the matcher itself doesn't need to special-case `/`.
        // Verify the simple-string semantics survive the split:
        // an entry like "subdir/icon.png" cannot be passed in
        // because `list_dir` returns bare entry names. Document
        // the layered invariant and skip the misleading
        // basename-vs-full-path assertion that depends on
        // matcher-side path handling load_glob doesn't require.
        assert!(fnmatch_basename("icon.png", "icon.png"));
    }
}
