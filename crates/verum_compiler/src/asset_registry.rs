//! Asset Registry for Compile-Time Asset Loading
//!
//! Provides build-time asset embedding via the `BuildAssets` meta-context.
//!
//! # Asset Loading (Meta System)
//!
//! Asset loading is the ONLY exception to the meta-system's "no I/O" sandbox rule.
//! Meta functions with `using BuildAssets` context may load files at compile-time,
//! with strict safety guarantees: deterministic builds (same input files produce same
//! output), rebuild triggers on asset changes, path restrictions (relative to project
//! root only, no `..` traversal), and no network access. The build system tracks all
//! loaded assets for cache invalidation.
//!
//! # Security
//!
//! Asset loading is strictly controlled via the BuildAssets context:
//! - Only relative paths from project root allowed
//! - No path traversal (..)
//! - No network access
//! - Explicit `using BuildAssets` context required
//! - All loaded assets tracked for rebuild detection
//!
//! # Example
//!
//! ```verum
//! @tagged_literal("img")
//! meta fn image_literal(path: Text) -> EmbeddedImage using BuildAssets {
//!     let data = BuildAssets.load(path)?;
//!     EmbeddedImage { data, format: detect_format(&data) }
//! }
//!
//! const LOGO: EmbeddedImage = img#"assets/logo.png";
//! ```

use parking_lot::RwLock;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

use crate::hash::ContentHash as Blake3Hasher;

/// Content hash for caching (Blake3-based, 64-bit truncated for cache keys)
pub type ContentHash = u64;

/// Registry for compile-time asset loading
///
/// Manages:
/// - Loading assets from the project directory
/// - Caching loaded assets by content hash
/// - Tracking assets for rebuild detection
/// - Validating paths for security
pub struct AssetRegistry {
    /// Project root directory
    project_root: PathBuf,
    /// Tracked assets for rebuild detection
    tracked_assets: Arc<RwLock<HashSet<PathBuf>>>,
    /// Asset metadata (path -> metadata)
    asset_metadata: Arc<RwLock<std::collections::HashMap<PathBuf, AssetMetadata>>>,
    /// L1 cache: In-memory cache (content hash -> data)
    memory_cache: Arc<RwLock<std::collections::HashMap<ContentHash, Vec<u8>>>>,
    /// Configuration
    config: AssetConfig,
}

/// Asset metadata for tracking
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    /// Original path relative to project root
    pub path: PathBuf,
    /// Size in bytes
    pub size: usize,
    /// Content hash for cache invalidation
    pub content_hash: ContentHash,
    /// Detected format
    pub format: AssetFormat,
    /// Last modified time
    pub last_modified: SystemTime,
    /// Whether embedded in binary
    pub embedded: bool,
}

/// Asset format detection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFormat {
    /// PNG image
    Png,
    /// JPEG image
    Jpeg,
    /// WebP image
    WebP,
    /// SVG image
    Svg,
    /// GIF image
    Gif,
    /// TrueType font
    Ttf,
    /// OpenType font
    Otf,
    /// WOFF2 font
    Woff2,
    /// JSON data
    Json,
    /// TOML data
    Toml,
    /// YAML data
    Yaml,
    /// CSV data
    Csv,
    /// Plain text
    Text,
    /// Unknown binary
    Binary,
}

