//! Phase 7.5: Final Linking
//!
//! This phase performs the final linking step where user-compiled code is combined
//! with the pre-compiled stdlib. This is crucial for achieving zero-cost abstractions
//! through LTO (Link Time Optimization).
//!
//! Runtime support is provided by stdlib via FFI and intrinsics. The VBC interpreter
//! handles all execution internally, and AOT compilation links directly with stdlib.
//!
//! ## NO LIBC ARCHITECTURE
//!
//! **IMPORTANT**: Verum does NOT link against libc or any system C libraries.
//! All runtime functionality is provided by:
//! - LLVM intrinsics (llvm.sin.f32, llvm.sqrt.f64, etc.)
//! - Custom Verum runtime implementations in `/core/`
//! - Platform-specific system calls via the Verum syscall layer
//!
//! This enables:
//! - Fully self-contained binaries with no external dependencies
//! - Consistent behavior across all platforms
//! - Smaller binary sizes (no libc bloat)
//! - Better optimization opportunities (no opaque library calls)
//!
//! Entry point: `/core/sys/init.vr` provides the custom entry point that
//! initializes the Verum runtime before calling the user's `main` function.
//!
//! ## Features
//!
//! - Object file linking with proper symbol resolution
//! - Stdlib linking (stdlib.vbca provides all runtime support)
//! - Link-Time Optimization (Thin/Full LTO)
//! - Cross-module inlining via LLVM bitcode
//! - Generic monomorphization during linking
//! - Self-contained linking (no libc, no external dependencies)
//! - Multiple output formats:
//!   - Native executable (default)
//!   - Shared library (.so/.dylib/.dll)
//!   - Static library (.a)
//!   - Object file (.o)
//!
//! ## Performance Characteristics
//!
//! | Linking Mode | Time | Binary Size | Runtime Performance |
//! |-------------|------|-------------|-------------------|
//! | Debug (no LTO) | ~100ms | 5MB | Baseline |
//! | Release (thin LTO) | ~500ms | 3MB | 1.5x faster |
//! | Release (full LTO) | ~2s | 2.5MB | 2x faster |
//!
//! Phase 7.5: Final linking. Links object files with libverum_std.a,
//! applies LTO optimization, produces final executable binary.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;
use tracing::{debug, info, warn};
use verum_common::{List, Map, Text};

use super::phase0_stdlib::StdlibArtifacts;
use super::{CompilationPhase, ExecutionTier, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

// V-LLSI no-libc linking configuration
use verum_codegen::link::{NoLibcConfig, Platform};

// =============================================================================
// Configuration Types
// =============================================================================

/// Output kind for the linking phase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputKind {
    /// Native executable (default)
    #[default]
    Executable,
    /// Shared library (.so on Linux, .dylib on macOS, .dll on Windows)
    SharedLibrary,
    /// Static library (.a)
    StaticLibrary,
    /// Object file (.o) - no linking, just combine object files
    ObjectFile,
}

impl OutputKind {
    /// Get the appropriate file extension for this output kind
    pub fn extension(&self) -> &'static str {
        match self {
            OutputKind::Executable => {
                #[cfg(target_os = "windows")]
                {
                    "exe"
                }
                #[cfg(not(target_os = "windows"))]
                {
                    ""
                }
            }
            OutputKind::SharedLibrary => {
                #[cfg(target_os = "macos")]
                {
                    "dylib"
                }
                #[cfg(target_os = "windows")]
                {
                    "dll"
                }
                #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
                {
                    "so"
                }
            }
            OutputKind::StaticLibrary => "a",
            OutputKind::ObjectFile => "o",
        }
    }

    /// Get the appropriate library prefix for this output kind
    pub fn library_prefix(&self) -> &'static str {
        match self {
            OutputKind::SharedLibrary | OutputKind::StaticLibrary => {
                #[cfg(target_os = "windows")]
                {
                    ""
                }
                #[cfg(not(target_os = "windows"))]
                {
                    "lib"
                }
            }
            _ => "",
        }
    }
}

/// LTO (Link-Time Optimization) configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LTOConfig {
    /// No LTO (fastest linking)
    None,
    /// Thin LTO (good balance of speed and optimization)
    Thin,
    /// Full LTO (best optimization, slowest linking)
    Full,
}

impl Default for LTOConfig {
    fn default() -> Self {
        LTOConfig::Thin
    }
}

/// Linking configuration
#[derive(Debug, Clone)]
pub struct LinkingConfig {
    /// Output kind (executable, shared library, static library, object file)
    pub output_kind: OutputKind,

    /// LTO mode
    pub lto: LTOConfig,

    /// Use LLVM linker (lld)
    pub use_llvm_linker: bool,

    /// Enable position-independent code
    pub pic: bool,

    /// Strip debug symbols
    pub strip: bool,

    /// Additional linker flags
    pub extra_flags: List<Text>,

    /// Output path for executable
    pub output_path: PathBuf,

    /// Library search paths
    pub library_paths: List<PathBuf>,

    /// Libraries to link
    pub libraries: List<Text>,

    /// Export symbols for shared library (public API)
    pub exported_symbols: List<Text>,

    /// Entry point symbol (for executables)
    pub entry_point: Option<Text>,

    /// Enable debug info
    pub debug_info: bool,

    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target_triple: Option<Text>,

    /// Enable static linking (no runtime dependencies)
    pub static_link: bool,

    /// Strip debug symbols only (keep function names)
    pub strip_debug_only: bool,

    /// V-LLSI no-libc linking configuration.
    ///
    /// When set, Verum produces fully self-contained binaries without libc:
    /// - **Linux**: Direct syscalls (stable kernel ABI), no libraries
    /// - **macOS**: libSystem.B.dylib only (Apple prohibits direct syscalls)
    /// - **Windows**: ntdll.dll + kernel32.dll only (no MSVC CRT)
    /// - **FreeBSD**: Direct syscalls
    /// - **Embedded**: No OS dependencies, custom entry point
    pub no_libc_config: Option<NoLibcConfig>,
}

