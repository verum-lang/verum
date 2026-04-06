//! Language profile system for module-level control.
//!
//! Implements the profile system from Section 13 of the specification:
//! - Module-level profile declarations (@profile attribute)
//! - Profile inheritance from parent modules
//! - Selective feature enabling (@feature attribute)
//! - Profile compatibility validation
//! - Profile-aware module resolution
//!
//! # Overview
//!
//! Verum supports three language profiles:
//! - **Application**: Safe, productive, async-first (default)
//! - **Systems**: Unsafe allowed, manual memory management
//! - **Research**: Formal verification, dependent types, proofs
//!
//! # Profile Hierarchy
//!
//! ```text
//! Research (most permissive)
//!     ↓
//! Systems (intermediate)
//!     ↓
//! Application (most restrictive, default)
//! ```
//!
//! Research profile can access everything.
//! Systems profile can access Systems and Application modules.
//! Application profile can only access Application modules.
//!
//! Profile hierarchy (most permissive to most restrictive):
//! Research > Systems > Application (default)

use crate::ModuleInfo;
use crate::error::{ModuleError, ModuleResult};
use crate::path::ModulePath;
use std::collections::HashSet;
use verum_ast::Span;
use verum_common::{List, Map, Maybe, Text};

/// Language profile - determines what features are available in a module.
///
/// Determines what features are available in a module. Declared with
/// @profile(application|systems|research). Default is Application.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub enum LanguageProfile {
    /// Application profile: Safe, productive, async-first (default).
    ///
    /// Features:
    /// - Full async/await support
    /// - Context system (using/provide)
    /// - Safe references only (&T, &checked T)
    /// - Automatic memory management
    #[default]
    Application,

    /// Systems profile: Maximum control for low-level programming.
    ///
    /// Features (in addition to Application):
    /// - Unsafe blocks and raw pointers
    /// - Manual memory management
    /// - Inline assembly
    /// - Custom allocators
    /// - FFI without wrappers
    Systems,

    /// Research profile: Formal verification and dependent types.
    ///
    /// Features (in addition to Systems):
    /// - Dependent types (Pi/Sigma)
    /// - Formal proofs (@verify)
    /// - Refinement type inference
    /// - SMT solver integration
    /// - Contract verification
    Research,
}

impl LanguageProfile {
    /// Check if this profile can access modules with the target profile.
    ///
    /// Profile compatibility follows the hierarchy:
    /// - Research can access: Research, Systems, Application
    /// - Systems can access: Systems, Application
    /// - Application can access: Application only
    ///
    /// Profile compatibility follows the hierarchy:
    /// Research can access everything, Systems can access Systems+Application,
    /// Application can only access Application.
    pub fn can_access(&self, target: LanguageProfile) -> bool {
        match (self, target) {
            // Research can access everything
            (LanguageProfile::Research, _) => true,
            // Systems can access Systems and Application
            (LanguageProfile::Systems, LanguageProfile::Research) => false,
            (LanguageProfile::Systems, _) => true,
            // Application can only access Application
            (LanguageProfile::Application, LanguageProfile::Application) => true,
            (LanguageProfile::Application, _) => false,
        }
    }

    /// Check if a child module can have this profile given the parent profile.
    ///
    /// Child modules can be MORE restrictive (lower in hierarchy) but NOT less restrictive.
    ///
    /// Profile inheritance rules: child modules can be MORE restrictive
    /// (lower in hierarchy) but NOT less restrictive than parent.
    pub fn can_be_child_of(&self, parent: LanguageProfile) -> bool {
        // Child can be same or more restrictive than parent
        match (parent, self) {
            // Research parent allows any child
            (LanguageProfile::Research, _) => true,
            // Systems parent allows Systems or Application child
            (LanguageProfile::Systems, LanguageProfile::Research) => false,
            (LanguageProfile::Systems, _) => true,
            // Application parent only allows Application child
            (LanguageProfile::Application, LanguageProfile::Application) => true,
            (LanguageProfile::Application, _) => false,
        }
    }

