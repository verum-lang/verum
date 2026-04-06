// Configuration management for Verum projects
// Handles verum.toml parsing and project manifests
// Parses verum.toml project manifests with sections: [package], [language],
// [dependencies], [profiles], [build], [workspace], [lsp], [registry]

use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Text};

// ========================================================================
// MLIR Backend Configuration Types (formerly LLVM config)
// These types are kept for manifest compatibility but now configure MLIR
// ========================================================================

/// MLIR/LLVM backend configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlvmConfig {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    #[serde(default)]
    pub target_triple: Option<Text>,
    /// Target CPU (e.g., "native", "generic")
    #[serde(default)]
    pub target_cpu: Option<Text>,
    /// Target features (e.g., "+avx2,+fma")
    #[serde(default)]
    pub target_features: Option<Text>,
}

impl LlvmConfig {
    /// Validate the LLVM/MLIR configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Target triple validation is now handled by MLIR backend
        Ok(())
    }
}

/// Optimization pass configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptimizationConfig {
    /// Optimization level (0-3)
    #[serde(default)]
    pub level: u8,
    /// Enable size optimization
    #[serde(default)]
    pub size_opt: bool,
    /// Enable inline optimization
    #[serde(default)]
    pub inline: bool,
}

impl OptimizationConfig {
    /// Validate the optimization configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.level > 3 {
            return Err(format!("Optimization level must be 0-3, got {}", self.level));
        }
        Ok(())
    }

    /// Create a debug configuration (no optimizations)
    pub fn debug() -> Self {
        Self {
            level: 0,
            size_opt: false,
            inline: false,
        }
    }
}

/// Link-time optimization configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LtoConfig {
    /// Enable LTO
    #[serde(default)]
    pub enabled: bool,
    /// LTO mode: "thin" or "full"
    #[serde(default)]
    pub mode: Option<Text>,
}

impl LtoConfig {
    /// Validate the LTO configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        if let Some(mode) = &self.mode {
            match mode.as_str() {
                "thin" | "full" => {}
                _ => return Err(format!("Invalid LTO mode: {}, must be 'thin' or 'full'", mode)),
            }
        }
        Ok(())
    }
}

/// Profile-guided optimization configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PgoConfig {
    /// Enable PGO
    #[serde(default)]
    pub enabled: bool,
    /// Profile data path
    #[serde(default)]
    pub profile_path: Option<Text>,
}

impl PgoConfig {
    /// Validate the PGO configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Profile path validation is deferred to runtime
        Ok(())
    }
}

/// Cross-compilation configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrossCompileConfig {
    /// Target platform
    #[serde(default)]
    pub target: Option<Text>,
    /// Sysroot path
    #[serde(default)]
    pub sysroot: Option<Text>,
    /// Linker to use
    #[serde(default)]
    pub linker: Option<Text>,
}

impl CrossCompileConfig {
    /// Validate the cross-compilation configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Cross-compilation validation is now handled by MLIR backend
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub cog: Cog,
    #[serde(default)]
    pub language: LanguageConfig,
    #[serde(default)]
    pub dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub dev_dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub build_dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub features: Map<Text, List<Text>>,
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub workspace: Option<WorkspaceConfig>,
    #[serde(default)]
    pub lsp: LspConfig,
    #[serde(default)]
    pub registry: RegistryConfig,

    // ========================================================================
    // LLVM Backend Configuration Sections
    // ========================================================================
    /// LLVM backend configuration (target, CPU, features)
    #[serde(default)]
    pub llvm: LlvmConfig,

    /// Optimization pass configuration
    #[serde(default)]
    pub optimization: OptimizationConfig,

    /// Link-time optimization configuration
    #[serde(default)]
    pub lto: LtoConfig,

    /// Profile-guided optimization configuration
    #[serde(default)]
    pub pgo: PgoConfig,

