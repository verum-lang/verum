//! Linking Infrastructure for Verum
//!
//! This module provides a unified API for linking Verum programs,
//! supporting multiple output formats and link-time optimization.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      verum_codegen::link                    │
//! │  ┌──────────────┐  ┌─────────────┐  ┌──────────────────┐   │
//! │  │ LinkSession  │──│  LtoEngine  │──│ LinkerBackend    │   │
//! │  │              │  │             │  │ (LLD/System)     │   │
//! │  └──────────────┘  └─────────────┘  └──────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//!           │                 │                   │
//!           ▼                 ▼                   ▼
//!     Object Files      LLVM Bitcode         Native Binary
//! ```
//!
//! # Example
//!
//! ```no_run
//! use verum_codegen::link::{LinkSession, OutputFormat};
//!
//! let session = LinkSession::new()
//!     .output_format(OutputFormat::Executable)
//!     .output("program")
//!     .add_object("main.o")
//!     .add_object("lib.o")
//!     .enable_lto()
//!     .strip_symbols()
//!     .build()?
//!     .link()?;
//! # Ok::<(), verum_codegen::link::LinkError>(())
//! ```

use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{info, warn};

pub use verum_llvm_sys::lld::{LinkerFlavor, LinkerResult};
pub use verum_llvm::lto::{LtoConfig, LtoMode, ThinLtoCache};

/// Link errors
#[derive(Debug, Error)]
pub enum LinkError {
    #[error("Linking failed: {message}")]
    LinkFailed { message: String },

    #[error("LTO failed: {message}")]
    LtoFailed { message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Missing input: no object files provided")]
    NoInputs,

    #[error("Missing output: output path not specified")]
    NoOutput,

    #[error("Backend error: {0}")]
    Backend(String),

    #[error("LLD error: {0}")]
    Lld(#[from] verum_llvm_sys::lld::LinkerError),

    #[error("LLVM error: {0}")]
    Llvm(#[from] verum_llvm::error::LlvmError),
}

/// Result type for link operations
pub type LinkResult<T> = Result<T, LinkError>;

// =============================================================================
// Target Platform for No-Libc Linking
// =============================================================================

/// Target platform for linking.
///
/// Verum uses platform-specific no-libc linking:
/// - **Linux**: Direct syscalls (stable ABI) - no libraries
/// - **macOS**: libSystem.B.dylib only (Apple requirement)
/// - **Windows**: ntdll.dll + kernel32.dll only (no MSVC CRT)
/// - **FreeBSD**: Direct syscalls (stable ABI) - no libraries
/// - **Embedded**: Bare-metal, no OS
///
/// V-LLSI (Verum Low-Level System Interface) Architecture:
/// Verum uses a self-hosted, no-libc runtime architecture with platform-specific
/// system call layers. Each platform has its own syscall/library strategy:
/// - Linux: Direct syscalls via stable ABI (no libraries needed)
/// - macOS: libSystem.B.dylib only (Apple prohibits direct syscalls)
/// - Windows: ntdll.dll + kernel32.dll only (no MSVC/UCRT dependency)
/// - FreeBSD: Direct syscalls via stable ABI
/// - Embedded: Bare-metal, no OS or system libraries
/// All runtime functionality is provided by LLVM intrinsics, core/ stdlib,
/// and platform-specific sys/ modules instead of libc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Platform {
    /// Linux (direct syscalls via stable ABI)
    #[default]
    Linux,
    /// macOS (libSystem.B.dylib only - Apple prohibits direct syscalls)
    MacOS,
    /// Windows (ntdll.dll + kernel32.dll only, no MSVC/UCRT)
    Windows,
    /// FreeBSD (direct syscalls)
    FreeBSD,
    /// WebAssembly with WASI (WebAssembly System Interface)
    /// Supports I/O, filesystem, clocks via WASI imports.
    WasmWasi,
    /// WebAssembly without WASI (pure embedded/browser)
    /// No system interface — host provides functionality via @wasm_import.
    WasmEmbedded,
    /// Embedded/bare-metal (no OS, no system libraries)
    Embedded,
}

impl Platform {
    /// Get platform from target triple string.
    pub fn from_triple(triple: &str) -> Option<Self> {
        if triple.contains("wasm") {
            // WASM targets: distinguish WASI from embedded
            if triple.contains("wasi") {
                Some(Platform::WasmWasi)
            } else {
                Some(Platform::WasmEmbedded)
            }
        } else if triple.contains("linux") {
            Some(Platform::Linux)
        } else if triple.contains("darwin") || triple.contains("macos") {
            Some(Platform::MacOS)
        } else if triple.contains("windows") {
            Some(Platform::Windows)
        } else if triple.contains("freebsd") {
            Some(Platform::FreeBSD)
        } else if triple.contains("none") || triple.contains("unknown-unknown") {
            Some(Platform::Embedded)
        } else {
            None
        }
    }

    /// Get platform name.
    pub fn name(&self) -> &str {
        match self {
            Platform::Linux => "linux",
            Platform::MacOS => "macos",
            Platform::Windows => "windows",
            Platform::FreeBSD => "freebsd",
            Platform::WasmWasi => "wasm-wasi",
            Platform::WasmEmbedded => "wasm",
            Platform::Embedded => "embedded",
        }
    }

    /// Detect platform from host.
    pub fn host() -> Self {
        #[cfg(target_os = "linux")]
        { Platform::Linux }
        #[cfg(target_os = "macos")]
        { Platform::MacOS }
        #[cfg(target_os = "windows")]
        { Platform::Windows }
        #[cfg(target_os = "freebsd")]
        { Platform::FreeBSD }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd"
        )))]
        { Platform::Linux } // Default to Linux for unknown platforms
    }
}

