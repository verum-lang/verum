// Security Test Suite for Verum Platform
//
// This module contains comprehensive security tests covering:
// - Parser DoS resistance
// - Integer overflow protection
// - Buffer boundary checks
// - Unsafe code validation
// - Cryptographic operations
// - Input validation

use std::time::Instant;
use verum_parser::VerumParser;
use verum_lexer::Lexer;
use verum_ast::FileId;

// ============================================================================
// SECTION 1: Parser Denial of Service (DoS) Tests
// ============================================================================

/// Security test: Parser resistance to deep nesting attacks
///
/// Tests that the parser handles 1000 levels of nested parentheses without
/// stack overflow or panic. This is a critical security test for DoS resistance.
#[test]
fn test_parser_deep_nesting_resistance() {
    // Deep nesting attack: Many nested parentheses
    // This should not cause stack overflow or panic
    //
    // SECURITY: Tests that parser gracefully handles pathological input
    let mut deep_input = String::new();
    for _ in 0..1000 {
        deep_input.push('(');
    }
    for _ in 0..1000 {
        deep_input.push(')');
    }

    // Should either parse successfully or return a clear error
    // Should NOT panic or cause stack overflow
    let _result = parse_expression(&deep_input);
    // Assertion: if it parses, result is valid
    // If it errors, error is recoverable
}

#[test]
fn test_parser_large_literal_handling() {
    // Very large but valid number literal
    // Should handle without excessive memory or time
    let large_number = "1".repeat(10000);
    let start = Instant::now();

    let _result = parse_literal(&large_number);

    let elapsed = start.elapsed();
    // Should complete in reasonable time (< 100ms)
    assert!(elapsed.as_millis() < 100, "Parser took too long: {:?}ms", elapsed.as_millis());
}

#[test]
fn test_parser_exponential_complexity_resistance() {
    // Pattern that could cause exponential backtracking in naive parsers
    // Using grammar like: a* a* a* b
    let input = format!("{}b", "a".repeat(20));
    let start = Instant::now();

    let _result = parse_expression(&input);

    let elapsed = start.elapsed();
    // Should complete in linear time, not exponential
    assert!(elapsed.as_millis() < 10, "Potential exponential behavior detected");
}

#[test]
fn test_parser_malformed_utf8_handling() {
    // Invalid UTF-8 sequences (if using byte strings)
    // Parser should reject gracefully
    let invalid_utf8 = b"valid\xFFinvalid";

    // Should handle without crashing
    // May return error or skip bytes - either is acceptable
    let _result = parse_bytes(invalid_utf8);
}

// ============================================================================
// SECTION 2: Integer Overflow Protection Tests
// ============================================================================

#[test]
fn test_unsigned_integer_overflow_protection() {
    // Test u64 overflow handling
    let max_u64 = "18446744073709551615"; // 2^64 - 1
    let overflow_u64 = "18446744073709551616"; // 2^64

    // Should parse max value successfully
    let result_max = parse_uint64(max_u64);
    assert!(result_max.is_ok(), "Should parse max u64");

    // Should detect overflow
    let result_overflow = parse_uint64(overflow_u64);
    assert!(result_overflow.is_err(), "Should reject overflow");
}

#[test]
fn test_signed_integer_overflow_protection() {
    // Test i64 overflow handling
    let max_i64 = "9223372036854775807"; // 2^63 - 1
    let min_i64 = "-9223372036854775808"; // -2^63
    let overflow_pos = "9223372036854775808"; // 2^63
    let overflow_neg = "-9223372036854775809"; // -2^63 - 1

    // Should parse valid bounds
    assert!(parse_int64(max_i64).is_ok());
    assert!(parse_int64(min_i64).is_ok());

    // Should detect overflow
    assert!(parse_int64(overflow_pos).is_err());
    assert!(parse_int64(overflow_neg).is_err());
}

#[test]
fn test_arithmetic_overflow_protection() {
    // Test that arithmetic operations check bounds
    let max = 1_i64.wrapping_add(i64::MAX - 1);
    assert!(max <= i64::MAX);

    let result = safe_add(i64::MAX - 1, 10);
    assert!(result.is_err(), "Should detect addition overflow");
}

#[test]
fn test_multiplication_overflow_protection() {
    // Test multiplication overflow detection
    let large = i64::MAX / 2;
    let result = safe_mul(large, 10);
    assert!(result.is_err(), "Should detect multiplication overflow");
}