    /// Cross-compilation configuration
    #[serde(default)]
    pub cross_compile: CrossCompileConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cog {
    pub name: Text,
    pub version: Text,
    #[serde(default)]
    pub authors: List<Text>,
    #[serde(default)]
    pub description: Option<Text>,
    #[serde(default)]
    pub license: Option<Text>,
    #[serde(default)]
    pub repository: Option<Text>,
    #[serde(default)]
    pub homepage: Option<Text>,
    #[serde(default)]
    pub keywords: List<Text>,
    #[serde(default)]
    pub categories: List<Text>,
}

// Language profile configuration
// Profiles (systems, application, scripting) determine available features,
// default verification level, and compilation tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    pub profile: LanguageProfile,
}

impl Default for LanguageConfig {
    fn default() -> Self {
        Self {
            profile: LanguageProfile::Application,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LanguageProfile {
    Application, // 80% users: No unsafe, refinements + runtime checks
    Systems,     // 15% users: Full language including unsafe
    Research,    // 5% users: Dependent types, formal proofs
}

impl LanguageProfile {
    pub fn allows_unsafe(&self) -> bool {
        matches!(self, LanguageProfile::Systems)
    }

    pub fn requires_verification(&self) -> bool {
        matches!(self, LanguageProfile::Research)
    }

    pub fn description(&self) -> &'static str {
        match self {
            LanguageProfile::Application => "No unsafe, refinements + runtime checks (80% users)",
            LanguageProfile::Systems => "Full language including unsafe (15% users)",
            LanguageProfile::Research => "Dependent types, formal proofs (5% users)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Simple(Text),
    Detailed {
        version: Option<Text>,
        path: Option<PathBuf>,
        git: Option<Text>,
        branch: Option<Text>,
        tag: Option<Text>,
        rev: Option<Text>,
        features: Option<List<Text>>,
        optional: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuildConfig {
    #[serde(default = "default_target")]
    pub target: Text,
    #[serde(default = "default_opt_level")]
    pub opt_level: u8,
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub lto: bool,
    #[serde(default)]
    pub codegen_units: Option<usize>,
    #[serde(default)]
    pub panic: PanicStrategy,
}

fn default_target() -> Text {
    "native".into()
}

fn default_opt_level() -> u8 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanicStrategy {
    #[default]
    Unwind,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    #[serde(default)]
    pub dev: Profile,
    #[serde(default)]
    pub release: Profile,
    #[serde(default)]
    pub test: Profile,
    #[serde(default)]
    pub bench: Profile,
}

// Two-tier compilation modes
// Tier 0: VBC Interpreter (instant start, full diagnostics)
// Tier 1: AOT via LLVM (optimized native binary)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum CompilationTier {
    /// Tier 0: VBC Interpreter (instant start, full diagnostics)
    #[serde(rename = "0", alias = "interpreter", alias = "interp")]
    #[default]
    Interpreter,

    /// Tier 1: AOT compilation via LLVM (production, 85-95% native speed)
    #[serde(rename = "1", alias = "aot", alias = "release", alias = "native")]
    Aot,
}

impl CompilationTier {
    /// Parse tier from numeric value (0-1)
    pub fn from_u8(tier: u8) -> Option<Self> {
        match tier {
            0 => Some(CompilationTier::Interpreter),
            1 => Some(CompilationTier::Aot),
            _ => None,
        }
    }

    /// Parse tier from string (numeric or named)
    /// Accepts: "0", "1", "interpreter", "aot", "release", "native"
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "0" | "interpreter" | "interp" => Some(CompilationTier::Interpreter),
            "1" | "aot" | "release" | "native" => Some(CompilationTier::Aot),
            _ => None,
        }
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            CompilationTier::Interpreter => 0,
            CompilationTier::Aot => 1,
        }
    }

    /// Human-readable name for the tier
    pub fn name(&self) -> &'static str {
        match self {
            CompilationTier::Interpreter => "interpreter",
            CompilationTier::Aot => "aot",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CompilationTier::Interpreter => "VBC Interpreter (instant start, full diagnostics)",
            CompilationTier::Aot => "AOT compilation (production, 85-95% native speed)",
        }
    }

    /// List all valid tier names for help text
    pub fn valid_values() -> &'static str {
        "interpreter|aot (or 0-1)"
    }
}