// =============================================================================
// No-Libc Linking Configuration (V-LLSI Architecture)
// =============================================================================

/// Platform-specific linking configuration for no-libc builds.
///
/// Verum uses a fully self-contained runtime without libc dependency.
/// All runtime functionality is provided by:
/// - LLVM intrinsics for math (llvm.sin.f32, llvm.sqrt.f64, etc.)
/// - /core/ for runtime (threads, memory, I/O)
/// - /core/sys/ for platform-specific syscalls
///
/// V-LLSI Architecture - No-libc linking:
/// Verum compiles with a fully self-contained runtime. System functionality
/// comes from LLVM intrinsics (math ops), core/ stdlib (threads, memory, I/O),
/// and core/sys/ platform-specific syscall wrappers. No libc or C runtime
/// is linked. Platform-specific libraries are minimal:
/// - Linux: none (raw syscalls)
/// - macOS: libSystem.B.dylib (Apple requirement)
/// - Windows: ntdll.dll + kernel32.dll
/// - Embedded: none (bare-metal)
#[derive(Debug, Clone)]
pub struct NoLibcConfig {
    /// Target platform
    pub platform: Platform,
    /// Entry point symbol
    pub entry_point: String,
    /// Libraries to link (only system libraries, not libc)
    pub libraries: Vec<String>,
    /// Linker flags
    pub flags: Vec<String>,
}

impl NoLibcConfig {
    /// Create Linux no-libc linking configuration.
    ///
    /// Linux uses direct syscalls via the stable kernel ABI.
    /// Entry point: _start (from /core/sys/init.vr)
    /// No external libraries required.
    pub fn linux() -> Self {
        Self {
            platform: Platform::Linux,
            entry_point: "_start".to_string(),
            libraries: vec![
                // NO libc, NO libm, NO pthread
                // All functionality via direct syscalls
            ],
            flags: vec![
                "-nostdlib".to_string(),
                "-nostartfiles".to_string(),
                "--gc-sections".to_string(),
            ],
        }
    }

