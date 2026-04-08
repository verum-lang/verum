//! Output normalization for differential testing
//!
//! This module provides functions to normalize program output for reliable
//! comparison across different execution tiers. It handles:
//!
//! - Memory address stripping (heap pointers, stack addresses)
//! - Timestamp normalization (ISO 8601, Unix timestamps, log formats)
//! - Thread ID normalization (platform-specific thread identifiers)
//! - File path normalization (cross-platform path separators)
//! - Float precision canonicalization (consistent decimal places)
//! - Whitespace normalization (line endings, trailing spaces)
//! - Platform-specific output differences
//! - Hash ordering canonicalization
//! - Canonical JSON output (sorted keys, consistent formatting)
//! - ANSI escape code stripping
//! - Unicode normalization (NFC/NFD forms)

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Normalization configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationConfig {
    /// Strip memory addresses (e.g., 0x7fff1234abcd)
    pub strip_addresses: bool,
    /// Normalize floating-point output precision
    pub normalize_floats: bool,
    /// Number of decimal places for float normalization
    pub float_precision: usize,
    /// Strip timestamps in various formats
    pub strip_timestamps: bool,
    /// Normalize line endings (CRLF -> LF)
    pub normalize_line_endings: bool,
    /// Trim trailing whitespace
    pub trim_trailing_whitespace: bool,
    /// Normalize Unicode to NFC
    pub normalize_unicode: bool,
    /// Strip ANSI color codes
    pub strip_ansi_codes: bool,
    /// Sort lines in unordered collections
    pub sort_unordered_output: bool,
    /// Strip process/thread IDs
    pub strip_process_ids: bool,
    /// Strip file paths with platform-specific separators
    pub normalize_paths: bool,
    /// Canonicalize JSON output (sorted keys, consistent formatting)
    pub canonicalize_json: bool,
    /// Strip thread identifiers
    pub strip_thread_ids: bool,
    /// Normalize file descriptors (fd:N -> fd:X)
    pub normalize_file_descriptors: bool,
    /// Strip memory allocator info
    pub strip_allocator_info: bool,
    /// Replace UUID/GUID patterns
    pub strip_uuids: bool,
    /// Custom patterns to strip (regex list)
    pub custom_strip_patterns: Vec<String>,
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            strip_addresses: true,
            normalize_floats: true,
            float_precision: 10,
            strip_timestamps: true,
            normalize_line_endings: true,
            trim_trailing_whitespace: true,
            normalize_unicode: false,
            strip_ansi_codes: true,
            sort_unordered_output: false,
            strip_process_ids: true,
            normalize_paths: true,
            canonicalize_json: true,
            strip_thread_ids: true,
            normalize_file_descriptors: true,
            strip_allocator_info: true,
            strip_uuids: true,
            custom_strip_patterns: vec![],
        }
    }
}

impl NormalizationConfig {
    /// Create config with all normalization enabled
    pub fn aggressive() -> Self {
        Self {
            strip_addresses: true,
            normalize_floats: true,
            float_precision: 6,
            strip_timestamps: true,
            normalize_line_endings: true,
            trim_trailing_whitespace: true,
            normalize_unicode: true,
            strip_ansi_codes: true,
            sort_unordered_output: true,
            strip_process_ids: true,
            normalize_paths: true,
            canonicalize_json: true,
            strip_thread_ids: true,
            normalize_file_descriptors: true,
            strip_allocator_info: true,
            strip_uuids: true,
            custom_strip_patterns: vec![],
        }
    }

    /// Create config for exact matching (minimal normalization)
    pub fn exact() -> Self {
        Self {
            strip_addresses: false,
            normalize_floats: false,
            float_precision: 15,
            strip_timestamps: false,
            normalize_line_endings: true,
            trim_trailing_whitespace: false,
            normalize_unicode: false,
            strip_ansi_codes: false,
            sort_unordered_output: false,
            strip_process_ids: false,
            normalize_paths: false,
            canonicalize_json: false,
            strip_thread_ids: false,
            normalize_file_descriptors: false,
            strip_allocator_info: false,
            strip_uuids: false,
            custom_strip_patterns: vec![],
        }
    }