    /// Parse profile from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "application" => Some(LanguageProfile::Application),
            "systems" => Some(LanguageProfile::Systems),
            "research" => Some(LanguageProfile::Research),
            _ => None,
        }
    }

    /// Get the display name of this profile.
    pub fn name(&self) -> &'static str {
        match self {
            LanguageProfile::Application => "application",
            LanguageProfile::Systems => "systems",
            LanguageProfile::Research => "research",
        }
    }

    /// Get unavailable features when accessing from this profile to target.
    ///
    /// Lists features unavailable when accessing from a less permissive profile.
    pub fn unavailable_features(&self, _target: LanguageProfile) -> List<Text> {
        let mut features = List::new();

        match self {
            LanguageProfile::Application => {
                // Application cannot use these features from Systems/Research
                features.push(Text::from("Unsafe pointer operations"));
                features.push(Text::from("Manual memory management"));
                features.push(Text::from("Inline assembly"));
                features.push(Text::from("Raw FFI bindings"));
            }
            LanguageProfile::Systems => {
                // Systems cannot use these features from Research
                features.push(Text::from("Dependent types (Pi/Sigma)"));
                features.push(Text::from("Formal proof verification"));
                features.push(Text::from("SMT-backed contracts"));
            }
            LanguageProfile::Research => {
                // Research has access to everything
            }
        }

        features
    }
}

impl std::fmt::Display for LanguageProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Optional features that can be enabled within a profile.
///
/// Features are independent of profiles and can be selectively enabled
/// using `@feature(enable: [...])` attribute.
///
/// Features are independent of profiles and can be selectively enabled
/// using `@feature(enable: [...])`. Features must be compatible with the
/// base profile and are additive (don't remove profile capabilities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ModuleFeature {
    /// Allow unsafe blocks in this module
    Unsafe,
    /// Allow inline assembly
    InlineAsm,
    /// Allow custom allocators
    CustomAllocator,
    /// Enable dependent types
    DependentTypes,
    /// Enable formal verification
    FormalVerification,
    /// Enable SMT solver integration
    SmtSolver,
    /// Enable GPU compute
    GpuCompute,
    /// Enable SIMD intrinsics
    Simd,
    /// Enable FFI without wrappers
    RawFfi,
}

impl ModuleFeature {
    /// Parse feature from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "unsafe" => Some(ModuleFeature::Unsafe),
            "inline_asm" | "inlineasm" => Some(ModuleFeature::InlineAsm),
            "custom_allocator" | "customallocator" => Some(ModuleFeature::CustomAllocator),
            "dependent_types" | "dependenttypes" => Some(ModuleFeature::DependentTypes),
            "formal_verification" | "formalverification" => Some(ModuleFeature::FormalVerification),
            "smt_solver" | "smtsolver" | "smt" => Some(ModuleFeature::SmtSolver),
            "gpu_compute" | "gpucompute" | "gpu" => Some(ModuleFeature::GpuCompute),
            "simd" => Some(ModuleFeature::Simd),
            "raw_ffi" | "rawffi" => Some(ModuleFeature::RawFfi),
            _ => None,
        }
    }

    /// Get the name of this feature.
    pub fn name(&self) -> &'static str {
        match self {
            ModuleFeature::Unsafe => "unsafe",
            ModuleFeature::InlineAsm => "inline_asm",
            ModuleFeature::CustomAllocator => "custom_allocator",
            ModuleFeature::DependentTypes => "dependent_types",
            ModuleFeature::FormalVerification => "formal_verification",
            ModuleFeature::SmtSolver => "smt_solver",
            ModuleFeature::GpuCompute => "gpu_compute",
            ModuleFeature::Simd => "simd",
            ModuleFeature::RawFfi => "raw_ffi",
        }
    }

    /// Check if this feature is compatible with the given profile.
    ///
    /// Some features require certain profiles to be used.
    pub fn is_compatible_with(&self, profile: LanguageProfile) -> bool {
        match self {
            // These features require at least Systems profile
            ModuleFeature::Unsafe
            | ModuleFeature::InlineAsm
            | ModuleFeature::CustomAllocator
            | ModuleFeature::RawFfi => matches!(
                profile,
                LanguageProfile::Systems | LanguageProfile::Research
            ),

            // These features require Research profile
            ModuleFeature::DependentTypes
            | ModuleFeature::FormalVerification
            | ModuleFeature::SmtSolver => matches!(profile, LanguageProfile::Research),

            // These features work with any profile
            ModuleFeature::GpuCompute | ModuleFeature::Simd => true,
        }
    }

    /// Get the minimum required profile for this feature.
    pub fn minimum_profile(&self) -> LanguageProfile {
        match self {
            ModuleFeature::Unsafe
            | ModuleFeature::InlineAsm
            | ModuleFeature::CustomAllocator
            | ModuleFeature::RawFfi => LanguageProfile::Systems,

            ModuleFeature::DependentTypes
            | ModuleFeature::FormalVerification
            | ModuleFeature::SmtSolver => LanguageProfile::Research,

            ModuleFeature::GpuCompute | ModuleFeature::Simd => LanguageProfile::Application,
        }
    }
}