    /// Create macOS no-libc linking configuration.
    ///
    /// macOS prohibits direct syscalls - must use libSystem.B.dylib.
    /// This is the minimal system library on macOS.
    /// Entry point: main (via dyld)
    pub fn macos() -> Self {
        Self {
            platform: Platform::MacOS,
            entry_point: "main".to_string(),
            libraries: vec![
                // Only libSystem - minimal system library
                "System".to_string(), // libSystem.B.dylib
            ],
            flags: vec![
                "-nostdlib".to_string(),
                // Link only with libSystem
                "-lSystem".to_string(),
                // Metal GPU compute framework (Apple Silicon)
                "-framework".to_string(), "Metal".to_string(),
                "-framework".to_string(), "Foundation".to_string(),
                // Objective-C runtime (for Metal bridge)
                "-lobjc".to_string(),
            ],
        }
    }

    /// Create Windows no-libc linking configuration.
    ///
    /// Windows uses IAT imports from ntdll.dll and kernel32.dll only.
    /// Entry point: mainCRTStartup
    /// NO MSVC CRT, NO UCRT.
    pub fn windows() -> Self {
        Self {
            platform: Platform::Windows,
            entry_point: "mainCRTStartup".to_string(),
            libraries: vec![
                // NT Native API and basic kernel functions
                "ntdll".to_string(),
                "kernel32".to_string(),
                // NO msvcrt, NO ucrt, NO other CRT
            ],
            flags: vec![
                "/NODEFAULTLIB".to_string(),
                "/ENTRY:mainCRTStartup".to_string(),
                "/SUBSYSTEM:CONSOLE".to_string(),
                // Dead-code elimination for smaller binaries
                "/OPT:REF".to_string(),
                "/OPT:ICF".to_string(),
            ],
        }
    }

    /// Create Windows linking configuration for GUI (no console window).
    pub fn windows_gui() -> Self {
        let mut cfg = Self::windows();
        cfg.flags.retain(|f| !f.starts_with("/SUBSYSTEM:"));
        cfg.flags.push("/SUBSYSTEM:WINDOWS".to_string());
        cfg
    }

    /// Create FreeBSD no-libc linking configuration.
    ///
    /// FreeBSD uses direct syscalls similar to Linux.
    pub fn freebsd() -> Self {
        Self {
            platform: Platform::FreeBSD,
            entry_point: "_start".to_string(),
            libraries: vec![],
            flags: vec![
                "-nostdlib".to_string(),
                "-nostartfiles".to_string(),
                "--gc-sections".to_string(),
            ],
        }
    }

    /// Create embedded/bare-metal linking configuration.
    ///
    /// Embedded targets have no OS and no system libraries.
    /// Entry point is typically Reset_Handler or platform-specific.
    pub fn embedded() -> Self {
        Self {
            platform: Platform::Embedded,
            entry_point: "Reset_Handler".to_string(),
            libraries: vec![],
            flags: vec![
                "-nostdlib".to_string(),
                "-nostartfiles".to_string(),
                "-ffreestanding".to_string(),
                "--gc-sections".to_string(),
            ],
        }
    }

    /// Create WASM-WASI linking configuration.
    ///
    /// Uses WASI (WebAssembly System Interface) for I/O, filesystem, clocks.
    /// Entry point: _start (WASI convention).
    /// Linked with wasm-ld (LLVM LLD WASM flavor).
    pub fn wasm_wasi() -> Self {
        Self {
            platform: Platform::WasmWasi,
            entry_point: "_start".to_string(),
            libraries: vec![
                // No libraries — WASI imports are resolved by the runtime
            ],
            flags: vec![
                "--no-entry".to_string(), // For library modules; removed for executables
                "--export-all".to_string(),
                "--allow-undefined".to_string(), // WASI imports resolved at runtime
                "--gc-sections".to_string(),
            ],
        }
    }

