//! Build Assets Intrinsics (Tier 1 - Requires BuildAssets)
//!
//! Provides compile-time file system access with security restrictions.
//! All functions in this module require the `BuildAssets` context since they
//! access the file system.
//!
//! ## File Loading
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `load_text(path)` | `(Text) -> Text` | Load file as text |
//! | `include_bytes(path)` | `(Text) -> Bytes` | Load file as bytes |
//! | `include_str(path)` | `(Text) -> Text` | Alias for load_text |
//!
//! ## File System Operations
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `asset_exists(path)` | `(Text) -> Bool` | Check if file exists |
//! | `asset_list_dir(path)` | `(Text) -> List<Text>` | List directory contents |
//! | `asset_metadata(path)` | `(Text) -> AssetMetadata` | Get file metadata |
//!
//! ## Security
//!
//! All paths are restricted to the project root and configured asset directories.
//! Path traversal (e.g., `..`) and absolute paths are not allowed.
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [BuildAssets]` context.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register build assets builtins with context requirements
///
/// All file system functions require BuildAssets context since they
/// access files from the project directory.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // File Loading (Tier 1 - BuildAssets)
    // ========================================================================

    map.insert(
        Text::from("load_text"),
        BuiltinInfo::build_assets(
            meta_load_text,
            "Load file content as text",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("include_str"),
        BuiltinInfo::build_assets(
            meta_load_text,
            "Load file content as text (alias for load_text)",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("include_bytes"),
        BuiltinInfo::build_assets(
            meta_include_bytes,
            "Load file content as bytes",
            "(Text) -> Bytes",
        ),
    );

    // ========================================================================
    // File System Operations (Tier 1 - BuildAssets)
    // ========================================================================

    map.insert(
        Text::from("asset_exists"),
        BuiltinInfo::build_assets(
            meta_asset_exists,
            "Check if asset file exists",
            "(Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("asset_list_dir"),
        BuiltinInfo::build_assets(
            meta_asset_list_dir,
            "List directory contents",
            "(Text) -> List<Text>",
        ),
    );
    map.insert(
        Text::from("asset_metadata"),
        BuiltinInfo::build_assets(
            meta_asset_metadata,
            "Get file metadata (size, modified, type)",
            "(Text) -> (UInt, UInt, Bool, Bool, Bool)",
        ),
    );
}

// ============================================================================
// File Loading
// ============================================================================

/// Load file content as text
fn meta_load_text(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(path) => {
            let content = ctx.build_assets.load_text(path.as_str())?;
            Ok(ConstValue::Text(content))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Load file content as bytes
fn meta_include_bytes(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(path) => {
            let content = ctx.build_assets.load(path.as_str())?;
            Ok(ConstValue::Bytes(content))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// File System Operations
// ============================================================================

/// Check if file exists
fn meta_asset_exists(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(path) => {
            let exists = ctx.build_assets.exists(path.as_str());
            Ok(ConstValue::Bool(exists))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// List directory contents
fn meta_asset_list_dir(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(path) => {
            let entries = ctx.build_assets.list_dir(path.as_str())?;
            let result: List<ConstValue> = entries
                .iter()
                .map(|e| ConstValue::Text(e.clone()))
                .collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get file metadata
fn meta_asset_metadata(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(path) => {
            let metadata = ctx.build_assets.metadata(path.as_str())?;
            // Return as tuple: (size, modified_ns, is_directory, is_file, is_symlink)
            Ok(ConstValue::Tuple(List::from(vec![
                ConstValue::UInt(metadata.size.into()),
                ConstValue::UInt(metadata.modified_ns.into()),
                ConstValue::Bool(metadata.is_directory),
                ConstValue::Bool(metadata.is_file),
                ConstValue::Bool(metadata.is_symlink),
            ])))
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
    use crate::meta::subsystems::build_assets::BuildAssetsInfo;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_context() -> (MetaContext, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_string_lossy().to_string();

        let mut ctx = MetaContext::new();
        ctx.build_assets = BuildAssetsInfo::new()
            .with_project_root(temp_path);

        (ctx, temp_dir)
    }

    #[test]
    fn test_load_text() {
        let (mut ctx, temp_dir) = create_test_context();

        // Create a test file
        let file_path = temp_dir.path().join("test.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "Hello, World!").unwrap();

        let args = List::from(vec![ConstValue::Text(Text::from("test.txt"))]);
        let result = meta_load_text(&mut ctx, args).unwrap();

        if let ConstValue::Text(content) = result {
            assert!(content.contains("Hello, World!"));
        } else {
            panic!("Expected Text");
        }
    }

    #[test]
    fn test_include_bytes() {
        let (mut ctx, temp_dir) = create_test_context();

        // Create a test file
        let file_path = temp_dir.path().join("test.bin");
        std::fs::write(&file_path, &[0x00, 0x01, 0x02, 0xFF]).unwrap();

        let args = List::from(vec![ConstValue::Text(Text::from("test.bin"))]);
        let result = meta_include_bytes(&mut ctx, args).unwrap();

        if let ConstValue::Bytes(bytes) = result {
            assert_eq!(bytes, vec![0x00, 0x01, 0x02, 0xFF]);
        } else {
            panic!("Expected Bytes");
        }
    }

    #[test]
    fn test_asset_exists() {
        let (mut ctx, temp_dir) = create_test_context();

        // Create a test file
        let file_path = temp_dir.path().join("exists.txt");
        std::fs::write(&file_path, "test").unwrap();

        // Test existing file
        let args = List::from(vec![ConstValue::Text(Text::from("exists.txt"))]);
        let result = meta_asset_exists(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        // Test non-existing file
        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent.txt"))]);
        let result = meta_asset_exists(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_asset_list_dir() {
        let (mut ctx, temp_dir) = create_test_context();

        // Create test files
        std::fs::write(temp_dir.path().join("file1.txt"), "").unwrap();
        std::fs::write(temp_dir.path().join("file2.txt"), "").unwrap();
        std::fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let args = List::from(vec![ConstValue::Text(Text::from("."))]);
        let result = meta_asset_list_dir(&mut ctx, args).unwrap();

        if let ConstValue::Array(entries) = result {
            assert!(entries.len() >= 2); // At least file1.txt and file2.txt
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_asset_metadata() {
        let (mut ctx, temp_dir) = create_test_context();

        // Create a test file with known content
        let file_path = temp_dir.path().join("meta.txt");
        std::fs::write(&file_path, "Hello").unwrap();

        let args = List::from(vec![ConstValue::Text(Text::from("meta.txt"))]);
        let result = meta_asset_metadata(&mut ctx, args).unwrap();

        if let ConstValue::Tuple(fields) = result {
            assert_eq!(fields.len(), 5);
            // size should be 5 bytes
            if let ConstValue::UInt(size) = &fields[0] {
                assert_eq!(*size, 5);
            } else {
                panic!("Expected UInt for size");
            }
            // is_file should be true
            if let ConstValue::Bool(is_file) = &fields[3] {
                assert!(*is_file);
            } else {
                panic!("Expected Bool for is_file");
            }
        } else {
            panic!("Expected Tuple");
        }
    }

    #[test]
    fn test_path_traversal_blocked() {
        let (mut ctx, _temp_dir) = create_test_context();

        // Attempt path traversal
        let args = List::from(vec![ConstValue::Text(Text::from("../etc/passwd"))]);
        let result = meta_load_text(&mut ctx, args);

        assert!(result.is_err());
        if let Err(MetaError::Other(msg)) = result {
            assert!(msg.contains("Path traversal"));
        }
    }

    #[test]
    fn test_absolute_path_blocked() {
        let (mut ctx, _temp_dir) = create_test_context();

        // Attempt absolute path
        let args = List::from(vec![ConstValue::Text(Text::from("/etc/passwd"))]);
        let result = meta_load_text(&mut ctx, args);

        assert!(result.is_err());
        if let Err(MetaError::Other(msg)) = result {
            assert!(msg.contains("Absolute paths"));
        }
    }
}