impl Default for LinkingConfig {
    fn default() -> Self {
        Self {
            output_kind: OutputKind::Executable,
            lto: LTOConfig::Thin,
            use_llvm_linker: cfg!(target_os = "linux"), // lld works best on Linux
            pic: true,
            strip: false,
            extra_flags: List::new(),
            output_path: PathBuf::from("a.out"),
            library_paths: List::new(),
            // NO LIBC: Verum does not link against libc or system libraries.
            // All runtime functionality is provided by LLVM intrinsics or
            // custom implementations in /core/. See module documentation.
            libraries: List::new(),
            exported_symbols: List::new(),
            entry_point: Some("main".into()),
            debug_info: true,
            target_triple: None,
            static_link: false,
            strip_debug_only: false,
            // By default, use no-libc for the host platform
            no_libc_config: Some(NoLibcConfig::for_host()),
        }
    }
}

impl LinkingConfig {
    /// Create a configuration for building an executable
    pub fn for_executable(output_path: PathBuf) -> Self {
        Self {
            output_kind: OutputKind::Executable,
            output_path,
            ..Default::default()
        }
    }

    /// Create a configuration for building a shared library
    pub fn for_shared_library(output_path: PathBuf) -> Self {
        Self {
            output_kind: OutputKind::SharedLibrary,
            output_path,
            pic: true,         // PIC is required for shared libraries
            entry_point: None, // No entry point for libraries
            ..Default::default()
        }
    }

    /// Create a configuration for building a static library
    pub fn for_static_library(output_path: PathBuf) -> Self {
        Self {
            output_kind: OutputKind::StaticLibrary,
            output_path,
            lto: LTOConfig::None, // LTO is deferred to final link
            entry_point: None,
            ..Default::default()
        }
    }

    /// Create a configuration for producing an object file
    pub fn for_object_file(output_path: PathBuf) -> Self {
        Self {
            output_kind: OutputKind::ObjectFile,
            output_path,
            lto: LTOConfig::None,
            entry_point: None,
            ..Default::default()
        }
    }

    /// Configure for no-libc linking on the specified platform.
    ///
    /// V-LLSI Architecture: Verum produces fully self-contained binaries:
    /// - **Linux**: Direct syscalls (stable kernel ABI), no external libraries
    /// - **macOS**: libSystem.B.dylib only (Apple prohibits direct syscalls)
    /// - **Windows**: ntdll.dll + kernel32.dll only (no MSVC CRT)
    /// - **FreeBSD**: Direct syscalls
    /// - **Embedded**: No OS dependencies, custom entry point
    pub fn with_no_libc(mut self, platform: Platform) -> Self {
        self.no_libc_config = Some(NoLibcConfig::for_platform(platform));
        self
    }

    /// Configure for no-libc linking on the host platform.
    pub fn with_no_libc_host(mut self) -> Self {
        self.no_libc_config = Some(NoLibcConfig::for_host());
        self
    }

    /// Configure for embedded/bare-metal target (no OS).
    pub fn for_embedded(mut self) -> Self {
        self.no_libc_config = Some(NoLibcConfig::embedded());
        self.static_link = true;
        self
    }

    /// Check if this configuration uses no-libc linking.
    pub fn is_no_libc(&self) -> bool {
        self.no_libc_config.is_some()
    }

    /// Get the effective entry point based on no-libc configuration.
    ///
    /// Returns the platform-specific entry point for no-libc builds,
    /// or the configured entry point for standard builds.
    pub fn effective_entry_point(&self) -> Option<&str> {
        if let Some(ref config) = self.no_libc_config {
            Some(&config.entry_point)
        } else {
            self.entry_point.as_ref().map(|t| t.as_str())
        }
    }
}

/// External symbol binding type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolBinding {
    /// Global symbol (visible outside this module)
    Global,
    /// Weak symbol (can be overridden)
    Weak,
    /// Local symbol (only visible within this module)
    Local,
}

/// External symbol type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolType {
    /// Function symbol
    Function,
    /// Data/variable symbol
    Data,
    /// Undefined (needs to be resolved from another module)
    Undefined,
}

/// External symbol reference
#[derive(Debug, Clone)]
pub struct ExternalSymbol {
    /// Symbol name
    pub name: Text,
    /// Binding type
    pub binding: SymbolBinding,
    /// Symbol type
    pub sym_type: SymbolType,
    /// Address (if resolved, 0 otherwise)
    pub address: u64,
    /// Size in bytes (for data symbols)
    pub size: u64,
    /// Source module
    pub source_module: Option<Text>,
}

impl ExternalSymbol {
    /// Create a new undefined symbol reference
    pub fn undefined(name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            binding: SymbolBinding::Global,
            sym_type: SymbolType::Undefined,
            address: 0,
            size: 0,
            source_module: None,
        }
    }

    /// Create a new defined function symbol
    pub fn function(name: impl Into<Text>, address: u64) -> Self {
        Self {
            name: name.into(),
            binding: SymbolBinding::Global,
            sym_type: SymbolType::Function,
            address,
            size: 0,
            source_module: None,
        }
    }

    /// Check if symbol is resolved
    pub fn is_resolved(&self) -> bool {
        self.sym_type != SymbolType::Undefined
    }
}

/// Symbol table for tracking symbols during linking
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// All symbols by name
    symbols: Map<Text, ExternalSymbol>,
    /// Undefined symbols that need resolution
    undefined: List<Text>,
    /// Exported symbols (public API)
    exported: List<Text>,
}

impl SymbolTable {
    /// Create a new empty symbol table
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a symbol to the table
    pub fn add_symbol(&mut self, symbol: ExternalSymbol) {
        let name = symbol.name.clone();
        if symbol.sym_type == SymbolType::Undefined {
            self.undefined.push(name.clone());
        }
        self.symbols.insert(name, symbol);
    }