impl std::fmt::Display for ModuleFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Profile configuration for a module.
///
/// Stores the declared profile and any enabled features.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleProfile {
    /// The declared language profile
    pub profile: LanguageProfile,
    /// Explicitly enabled features
    pub enabled_features: HashSet<ModuleFeature>,
    /// Whether this profile was explicitly declared or inherited
    pub explicit: bool,
    /// Span of the @profile attribute (if explicit)
    pub span: Option<Span>,
}

impl ModuleProfile {
    /// Create a new module profile with explicit declaration.
    pub fn new(profile: LanguageProfile) -> Self {
        Self {
            profile,
            enabled_features: HashSet::new(),
            explicit: true,
            span: None,
        }
    }

    /// Create an inherited profile from parent.
    pub fn inherited(parent_profile: LanguageProfile) -> Self {
        Self {
            profile: parent_profile,
            enabled_features: HashSet::new(),
            explicit: false,
            span: None,
        }
    }

    /// Create default Application profile.
    pub fn default_application() -> Self {
        Self {
            profile: LanguageProfile::Application,
            enabled_features: HashSet::new(),
            explicit: false,
            span: None,
        }
    }

    /// Enable a feature in this profile.
    pub fn enable_feature(&mut self, feature: ModuleFeature) -> ModuleResult<()> {
        if !feature.is_compatible_with(self.profile) {
            return Err(ModuleError::Other {
                message: Text::from(format!(
                    "Feature '{}' requires {} profile, but module has {} profile",
                    feature,
                    feature.minimum_profile(),
                    self.profile
                )),
                span: self.span,
            });
        }
        self.enabled_features.insert(feature);
        Ok(())
    }

    /// Check if a feature is enabled.
    pub fn has_feature(&self, feature: ModuleFeature) -> bool {
        // Feature is available if:
        // 1. Explicitly enabled via @feature
        // 2. OR profile inherently supports it (e.g., Systems supports Unsafe)
        self.enabled_features.contains(&feature) || self.profile_supports_feature(feature)
    }

    /// Check if the profile inherently supports a feature.
    fn profile_supports_feature(&self, feature: ModuleFeature) -> bool {
        match self.profile {
            LanguageProfile::Research => true, // Research supports all features
            LanguageProfile::Systems => matches!(
                feature,
                ModuleFeature::Unsafe
                    | ModuleFeature::InlineAsm
                    | ModuleFeature::CustomAllocator
                    | ModuleFeature::RawFfi
                    | ModuleFeature::GpuCompute
                    | ModuleFeature::Simd
            ),
            LanguageProfile::Application => {
                matches!(feature, ModuleFeature::GpuCompute | ModuleFeature::Simd)
            }
        }
    }

