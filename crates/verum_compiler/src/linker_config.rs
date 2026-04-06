//! Linker configuration from Verum.toml
//!
//! This module provides comprehensive configuration for the linking phase,
//! supporting all LLD linker options through Verum.toml configuration.
//!
//! ## Configuration Schema
//!
//! ```toml
//! [linker]
//! # Output kind: "executable", "shared", "static", "object"
//! output = "executable"
//!
//! # LTO mode: "none", "thin", "full"
//! lto = "thin"
//!
//! # Use LLVM linker (lld) instead of system linker
//! use_lld = true
//!
//! # Enable position-independent code
//! pic = true
//!
//! # Strip debug symbols
//! strip = false
//!
//! # Keep debug info in output
//! debug_info = true
//!
//! # Static linking (no runtime dependencies)
//! static_link = false
//!
//! # Strip debug symbols only (keep function names)
//! strip_debug_only = false
//!
//! # Entry point symbol name (for executables)
//! entry_point = "main"
//!
//! # Target triple (e.g., "x86_64-unknown-linux-gnu")
//! target = "native"
//!
//! # Library search paths
//! library_paths = ["/usr/local/lib", "vendor/lib"]
//!
//! # Libraries to link
//! libraries = ["pthread", "m", "dl"]
//!
//! # Symbols to export (for shared libraries)
//! exports = ["api_init", "api_cleanup"]
//!
//! # Extra linker flags
//! extra_flags = ["-Wl,--as-needed"]
//!
//! # Platform-specific settings
//! [linker.linux]
//! libraries = ["rt", "pthread"]
//!
//! [linker.macos]
//! extra_flags = ["-framework", "CoreFoundation"]
//!
//! [linker.windows]
//! libraries = ["kernel32", "user32"]
//! ```
//!

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use verum_common::{List, Text};

use crate::phases::linking::{LTOConfig, LinkingConfig, OutputKind};

// V-LLSI no-libc linking configuration
use verum_codegen::link::{NoLibcConfig, Platform};

/// Complete linker configuration from Verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkerTomlConfig {
    /// Linker settings section
    #[serde(default)]
    pub linker: LinkerSection,
}

/// [linker] section in Verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkerSection {
    /// Output type: "executable", "shared", "static", "object"
    #[serde(default = "default_output")]
    pub output: String,

    /// LTO mode: "none", "thin", "full"
    #[serde(default = "default_lto")]
    pub lto: String,

    /// Use LLVM linker (lld) instead of system linker
    #[serde(default = "default_use_lld")]
    pub use_lld: bool,

    /// Enable position-independent code
    #[serde(default = "default_pic")]
    pub pic: bool,

    /// Strip all debug symbols from output
    #[serde(default)]
    pub strip: bool,

    /// Include debug info in output
    #[serde(default = "default_debug_info")]
    pub debug_info: bool,

    /// Enable static linking (no runtime dependencies)
    #[serde(default)]
    pub static_link: bool,

    /// Strip debug symbols only (keep function names for stack traces)
    #[serde(default)]
    pub strip_debug_only: bool,

    /// Entry point symbol name (for executables, default: "main")
    #[serde(default)]
    pub entry_point: Option<String>,

    /// Target triple (e.g., "x86_64-unknown-linux-gnu", "native" for host)
    #[serde(default)]
    pub target: Option<String>,

    /// Library search paths
    #[serde(default)]
    pub library_paths: Vec<String>,

    /// Libraries to link
    #[serde(default = "default_libraries")]
    pub libraries: Vec<String>,

    /// Symbols to export (for shared libraries)
    #[serde(default)]
    pub exports: Vec<String>,

    /// Extra linker flags passed directly to the linker
    #[serde(default)]
    pub extra_flags: Vec<String>,

    /// Linux-specific settings
    #[serde(default)]
    pub linux: Option<PlatformLinkerSection>,

    /// macOS-specific settings
    #[serde(default)]
    pub macos: Option<PlatformLinkerSection>,

    /// Windows-specific settings
    #[serde(default)]
    pub windows: Option<PlatformLinkerSection>,
}

/// Platform-specific linker settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformLinkerSection {
    /// Additional library search paths for this platform
    #[serde(default)]
    pub library_paths: Vec<String>,

    /// Additional libraries to link for this platform
    #[serde(default)]
    pub libraries: Vec<String>,

    /// Additional symbols to export for this platform
    #[serde(default)]
    pub exports: Vec<String>,

    /// Additional linker flags for this platform
    #[serde(default)]
    pub extra_flags: Vec<String>,
}