    /// Create WASM embedded linking configuration.
    ///
    /// No WASI — pure WASM module for browser or custom host.
    /// Host provides functionality via @wasm_import.
    pub fn wasm_embedded() -> Self {
        Self {
            platform: Platform::WasmEmbedded,
            entry_point: "main".to_string(),
            libraries: vec![],
            flags: vec![
                "--no-entry".to_string(),
                "--export-all".to_string(),
                "--allow-undefined".to_string(),
                "--gc-sections".to_string(),
            ],
        }
    }

    /// Get no-libc configuration for the given platform.
    pub fn for_platform(platform: Platform) -> Self {
        match platform {
            Platform::Linux => Self::linux(),
            Platform::MacOS => Self::macos(),
            Platform::Windows => Self::windows(),
            Platform::FreeBSD => Self::freebsd(),
            Platform::WasmWasi => Self::wasm_wasi(),
            Platform::WasmEmbedded => Self::wasm_embedded(),
            Platform::Embedded => Self::embedded(),
        }
    }

    /// Get no-libc configuration for the host platform.
    pub fn for_host() -> Self {
        Self::for_platform(Platform::host())
    }
}

// =============================================================================
// Output Formats
// =============================================================================

/// Output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Executable binary
    #[default]
    Executable,
    /// Shared library (.so, .dylib, .dll)
    SharedLibrary,
    /// Static library (.a, .lib)
    StaticLibrary,
    /// Relocatable object file (.o)
    Object,
    /// WebAssembly module (.wasm)
    Wasm,
}

/// Input file type
#[derive(Debug, Clone)]
pub enum InputFile {
    /// Native object file (.o)
    Object(PathBuf),
    /// Static library (.a)
    Archive(PathBuf),
    /// LLVM bitcode (.bc) for LTO
    Bitcode(PathBuf),
    /// LLVM IR text (.ll)
    LlvmIr(PathBuf),
    /// Library by name (resolved via search paths)
    Library(String),
}

impl InputFile {
    pub fn object(path: impl AsRef<Path>) -> Self {
        Self::Object(path.as_ref().to_path_buf())
    }

    pub fn archive(path: impl AsRef<Path>) -> Self {
        Self::Archive(path.as_ref().to_path_buf())
    }

    pub fn bitcode(path: impl AsRef<Path>) -> Self {
        Self::Bitcode(path.as_ref().to_path_buf())
    }

    pub fn library(name: impl AsRef<str>) -> Self {
        Self::Library(name.as_ref().to_string())
    }
}

/// Link configuration
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// Output format
    pub output_format: OutputFormat,
    /// Output file path
    pub output: PathBuf,
    /// Input files
    pub inputs: Vec<InputFile>,
    /// Library search paths
    pub library_paths: Vec<PathBuf>,
    /// Entry point symbol (for executables)
    pub entry_point: Option<String>,
    /// Exported symbols
    pub exports: Vec<String>,
    /// Shared library soname
    pub soname: Option<String>,
    /// Strip symbols
    pub strip: bool,
    /// Strip debug info only
    pub strip_debug: bool,
    /// Position-independent executable
    pub pie: bool,
    /// Static linking
    pub static_link: bool,
    /// GC unused sections
    pub gc_sections: bool,
    /// Enable LTO
    pub lto: Option<LtoConfig>,
    /// Target triple
    pub target_triple: Option<String>,
    /// Linker script
    pub linker_script: Option<PathBuf>,
    /// rpaths
    pub rpaths: Vec<PathBuf>,
    /// Extra linker arguments
    pub extra_args: Vec<String>,
    /// Verbose output
    pub verbose: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            output_format: OutputFormat::Executable,
            output: PathBuf::new(),
            inputs: Vec::new(),
            library_paths: Vec::new(),
            entry_point: None,
            exports: Vec::new(),
            soname: None,
            strip: false,
            strip_debug: false,
            pie: false,
            static_link: false,
            gc_sections: true,
            lto: None,
            target_triple: None,
            linker_script: None,
            rpaths: Vec::new(),
            extra_args: Vec::new(),
            verbose: false,
        }
    }
}

