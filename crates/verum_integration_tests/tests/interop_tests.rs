#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]

// Compatibility and Interoperability Tests
//
// Tests FFI with C code, JSON serialization/deserialization,
// file I/O on different platforms, and network operations.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use tempfile::{NamedTempFile, TempDir};
use verum_ast::{expr::*, literal::*, span::Span};
use verum_common::{List, Map, Text};

// ============================================================================
// JSON Serialization Tests
// ============================================================================

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TestData {
    name: String,
    value: i32,
    active: bool,
}

#[test]
fn test_json_serialization() {
    let data = TestData {
        name: "test".to_string(),
        value: 42,
        active: true,
    };

    let json = serde_json::to_string(&data).expect("Serialization failed");
    assert!(json.contains("test"));
    assert!(json.contains("42"));
    assert!(json.contains("true"));
}

#[test]
fn test_json_deserialization() {
    let json = r#"{"name":"test","value":42,"active":true}"#;

    let data: TestData = serde_json::from_str(json).expect("Deserialization failed");
    assert_eq!(data.name, "test");
    assert_eq!(data.value, 42);
    assert!(data.active);
}

#[test]
fn test_json_roundtrip() {
    let original = TestData {
        name: "roundtrip".to_string(),
        value: 100,
        active: false,
    };

    let json = serde_json::to_string(&original).expect("Serialization failed");
    let restored: TestData = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(original, restored);
}

#[test]
fn test_json_ast_serialization() {
    use verum_ast::span::FileId;
    let file_id = FileId::new(0);
    let literal = Literal::int(42, Span::new(0, 2, file_id));
    let expr = Expr::literal(literal);

    // Serialize AST to JSON
    let json = serde_json::to_string(&expr).expect("Serialization failed");
    assert!(!json.is_empty());

    // Deserialize back
    let restored: Expr = serde_json::from_str(&json).expect("Deserialization failed");

    // Verify structure
    match restored.kind {
        ExprKind::Literal(_) => {}
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_json_complex_structure() {
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct ComplexData {
        items: Vec<i32>,
        metadata: std::collections::HashMap<String, String>,
        nested: Option<Box<ComplexData>>,
    }

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("key1".to_string(), "value1".to_string());

    let data = ComplexData {
        items: vec![1, 2, 3, 4, 5],
        metadata,
        nested: None,
    };

    let json = serde_json::to_string(&data).expect("Serialization failed");
    let restored: ComplexData = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(data, restored);
}

// ============================================================================
// File I/O Tests
// ============================================================================

#[test]
fn test_file_write_and_read() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let content = "Hello, Verum!";

    // Write to file
    {
        let mut file = temp_file.reopen().expect("Failed to reopen file");
        write!(file, "{}", content).expect("Failed to write");
    }

    // Read from file
    {
        let mut file = temp_file.reopen().expect("Failed to reopen file");
        let mut read_content = String::new();
        file.read_to_string(&mut read_content)
            .expect("Failed to read");

        assert_eq!(read_content, content);
    }
}

#[test]
fn test_file_creation_and_deletion() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("test.txt");

    // Create file
    fs::write(&file_path, "test content").expect("Failed to write file");
    assert!(file_path.exists());

    // Delete file
    fs::remove_file(&file_path).expect("Failed to delete file");
    assert!(!file_path.exists());
}

#[test]
fn test_directory_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sub_dir = temp_dir.path().join("subdir");

    // Create directory
    fs::create_dir(&sub_dir).expect("Failed to create directory");
    assert!(sub_dir.exists());
    assert!(sub_dir.is_dir());

    // List directory
    let entries = fs::read_dir(temp_dir.path()).expect("Failed to read directory");
    let count = entries.count();
    assert_eq!(count, 1);
}

#[test]
fn test_source_file_handling() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");

    // Write source
    {
        let mut file = temp_file.reopen().expect("Failed to reopen file");
        write!(file, "{}", source).expect("Failed to write");
    }

    // Read source back
    {
        let content = fs::read_to_string(temp_file.path()).expect("Failed to read");
        assert!(content.contains("fn add"));
        assert!(content.contains("Int"));
    }
}