    /// Mark a symbol as exported
    pub fn export_symbol(&mut self, name: impl Into<Text>) {
        self.exported.push(name.into());
    }

    /// Get a symbol by name
    pub fn get_symbol(&self, name: &str) -> Option<&ExternalSymbol> {
        self.symbols.get(&Text::from(name))
    }

    /// Resolve an undefined symbol with a definition
    pub fn resolve_symbol(&mut self, name: &str, address: u64) -> bool {
        if let Some(sym) = self.symbols.get_mut(&Text::from(name)) {
            if sym.sym_type == SymbolType::Undefined {
                sym.address = address;
                sym.sym_type = SymbolType::Function; // Assume function for now
                return true;
            }
        }
        false
    }

    /// Get all undefined symbols
    pub fn get_undefined(&self) -> &List<Text> {
        &self.undefined
    }

    /// Get count of undefined symbols
    pub fn undefined_count(&self) -> usize {
        self.symbols
            .iter()
            .filter(|(_, s)| s.sym_type == SymbolType::Undefined)
            .count()
    }

    /// Get count of resolved symbols
    pub fn resolved_count(&self) -> usize {
        self.symbols
            .iter()
            .filter(|(_, s)| s.sym_type != SymbolType::Undefined)
            .count()
    }
}

/// Object file representation
#[derive(Debug, Clone)]
pub struct ObjectFile {
    /// Path to object file
    pub path: PathBuf,

    /// Module name
    pub module_name: Text,

    /// Size in bytes
    pub size: usize,

    /// Contains bitcode for LTO
    pub has_bitcode: bool,

    /// Defined symbols in this object
    pub defined_symbols: List<Text>,

    /// Undefined symbols (external references)
    pub undefined_symbols: List<Text>,
}

impl ObjectFile {
    /// Create a new object file representation
    pub fn new(path: PathBuf, module_name: impl Into<Text>) -> Result<Self> {
        let size = std::fs::metadata(&path)
            .map(|m| m.len() as usize)
            .unwrap_or(0);

        Ok(Self {
            path,
            module_name: module_name.into(),
            size,
            has_bitcode: false,
            defined_symbols: List::new(),
            undefined_symbols: List::new(),
        })
    }

    /// Create from path, extracting module name from filename
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let module_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Self::new(path, module_name)
    }
}

/// LLVM bitcode representation
#[derive(Debug, Clone)]
pub struct Bitcode {
    /// Path to bitcode file
    pub path: PathBuf,

    /// Size in bytes
    pub size: usize,
}

impl Bitcode {
    pub fn from_file(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let size = std::fs::metadata(&path)?.len() as usize;
        Ok(Self { path, size })
    }
}

/// Generated binary/executable
#[derive(Debug, Clone)]
pub struct Binary {
    /// Path to binary
    pub path: PathBuf,

    /// Size in bytes
    pub size: usize,

    /// Is executable
    pub executable: bool,
}

impl Binary {
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let metadata = std::fs::metadata(&path)?;
        let size = metadata.len() as usize;

        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };

        #[cfg(not(unix))]
        let executable = true;

        Ok(Self {
            path,
            size,
            executable,
        })
    }
}

// =============================================================================
// Linking Statistics
// =============================================================================

/// Statistics for linking phase
#[derive(Debug, Clone, Default)]
pub struct LinkingStats {
    /// Number of object files linked
    pub object_files_linked: usize,

    /// Total size of object files (bytes)
    pub total_object_size: usize,

    /// Size of final binary (bytes)
    pub binary_size: usize,

    /// LTO mode used
    pub lto_mode: Option<LTOConfig>,

    /// Time spent merging bitcode (ms)
    pub bitcode_merge_time_ms: u64,

    /// Time spent in LTO optimization (ms)
    pub lto_optimization_time_ms: u64,

    /// Time spent in system linking (ms)
    pub system_link_time_ms: u64,

    /// Total linking time (ms)
    pub total_link_time_ms: u64,

    /// Number of symbols resolved
    pub symbols_resolved: usize,

    /// Number of functions inlined by LTO
    pub functions_inlined: usize,

    /// Dead code eliminated (bytes)
    pub dead_code_eliminated: usize,
}

impl LinkingStats {
    pub fn report(&self) -> Text {
        let mut report = format!(
            "Linking Statistics:\n\
             ==================\n\
             Object Files: {} ({:.2} MB)\n\
             Binary Size: {:.2} MB\n\
             Total Time: {:.2}s\n",
            self.object_files_linked,
            self.total_object_size as f64 / 1_048_576.0,
            self.binary_size as f64 / 1_048_576.0,
            self.total_link_time_ms as f64 / 1000.0
        );

        if let Some(lto) = self.lto_mode {
            report.push_str(&format!(
                "\nLTO Mode: {:?}\n\
                 Bitcode Merge: {:.2}s\n\
                 LTO Optimization: {:.2}s\n\
                 Functions Inlined: {}\n\
                 Dead Code Eliminated: {:.2} KB\n",
                lto,
                self.bitcode_merge_time_ms as f64 / 1000.0,
                self.lto_optimization_time_ms as f64 / 1000.0,
                self.functions_inlined,
                self.dead_code_eliminated as f64 / 1024.0
            ));
        }

        report.push_str(&format!(
            "\nSystem Link: {:.2}s\n\
             Symbols Resolved: {}\n",
            self.system_link_time_ms as f64 / 1000.0,
            self.symbols_resolved
        ));

        report.into()
    }
}

// =============================================================================
// Final Linker Implementation
// =============================================================================

/// Phase 7.5: Final Linking
///
/// This linker supports multiple output formats and properly resolves symbols
/// from object files, stdlib, and system libraries.
pub struct FinalLinker {
    /// Target execution tier
    target_tier: ExecutionTier,

