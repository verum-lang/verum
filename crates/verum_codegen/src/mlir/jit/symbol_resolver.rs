//! Industrial-grade Symbol Resolution for JIT compilation.
//!
//! Provides comprehensive symbol resolution for verum_std FFI functions
//! and other runtime dependencies with:
//!
//! - Dynamic library loading (cross-platform)
//! - Lazy symbol binding with caching
//! - Symbol validation and type checking
//! - Error recovery and fallback mechanisms
//! - Statistics and diagnostics
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    Symbol Resolution Pipeline                        │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//!   JIT-compiled code
//!         │
//!         ▼
//! ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
//! │  Symbol Lookup  │───▶│   Cache Check   │───▶│  Library Load   │
//! │                 │    │                 │    │  (lazy binding) │
//! └─────────────────┘    └─────────────────┘    └─────────────────┘
//!         │                      │                      │
//!         │               Cache hit              Library loaded
//!         │                      │                      │
//!         ▼                      ▼                      ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Symbol Address Cache                        │
//! │  HashMap<SymbolName, (Address, Metadata)>                       │
//! └─────────────────────────────────────────────────────────────────┘
//!         │
//!         ▼
//!   Native call
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{SymbolResolver, SymbolInfo};
//!
//! let mut resolver = SymbolResolver::new();
//!
//! // Load verum_std dynamically
//! resolver.load_library("libverum_std.so")?;
//!
//! // Resolve a symbol
//! let info = resolver.resolve("verum_std_list_i64_new")?;
//! println!("Symbol at: {:p}", info.address);
//! ```

use crate::mlir::error::{MlirError, Result};
use dashmap::DashMap;
use libloading::{Library, Symbol as LibSymbol};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use verum_common::Text;

// ============================================================================
// Symbol Metadata and Information
// ============================================================================

/// Metadata about a resolved symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolMetadata {
    /// Original symbol name.
    pub name: Text,

    /// Library this symbol came from.
    pub library: Text,

    /// Symbol category for organization.
    pub category: SymbolCategory,

    /// Parameter types (for validation).
    pub param_types: Vec<FfiType>,

    /// Return type (for validation).
    pub return_type: FfiType,

    /// Whether this symbol requires CBGR validation.
    pub requires_cbgr: bool,

    /// Documentation string (optional).
    pub doc: Option<Text>,
}

/// Categories of FFI symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolCategory {
    /// List operations (new, push, pop, get, etc.)
    ListOps,
    /// Map operations (new, insert, get, etc.)
    MapOps,
    /// Set operations (new, insert, contains, etc.)
    SetOps,
    /// Text/string operations
    TextOps,
    /// Maybe/Option operations
    MaybeOps,
    /// Result operations
    ResultOps,
    /// I/O operations (print, read, etc.)
    IoOps,
    /// Math functions
    MathOps,
    /// Memory management (alloc, free, etc.)
    MemoryOps,
    /// CBGR-specific operations
    CbgrOps,
    /// User-defined/external symbols
    External,
}

/// FFI type representation for validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FfiType {
    Void,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Usize,
    /// Raw pointer to type
    Ptr(Box<FfiType>),
    /// Const pointer to type
    ConstPtr(Box<FfiType>),
    /// Opaque type (e.g., CBGRTracked<T>)
    Opaque(Text),
    /// Function pointer
    FnPtr {
        params: Vec<FfiType>,
        ret: Box<FfiType>,
    },
}

/// Complete information about a resolved symbol.
#[derive(Debug)]
pub struct SymbolInfo {
    /// Symbol address (for calling).
    pub address: *mut (),

    /// Symbol metadata.
    pub metadata: SymbolMetadata,

    /// Resolution timestamp (for cache management).
    pub resolved_at: instant::Instant,
}

// SAFETY: SymbolInfo can be sent/shared across threads because:
// - `address` is a raw pointer to a symbol in a loaded shared library
// - The library remains loaded (and thus the symbol address valid) for the
//   lifetime of the SymbolResolver that owns the LoadedLibrary
// - SymbolInfo is read-only after construction (no mutation of the pointer)
unsafe impl Send for SymbolInfo {}
unsafe impl Sync for SymbolInfo {}

// ============================================================================
// Library Management
// ============================================================================

/// Wrapper for a loaded dynamic library.
struct LoadedLibrary {
    /// The actual library handle.
    library: Library,

    /// Path to the library.
    path: PathBuf,

    /// Symbols resolved from this library.
    symbols: Vec<Text>,
}

// SAFETY: LoadedLibrary can be sent/shared across threads because:
// - The `library` (libloading::Library) handle is safe to share; symbol lookups
//   are thread-safe on all supported platforms (dlsym is thread-safe per POSIX)
// - Libraries are never unloaded while the SymbolResolver is alive, so all
//   symbol pointers derived from them remain valid
// - `path` and `symbols` are plain data with no thread-safety concerns
unsafe impl Send for LoadedLibrary {}
unsafe impl Sync for LoadedLibrary {}