// Reference system modes
// Three-tier CBGR reference model: Managed (&T, ~15ns checks), Checked
// (&checked T, 0ns compiler-proven), Mixed (auto-select per reference)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReferenceMode {
    #[default]
    Managed, // CBGR checks (~15ns overhead)
    Checked, // Static verification (0ns)
    Mixed,   // Smart selection (recommended)
}

// Verification levels
// Gradual verification: None (no checks), Runtime (default, runtime assertions),
// Proof (formal verification via Z3 SMT solver for refinement types and contracts)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum VerificationLevel {
    None, // No verification (unsafe!)
    #[default]
    Runtime, // Runtime checks (default)
    Proof, // Formal verification
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub tier: CompilationTier,
    #[serde(default)]
    pub verification: VerificationLevel,
    #[serde(default = "default_dev_opt")]
    pub opt_level: u8,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub debug_assertions: bool,
    #[serde(default)]
    pub overflow_checks: bool,
    #[serde(default)]
    pub lto: bool,
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub codegen_units: Option<usize>,
    #[serde(default)]
    pub cbgr_checks: CbgrCheckMode,
}

// CBGR check modes
// All: every reference checked at runtime (~15ns each)
// Optimized: escape analysis eliminates provably-safe checks
// Proven: only emit checks where safety cannot be statically proven
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum CbgrCheckMode {
    #[default]
    All, // All CBGR checks enabled
    Optimized, // Escape analysis optimization
    Proven,    // Only unproven checks
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            tier: CompilationTier::Interpreter,
            verification: VerificationLevel::Runtime,
            opt_level: 0,
            debug: true,
            debug_assertions: true,
            overflow_checks: true,
            lto: false,
            incremental: true,
            codegen_units: Some(256),
            cbgr_checks: CbgrCheckMode::All,
        }
    }
}

// Workspace configuration
// Multi-cog workspace with shared dependencies and unified build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub members: List<Text>,
    #[serde(default)]
    pub exclude: List<Text>,
}

// LSP configuration
// IDE integration settings: CBGR cost hints, refinement validation mode,
// auto-import, format-on-save
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default = "default_true")]
    pub enable_cost_hints: bool,
    #[serde(default = "default_incremental")]
    pub validation_mode: Text,
    #[serde(default = "default_true")]
    pub auto_import: bool,
    #[serde(default)]
    pub format_on_save: bool,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enable_cost_hints: true,
            validation_mode: "incremental".into(),
            auto_import: true,
            format_on_save: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_incremental() -> Text {
    "incremental".into()
}

// Registry configuration
// Cog registry URL and authentication for package distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry")]
    pub index: Text,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            index: default_registry(),
        }
    }
}

fn default_registry() -> Text {
    "https://packages.verum.lang".into()
}

fn default_dev_opt() -> u8 {
    0
}