    /// Set the span for error reporting.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

impl Default for ModuleProfile {
    fn default() -> Self {
        Self::default_application()
    }
}

/// Profile checker - validates profile compatibility across modules.
///
/// Implements profile validation from Section 13.5-13.6:
/// - Validates module profile declarations
/// - Checks profile inheritance rules
/// - Validates import compatibility
/// - Generates detailed error messages
///
/// Validates profile compatibility across modules using the profile
/// resolution algorithm: (1) check module declaration, (2) check
/// inheritance chain, (3) check feature gates, (4) validate imports,
/// (5) error if incompatible with detailed resolution suggestions.
#[derive(Debug)]
pub struct ProfileChecker {
    /// Current compilation profile (from command line or verum.toml)
    compilation_profile: LanguageProfile,
    /// Module profiles cache
    module_profiles: Map<ModulePath, ModuleProfile>,
}

impl ProfileChecker {
    /// Create a new profile checker with the given compilation profile.
    pub fn new(compilation_profile: LanguageProfile) -> Self {
        Self {
            compilation_profile,
            module_profiles: Map::new(),
        }
    }

    /// Register a module's profile.
    pub fn register_profile(&mut self, path: ModulePath, profile: ModuleProfile) {
        self.module_profiles.insert(path, profile);
    }

    /// Get a module's profile.
    pub fn get_profile(&self, path: &ModulePath) -> Maybe<&ModuleProfile> {
        match self.module_profiles.get(path) {
            Some(p) => Maybe::Some(p),
            None => Maybe::None,
        }
    }

    /// Extract profile from module's attributes.
    ///
    /// Parses @profile() and @feature() attributes from the module AST.
    /// Attributes are stored in each Item's attributes field, not as separate items.
    pub fn extract_profile(
        &self,
        module: &ModuleInfo,
        parent_profile: Option<LanguageProfile>,
    ) -> ModuleResult<ModuleProfile> {
        let mut profile = parent_profile
            .map(ModuleProfile::inherited)
            .unwrap_or_else(ModuleProfile::default_application);

        // Look for @profile and @feature attributes in module's items
        for item in &module.ast.items {
            for attr in &item.attributes {
                // Check for @profile attribute
                if attr.name.as_str() == "profile" {
                    if let Maybe::Some(ref args) = attr.args
                        && let Some(first_arg) = args.first()
                        && let Some(profile_name) = self.extract_expr_string(first_arg)
                        && let Some(lang_profile) = LanguageProfile::from_str(&profile_name)
                    {
                        // Validate profile inheritance
                        if let Some(parent) = parent_profile
                            && !lang_profile.can_be_child_of(parent)
                        {
                            return Err(ModuleError::ProfileIncompatible {
                                module_path: module.path.clone(),
                                required_profile: Text::from(parent.name()),
                                current_profile: Text::from(lang_profile.name()),
                                span: Some(item.span),
                            });
                        }
                        profile.profile = lang_profile;
                        profile.explicit = true;
                        profile.span = Some(attr.span);
                    }
                }
                // Check for @feature attribute
                else if attr.name.as_str() == "feature" {
                    self.parse_feature_attr(attr, &mut profile)?;
                }
            }
        }

        // Also check module-level attributes (on the Module itself)
        for attr in &module.ast.attributes {
            if attr.name.as_str() == "profile" {
                if let Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                    && let Some(profile_name) = self.extract_expr_string(first_arg)
                    && let Some(lang_profile) = LanguageProfile::from_str(&profile_name)
                {
                    if let Some(parent) = parent_profile
                        && !lang_profile.can_be_child_of(parent)
                    {
                        return Err(ModuleError::ProfileIncompatible {
                            module_path: module.path.clone(),
                            required_profile: Text::from(parent.name()),
                            current_profile: Text::from(lang_profile.name()),
                            span: Some(attr.span),
                        });
                    }
                    profile.profile = lang_profile;
                    profile.explicit = true;
                    profile.span = Some(attr.span);
                }
            } else if attr.name.as_str() == "feature" {
                self.parse_feature_attr(attr, &mut profile)?;
            }
        }

        Ok(profile)
    }