/// Link session builder
#[derive(Default)]
pub struct LinkSession {
    config: LinkConfig,
}

impl LinkSession {
    /// Create new link session
    pub fn new() -> Self {
        Self::default()
    }

    /// Set output format
    pub fn output_format(mut self, format: OutputFormat) -> Self {
        self.config.output_format = format;
        self
    }

    /// Set output path
    pub fn output(mut self, path: impl AsRef<Path>) -> Self {
        self.config.output = path.as_ref().to_path_buf();
        self
    }

    /// Add object file
    pub fn add_object(mut self, path: impl AsRef<Path>) -> Self {
        self.config.inputs.push(InputFile::Object(path.as_ref().to_path_buf()));
        self
    }

    /// Add multiple object files
    pub fn add_objects<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for path in paths {
            self.config.inputs.push(InputFile::Object(path.as_ref().to_path_buf()));
        }
        self
    }

    /// Add static library archive
    pub fn add_archive(mut self, path: impl AsRef<Path>) -> Self {
        self.config.inputs.push(InputFile::Archive(path.as_ref().to_path_buf()));
        self
    }

    /// Add bitcode file for LTO
    pub fn add_bitcode(mut self, path: impl AsRef<Path>) -> Self {
        self.config.inputs.push(InputFile::Bitcode(path.as_ref().to_path_buf()));
        self
    }

    /// Add library by name
    pub fn add_library(mut self, name: impl AsRef<str>) -> Self {
        self.config.inputs.push(InputFile::Library(name.as_ref().to_string()));
        self
    }

    /// Add multiple libraries
    pub fn add_libraries<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            self.config.inputs.push(InputFile::Library(name.as_ref().to_string()));
        }
        self
    }

    /// Add library search path
    pub fn add_library_path(mut self, path: impl AsRef<Path>) -> Self {
        self.config.library_paths.push(path.as_ref().to_path_buf());
        self
    }

    /// Add multiple library search paths
    pub fn add_library_paths<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for path in paths {
            self.config.library_paths.push(path.as_ref().to_path_buf());
        }
        self
    }

    /// Set entry point
    pub fn entry_point(mut self, symbol: impl AsRef<str>) -> Self {
        self.config.entry_point = Some(symbol.as_ref().to_string());
        self
    }

    /// Add exported symbol
    pub fn export_symbol(mut self, symbol: impl AsRef<str>) -> Self {
        self.config.exports.push(symbol.as_ref().to_string());
        self
    }

    /// Set soname for shared library
    pub fn soname(mut self, name: impl AsRef<str>) -> Self {
        self.config.soname = Some(name.as_ref().to_string());
        self
    }

    /// Enable symbol stripping
    pub fn strip_symbols(mut self) -> Self {
        self.config.strip = true;
        self
    }

    /// Strip debug info only
    pub fn strip_debug_info(mut self) -> Self {
        self.config.strip_debug = true;
        self
    }

    /// Enable position-independent executable
    pub fn pie(mut self) -> Self {
        self.config.pie = true;
        self
    }

    /// Enable static linking
    pub fn static_link(mut self) -> Self {
        self.config.static_link = true;
        self
    }

    /// Disable GC of unused sections
    pub fn no_gc_sections(mut self) -> Self {
        self.config.gc_sections = false;
        self
    }

    /// Enable LTO with default configuration
    pub fn enable_lto(mut self) -> Self {
        self.config.lto = Some(LtoConfig::default());
        self
    }

    /// Enable ThinLTO
    pub fn enable_thin_lto(mut self) -> Self {
        self.config.lto = Some(LtoConfig::new(LtoMode::Thin));
        self
    }

    /// Enable ThinLTO with cache
    pub fn enable_thin_lto_with_cache(mut self, cache_dir: impl AsRef<Path>) -> Self {
        self.config.lto = Some(LtoConfig::thin_with_cache(cache_dir));
        self
    }

    /// Enable Full LTO
    pub fn enable_full_lto(mut self) -> Self {
        self.config.lto = Some(LtoConfig::new(LtoMode::Full));
        self
    }

    /// Set LTO configuration
    pub fn lto_config(mut self, config: LtoConfig) -> Self {
        self.config.lto = Some(config);
        self
    }

    /// Set target triple
    pub fn target(mut self, triple: impl AsRef<str>) -> Self {
        self.config.target_triple = Some(triple.as_ref().to_string());
        self
    }

    /// Set linker script
    pub fn linker_script(mut self, path: impl AsRef<Path>) -> Self {
        self.config.linker_script = Some(path.as_ref().to_path_buf());
        self
    }

    /// Add rpath
    pub fn rpath(mut self, path: impl AsRef<Path>) -> Self {
        self.config.rpaths.push(path.as_ref().to_path_buf());
        self
    }

    /// Add extra linker argument
    pub fn extra_arg(mut self, arg: impl AsRef<str>) -> Self {
        self.config.extra_args.push(arg.as_ref().to_string());
        self
    }

    /// Enable verbose output
    pub fn verbose(mut self) -> Self {
        self.config.verbose = true;
        self
    }

    // =========================================================================
    // No-Libc Configuration (V-LLSI Architecture)
    // =========================================================================

    /// Apply no-libc linking configuration for the target platform.
    ///
    /// This configures the linker to produce a self-contained binary without
    /// libc dependency. Platform-specific behavior:
    ///
    /// - **Linux**: Direct syscalls, no libraries
    /// - **macOS**: libSystem.B.dylib only (Apple requirement)
    /// - **Windows**: ntdll.dll + kernel32.dll only (no CRT)
    /// - **FreeBSD**: Direct syscalls, no libraries
    /// - **Embedded**: Bare-metal, no OS
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_codegen::link::{LinkSession, Platform, OutputFormat};
    ///
    /// let session = LinkSession::new()
    ///     .output_format(OutputFormat::Executable)
    ///     .output("program")
    ///     .add_object("main.o")
    ///     .with_no_libc(Platform::Linux)
    ///     .build()?
    ///     .link()?;
    /// # Ok::<(), verum_codegen::link::LinkError>(())
    /// ```
    pub fn with_no_libc(self, platform: Platform) -> Self {
        self.apply_no_libc_config(NoLibcConfig::for_platform(platform))
    }

    /// Apply no-libc configuration for the host platform.
    pub fn with_no_libc_host(self) -> Self {
        self.apply_no_libc_config(NoLibcConfig::for_host())
    }

    /// Apply a custom no-libc configuration.
    pub fn apply_no_libc_config(mut self, config: NoLibcConfig) -> Self {
        // Set entry point
        self.config.entry_point = Some(config.entry_point);

        // Enable static linking (no runtime library resolution)
        self.config.static_link = true;

        // Add system libraries (minimal set per platform)
        for lib in config.libraries {
            self.config.inputs.push(InputFile::Library(lib));
        }

        // Add linker flags
        for flag in config.flags {
            self.config.extra_args.push(flag);
        }

        // Store target info if not already set
        if self.config.target_triple.is_none() {
            self.config.target_triple = Some(match config.platform {
                Platform::Linux => "x86_64-unknown-linux-gnu".to_string(),
                Platform::MacOS => "x86_64-apple-darwin".to_string(),
                Platform::Windows => "x86_64-pc-windows-msvc".to_string(),
                Platform::FreeBSD => "x86_64-unknown-freebsd".to_string(),
                Platform::WasmWasi => "wasm32-wasi".to_string(),
                Platform::WasmEmbedded => "wasm32-unknown-unknown".to_string(),
                Platform::Embedded => "arm-none-eabi".to_string(),
            });
        }

        self
    }

    /// Build and validate the link session
    pub fn build(self) -> LinkResult<PreparedLink> {
        if self.config.inputs.is_empty() {
            return Err(LinkError::NoInputs);
        }

        if self.config.output.as_os_str().is_empty() {
            return Err(LinkError::NoOutput);
        }

        Ok(PreparedLink {
            config: self.config,
        })
    }
}

