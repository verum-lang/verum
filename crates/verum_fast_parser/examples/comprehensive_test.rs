//! Comprehensive VCS parser test runner
//! Tests all files in vcs/specs/parser/{success,fail}

use verum_fast_parser::FastParser;
use verum_ast::span::FileId;
use std::fs;
use std::path::Path;

fn extract_expected_error(content: &str) -> Option<String> {
    for line in content.lines() {
        if line.contains("@expected-error:") {
            return Some(line.split("@expected-error:").nth(1)?.trim().to_string());
        }
    }
    None
}

fn test_file(path: &Path, expect_success: bool) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let expected_error = extract_expected_error(&content);
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    let result = parser.parse_module_str(&content, file_id);

    let file_name = path.file_name().unwrap().to_str().unwrap();

    if expect_success {
        // Success test: should parse without errors
        match result {
            Ok(_) => Ok(()),
            Err(errors) => {
                let error_info: Vec<String> = errors.iter().map(|e| {
                    let code = e.code.as_ref().map(|c| c.as_str()).unwrap_or("E000");
                    format!("{}: {:?}", code, e.kind)
                }).collect();
                Err(format!("{}: expected success, got {:?}", file_name, error_info))
            }
        }
    } else {
        // Fail test: should produce an error
        match result {
            Ok(_) => {
                Err(format!("{}: expected error {:?}, got success", file_name, expected_error))
            }
            Err(errors) => {
                let error_codes: Vec<String> = errors.iter().map(|e| {
                    e.code.as_ref().map(|c| c.as_str().to_string()).unwrap_or_else(|| "E000".to_string())
                }).collect();

                if let Some(expected) = &expected_error {
                    if error_codes.iter().any(|c| c == expected) {
                        Ok(())
                    } else {
                        Err(format!("{}: expected {}, got {:?}", file_name, expected, error_codes))
                    }
                } else {
                    // No specific error expected, just need some error
                    Ok(())
                }
            }
        }
    }
}

fn test_directory(dir: &str, expect_success: bool) -> (usize, usize, Vec<String>) {
    let mut passed = 0;
    let mut failed = 0;
    let mut failures = Vec::new();

    let path = Path::new(dir);
    if !path.exists() {
        return (0, 0, vec![format!("Directory not found: {}", dir)]);
    }

    fn collect_vr_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_vr_files(&path, files);
                } else if path.extension().map(|e| e == "vr").unwrap_or(false) {
                    files.push(path);
                }
            }
        }
    }

    let mut files = Vec::new();
    collect_vr_files(path, &mut files);
    files.sort();

    for file_path in &files {
        match test_file(file_path, expect_success) {
            Ok(_) => passed += 1,
            Err(e) => {
                failed += 1;
                // Include relative path from vcs/specs/parser; fall back to
                // the full path if the example is run from a different checkout.
                let rel_path = file_path
                    .strip_prefix(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vcs/specs/parser"))
                    .unwrap_or(file_path);
                failures.push(format!("{}: {}", rel_path.display(), e.split(": ").skip(1).collect::<Vec<_>>().join(": ")));
            }
        }
    }

    (passed, failed, failures)
}

fn main() {
    println!("=== VCS Parser Comprehensive Test Suite ===\n");

    // Test success directory
    println!("--- SUCCESS TESTS (should parse without errors) ---");
    let (s_passed, s_failed, s_failures) = test_directory(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../vcs/specs/parser/success"),
        true
    );
    let s_total = s_passed + s_failed;
    let s_pct = if s_total > 0 { (s_passed as f64 / s_total as f64) * 100.0 } else { 0.0 };
    println!("Success tests: {}/{} passed ({:.1}%)", s_passed, s_total, s_pct);

    if !s_failures.is_empty() {
        println!("\nFailed success tests:");
        for f in &s_failures {
            println!("  {}", f);
        }
    }

    // Test fail directory
    println!("\n--- FAIL TESTS (should produce specific errors) ---");
    let (f_passed, f_failed, f_failures) = test_directory(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../vcs/specs/parser/fail"),
        false
    );
    let f_total = f_passed + f_failed;
    let f_pct = if f_total > 0 { (f_passed as f64 / f_total as f64) * 100.0 } else { 0.0 };
    println!("Fail tests: {}/{} passed ({:.1}%)", f_passed, f_total, f_pct);

    if !f_failures.is_empty() {
        println!("\nFailed fail tests:");
        for f in &f_failures {
            println!("  {}", f);
        }
    }

    // Summary
    let total_passed = s_passed + f_passed;
    let total_failed = s_failed + f_failed;
    let total = total_passed + total_failed;
    let total_pct = if total > 0 { (total_passed as f64 / total as f64) * 100.0 } else { 0.0 };

    println!("\n=== SUMMARY ===");
    println!("Success tests: {}/{} ({:.1}%)", s_passed, s_total, s_pct);
    println!("Fail tests:    {}/{} ({:.1}%)", f_passed, f_total, f_pct);
    println!("TOTAL:         {}/{} ({:.1}%)", total_passed, total, total_pct);

    if total_failed > 0 {
        println!("\n{} tests need attention.", total_failed);
        std::process::exit(1);
    } else {
        println!("\nAll tests passed!");
    }
}