// =============================================================================
// Default Values
// =============================================================================

fn default_output() -> String {
    "executable".to_string()
}

fn default_lto() -> String {
    "thin".to_string()
}

fn default_use_lld() -> bool {
    cfg!(target_os = "linux")
}

fn default_pic() -> bool {
    true
}

fn default_debug_info() -> bool {
    true
}

fn default_libraries() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        vec!["pthread".to_string(), "m".to_string(), "dl".to_string()]
    }
    #[cfg(target_os = "macos")]
    {
        vec!["System".to_string()]
    }
    #[cfg(target_os = "windows")]
    {
        vec!["kernel32".to_string(), "msvcrt".to_string()]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        vec![]
    }
}

// =============================================================================
// Default Implementations
// =============================================================================

impl Default for LinkerSection {
    fn default() -> Self {
        Self {
            output: default_output(),
            lto: default_lto(),
            use_lld: default_use_lld(),
            pic: default_pic(),
            strip: false,
            debug_info: default_debug_info(),
            static_link: false,
            strip_debug_only: false,
            entry_point: Some("main".to_string()),
            target: None,
            library_paths: Vec::new(),
            libraries: default_libraries(),
            exports: Vec::new(),
            extra_flags: Vec::new(),
            linux: None,
            macos: None,
            windows: None,
        }
    }
}

impl Default for LinkerTomlConfig {
    fn default() -> Self {
        Self {
            linker: LinkerSection::default(),
        }
    }
}

// =============================================================================
// Parsing Methods
// =============================================================================

impl LinkerTomlConfig {
    /// Load from Verum.toml file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read {}", path.as_ref().display()))?;