impl Manifest {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| CliError::ProjectNotFound(path.to_path_buf()))?;

        toml::from_str(&content).map_err(CliError::from)
    }

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn find_manifest_dir() -> Result<PathBuf> {
        let mut current = std::env::current_dir()?;

        loop {
            // Try both verum.toml and Verum.toml for compatibility
            let manifest_path = current.join("verum.toml");
            let manifest_path_alt = current.join("Verum.toml");

            if manifest_path.exists() || manifest_path_alt.exists() {
                return Ok(current);
            }

            if !current.pop() {
                return Err(CliError::ProjectNotFound(PathBuf::from("verum.toml")));
            }
        }
    }

    /// Load manifest from a directory (alias for find + from_file)
    pub fn load<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let dir = dir.as_ref();
        let manifest_path = Self::manifest_path(dir);
        Self::from_file(&manifest_path)
    }

    pub fn manifest_path(dir: &Path) -> PathBuf {
        let verum_path = dir.join("verum.toml");
        if verum_path.exists() {
            verum_path
        } else {
            dir.join("Verum.toml")
        }
    }

    pub fn get_profile(&self, release: bool) -> &Profile {
        if release {
            &self.profile.release
        } else {
            &self.profile.dev
        }
    }

    pub fn all_dependencies(&self) -> List<(&Text, &Dependency)> {
        self.dependencies
            .iter()
            .chain(self.dev_dependencies.iter())
            .chain(self.build_dependencies.iter())
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        // Validate cog name
        if !is_valid_cog_name(self.cog.name.as_str()) {
            return Err(CliError::InvalidProjectName(self.cog.name.to_string()));
        }

        // Validate version
        if semver::Version::parse(self.cog.version.as_str()).is_err() {
            return Err(CliError::Custom(format!(
                "Invalid version: {}",
                self.cog.version
            )));
        }

        // Validate LLVM configuration sections
        self.llvm
            .validate()
            .map_err(|e| CliError::Custom(format!("LLVM config: {}", e)))?;
        self.optimization
            .validate()
            .map_err(|e| CliError::Custom(format!("Optimization config: {}", e)))?;
        self.lto
            .validate()
            .map_err(|e| CliError::Custom(format!("LTO config: {}", e)))?;
        self.pgo
            .validate()
            .map_err(|e| CliError::Custom(format!("PGO config: {}", e)))?;
        self.cross_compile
            .validate()
            .map_err(|e| CliError::Custom(format!("Cross-compile config: {}", e)))?;

        Ok(())
    }
}

pub fn is_valid_cog_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

pub fn create_default_manifest(
    name: &str,
    _is_library: bool,
    profile: LanguageProfile,
) -> Manifest {
    Manifest {
        cog: Cog {
            name: name.into(),
            version: "0.1.0".into(),
            authors: List::new(),
            description: None,
            license: Some("MIT OR Apache-2.0".into()),
            repository: None,
            homepage: None,
            keywords: List::new(),
            categories: List::new(),
        },
        language: LanguageConfig { profile },
        dependencies: {
            let mut deps = Map::new();
            deps.insert("stdlib".into(), Dependency::Simple("0.1".into()));
            deps
        },
        dev_dependencies: Map::new(),
        build_dependencies: Map::new(),
        build: BuildConfig::default(),
        features: {
            let mut features = Map::new();
            features.insert("default".into(), List::new());
            features
        },
        profile: ProfileConfig {
            dev: Profile {
                tier: CompilationTier::Interpreter,
                verification: VerificationLevel::Runtime,
                opt_level: 0,
                debug: true,
                debug_assertions: true,
                overflow_checks: true,
                lto: false,
                incremental: true,
                codegen_units: Some(256),
                cbgr_checks: CbgrCheckMode::All,
            },
            release: Profile {
                tier: CompilationTier::Aot,
                verification: VerificationLevel::Runtime,
                opt_level: 3,
                debug: false,
                debug_assertions: false,
                overflow_checks: false,
                lto: true,
                incremental: false,
                codegen_units: Some(16),
                cbgr_checks: CbgrCheckMode::Optimized,
            },
            test: Profile::default(),
            bench: Profile::default(),
        },
        workspace: None,
        lsp: LspConfig::default(),
        registry: RegistryConfig::default(),
        llvm: LlvmConfig::default(),
        optimization: if profile == LanguageProfile::Research {
            // Research profile: maximum verification, minimal optimization
            OptimizationConfig::debug()
        } else {
            OptimizationConfig::default()
        },
        lto: LtoConfig::default(),
        pgo: PgoConfig::default(),
        cross_compile: CrossCompileConfig::default(),
    }
}

/// Type alias for backwards compatibility
/// Some modules use Config instead of Manifest
pub type Config = Manifest;