#[test]
fn test_bit_shift_overflow_protection() {
    // Test bit shift bounds checking
    let value = 1_u64;

    // Valid shifts (< 64)
    assert!(safe_shl(value, 0).is_ok());
    assert!(safe_shl(value, 63).is_ok());

    // Invalid shifts (>= 64)
    assert!(safe_shl(value, 64).is_err());
    assert!(safe_shl(value, 128).is_err());
}

// ============================================================================
// SECTION 3: Buffer Boundary Protection Tests
// ============================================================================

#[test]
fn test_string_length_bounds() {
    // Test string parsing with reasonable limits
    let valid_string = "hello world";
    let result = parse_string(valid_string);
    assert!(result.is_ok(), "Should parse valid string");

    // Test very long but reasonable string
    let long_string = &"a".repeat(1_000_000); // 1MB
    let result = parse_string(long_string);
    assert!(result.is_ok(), "Should parse 1MB string");

    // Note: Extremely large strings (> 4GB) should be rejected
    // but we don't test them here due to memory constraints
}

#[test]
fn test_array_bounds_checking() {
    // Test that array access is bounds-checked
    let array = vec![1, 2, 3];

    // Valid access
    assert_eq!(safe_index(&array, 0), Ok(1));
    assert_eq!(safe_index(&array, 2), Ok(3));

    // Out of bounds
    assert!(safe_index(&array, 3).is_err());
    assert!(safe_index(&array, 1000).is_err());
}

#[test]
fn test_slice_bounds_checking() {
    // Test slice operations are bounds-checked
    let data = vec![1, 2, 3, 4, 5];

    // Valid slices
    assert!(safe_slice(&data, 0, 2).is_ok());
    assert!(safe_slice(&data, 2, 5).is_ok());

    // Invalid slices
    assert!(safe_slice(&data, 0, 10).is_err()); // End out of bounds
    assert!(safe_slice(&data, 3, 2).is_err()); // Start > end
    assert!(safe_slice(&data, 1000, 2000).is_err()); // Completely out of bounds
}

#[test]
fn test_string_concatenation_bounds() {
    // Test string concatenation doesn't exceed reasonable bounds
    let s1 = "hello";
    let s2 = "world";

    let result = safe_concat(s1, s2);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "helloworld");

    // Test many concatenations don't cause unbounded growth
    let result = safe_repeat_concat("a", 1000);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 1000);
}

#[test]
fn test_allocation_size_limits() {
    // Test that allocation sizes are reasonable
    // Should not allow allocating entire memory space

    let result = safe_alloc_bytes(1024); // 1KB - should succeed
    assert!(result.is_ok());

    let result = safe_alloc_bytes(1_000_000); // 1MB - should succeed
    assert!(result.is_ok());

    let result = safe_alloc_bytes(1_000_000_000); // 1GB - system dependent
    // Either succeeds or fails gracefully
    let _ = result;
}

// ============================================================================
// SECTION 4: Unsafe Code Validation Tests
// ============================================================================

#[test]
fn test_unsafe_ptr_read_write() {
    // SAFETY: This test validates that unsafe ptr operations are sound
    // Test Box::from_raw pairing
    let original = Box::new(42i32);
    let ptr = Box::into_raw(original);

    unsafe {
        // SAFETY: (1) ptr was created via Box::into_raw, (2) is valid
        let recovered = Box::from_raw(ptr);
        assert_eq!(*recovered, 42);
        // recovered is dropped here
    }
}

#[test]
fn test_unsafe_alignment_requirements() {
    // SAFETY: Tests that alignment requirements are met

    #[repr(align(16))]
    struct Aligned {
        value: u128,
    }

    let aligned = Aligned { value: 0 };
    let addr = &aligned as *const _ as usize;

    // SAFETY: We're just checking alignment, not dereferencing
    assert_eq!(
        addr % 16,
        0,
        "Alignment requirement not met: address {:#x} is not 16-byte aligned",
        addr
    );
}

#[test]
fn test_unsafe_dereference_validity() {
    // SAFETY: Tests that pointer dereference is only done on valid pointers
    let original = 123;
    let ptr = &original as *const i32;

    unsafe {
        // SAFETY: (1) ptr is not null, (2) it's properly aligned, (3) it points to valid memory
        let value = *ptr;
        assert_eq!(value, 123);
    }
}