        let config: LinkerTomlConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.as_ref().display()))?;

        Ok(config)
    }

    /// Load from current directory (looks for Verum.toml)
    pub fn load() -> Result<Self> {
        let path = PathBuf::from("Verum.toml");
        if path.exists() {
            Self::load_from_file(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Load from string content
    pub fn from_str(content: &str) -> Result<Self> {
        let config: LinkerTomlConfig =
            toml::from_str(content).with_context(|| "Failed to parse TOML content")?;
        Ok(config)
    }

    /// Convert to LinkingConfig for use in the compilation pipeline
    pub fn to_linking_config(&self, output_path: PathBuf) -> Result<LinkingConfig> {
        let section = &self.linker;

        // Parse output kind
        let output_kind = parse_output_kind(&section.output)?;

        // Parse LTO config
        let lto = parse_lto_config(&section.lto)?;

        // Parse target triple
        let target_triple = section.target.as_ref().and_then(|t| {
            if t == "native" || t.is_empty() {
                None
            } else {
                Some(Text::from(t.as_str()))
            }
        });

        // Parse entry point
        let entry_point = match output_kind {
            OutputKind::Executable => section
                .entry_point
                .as_ref()
                .map(|s| Text::from(s.as_str()))
                .or_else(|| Some(Text::from("main"))),
            _ => None,
        };

        // Merge platform-specific settings
        let (library_paths, libraries, exports, extra_flags) =
            self.merge_platform_settings(section);

        // Determine platform from target triple for no-libc configuration
        let platform = target_triple
            .as_ref()
            .and_then(|t| Platform::from_triple(t.as_str()))
            .unwrap_or_else(Platform::host);

        Ok(LinkingConfig {
            output_kind,
            lto,
            use_llvm_linker: section.use_lld,
            pic: section.pic,
            strip: section.strip,
            extra_flags,
            output_path,
            library_paths,
            libraries,
            exported_symbols: exports,
            entry_point,
            debug_info: section.debug_info,
            target_triple,
            static_link: section.static_link,
            strip_debug_only: section.strip_debug_only,
            // V-LLSI: Always use no-libc linking for self-contained binaries
            no_libc_config: Some(NoLibcConfig::for_platform(platform)),
        })
    }

    /// Merge base settings with platform-specific settings
    fn merge_platform_settings(
        &self,
        section: &LinkerSection,
    ) -> (List<PathBuf>, List<Text>, List<Text>, List<Text>) {
        let mut library_paths: Vec<PathBuf> =
            section.library_paths.iter().map(PathBuf::from).collect();
        let mut libraries: Vec<Text> = section
            .libraries
            .iter()
            .map(|s| Text::from(s.as_str()))
            .collect();
        let mut exports: Vec<Text> = section
            .exports
            .iter()
            .map(|s| Text::from(s.as_str()))
            .collect();
        let mut extra_flags: Vec<Text> = section
            .extra_flags
            .iter()
            .map(|s| Text::from(s.as_str()))
            .collect();

        // Get current platform settings
        let platform_settings = self.get_current_platform_settings(section);

        if let Some(ps) = platform_settings {
            library_paths.extend(ps.library_paths.iter().map(PathBuf::from));
            libraries.extend(ps.libraries.iter().map(|s| Text::from(s.as_str())));
            exports.extend(ps.exports.iter().map(|s| Text::from(s.as_str())));
            extra_flags.extend(ps.extra_flags.iter().map(|s| Text::from(s.as_str())));
        }

        (
            List::from(library_paths),
            List::from(libraries),
            List::from(exports),
            List::from(extra_flags),
        )
    }

    /// Get platform-specific settings for current platform
    fn get_current_platform_settings<'a>(
        &'a self,
        section: &'a LinkerSection,
    ) -> Option<&'a PlatformLinkerSection> {
        #[cfg(target_os = "linux")]
        {
            section.linux.as_ref()
        }
        #[cfg(target_os = "macos")]
        {
            section.macos.as_ref()
        }
        #[cfg(target_os = "windows")]
        {
            section.windows.as_ref()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse output kind from string
fn parse_output_kind(s: &str) -> Result<OutputKind> {
    match s.to_lowercase().as_str() {
        "executable" | "exe" | "bin" => Ok(OutputKind::Executable),
        "shared" | "dylib" | "so" | "dll" | "shared_library" | "dynamic" => {
            Ok(OutputKind::SharedLibrary)
        }
        "static" | "lib" | "a" | "static_library" | "archive" => Ok(OutputKind::StaticLibrary),
        "object" | "obj" | "o" => Ok(OutputKind::ObjectFile),
        _ => anyhow::bail!(
            "Invalid output kind '{}'. Expected: executable, shared, static, or object",
            s
        ),
    }
}

/// Parse LTO config from string
fn parse_lto_config(s: &str) -> Result<LTOConfig> {
    match s.to_lowercase().as_str() {
        "none" | "off" | "false" | "0" => Ok(LTOConfig::None),
        "thin" | "thinlto" => Ok(LTOConfig::Thin),
        "full" | "lto" | "true" | "1" => Ok(LTOConfig::Full),
        _ => anyhow::bail!(
            "Invalid LTO mode '{}'. Expected: none, thin, or full",
            s
        ),
    }
}

// =============================================================================
// Profile-Based Configuration
// =============================================================================

/// Build profile configuration from Verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    /// Profile-specific linker settings
    #[serde(default)]
    pub linker: LinkerSection,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            linker: LinkerSection::default(),
        }
    }
}

/// Complete project configuration from Verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Cog metadata
    #[serde(default, rename = "cog")]
    pub cog: CogSection,

    /// Default linker settings
    #[serde(default)]
    pub linker: LinkerSection,

    /// Dev profile settings
    #[serde(default, rename = "profile.dev")]
    pub profile_dev: Option<ProfileConfig>,

    /// Release profile settings
    #[serde(default, rename = "profile.release")]
    pub profile_release: Option<ProfileConfig>,

    /// Custom profiles
    #[serde(default)]
    pub profile: std::collections::HashMap<String, ProfileConfig>,
}

/// [cog] section in Verum.toml
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CogSection {
    /// Cog name
    #[serde(default)]
    pub name: String,

    /// Cog version
    #[serde(default)]
    pub version: String,

    /// Cog authors
    #[serde(default)]
    pub authors: Vec<String>,

    /// Cog description
    #[serde(default)]
    pub description: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            cog: CogSection::default(),
            linker: LinkerSection::default(),
            profile_dev: None,
            profile_release: None,
            profile: std::collections::HashMap::new(),
        }
    }
}

impl ProjectConfig {
    /// Load from Verum.toml file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read {}", path.as_ref().display()))?;

