//! Phase 0: stdlib Compilation & Preparation
//!
//! This phase compiles the Verum standard library (verum_std) from Rust source
//! to static library and LLVM bitcode, generating FFI exports and symbol registries
//! for consumption by all execution tiers (interpreter, JIT, AOT).
//!
//! ## Key Outputs
//!
//! - `libverum_std.a` - Static library for AOT linking
//! - `libverum_std.bc` - LLVM bitcode for LTO optimization
//! - `registry.rs` - Symbol mappings (Verum names → Rust implementations)
//! - FFI exports - C-compatible function wrappers
//! - Monomorphization cache - Pre-instantiated generic types
//!
//! ## Caching Strategy
//!
//! Phase 0 runs once per build and caches outputs. Subsequent compilations
//! check if verum_std source has changed; if not, cached artifacts are reused.
//!
//! ## Performance Target
//!
//! - Initial compilation: ~1-5 seconds
//! - Cached reuse: ~10-50 milliseconds
//!
//! Phase 0: stdlib preparation. Compiles verum_std to static library,
//! generates C-compatible FFI exports, builds symbol registry,
//! prepares LLVM bitcode for LTO, caches monomorphized generics.

use anyhow::{Context as AnyhowContext, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use tracing::{debug, info, warn};
use verum_common::{List, Map, Text};
use verum_types::computational_properties::{ComputationalProperty, PropertySet};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};
use verum_diagnostics::Diagnostic;

// ============================================================================
// Core Data Structures
// ============================================================================

/// Artifacts produced by Phase 0 stdlib compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdlibArtifacts {
    /// Path to static library (libverum_std.a)
    pub static_library: PathBuf,

    /// Path to LLVM bitcode (libverum_std.bc)
    pub bitcode_library: PathBuf,

    /// FFI export definitions (C header + symbols)
    pub ffi_exports: FFIExports,

    /// Symbol registry (Verum names → C symbols)
    pub registry: StdlibRegistry,

    /// Pre-instantiated generic types
    pub monomorphization_cache: MonomorphizationCache,

    /// Build timestamp
    pub build_timestamp: SystemTime,

    /// Hash of source files (for invalidation)
    pub source_hash: Text,
}

/// FFI exports generated for C-compatible interop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FFIExports {
    /// C header file content
    pub header_content: Text,

    /// Map of Verum function → C symbol name
    pub symbol_mappings: Map<Text, Text>,
}

/// Registry mapping Verum function paths to implementation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdlibRegistry {
    /// All registered functions
    pub functions: Map<Text, FunctionDescriptor>,
}

/// Descriptor for a single stdlib function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDescriptor {
    /// Verum path (e.g., "List.push")
    pub verum_path: Text,

    /// Mangled C symbol (e.g., "verum_std_list_i64_push")
    pub c_symbol: Text,

    /// Function signature (for type checking)
    pub signature: FunctionSignature,

    /// Computational properties (Pure, IO, Async, Fallible, Mutates, etc.)
    pub properties: PropertySet,

    /// Context requirements (e.g., ["Database", "Logger"])
    pub context_requirements: List<Text>,

    /// Direct implementation for interpreter (Tier 0)
    pub tier0_impl: Option<Text>,

    /// JIT address (Tier 1-2, populated at runtime)
    pub jit_address: Option<usize>,

    /// Intrinsic ID for inlining optimization
    pub intrinsic_id: Option<IntrinsicId>,

    /// Whether this function is unsafe
    pub is_unsafe: bool,

    /// Documentation string (extracted from source)
    pub doc: Option<Text>,
}

/// Function signature representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    /// Parameter types
    pub params: List<Text>,

    /// Return type
    pub return_type: Text,

    /// Type parameters (for generics)
    pub type_params: List<Text>,
}

/// Intrinsic function identifier for optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntrinsicId {
    // Collection operations
    ListNew,
    ListPush,
    ListPop,
    ListGet,
    ListLen,
    MapNew,
    MapInsert,
    MapGet,
    SetNew,
    SetInsert,

    // Text operations
    TextNew,
    TextLen,
    TextConcat,

    // Math operations
    MathSqrt,
    MathPow,
    MathSin,
    MathCos,

    // Memory operations
    HeapAlloc,
    HeapDealloc,
    SharedNew,
    SharedClone,
}

/// Cache of pre-instantiated generic types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonomorphizationCache {
    /// Pre-instantiated type combinations
    pub instantiations: List<MonomorphizedFunction>,
}

/// A single monomorphized function instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonomorphizedFunction {
    /// Generic function path
    pub generic_path: Text,

    /// Concrete type arguments
    pub type_args: List<Text>,

    /// Mangled symbol name
    pub symbol: Text,
}

/// Parsed FFI function information extracted from source
#[derive(Debug, Clone)]
pub struct ParsedFFIFunction {
    /// The C symbol name (e.g., "verum_std_list_i64_push")
    pub symbol: String,
    /// Parameter types as strings
    pub params: Vec<String>,
    /// Return type as string
    pub return_type: String,
    /// Whether the function is marked unsafe
    pub is_unsafe: bool,
    /// Documentation comment if present
    pub doc: Option<String>,
}

// ============================================================================
// FFI Bridge Tables (data-driven stdlib binding)
// ============================================================================
//
// These tables describe the FFI boundary between the Verum stdlib (implemented
// in Rust as `verum_std`) and user code. They are NOT semantic assumptions about
// stdlib types — they are the *wiring* that lets generated code call into the
// Rust-side implementations. If `verum_std` adds a new monomorphization, add it
// here. If a new primitive is added to the language, add it here.

/// FFI-visible primitive type. Drives C type names, mangling, and instantiation.
struct PrimitiveFfi {
    /// Verum type name (e.g., "i64", "Text")
    verum: &'static str,
    /// C type spelling for the header (e.g., "int64_t", "void*")
    c_type: &'static str,
    /// Mangled fragment used in symbol names (lowercased `verum` by default)
    mangled: &'static str,
}