    /// Create config for semantic equivalence (reasonable normalization)
    pub fn semantic() -> Self {
        Self {
            strip_addresses: true,
            normalize_floats: true,
            float_precision: 10,
            strip_timestamps: true,
            normalize_line_endings: true,
            trim_trailing_whitespace: true,
            normalize_unicode: false,
            strip_ansi_codes: true,
            sort_unordered_output: false,
            strip_process_ids: true,
            normalize_paths: true,
            canonicalize_json: true,
            strip_thread_ids: true,
            normalize_file_descriptors: true,
            strip_allocator_info: true,
            strip_uuids: true,
            custom_strip_patterns: vec![],
        }
    }

    /// Create config for differential testing (standard settings)
    pub fn differential() -> Self {
        Self {
            strip_addresses: true,
            normalize_floats: true,
            float_precision: 10,
            strip_timestamps: true,
            normalize_line_endings: true,
            trim_trailing_whitespace: true,
            normalize_unicode: true,
            strip_ansi_codes: true,
            sort_unordered_output: true,
            strip_process_ids: true,
            normalize_paths: true,
            canonicalize_json: true,
            strip_thread_ids: true,
            normalize_file_descriptors: true,
            strip_allocator_info: true,
            strip_uuids: true,
            custom_strip_patterns: vec![],
        }
    }
}

/// Compiled regex patterns for normalization
static ADDRESS_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"0x[0-9a-fA-F]{6,16}").unwrap());

static TIMESTAMP_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // ISO 8601: 2024-01-15T10:30:45.123Z
        Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?").unwrap(),
        // Unix timestamp: 1705312245 (careful to not match other 10+ digit numbers)
        Regex::new(r"\b(timestamp|time|ts)[=: ]+\d{10,13}\b").unwrap(),
        // Common log format: [15/Jan/2024:10:30:45 +0000]
        Regex::new(r"\[\d{2}/\w{3}/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4}\]").unwrap(),
        // HH:MM:SS.mmm with context
        Regex::new(r"\b\d{2}:\d{2}:\d{2}(\.\d{1,6})?\b").unwrap(),
        // Date formats: YYYY/MM/DD, MM/DD/YYYY, DD/MM/YYYY
        Regex::new(r"\b\d{2,4}[/-]\d{2}[/-]\d{2,4}\b").unwrap(),
    ]
});

static ANSI_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1B\[[0-9;]*[a-zA-Z]").unwrap());

static PID_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(pid|PID|process)[=: ]+\d+\b").unwrap());

static THREAD_ID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // Matches thread IDs in various formats
    Regex::new(r"\b(thread|tid|Thread|TID)[=: ]+\d+\b|\bThread-\d+\b|\bthread-\d+\b|\bworker-\d+\b")
        .unwrap()
});

static FLOAT_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"-?\d+\.\d+([eE][+-]?\d+)?").unwrap());

static UUID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // UUID/GUID pattern: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .unwrap()
});

static FD_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // File descriptor pattern: fd:N, fd=N, FD(N)
    Regex::new(r"\b(fd|FD)[=:(]+\d+[)]?\b").unwrap()
});

static ALLOCATOR_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // Memory allocator info patterns
    Regex::new(r"\b(alloc|malloc|free|realloc|heap)\s*\([^)]+\)").unwrap()
});

/// Output normalizer
pub struct Normalizer {
    config: NormalizationConfig,
    custom_patterns: Vec<Regex>,
}

impl Normalizer {
    /// Create a new normalizer with the given configuration
    pub fn new(config: NormalizationConfig) -> Self {
        let custom_patterns: Vec<Regex> = config
            .custom_strip_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        Self {
            config,
            custom_patterns,
        }
    }

    /// Create a normalizer with default configuration
    pub fn default_config() -> Self {
        Self::new(NormalizationConfig::default())
    }