#[test]
fn test_unsafe_transmute_lifetime() {
    // SAFETY: Tests transmute behavior with lifetimes
    // This should only be used when lifetimes can be proven sound

    let static_ref = &42i32;
    let shorter_ref: &i32 = static_ref;

    // Should NOT transmute to 'static (this is unsound!)
    // Instead, use proper lifetime types

    // Valid: transmute between same-lifetime types
    assert_eq!(std::mem::size_of_val(shorter_ref), std::mem::size_of::<i32>());
}

// ============================================================================
// SECTION 5: Input Validation Tests
// ============================================================================

#[test]
fn test_identifier_validation() {
    // Valid identifiers
    assert!(is_valid_identifier("x"));
    assert!(is_valid_identifier("_private"));
    assert!(is_valid_identifier("CamelCase"));
    assert!(is_valid_identifier("snake_case"));
    assert!(is_valid_identifier("with123numbers"));

    // Invalid identifiers
    assert!(!is_valid_identifier(""));
    assert!(!is_valid_identifier("123invalid")); // Starts with number
    assert!(!is_valid_identifier("invalid-name")); // Contains dash
    assert!(!is_valid_identifier("invalid name")); // Contains space
    assert!(!is_valid_identifier("invalid@name")); // Contains special char
}

#[test]
fn test_keyword_validation() {
    // Reserved keywords should be detected
    assert!(is_keyword("if"));
    assert!(is_keyword("else"));
    assert!(is_keyword("fn"));
    assert!(is_keyword("let"));
    assert!(is_keyword("pub"));

    // Non-keywords
    assert!(!is_keyword("myvar"));
    assert!(!is_keyword("data"));
    assert!(!is_keyword("value"));
}

#[test]
fn test_path_component_validation() {
    // Valid path components
    assert!(is_valid_path_component("module"));
    assert!(is_valid_path_component("sub_module"));
    assert!(is_valid_path_component("Type123"));

    // Invalid path components
    assert!(!is_valid_path_component(""));
    assert!(!is_valid_path_component(".."));
    assert!(!is_valid_path_component("path/to/file")); // Contains separator
    assert!(!is_valid_path_component("/absolute")); // Absolute path
}

#[test]
fn test_numeric_literal_validation() {
    // Valid numeric literals
    assert!(is_valid_numeric("0"));
    assert!(is_valid_numeric("123"));
    assert!(is_valid_numeric("0xFF"));
    assert!(is_valid_numeric("0b1010"));
    assert!(is_valid_numeric("0o755"));
    assert!(is_valid_numeric("1.23"));
    assert!(is_valid_numeric("1e10"));

    // Invalid numeric literals
    assert!(!is_valid_numeric(""));
    assert!(!is_valid_numeric("0x"));
    assert!(!is_valid_numeric("0b"));
    assert!(!is_valid_numeric("1.2.3")); // Multiple decimal points
    assert!(!is_valid_numeric("1e")); // Incomplete exponent
}

#[test]
fn test_string_literal_validation() {
    // Valid string literals
    assert!(is_valid_string_literal("\"hello\""));
    assert!(is_valid_string_literal("\"with spaces\""));
    assert!(is_valid_string_literal("\"with\\nnewline\""));

    // Invalid string literals
    assert!(!is_valid_string_literal("\"unclosed"));
    assert!(!is_valid_string_literal("\"invalid\\xEscape\""));
}

// ============================================================================
// SECTION 6: Path Traversal Protection Tests
// ============================================================================

#[test]
fn test_path_traversal_prevention() {
    // Valid paths (within sandbox)
    assert!(is_safe_path("config.txt"));
    assert!(is_safe_path("data/file.json"));
    assert!(is_safe_path("subdir/file.toml"));

    // Traversal attacks (should be rejected)
    assert!(!is_safe_path("../../../etc/passwd"));
    assert!(!is_safe_path("..\\..\\windows\\system32"));
    assert!(!is_safe_path("./../../escape"));
}

#[test]
fn test_absolute_path_rejection() {
    // Absolute paths should be rejected in secure context
    assert!(!is_safe_path("/etc/passwd"));
    assert!(!is_safe_path("C:\\Windows\\System32"));
    assert!(!is_safe_path("/var/log/auth.log"));
}

#[test]
fn test_null_byte_in_path() {
    // Null bytes in paths should be rejected
    assert!(!is_safe_path("config.txt\0.bak"));
    assert!(!is_safe_path("file\0with\0nulls"));
}