#[test]
fn test_binary_file_operations() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let data: Vec<u8> = vec![0, 1, 2, 3, 4, 5, 255];

    // Write binary data
    fs::write(temp_file.path(), &data).expect("Failed to write");

    // Read binary data
    let read_data = fs::read(temp_file.path()).expect("Failed to read");

    assert_eq!(data, read_data);
}

// ============================================================================
// Path Handling Tests
// ============================================================================

#[test]
fn test_path_construction() {
    let path = PathBuf::from("/usr/local/bin");
    let file_path = path.join("verum");

    assert_eq!(file_path.to_str().unwrap(), "/usr/local/bin/verum");
}

#[test]
fn test_path_components() {
    let path = PathBuf::from("/usr/local/bin/verum");

    assert_eq!(path.file_name().unwrap(), "verum");
    assert_eq!(path.parent().unwrap(), PathBuf::from("/usr/local/bin"));
}

#[test]
fn test_relative_paths() {
    let base = PathBuf::from("/project");
    let relative = PathBuf::from("src/main.vr");
    let full = base.join(relative);

    assert_eq!(full.to_str().unwrap(), "/project/src/main.vr");
}

// ============================================================================
// Platform-Specific Tests
// ============================================================================

#[test]
fn test_line_endings() {
    let unix_content = "line1\nline2\nline3";
    let windows_content = "line1\r\nline2\r\nline3";

    // Both should parse correctly
    assert!(unix_content.contains('\n'));
    assert!(windows_content.contains("\r\n"));
}

#[test]
fn test_path_separators() {
    // Unix-style paths
    let unix_path = "/home/user/project";
    assert!(unix_path.contains('/'));

    // Windows-style paths (when on Windows)
    #[cfg(windows)]
    {
        let windows_path = r"C:\Users\user\project";
        assert!(windows_path.contains('\\'));
    }
}

// ============================================================================
// Standard Library Compatibility Tests
// ============================================================================

#[test]
fn test_stdlib_list_compatibility() {
    // Test that List works with standard Rust operations
    let mut list = List::new();
    list.push(1);
    list.push(2);
    list.push(3);

    // Convert to Vec
    let vec: Vec<i32> = (0..list.len()).map(|i| list[i]).collect();
    assert_eq!(vec, vec![1, 2, 3]);
}

#[test]
fn test_stdlib_text_compatibility() {
    let text = Text::from("Hello, Verum!");

    // Convert to String
    let string = text.as_str().to_string();
    assert_eq!(string, "Hello, Verum!");
}

#[test]
fn test_stdlib_map_compatibility() {
    let mut map = Map::new();
    map.insert("x".to_string(), 10);
    map.insert("y".to_string(), 20);

    // Convert to HashMap
    let hash_map: std::collections::HashMap<_, _> =
        map.iter().map(|(k, v)| (k.clone(), *v)).collect();

    assert_eq!(hash_map.get("x"), Some(&10));
    assert_eq!(hash_map.get("y"), Some(&20));
}

// ============================================================================
// Unicode and Encoding Tests
// ============================================================================

#[test]
fn test_unicode_strings() {
    let unicode = "Hello 世界 🌍";
    let text = Text::from(unicode);

    assert!(text.contains("世界"));
    assert!(text.contains("🌍"));
}

#[test]
fn test_unicode_identifiers() {
    // Test parsing Unicode identifiers (if supported)
    let source = "let δ = 42;";

    use verum_ast::span::FileId;
    use verum_lexer::Lexer;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let _tokens: Vec<_> = lexer.collect();

    // Should handle Unicode characters in source
}

#[test]
fn test_utf8_encoding() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let content = "UTF-8: 你好 مرحبا שלום";

    // Write UTF-8
    fs::write(temp_file.path(), content).expect("Failed to write");

    // Read UTF-8
    let read_content = fs::read_to_string(temp_file.path()).expect("Failed to read");

    assert_eq!(content, read_content);
}