    /// Stdlib artifacts from Phase 0
    stdlib_artifacts: Option<StdlibArtifacts>,

    /// Linking configuration
    config: LinkingConfig,

    /// Statistics
    stats: LinkingStats,

    /// Symbol table for tracking external symbols
    symbol_table: SymbolTable,

}

impl FinalLinker {
    /// Create a new final linker
    pub fn new(target_tier: ExecutionTier, config: LinkingConfig) -> Self {
        Self {
            target_tier,
            stdlib_artifacts: None,
            config,
            stats: LinkingStats::default(),
            symbol_table: SymbolTable::new(),
        }
    }

    /// Set stdlib artifacts
    pub fn with_core_artifacts(mut self, artifacts: StdlibArtifacts) -> Self {
        self.stdlib_artifacts = Some(artifacts);
        self
    }

    /// Add exported symbols
    pub fn with_exported_symbols(mut self, symbols: List<Text>) -> Self {
        for sym in symbols {
            self.symbol_table.export_symbol(sym);
        }
        self
    }

    /// Link object files to produce final binary
    ///
    /// This method dispatches to the appropriate linking strategy based on:
    /// 1. Target execution tier (interpreter, JIT, AOT)
    /// 2. Output kind (executable, shared library, static library, object file)
    pub fn link(&mut self, object_files: List<ObjectFile>) -> Result<Binary> {
        let start = Instant::now();

        info!(
            "Phase 7.5: Final Linking ({:?}, output: {:?})",
            self.target_tier, self.config.output_kind
        );

        // Step 1: Collect and resolve symbols from all object files
        self.collect_symbols(&object_files)?;

        // Step 2: Resolve external symbols from stdlib and system libraries
        self.resolve_external_symbols()?;

        // Step 3: Perform linking based on tier and output kind
        let binary = match self.config.output_kind {
            OutputKind::StaticLibrary => {
                // Static library: combine object files into .a archive
                self.create_static_library(&object_files)?
            }
            OutputKind::ObjectFile => {
                // Object file: merge objects into single relocatable
                self.create_merged_object(&object_files)?
            }
            OutputKind::SharedLibrary | OutputKind::Executable => {
                // Shared library or executable: perform full linking
                match self.target_tier {
                    ExecutionTier::Interpreter => {
                        // Interpreter doesn't need linking
                        info!("Tier 0: No linking required (interpreter mode)");
                        Binary {
                            path: PathBuf::from("<interpreter>"),
                            size: 0,
                            executable: false,
                        }
                    }

                    ExecutionTier::Aot => {
                        // AOT performs static linking with LTO
                        info!("AOT: Static linking with LTO");
                        self.static_link_with_lto(&object_files)?
                    }
                }
            }
        };

        let elapsed = start.elapsed();
        self.stats.total_link_time_ms = elapsed.as_millis() as u64;
        self.stats.symbols_resolved = self.symbol_table.resolved_count();

        info!(
            "Linking complete: {} ({:.2} MB) in {:.2}s, {} symbols resolved",
            binary.path.display(),
            binary.size as f64 / 1_048_576.0,
            elapsed.as_secs_f64(),
            self.stats.symbols_resolved
        );

        Ok(binary)
    }

    /// Collect symbols from all object files
    fn collect_symbols(&mut self, object_files: &[ObjectFile]) -> Result<()> {
        debug!(
            "Collecting symbols from {} object files",
            object_files.len()
        );

        for obj in object_files {
            // Add defined symbols
            for sym in &obj.defined_symbols {
                self.symbol_table
                    .add_symbol(ExternalSymbol::function(sym.clone(), 0));
            }

            // Add undefined symbols
            for sym in &obj.undefined_symbols {
                if self.symbol_table.get_symbol(sym.as_str()).is_none() {
                    self.symbol_table
                        .add_symbol(ExternalSymbol::undefined(sym.clone()));
                }
            }
        }

        debug!(
            "Collected {} symbols ({} undefined)",
            self.symbol_table.resolved_count() + self.symbol_table.undefined_count(),
            self.symbol_table.undefined_count()
        );

        Ok(())
    }

    /// Resolve external symbols from stdlib and system libraries
    fn resolve_external_symbols(&mut self) -> Result<()> {
        debug!("Resolving external symbols");

        // Get stdlib symbols if available
        if let Some(ref artifacts) = self.stdlib_artifacts {
            for (verum_path, c_symbol) in &artifacts.ffi_exports.symbol_mappings {
                // Try to resolve any undefined symbol that matches
                self.symbol_table.resolve_symbol(c_symbol.as_str(), 0);
                self.symbol_table.resolve_symbol(verum_path.as_str(), 0);
            }
        }

        // System library symbols are resolved by the linker at runtime
        // We just track them for diagnostic purposes
        let undefined_count = self.symbol_table.undefined_count();
        if undefined_count > 0 {
            debug!(
                "{} symbols still undefined (will be resolved by system linker)",
                undefined_count
            );
        }

        Ok(())
    }

    /// Create a static library (.a) from object files
    fn create_static_library(&mut self, object_files: &[ObjectFile]) -> Result<Binary> {
        let start = Instant::now();

        info!(
            "Creating static library: {}",
            self.config.output_path.display()
        );

        self.stats.object_files_linked = object_files.len();
        self.stats.total_object_size = object_files.iter().map(|o| o.size).sum();

        // Use ar to create the static library
        let mut cmd = Command::new("ar");
        cmd.arg("rcs"); // r=insert, c=create, s=index
        cmd.arg(&self.config.output_path);

        for obj in object_files {
            cmd.arg(&obj.path);
        }

        let output = cmd.output().context("Failed to execute ar")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Static library creation failed: {}", stderr);
        }