    /// Normalize the given output string
    pub fn normalize(&self, input: &str) -> String {
        let mut output = input.to_string();

        // Order matters for some transformations

        // 1. Normalize line endings first
        if self.config.normalize_line_endings {
            output = output.replace("\r\n", "\n").replace('\r', "\n");
        }

        // 2. Strip ANSI codes (before other text processing)
        if self.config.strip_ansi_codes {
            output = ANSI_PATTERN.replace_all(&output, "").to_string();
        }

        // 3. Strip memory addresses
        if self.config.strip_addresses {
            output = ADDRESS_PATTERN.replace_all(&output, "<ADDR>").to_string();
        }

        // 4. Strip timestamps
        if self.config.strip_timestamps {
            for pattern in TIMESTAMP_PATTERNS.iter() {
                output = pattern.replace_all(&output, "<TIME>").to_string();
            }
        }

        // 5. Strip process IDs
        if self.config.strip_process_ids {
            output = PID_PATTERN.replace_all(&output, "<PID>").to_string();
        }

        // 6. Strip thread IDs
        if self.config.strip_thread_ids {
            output = THREAD_ID_PATTERN
                .replace_all(&output, "<THREAD>")
                .to_string();
        }

        // 7. Strip UUIDs
        if self.config.strip_uuids {
            output = UUID_PATTERN.replace_all(&output, "<UUID>").to_string();
        }

        // 8. Normalize file descriptors
        if self.config.normalize_file_descriptors {
            output = FD_PATTERN.replace_all(&output, "<FD>").to_string();
        }

        // 9. Strip allocator info
        if self.config.strip_allocator_info {
            output = ALLOCATOR_PATTERN
                .replace_all(&output, "<ALLOC>")
                .to_string();
        }

        // 10. Normalize floats
        if self.config.normalize_floats {
            output = self.normalize_floats(&output);
        }

        // 11. Normalize paths
        if self.config.normalize_paths {
            output = self.normalize_paths(&output);
        }

        // 12. Canonicalize JSON
        if self.config.canonicalize_json {
            output = self.canonicalize_json(&output);
        }

        // 13. Apply custom patterns
        for pattern in &self.custom_patterns {
            output = pattern.replace_all(&output, "<STRIPPED>").to_string();
        }

        // 14. Trim trailing whitespace
        if self.config.trim_trailing_whitespace {
            output = output
                .lines()
                .map(|line| line.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
        }

        // 15. Unicode normalization
        if self.config.normalize_unicode {
            output = self.normalize_unicode(&output);
        }

        output
    }

    /// Normalize floating-point numbers to consistent precision
    fn normalize_floats(&self, input: &str) -> String {
        let precision = self.config.float_precision;

        FLOAT_PATTERN
            .replace_all(input, |caps: &regex::Captures| {
                let float_str = &caps[0];
                if let Ok(f) = float_str.parse::<f64>() {
                    // Handle special values
                    if f.is_nan() {
                        return "NaN".to_string();
                    }
                    if f.is_infinite() {
                        return if f > 0.0 { "Inf" } else { "-Inf" }.to_string();
                    }
                    // Round to specified precision
                    format!("{:.prec$}", f, prec = precision)
                } else {
                    float_str.to_string()
                }
            })
            .to_string()
    }

    /// Normalize file paths for cross-platform comparison
    fn normalize_paths(&self, input: &str) -> String {
        // Replace backslashes with forward slashes
        let mut output = input.replace('\\', "/");

        // Normalize common path patterns
        let path_pattern = Regex::new(r"(?:[A-Za-z]:)?(?:/[^/\s:]+)+").unwrap();
        output = path_pattern
            .replace_all(&output, |caps: &regex::Captures| {
                let path = &caps[0];
                // Remove drive letters for cross-platform
                path.trim_start_matches(|c: char| c.is_ascii_alphabetic())
                    .trim_start_matches(':')
                    .to_string()
            })
            .to_string();

        output
    }

    /// Normalize Unicode to NFC form
    fn normalize_unicode(&self, input: &str) -> String {
        use unicode_normalization::UnicodeNormalization;
        input.nfc().collect()
    }

    /// Canonicalize JSON output (sorted keys, consistent formatting)
    fn canonicalize_json(&self, input: &str) -> String {
        // Try to find JSON objects/arrays in the output and canonicalize them
        let mut result = String::new();
        let mut chars = input.chars().peekable();
        let mut in_string = false;
        let mut current_json = String::new();
        let mut json_depth = 0;
        let mut json_start_char = ' ';

        while let Some(c) = chars.next() {
            if json_depth == 0 && !in_string {
                if c == '{' || c == '[' {
                    json_start_char = c;
                    json_depth = 1;
                    current_json.push(c);
                } else {
                    result.push(c);
                }
            } else if json_depth > 0 {
                current_json.push(c);

                if c == '"' && !in_string {
                    in_string = true;
                } else if c == '"' && in_string {
                    // Check for escape
                    let escaped =
                        current_json.len() >= 2 && current_json.chars().rev().nth(1) == Some('\\');
                    if !escaped {
                        in_string = false;
                    }
                } else if !in_string {
                    if c == '{' || c == '[' {
                        json_depth += 1;
                    } else if c == '}' || c == ']' {
                        json_depth -= 1;
                        if json_depth == 0 {
                            // Try to parse and canonicalize
                            if let Ok(value) =
                                serde_json::from_str::<serde_json::Value>(&current_json)
                            {
                                result.push_str(&canonical_json_string(&value));
                            } else {
                                // Not valid JSON, keep as-is
                                result.push_str(&current_json);
                            }
                            current_json.clear();
                        }
                    }
                }
            }
        }

        // Handle any remaining content
        if !current_json.is_empty() {
            result.push_str(&current_json);
        }

        result
    }

    /// Sort lines within marked unordered blocks
    pub fn sort_unordered_blocks(&self, input: &str) -> String {
        if !self.config.sort_unordered_output {
            return input.to_string();
        }

        let mut output = String::new();
        let mut in_block = false;
        let mut block_lines: Vec<&str> = Vec::new();

        for line in input.lines() {
            if line.contains("@unordered:start") {
                in_block = true;
                output.push_str(line);
                output.push('\n');
            } else if line.contains("@unordered:end") {
                // Sort and output block
                block_lines.sort();
                for block_line in &block_lines {
                    output.push_str(block_line);
                    output.push('\n');
                }
                block_lines.clear();
                in_block = false;
                output.push_str(line);
                output.push('\n');
            } else if in_block {
                block_lines.push(line);
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }

        // Handle unterminated blocks
        for line in block_lines {
            output.push_str(line);
            output.push('\n');
        }

        output.trim_end_matches('\n').to_string() + "\n"
    }
}

impl Default for Normalizer {
    fn default() -> Self {
        Self::default_config()
    }
}

/// Semantic output representation for structured comparison
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NormalizedValue {
    Null,
    Bool(bool),
    Int(i64),
    Float { value: f64, precision: usize },
    Text(String),
    List(Vec<NormalizedValue>),
    Map(BTreeMap<String, NormalizedValue>),
    Set(Vec<NormalizedValue>), // Sorted for comparison
}

impl NormalizedValue {
    /// Parse a string output into structured form
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();

        // Try parsing as various types
        if trimmed.is_empty() || trimmed == "()" || trimmed == "null" || trimmed == "None" {
            return NormalizedValue::Null;
        }

        if trimmed == "true" {
            return NormalizedValue::Bool(true);
        }
        if trimmed == "false" {
            return NormalizedValue::Bool(false);
        }

        if let Ok(i) = trimmed.parse::<i64>() {
            return NormalizedValue::Int(i);
        }

        if let Ok(f) = trimmed.parse::<f64>() {
            return NormalizedValue::Float {
                value: f,
                precision: 10,
            };
        }

        // Check for list syntax
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let inner = &trimmed[1..trimmed.len() - 1];
            let items: Vec<NormalizedValue> = Self::parse_list_items(inner)
                .into_iter()
                .map(|s| NormalizedValue::parse(&s))
                .collect();
            return NormalizedValue::List(items);
        }

        // Check for map/set syntax
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            let inner = &trimmed[1..trimmed.len() - 1];
            if inner.contains(':') {
                // Map
                let mut map = BTreeMap::new();
                for item in Self::parse_list_items(inner) {
                    if let Some((key, value)) = item.split_once(':') {
                        let key = key.trim().trim_matches('"').to_string();
                        let value = NormalizedValue::parse(value.trim());
                        map.insert(key, value);
                    }
                }
                return NormalizedValue::Map(map);
            } else {
                // Set
                let mut items: Vec<NormalizedValue> = Self::parse_list_items(inner)
                    .into_iter()
                    .map(|s| NormalizedValue::parse(&s))
                    .collect();
                items.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));
                return NormalizedValue::Set(items);
            }
        }

        // Default to text
        let text = if trimmed.starts_with('"') && trimmed.ends_with('"') {
            trimmed[1..trimmed.len() - 1].to_string()
        } else {
            trimmed.to_string()
        };

        NormalizedValue::Text(text)
    }

    /// Parse comma-separated items respecting nesting
    fn parse_list_items(input: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut current = String::new();
        let mut depth = 0;
        let mut in_string = false;
        let mut escape = false;

        for c in input.chars() {
            if escape {
                current.push(c);
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => {
                    current.push(c);
                    escape = true;
                }
                '"' => {
                    current.push(c);
                    in_string = !in_string;
                }
                '[' | '{' | '(' if !in_string => {
                    current.push(c);
                    depth += 1;
                }
                ']' | '}' | ')' if !in_string => {
                    current.push(c);
                    depth -= 1;
                }
                ',' if !in_string && depth == 0 => {
                    let item = current.trim().to_string();
                    if !item.is_empty() {
                        items.push(item);
                    }
                    current.clear();
                }
                _ => current.push(c),
            }
        }

        let item = current.trim().to_string();
        if !item.is_empty() {
            items.push(item);
        }

        items
    }

    /// Check approximate equality
    pub fn approx_eq(&self, other: &NormalizedValue, epsilon: f64) -> bool {
        match (self, other) {
            (NormalizedValue::Float { value: a, .. }, NormalizedValue::Float { value: b, .. }) => {
                if a.is_nan() && b.is_nan() {
                    return true;
                }
                if a.is_infinite() && b.is_infinite() {
                    return a.signum() == b.signum();
                }
                (a - b).abs() < epsilon
            }
            (NormalizedValue::List(a), NormalizedValue::List(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y, epsilon))
            }
            (NormalizedValue::Set(a), NormalizedValue::Set(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y, epsilon))
            }
            (NormalizedValue::Map(a), NormalizedValue::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).map_or(false, |v2| v.approx_eq(v2, epsilon)))
            }
            _ => self == other,
        }
    }
}