/// Prepared link session ready for execution
pub struct PreparedLink {
    config: LinkConfig,
}

impl PreparedLink {
    /// Get the configuration
    pub fn config(&self) -> &LinkConfig {
        &self.config
    }

    /// Perform the link
    pub fn link(self) -> LinkResult<LinkOutput> {
        info!("Starting link: {} inputs -> {}",
              self.config.inputs.len(),
              self.config.output.display());

        if self.config.lto.is_some() {
            return self.link_with_lto();
        }

        self.link_with_lld()
    }

    /// Link using LLD
    fn link_with_lld(self) -> LinkResult<LinkOutput> {
        use verum_llvm_sys::lld::Linker;

        let flavor = self.determine_linker_flavor();
        let mut linker = Linker::new(flavor);

        // Set output
        linker = linker.output(&self.config.output);

        // Output format
        match self.config.output_format {
            OutputFormat::SharedLibrary => {
                linker = linker.shared();
                if let Some(ref name) = self.config.soname {
                    linker = linker.soname(name);
                }
            }
            OutputFormat::Executable if self.config.pie => {
                linker = linker.pie();
            }
            _ => {}
        }

        // Static linking
        if self.config.static_link {
            linker = linker.static_link();
        }

        // GC sections
        if self.config.gc_sections {
            linker = linker.gc_sections();
        }

        // Strip
        if self.config.strip {
            linker = linker.strip();
        } else if self.config.strip_debug {
            linker = linker.strip_debug();
        }

        // Entry point
        if let Some(ref entry) = self.config.entry_point {
            linker = linker.entry(entry);
        }

        // Library paths
        for path in &self.config.library_paths {
            linker = linker.add_library_path(path);
        }

        // rpaths
        for path in &self.config.rpaths {
            linker = linker.rpath(path);
        }

        // Linker script
        if let Some(ref script) = self.config.linker_script {
            linker = linker.linker_script(script);
        }

        // Exports
        for symbol in &self.config.exports {
            linker = linker.export_symbol(symbol);
        }

        // Input files
        for input in &self.config.inputs {
            match input {
                InputFile::Object(path) | InputFile::Archive(path) => {
                    linker = linker.add_object(path);
                }
                InputFile::Library(name) => {
                    linker = linker.add_library(name);
                }
                InputFile::Bitcode(_) | InputFile::LlvmIr(_) => {
                    warn!("Bitcode/IR files ignored without LTO enabled");
                }
            }
        }

        // Extra args
        for arg in &self.config.extra_args {
            linker = linker.arg(arg);
        }

        // Verbose
        if self.config.verbose {
            linker = linker.verbose();
        }

        // Execute link
        let result = linker.link()?;

        Ok(LinkOutput {
            output_path: self.config.output.clone(),
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    /// Determine linker flavor based on config
    fn determine_linker_flavor(&self) -> LinkerFlavor {
        if let Some(ref triple) = self.config.target_triple {
            if triple.contains("linux") || triple.contains("freebsd") {
                return LinkerFlavor::Elf;
            }
            if triple.contains("darwin") || triple.contains("macos") {
                return LinkerFlavor::MachO;
            }
            if triple.contains("windows") {
                return LinkerFlavor::Coff;
            }
            if triple.contains("wasm") {
                return LinkerFlavor::Wasm;
            }
        }

        match self.config.output_format {
            OutputFormat::Wasm => LinkerFlavor::Wasm,
            _ => LinkerFlavor::native(),
        }
    }

    /// Link with LTO
    fn link_with_lto(self) -> LinkResult<LinkOutput> {
        use verum_llvm::lto::lto_compile;

        let lto_config = self.config.lto.clone().unwrap_or_default();

        info!("Performing {:?} LTO on {} inputs", lto_config.mode, self.config.inputs.len());

        // Collect bitcode files
        let mut bitcode_files = Vec::new();
        let mut object_files = Vec::new();
        let mut temp_dir = None;

        for input in &self.config.inputs {
            match input {
                InputFile::Bitcode(path) => {
                    let data = std::fs::read(path)?;
                    bitcode_files.push(data);
                }
                InputFile::LlvmIr(path) => {
                    let ir = std::fs::read_to_string(path)?;
                    let ctx = verum_llvm::context::Context::create();
                    let mem_buf = verum_llvm::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                        ir.as_bytes(),
                        path.to_string_lossy().as_ref(),
                    );
                    let module = ctx.create_module_from_ir(mem_buf)
                        .map_err(|e| LinkError::Backend(format!("Failed to parse LLVM IR: {}", e.to_string())))?;
                    let bc = module.write_bitcode_to_memory();
                    bitcode_files.push(bc.as_slice().to_vec());
                }
                InputFile::Object(path) | InputFile::Archive(path) => {
                    object_files.push(path.clone());
                }
                InputFile::Library(_) => {}
            }
        }

        // Perform LTO if we have bitcode
        if !bitcode_files.is_empty() {
            let bc_refs: Vec<&[u8]> = bitcode_files.iter().map(|v| v.as_slice()).collect();
            let lto_objects = lto_compile(&bc_refs, &lto_config)?;

            let dir = tempfile::tempdir()?;
            for (i, obj) in lto_objects.iter().enumerate() {
                let path = dir.path().join(format!("lto_{}.o", i));
                std::fs::write(&path, obj)?;
                object_files.push(path);
            }
            temp_dir = Some(dir);
        }

        // Link with LLD
        let flavor = self.determine_linker_flavor();
        let mut linker = verum_llvm_sys::lld::Linker::new(flavor);

        linker = linker.output(&self.config.output);

        match self.config.output_format {
            OutputFormat::SharedLibrary => {
                linker = linker.shared();
                if let Some(ref name) = self.config.soname {
                    linker = linker.soname(name);
                }
            }
            OutputFormat::Executable if self.config.pie => {
                linker = linker.pie();
            }
            _ => {}
        }

        if self.config.gc_sections {
            linker = linker.gc_sections();
        }

        if self.config.strip {
            linker = linker.strip();
        }

        for path in &self.config.library_paths {
            linker = linker.add_library_path(path);
        }

        for obj in &object_files {
            linker = linker.add_object(obj);
        }

        for input in &self.config.inputs {
            if let InputFile::Library(name) = input {
                linker = linker.add_library(name);
            }
        }

        let result = linker.link()?;

        // Keep temp_dir alive until after linking
        drop(temp_dir);

        Ok(LinkOutput {
            output_path: self.config.output.clone(),
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }
}

/// Output of a link operation
#[derive(Debug)]
pub struct LinkOutput {
    /// Path to the output file
    pub output_path: PathBuf,
    /// Standard output from linker
    pub stdout: String,
    /// Standard error from linker
    pub stderr: String,
}

/// Initialize the linker subsystem
pub fn init() {
    verum_llvm_sys::lld::init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_session_builder() {
        let result = LinkSession::new()
            .output_format(OutputFormat::Executable)
            .output("test_program")
            .add_object("main.o")
            .add_library("c")
            .strip_symbols()
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_inputs() {
        let result = LinkSession::new()
            .output("test")
            .build();

        assert!(matches!(result, Err(LinkError::NoInputs)));
    }

    #[test]
    fn test_missing_output() {
        let result = LinkSession::new()
            .add_object("test.o")
            .build();

        assert!(matches!(result, Err(LinkError::NoOutput)));
    }
}