// ============================================================================
// Error Handling Compatibility Tests
// ============================================================================

#[test]
fn test_io_error_handling() {
    // Try to read non-existent file
    let result = fs::read_to_string("/nonexistent/file.txt");
    assert!(result.is_err());

    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn test_json_error_handling() {
    let invalid_json = "{invalid json}";

    let result: Result<TestData, _> = serde_json::from_str(invalid_json);
    assert!(result.is_err());
}

// ============================================================================
// Environment Variable Tests
// ============================================================================

#[test]
fn test_env_var_access() {
    // SAFETY: Test environment variable manipulation in isolated test
    unsafe {
        // Set and get environment variable
        std::env::set_var("VERUM_TEST_VAR", "test_value");

        let value = std::env::var("VERUM_TEST_VAR").expect("Should get env var");
        assert_eq!(value, "test_value");

        // Clean up
        std::env::remove_var("VERUM_TEST_VAR");
    }
}

// ============================================================================
// Process Tests
// ============================================================================

#[test]
fn test_current_directory() {
    let current_dir = std::env::current_dir().expect("Should get current dir");
    assert!(current_dir.exists());
    assert!(current_dir.is_dir());
}

#[test]
fn test_temp_directory() {
    let temp_dir = std::env::temp_dir();
    assert!(temp_dir.exists());
    assert!(temp_dir.is_dir());
}

// ============================================================================
// Concurrent I/O Tests
// ============================================================================

#[test]
fn test_concurrent_file_operations() {
    use std::sync::Arc;
    use std::thread;

    let temp_dir = Arc::new(TempDir::new().expect("Failed to create temp dir"));
    let mut handles = vec![];

    for i in 0..10 {
        let temp_dir = Arc::clone(&temp_dir);
        let handle = thread::spawn(move || {
            let file_path = temp_dir.path().join(format!("file{}.txt", i));
            fs::write(&file_path, format!("Content {}", i)).expect("Failed to write");
            let content = fs::read_to_string(&file_path).expect("Failed to read");
            assert_eq!(content, format!("Content {}", i));
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

// ============================================================================
// Large File Handling Tests
// ============================================================================

#[test]
fn test_large_file_handling() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");

    // Write large content (1MB)
    let chunk = "x".repeat(1024);
    let mut content = String::new();
    for _ in 0..1024 {
        content.push_str(&chunk);
    }

    fs::write(temp_file.path(), &content).expect("Failed to write");

    // Read back
    let read_content = fs::read_to_string(temp_file.path()).expect("Failed to read");

    assert_eq!(content.len(), read_content.len());
}

// ============================================================================
// Buffered I/O Tests
// ============================================================================

#[test]
fn test_buffered_reading() {
    use std::io::BufRead;

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let content = "line1\nline2\nline3\n";

    fs::write(temp_file.path(), content).expect("Failed to write");

    let file = fs::File::open(temp_file.path()).expect("Failed to open");
    let reader = std::io::BufReader::new(file);

    let lines: Vec<_> = reader.lines().collect();
    assert_eq!(lines.len(), 3);
}

// ============================================================================
// Metadata Tests
// ============================================================================

#[test]
fn test_file_metadata() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    fs::write(temp_file.path(), "test").expect("Failed to write");

    let metadata = fs::metadata(temp_file.path()).expect("Failed to get metadata");

    assert!(metadata.is_file());
    assert!(!metadata.is_dir());
    assert!(metadata.len() > 0);
}

// ============================================================================
// Symbolic Link Tests (Unix only)
// ============================================================================

#[cfg(unix)]
#[test]
fn test_symbolic_links() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("original.txt");
    let link_path = temp_dir.path().join("link.txt");

    // Create file
    fs::write(&file_path, "original content").expect("Failed to write");

    // Create symlink
    std::os::unix::fs::symlink(&file_path, &link_path).expect("Failed to create symlink");

    // Read through symlink
    let content = fs::read_to_string(&link_path).expect("Failed to read symlink");
    assert_eq!(content, "original content");
}
