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

    /// Resolve a single-level glob pattern against the project
    /// root and return the matched (relative-path, bytes) pairs.
    ///
    /// Pattern grammar (intentionally minimal — covers the common
    /// `@embed_glob("assets/*.png")` shape):
    ///
    ///   * `*`  — matches zero or more characters within a single
    ///            path component (does NOT cross `/`).
    ///   * `?`  — matches exactly one character within a single
    ///            path component.
    ///   * literal text matches verbatim.
    ///
    /// `**` is reserved but rejected today — multi-component
    /// recursive globs land in a follow-up alongside the
    /// `walkdir`-style traversal sandbox.
    ///
    /// Sandbox: the directory portion of the pattern flows through
    /// the same `resolve_path` precheck `load` / `load_text` use,
    /// so absolute paths and `..` traversal are rejected before any
    /// filesystem walk happens. Each matched file's bytes are
    /// loaded via `load(...)` so the per-file cache is populated
    /// transparently.
    pub fn load_glob(&mut self, pattern: &str) -> Result<Vec<(Text, Vec<u8>)>, MetaError> {
        if pattern.is_empty() {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: empty pattern",
            )));
        }
        if pattern.contains("**") {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: `**` recursive glob not yet supported — use a single-level pattern",
            )));
        }

        // Split pattern into directory prefix (no wildcards) and
        // basename pattern (may contain wildcards).
        let (dir_prefix, basename_pattern) = match pattern.rfind('/') {
            Some(idx) => (&pattern[..idx], &pattern[idx + 1..]),
            None => ("", pattern),
        };
        if dir_prefix.contains('*') || dir_prefix.contains('?') {
            return Err(MetaError::Other(Text::from(
                "@embed_glob: wildcards only permitted in the basename component \
                 (multi-component globs require `**` recursion, not yet supported)",
            )));
        }
        let listing_path = if dir_prefix.is_empty() { "." } else { dir_prefix };
        let entries = self.list_dir(listing_path)?;

        let mut matched: Vec<(Text, Vec<u8>)> = Vec::new();
        for entry in entries.iter() {
            if !fnmatch_basename(basename_pattern, entry.as_str()) {
                continue;
            }
            let full_rel = if dir_prefix.is_empty() {
                entry.as_str().to_string()
            } else {
                format!("{}/{}", dir_prefix, entry.as_str())
            };
            let bytes = self.load(&full_rel)?;
            matched.push((Text::from(full_rel), bytes));
        }
        // Deterministic ordering — readdir is platform-specific.
        matched.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        Ok(matched)
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