    /// Parse @feature(enable: [...]) attribute.
    fn parse_feature_attr(
        &self,
        attr: &verum_ast::Attribute,
        profile: &mut ModuleProfile,
    ) -> ModuleResult<()> {
        if let Maybe::Some(ref args) = attr.args {
            for arg in args {
                // Look for array arguments with feature names
                if let Some(features) = self.extract_expr_string_list(arg) {
                    for feature_name in features {
                        if let Some(feature) = ModuleFeature::from_str(&feature_name) {
                            profile.enable_feature(feature)?;
                        } else {
                            return Err(ModuleError::Other {
                                message: Text::from(format!("Unknown feature: '{}'", feature_name)),
                                span: Some(attr.span),
                            });
                        }
                    }
                } else if let Some(feature_name) = self.extract_expr_string(arg) {
                    // Single feature name
                    if let Some(feature) = ModuleFeature::from_str(&feature_name) {
                        profile.enable_feature(feature)?;
                    } else {
                        return Err(ModuleError::Other {
                            message: Text::from(format!("Unknown feature: '{}'", feature_name)),
                            span: Some(attr.span),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract string value from an expression.
    fn extract_expr_string(&self, expr: &verum_ast::expr::Expr) -> Option<String> {
        use verum_ast::LiteralKind;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Text(s) => Some(s.as_str().to_string()),
                _ => None,
            },
            ExprKind::Path(path) => {
                // Handle identifier-style arguments like @profile(application)
                if path.segments.len() == 1
                    && let verum_ast::PathSegment::Name(ident) = &path.segments[0]
                {
                    return Some(ident.name.to_string());
                }
                None
            }
            _ => None,
        }
    }

    /// Extract list of strings from an array expression.
    fn extract_expr_string_list(&self, expr: &verum_ast::expr::Expr) -> Option<List<String>> {
        use verum_ast::ArrayExpr;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Array(array_expr) => match array_expr {
                ArrayExpr::List(elements) => {
                    let strings: List<String> = elements
                        .iter()
                        .filter_map(|e| self.extract_expr_string(e))
                        .collect();
                    if strings.is_empty() {
                        None
                    } else {
                        Some(strings)
                    }
                }
                ArrayExpr::Repeat { .. } => None, // Repeat arrays don't make sense for feature lists
            },
            ExprKind::Tuple(elements) => {
                let strings: List<String> = elements
                    .iter()
                    .filter_map(|e| self.extract_expr_string(e))
                    .collect();
                if strings.is_empty() {
                    None
                } else {
                    Some(strings)
                }
            }
            _ => None,
        }
    }

    /// Check if importing from target module is allowed.
    ///
    /// Validates that the current module's profile can access the target module.
    ///
    /// Profile compatibility follows the hierarchy:
    /// Research can access everything, Systems can access Systems+Application,
    /// Application can only access Application.
    pub fn check_import(
        &self,
        from_module: &ModulePath,
        target_module: &ModulePath,
        span: Option<Span>,
    ) -> ModuleResult<()> {
        let from_profile = self.get_effective_profile(from_module);
        let target_profile = self.get_effective_profile(target_module);

        if !from_profile.can_access(target_profile) {
            let _unavailable = from_profile.unavailable_features(target_profile);

            return Err(ModuleError::ProfileIncompatible {
                module_path: target_module.clone(),
                required_profile: Text::from(target_profile.name()),
                current_profile: Text::from(from_profile.name()),
                span,
            });
        }

        Ok(())
    }

    /// Get the effective profile for a module.
    ///
    /// Returns the module's declared profile, inherited profile, or compilation default.
    pub fn get_effective_profile(&self, path: &ModulePath) -> LanguageProfile {
        if let Maybe::Some(profile) = self.get_profile(path) {
            return profile.profile;
        }

        // Try to inherit from parent
        if let Maybe::Some(parent) = path.parent() {
            return self.get_effective_profile(&parent);
        }

        // Fall back to compilation profile
        self.compilation_profile
    }

    /// Check if an unsafe block is allowed in the module.
    pub fn is_unsafe_allowed(&self, module_path: &ModulePath) -> bool {
        if let Maybe::Some(profile) = self.get_profile(module_path) {
            return profile.has_feature(ModuleFeature::Unsafe);
        }

        // Check if compilation profile allows unsafe
        matches!(
            self.compilation_profile,
            LanguageProfile::Systems | LanguageProfile::Research
        )
    }

    /// Validate all registered module profiles.
    ///
    /// Checks:
    /// 1. Profile inheritance is valid (child not less restrictive than parent)
    /// 2. All enabled features are compatible with profile
    /// 3. All imports are compatible with profile
    pub fn validate_all(&self) -> ModuleResult<()> {
        for (path, profile) in self.module_profiles.iter() {
            // Check parent compatibility
            if let Maybe::Some(parent_path) = path.parent() {
                let parent_profile = self.get_effective_profile(&parent_path);
                if !profile.profile.can_be_child_of(parent_profile) {
                    return Err(ModuleError::ProfileIncompatible {
                        module_path: path.clone(),
                        required_profile: Text::from(format!(
                            "{} or more restrictive",
                            parent_profile
                        )),
                        current_profile: Text::from(profile.profile.name()),
                        span: profile.span,
                    });
                }
            }

            // Validate features are compatible
            for feature in &profile.enabled_features {
                if !feature.is_compatible_with(profile.profile) {
                    return Err(ModuleError::Other {
                        message: Text::from(format!(
                            "Feature '{}' in module '{}' requires {} profile, but has {} profile",
                            feature,
                            path,
                            feature.minimum_profile(),
                            profile.profile
                        )),
                        span: profile.span,
                    });
                }
            }
        }

        Ok(())
    }

    /// Get the compilation profile.
    pub fn compilation_profile(&self) -> LanguageProfile {
        self.compilation_profile
    }

    /// Generate detailed error message for profile incompatibility.
    ///
    /// Generates detailed error including: required profile, current profile,
    /// unavailable features list, and resolution suggestions (change verum.toml,
    /// move module, or use compatible public APIs).
    pub fn format_profile_error(
        &self,
        from_module: &ModulePath,
        target_module: &ModulePath,
    ) -> Text {
        let from_profile = self.get_effective_profile(from_module);
        let target_profile = self.get_effective_profile(target_module);
        let unavailable = from_profile.unavailable_features(target_profile);

        let mut msg = format!(
            "ERROR: Module incompatible with current profile\n\n\
             Module '{}' requires one of: [{}]\n\
             Current compilation profile: {}\n",
            target_module, target_profile, from_profile
        );

        if !unavailable.is_empty() {
            msg.push_str("\nThe following features are unavailable:\n");
            for feature in unavailable {
                msg.push_str(&format!("  - {}\n", feature));
            }
        }

        msg.push_str(&format!(
            "\nHelp: Either:\n\
             1. Change verum.toml profile = \"{}\"\n\
             2. Move this module to a {}-only submodule\n\
             3. Use public APIs from {}-compatible modules\n",
            target_profile, target_profile, from_profile
        ));

        Text::from(msg)
    }
}

impl Default for ProfileChecker {
    fn default() -> Self {
        Self::new(LanguageProfile::Application)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_hierarchy() {
        // Research can access everything
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Research));
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Application));

        // Systems can access Systems and Application
        assert!(!LanguageProfile::Systems.can_access(LanguageProfile::Research));
        assert!(LanguageProfile::Systems.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Systems.can_access(LanguageProfile::Application));

        // Application can only access Application
        assert!(!LanguageProfile::Application.can_access(LanguageProfile::Research));
        assert!(!LanguageProfile::Application.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Application.can_access(LanguageProfile::Application));
    }

    #[test]
    fn test_profile_inheritance() {
        // Research parent allows any child
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Research));
        assert!(LanguageProfile::Systems.can_be_child_of(LanguageProfile::Research));
        assert!(LanguageProfile::Research.can_be_child_of(LanguageProfile::Research));

