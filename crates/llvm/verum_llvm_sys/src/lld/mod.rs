//! verum_lld - Linker Bindings for Verum Compiler
//!
//! This crate provides linker functionality for the Verum compiler.
//! It supports two modes:
//!
//! 1. **embedded-lld**: Uses LLVM's LLD linker directly (requires LLD dev headers)
//! 2. **system-linker**: Uses the system's linker (cc/ld) - default
//!
//! # Installation of LLD Development Headers
//!
//! ## macOS (with LLD from source)
//! ```bash
//! # Clone LLVM with LLD
//! git clone --depth 1 https://github.com/llvm/llvm-project.git
//! cd llvm-project
//! mkdir build && cd build
//! cmake -G Ninja ../llvm \
//!   -DLLVM_ENABLE_PROJECTS="lld" \
//!   -DCMAKE_BUILD_TYPE=Release \
//!   -DCMAKE_INSTALL_PREFIX=/usr/local/llvm-lld
//! ninja && ninja install
//!
//! # Set environment variable
//! export LLVM_PREFIX=/usr/local/llvm-lld
//! ```
//!
//! ## Ubuntu/Debian
//! ```bash
//! apt-get install lld-21 liblld-21-dev
//! # or
//! apt-get install lld liblld-dev
//! ```
//!
//! ## Arch Linux
//! ```bash
//! pacman -S lld
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_lld::{Linker, LinkerFlavor};
//!
//! let linker = Linker::new(LinkerFlavor::native());
//! let result = linker
//!     .add_object("main.o")
//!     .add_library("c")
//!     .output("program")
//!     .link();
//! ```

use std::path::Path;
use std::process::Command;
use thiserror::Error;
use tracing::debug;

/// Linker errors
#[derive(Debug, Error)]
pub enum LinkerError {
    #[error("Linking failed: {message}")]
    LinkFailed { message: String },

    #[error("Linker crashed: {message}")]
    Crashed { message: String },

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Unsupported linker flavor: {0:?}")]
    UnsupportedFlavor(LinkerFlavor),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for linker operations
pub type LinkerResult<T> = Result<T, LinkerError>;

/// Linker output format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkerFlavor {
    /// ELF format (Linux, BSD)
    Elf,
    /// Mach-O format (macOS, iOS)
    MachO,
    /// COFF/PE format (Windows)
    Coff,
    /// WebAssembly
    Wasm,
}

impl LinkerFlavor {
    /// Get the default flavor for the current platform
    pub fn native() -> Self {
        #[cfg(target_os = "linux")]
        return LinkerFlavor::Elf;

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        return LinkerFlavor::MachO;

        #[cfg(target_os = "windows")]
        return LinkerFlavor::Coff;

        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "ios",
            target_os = "windows"
        )))]
        return LinkerFlavor::Elf;
    }

    /// Check if this flavor is supported
    pub fn is_supported(&self) -> bool {
        true // System linker supports all formats
    }
}

/// Output of a link operation
#[derive(Debug)]
pub struct LinkOutput {
    /// Whether linking succeeded
    pub success: bool,
    /// Standard output from linker
    pub stdout: String,
    /// Standard error from linker
    pub stderr: String,
}

/// High-level linker builder
#[derive(Debug, Clone)]
pub struct Linker {
    flavor: LinkerFlavor,
    args: Vec<String>,
}

impl Linker {
    /// Create a new linker with the specified flavor
    pub fn new(flavor: LinkerFlavor) -> Self {
        Self {
            flavor,
            args: Vec::new(),
        }
    }

    /// Create a linker for the native platform
    pub fn native() -> Self {
        Self::new(LinkerFlavor::native())
    }