/// Primitives that can be monomorphized into stdlib generics at the FFI layer.
const FFI_PRIMITIVES: &[PrimitiveFfi] = &[
    PrimitiveFfi { verum: "i32",  c_type: "int32_t", mangled: "i32"  },
    PrimitiveFfi { verum: "i64",  c_type: "int64_t", mangled: "i64"  },
    PrimitiveFfi { verum: "f64",  c_type: "double",  mangled: "f64"  },
    PrimitiveFfi { verum: "bool", c_type: "uint8_t", mangled: "bool" },
    PrimitiveFfi { verum: "Text", c_type: "void*",   mangled: "text" },
];

/// Pre-monomorphized Map<K,V> key-value pairs exposed at the FFI layer.
const FFI_MAP_PAIRS: &[(&str, &str)] = &[
    ("Text", "i64"),
    ("Text", "Text"),
    ("i64", "Text"),
];

/// Primitives exposed for Maybe<T> monomorphizations at the FFI layer.
/// (Subset of FFI_PRIMITIVES — Maybe<bool> is not currently materialized.)
const FFI_MAYBE_PRIMITIVES: &[&str] = &["i32", "i64", "f64", "Text"];

/// Stdlib container types and their expected generic parameter names.
/// Used to synthesize FunctionSignature type_params for FFI descriptors.
/// Driven lookup replaces per-type match arms.
const GENERIC_TYPE_PARAMS: &[(&str, &[&str])] = &[
    ("List",    &["T"]),
    ("Set",     &["T"]),
    ("Deque",   &["T"]),
    ("Maybe",   &["T"]),
    ("Option",  &["T"]),
    ("Map",     &["K", "V"]),
    ("HashMap", &["K", "V"]),
    ("Result",  &["T", "E"]),
];

/// Path fragments that identify a stdlib function as a compiler intrinsic.
/// Format: `(type_segment, method_segment, intrinsic_id)` — matched via
/// `path.contains(...)` on the full Verum path (e.g. `"List.<i64>.push"`).
const INTRINSIC_PATTERNS: &[(&str, &str, IntrinsicId)] = &[
    ("List", ".new",  IntrinsicId::ListNew),
    ("List", ".push", IntrinsicId::ListPush),
    ("List", ".pop",  IntrinsicId::ListPop),
    ("List", ".get",  IntrinsicId::ListGet),
    ("List", ".len",  IntrinsicId::ListLen),
    ("Map",  ".new",    IntrinsicId::MapNew),
    ("Map",  ".insert", IntrinsicId::MapInsert),
    ("Map",  ".get",    IntrinsicId::MapGet),
    ("Set",  ".new",    IntrinsicId::SetNew),
    ("Set",  ".insert", IntrinsicId::SetInsert),
    ("Text", ".new",    IntrinsicId::TextNew),
    ("Text", ".len",    IntrinsicId::TextLen),
    ("Text", ".concat", IntrinsicId::TextConcat),
];

/// Look up a primitive's FFI metadata. Returns `None` for unknown types, which
/// callers treat as an opaque pointer (`void*`).
fn lookup_primitive(verum: &str) -> Option<&'static PrimitiveFfi> {
    FFI_PRIMITIVES.iter().find(|p| p.verum == verum)
}

// ============================================================================
// Phase 0 Implementation
// ============================================================================

/// Phase 0: stdlib Compilation & Preparation
pub struct Phase0CoreCompiler {
    /// Path to verum_std crate
    stdlib_path: PathBuf,

    /// Cache directory for artifacts
    cache_dir: PathBuf,

    /// Whether to force rebuild (ignore cache)
    force_rebuild: bool,
}