        // Systems parent allows Systems or Application
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Systems));
        assert!(LanguageProfile::Systems.can_be_child_of(LanguageProfile::Systems));
        assert!(!LanguageProfile::Research.can_be_child_of(LanguageProfile::Systems));

        // Application parent only allows Application
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Application));
        assert!(!LanguageProfile::Systems.can_be_child_of(LanguageProfile::Application));
        assert!(!LanguageProfile::Research.can_be_child_of(LanguageProfile::Application));
    }

    #[test]
    fn test_feature_compatibility() {
        // Unsafe requires Systems
        assert!(!ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Application));
        assert!(ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Research));

        // DependentTypes requires Research
        assert!(!ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Application));
        assert!(!ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Research));

        // SIMD works everywhere
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Application));
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Research));
    }

    #[test]
    fn test_module_profile_features() {
        let mut profile = ModuleProfile::new(LanguageProfile::Systems);

        // Systems profile inherently supports Unsafe
        assert!(profile.has_feature(ModuleFeature::Unsafe));
        assert!(profile.has_feature(ModuleFeature::Simd));

        // Systems profile does NOT support DependentTypes
        assert!(!profile.has_feature(ModuleFeature::DependentTypes));

        // Cannot enable incompatible feature
        assert!(
            profile
                .enable_feature(ModuleFeature::DependentTypes)
                .is_err()
        );
    }

    #[test]
    fn test_profile_checker_import_validation() {
        let mut checker = ProfileChecker::new(LanguageProfile::Application);

        let app_module = ModulePath::from_str("app.main");
        let sys_module = ModulePath::from_str("sys.memory");

        checker.register_profile(
            app_module.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        checker.register_profile(
            sys_module.clone(),
            ModuleProfile::new(LanguageProfile::Systems),
        );

        // App cannot import from Systems
        assert!(
            checker
                .check_import(&app_module, &sys_module, None)
                .is_err()
        );

        // Systems can import from App (hypothetically if we change the compilation profile)
        let mut sys_checker = ProfileChecker::new(LanguageProfile::Systems);
        sys_checker.register_profile(
            app_module.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        sys_checker.register_profile(
            sys_module.clone(),
            ModuleProfile::new(LanguageProfile::Systems),
        );

        assert!(
            sys_checker
                .check_import(&sys_module, &app_module, None)
                .is_ok()
        );
    }

    #[test]
    fn test_profile_from_str() {
        assert_eq!(
            LanguageProfile::from_str("application"),
            Some(LanguageProfile::Application)
        );
        assert_eq!(
            LanguageProfile::from_str("APPLICATION"),
            Some(LanguageProfile::Application)
        );
        assert_eq!(
            LanguageProfile::from_str("systems"),
            Some(LanguageProfile::Systems)
        );
        assert_eq!(
            LanguageProfile::from_str("research"),
            Some(LanguageProfile::Research)
        );
        assert_eq!(LanguageProfile::from_str("invalid"), None);
    }

    #[test]
    fn test_feature_from_str() {
        assert_eq!(
            ModuleFeature::from_str("unsafe"),
            Some(ModuleFeature::Unsafe)
        );
        assert_eq!(
            ModuleFeature::from_str("inline_asm"),
            Some(ModuleFeature::InlineAsm)
        );
        assert_eq!(
            ModuleFeature::from_str("inlineasm"),
            Some(ModuleFeature::InlineAsm)
        );
        assert_eq!(
            ModuleFeature::from_str("smt"),
            Some(ModuleFeature::SmtSolver)
        );
        assert_eq!(
            ModuleFeature::from_str("gpu"),
            Some(ModuleFeature::GpuCompute)
        );
        assert_eq!(ModuleFeature::from_str("invalid"), None);
    }

    #[test]
    fn test_profile_checker_validate_all() {
        let mut checker = ProfileChecker::new(LanguageProfile::Systems);

        let parent = ModulePath::from_str("parent");
        let child = ModulePath::from_str("parent.child");

        // Valid: Systems parent with Application child
        checker.register_profile(parent.clone(), ModuleProfile::new(LanguageProfile::Systems));
        checker.register_profile(
            child.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );

        assert!(checker.validate_all().is_ok());
    }

    #[test]
    fn test_profile_checker_invalid_inheritance() {
        let mut checker = ProfileChecker::new(LanguageProfile::Application);

        let parent = ModulePath::from_str("parent");
        let child = ModulePath::from_str("parent.child");

        // Invalid: Application parent with Systems child
        checker.register_profile(
            parent.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        checker.register_profile(child.clone(), ModuleProfile::new(LanguageProfile::Systems));

        assert!(checker.validate_all().is_err());
    }
}
