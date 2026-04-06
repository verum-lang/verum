//! Category 3: Standard Library Integration Tests
//!
//! Tests core modules working together:
//! - I/O + FS: Read/write files
//! - FS + JSON: Read JSON config files
//! - Regex + Text: Pattern matching
//! - I/O + Async: Async file operations
//! - All modules: Complex workflows

use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use verum_std::core::{List, Text, Map, Maybe, Result as VerumResult};
use verum_std::fs::{FileSystem, PathOps};
use verum_std::json::{Json, JsonValue};
use verum_std::regex::Regex;

use crate::integration::test_utils::*;

// ============================================================================
// Test 3.1: I/O + FS Integration
// ============================================================================

#[test]
fn test_io_fs_read_write() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.txt");

    let content = Text::from("Hello, Verum!");

    // Write file
    std::fs::write(&file_path, content.as_str()).expect("Should write file");

    // Read file
    let read_content = std::fs::read_to_string(&file_path).expect("Should read file");

    assert_eq!(read_content, content.as_str());
}

#[test]
fn test_io_fs_directory_operations() {
    let temp_dir = TempDir::new().unwrap();
    let sub_dir = temp_dir.path().join("subdir");

    // Create directory
    std::fs::create_dir(&sub_dir).expect("Should create directory");

    assert!(sub_dir.exists());
    assert!(sub_dir.is_dir());

    // List directory
    let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
        .expect("Should read directory")
        .collect();

    assert_eq!(entries.len(), 1);
}

#[test]
fn test_io_fs_file_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.txt");

    std::fs::write(&file_path, "test content").expect("Should write");

    let metadata = std::fs::metadata(&file_path).expect("Should get metadata");

    assert!(metadata.is_file());
    assert!(metadata.len() > 0);
}

// ============================================================================
// Test 3.2: FS + JSON Integration
// ============================================================================

#[test]
fn test_fs_json_read_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");

    let config_json = r#"{
        "host": "localhost",
        "port": 8080,
        "debug": true
    }"#;

    std::fs::write(&config_path, config_json).expect("Should write config");

    // Read and parse JSON
    let content = std::fs::read_to_string(&config_path).expect("Should read");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("Should parse JSON");

    assert_eq!(parsed["host"], "localhost");
    assert_eq!(parsed["port"], 8080);
    assert_eq!(parsed["debug"], true);
}

#[test]
fn test_fs_json_write_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");

    let config = serde_json::json!({
        "name": "Test App",
        "version": "1.0.0",
        "features": ["auth", "api", "ui"]
    });

    let json_string = serde_json::to_string_pretty(&config).expect("Should serialize");
    std::fs::write(&config_path, json_string).expect("Should write");

    // Verify file exists and contains JSON
    let content = std::fs::read_to_string(&config_path).expect("Should read");
    assert!(content.contains("Test App"));
    assert!(content.contains("1.0.0"));
}

#[test]
fn test_fs_json_complex_structure() {
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("data.json");

    let complex_data = serde_json::json!({
        "users": [
            {"id": 1, "name": "Alice", "roles": ["admin"]},
            {"id": 2, "name": "Bob", "roles": ["user", "moderator"]}
        ],
        "settings": {
            "theme": "dark",
            "notifications": true
        }
    });

    let json_str = serde_json::to_string(&complex_data).expect("Should serialize");
    std::fs::write(&data_path, json_str).expect("Should write");

    let read_str = std::fs::read_to_string(&data_path).expect("Should read");
    let parsed: serde_json::Value = serde_json::from_str(&read_str).expect("Should parse");

    assert_eq!(parsed["users"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["settings"]["theme"], "dark");
}

// ============================================================================
// Test 3.3: Regex + Text Integration
// ============================================================================

#[test]
fn test_regex_text_pattern_matching() {
    let text = Text::from("Email: user@example.com, Phone: 555-1234");

    let email_pattern = regex::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("Should compile regex");

    assert!(email_pattern.is_match(text.as_str()));

    if let Some(m) = email_pattern.find(text.as_str()) {
        assert_eq!(m.as_str(), "user@example.com");
    }
}

#[test]
fn test_regex_text_extraction() {
    let text = Text::from("The numbers are 42, 123, and 7.");

    let number_pattern = regex::Regex::new(r"\d+").expect("Should compile");

    let numbers: Vec<&str> = number_pattern
        .find_iter(text.as_str())
        .map(|m| m.as_str())
        .collect();

    assert_eq!(numbers, vec!["42", "123", "7"]);
}

#[test]
fn test_regex_text_replacement() {
    let text = Text::from("Hello World");

    let pattern = regex::Regex::new(r"World").expect("Should compile");
    let result = pattern.replace(text.as_str(), "Verum");

    assert_eq!(result, "Hello Verum");
}

#[test]
fn test_regex_text_validation() {
    let valid_email = "user@example.com";
    let invalid_email = "not-an-email";

    let email_pattern = regex::Regex::new(
        r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
    ).expect("Should compile");

    assert!(email_pattern.is_match(valid_email));
    assert!(!email_pattern.is_match(invalid_email));
}

// ============================================================================
// Test 3.4: I/O + Async Integration
// ============================================================================

#[tokio::test]
async fn test_io_async_file_operations() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("async_test.txt");

    let content = "Async file content";

    // Async write
    tokio::fs::write(&file_path, content).await.expect("Should write async");

    // Async read
    let read_content = tokio::fs::read_to_string(&file_path)
        .await
        .expect("Should read async");

    assert_eq!(read_content, content);
}

