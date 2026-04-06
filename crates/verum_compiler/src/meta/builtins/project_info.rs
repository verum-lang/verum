//! Project Info Intrinsics (Tier 1 - Requires ProjectInfo)
//!
//! Provides compile-time access to project metadata from Verum.toml.
//! All functions require the `ProjectInfo` context.
//!
//! ## Package Metadata
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `project_package_name()` | `() -> Text` | Package name from Verum.toml |
//! | `project_package_version()` | `() -> Text` | Package version |
//! | `project_package_authors()` | `() -> List<Text>` | Package authors |
//!
//! ## Dependencies
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `project_dependencies()` | `() -> List<(Text, Text)>` | All dependencies |
//! | `project_has_dependency(name)` | `(Text) -> Bool` | Check dependency exists |
//!
//! ## Build Configuration
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `project_target_os()` | `() -> Text` | Target OS |
//! | `project_target_arch()` | `() -> Text` | Target architecture |
//! | `project_is_debug()` | `() -> Bool` | Debug build? |
//! | `project_is_release()` | `() -> Bool` | Release build? |
//! | `project_root()` | `() -> Text` | Project root directory |
//! | `project_source_dir()` | `() -> Text` | Source directory |
//! | `project_enabled_features()` | `() -> List<Text>` | Enabled features |
//! | `project_is_feature_enabled(f)` | `(Text) -> Bool` | Check feature enabled |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [ProjectInfo]` context.