impl Phase0CoreCompiler {
    /// Create a new Phase 0 compiler
    pub fn new(stdlib_path: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            stdlib_path,
            cache_dir,
            force_rebuild: false,
        }
    }

    /// Enable force rebuild (ignore cache)
    pub fn force_rebuild(mut self, force: bool) -> Self {
        self.force_rebuild = force;
        self
    }

    /// Compile stdlib to static library and bitcode
    pub fn compile_core(&self) -> Result<StdlibArtifacts> {
        info!("Phase 0: Preparing stdlib");

        // Check cache or build from source
        if !self.force_rebuild {
            if let Ok(cached) = self.load_cached_artifacts() {
                if self.is_cache_valid(&cached)? {
                    info!("Using cached stdlib artifacts");
                    return Ok(cached);
                }
            }
        }

        info!("Building stdlib from source...");

        // Step 1: Compile Rust code to static library
        let static_lib = self.compile_to_static_lib()?;

        // Step 2: Generate LLVM bitcode for LTO
        let bitcode = self.generate_llvm_bitcode()?;

        // Step 3: Generate FFI exports
        let ffi_exports = self.generate_ffi_exports()?;

        // Step 4: Build stdlib registry
        let registry = self.build_stdlib_registry(&ffi_exports)?;

        // Step 5: Cache monomorphized generics
        let mono_cache = self.monomorphize_common_types()?;

        // Step 6: Compute source hash for cache invalidation
        let source_hash = self.compute_source_hash()?;

        let artifacts = StdlibArtifacts {
            static_library: static_lib,
            bitcode_library: bitcode,
            ffi_exports,
            registry,
            monomorphization_cache: mono_cache,
            build_timestamp: SystemTime::now(),
            source_hash,
        };

        // Cache artifacts for future builds
        self.cache_artifacts(&artifacts)?;

        Ok(artifacts)
    }

    // ------------------------------------------------------------------------
    // Step 1: Compile to Static Library
    // ------------------------------------------------------------------------

    fn compile_to_static_lib(&self) -> Result<PathBuf> {
        info!("  [1/5] Compiling to static library...");

        let output_dir = self.cache_dir.join("lib");
        fs::create_dir_all(&output_dir)?;

        // Run cargo build with staticlib crate-type
        let status = Command::new("cargo")
            .args(&[
                "rustc",
                "--release",
                "--manifest-path",
                self.stdlib_path.join("Cargo.toml").to_str().unwrap(),
                "--",
                "--crate-type=staticlib",
            ])
            .env("CARGO_TARGET_DIR", &output_dir)
            .status()
            .context("Failed to execute cargo rustc")?;

        if !status.success() {
            bail!("Failed to compile verum_std to static library");
        }

        // Locate the generated .a file
        let lib_path = output_dir.join("release").join("libverum_std.a");

        if !lib_path.exists() {
            bail!(
                "Static library not found at expected path: {}",
                lib_path.display()
            );
        }

        info!("    ✓ Static library: {}", lib_path.display());

        Ok(lib_path)
    }

    // ------------------------------------------------------------------------
    // Step 2: Generate LLVM Bitcode
    // ------------------------------------------------------------------------

    fn generate_llvm_bitcode(&self) -> Result<PathBuf> {
        info!("  [2/5] Generating LLVM bitcode...");

        let output_dir = self.cache_dir.join("bc");
        fs::create_dir_all(&output_dir)?;

        // Run cargo build with llvm-ir emit
        let status = Command::new("cargo")
            .args(&[
                "rustc",
                "--release",
                "--manifest-path",
                self.stdlib_path.join("Cargo.toml").to_str().unwrap(),
                "--",
                "--emit=llvm-bc",
            ])
            .env("CARGO_TARGET_DIR", &output_dir)
            .status()
            .context("Failed to execute cargo rustc for bitcode")?;

        if !status.success() {
            warn!("Cargo rustc for bitcode failed, attempting to discover existing bitcode...");
            // Fall back to discovering existing bitcode files
            if let Some(discovered_bc) = self.discover_bitcode_file(&output_dir)? {
                info!(
                    "    ✓ Found existing LLVM bitcode: {}",
                    discovered_bc.display()
                );
                return Ok(discovered_bc);
            }
            bail!(
                "Failed to generate LLVM bitcode and no existing bitcode found. \
                   Ensure LLVM tools are installed and rustc can emit bitcode."
            );
        }

        // Search for the generated .bc file in the deps directory
        let deps_dir = output_dir.join("release").join("deps");
        if let Some(bc_path) = self.discover_bitcode_file(&deps_dir)? {
            info!("    ✓ LLVM bitcode: {}", bc_path.display());
            return Ok(bc_path);
        }

        // Try the direct release path
        let direct_bc_path = output_dir.join("release").join("libverum_std.bc");
        if direct_bc_path.exists() {
            info!("    ✓ LLVM bitcode: {}", direct_bc_path.display());
            return Ok(direct_bc_path);
        }

        // Search recursively for any .bc file
        if let Some(found_bc) = self.find_bitcode_recursive(&output_dir)? {
            info!("    ✓ LLVM bitcode (discovered): {}", found_bc.display());
            return Ok(found_bc);
        }

        bail!(
            "LLVM bitcode generation succeeded but output file not found. \
             Searched in: {}",
            deps_dir.display()
        );
    }

    /// Discover a bitcode file in the given directory matching verum_std pattern
    fn discover_bitcode_file(&self, dir: &Path) -> Result<Option<PathBuf>> {
        if !dir.exists() {
            return Ok(None);
        }

        let entries = fs::read_dir(dir).context("Failed to read bitcode directory")?;

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Match patterns like: verum_std-*.bc or libverum_std*.bc
                    if filename.ends_with(".bc")
                        && (filename.contains("verum_std") || filename.starts_with("libverum_std"))
                    {
                        return Ok(Some(path));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Recursively search for bitcode files
    fn find_bitcode_recursive(&self, dir: &Path) -> Result<Option<PathBuf>> {
        if !dir.exists() {
            return Ok(None);
        }

        // First check the current directory
        if let Some(found) = self.discover_bitcode_file(dir)? {
            return Ok(Some(found));
        }

        // Recursively search subdirectories
        let entries = fs::read_dir(dir).context("Failed to read directory for bitcode search")?;

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = self.find_bitcode_recursive(&path)? {
                    return Ok(Some(found));
                }
            }
        }

        Ok(None)
    }

    // ------------------------------------------------------------------------
    // Step 3: Generate FFI Exports
    // ------------------------------------------------------------------------

    fn generate_ffi_exports(&self) -> Result<FFIExports> {
        info!("  [3/5] Generating FFI exports...");

        // Generate C-compatible wrappers for common types
        let mut symbol_mappings = Map::new();
        let mut header_lines = List::new();

        header_lines.push("/* Auto-generated FFI exports for verum_std */".into());
        header_lines.push("#ifndef VERUM_STD_FFI_H".into());
        header_lines.push("#define VERUM_STD_FFI_H\n".into());
        header_lines.push("#include <stdint.h>".into());
        header_lines.push("#include <stddef.h>\n".into());

        // Generate exports for List<T>
        self.generate_list_exports(&mut symbol_mappings, &mut header_lines)?;

        // Generate exports for Map<K, V>
        self.generate_map_exports(&mut symbol_mappings, &mut header_lines)?;

        // Generate exports for Text
        self.generate_text_exports(&mut symbol_mappings, &mut header_lines)?;

        // Generate exports for Maybe<T>
        self.generate_maybe_exports(&mut symbol_mappings, &mut header_lines)?;

        header_lines.push("\n#endif /* VERUM_STD_FFI_H */".into());

        let header_content = header_lines.join("\n");

        info!("    ✓ Generated {} FFI exports", symbol_mappings.len());

        Ok(FFIExports {
            header_content,
            symbol_mappings,
        })
    }

    fn generate_list_exports(
        &self,
        mappings: &mut Map<Text, Text>,
        header: &mut List<Text>,
    ) -> Result<()> {
        // Iterate the shared FFI primitive table (see FFI_PRIMITIVES).
        for prim in FFI_PRIMITIVES {
            // List.new
            let verum_path: Text = format!("List.<{}>.new", prim.verum).into();
            let c_symbol: Text = format!("verum_std_list_{}_new", prim.mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(format!("void* {}();", c_symbol).into());

            // List.push
            let verum_path: Text = format!("List.<{}>.push", prim.verum).into();
            let c_symbol: Text = format!("verum_std_list_{}_push", prim.mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(
                format!("void {}(void* list, {} value);", c_symbol, prim.c_type).into(),
            );

            // List.len
            let verum_path: Text = format!("List.<{}>.len", prim.verum).into();
            let c_symbol: Text = format!("verum_std_list_{}_len", prim.mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(format!("size_t {}(void* list);", c_symbol).into());
        }

        Ok(())
    }

    fn generate_map_exports(
        &self,
        mappings: &mut Map<Text, Text>,
        header: &mut List<Text>,
    ) -> Result<()> {
        // Iterate the shared FFI map-pair table (see FFI_MAP_PAIRS).
        for (k, v) in FFI_MAP_PAIRS {
            let k_prim = lookup_primitive(k);
            let v_prim = lookup_primitive(v);
            let k_mangled = k_prim.map(|p| p.mangled).unwrap_or(k);
            let v_mangled = v_prim.map(|p| p.mangled).unwrap_or(v);
            let k_c = k_prim.map(|p| p.c_type).unwrap_or("void*");
            let v_c = v_prim.map(|p| p.c_type).unwrap_or("void*");

            // Map.new
            let verum_path: Text = format!("Map.<{}, {}>.new", k, v).into();
            let c_symbol: Text = format!("verum_std_map_{}_{}_new", k_mangled, v_mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(format!("void* {}();", c_symbol).into());

            // Map.insert
            let verum_path: Text = format!("Map.<{}, {}>.insert", k, v).into();
            let c_symbol: Text = format!("verum_std_map_{}_{}_insert", k_mangled, v_mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(
                format!("void {}(void* map, {} key, {} value);", c_symbol, k_c, v_c).into(),
            );
        }

        Ok(())
    }

    fn generate_text_exports(
        &self,
        mappings: &mut Map<Text, Text>,
        header: &mut List<Text>,
    ) -> Result<()> {
        // Text.new
        mappings.insert("Text.new".into(), "verum_std_text_new".into());
        header.push("void* verum_std_text_new(const char* data, size_t len);".into());

        // Text.len
        mappings.insert("Text.len".into(), "verum_std_text_len".into());
        header.push("size_t verum_std_text_len(void* text);".into());

        // Text.concat
        mappings.insert("Text.concat".into(), "verum_std_text_concat".into());
        header.push("void* verum_std_text_concat(void* a, void* b);".into());

        Ok(())
    }

    fn generate_maybe_exports(
        &self,
        mappings: &mut Map<Text, Text>,
        header: &mut List<Text>,
    ) -> Result<()> {
        // Iterate the shared FFI maybe-primitive table (see FFI_MAYBE_PRIMITIVES).
        for ty in FFI_MAYBE_PRIMITIVES {
            let prim = lookup_primitive(ty);
            let ty_mangled = prim.map(|p| p.mangled).unwrap_or(ty);
            let ty_c = prim.map(|p| p.c_type).unwrap_or("void*");

            // Maybe.Some
            let verum_path: Text = format!("Maybe.<{}>.Some", ty).into();
            let c_symbol: Text = format!("verum_std_maybe_{}_some", ty_mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(format!("void* {}({} value);", c_symbol, ty_c).into());

            // Maybe.None
            let verum_path: Text = format!("Maybe.<{}>.None", ty).into();
            let c_symbol: Text = format!("verum_std_maybe_{}_none", ty_mangled).into();
            mappings.insert(verum_path.clone(), c_symbol.clone());
            header.push(format!("void* {}();", c_symbol).into());
        }

        Ok(())
    }

    // Note: C type lookups are performed directly via `lookup_primitive(...)`
    // from the FFI export generators; no per-type method is needed.

    // ------------------------------------------------------------------------
    // Step 4: Build stdlib Registry
    // ------------------------------------------------------------------------

    fn build_stdlib_registry(&self, ffi_exports: &FFIExports) -> Result<StdlibRegistry> {
        info!("  [4/5] Building stdlib registry...");

        let mut functions = Map::new();

        // Parse FFI source file to extract actual function signatures
        let ffi_source_path = self.stdlib_path.join("src").join("ffi.rs");
        let parsed_signatures = self.parse_ffi_source(&ffi_source_path)?;

        // Convert FFI exports to function descriptors
        for (verum_path, c_symbol) in &ffi_exports.symbol_mappings {
            let descriptor = self.create_function_descriptor(
                verum_path.as_str(),
                c_symbol.as_str(),
                &parsed_signatures,
            )?;
            functions.insert(verum_path.clone(), descriptor);
        }

        info!("    ✓ Registered {} functions", functions.len());

        Ok(StdlibRegistry { functions })
    }

    /// Parse the FFI source file to extract actual function signatures
    fn parse_ffi_source(&self, ffi_path: &Path) -> Result<Map<Text, ParsedFFIFunction>> {
        let mut parsed = Map::new();

        // Read the FFI source file
        let source = match fs::read_to_string(ffi_path) {
            Ok(content) => content,
            Err(e) => {
                warn!(
                    "Could not read FFI source file {}: {}. Using inferred signatures.",
                    ffi_path.display(),
                    e
                );
                return Ok(parsed);
            }
        };

        // Regex to match FFI function declarations
        // Pattern matches: #[unsafe(no_mangle)] pub [unsafe] extern "C" fn name(params) -> return_type
        let fn_pattern = Regex::new(
            r#"(?x)
            # Optional doc comments (capture group 1)
            (?:///\s*(.+)\n)*
            # The no_mangle attribute
            \#\[unsafe\(no_mangle\)\]\s*
            # pub and optional unsafe
            pub\s+(unsafe\s+)?extern\s+"C"\s+fn\s+
            # Function name (capture group 2 or 3 depending on unsafe)
            (\w+)\s*
            # Parameters (capture group 3 or 4)
            \(([^)]*)\)\s*
            # Optional return type (capture group 4 or 5)
            (?:->\s*([^\{]+))?
            "#,
        )
        .context("Failed to compile FFI function regex")?;

        for cap in fn_pattern.captures_iter(&source) {
            let is_unsafe = cap.get(2).map_or(false, |m| m.as_str().contains("unsafe"));
            let fn_name = cap.get(3).map_or("", |m| m.as_str());
            let params_str = cap.get(4).map_or("", |m| m.as_str());
            let return_type = cap
                .get(5)
                .map_or("()".to_string(), |m| m.as_str().trim().to_string());

            // Parse parameters
            let params: Vec<String> = if params_str.trim().is_empty() {
                vec![]
            } else {
                params_str
                    .split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            };

            // Extract doc comment if present
            let doc = cap.get(1).map(|m| m.as_str().to_string());

            parsed.insert(
                Text::from(fn_name),
                ParsedFFIFunction {
                    symbol: fn_name.to_string(),
                    params,
                    return_type,
                    is_unsafe,
                    doc,
                },
            );
        }

        debug!(
            "Parsed {} FFI function signatures from {}",
            parsed.len(),
            ffi_path.display()
        );

        Ok(parsed)
    }

    fn create_function_descriptor(
        &self,
        verum_path: &str,
        c_symbol: &str,
        parsed_signatures: &Map<Text, ParsedFFIFunction>,
    ) -> Result<FunctionDescriptor> {
        // Try to get the parsed signature from the FFI source
        let (signature, is_unsafe, doc) =
            if let Some(parsed) = parsed_signatures.get(&Text::from(c_symbol)) {
                let sig = self.convert_parsed_signature(parsed)?;
                (sig, parsed.is_unsafe, parsed.doc.clone().map(Text::from))
            } else {
                // Fall back to inferred signature
                let sig = self.infer_signature(verum_path)?;
                (sig, c_symbol.contains("unsafe"), None)
            };

        // Determine computational properties based on function characteristics
        let properties = self.infer_computational_properties(verum_path, c_symbol);

        // Determine context requirements (most stdlib functions don't require contexts)
        let context_requirements = self.infer_context_requirements(verum_path);

        // Determine if this is an intrinsic
        let intrinsic_id = self.detect_intrinsic(verum_path);

        Ok(FunctionDescriptor {
            verum_path: verum_path.into(),
            c_symbol: c_symbol.into(),
            signature,
            properties,
            context_requirements,
            tier0_impl: None,  // Populated by interpreter
            jit_address: None, // Populated by JIT
            intrinsic_id,
            is_unsafe,
            doc,
        })
    }

    /// Convert a parsed FFI signature to our FunctionSignature type
    fn convert_parsed_signature(&self, parsed: &ParsedFFIFunction) -> Result<FunctionSignature> {
        let mut params = List::new();

        for param in &parsed.params {
            // Parse "name: Type" format
            let param_text = self.rust_type_to_verum_type(param);
            params.push(Text::from(param_text));
        }

        let return_type = self.rust_type_to_verum_type(&parsed.return_type);

        // Infer type parameters from the signature
        let type_params = self.infer_type_params_from_signature(&params, &return_type);

        Ok(FunctionSignature {
            params,
            return_type: Text::from(return_type),
            type_params,
        })
    }

    /// Convert Rust FFI types to Verum types
    fn rust_type_to_verum_type(&self, rust_type: &str) -> String {
        let trimmed = rust_type.trim();

        // Handle pointer types
        if trimmed.starts_with("*mut ") || trimmed.starts_with("*const ") {
            let inner = trimmed
                .strip_prefix("*mut ")
                .or_else(|| trimmed.strip_prefix("*const "))
                .unwrap_or(trimmed);

            // Map common pointer types
            if inner.contains("CBGRTracked<List<") {
                return format!("&mut List<{}>", self.extract_inner_type(inner));
            }
            if inner.contains("CBGRTracked<Map<") {
                return format!("&mut Map<{}>", self.extract_inner_type(inner));
            }
            if inner.contains("CBGRTracked<Text>") {
                return "&mut Text".to_string();
            }
            if inner.contains("CBGRTracked<Maybe<") {
                return format!("&mut Maybe<{}>", self.extract_inner_type(inner));
            }
            return format!("&mut {}", inner);
        }

        // Handle primitive types
        match trimmed {
            "i8" => "Int8".to_string(),
            "i16" => "Int16".to_string(),
            "i32" => "Int32".to_string(),
            "i64" => "Int".to_string(),
            "u8" => "UInt8".to_string(),
            "u16" => "UInt16".to_string(),
            "u32" => "UInt32".to_string(),
            "u64" => "UInt64".to_string(),
            "usize" => "USize".to_string(),
            "f32" => "Float32".to_string(),
            "f64" => "Float".to_string(),
            "bool" => "Bool".to_string(),
            "()" => "Unit".to_string(),
            _ => trimmed.to_string(),
        }
    }

    /// Extract inner type from generic like "CBGRTracked<List<i64>>"
    fn extract_inner_type(&self, rust_type: &str) -> String {
        // Find the innermost generic parameter
        if let Some(start) = rust_type.find('<') {
            if let Some(end) = rust_type.rfind('>') {
                let inner = &rust_type[start + 1..end];
                // Handle nested generics
                if let Some(inner_start) = inner.find('<') {
                    if let Some(inner_end) = inner.rfind('>') {
                        return inner[inner_start + 1..inner_end].to_string();
                    }
                }
                return inner.to_string();
            }
        }
        rust_type.to_string()
    }

    /// Infer type parameters from signature
    fn infer_type_params_from_signature(
        &self,
        params: &List<Text>,
        return_type: &str,
    ) -> List<Text> {
        let mut type_params = List::new();
        let mut seen = std::collections::HashSet::new();

        // Check for common generic type variables
        let all_text = format!(
            "{} {}",
            params
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            return_type
        );

        for var in &["T", "K", "V", "U", "E"] {
            if all_text.contains(&format!("<{}>", var))
                || all_text.contains(&format!("{}>", var))
                || all_text.contains(&format!("<{}", var))
            {
                if seen.insert(*var) {
                    type_params.push(Text::from(*var));
                }
            }
        }

        type_params
    }

    /// Infer computational properties for a function
    fn infer_computational_properties(&self, verum_path: &str, c_symbol: &str) -> PropertySet {
        let mut properties = Vec::new();

        // I/O functions
        if c_symbol.contains("print")
            || c_symbol.contains("read")
            || c_symbol.contains("write")
            || c_symbol.contains("file")
        {
            properties.push(ComputationalProperty::IO);
        }

        // Mutating functions
        if c_symbol.contains("push")
            || c_symbol.contains("pop")
            || c_symbol.contains("insert")
            || c_symbol.contains("remove")
            || c_symbol.contains("clear")
            || c_symbol.contains("set")
            || verum_path.contains("mut")
        {
            properties.push(ComputationalProperty::Mutates);
        }

        // Allocation functions
        if c_symbol.contains("new")
            || c_symbol.contains("alloc")
            || c_symbol.contains("create")
            || c_symbol.contains("clone")
        {
            properties.push(ComputationalProperty::Allocates);
        }

        // Deallocation functions
        if c_symbol.contains("free") || c_symbol.contains("drop") || c_symbol.contains("dealloc") {
            properties.push(ComputationalProperty::Deallocates);
        }

        // FFI functions
        if c_symbol.starts_with("verum_std_") {
            properties.push(ComputationalProperty::FFI);
        }

        // If no properties were added, it's pure
        if properties.is_empty() {
            PropertySet::pure()
        } else {
            PropertySet::from_properties(properties)
        }
    }

    /// Infer context requirements for a function
    fn infer_context_requirements(&self, verum_path: &str) -> List<Text> {
        let mut requirements = List::new();

        // Database-related functions
        if verum_path.contains("database")
            || verum_path.contains("sql")
            || verum_path.contains("query")
        {
            requirements.push(Text::from("Database"));
        }

        // Logger-related functions
        if verum_path.contains("log")
            || verum_path.contains("debug")
            || verum_path.contains("trace")
        {
            requirements.push(Text::from("Logger"));
        }

        // File system functions
        if verum_path.contains("file") || verum_path.contains("path") || verum_path.contains("dir")
        {
            requirements.push(Text::from("FileSystem"));
        }

        // Network functions
        if verum_path.contains("http")
            || verum_path.contains("socket")
            || verum_path.contains("network")
        {
            requirements.push(Text::from("Network"));
        }

        requirements
    }

    fn infer_signature(&self, path: &str) -> Result<FunctionSignature> {
        // Comprehensive signature inference based on method patterns
        // This maps Verum stdlib method names to their expected signatures

        // Extract type name and method name
        let parts: Vec<&str> = path.split('.').collect();
        let (type_name, method_name) = if parts.len() >= 2 {
            (parts[0], parts[parts.len() - 1])
        } else {
            ("", path)
        };

        // Collection constructors
        if method_name == "new" || method_name == "empty" {
            return Ok(FunctionSignature {
                params: List::new(),
                return_type: Text::from(type_name),
                type_params: self.infer_type_params(type_name),
            });
        }

        // Collection with capacity
        if method_name == "with_capacity" {
            return Ok(FunctionSignature {
                params: List::from(vec![Text::from("capacity: usize")]),
                return_type: Text::from(type_name),
                type_params: self.infer_type_params(type_name),
            });
        }

        // Common collection methods
        match method_name {
            // Size/capacity queries
            "len" | "length" | "size" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("usize"),
                type_params: List::new(),
            }),
            "is_empty" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("bool"),
                type_params: List::new(),
            }),
            "capacity" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("usize"),
                type_params: List::new(),
            }),

            // List operations
            "push" | "push_back" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &mut Self"), Text::from("value: T")]),
                return_type: Text::from("void"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "pop" | "pop_back" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &mut Self")]),
                return_type: Text::from("Maybe<T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "get" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self"), Text::from("index: usize")]),
                return_type: Text::from("Maybe<&T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "get_mut" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: &mut Self"),
                    Text::from("index: usize"),
                ]),
                return_type: Text::from("Maybe<&mut T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "first" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("Maybe<&T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "last" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("Maybe<&T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "clear" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &mut Self")]),
                return_type: Text::from("void"),
                type_params: List::new(),
            }),
            "reserve" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: &mut Self"),
                    Text::from("additional: usize"),
                ]),
                return_type: Text::from("void"),
                type_params: List::new(),
            }),

            // Map operations
            "insert" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: &mut Self"),
                    Text::from("key: K"),
                    Text::from("value: V"),
                ]),
                return_type: Text::from("Maybe<V>"),
                type_params: List::from(vec![Text::from("K"), Text::from("V")]),
            }),
            "remove" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &mut Self"), Text::from("key: &K")]),
                return_type: Text::from("Maybe<V>"),
                type_params: List::from(vec![Text::from("K"), Text::from("V")]),
            }),
            "contains_key" | "contains" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self"), Text::from("key: &K")]),
                return_type: Text::from("bool"),
                type_params: List::from(vec![Text::from("K")]),
            }),

            // Text operations
            "as_str" | "as_bytes" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("&str"),
                type_params: List::new(),
            }),
            "to_uppercase" | "to_lowercase" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("Text"),
                type_params: List::new(),
            }),
            "trim" | "trim_start" | "trim_end" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("&str"),
                type_params: List::new(),
            }),
            "split" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self"), Text::from("pattern: &str")]),
                return_type: Text::from("List<Text>"),
                type_params: List::new(),
            }),
            "concat" | "join" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self"), Text::from("other: &str")]),
                return_type: Text::from("Text"),
                type_params: List::new(),
            }),
            "starts_with" | "ends_with" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self"), Text::from("pattern: &str")]),
                return_type: Text::from("bool"),
                type_params: List::new(),
            }),

            // Iterator methods
            "iter" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("Iterator<Item = &T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "iter_mut" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &mut Self")]),
                return_type: Text::from("Iterator<Item = &mut T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "map" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: Self"), Text::from("f: fn(T) -> U")]),
                return_type: Text::from("Iterator<Item = U>"),
                type_params: List::from(vec![Text::from("T"), Text::from("U")]),
            }),
            "filter" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: Self"),
                    Text::from("predicate: fn(&T) -> bool"),
                ]),
                return_type: Text::from("Iterator<Item = T>"),
                type_params: List::from(vec![Text::from("T")]),
            }),
            "fold" | "reduce" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: Self"),
                    Text::from("init: U"),
                    Text::from("f: fn(U, T) -> U"),
                ]),
                return_type: Text::from("U"),
                type_params: List::from(vec![Text::from("T"), Text::from("U")]),
            }),
            "collect" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: Self")]),
                return_type: Text::from("C"),
                type_params: List::from(vec![Text::from("C")]),
            }),

            // Clone/copy
            "clone" => Ok(FunctionSignature {
                params: List::from(vec![Text::from("self: &Self")]),
                return_type: Text::from("Self"),
                type_params: List::new(),
            }),

            // Debug/display
            "fmt" | "debug_fmt" => Ok(FunctionSignature {
                params: List::from(vec![
                    Text::from("self: &Self"),
                    Text::from("f: &mut Formatter"),
                ]),
                return_type: Text::from("Result<(), Error>"),
                type_params: List::new(),
            }),

            // Default fallback
            _ => Ok(FunctionSignature {
                params: List::new(),
                return_type: Text::from("void"),
                type_params: List::new(),
            }),
        }
    }

    /// Infer type parameters for a given type name.
    ///
    /// Driven by `GENERIC_TYPE_PARAMS`: stdlib container types that need
    /// generic parameters to be materialized at FFI time.
    fn infer_type_params(&self, type_name: &str) -> List<Text> {
        GENERIC_TYPE_PARAMS
            .iter()
            .find(|(name, _)| *name == type_name)
            .map(|(_, params)| List::from(params.iter().map(|p| Text::from(*p)).collect::<Vec<_>>()))
            .unwrap_or_default()
    }

    /// Detect whether a stdlib path corresponds to a compiler intrinsic.
    ///
    /// Driven by `INTRINSIC_PATTERNS`: matches `(type_segment, method_segment)`
    /// appearing in the path. Used by codegen to inline rather than FFI-call.
    fn detect_intrinsic(&self, path: &str) -> Option<IntrinsicId> {
        INTRINSIC_PATTERNS
            .iter()
            .find(|(ty_seg, method_seg, _)| path.contains(ty_seg) && path.contains(method_seg))
            .map(|(_, _, id)| *id)
    }

    // ------------------------------------------------------------------------
    // Step 5: Monomorphize Common Types
    // ------------------------------------------------------------------------

    fn monomorphize_common_types(&self) -> Result<MonomorphizationCache> {
        info!("  [5/5] Caching monomorphized generics...");

        let mut instantiations = List::new();

        // Pre-instantiate common List types from the shared FFI primitive table.
        for prim in FFI_PRIMITIVES {
            instantiations.push(MonomorphizedFunction {
                generic_path: "List.new".into(),
                type_args: List::from(vec![Text::from(prim.verum)]),
                symbol: format!("verum_std_list_{}_new", prim.mangled).into(),
            });
        }

        // Pre-instantiate common Map types from the shared FFI map-pair table.
        for (k, v) in FFI_MAP_PAIRS {
            let k_mangled = lookup_primitive(k).map(|p| p.mangled).unwrap_or(k);
            let v_mangled = lookup_primitive(v).map(|p| p.mangled).unwrap_or(v);
            instantiations.push(MonomorphizedFunction {
                generic_path: "Map.new".into(),
                type_args: List::from(vec![Text::from(*k), Text::from(*v)]),
                symbol: format!("verum_std_map_{}_{}_new", k_mangled, v_mangled).into(),
            });
        }

        info!(
            "    ✓ Cached {} monomorphized instances",
            instantiations.len()
        );

        Ok(MonomorphizationCache { instantiations })
    }

    // ------------------------------------------------------------------------
    // Caching & Validation
    // ------------------------------------------------------------------------

    fn compute_source_hash(&self) -> Result<Text> {
        // Compute hash of all source files in verum_std for proper cache invalidation
        // using Blake3 for fast, high-quality hashing.
        let mut hasher = crate::hash::ContentHash::new();

        // Hash Cargo.toml for dependency changes
        let cargo_toml = self.stdlib_path.join("Cargo.toml");
        if let Ok(metadata) = fs::metadata(&cargo_toml) {
            if let Ok(mtime) = metadata.modified() {
                if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(&duration.as_nanos().to_le_bytes());
                }
            }
        }

        // Hash all .rs files in src/ directory
        let src_dir = self.stdlib_path.join("src");
        if src_dir.exists() {
            if let Ok(entries) = fs::read_dir(&src_dir) {
                let mut paths: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().map(|ext| ext == "rs").unwrap_or(false))
                    .collect();

                // Sort for deterministic hashing
                paths.sort();

                for path in paths {
                    if let Ok(metadata) = fs::metadata(&path) {
                        // Hash file path and modification time
                        if let Some(path_str) = path.to_str() {
                            hasher.update_str(path_str);
                        }
                        if let Ok(mtime) = metadata.modified() {
                            if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                                hasher.update(&duration.as_nanos().to_le_bytes());
                            }
                        }
                        // Also hash file size for extra reliability
                        hasher.update(&metadata.len().to_le_bytes());
                    }
                }
            }
        }

        // Also check lib.rs and mod.rs files in subdirectories
        self.hash_subdir_files(&src_dir, &mut hasher);

        Ok(format!("{:016x}", hasher.finalize().to_u64()).into())
    }

    /// Recursively hash modification times of .rs files in subdirectories using Blake3.
    fn hash_subdir_files(
        &self,
        dir: &std::path::Path,
        hasher: &mut crate::hash::ContentHash,
    ) {
        if let Ok(entries) = fs::read_dir(dir) {
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            paths.sort_by_key(|e| e.path());

            for entry in paths {
                let path = entry.path();
                if path.is_dir() {
                    self.hash_subdir_files(&path, hasher);
                } else if path.extension().map(|ext| ext == "rs").unwrap_or(false) {
                    if let Ok(metadata) = fs::metadata(&path) {
                        if let Some(path_str) = path.to_str() {
                            hasher.update_str(path_str);
                        }
                        if let Ok(mtime) = metadata.modified() {
                            if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                                hasher.update(&duration.as_nanos().to_le_bytes());
                            }
                        }
                        hasher.update(&metadata.len().to_le_bytes());
                    }
                }
            }
        }
    }

    fn cache_artifacts(&self, artifacts: &StdlibArtifacts) -> Result<()> {
        fs::create_dir_all(&self.cache_dir)?;

        let cache_file = self.cache_dir.join("stdlib_artifacts.json");
        let json = serde_json::to_string_pretty(artifacts)?;
        fs::write(&cache_file, json)?;

        debug!("Cached artifacts to: {}", cache_file.display());

        Ok(())
    }

    fn load_cached_artifacts(&self) -> Result<StdlibArtifacts> {
        let cache_file = self.cache_dir.join("stdlib_artifacts.json");
        let json = fs::read_to_string(&cache_file)?;
        let artifacts: StdlibArtifacts = serde_json::from_str(&json)?;

        Ok(artifacts)
    }

    fn is_cache_valid(&self, cached: &StdlibArtifacts) -> Result<bool> {
        // Check if source files have changed
        let current_hash = self.compute_source_hash()?;

        if current_hash != cached.source_hash {
            debug!("Cache invalid: source files changed");
            return Ok(false);
        }

        // Check if output files exist
        if !cached.static_library.exists() {
            debug!("Cache invalid: static library missing");
            return Ok(false);
        }

        debug!(
            "Cache valid: reusing artifacts from {:?}",
            cached.build_timestamp
        );

        Ok(true)
    }
}