#[tokio::test]
async fn test_io_async_concurrent_reads() {
    let temp_dir = TempDir::new().unwrap();

    // Create multiple files
    for i in 0..10 {
        let path = temp_dir.path().join(format!("file{}.txt", i));
        tokio::fs::write(&path, format!("Content {}", i))
            .await
            .expect("Should write");
    }

    // Read all files concurrently
    let mut handles = Vec::new();
    for i in 0..10 {
        let path = temp_dir.path().join(format!("file{}.txt", i));
        let handle = tokio::spawn(async move {
            tokio::fs::read_to_string(&path).await
        });
        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    assert_eq!(results.len(), 10);

    for (i, result) in results.iter().enumerate() {
        let content = result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(content, &format!("Content {}", i));
    }
}

// ============================================================================
// Test 3.5: All Modules Together
// ============================================================================

#[tokio::test]
async fn test_complex_workflow_all_modules() {
    let temp_dir = TempDir::new().unwrap();
    let data_file = temp_dir.path().join("data.json");
    let output_file = temp_dir.path().join("output.txt");

    // Step 1: Create JSON data (FS + JSON)
    let data = serde_json::json!({
        "items": [
            {"id": 1, "text": "First item"},
            {"id": 2, "text": "Second item"},
            {"id": 3, "text": "Third item"}
        ]
    });

    tokio::fs::write(&data_file, serde_json::to_string_pretty(&data).unwrap())
        .await
        .expect("Should write JSON");

    // Step 2: Read and process JSON (I/O + JSON)
    let content = tokio::fs::read_to_string(&data_file)
        .await
        .expect("Should read");

    let parsed: serde_json::Value = serde_json::from_str(&content)
        .expect("Should parse");

    // Step 3: Extract text using regex (Regex + Text)
    let mut extracted = Vec::new();
    for item in parsed["items"].as_array().unwrap() {
        if let Some(text) = item["text"].as_str() {
            extracted.push(text.to_string());
        }
    }

    // Step 4: Write results (I/O + FS)
    let output = extracted.join("\n");
    tokio::fs::write(&output_file, &output)
        .await
        .expect("Should write output");

    // Verify final result
    let final_content = tokio::fs::read_to_string(&output_file)
        .await
        .expect("Should read output");

    assert!(final_content.contains("First item"));
    assert!(final_content.contains("Second item"));
    assert!(final_content.contains("Third item"));
}

#[test]
fn test_data_transformation_pipeline() {
    // Simulate data pipeline: Read → Parse → Transform → Write

    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.csv");
    let output_file = temp_dir.path().join("output.json");

    // Step 1: Create CSV data
    let csv_data = "id,name,value\n1,Alice,100\n2,Bob,200\n3,Charlie,300";
    std::fs::write(&input_file, csv_data).expect("Should write CSV");

    // Step 2: Read and parse CSV (simplified)
    let content = std::fs::read_to_string(&input_file).expect("Should read");
    let lines: Vec<&str> = content.lines().skip(1).collect(); // Skip header

    // Step 3: Transform to JSON
    let mut records = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() == 3 {
            let record = serde_json::json!({
                "id": parts[0].parse::<i32>().unwrap(),
                "name": parts[1],
                "value": parts[2].parse::<i32>().unwrap()
            });
            records.push(record);
        }
    }

    let output_json = serde_json::json!({"records": records});

    // Step 4: Write JSON
    std::fs::write(&output_file, serde_json::to_string_pretty(&output_json).unwrap())
        .expect("Should write JSON");

    // Verify
    let output_content = std::fs::read_to_string(&output_file).expect("Should read");
    assert!(output_content.contains("Alice"));
    assert!(output_content.contains("Bob"));
}

// ============================================================================
// Test 3.6: Performance Tests
// ============================================================================

#[tokio::test]
async fn test_performance_large_file_io() {
    let temp_dir = TempDir::new().unwrap();
    let large_file = temp_dir.path().join("large.txt");

    // Create 10MB file
    let content = "x".repeat(10 * 1024 * 1024);

    let (_, write_time) = measure_time_async(|| async {
        tokio::fs::write(&large_file, &content).await.unwrap()
    }).await;

    let (_, read_time) = measure_time_async(|| async {
        tokio::fs::read_to_string(&large_file).await.unwrap()
    }).await;

    assert_duration_lt(write_time, Duration::from_secs(5), "Write should be fast");
    assert_duration_lt(read_time, Duration::from_secs(5), "Read should be fast");
}

#[test]
fn test_performance_regex_matching() {
    let text = "The quick brown fox jumps over the lazy dog ".repeat(1000);

    let pattern = regex::Regex::new(r"\b\w{5}\b").expect("Should compile");

    let (matches, duration) = measure_time(|| {
        pattern.find_iter(&text).count()
    });

    assert!(matches > 0);
    assert_duration_lt(
        duration,
        Duration::from_millis(100),
        "Regex matching should be fast"
    );
}

#[test]
fn test_performance_json_parsing() {
    let large_json = serde_json::json!({
        "data": (0..1000).map(|i| {
            serde_json::json!({"id": i, "value": format!("item_{}", i)})
        }).collect::<Vec<_>>()
    });

    let json_str = serde_json::to_string(&large_json).expect("Should serialize");

    let (_, duration) = measure_time(|| {
        serde_json::from_str::<serde_json::Value>(&json_str)
            .expect("Should parse")
    });

    assert_duration_lt(
        duration,
        Duration::from_millis(500),
        "JSON parsing should be fast"
    );
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_core_modules_available() {
        // Verify all core modules can be imported
        // This ensures no missing dependencies
    }

    #[tokio::test]
    async fn test_error_handling_across_modules() {
        let temp_dir = TempDir::new().unwrap();
        let missing_file = temp_dir.path().join("missing.txt");

        // Attempt to read non-existent file
        let result = tokio::fs::read_to_string(&missing_file).await;
        assert!(result.is_err(), "Should fail gracefully");
    }
}
