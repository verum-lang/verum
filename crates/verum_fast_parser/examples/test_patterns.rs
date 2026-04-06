//! Simple script to test parser against VCS pattern tests
use verum_fast_parser::FastParser;
use verum_ast::span::FileId;
use std::fs;

fn extract_expected_error(content: &str) -> Option<String> {
    for line in content.lines() {
        if line.contains("@expected-error:") {
            return Some(line.split("@expected-error:").nth(1)?.trim().to_string());
        }
    }
    None
}

fn test_dir(dir: &str, test_type: &str) {
    let mut passed = 0;
    let mut failed = 0;
    let mut failed_tests = Vec::new();

    let mut paths: Vec<_> = match fs::read_dir(dir) {
        Ok(d) => d
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "vr").unwrap_or(false))
            .collect(),
        Err(_) => {
            println!("{} Tests: directory not found", test_type);
            return;
        }
    };
    paths.sort();

    for path in &paths {
        let content = fs::read_to_string(&path).expect("Failed to read file");
        let expected_error = extract_expected_error(&content);

        let parser = FastParser::new();
        let file_id = FileId::new(0);
        let result = parser.parse_module_str(&content, file_id);

        let test_name = path.file_name().unwrap().to_str().unwrap();

        if test_type.contains("fail") {
            match result {
                Ok(_) => {
                    failed += 1;
                    failed_tests.push(format!("  {}: expected error {:?}, got success", test_name, expected_error));
                }
                Err(errors) => {
                    let error_codes: Vec<String> = errors.iter().map(|e| {
                        e.code.as_ref().map(|c| c.as_str().to_string()).unwrap_or_else(|| "E000".to_string())
                    }).collect();
                    if let Some(expected) = &expected_error {
                        if error_codes.iter().any(|c| c == expected) {
                            passed += 1;
                        } else {
                            failed += 1;
                            failed_tests.push(format!("  {}: expected {}, got {:?}", test_name, expected, error_codes));
                        }
                    } else {
                        passed += 1;
                    }
                }
            }
        } else {
            // Success tests
            match result {
                Ok(_) => passed += 1,
                Err(errors) => {
                    failed += 1;
                    let error_codes: Vec<String> = errors.iter().map(|e| {
                        let code = e.code.as_ref().map(|c| c.as_str()).unwrap_or("E000");
                        format!("{}: {:?}", code, e.kind)
                    }).collect();
                    failed_tests.push(format!("  {}: expected success, got {:?}", test_name, error_codes));
                }
            }
        }
    }

    let total = passed + failed;
    if total > 0 {
        println!("{} Tests: {}/{} passed ({:.1}%)", test_type, passed, total, (passed as f64 / total as f64) * 100.0);
        if !failed_tests.is_empty() {
            for t in &failed_tests {
                println!("{}", t);
            }
        }
    }
}

fn main() {
    println!("=== Pattern Fail Tests ===");
    test_dir("vcs/specs/parser/fail/patterns", "Pattern fail");

    println!("\n=== Pattern Success Tests ===");
    test_dir("vcs/specs/parser/success/patterns", "Pattern success");

    println!("\n=== Proof Fail Tests ===");
    test_dir("vcs/specs/parser/fail/proofs", "Proof fail");

    println!("\n=== Statement Fail Tests ===");
    test_dir("vcs/specs/parser/fail/statements", "Statement fail");
}