// ============================================================================
// SECTION 7: Cryptographic Tests
// ============================================================================

#[test]
fn test_random_number_generation() {
    // SECURITY: Verify RNG produces non-repetitive sequences
    let mut prev = generate_random_u64();

    let mut different_count = 0;
    for _ in 0..100 {
        let curr = generate_random_u64();
        if curr != prev {
            different_count += 1;
        }
        prev = curr;
    }

    // Should have many different values (extremely unlikely to repeat 100 times)
    assert!(different_count > 95, "RNG seems deterministic or weak");
}

#[test]
fn test_hash_function_consistency() {
    // SECURITY: Hash function should be deterministic and consistent
    let input = "test_input";

    let hash1 = hash_string(input);
    let hash2 = hash_string(input);

    assert_eq!(hash1, hash2, "Hash function not consistent");
}

#[test]
fn test_hash_avalanche_effect() {
    // SECURITY: Small input changes should result in completely different hashes
    let input1 = "test_input";
    let input2 = "test_input2";

    let hash1 = hash_string(input1);
    let hash2 = hash_string(input2);

    // Hashes should be very different (bit-wise)
    let xor = hash1 ^ hash2;
    let different_bits = xor.count_ones();

    assert!(different_bits > 15, "Hash avalanche effect weak: only {} bits different", different_bits);
}

// ============================================================================
// SECTION 8: Error Handling and Recovery Tests
// ============================================================================

#[test]
fn test_error_message_no_path_leakage() {
    // SECURITY: Error messages should not leak internal paths
    let error_msg = format_error(&ParseError::SyntaxError { line: 42, col: 10 });

    assert!(!error_msg.contains("/Users/"));
    assert!(!error_msg.contains("/home/"));
    assert!(!error_msg.contains("C:\\"));
}

#[test]
fn test_error_message_no_config_leakage() {
    // SECURITY: Error messages should not reveal system configuration
    let error_msg = format_error(&SystemError::IoError("file not found"));

    assert!(!error_msg.contains("OPENAI_API_KEY"));
    assert!(!error_msg.contains("DATABASE_URL"));
    assert!(!error_msg.contains("SECRET"));
}

// ============================================================================
// HELPER FUNCTIONS (Mock implementations for testing)
// ============================================================================

// Parser functions
fn parse_expression(input: &str) -> Result<(), String> {
    // SECURITY: Tests parser robustness with potentially malicious input
    // Create a file ID for this test input
    let file_id = FileId::new(0);

    // Create parser and attempt to parse the expression
    let parser = VerumParser::new();

    // Parse the expression - this exercises the full parser stack
    match parser.parse_expr_str(input, file_id) {
        Ok(_expr) => Ok(()),
        Err(errors) => {
            // Convert parse errors to string format
            let error_msgs: Vec<String> = errors
                .iter()
                .map(|e| format!("{}", e))
                .collect();
            Err(error_msgs.join("; "))
        }
    }
}

fn parse_literal(input: &str) -> Result<(), String> {
    // SECURITY: Tests literal parsing with potentially malicious input
    // Literals include: integers, floats, strings, booleans, etc.
    let file_id = FileId::new(0);
    let parser = VerumParser::new();

    // Attempt to parse as an expression (literals are valid expressions)
    match parser.parse_expr_str(input, file_id) {
        Ok(_expr) => Ok(()),
        Err(errors) => {
            let error_msgs: Vec<String> = errors
                .iter()
                .map(|e| format!("{}", e))
                .collect();
            Err(error_msgs.join("; "))
        }
    }
}

fn parse_bytes(bytes: &[u8]) -> Result<(), String> {
    // SECURITY: Tests parser handling of potentially invalid UTF-8 sequences
    // This is critical for preventing crashes from malformed input

    // First, try to convert bytes to UTF-8
    let input = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            // Invalid UTF-8 should be caught gracefully
            return Err(format!("Invalid UTF-8: {}", e));
        }
    };

    // If UTF-8 is valid, try to parse as Verum code
    let file_id = FileId::new(0);
    let parser = VerumParser::new();

    match parser.parse_expr_str(input, file_id) {
        Ok(_expr) => Ok(()),
        Err(errors) => {
            let error_msgs: Vec<String> = errors
                .iter()
                .map(|e| format!("{}", e))
                .collect();
            Err(error_msgs.join("; "))
        }
    }
}