// ============================================================================
// CompilationPhase Implementation
// ============================================================================

impl CompilationPhase for Phase0CoreCompiler {
    fn name(&self) -> &str {
        "Phase 0: stdlib Compilation & Preparation"
    }

    fn description(&self) -> &str {
        "Compile verum_std to static library, generate FFI exports, and build symbol registry"
    }

    fn execute(&self, _input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = std::time::Instant::now();

        let artifacts = self.compile_core().map_err(|e| {
            vec![
                verum_diagnostics::DiagnosticBuilder::error()
                    .message(format!("Phase 0 stdlib compilation failed: {}", e))
                    .build(),
            ]
        })?;

        let duration = start.elapsed();

        let mut metrics = PhaseMetrics::new(self.name())
            .with_duration(duration)
            .with_items_processed(artifacts.registry.functions.len());

        metrics.add_custom_metric("static_lib", artifacts.static_library.display().to_string());
        metrics.add_custom_metric("bitcode", artifacts.bitcode_library.display().to_string());
        metrics.add_custom_metric(
            "ffi_exports",
            artifacts.ffi_exports.symbol_mappings.len().to_string(),
        );
        metrics.add_custom_metric(
            "monomorphized",
            artifacts
                .monomorphization_cache
                .instantiations
                .len()
                .to_string(),
        );

        info!("{}", metrics.report());

        Ok(PhaseOutput {
            data: PhaseData::SourceFiles(List::new()), // Phase 0 doesn't transform source
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        false // Phase 0 must complete before other phases
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Create a Phase 0 compiler with default settings
pub fn create_phase0_compiler(workspace_root: &Path) -> Result<Phase0CoreCompiler> {
    let stdlib_path = workspace_root.join("crates/verum_std");
    let cache_dir = workspace_root.join("target/verum_cache/stdlib");

    Ok(Phase0CoreCompiler::new(stdlib_path, cache_dir))
}

/// Compile stdlib with default settings
pub fn compile_core(workspace_root: &Path) -> Result<StdlibArtifacts> {
    let compiler = create_phase0_compiler(workspace_root)?;
    compiler.compile_core()
}

/// Compile stdlib and return registry only (for interpreter)
pub fn get_stdlib_registry(workspace_root: &Path) -> Result<StdlibRegistry> {
    let artifacts = compile_core(workspace_root)?;
    Ok(artifacts.registry)
}

// ============================================================================
// Tests
// ============================================================================
