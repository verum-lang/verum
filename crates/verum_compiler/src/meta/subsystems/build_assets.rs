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