    /// Add a raw argument
    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.args.push(arg.as_ref().to_string());
        self
    }

    /// Add multiple raw arguments
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_string());
        }
        self
    }

    /// Add an input object file
    pub fn add_object(self, path: impl AsRef<Path>) -> Self {
        self.arg(path.as_ref().to_string_lossy())
    }

    /// Add multiple input object files
    pub fn add_objects<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for path in paths {
            self = self.add_object(path);
        }
        self
    }

    /// Add a static library
    pub fn add_static_lib(self, path: impl AsRef<Path>) -> Self {
        self.arg(path.as_ref().to_string_lossy())
    }

    /// Add a library search path
    pub fn add_library_path(self, path: impl AsRef<Path>) -> Self {
        match self.flavor {
            LinkerFlavor::Coff => self.arg(format!("/LIBPATH:{}", path.as_ref().display())),
            _ => self.arg("-L").arg(path.as_ref().to_string_lossy()),
        }
    }

    /// Link against a library by name
    pub fn add_library(self, name: impl AsRef<str>) -> Self {
        match self.flavor {
            LinkerFlavor::Coff => {
                let name = name.as_ref();
                if name.ends_with(".lib") {
                    self.arg(name)
                } else {
                    self.arg(format!("{}.lib", name))
                }
            }
            _ => self.arg(format!("-l{}", name.as_ref())),
        }
    }

    /// Set output file path
    pub fn output(self, path: impl AsRef<Path>) -> Self {
        match self.flavor {
            LinkerFlavor::Coff => self.arg(format!("/OUT:{}", path.as_ref().display())),
            _ => self.arg("-o").arg(path.as_ref().to_string_lossy()),
        }
    }

    /// Create a shared library
    pub fn shared(self) -> Self {
        match self.flavor {
            LinkerFlavor::Elf => self.arg("-shared"),
            LinkerFlavor::MachO => self.arg("-dynamiclib"),
            LinkerFlavor::Coff => self.arg("/DLL"),
            LinkerFlavor::Wasm => self.arg("--shared"),
        }
    }

    /// Create a position-independent executable
    pub fn pie(self) -> Self {
        self.arg("-pie")
    }

    /// Create a static executable
    pub fn static_link(self) -> Self {
        self.arg("-static")
    }

    /// Strip symbols
    pub fn strip(self) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::Wasm => self.arg("-s"),
            LinkerFlavor::MachO => self.arg("-Wl,-dead_strip"),
            LinkerFlavor::Coff => self,
        }
    }

    /// Strip debug info only
    pub fn strip_debug(self) -> Self {
        match self.flavor {
            LinkerFlavor::Coff => self.arg("/DEBUG:NONE"),
            _ => self.arg("-Wl,--strip-debug"),
        }
    }

    /// Set entry point
    pub fn entry(self, symbol: impl AsRef<str>) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::Wasm => {
                self.arg("-e").arg(symbol.as_ref())
            }
            LinkerFlavor::MachO => self.arg("-e").arg(symbol.as_ref()),
            LinkerFlavor::Coff => self.arg(format!("/ENTRY:{}", symbol.as_ref())),
        }
    }

    /// Enable garbage collection of unused sections
    pub fn gc_sections(self) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::Wasm => self.arg("-Wl,--gc-sections"),
            LinkerFlavor::MachO => self.arg("-Wl,-dead_strip"),
            LinkerFlavor::Coff => self.arg("/OPT:REF"),
        }
    }

    /// Set soname (shared library name)
    pub fn soname(self, name: impl AsRef<str>) -> Self {
        match self.flavor {
            LinkerFlavor::Elf => self.arg(format!("-Wl,-soname,{}", name.as_ref())),
            LinkerFlavor::MachO => self.arg("-Wl,-install_name").arg(name.as_ref()),
            _ => self,
        }
    }

    /// Set rpath
    pub fn rpath(self, path: impl AsRef<Path>) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::MachO => {
                self.arg(format!("-Wl,-rpath,{}", path.as_ref().display()))
            }
            _ => self,
        }
    }

    /// Add exported symbol
    pub fn export_symbol(self, symbol: impl AsRef<str>) -> Self {
        match self.flavor {
            LinkerFlavor::Elf => self.arg(format!("-Wl,--export-dynamic-symbol={}", symbol.as_ref())),
            LinkerFlavor::MachO => self.arg("-Wl,-exported_symbol").arg(symbol.as_ref()),
            LinkerFlavor::Coff => self.arg(format!("/EXPORT:{}", symbol.as_ref())),
            LinkerFlavor::Wasm => self.arg(format!("--export={}", symbol.as_ref())),
        }
    }

    /// Set linker script
    pub fn linker_script(self, path: impl AsRef<Path>) -> Self {
        self.arg("-T").arg(path.as_ref().to_string_lossy())
    }

    /// Enable verbose output
    pub fn verbose(self) -> Self {
        self.arg("-v")
    }

    /// Perform the link operation
    pub fn link(self) -> LinkerResult<LinkOutput> {
        #[cfg(feature = "lld-subprocess")]
        {
            self.link_with_lld_subprocess()
        }
        #[cfg(all(not(feature = "lld-subprocess"), feature = "system-linker"))]
        {
            self.link_with_system_linker()
        }
        #[cfg(all(not(feature = "lld-subprocess"), not(feature = "system-linker")))]
        {
            self.link_with_system_linker()
        }
    }

    /// Link using LLD subprocess (ld.lld, ld64.lld)
    #[cfg(feature = "lld-subprocess")]
    fn link_with_lld_subprocess(self) -> LinkerResult<LinkOutput> {
        // Find LLD binary
        let lld_binary = self.find_lld_binary()?;

        debug!("Using LLD: {}", lld_binary);
        debug!("Linker args: {:?}", self.args);

        let mut cmd = Command::new(&lld_binary);
        cmd.args(&self.args);

        let output = cmd.output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(LinkerError::LinkFailed {
                message: if stderr.is_empty() { stdout } else { stderr },
            });
        }

        Ok(LinkOutput {
            success: true,
            stdout,
            stderr,
        })
    }

    /// Find LLD binary for current platform
    #[cfg(feature = "lld-subprocess")]
    fn find_lld_binary(&self) -> LinkerResult<String> {
        let candidates = match self.flavor {
            LinkerFlavor::Elf => vec!["ld.lld", "lld"],
            LinkerFlavor::MachO => vec!["ld64.lld", "lld"],
            LinkerFlavor::Coff => vec!["lld-link", "lld"],
            LinkerFlavor::Wasm => vec!["wasm-ld", "lld"],
        };

        // Check common paths
        let search_paths = [
            "/opt/homebrew/bin",
            "/opt/homebrew/opt/llvm/bin",
            "/usr/local/bin",
            "/usr/local/opt/llvm/bin",
            "/usr/bin",
            "/usr/lib/llvm-21/bin",
            "/usr/lib/llvm-20/bin",
            "/usr/lib/llvm-19/bin",
        ];

        for candidate in &candidates {
            // Check in PATH first
            if let Ok(path) = std::process::Command::new("which")
                .arg(candidate)
                .output()
            {
                if path.status.success() {
                    let path_str = String::from_utf8_lossy(&path.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        return Ok(path_str);
                    }
                }
            }

            // Check in known paths
            for search_path in &search_paths {
                let full_path = format!("{}/{}", search_path, candidate);
                if std::path::Path::new(&full_path).exists() {
                    return Ok(full_path);
                }
            }
        }

        // Fallback to system linker
        Err(LinkerError::Internal(format!(
            "LLD not found. Install LLVM with LLD or use system-linker feature. \
             Searched for: {:?}",
            candidates
        )))
    }

    /// Link using system linker (cc/clang/gcc)
    #[allow(dead_code)]
    fn link_with_system_linker(self) -> LinkerResult<LinkOutput> {
        // Determine which linker to use
        let linker = std::env::var("CC").unwrap_or_else(|_| {
            #[cfg(target_os = "macos")]
            { "clang".to_string() }
            #[cfg(not(target_os = "macos"))]
            { "cc".to_string() }
        });

        debug!("Using system linker: {}", linker);
        debug!("Linker args: {:?}", self.args);

        let mut cmd = Command::new(&linker);
        cmd.args(&self.args);

        let output = cmd.output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(LinkerError::LinkFailed {
                message: if stderr.is_empty() { stdout } else { stderr },
            });
        }

        Ok(LinkOutput {
            success: true,
            stdout,
            stderr,
        })
    }

    /// Enable LTO
    pub fn lto(self) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::MachO => self.arg("--lto=full"),
            _ => self,
        }
    }

    /// Enable ThinLTO
    pub fn thin_lto(self) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::MachO => self.arg("--lto=thin"),
            _ => self,
        }
    }

    /// Set ThinLTO cache directory
    pub fn thin_lto_cache(self, dir: impl AsRef<Path>) -> Self {
        match self.flavor {
            LinkerFlavor::Elf | LinkerFlavor::MachO => {
                self.arg(format!("--thinlto-cache-dir={}", dir.as_ref().display()))
            }
            _ => self,
        }
    }
}