/// Convert a JSON value to canonical form (sorted keys, consistent formatting)
fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            // Sort keys and recursively canonicalize values
            let mut sorted_pairs: Vec<_> = map.iter().collect();
            sorted_pairs.sort_by(|a, b| a.0.cmp(b.0));

            let pairs: Vec<String> = sorted_pairs
                .into_iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonical_json_string(v)))
                .collect();

            format!("{{{}}}", pairs.join(","))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json_string).collect();
            format!("[{}]", items.join(","))
        }
        serde_json::Value::Number(n) => {
            // Normalize number representation
            if let Some(f) = n.as_f64() {
                if f.is_nan() {
                    "\"NaN\"".to_string()
                } else if f.is_infinite() {
                    if f > 0.0 {
                        "\"Infinity\"".to_string()
                    } else {
                        "\"-Infinity\"".to_string()
                    }
                } else if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
                    // Integer-like float
                    format!("{}", f as i64)
                } else {
                    // Use scientific notation for very large/small numbers
                    if f.abs() > 1e15 || (f.abs() < 1e-10 && f != 0.0) {
                        format!("{:e}", f)
                    } else {
                        format!("{}", f)
                    }
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => {
            // Escape and quote string
            serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
}

/// Normalize thread ID mapping to consistent identifiers
pub fn normalize_thread_mapping(output: &str) -> (String, BTreeMap<String, String>) {
    let mut mapping: BTreeMap<String, String> = BTreeMap::new();
    let mut counter = 0;

    let result = THREAD_ID_PATTERN
        .replace_all(output, |caps: &regex::Captures| {
            let thread_id = &caps[0];
            if let Some(normalized) = mapping.get(thread_id) {
                normalized.clone()
            } else {
                let normalized = format!("thread-{}", counter);
                counter += 1;
                mapping.insert(thread_id.to_string(), normalized.clone());
                normalized
            }
        })
        .to_string();

    (result, mapping)
}

/// Normalize memory address mapping to consistent identifiers
pub fn normalize_address_mapping(output: &str) -> (String, BTreeMap<String, String>) {
    let mut mapping: BTreeMap<String, String> = BTreeMap::new();
    let mut counter = 0;

    let result = ADDRESS_PATTERN
        .replace_all(output, |caps: &regex::Captures| {
            let addr = &caps[0];
            if let Some(normalized) = mapping.get(addr) {
                normalized.clone()
            } else {
                let normalized = format!("0xADDR_{}", counter);
                counter += 1;
                mapping.insert(addr.to_string(), normalized.clone());
                normalized
            }
        })
        .to_string();

    (result, mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_addresses() {
        let normalizer = Normalizer::new(NormalizationConfig {
            strip_addresses: true,
            ..Default::default()
        });

        let input = "Object at 0x7fff1234abcd";
        let output = normalizer.normalize(input);
        assert_eq!(output, "Object at <ADDR>");
    }

    #[test]
    fn test_normalize_floats() {
        let normalizer = Normalizer::new(NormalizationConfig {
            normalize_floats: true,
            float_precision: 3,
            ..Default::default()
        });

        let input = "Value: 3.14159265";
        let output = normalizer.normalize(input);
        assert_eq!(output, "Value: 3.142");
    }

    #[test]
    fn test_normalize_timestamps() {
        let normalizer = Normalizer::new(NormalizationConfig {
            strip_timestamps: true,
            ..Default::default()
        });

        let input = "Logged at 2024-01-15T10:30:45.123Z";
        let output = normalizer.normalize(input);
        assert!(output.contains("<TIME>"));
    }

    #[test]
    fn test_normalize_line_endings() {
        let normalizer = Normalizer::new(NormalizationConfig {
            normalize_line_endings: true,
            ..Default::default()
        });

        let input = "line1\r\nline2\rline3";
        let output = normalizer.normalize(input);
        assert_eq!(output, "line1\nline2\nline3");
    }

    #[test]
    fn test_normalize_ansi() {
        let normalizer = Normalizer::new(NormalizationConfig {
            strip_ansi_codes: true,
            ..Default::default()
        });

        let input = "\x1B[31mError\x1B[0m: failed";
        let output = normalizer.normalize(input);
        assert_eq!(output, "Error: failed");
    }

    #[test]
    fn test_normalized_value_parse() {
        assert_eq!(NormalizedValue::parse("42"), NormalizedValue::Int(42));
        assert_eq!(NormalizedValue::parse("true"), NormalizedValue::Bool(true));
        assert_eq!(
            NormalizedValue::parse("[1, 2, 3]"),
            NormalizedValue::List(vec![
                NormalizedValue::Int(1),
                NormalizedValue::Int(2),
                NormalizedValue::Int(3),
            ])
        );
    }

    #[test]
    fn test_float_approx_eq() {
        let a = NormalizedValue::Float {
            value: 1.0,
            precision: 10,
        };
        let b = NormalizedValue::Float {
            value: 1.0 + 1e-11,
            precision: 10,
        };
        assert!(a.approx_eq(&b, 1e-10));
    }
}