use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register project info builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Package Metadata (Tier 1 - ProjectInfo)
    // ========================================================================

    map.insert(
        Text::from("project_package_name"),
        BuiltinInfo::project_info(
            meta_project_package_name,
            "Get the package name from Verum.toml",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_package_version"),
        BuiltinInfo::project_info(
            meta_project_package_version,
            "Get the package version from Verum.toml",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_package_authors"),
        BuiltinInfo::project_info(
            meta_project_package_authors,
            "Get the package authors from Verum.toml",
            "() -> List<Text>",
        ),
    );

    // ========================================================================
    // Dependencies (Tier 1 - ProjectInfo)
    // ========================================================================

    map.insert(
        Text::from("project_dependencies"),
        BuiltinInfo::project_info(
            meta_project_dependencies,
            "Get all dependencies as (name, version) pairs",
            "() -> List<(Text, Text)>",
        ),
    );
    map.insert(
        Text::from("project_has_dependency"),
        BuiltinInfo::project_info(
            meta_project_has_dependency,
            "Check if a dependency exists by name",
            "(Text) -> Bool",
        ),
    );

    // ========================================================================
    // Build Configuration (Tier 1 - ProjectInfo)
    // ========================================================================

    map.insert(
        Text::from("project_target_os"),
        BuiltinInfo::project_info(
            meta_project_target_os,
            "Get the target operating system",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_target_arch"),
        BuiltinInfo::project_info(
            meta_project_target_arch,
            "Get the target architecture",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_is_debug"),
        BuiltinInfo::project_info(
            meta_project_is_debug,
            "Check if this is a debug build",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("project_is_release"),
        BuiltinInfo::project_info(
            meta_project_is_release,
            "Check if this is a release build",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("project_root"),
        BuiltinInfo::project_info(
            meta_project_root,
            "Get the project root directory path",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_source_dir"),
        BuiltinInfo::project_info(
            meta_project_source_dir,
            "Get the project source directory path",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("project_enabled_features"),
        BuiltinInfo::project_info(
            meta_project_enabled_features,
            "Get all enabled feature flags",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("project_is_feature_enabled"),
        BuiltinInfo::project_info(
            meta_project_is_feature_enabled,
            "Check if a specific feature flag is enabled",
            "(Text) -> Bool",
        ),
    );
}

// ============================================================================
// Package Metadata
// ============================================================================

/// Get the package name from Verum.toml
fn meta_project_package_name(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Text(ctx.project_info.name.clone()))
}

/// Get the package version from Verum.toml
fn meta_project_package_version(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Text(ctx.project_info.version.clone()))
}

/// Get the package authors from Verum.toml
fn meta_project_package_authors(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let authors: List<ConstValue> = ctx
        .project_info
        .authors
        .iter()
        .map(|a| ConstValue::Text(a.clone()))
        .collect();
    Ok(ConstValue::Array(authors))
}

// ============================================================================
// Dependencies
// ============================================================================

/// Get all dependencies as (name, version) pairs
fn meta_project_dependencies(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let deps: List<ConstValue> = ctx
        .project_info
        .dependencies
        .iter()
        .map(|(name, version)| {
            ConstValue::Tuple(List::from(vec![
                ConstValue::Text(name.clone()),
                ConstValue::Text(version.clone()),
            ]))
        })
        .collect();
    Ok(ConstValue::Array(deps))
}

/// Check if a dependency exists by name
fn meta_project_has_dependency(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }
    match &args[0] {
        ConstValue::Text(name) => {
            let exists = ctx.project_info.dependencies.contains_key(name)
                || ctx.project_info.dev_dependencies.contains_key(name);
            Ok(ConstValue::Bool(exists))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Build Configuration
// ============================================================================

/// Get the target operating system
fn meta_project_target_os(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let os = if ctx.project_info.target_os.is_empty() {
        Text::from(std::env::consts::OS)
    } else {
        ctx.project_info.target_os.clone()
    };
    Ok(ConstValue::Text(os))
}

/// Get the target architecture
fn meta_project_target_arch(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let arch = if ctx.project_info.target_arch.is_empty() {
        Text::from(std::env::consts::ARCH)
    } else {
        ctx.project_info.target_arch.clone()
    };
    Ok(ConstValue::Text(arch))
}

/// Check if this is a debug build
fn meta_project_is_debug(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Bool(ctx.project_info.is_debug))
}

/// Check if this is a release build
fn meta_project_is_release(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Bool(!ctx.project_info.is_debug))
}

/// Get the project root directory path
fn meta_project_root(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Text(ctx.project_info.project_root.clone()))
}

/// Get the project source directory path
fn meta_project_source_dir(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let source_dir = if ctx.project_info.source_dir.is_empty() {
        // Default to "src" subdirectory of project root
        if ctx.project_info.project_root.is_empty() {
            Text::from("src")
        } else {
            Text::from(format!("{}/src", ctx.project_info.project_root))
        }
    } else {
        ctx.project_info.source_dir.clone()
    };
    Ok(ConstValue::Text(source_dir))
}

/// Get all enabled feature flags
fn meta_project_enabled_features(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    let features: List<ConstValue> = ctx
        .project_info
        .enabled_features
        .iter()
        .map(|f| ConstValue::Text(f.clone()))
        .collect();
    Ok(ConstValue::Array(features))
}

/// Check if a specific feature flag is enabled
fn meta_project_is_feature_enabled(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }
    match &args[0] {
        ConstValue::Text(feature) => {
            let enabled = ctx
                .project_info
                .enabled_features
                .iter()
                .any(|f| f == feature);
            Ok(ConstValue::Bool(enabled))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> MetaContext {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts
            .enable(super::super::context_requirements::RequiredContext::ProjectInfo);

        // Set up project info
        ctx.project_info.name = Text::from("my_project");
        ctx.project_info.version = Text::from("1.2.3");
        ctx.project_info.authors = List::from(vec![
            Text::from("Alice"),
            Text::from("Bob"),
        ]);
        ctx.project_info.is_debug = true;
        ctx.project_info.project_root = Text::from("/home/user/project");
        ctx.project_info.source_dir = Text::from("/home/user/project/src");
        ctx.project_info
            .dependencies
            .insert(Text::from("serde"), Text::from("1.0"));
        ctx.project_info
            .dependencies
            .insert(Text::from("tokio"), Text::from("1.28"));
        ctx.project_info
            .enabled_features
            .push(Text::from("async"));
        ctx.project_info
            .enabled_features
            .push(Text::from("simd"));

        ctx
    }

    #[test]
    fn test_package_name() {
        let mut ctx = create_test_context();
        let result = meta_project_package_name(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("my_project")));
    }

    #[test]
    fn test_package_version() {
        let mut ctx = create_test_context();
        let result = meta_project_package_version(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("1.2.3")));
    }

    #[test]
    fn test_package_authors() {
        let mut ctx = create_test_context();
        let result = meta_project_package_authors(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(authors) = result {
            assert_eq!(authors.len(), 2);
            assert_eq!(authors[0], ConstValue::Text(Text::from("Alice")));
            assert_eq!(authors[1], ConstValue::Text(Text::from("Bob")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_dependencies() {
        let mut ctx = create_test_context();
        let result = meta_project_dependencies(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(deps) = result {
            assert_eq!(deps.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_has_dependency() {
        let mut ctx = create_test_context();

        let args = List::from(vec![ConstValue::Text(Text::from("serde"))]);
        let result = meta_project_has_dependency(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_project_has_dependency(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_is_debug_release() {
        let mut ctx = create_test_context();

        let result = meta_project_is_debug(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        let result = meta_project_is_release(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_project_root() {
        let mut ctx = create_test_context();
        let result = meta_project_root(&mut ctx, List::new()).unwrap();
        assert_eq!(
            result,
            ConstValue::Text(Text::from("/home/user/project"))
        );
    }

    #[test]
    fn test_source_dir() {
        let mut ctx = create_test_context();
        let result = meta_project_source_dir(&mut ctx, List::new()).unwrap();
        assert_eq!(
            result,
            ConstValue::Text(Text::from("/home/user/project/src"))
        );
    }

    #[test]
    fn test_enabled_features() {
        let mut ctx = create_test_context();
        let result = meta_project_enabled_features(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(features) = result {
            assert_eq!(features.len(), 2);
            assert_eq!(features[0], ConstValue::Text(Text::from("async")));
            assert_eq!(features[1], ConstValue::Text(Text::from("simd")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_is_feature_enabled() {
        let mut ctx = create_test_context();

        let args = List::from(vec![ConstValue::Text(Text::from("async"))]);
        let result = meta_project_is_feature_enabled(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        let args = List::from(vec![ConstValue::Text(Text::from("gpu"))]);
        let result = meta_project_is_feature_enabled(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_target_os_fallback() {
        let mut ctx = create_test_context();
        // target_os is empty by default, should fall back to host OS
        ctx.project_info.target_os = Text::from("");
        let result = meta_project_target_os(&mut ctx, List::new()).unwrap();
        if let ConstValue::Text(os) = result {
            assert!(!os.is_empty());
        } else {
            panic!("Expected Text");
        }
    }

    #[test]
    fn test_target_arch_fallback() {
        let mut ctx = create_test_context();
        ctx.project_info.target_arch = Text::from("");
        let result = meta_project_target_arch(&mut ctx, List::new()).unwrap();
        if let ConstValue::Text(arch) = result {
            assert!(!arch.is_empty());
        } else {
            panic!("Expected Text");
        }
    }
}