impl AssetFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "png" => AssetFormat::Png,
            "jpg" | "jpeg" => AssetFormat::Jpeg,
            "webp" => AssetFormat::WebP,
            "svg" => AssetFormat::Svg,
            "gif" => AssetFormat::Gif,
            "ttf" => AssetFormat::Ttf,
            "otf" => AssetFormat::Otf,
            "woff2" => AssetFormat::Woff2,
            "json" => AssetFormat::Json,
            "toml" => AssetFormat::Toml,
            "yaml" | "yml" => AssetFormat::Yaml,
            "csv" => AssetFormat::Csv,
            "txt" | "md" | "rs" | "v" => AssetFormat::Text,
            _ => AssetFormat::Binary,
        }
    }

    /// Detect format from magic bytes
    pub fn from_magic_bytes(data: &[u8]) -> Self {
        if data.len() < 4 {
            return AssetFormat::Binary;
        }

        // PNG: 89 50 4E 47
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            return AssetFormat::Png;
        }

        // JPEG: FF D8 FF
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return AssetFormat::Jpeg;
        }

        // GIF: GIF87a or GIF89a
        if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            return AssetFormat::Gif;
        }

        // WebP: RIFF....WEBP
        if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
            return AssetFormat::WebP;
        }

        // SVG: <?xml or <svg
        if data.starts_with(b"<?xml") || data.starts_with(b"<svg") {
            return AssetFormat::Svg;
        }

        // WOFF2: wOF2
        if data.starts_with(b"wOF2") {
            return AssetFormat::Woff2;
        }

        // TrueType: 00 01 00 00
        if data.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
            return AssetFormat::Ttf;
        }

        // OpenType: OTTO
        if data.starts_with(b"OTTO") {
            return AssetFormat::Otf;
        }

        // JSON: starts with { or [
        if data.starts_with(b"{") || data.starts_with(b"[") {
            return AssetFormat::Json;
        }

        // Try to detect text
        if data.iter().take(1024).all(|&b| b.is_ascii() || b > 127) {
            return AssetFormat::Text;
        }

        AssetFormat::Binary
    }
}

/// Asset loading configuration
#[derive(Debug, Clone)]
pub struct AssetConfig {
    /// Maximum image size in bytes (default: 50MB)
    pub max_image_size: usize,
    /// Maximum font size in bytes (default: 100MB)
    pub max_font_size: usize,
    /// Maximum data file size in bytes (default: 10MB)
    pub max_data_size: usize,
    /// Maximum text file size in bytes (default: 50MB)
    pub max_text_size: usize,
    /// Maximum binary file size in bytes (default: 100MB)
    pub max_binary_size: usize,
    /// Allowed paths (empty = all paths allowed)
    pub allowed_paths: Vec<PathBuf>,
    /// Denied paths
    pub denied_paths: Vec<PathBuf>,
    /// Enable strict mode (only allowed_paths can be loaded)
    pub strict_mode: bool,
    /// Maximum number of items in memory cache (default: 1000)
    /// MEMORY SAFETY: Prevents unbounded cache growth in long-running sessions
    pub max_cache_items: usize,
    /// Maximum total bytes in memory cache (default: 256MB)
    /// MEMORY SAFETY: Limits total memory consumption by cache
    pub max_cache_bytes: usize,
}

impl Default for AssetConfig {
    fn default() -> Self {
        Self {
            max_image_size: 50 * 1024 * 1024,   // 50MB
            max_font_size: 100 * 1024 * 1024,   // 100MB
            max_data_size: 10 * 1024 * 1024,    // 10MB
            max_text_size: 50 * 1024 * 1024,    // 50MB
            max_binary_size: 100 * 1024 * 1024, // 100MB
            allowed_paths: vec![
                PathBuf::from("assets"),
                PathBuf::from("static"),
                PathBuf::from("resources"),
                PathBuf::from("config"),
            ],
            denied_paths: vec![
                PathBuf::from(".git"),
                PathBuf::from(".env"),
                PathBuf::from("secrets"),
                PathBuf::from("private"),
            ],
            strict_mode: false,
            // MEMORY SAFETY: Cache limits prevent unbounded memory growth
            max_cache_items: 1000,              // Maximum 1000 cached items
            max_cache_bytes: 256 * 1024 * 1024, // Maximum 256MB total cache
        }
    }
}

impl AssetRegistry {
    /// Create a new asset registry
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            tracked_assets: Arc::new(RwLock::new(HashSet::new())),
            asset_metadata: Arc::new(RwLock::new(std::collections::HashMap::new())),
            memory_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            config: AssetConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(project_root: PathBuf, config: AssetConfig) -> Self {
        Self {
            project_root,
            tracked_assets: Arc::new(RwLock::new(HashSet::new())),
            asset_metadata: Arc::new(RwLock::new(std::collections::HashMap::new())),
            memory_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            config,
        }
    }