// ============================================================================
// Symbol Resolver Statistics
// ============================================================================

/// Statistics for symbol resolution.
#[derive(Debug, Default)]
pub struct SymbolResolverStats {
    /// Number of successful resolutions.
    pub resolutions: AtomicU64,

    /// Number of cache hits.
    pub cache_hits: AtomicU64,

    /// Number of cache misses.
    pub cache_misses: AtomicU64,

    /// Number of failed resolutions.
    pub failures: AtomicU64,

    /// Number of libraries loaded.
    pub libraries_loaded: AtomicU64,

    /// Total resolution time (microseconds).
    pub total_resolution_time_us: AtomicU64,
}

impl SymbolResolverStats {
    /// Create new statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get resolution success rate.
    pub fn success_rate(&self) -> f64 {
        let total = self.resolutions.load(Ordering::Relaxed)
            + self.failures.load(Ordering::Relaxed);
        if total == 0 {
            1.0
        } else {
            self.resolutions.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Get cache hit rate.
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits.load(Ordering::Relaxed)
            + self.cache_misses.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.cache_hits.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Get average resolution time in microseconds.
    pub fn avg_resolution_time_us(&self) -> f64 {
        let resolutions = self.resolutions.load(Ordering::Relaxed);
        if resolutions == 0 {
            0.0
        } else {
            self.total_resolution_time_us.load(Ordering::Relaxed) as f64
                / resolutions as f64
        }
    }
}

// ============================================================================
// Symbol Resolver
// ============================================================================

/// Industrial-grade symbol resolver for JIT.
///
/// Provides comprehensive symbol resolution with:
/// - Dynamic library loading
/// - Lazy binding and caching
/// - Thread-safe concurrent access
/// - Statistics and diagnostics
pub struct SymbolResolver {
    /// Loaded libraries.
    libraries: RwLock<Vec<LoadedLibrary>>,

    /// Symbol cache: name -> (address, metadata).
    cache: DashMap<Text, Arc<SymbolInfo>>,

    /// Pre-registered symbols (e.g., intrinsics).
    registered: DashMap<Text, *mut ()>,

    /// Symbol metadata registry.
    metadata: DashMap<Text, SymbolMetadata>,

    /// Resolution statistics.
    stats: Arc<SymbolResolverStats>,

    /// Standard library paths to search.
    search_paths: RwLock<Vec<PathBuf>>,

    /// Whether to enable verbose logging.
    verbose: bool,
}

impl SymbolResolver {
    /// Create a new symbol resolver.
    pub fn new() -> Self {
        let resolver = Self {
            libraries: RwLock::new(Vec::new()),
            cache: DashMap::new(),
            registered: DashMap::new(),
            metadata: DashMap::new(),
            stats: Arc::new(SymbolResolverStats::new()),
            search_paths: RwLock::new(Vec::new()),
            verbose: false,
        };

        // Register built-in metadata for verum_std symbols
        resolver.register_stdlib_metadata();

        // Add default search paths
        resolver.add_default_search_paths();

        resolver
    }

    /// Create resolver with verbose logging.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Add a library search path.
    pub fn add_search_path(&self, path: impl AsRef<Path>) {
        self.search_paths.write().push(path.as_ref().to_path_buf());
    }

    /// Add default platform-specific search paths.
    fn add_default_search_paths(&self) {
        let mut paths = self.search_paths.write();

        // Add target directory (for development)
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let target_debug = PathBuf::from(&manifest_dir)
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("target")
                .join("debug");
            let target_release = PathBuf::from(&manifest_dir)
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("target")
                .join("release");
            paths.push(target_debug);
            paths.push(target_release);
        }

        // Platform-specific paths
        #[cfg(target_os = "linux")]
        {
            paths.push(PathBuf::from("/usr/lib"));
            paths.push(PathBuf::from("/usr/local/lib"));
            paths.push(PathBuf::from("/lib"));
            if let Ok(ld_path) = std::env::var("LD_LIBRARY_PATH") {
                for p in ld_path.split(':') {
                    paths.push(PathBuf::from(p));
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            paths.push(PathBuf::from("/usr/local/lib"));
            paths.push(PathBuf::from("/opt/homebrew/lib"));
            if let Ok(dyld_path) = std::env::var("DYLD_LIBRARY_PATH") {
                for p in dyld_path.split(':') {
                    paths.push(PathBuf::from(p));
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            paths.push(PathBuf::from("C:\\Windows\\System32"));
            if let Ok(path) = std::env::var("PATH") {
                for p in path.split(';') {
                    paths.push(PathBuf::from(p));
                }
            }
        }
    }

    /// Load a dynamic library.
    pub fn load_library(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let start = instant::Instant::now();

        // Try to find the library
        let full_path = self.find_library(path)?;

        if self.verbose {
            tracing::info!("Loading library: {}", full_path.display());
        }

        // Load the library
        // SAFETY: We trust the library path and handle errors properly
        let library = unsafe { Library::new(&full_path) }.map_err(|e| {
            MlirError::LibraryLoadError {
                path: Text::from(full_path.to_string_lossy().to_string()),
                message: Text::from(e.to_string()),
            }
        })?;

        let loaded = LoadedLibrary {
            library,
            path: full_path.clone(),
            symbols: Vec::new(),
        };

        self.libraries.write().push(loaded);
        self.stats.libraries_loaded.fetch_add(1, Ordering::Relaxed);

        if self.verbose {
            let elapsed = start.elapsed();
            tracing::info!(
                "Library loaded in {:?}: {}",
                elapsed,
                full_path.display()
            );
        }

        Ok(())
    }

    /// Find library in search paths.
    fn find_library(&self, name: &Path) -> Result<PathBuf> {
        // If absolute path, use directly
        if name.is_absolute() {
            if name.exists() {
                return Ok(name.to_path_buf());
            }
            return Err(MlirError::LibraryNotFound {
                name: Text::from(name.to_string_lossy().to_string()),
            });
        }

        // Get platform-specific library name
        let lib_name = self.platform_library_name(name);

        // Search in paths
        let paths = self.search_paths.read();
        for search_path in paths.iter() {
            let full_path = search_path.join(&lib_name);
            if full_path.exists() {
                return Ok(full_path);
            }

            // Also try without platform prefix/suffix
            let full_path = search_path.join(name);
            if full_path.exists() {
                return Ok(full_path);
            }
        }

        Err(MlirError::LibraryNotFound {
            name: Text::from(name.to_string_lossy().to_string()),
        })
    }

    /// Get platform-specific library name.
    fn platform_library_name(&self, name: &Path) -> PathBuf {
        let name_str = name.to_string_lossy();

        #[cfg(target_os = "linux")]
        {
            if name_str.ends_with(".so") || name_str.contains(".so.") {
                name.to_path_buf()
            } else if name_str.starts_with("lib") {
                PathBuf::from(format!("{}.so", name_str))
            } else {
                PathBuf::from(format!("lib{}.so", name_str))
            }
        }

        #[cfg(target_os = "macos")]
        {
            if name_str.ends_with(".dylib") {
                name.to_path_buf()
            } else if name_str.starts_with("lib") {
                PathBuf::from(format!("{}.dylib", name_str))
            } else {
                PathBuf::from(format!("lib{}.dylib", name_str))
            }
        }

        #[cfg(target_os = "windows")]
        {
            if name_str.ends_with(".dll") {
                name.to_path_buf()
            } else {
                PathBuf::from(format!("{}.dll", name_str))
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            name.to_path_buf()
        }
    }

    /// Load the verum standard library.
    pub fn load_verum_std(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        const VERUM_STD_NAME: &str = "libverum_std.so";

        #[cfg(target_os = "macos")]
        const VERUM_STD_NAME: &str = "libverum_std.dylib";

        #[cfg(target_os = "windows")]
        const VERUM_STD_NAME: &str = "verum_std.dll";

        self.load_library(VERUM_STD_NAME)
    }

    /// Resolve a symbol by name.
    pub fn resolve(&self, name: &str) -> Result<Arc<SymbolInfo>> {
        let start = instant::Instant::now();
        let name_text = Text::from(name);

        // Check cache first
        if let Some(info) = self.cache.get(&name_text) {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(info.clone());
        }
        self.stats.cache_misses.fetch_add(1, Ordering::Relaxed);

        // Check registered symbols
        if let Some(addr) = self.registered.get(&name_text) {
            let metadata = self.metadata.get(&name_text)
                .map(|m| m.clone())
                .unwrap_or_else(|| SymbolMetadata {
                    name: name_text.clone(),
                    library: Text::from("<registered>"),
                    category: SymbolCategory::External,
                    param_types: vec![],
                    return_type: FfiType::Void,
                    requires_cbgr: false,
                    doc: None,
                });

            let info = Arc::new(SymbolInfo {
                address: *addr,
                metadata,
                resolved_at: instant::Instant::now(),
            });

            self.cache.insert(name_text, info.clone());
            self.stats.resolutions.fetch_add(1, Ordering::Relaxed);
            return Ok(info);
        }

        // Try to find in loaded libraries
        let c_name = CString::new(name).map_err(|_| MlirError::SymbolNotFound {
            name: name_text.clone(),
        })?;

        let libraries = self.libraries.read();
        for loaded in libraries.iter() {
            // SAFETY: We're looking up a symbol from a library we loaded
            let result: std::result::Result<LibSymbol<'_, *mut ()>, _> =
                unsafe { loaded.library.get(c_name.as_bytes_with_nul()) };

            if let Ok(symbol) = result {
                let addr = *symbol;

                // Get or create metadata
                let metadata = self.metadata.get(&name_text)
                    .map(|m| m.clone())
                    .unwrap_or_else(|| self.infer_metadata(name, &loaded.path));

                let info = Arc::new(SymbolInfo {
                    address: addr,
                    metadata,
                    resolved_at: instant::Instant::now(),
                });

                self.cache.insert(name_text, info.clone());
                self.stats.resolutions.fetch_add(1, Ordering::Relaxed);

                let elapsed = start.elapsed();
                self.stats.total_resolution_time_us.fetch_add(
                    elapsed.as_micros() as u64,
                    Ordering::Relaxed,
                );

                if self.verbose {
                    tracing::debug!("Resolved symbol '{}' at {:p} in {:?}", name, addr, elapsed);
                }

                return Ok(info);
            }
        }

        self.stats.failures.fetch_add(1, Ordering::Relaxed);
        Err(MlirError::SymbolNotFound { name: name_text })
    }

    /// Try to resolve a symbol, returning None if not found.
    pub fn try_resolve(&self, name: &str) -> Option<Arc<SymbolInfo>> {
        self.resolve(name).ok()
    }

    /// Register a symbol manually.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the lifetime of the resolver.
    pub unsafe fn register(&self, name: impl Into<Text>, ptr: *mut ()) {
        self.registered.insert(name.into(), ptr);
    }

    /// Register a symbol with metadata.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the lifetime of the resolver.
    pub unsafe fn register_with_metadata(
        &self,
        name: impl Into<Text>,
        ptr: *mut (),
        metadata: SymbolMetadata,
    ) {
        let name = name.into();
        self.registered.insert(name.clone(), ptr);
        self.metadata.insert(name, metadata);
    }

    /// Check if a symbol is available.
    pub fn contains(&self, name: &str) -> bool {
        let name_text = Text::from(name);
        self.cache.contains_key(&name_text)
            || self.registered.contains_key(&name_text)
            || self.resolve(name).is_ok()
    }

    /// Get all resolved symbol names.
    pub fn resolved_symbols(&self) -> Vec<Text> {
        self.cache.iter().map(|e| e.key().clone()).collect()
    }

    /// Get all registered symbol names.
    pub fn registered_symbols(&self) -> Vec<Text> {
        self.registered.iter().map(|e| e.key().clone()).collect()
    }

    /// Get statistics.
    pub fn stats(&self) -> &SymbolResolverStats {
        &self.stats
    }

    /// Clear the symbol cache (but keep registered symbols).
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get the number of loaded libraries.
    pub fn library_count(&self) -> usize {
        self.libraries.read().len()
    }

    /// Infer metadata from symbol name.
    fn infer_metadata(&self, name: &str, library: &Path) -> SymbolMetadata {
        let category = if name.contains("list_") {
            SymbolCategory::ListOps
        } else if name.contains("map_") {
            SymbolCategory::MapOps
        } else if name.contains("set_") {
            SymbolCategory::SetOps
        } else if name.contains("text_") {
            SymbolCategory::TextOps
        } else if name.contains("maybe_") {
            SymbolCategory::MaybeOps
        } else if name.contains("result_") {
            SymbolCategory::ResultOps
        } else if name.contains("print") || name.contains("read") {
            SymbolCategory::IoOps
        } else if name.contains("math_") {
            SymbolCategory::MathOps
        } else if name.contains("_free") || name.contains("_new") {
            SymbolCategory::MemoryOps
        } else if name.contains("cbgr_") {
            SymbolCategory::CbgrOps
        } else {
            SymbolCategory::External
        };

        SymbolMetadata {
            name: Text::from(name),
            library: Text::from(library.to_string_lossy().to_string()),
            category,
            param_types: vec![],
            return_type: FfiType::Void,
            requires_cbgr: name.contains("push")
                || name.contains("pop")
                || name.contains("insert")
                || name.contains("remove")
                || name.contains("get"),
            doc: None,
        }
    }

    /// Register metadata for all verum_std symbols.
    fn register_stdlib_metadata(&self) {
        // List<i64> operations
        self.register_list_metadata("i64", FfiType::I64);
        self.register_list_metadata("i32", FfiType::I32);
        self.register_list_metadata("u64", FfiType::U64);
        self.register_list_metadata("u32", FfiType::U32);
        self.register_list_metadata("f64", FfiType::F64);
        self.register_list_metadata("bool", FfiType::U8);
        self.register_list_metadata("text", FfiType::Opaque(Text::from("Text")));

        // Map operations
        self.register_map_metadata("i64", "i64", FfiType::I64, FfiType::I64);
        self.register_map_metadata(
            "text",
            "i64",
            FfiType::Opaque(Text::from("Text")),
            FfiType::I64,
        );
        self.register_map_metadata(
            "text",
            "text",
            FfiType::Opaque(Text::from("Text")),
            FfiType::Opaque(Text::from("Text")),
        );

        // Set operations
        self.register_set_metadata("i64", FfiType::I64);
        self.register_set_metadata("text", FfiType::Opaque(Text::from("Text")));

        // Text operations
        self.register_text_metadata();

        // Maybe operations
        self.register_maybe_metadata("i64", FfiType::I64);
        self.register_maybe_metadata("text", FfiType::Opaque(Text::from("Text")));

        // Result operations
        self.register_result_metadata();

        // I/O operations
        self.register_io_metadata();

        // Math operations
        self.register_math_metadata();
    }

    fn register_list_metadata(&self, type_name: &str, elem_type: FfiType) {
        let base = format!("verum_std_list_{}", type_name);
        let opaque = FfiType::Opaque(Text::from(format!("CBGRTracked<List<{}>>", type_name)));

        // new
        self.metadata.insert(
            Text::from(format!("{}_new", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_new", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::ListOps,
                param_types: vec![],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from(format!("Create new List<{}>", type_name))),
            },
        );

        // push (with CBGR parameters)
        self.metadata.insert(
            Text::from(format!("{}_push", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_push", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::ListOps,
                param_types: vec![
                    FfiType::Ptr(Box::new(opaque.clone())),
                    elem_type.clone(),
                    FfiType::U32, // expected_gen
                    FfiType::U32, // expected_epoch
                ],
                return_type: FfiType::U32, // error code
                requires_cbgr: true,
                doc: Some(Text::from(format!("Push value to List<{}>", type_name))),
            },
        );

        // pop
        self.metadata.insert(
            Text::from(format!("{}_pop", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_pop", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::ListOps,
                param_types: vec![
                    FfiType::Ptr(Box::new(opaque.clone())),
                    FfiType::U32,
                    FfiType::U32,
                    FfiType::Ptr(Box::new(elem_type.clone())), // out_value
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from(format!("Pop value from List<{}>", type_name))),
            },
        );

        // len
        self.metadata.insert(
            Text::from(format!("{}_len", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_len", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::ListOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::Usize,
                requires_cbgr: true,
                doc: Some(Text::from(format!("Get length of List<{}>", type_name))),
            },
        );

        // get
        self.metadata.insert(
            Text::from(format!("{}_get", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_get", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::ListOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    FfiType::Usize, // index
                    FfiType::U32,
                    FfiType::U32,
                    FfiType::Ptr(Box::new(elem_type.clone())), // out_value
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from(format!("Get element from List<{}>", type_name))),
            },
        );

        // free
        self.metadata.insert(
            Text::from(format!("{}_free", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_free", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MemoryOps,
                param_types: vec![FfiType::Ptr(Box::new(opaque.clone()))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from(format!("Free List<{}>", type_name))),
            },
        );

        // generation
        self.metadata.insert(
            Text::from(format!("{}_generation", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_generation", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::CbgrOps,
                param_types: vec![FfiType::ConstPtr(Box::new(opaque.clone()))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Get CBGR generation")),
            },
        );

        // epoch
        self.metadata.insert(
            Text::from(format!("{}_epoch", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_epoch", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::CbgrOps,
                param_types: vec![FfiType::ConstPtr(Box::new(opaque))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Get CBGR epoch")),
            },
        );
    }

    fn register_map_metadata(
        &self,
        key_type: &str,
        value_type: &str,
        key_ffi: FfiType,
        value_ffi: FfiType,
    ) {
        let base = format!("verum_std_map_{}_{}", key_type, value_type);
        let opaque = FfiType::Opaque(Text::from(format!(
            "CBGRTracked<Map<{}, {}>>",
            key_type, value_type
        )));

        // new
        self.metadata.insert(
            Text::from(format!("{}_new", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_new", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MapOps,
                param_types: vec![],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from(format!(
                    "Create new Map<{}, {}>",
                    key_type, value_type
                ))),
            },
        );

        // insert
        self.metadata.insert(
            Text::from(format!("{}_insert", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_insert", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MapOps,
                param_types: vec![
                    FfiType::Ptr(Box::new(opaque.clone())),
                    key_ffi.clone(),
                    value_ffi.clone(),
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from("Insert key-value pair")),
            },
        );

        // get
        self.metadata.insert(
            Text::from(format!("{}_get", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_get", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MapOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    key_ffi,
                    FfiType::U32,
                    FfiType::U32,
                    FfiType::Ptr(Box::new(value_ffi)),
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from("Get value by key")),
            },
        );

        // free
        self.metadata.insert(
            Text::from(format!("{}_free", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_free", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MemoryOps,
                param_types: vec![FfiType::Ptr(Box::new(opaque))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Free map")),
            },
        );
    }

    fn register_set_metadata(&self, type_name: &str, elem_type: FfiType) {
        let base = format!("verum_std_set_{}", type_name);
        let opaque = FfiType::Opaque(Text::from(format!("CBGRTracked<Set<{}>>", type_name)));

        // new
        self.metadata.insert(
            Text::from(format!("{}_new", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_new", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::SetOps,
                param_types: vec![],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from(format!("Create new Set<{}>", type_name))),
            },
        );

        // insert
        self.metadata.insert(
            Text::from(format!("{}_insert", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_insert", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::SetOps,
                param_types: vec![
                    FfiType::Ptr(Box::new(opaque.clone())),
                    elem_type.clone(),
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from("Insert element")),
            },
        );

        // contains
        self.metadata.insert(
            Text::from(format!("{}_contains", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_contains", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::SetOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    elem_type,
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::U8,
                requires_cbgr: true,
                doc: Some(Text::from("Check if element exists")),
            },
        );

        // free
        self.metadata.insert(
            Text::from(format!("{}_free", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_free", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MemoryOps,
                param_types: vec![FfiType::Ptr(Box::new(opaque))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Free set")),
            },
        );
    }

    fn register_text_metadata(&self) {
        let opaque = FfiType::Opaque(Text::from("CBGRTracked<Text>"));

        // from_str
        self.metadata.insert(
            Text::from("verum_std_text_from_str"),
            SymbolMetadata {
                name: Text::from("verum_std_text_from_str"),
                library: Text::from("stdlib"),
                category: SymbolCategory::TextOps,
                param_types: vec![FfiType::ConstPtr(Box::new(FfiType::U8)), FfiType::Usize],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from("Create Text from byte slice")),
            },
        );

        // len
        self.metadata.insert(
            Text::from("verum_std_text_len"),
            SymbolMetadata {
                name: Text::from("verum_std_text_len"),
                library: Text::from("stdlib"),
                category: SymbolCategory::TextOps,
                param_types: vec![FfiType::ConstPtr(Box::new(opaque.clone()))],
                return_type: FfiType::Usize,
                requires_cbgr: false,
                doc: Some(Text::from("Get text length")),
            },
        );

        // as_ptr
        self.metadata.insert(
            Text::from("verum_std_text_as_ptr"),
            SymbolMetadata {
                name: Text::from("verum_std_text_as_ptr"),
                library: Text::from("stdlib"),
                category: SymbolCategory::TextOps,
                param_types: vec![FfiType::ConstPtr(Box::new(opaque.clone()))],
                return_type: FfiType::ConstPtr(Box::new(FfiType::U8)),
                requires_cbgr: false,
                doc: Some(Text::from("Get pointer to text data")),
            },
        );

        // free
        self.metadata.insert(
            Text::from("verum_std_text_free"),
            SymbolMetadata {
                name: Text::from("verum_std_text_free"),
                library: Text::from("stdlib"),
                category: SymbolCategory::MemoryOps,
                param_types: vec![FfiType::Ptr(Box::new(opaque))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Free text")),
            },
        );
    }

    fn register_maybe_metadata(&self, type_name: &str, inner_type: FfiType) {
        let base = format!("verum_std_maybe_{}", type_name);
        let opaque = FfiType::Opaque(Text::from(format!("CBGRTracked<Maybe<{}>>", type_name)));

        // some
        self.metadata.insert(
            Text::from(format!("{}_some", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_some", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MaybeOps,
                param_types: vec![inner_type.clone()],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from("Create Maybe::Some")),
            },
        );

        // none
        self.metadata.insert(
            Text::from(format!("{}_none", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_none", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MaybeOps,
                param_types: vec![],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from("Create Maybe::None")),
            },
        );

        // is_some
        self.metadata.insert(
            Text::from(format!("{}_is_some", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_is_some", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MaybeOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::U8,
                requires_cbgr: true,
                doc: Some(Text::from("Check if Some")),
            },
        );

        // unwrap
        self.metadata.insert(
            Text::from(format!("{}_unwrap", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_unwrap", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MaybeOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque.clone())),
                    FfiType::U32,
                    FfiType::U32,
                    FfiType::Ptr(Box::new(inner_type)),
                ],
                return_type: FfiType::U32,
                requires_cbgr: true,
                doc: Some(Text::from("Unwrap Maybe value")),
            },
        );

        // free
        self.metadata.insert(
            Text::from(format!("{}_free", base)),
            SymbolMetadata {
                name: Text::from(format!("{}_free", base)),
                library: Text::from("stdlib"),
                category: SymbolCategory::MemoryOps,
                param_types: vec![FfiType::Ptr(Box::new(opaque))],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Free Maybe")),
            },
        );
    }

    fn register_result_metadata(&self) {
        // Result<i64, Text>
        let opaque = FfiType::Opaque(Text::from("CBGRTracked<Result<i64, Text>>"));

        self.metadata.insert(
            Text::from("verum_std_result_i64_ok"),
            SymbolMetadata {
                name: Text::from("verum_std_result_i64_ok"),
                library: Text::from("stdlib"),
                category: SymbolCategory::ResultOps,
                param_types: vec![FfiType::I64],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from("Create Result::Ok")),
            },
        );

        self.metadata.insert(
            Text::from("verum_std_result_i64_err"),
            SymbolMetadata {
                name: Text::from("verum_std_result_i64_err"),
                library: Text::from("stdlib"),
                category: SymbolCategory::ResultOps,
                param_types: vec![FfiType::ConstPtr(Box::new(FfiType::U8)), FfiType::Usize],
                return_type: FfiType::Ptr(Box::new(opaque.clone())),
                requires_cbgr: false,
                doc: Some(Text::from("Create Result::Err")),
            },
        );

        self.metadata.insert(
            Text::from("verum_std_result_i64_is_ok"),
            SymbolMetadata {
                name: Text::from("verum_std_result_i64_is_ok"),
                library: Text::from("stdlib"),
                category: SymbolCategory::ResultOps,
                param_types: vec![
                    FfiType::ConstPtr(Box::new(opaque)),
                    FfiType::U32,
                    FfiType::U32,
                ],
                return_type: FfiType::U8,
                requires_cbgr: true,
                doc: Some(Text::from("Check if Result is Ok")),
            },
        );
    }

    fn register_io_metadata(&self) {
        // print
        self.metadata.insert(
            Text::from("verum_std_print"),
            SymbolMetadata {
                name: Text::from("verum_std_print"),
                library: Text::from("stdlib"),
                category: SymbolCategory::IoOps,
                param_types: vec![FfiType::ConstPtr(Box::new(FfiType::U8)), FfiType::Usize],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Print to stdout")),
            },
        );

        // println
        self.metadata.insert(
            Text::from("verum_std_println"),
            SymbolMetadata {
                name: Text::from("verum_std_println"),
                library: Text::from("stdlib"),
                category: SymbolCategory::IoOps,
                param_types: vec![FfiType::ConstPtr(Box::new(FfiType::U8)), FfiType::Usize],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Print line to stdout")),
            },
        );

        // eprint
        self.metadata.insert(
            Text::from("verum_std_eprint"),
            SymbolMetadata {
                name: Text::from("verum_std_eprint"),
                library: Text::from("stdlib"),
                category: SymbolCategory::IoOps,
                param_types: vec![FfiType::ConstPtr(Box::new(FfiType::U8)), FfiType::Usize],
                return_type: FfiType::U32,
                requires_cbgr: false,
                doc: Some(Text::from("Print to stderr")),
            },
        );

        // read_line
        self.metadata.insert(
            Text::from("verum_std_read_line"),
            SymbolMetadata {
                name: Text::from("verum_std_read_line"),
                library: Text::from("stdlib"),
                category: SymbolCategory::IoOps,
                param_types: vec![],
                return_type: FfiType::Ptr(Box::new(FfiType::Opaque(Text::from(
                    "CBGRTracked<Text>",
                )))),
                requires_cbgr: false,
                doc: Some(Text::from("Read line from stdin")),
            },
        );
    }

    fn register_math_metadata(&self) {
        // abs_i64
        self.metadata.insert(
            Text::from("verum_std_math_abs_i64"),
            SymbolMetadata {
                name: Text::from("verum_std_math_abs_i64"),
                library: Text::from("stdlib"),
                category: SymbolCategory::MathOps,
                param_types: vec![FfiType::I64],
                return_type: FfiType::I64,
                requires_cbgr: false,
                doc: Some(Text::from("Absolute value (i64)")),
            },
        );

        // sqrt
        self.metadata.insert(
            Text::from("verum_std_math_sqrt"),
            SymbolMetadata {
                name: Text::from("verum_std_math_sqrt"),
                library: Text::from("stdlib"),
                category: SymbolCategory::MathOps,
                param_types: vec![FfiType::F64],
                return_type: FfiType::F64,
                requires_cbgr: false,
                doc: Some(Text::from("Square root")),
            },
        );

        // sin, cos, tan
        for name in &["sin", "cos", "tan"] {
            self.metadata.insert(
                Text::from(format!("verum_std_math_{}", name)),
                SymbolMetadata {
                    name: Text::from(format!("verum_std_math_{}", name)),
                    library: Text::from("stdlib"),
                    category: SymbolCategory::MathOps,
                    param_types: vec![FfiType::F64],
                    return_type: FfiType::F64,
                    requires_cbgr: false,
                    doc: Some(Text::from(format!("{} (radians)", name))),
                },
            );
        }

        // pow
        self.metadata.insert(
            Text::from("verum_std_math_pow"),
            SymbolMetadata {
                name: Text::from("verum_std_math_pow"),
                library: Text::from("stdlib"),
                category: SymbolCategory::MathOps,
                param_types: vec![FfiType::F64, FfiType::F64],
                return_type: FfiType::F64,
                requires_cbgr: false,
                doc: Some(Text::from("Power function")),
            },
        );

        // floor, ceil, round
        for name in &["floor", "ceil", "round"] {
            self.metadata.insert(
                Text::from(format!("verum_std_math_{}", name)),
                SymbolMetadata {
                    name: Text::from(format!("verum_std_math_{}", name)),
                    library: Text::from("stdlib"),
                    category: SymbolCategory::MathOps,
                    param_types: vec![FfiType::F64],
                    return_type: FfiType::F64,
                    requires_cbgr: false,
                    doc: Some(Text::from(format!("{} function", name))),
                },
            );
        }
    }

    /// Get all stdlib symbol names for registration with JIT.
    pub fn stdlib_symbols(&self) -> Vec<(Text, *mut ())> {
        // This returns symbols that have been resolved
        // The actual loading happens via load_verum_std()
        self.cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().address))
            .collect()
    }

    /// Pre-resolve all known stdlib symbols.
    pub fn preload_stdlib_symbols(&self) -> Result<usize> {
        let mut count = 0;

        // Get all registered metadata names and try to resolve them
        let names: Vec<Text> = self.metadata.iter().map(|e| e.key().clone()).collect();

        for name in names {
            if self.try_resolve(name.as_str()).is_some() {
                count += 1;
            }
        }

        Ok(count)
    }
}

impl Default for SymbolResolver {
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
    fn test_symbol_resolver_new() {
        let resolver = SymbolResolver::new();
        assert_eq!(resolver.library_count(), 0);
        assert!(resolver.resolved_symbols().is_empty());
    }

    #[test]
    fn test_symbol_registration() {
        let resolver = SymbolResolver::new();
        let dummy_ptr = 0x1000 as *mut ();

        // SAFETY: Using dummy pointer for test
        unsafe {
            resolver.register("test_symbol", dummy_ptr);
        }

        assert!(resolver.contains("test_symbol"));
        let info = resolver.resolve("test_symbol").unwrap();
        assert_eq!(info.address, dummy_ptr);
    }

    #[test]
    fn test_symbol_metadata() {
        let resolver = SymbolResolver::new();

        // Check that stdlib metadata is registered
        let names = resolver
            .metadata
            .iter()
            .map(|e| e.key().clone())
            .collect::<Vec<_>>();
        assert!(names.iter().any(|n| n.as_str() == "verum_std_list_i64_new"));
        assert!(names.iter().any(|n| n.as_str() == "verum_std_text_len"));
        assert!(names.iter().any(|n| n.as_str() == "verum_std_math_sqrt"));
    }

    #[test]
    fn test_platform_library_name() {
        let resolver = SymbolResolver::new();

        #[cfg(target_os = "linux")]
        {
            let name = resolver.platform_library_name(Path::new("verum_std"));
            assert!(name.to_string_lossy().ends_with(".so"));
        }

        #[cfg(target_os = "macos")]
        {
            let name = resolver.platform_library_name(Path::new("verum_std"));
            assert!(name.to_string_lossy().ends_with(".dylib"));
        }

        #[cfg(target_os = "windows")]
        {
            let name = resolver.platform_library_name(Path::new("verum_std"));
            assert!(name.to_string_lossy().ends_with(".dll"));
        }
    }

    #[test]
    fn test_statistics() {
        let resolver = SymbolResolver::new();
        let dummy_ptr = 0x1000 as *mut ();

        // SAFETY: Using dummy pointer for test
        unsafe {
            resolver.register("test", dummy_ptr);
        }

        // First resolution - cache miss
        let _ = resolver.resolve("test");
        assert_eq!(resolver.stats().cache_misses.load(Ordering::Relaxed), 1);

        // Second resolution - cache hit
        let _ = resolver.resolve("test");
        assert_eq!(resolver.stats().cache_hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_symbol_not_found() {
        let resolver = SymbolResolver::new();
        let result = resolver.resolve("nonexistent_symbol");
        assert!(result.is_err());
    }

    #[test]
    fn test_infer_metadata() {
        let resolver = SymbolResolver::new();
        let metadata = resolver.infer_metadata("verum_std_list_i64_push", Path::new("test.so"));

        assert_eq!(metadata.category, SymbolCategory::ListOps);
        assert!(metadata.requires_cbgr);
    }
}