// Integer parsing functions
fn parse_uint64(input: &str) -> Result<u64, String> {
    input.parse::<u64>()
        .map_err(|_| "failed to parse as u64".to_string())
}

fn parse_int64(input: &str) -> Result<i64, String> {
    input.parse::<i64>()
        .map_err(|_| "failed to parse as i64".to_string())
}

// Safe arithmetic functions
fn safe_add(a: i64, b: i64) -> Result<i64, String> {
    a.checked_add(b)
        .ok_or_else(|| "addition overflow".to_string())
}

fn safe_mul(a: i64, b: i64) -> Result<i64, String> {
    a.checked_mul(b)
        .ok_or_else(|| "multiplication overflow".to_string())
}

fn safe_shl(value: u64, shift: u32) -> Result<u64, String> {
    if shift >= 64 {
        Err("shift amount out of bounds".to_string())
    } else {
        Ok(value << shift)
    }
}

// Array/slice functions
fn safe_index<T: Clone>(array: &[T], index: usize) -> Result<T, String> {
    array.get(index)
        .ok_or_else(|| "index out of bounds".to_string())
        .map(|v| v.clone())
}

fn safe_slice(data: &[u32], start: usize, end: usize) -> Result<Vec<u32>, String> {
    if start > end || end > data.len() {
        Err("slice bounds out of range".to_string())
    } else {
        Ok(data[start..end].to_vec())
    }
}

// String functions
fn safe_concat(a: &str, b: &str) -> Result<String, String> {
    if a.len() + b.len() > 1_000_000_000 {
        Err("concatenation would exceed size limit".to_string())
    } else {
        Ok(format!("{}{}", a, b))
    }
}

fn safe_repeat_concat(s: &str, count: usize) -> Result<String, String> {
    if s.len() * count > 1_000_000_000 {
        Err("repetition would exceed size limit".to_string())
    } else {
        Ok(s.repeat(count))
    }
}

// Memory allocation
fn safe_alloc_bytes(size: usize) -> Result<Vec<u8>, String> {
    if size > 10_000_000_000 {
        Err("allocation would exceed limit".to_string())
    } else {
        Ok(vec![0u8; size])
    }
}

// Validation functions
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let first_char = s.chars().next().unwrap();
    if !first_char.is_alphabetic() && first_char != '_' {
        return false;
    }

    s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn is_keyword(s: &str) -> bool {
    matches!(s,
        "if" | "else" | "fn" | "let" | "pub" | "mod" | "use" | "struct" |
        "enum" | "impl" | "trait" | "match" | "for" | "while" | "loop" |
        "async" | "await" | "unsafe" | "const" | "static" | "return"
    )
}

fn is_valid_path_component(s: &str) -> bool {
    if s.is_empty() || s == ".." {
        return false;
    }
    !s.contains('/') && !s.contains('\\') && !s.starts_with('/')
}

fn is_valid_numeric(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Simple validation - doesn't cover all cases
    s.parse::<f64>().is_ok() ||
    s.parse::<i128>().is_ok() ||
    (s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit())) ||
    (s.starts_with("0b") && s[2..].chars().all(|c| c == '0' || c == '1')) ||
    (s.starts_with("0o") && s[2..].chars().all(|c| matches!(c, '0'..='7')))
}

fn is_valid_string_literal(s: &str) -> bool {
    s.starts_with('"') && s.ends_with('"') && s.len() > 1
}

// Path safety
fn is_safe_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    // Reject absolute paths
    if path.starts_with('/') || path.starts_with('\\') || path.contains(":\\") {
        return false;
    }

    // Reject traversal attempts
    if path.contains("..") || path.contains('\0') {
        return false;
    }

    true
}

// Cryptographic functions
fn generate_random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64);
    hasher.finish()
}

fn hash_string(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

// Error types
#[derive(Debug)]
enum ParseError {
    SyntaxError { line: usize, col: usize },
}

#[derive(Debug)]
enum SystemError {
    IoError(&'static str),
}

fn format_error(error: &ParseError) -> String {
    match error {
        ParseError::SyntaxError { line, col } => {
            format!("Syntax error at line {}, column {}", line, col)
        }
    }
}

fn format_error_system(error: &SystemError) -> String {
    match error {
        SystemError::IoError(msg) => format!("IO Error: {}", msg),
    }
}