        // Add stdlib to the archive if available
        if let Some(ref artifacts) = self.stdlib_artifacts {
            if artifacts.static_library.exists() {
                // Extract objects from stdlib and add to our archive
                let temp_dir = self.get_temp_dir();
                let mut extract_cmd = Command::new("ar");
                extract_cmd
                    .arg("x")
                    .arg(&artifacts.static_library)
                    .current_dir(&temp_dir);
                extract_cmd
                    .output()
                    .context("Failed to extract stdlib objects")?;

                // Add extracted objects to our archive
                let entries = std::fs::read_dir(&temp_dir)?;
                let mut add_cmd = Command::new("ar");
                add_cmd.arg("rs").arg(&self.config.output_path);

                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("o") {
                        add_cmd.arg(&path);
                    }
                }

                add_cmd
                    .output()
                    .context("Failed to add stdlib objects to archive")?;
            }
        }

        let elapsed = start.elapsed();
        self.stats.system_link_time_ms = elapsed.as_millis() as u64;

        let binary = Binary::from_path(&self.config.output_path)?;
        self.stats.binary_size = binary.size;

        info!(
            "Static library created: {} ({:.2} KB)",
            self.config.output_path.display(),
            binary.size as f64 / 1024.0
        );

        Ok(binary)
    }

    /// Create a merged object file from multiple objects
    fn create_merged_object(&mut self, object_files: &[ObjectFile]) -> Result<Binary> {
        let start = Instant::now();

        info!(
            "Creating merged object file: {}",
            self.config.output_path.display()
        );

        self.stats.object_files_linked = object_files.len();
        self.stats.total_object_size = object_files.iter().map(|o| o.size).sum();

        if object_files.is_empty() {
            bail!("No object files provided for merging");
        }

        if object_files.len() == 1 {
            // Just copy the single object file
            std::fs::copy(&object_files[0].path, &self.config.output_path)
                .context("Failed to copy object file")?;
        } else {
            // Use ld -r to create a relocatable merge
            let mut cmd = Command::new("ld");
            cmd.arg("-r"); // Relocatable output
            cmd.arg("-o").arg(&self.config.output_path);

            for obj in object_files {
                cmd.arg(&obj.path);
            }

            let output = cmd.output().context("Failed to execute ld -r")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("Object merging failed: {}", stderr);
            }
        }

        let elapsed = start.elapsed();
        self.stats.system_link_time_ms = elapsed.as_millis() as u64;

        let binary = Binary::from_path(&self.config.output_path)?;
        self.stats.binary_size = binary.size;

        info!(
            "Merged object created: {} ({:.2} KB)",
            self.config.output_path.display(),
            binary.size as f64 / 1024.0
        );

        Ok(binary)
    }

    /// Link shared library (.so/.dylib/.dll)
    fn link_shared_library(&mut self, native_obj: &ObjectFile) -> Result<Binary> {
        let start = Instant::now();

        info!(
            "Linking shared library: {}",
            self.config.output_path.display()
        );

        let mut cmd = if self.config.use_llvm_linker {
            Command::new("ld.lld")
        } else {
            Command::new("cc")
        };

        // Shared library flag
        cmd.arg("-shared");

        // Add object file
        cmd.arg(&native_obj.path);

        // Add stdlib static library if available
        if let Some(ref artifacts) = self.stdlib_artifacts {
            if artifacts.static_library.exists() {
                cmd.arg("-Wl,--whole-archive");
                cmd.arg(&artifacts.static_library);
                cmd.arg("-Wl,--no-whole-archive");
            }
        }

        // Library search paths
        for path in &self.config.library_paths {
            cmd.arg(format!("-L{}", path.display()));
        }

        // V-LLSI: Apply no-libc flags for shared libraries
        if let Some(ref no_libc) = self.config.no_libc_config {
            // Add platform-specific no-libc flags
            for flag in &no_libc.flags {
                // Skip entry point flags for shared libraries
                if !flag.contains("-nostartfiles") {
                    cmd.arg(flag);
                }
            }

            // Add only allowed platform libraries
            for lib in &no_libc.libraries {
                if lib.starts_with('-') || lib.starts_with('/') {
                    cmd.arg(lib);
                } else if lib.ends_with(".lib") || lib.ends_with(".dll") {
                    cmd.arg(lib);
                } else {
                    cmd.arg(format!("-l{}", lib));
                }
            }

            debug!(
                "V-LLSI shared library: Platform={:?}, Libraries={:?}",
                no_libc.platform, no_libc.libraries
            );
        } else {
            // Standard mode: user-specified libraries
            for lib in &self.config.libraries {
                cmd.arg(format!("-l{}", lib.as_str()));
            }
        }

        // Export symbols (version script or export list)
        if !self.config.exported_symbols.is_empty() {
            let exports_file = self.create_exports_file()?;
            #[cfg(target_os = "macos")]
            {
                cmd.arg("-exported_symbols_list").arg(&exports_file);
            }
            #[cfg(not(target_os = "macos"))]
            {
                cmd.arg(format!("-Wl,--version-script={}", exports_file.display()));
            }
        }

        // Platform-specific flags for shared libraries
        #[cfg(target_os = "macos")]
        {
            cmd.arg("-dynamiclib");
        }

        // Position-independent code (required for shared libraries)
        cmd.arg("-fPIC");

        // Strip symbols if requested
        if self.config.strip {
            cmd.arg("-s");
        }

        // Debug info
        if self.config.debug_info {
            cmd.arg("-g");
        }

        // Additional flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag.as_str());
        }

        // Output path
        cmd.arg("-o").arg(&self.config.output_path);

        let output = cmd
            .output()
            .context("Failed to execute shared library linker")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Shared library linking failed: {}", stderr);
        }

        let elapsed = start.elapsed();
        self.stats.system_link_time_ms = elapsed.as_millis() as u64;

        let binary = Binary::from_path(&self.config.output_path)?;
        self.stats.binary_size = binary.size;

        info!(
            "Shared library linked: {} ({:.2} MB) in {:.2}s",
            self.config.output_path.display(),
            binary.size as f64 / 1_048_576.0,
            elapsed.as_secs_f64()
        );

        Ok(binary)
    }

    /// Create an exports file for shared library symbol visibility
    fn create_exports_file(&self) -> Result<PathBuf> {
        let exports_path = self.get_temp_path("exports.txt");
        let mut content = String::new();

        #[cfg(target_os = "macos")]
        {
            // macOS uses a simple list format
            for sym in &self.config.exported_symbols {
                content.push_str(&format!("_{}\n", sym.as_str()));
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Linux uses version script format
            content.push_str("{\n  global:\n");
            for sym in &self.config.exported_symbols {
                content.push_str(&format!("    {};\n", sym.as_str()));
            }
            content.push_str("  local:\n    *;\n};\n");
        }

        std::fs::write(&exports_path, content).context("Failed to write exports file")?;

        Ok(exports_path)
    }

    /// Static linking with LTO (for AOT tier)
    ///
    /// This handles both executables and shared libraries based on output_kind.
    fn static_link_with_lto(&mut self, object_files: &[ObjectFile]) -> Result<Binary> {
        // Collect statistics
        self.stats.object_files_linked = object_files.len();
        self.stats.total_object_size = object_files.iter().map(|o| o.size).sum();
        self.stats.lto_mode = Some(self.config.lto);

        match self.config.lto {
            LTOConfig::None => {
                // Direct system linking without LTO
                match self.config.output_kind {
                    OutputKind::SharedLibrary => {
                        // For shared library without LTO, merge objects first then link
                        let merged = self.create_merged_object_internal(object_files)?;
                        self.link_shared_library(&merged)
                    }
                    _ => self.system_link_direct(object_files),
                }
            }

            LTOConfig::Thin | LTOConfig::Full => {
                // Full LTO pipeline with CBGR runtime integration
                self.lto_link_pipeline(object_files)
            }
        }
    }

    /// Create merged object without updating stats (internal helper)
    fn create_merged_object_internal(&mut self, object_files: &[ObjectFile]) -> Result<ObjectFile> {
        if object_files.is_empty() {
            bail!("No object files provided for merging");
        }

        if object_files.len() == 1 {
            return Ok(object_files[0].clone());
        }

        let output_path = self.get_temp_path("merged_internal.o");

        // Use ld -r to create a relocatable merge
        let mut cmd = Command::new("ld");
        cmd.arg("-r"); // Relocatable output
        cmd.arg("-o").arg(&output_path);

        for obj in object_files {
            cmd.arg(&obj.path);
        }

        let output = cmd.output().context("Failed to execute ld -r")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Object merging failed: {}", stderr);
        }

        ObjectFile::from_path(output_path)
    }

    /// LTO linking pipeline
    ///
    /// This is the main AOT linking pipeline that handles:
    /// - Bitcode merging and LTO optimization
    /// - Final system linking (for both executables and shared libraries)
    ///
    /// CBGR runtime is built into stdlib via VBC intrinsics.
    fn lto_link_pipeline(&mut self, object_files: &[ObjectFile]) -> Result<Binary> {
        info!(
            "Running {:?} LTO pipeline (output: {:?})",
            self.config.lto, self.config.output_kind
        );

        // Step 1: Merge all LLVM bitcode files
        let merged_bc = self.merge_bitcode(object_files)?;

        // Step 2: Link with stdlib bitcode if available
        let combined_bc = self.link_stdlib_bitcode(&merged_bc)?;

        // Step 3: Apply LTO optimizations
        let optimized_bc = self.apply_lto(&combined_bc)?;

        // Step 4: Generate final native code
        let native_obj = self.generate_native(&optimized_bc)?;

        // Step 5: System linking
        match self.config.output_kind {
            OutputKind::SharedLibrary => {
                self.link_shared_library(&native_obj)
            }
            OutputKind::Executable => {
                self.system_link_direct(std::slice::from_ref(&native_obj))
            }
            _ => {
                // Static library and object file are handled earlier
                unreachable!(
                    "OutputKind::{:?} should not reach LTO pipeline",
                    self.config.output_kind
                )
            }
        }
    }

    /// Merge bitcode from all object files
    fn merge_bitcode(&mut self, object_files: &[ObjectFile]) -> Result<Bitcode> {
        let start = Instant::now();

        debug!("Merging bitcode from {} object files", object_files.len());

        let output_path = self.get_temp_path("merged.bc");

        let mut cmd = Command::new("llvm-link");

        // Add all bitcode files
        for obj in object_files {
            if obj.has_bitcode {
                cmd.arg(&obj.path);
            }
        }

        cmd.arg("-o").arg(&output_path);

        let output = cmd.output().context("Failed to execute llvm-link")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Bitcode merge failed: {}", stderr);
        }

        let elapsed = start.elapsed();
        self.stats.bitcode_merge_time_ms = elapsed.as_millis() as u64;

        debug!("Bitcode merged in {:.2}ms", elapsed.as_millis());

        Bitcode::from_file(output_path)
    }

    /// Link user bitcode with stdlib bitcode
    fn link_stdlib_bitcode(&mut self, user_bc: &Bitcode) -> Result<Bitcode> {
        let stdlib_bc = match &self.stdlib_artifacts {
            Some(artifacts) => &artifacts.bitcode_library,
            None => {
                warn!("No stdlib artifacts available, skipping stdlib linking");
                return Ok(user_bc.clone());
            }
        };

        debug!("Linking with stdlib bitcode: {}", stdlib_bc.display());

        let output_path = self.get_temp_path("combined.bc");

        let mut cmd = Command::new("llvm-link");
        cmd.arg(&user_bc.path)
            .arg(stdlib_bc)
            .arg("-o")
            .arg(&output_path);

        let output = cmd.output().context("Failed to link stdlib bitcode")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Stdlib bitcode linking failed: {}", stderr);
        }

        Bitcode::from_file(output_path)
    }

    /// Apply LTO optimizations
    fn apply_lto(&mut self, bitcode: &Bitcode) -> Result<Bitcode> {
        let start = Instant::now();

        info!("Applying {:?} LTO optimizations", self.config.lto);

        let output_path = self.get_temp_path("optimized.bc");

        let mut cmd = Command::new("opt");
        cmd.arg(&bitcode.path);

        // Optimization level based on LTO mode
        match self.config.lto {
            LTOConfig::Thin => {
                cmd.args(&["-O2", "-flto=thin"]);
            }
            LTOConfig::Full => {
                cmd.args(&["-O3", "-flto=full"]);
                cmd.args(&["-inline", "-inline-threshold=225"]);
            }
            LTOConfig::None => {
                cmd.arg("-O1");
            }
        }

        // Standard link-time optimizations
        cmd.args(&[
            "-std-link-opts", // Standard link-time opts
            "-internalize",   // Mark non-exported as internal
            "-globaldce",     // Remove dead globals
            "-constmerge",    // Merge duplicate constants
            "-mergefunc",     // Merge identical functions
            "-cross-dso-cfi", // Cross-DSO CFI
        ]);

        // Devirtualization for better performance
        if self.config.lto == LTOConfig::Full {
            cmd.arg("-whole-program-vtables");
        }

        cmd.arg("-o").arg(&output_path);

        let output = cmd.output().context("Failed to execute opt")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("LTO optimization failed: {}", stderr);
        }

        let elapsed = start.elapsed();
        self.stats.lto_optimization_time_ms = elapsed.as_millis() as u64;

        // Estimate optimizations (rough heuristic based on size reduction)
        let original_size = bitcode.size;
        let optimized = Bitcode::from_file(&output_path)?;
        let size_reduction = original_size.saturating_sub(optimized.size);

        self.stats.dead_code_eliminated = size_reduction;
        self.stats.functions_inlined = size_reduction / 100; // Rough estimate

        info!(
            "LTO complete: {:.2}ms, eliminated {:.2} KB dead code",
            elapsed.as_millis(),
            size_reduction as f64 / 1024.0
        );

        Ok(optimized)
    }

    /// Generate native object file from optimized bitcode
    fn generate_native(&mut self, bitcode: &Bitcode) -> Result<ObjectFile> {
        debug!("Generating native code from optimized bitcode");

        let output_path = self.get_temp_path("optimized.o");

        let mut cmd = Command::new("llc");
        cmd.arg(&bitcode.path)
            .arg("-filetype=obj")
            .arg("-O3")
            .arg("-o")
            .arg(&output_path);

        let output = cmd.output().context("Failed to execute llc")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Native code generation failed: {}", stderr);
        }

        let size = std::fs::metadata(&output_path)?.len() as usize;

        Ok(ObjectFile {
            path: output_path,
            module_name: "optimized".into(),
            size,
            has_bitcode: false,
            defined_symbols: List::new(),
            undefined_symbols: List::new(),
        })
    }

    /// Direct system linking without LTO
    fn system_link_direct(&mut self, object_files: &[ObjectFile]) -> Result<Binary> {
        let start = Instant::now();

        // V-LLSI: Check if we're using no-libc linking
        let use_no_libc = self.config.no_libc_config.is_some();

        if use_no_libc {
            info!("Direct system linking (V-LLSI no-libc mode)");
        } else {
            info!("Direct system linking (standard mode)");
        }

        let mut cmd = Command::new("cc");

        // Honour `LinkingConfig.use_llvm_linker` in the
        // executable / static-archive linking path. Pre-fix the
        // field was only consulted by `system_link_dynamic` (the
        // shared-library path at line ~963), so a manifest setting
        // like `[linker] use_lld = true` had no effect on
        // executables — `system_link_direct` always invoked
        // whatever linker `cc` defaults to. Pass `-fuse-ld=lld`
        // when the flag is set; the cc driver forwards it to the
        // underlying linker selection. Fail-soft: if lld isn't
        // installed, cc will error out with its own diagnostic
        // and the user sees the gap directly.
        if self.config.use_llvm_linker {
            cmd.arg("-fuse-ld=lld");
        }

        // V-LLSI: Apply no-libc flags first (must come before object files)
        if let Some(ref no_libc) = self.config.no_libc_config {
            for flag in &no_libc.flags {
                cmd.arg(flag);
            }

            // Set custom entry point
            if !no_libc.entry_point.is_empty() {
                match no_libc.platform {
                    Platform::Windows => {
                        cmd.arg(format!("/ENTRY:{}", no_libc.entry_point));
                    }
                    _ => {
                        cmd.arg("-e").arg(&no_libc.entry_point);
                    }
                }
            }

            debug!(
                "V-LLSI: Platform={:?}, Entry={}, Libraries={:?}",
                no_libc.platform, no_libc.entry_point, no_libc.libraries
            );
        }

        // Add all object files
        for obj in object_files {
            cmd.arg(&obj.path);
        }

        // Add stdlib if available
        if let Some(ref artifacts) = self.stdlib_artifacts {
            cmd.arg(&artifacts.static_library);
        }

        // Library search paths
        for path in &self.config.library_paths {
            cmd.arg(format!("-L{}", path.display()));
        }

        // V-LLSI: Add only the allowed platform libraries
        if let Some(ref no_libc) = self.config.no_libc_config {
            for lib in &no_libc.libraries {
                // Handle different library specification formats
                if lib.starts_with('-') || lib.starts_with('/') {
                    // Already a linker flag (e.g., "-lSystem" or "/NODEFAULTLIB")
                    cmd.arg(lib);
                } else if lib.ends_with(".lib") || lib.ends_with(".dll") {
                    // Windows library file
                    cmd.arg(lib);
                } else {
                    // Standard library name
                    cmd.arg(format!("-l{}", lib));
                }
            }
        } else {
            // Standard mode: user-specified libraries
            for lib in &self.config.libraries {
                cmd.arg(format!("-l{}", lib.as_str()));
            }

            // Platform-specific defaults (only in standard mode)
            #[cfg(target_os = "linux")]
            cmd.arg("-lrt");

            #[cfg(target_os = "macos")]
            cmd.args(&["-framework", "CoreFoundation"]);
        }

        if self.config.pic {
            cmd.arg("-fPIC");
        }

        if self.config.strip {
            cmd.arg("-s");
        } else if self.config.strip_debug_only {
            // Strip debug symbols only — keeps function names for
            // backtraces while shedding the bulk of debug info.
            // Pre-fix `strip_debug_only` was a config field with no
            // readers; wiring honours the verum.toml `[link]
            // strip_debug_only = true` knob that documentation
            // already advertised.
            cmd.arg("-Wl,--strip-debug");
        }

        if self.config.debug_info {
            // `-g` retains DWARF / Mach-O debug sections through
            // linking so a debugger can map back to source.  Skipped
            // when `strip` already discards everything.
            cmd.arg("-g");
        }

        if self.config.static_link {
            // Static linking — no runtime dependencies on shared
            // libraries.  Pre-fix this flag was inert; macOS users
            // get a friendly diagnostic from `cc` if they request
            // `-static` (Apple actively rejects fully-static linking
            // against libSystem).
            cmd.arg("-static");
        }

        // `target_triple` overrides the host target.  When unset
        // (the default), the system `cc` picks its native triple.
        if let Some(ref triple) = self.config.target_triple {
            cmd.arg(format!("--target={}", triple.as_str()));
        }

        // `entry_point` overrides the default `main` symbol.  Only
        // honoured when `no_libc_config` is None, because no-libc
        // mode owns its own entry-point handling above (see line
        // ~1404).
        if self.config.no_libc_config.is_none() {
            if let Some(ref entry) = self.config.entry_point {
                if entry.as_str() != "main" && !entry.is_empty() {
                    cmd.arg("-e").arg(entry.as_str());
                }
            }
        }

        // Additional user flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag.as_str());
        }

        cmd.arg("-o").arg(&self.config.output_path);

        debug!("Linker command: {:?}", cmd);

        let output = cmd.output().context("Failed to execute system linker")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Direct system linking failed: {}", stderr);
        }

        let elapsed = start.elapsed();
        self.stats.system_link_time_ms = elapsed.as_millis() as u64;

        let binary = Binary::from_path(&self.config.output_path)?;
        self.stats.binary_size = binary.size;

        info!(
            "V-LLSI linking complete: {} ({} bytes)",
            binary.path.display(),
            binary.size
        );

        Ok(binary)
    }

    /// Get temporary directory
    fn get_temp_dir(&self) -> PathBuf {
        let temp_dir = std::env::temp_dir().join("verum_linking");
        std::fs::create_dir_all(&temp_dir).ok();
        temp_dir
    }

    /// Get temporary file path
    fn get_temp_path(&self, filename: &str) -> PathBuf {
        self.get_temp_dir().join(filename)
    }

    /// Get linking statistics
    pub fn get_stats(&self) -> &LinkingStats {
        &self.stats
    }
}