/// Initialize the linker subsystem
pub fn init() {
    // Nothing to do for system linker
}

/// Get linker version
pub fn version() -> &'static str {
    "system"
}

/// Convenience function to link executable
pub fn link_executable(
    objects: &[impl AsRef<Path>],
    libraries: &[impl AsRef<str>],
    output: impl AsRef<Path>,
) -> LinkerResult<LinkOutput> {
    let mut linker = Linker::native()
        .gc_sections()
        .output(&output);

    for obj in objects {
        linker = linker.add_object(obj);
    }

    for lib in libraries {
        linker = linker.add_library(lib);
    }

    linker.link()
}

/// Convenience function to link shared library
pub fn link_shared_library(
    objects: &[impl AsRef<Path>],
    libraries: &[impl AsRef<str>],
    output: impl AsRef<Path>,
    soname: Option<&str>,
) -> LinkerResult<LinkOutput> {
    let mut linker = Linker::native()
        .shared()
        .gc_sections()
        .output(&output);

    if let Some(name) = soname {
        linker = linker.soname(name);
    }

    for obj in objects {
        linker = linker.add_object(obj);
    }

    for lib in libraries {
        linker = linker.add_library(lib);
    }

    linker.link()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linker_flavor_native() {
        let flavor = LinkerFlavor::native();
        assert!(flavor.is_supported());
    }

    #[test]
    fn test_linker_builder() {
        let linker = Linker::native()
            .add_object("main.o")
            .add_library_path("/usr/lib")
            .add_library("c")
            .output("test");

        assert!(!linker.args.is_empty());
    }
}