    /// Load an asset at compile-time
    ///
    /// # Security
    /// - Validates path is relative
    /// - Checks for path traversal
    /// - Validates against allowed/denied paths
    /// - Checks size limits
    ///
    /// # Caching
    /// Uses content-hash based caching for efficiency
    pub fn load_asset(&self, path: &str) -> Result<Vec<u8>, Diagnostic> {
        let path = PathBuf::from(path);

        // Security validation
        self.validate_path(&path)?;

        // Resolve to full path
        let full_path = self.project_root.join(&path);

        if !full_path.exists() {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!("Asset not found: {}", path.display()))
                .help(format!(
                    "Ensure the file exists at: {}",
                    full_path.display()
                ))
                .build());
        }

        // Check if it's a file (not directory)
        if !full_path.is_file() {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!("Asset path is not a file: {}", path.display()))
                .build());
        }

        // Read the file
        let content = std::fs::read(&full_path).map_err(|e| {
            DiagnosticBuilder::new(Severity::Error)
                .message(format!("Failed to read asset: {}", e))
                .build()
        })?;

        // Compute content hash
        let hash = self.compute_hash(&content);

        // Check cache
        {
            let cache = self.memory_cache.read();
            if let Some(cached) = cache.get(&hash) {
                return Ok(cached.clone());
            }
        }

        // Validate size
        self.validate_size(&content, &path)?;

        // Detect format
        let format = AssetFormat::from_magic_bytes(&content);

        // Get modification time
        let last_modified = std::fs::metadata(&full_path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now());

        // Track the asset
        self.track_asset(&path, &content, hash, format, last_modified)?;

        // Cache the content with size limits
        self.cache_with_eviction(hash, content.clone());

        Ok(content)
    }

    /// Add content to cache with eviction if limits are exceeded.
    ///
    /// MEMORY SAFETY: This method ensures the cache never exceeds configured
    /// limits by evicting oldest entries when necessary. This prevents
    /// unbounded memory growth in long-running compilation sessions or LSP.
    fn cache_with_eviction(&self, hash: ContentHash, content: Vec<u8>) {
        let mut cache = self.memory_cache.write();

        // Calculate current cache size
        let current_bytes: usize = cache.values().map(|v| v.len()).sum();
        let new_item_bytes = content.len();

        // Check if we need to evict items
        let would_exceed_bytes = current_bytes + new_item_bytes > self.config.max_cache_bytes;
        let would_exceed_items = cache.len() >= self.config.max_cache_items;

        if would_exceed_bytes || would_exceed_items {
            // CRITICAL FIX: Evict entries to make room
            // Since HashMap doesn't maintain insertion order, we evict based on
            // size (largest first) to maximize freed space.
            // For a proper LRU, we'd need a different data structure.

            // Calculate how much space we need
            let bytes_needed = if would_exceed_bytes {
                (current_bytes + new_item_bytes).saturating_sub(self.config.max_cache_bytes) + 1
            } else {
                0
            };

            // Collect entries sorted by size (largest first for efficient eviction)
            let mut entries: Vec<_> = cache.iter().map(|(k, v)| (*k, v.len())).collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.1));

            let mut freed_bytes = 0;
            let mut items_to_remove = Vec::new();

            for (key, size) in entries {
                if freed_bytes >= bytes_needed
                    && cache.len() - items_to_remove.len() < self.config.max_cache_items
                {
                    break;
                }
                items_to_remove.push(key);
                freed_bytes += size;
            }

            // Remove evicted entries
            for key in items_to_remove {
                cache.remove(&key);
            }
        }

        // Insert new content
        cache.insert(hash, content);
    }

    /// Validate asset path for security
    fn validate_path(&self, path: &Path) -> Result<(), Diagnostic> {
        // Must be relative
        if path.is_absolute() {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message("Absolute paths not allowed in asset loading")
                .help("Use a path relative to the project root")
                .build());
        }

        // Check for path traversal
        let path_str = path.to_string_lossy();
        if path_str.contains("..") {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message("Path traversal (..) not allowed in asset loading")
                .help("Asset paths must stay within the project directory")
                .build());
        }

        // Check for network paths
        if path_str.starts_with("http://")
            || path_str.starts_with("https://")
            || path_str.starts_with("ftp://")
        {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message("Network asset loading not allowed")
                .help("Assets must be local files")
                .build());
        }

        // Check denied paths
        for denied in &self.config.denied_paths {
            if path.starts_with(denied) {
                return Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!("Access to {} is denied", denied.display()))
                    .help("This path is in the denied list")
                    .build());
            }
        }

        // Check strict mode
        if self.config.strict_mode && !self.config.allowed_paths.is_empty() {
            let allowed = self
                .config
                .allowed_paths
                .iter()
                .any(|allowed| path.starts_with(allowed));
            if !allowed {
                return Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!(
                        "Asset path '{}' not in allowed paths",
                        path.display()
                    ))
                    .help(format!(
                        "Allowed paths: {}",
                        self.config
                            .allowed_paths
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .build());
            }
        }

        Ok(())
    }

    /// Validate asset size
    fn validate_size(&self, content: &[u8], path: &Path) -> Result<(), Diagnostic> {
        let format = if let Some(ext) = path.extension() {
            AssetFormat::from_extension(&ext.to_string_lossy())
        } else {
            AssetFormat::from_magic_bytes(content)
        };

        let max_size = match format {
            AssetFormat::Png
            | AssetFormat::Jpeg
            | AssetFormat::WebP
            | AssetFormat::Svg
            | AssetFormat::Gif => self.config.max_image_size,
            AssetFormat::Ttf | AssetFormat::Otf | AssetFormat::Woff2 => self.config.max_font_size,
            AssetFormat::Json | AssetFormat::Toml | AssetFormat::Yaml | AssetFormat::Csv => {
                self.config.max_data_size
            }
            AssetFormat::Text => self.config.max_text_size,
            AssetFormat::Binary => self.config.max_binary_size,
        };

        if content.len() > max_size {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Asset too large: {} bytes (max: {} bytes for {:?})",
                    content.len(),
                    max_size,
                    format
                ))
                .help("Reduce file size or increase limit in verum.toml [meta.assets.limits]")
                .build());
        }

        Ok(())
    }

    /// Compute content hash using Blake3 (truncated to u64 for cache keys)
    ///
    /// Blake3 provides:
    /// - Cryptographic security guarantees
    /// - 3-10x faster than SHA-256
    /// - Consistent with compiler's unified hash infrastructure
    fn compute_hash(&self, content: &[u8]) -> ContentHash {
        let mut hasher = Blake3Hasher::new();
        hasher.update(content);
        hasher.finalize().to_u64()
    }

    /// Track an asset for rebuild detection
    fn track_asset(
        &self,
        path: &Path,
        content: &[u8],
        hash: ContentHash,
        format: AssetFormat,
        last_modified: SystemTime,
    ) -> Result<(), Diagnostic> {
        let metadata = AssetMetadata {
            path: path.to_path_buf(),
            size: content.len(),
            content_hash: hash,
            format,
            last_modified,
            embedded: false,
        };

        {
            let mut tracked = self.tracked_assets.write();
            tracked.insert(path.to_path_buf());
        }

        {
            let mut meta = self.asset_metadata.write();
            meta.insert(path.to_path_buf(), metadata);
        }

        Ok(())
    }

    /// Check if any tracked assets have changed
    pub fn check_for_changes(&self) -> Result<bool, Diagnostic> {
        let metadata = self.asset_metadata.read();

        for (path, meta) in metadata.iter() {
            let full_path = self.project_root.join(path);

            // Check if file still exists
            if !full_path.exists() {
                return Ok(true); // File deleted
            }

            // Check modification time
            if let Ok(current_meta) = std::fs::metadata(&full_path) {
                if let Ok(current_mtime) = current_meta.modified() {
                    if current_mtime > meta.last_modified {
                        return Ok(true); // File modified
                    }
                }
            }
        }

        Ok(false)
    }

    /// Get list of tracked assets
    pub fn tracked_assets(&self) -> Vec<PathBuf> {
        let tracked = self.tracked_assets.read();
        tracked.iter().cloned().collect()
    }

    /// Get asset metadata
    pub fn get_metadata(&self, path: &Path) -> Option<AssetMetadata> {
        let metadata = self.asset_metadata.read();
        metadata.get(path).cloned()
    }

    /// Clear cache (for testing)
    pub fn clear_cache(&self) {
        let mut cache = self.memory_cache.write();
        cache.clear();
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        let cache = self.memory_cache.read();
        let tracked = self.tracked_assets.read();

        CacheStats {
            cached_items: cache.len(),
            tracked_assets: tracked.len(),
            total_cached_bytes: cache.values().map(|v| v.len()).sum(),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of items in cache
    pub cached_items: usize,
    /// Number of tracked assets
    pub tracked_assets: usize,
    /// Total bytes cached
    pub total_cached_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_registry() -> (AssetRegistry, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let registry = AssetRegistry::new(temp_dir.path().to_path_buf());
        (registry, temp_dir)
    }

    #[test]
    fn test_asset_format_detection_png() {
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(AssetFormat::from_magic_bytes(&png_header), AssetFormat::Png);
    }

    #[test]
    fn test_asset_format_detection_jpeg() {
        let jpeg_header = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(
            AssetFormat::from_magic_bytes(&jpeg_header),
            AssetFormat::Jpeg
        );
    }

    #[test]
    fn test_asset_format_from_extension() {
        assert_eq!(AssetFormat::from_extension("png"), AssetFormat::Png);
        assert_eq!(AssetFormat::from_extension("jpg"), AssetFormat::Jpeg);
        assert_eq!(AssetFormat::from_extension("json"), AssetFormat::Json);
        assert_eq!(AssetFormat::from_extension("unknown"), AssetFormat::Binary);
    }

    #[test]
    fn test_load_asset_success() {
        let (registry, temp_dir) = create_test_registry();

        // Create test file
        let assets_dir = temp_dir.path().join("assets");
        std::fs::create_dir(&assets_dir).unwrap();

        let test_file = assets_dir.join("test.txt");
        std::fs::write(&test_file, b"Hello, World!").unwrap();

        // Load the asset
        let result = registry.load_asset("assets/test.txt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"Hello, World!");

        // Check it's tracked
        assert_eq!(registry.tracked_assets().len(), 1);
    }

    #[test]
    fn test_load_asset_not_found() {
        let (registry, _temp_dir) = create_test_registry();

        let result = registry.load_asset("nonexistent.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_path_traversal_blocked() {
        let (registry, _temp_dir) = create_test_registry();

        let result = registry.load_asset("../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_absolute_path_blocked() {
        let (registry, _temp_dir) = create_test_registry();

        let result = registry.load_asset("/etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_network_path_blocked() {
        let (registry, _temp_dir) = create_test_registry();

        let result = registry.load_asset("https://example.com/file.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_caching() {
        let (registry, temp_dir) = create_test_registry();

        // Create test file
        let test_file = temp_dir.path().join("cache_test.txt");
        std::fs::write(&test_file, b"Cache test").unwrap();

        // Load twice
        let result1 = registry.load_asset("cache_test.txt");
        let result2 = registry.load_asset("cache_test.txt");

        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert_eq!(result1.unwrap(), result2.unwrap());

        // Check cache stats
        let stats = registry.cache_stats();
        assert_eq!(stats.cached_items, 1);
    }

    #[test]
    fn test_size_limit() {
        let (registry, temp_dir) = create_test_registry();

        // Create a file larger than text limit
        let test_file = temp_dir.path().join("large.json");
        let large_content = vec![b'{'; 20 * 1024 * 1024]; // 20MB
        std::fs::write(&test_file, &large_content).unwrap();

        // Should fail due to size (json default limit is 10MB)
        let result = registry.load_asset("large.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_denied_path() {
        let (registry, temp_dir) = create_test_registry();

        // Create .git directory with file
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), b"test").unwrap();

        // Should be denied
        let result = registry.load_asset(".git/config");
        assert!(result.is_err());
    }
}