// =============================================================================
// CompilationPhase Implementation
// =============================================================================

impl CompilationPhase for FinalLinker {
    fn name(&self) -> &str {
        "Phase 7.5: Final Linking"
    }

    fn description(&self) -> &str {
        "Link object files with stdlib and apply LTO optimizations"
    }

    fn execute(
        &self,
        _input: PhaseInput,
    ) -> Result<PhaseOutput, List<verum_diagnostics::Diagnostic>> {
        // Note: This phase is typically invoked directly via link() method
        // rather than through the generic CompilationPhase interface

        let metrics =
            PhaseMetrics::new(self.name()).with_items_processed(self.stats.object_files_linked);

        Ok(PhaseOutput {
            data: PhaseData::SourceFiles(List::new()),
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        false // Linking must be serial
    }

    fn metrics(&self) -> PhaseMetrics {
        let mut metrics = PhaseMetrics::new(self.name());
        metrics.duration = std::time::Duration::from_millis(self.stats.total_link_time_ms);
        metrics.items_processed = self.stats.object_files_linked;
        metrics.add_custom_metric(
            "binary_size_mb",
            format!("{:.2}", self.stats.binary_size as f64 / 1_048_576.0),
        );
        if let Some(lto) = self.stats.lto_mode {
            metrics.add_custom_metric("lto_mode", format!("{:?}", lto));
        }
        metrics
    }
}

// =============================================================================
// Public API
// =============================================================================

/// Create a linker with default configuration
pub fn create_linker(tier: ExecutionTier) -> FinalLinker {
    FinalLinker::new(tier, LinkingConfig::default())
}

/// Create a linker with custom configuration
pub fn create_linker_with_config(tier: ExecutionTier, config: LinkingConfig) -> FinalLinker {
    FinalLinker::new(tier, config)
}

/// Quick link with default settings
pub fn quick_link(
    tier: ExecutionTier,
    object_files: List<ObjectFile>,
    output_path: PathBuf,
) -> Result<Binary> {
    let mut config = LinkingConfig::default();
    config.output_path = output_path;
    config.lto = LTOConfig::None;

    let mut linker = FinalLinker::new(tier, config);
    linker.link(object_files)
}

/// Release link with full LTO
pub fn release_link(
    tier: ExecutionTier,
    object_files: List<ObjectFile>,
    output_path: PathBuf,
    stdlib_artifacts: StdlibArtifacts,
) -> Result<Binary> {
    let mut config = LinkingConfig::default();
    config.output_path = output_path;
    config.lto = LTOConfig::Full;
    config.strip = true;

    let mut linker = FinalLinker::new(tier, config).with_core_artifacts(stdlib_artifacts);

    linker.link(object_files)
}