        let config: ProjectConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.as_ref().display()))?;

        Ok(config)
    }

    /// Load from current directory (looks for Verum.toml)
    pub fn load() -> Result<Self> {
        let path = PathBuf::from("Verum.toml");
        if path.exists() {
            Self::load_from_file(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Get linker configuration for a specific profile
    pub fn linker_config_for_profile(&self, profile: &str) -> LinkerSection {
        // First check named profiles
        if let Some(profile_config) = self.profile.get(profile) {
            return merge_linker_sections(&self.linker, &profile_config.linker);
        }

        // Check dev/release shortcuts
        match profile {
            "dev" | "debug" => {
                if let Some(ref dev) = self.profile_dev {
                    return merge_linker_sections(&self.linker, &dev.linker);
                }
            }
            "release" | "prod" => {
                if let Some(ref release) = self.profile_release {
                    return merge_linker_sections(&self.linker, &release.linker);
                }
            }
            _ => {}
        }

        // Fall back to default linker settings
        self.linker.clone()
    }

    /// Convert to LinkingConfig for a specific profile and output path
    pub fn to_linking_config(
        &self,
        profile: &str,
        output_path: PathBuf,
    ) -> Result<LinkingConfig> {
        let section = self.linker_config_for_profile(profile);
        let toml_config = LinkerTomlConfig { linker: section };
        toml_config.to_linking_config(output_path)
    }
}

/// Merge two linker sections, with profile settings overriding base settings
fn merge_linker_sections(base: &LinkerSection, profile: &LinkerSection) -> LinkerSection {
    // For string fields, use profile if non-default, otherwise base
    let output = if profile.output != default_output() {
        profile.output.clone()
    } else {
        base.output.clone()
    };

    let lto = if profile.lto != default_lto() {
        profile.lto.clone()
    } else {
        base.lto.clone()
    };

    // Merge lists (profile adds to base)
    let mut library_paths = base.library_paths.clone();
    library_paths.extend(profile.library_paths.clone());

    let mut libraries = base.libraries.clone();
    libraries.extend(profile.libraries.clone());

    let mut exports = base.exports.clone();
    exports.extend(profile.exports.clone());

    let mut extra_flags = base.extra_flags.clone();
    extra_flags.extend(profile.extra_flags.clone());

    LinkerSection {
        output,
        lto,
        use_lld: profile.use_lld || base.use_lld,
        pic: profile.pic || base.pic,
        strip: profile.strip || base.strip,
        debug_info: profile.debug_info && base.debug_info,
        static_link: profile.static_link || base.static_link,
        strip_debug_only: profile.strip_debug_only || base.strip_debug_only,
        entry_point: profile.entry_point.clone().or_else(|| base.entry_point.clone()),
        target: profile.target.clone().or_else(|| base.target.clone()),
        library_paths,
        libraries,
        exports,
        extra_flags,
        linux: merge_platform_sections(&base.linux, &profile.linux),
        macos: merge_platform_sections(&base.macos, &profile.macos),
        windows: merge_platform_sections(&base.windows, &profile.windows),
    }
}

/// Merge platform sections
fn merge_platform_sections(
    base: &Option<PlatformLinkerSection>,
    profile: &Option<PlatformLinkerSection>,
) -> Option<PlatformLinkerSection> {
    match (base, profile) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(p)) => Some(p.clone()),
        (Some(b), Some(p)) => {
            let mut library_paths = b.library_paths.clone();
            library_paths.extend(p.library_paths.clone());

            let mut libraries = b.libraries.clone();
            libraries.extend(p.libraries.clone());

            let mut exports = b.exports.clone();
            exports.extend(p.exports.clone());

            let mut extra_flags = b.extra_flags.clone();
            extra_flags.extend(p.extra_flags.clone());

            Some(PlatformLinkerSection {
                library_paths,
                libraries,
                exports,
                extra_flags,
            })
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LinkerTomlConfig::default();
        assert_eq!(config.linker.output, "executable");
        assert_eq!(config.linker.lto, "thin");
        assert!(config.linker.pic);
        assert!(!config.linker.strip);
        assert!(config.linker.debug_info);
    }

    #[test]
    fn test_parse_output_kind() {
        assert_eq!(parse_output_kind("executable").unwrap(), OutputKind::Executable);
        assert_eq!(parse_output_kind("exe").unwrap(), OutputKind::Executable);
        assert_eq!(parse_output_kind("shared").unwrap(), OutputKind::SharedLibrary);
        assert_eq!(parse_output_kind("dylib").unwrap(), OutputKind::SharedLibrary);
        assert_eq!(parse_output_kind("static").unwrap(), OutputKind::StaticLibrary);
        assert_eq!(parse_output_kind("object").unwrap(), OutputKind::ObjectFile);
        assert!(parse_output_kind("invalid").is_err());
    }

    #[test]
    fn test_parse_lto_config() {
        assert_eq!(parse_lto_config("none").unwrap(), LTOConfig::None);
        assert_eq!(parse_lto_config("thin").unwrap(), LTOConfig::Thin);
        assert_eq!(parse_lto_config("full").unwrap(), LTOConfig::Full);
        assert_eq!(parse_lto_config("off").unwrap(), LTOConfig::None);
        assert!(parse_lto_config("invalid").is_err());
    }

    #[test]
    fn test_parse_basic_toml() {
        let toml_str = r#"
[linker]
output = "executable"
lto = "full"
use_lld = true
pic = true
strip = false
debug_info = true
static_link = false
entry_point = "main"
libraries = ["pthread", "m"]
extra_flags = ["-Wl,--as-needed"]
"#;

        let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.linker.output, "executable");
        assert_eq!(config.linker.lto, "full");
        assert!(config.linker.use_lld);
        assert!(config.linker.pic);
        assert!(!config.linker.strip);
        assert!(config.linker.debug_info);
        assert_eq!(config.linker.libraries, vec!["pthread", "m"]);
        assert_eq!(config.linker.extra_flags, vec!["-Wl,--as-needed"]);
    }

    #[test]
    fn test_parse_platform_specific() {
        let toml_str = r#"
[linker]
output = "shared"
libraries = ["common"]

[linker.linux]
libraries = ["rt", "pthread"]
extra_flags = ["-Wl,-soname,libtest.so"]

[linker.macos]
extra_flags = ["-framework", "CoreFoundation"]

[linker.windows]
libraries = ["kernel32", "user32"]
"#;

        let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.linker.output, "shared");
        assert_eq!(config.linker.libraries, vec!["common"]);

        let linux = config.linker.linux.as_ref().unwrap();
        assert_eq!(linux.libraries, vec!["rt", "pthread"]);

        let macos = config.linker.macos.as_ref().unwrap();
        assert_eq!(macos.extra_flags, vec!["-framework", "CoreFoundation"]);

        let windows = config.linker.windows.as_ref().unwrap();
        assert_eq!(windows.libraries, vec!["kernel32", "user32"]);
    }

    #[test]
    fn test_to_linking_config() {
        let toml_str = r#"
[linker]
output = "executable"
lto = "thin"
use_lld = true
libraries = ["pthread", "m"]
"#;

        let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
        let linking_config = config
            .to_linking_config(PathBuf::from("output"))
            .unwrap();

        assert_eq!(linking_config.output_kind, OutputKind::Executable);
        assert_eq!(linking_config.lto, LTOConfig::Thin);
        assert!(linking_config.use_llvm_linker);
    }

    #[test]
    fn test_project_config_profiles() {
        let toml_str = r#"
[cog]
name = "test_app"
version = "0.1.0"

[linker]
output = "executable"
lto = "none"
debug_info = true

[profile.release.linker]
lto = "full"
strip = true
debug_info = false
"#;

        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cog.name, "test_app");

        // Dev profile should inherit base settings
        let dev_linker = config.linker_config_for_profile("dev");
        assert_eq!(dev_linker.lto, "none");
        assert!(dev_linker.debug_info);
        assert!(!dev_linker.strip);

        // Release profile should override
        let release_linker = config.linker_config_for_profile("release");
        assert_eq!(release_linker.lto, "full");
        assert!(!release_linker.debug_info);
        assert!(release_linker.strip);
    }

    #[test]
    fn test_shared_library_config() {
        let toml_str = r#"
[linker]
output = "shared"
pic = true
exports = ["api_init", "api_process", "api_cleanup"]
"#;

        let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
        let linking_config = config
            .to_linking_config(PathBuf::from("libtest.so"))
            .unwrap();

        assert_eq!(linking_config.output_kind, OutputKind::SharedLibrary);
        assert!(linking_config.pic);
        assert_eq!(linking_config.exported_symbols.len(), 3);
        assert!(linking_config.entry_point.is_none()); // No entry point for shared libs
    }

    #[test]
    fn test_static_library_config() {
        let toml_str = r#"
[linker]
output = "static"
"#;

        let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
        let linking_config = config
            .to_linking_config(PathBuf::from("libtest.a"))
            .unwrap();

        assert_eq!(linking_config.output_kind, OutputKind::StaticLibrary);
        assert!(linking_config.entry_point.is_none());
    }
}
