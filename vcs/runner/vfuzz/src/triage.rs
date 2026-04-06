//! Comprehensive crash triage and deduplication
//!
//! This module provides intelligent crash analysis to:
//! - Deduplicate crashes by root cause
//! - Classify crash types
//! - Extract minimal stack traces
//! - Generate bug reports
//! - Detect regressions
//! - Assess severity
//!
//! # Deduplication Strategy
//!
//! 1. Stack trace hashing (with normalization)
//! 2. Source location clustering
//! 3. Panic message similarity
//! 4. Crash signature extraction
//! 5. Locality-sensitive hashing for fuzzy matching
//!
//! # Severity Classification
//!
//! - **Critical**: Memory corruption, security issues, CBGR violations
//! - **High**: Compiler crashes, type system unsoundness
//! - **Medium**: SMT timeouts, OOM, stack overflow
//! - **Low**: Lexer/parser issues on malformed input
//!
//! # Regression Detection
//!
//! Compares crashes against a baseline to detect:
//! - New crashes introduced since baseline
//! - Fixed crashes (present in baseline but not current)
//! - Persistent crashes (present in both)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// Classification of a crash
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CrashClass {
    /// Compiler panic
    CompilerPanic(PanicKind),
    /// Lexer error (infinite loop, invalid token)
    LexerCrash,
    /// Parser crash (stack overflow, invalid state)
    ParserCrash,
    /// Type checker crash
    TypeCheckerCrash,
    /// CBGR violation
    CbgrViolation,
    /// SMT solver timeout or crash
    SmtCrash,
    /// Code generator crash
    CodegenCrash,
    /// Runtime crash
    RuntimeCrash(RuntimeCrashKind),
    /// Out of memory
    OutOfMemory,
    /// Stack overflow
    StackOverflow,
    /// Segmentation fault
    Segfault,
    /// Assertion failure
    AssertionFailure,
    /// Unknown/unclassified
    Unknown,
}

/// Kind of panic
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanicKind {
    /// Explicit panic! call
    Explicit,
    /// Unwrap on None
    UnwrapNone,
    /// Unwrap on Err
    UnwrapErr,
    /// Index out of bounds
    IndexOutOfBounds,
    /// Integer overflow
    IntegerOverflow,
    /// Division by zero
    DivisionByZero,
    /// Unreachable code
    Unreachable,
    /// Other panic
    Other,
}

/// Kind of runtime crash
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeCrashKind {
    /// Null pointer dereference
    NullPointer,
    /// Use after free
    UseAfterFree,
    /// Double free
    DoubleFree,
    /// Buffer overflow
    BufferOverflow,
    /// Data race
    DataRace,
    /// Deadlock
    Deadlock,
    /// Other runtime error
    Other,
}

/// A normalized stack frame
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StackFrame {
    /// Function name (normalized)
    pub function: String,
    /// File path (normalized, no absolute paths)
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Column number
    pub column: Option<u32>,
}

impl StackFrame {
    /// Create a new stack frame
    pub fn new(function: &str, file: Option<&str>, line: Option<u32>) -> Self {
        Self {
            function: Self::normalize_function(function),
            file: file.map(Self::normalize_path),
            line,
            column: None,
        }
    }

    /// Normalize function name (remove addresses, generics noise)
    fn normalize_function(name: &str) -> String {
        // Remove memory addresses
        let name = regex::Regex::new(r"0x[0-9a-fA-F]+")
            .unwrap()
            .replace_all(name, "0xADDR")
            .to_string();

        // Simplify generics (keep just the base type)
        let name = regex::Regex::new(r"<[^>]+>")
            .unwrap()
            .replace_all(&name, "<T>")
            .to_string();

        // Remove closure numbers
        let name = regex::Regex::new(r"\{\{closure\}\}#\d+")
            .unwrap()
            .replace_all(&name, "{{closure}}")
            .to_string();

        name
    }

    /// Normalize file path (remove user-specific parts)
    fn normalize_path(path: &str) -> String {
        // Remove home directory prefix
        let path = regex::Regex::new(r"/home/[^/]+/")
            .unwrap()
            .replace(path, "/~/")
            .to_string();

        let path = regex::Regex::new(r"/Users/[^/]+/")
            .unwrap()
            .replace(&path, "/~/")
            .to_string();

        // Remove cargo registry paths
        let path = regex::Regex::new(r"\.cargo/registry/[^/]+/[^/]+/")
            .unwrap()
            .replace(&path, ".cargo/registry/")
            .to_string();

        path
    }
}

/// A crash signature for deduplication
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CrashSignature {
    /// Classification
    pub class: CrashClass,
    /// Normalized top frames of stack (for dedup)
    pub top_frames: Vec<StackFrame>,
    /// Hash of the signature
    pub hash: String,
    /// Panic message (normalized)
    pub message: Option<String>,
}

impl CrashSignature {
    /// Create a signature from raw crash data
    pub fn from_crash(error: &str, backtrace: &str) -> Self {
        let class = Self::classify(error, backtrace);
        let top_frames = Self::extract_top_frames(backtrace, 5);
        let message = Self::extract_message(error);

        let hash = Self::compute_hash(&class, &top_frames, &message);

        Self {
            class,
            top_frames,
            hash,
            message,
        }
    }

    /// Classify the crash type
    fn classify(error: &str, backtrace: &str) -> CrashClass {
        let error_lower = error.to_lowercase();
        let bt_lower = backtrace.to_lowercase();

        // Check backtrace for component FIRST (most specific classification)
        if bt_lower.contains("verum_lexer") {
            return CrashClass::LexerCrash;
        }
        if bt_lower.contains("verum_parser") {
            return CrashClass::ParserCrash;
        }
        if bt_lower.contains("verum_types") || bt_lower.contains("type_check") {
            return CrashClass::TypeCheckerCrash;
        }
        if bt_lower.contains("verum_cbgr") {
            return CrashClass::CbgrViolation;
        }
        if bt_lower.contains("verum_smt") || bt_lower.contains("z3") {
            return CrashClass::SmtCrash;
        }
        if bt_lower.contains("verum_codegen") || bt_lower.contains("inkwell") {
            return CrashClass::CodegenCrash;
        }
        if bt_lower.contains("verum_runtime") {
            return CrashClass::RuntimeCrash(RuntimeCrashKind::Other);
        }

        // Check for memory issues
        if error_lower.contains("out of memory") || error_lower.contains("oom") {
            return CrashClass::OutOfMemory;
        }
        if error_lower.contains("stack overflow") {
            return CrashClass::StackOverflow;
        }
        if error_lower.contains("segmentation fault") || error_lower.contains("sigsegv") {
            return CrashClass::Segfault;
        }
        if error_lower.contains("assertion") {
            return CrashClass::AssertionFailure;
        }

        // Check for specific panic types (for crashes without component info)
        if error_lower.contains("panicked at") {
            let panic_kind = if error_lower.contains("unwrap()` on a `none`") {
                PanicKind::UnwrapNone
            } else if error_lower.contains("unwrap()` on an `err`") {
                PanicKind::UnwrapErr
            } else if error_lower.contains("index out of bounds") {
                PanicKind::IndexOutOfBounds
            } else if error_lower.contains("overflow") {
                PanicKind::IntegerOverflow
            } else if error_lower.contains("divide by zero")
                || error_lower.contains("division by zero")
            {
                PanicKind::DivisionByZero
            } else if error_lower.contains("unreachable") {
                PanicKind::Unreachable
            } else if error_lower.contains("explicit panic") {
                PanicKind::Explicit
            } else {
                PanicKind::Other
            };
            return CrashClass::CompilerPanic(panic_kind);
        }

        CrashClass::Unknown
    }

    /// Extract top N frames from backtrace
    fn extract_top_frames(backtrace: &str, count: usize) -> Vec<StackFrame> {
        let frame_re = regex::Regex::new(
            r"(?m)^\s*\d+:\s*(?:0x[0-9a-fA-F]+\s*-\s*)?(.+?)(?:\n\s*at\s+(.+?):(\d+))?",
        )
        .unwrap();

        let mut frames = Vec::new();
        for cap in frame_re.captures_iter(backtrace).take(count * 2) {
            let function = cap.get(1).map(|m| m.as_str()).unwrap_or("unknown");

            // Skip internal frames
            if function.contains("rust_begin_unwind")
                || function.contains("__rust_")
                || function.contains("std::panicking")
                || function.contains("core::panicking")
            {
                continue;
            }

            let file = cap.get(2).map(|m| m.as_str());
            let line = cap.get(3).and_then(|m| m.as_str().parse().ok());

            frames.push(StackFrame::new(function, file, line));

            if frames.len() >= count {
                break;
            }
        }

        frames
    }

    /// Extract and normalize the panic message
    fn extract_message(error: &str) -> Option<String> {
        // Extract the main message
        let msg_re = regex::Regex::new(r"panicked at '([^']+)'").ok()?;
        if let Some(cap) = msg_re.captures(error) {
            let msg = cap.get(1)?.as_str();

            // Normalize variable parts
            let msg = regex::Regex::new(r"\d+")
                .unwrap()
                .replace_all(msg, "N")
                .to_string();

            return Some(msg);
        }

        None
    }

    /// Compute hash for deduplication
    fn compute_hash(class: &CrashClass, frames: &[StackFrame], message: &Option<String>) -> String {
        let mut hasher = Sha256::new();

        // Include class
        hasher.update(format!("{:?}", class).as_bytes());

        // Include top frames
        for frame in frames {
            hasher.update(frame.function.as_bytes());
            if let Some(ref file) = frame.file {
                hasher.update(file.as_bytes());
            }
        }

        // Include normalized message
        if let Some(msg) = message {
            hasher.update(msg.as_bytes());
        }

        let hash = hasher.finalize();
        hex::encode(&hash[..16])
    }
}

/// Crash triage engine
pub struct CrashTriager {
    /// Known crash signatures
    known_signatures: HashMap<String, CrashInfo>,
    /// Signature to crash IDs mapping
    signature_to_crashes: HashMap<String, Vec<String>>,
    /// Total crashes seen
    total_crashes: usize,
    /// Unique crashes
    unique_crashes: usize,
}

/// Information about a crash group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashInfo {
    /// The signature
    pub signature: CrashSignature,
    /// First occurrence timestamp
    pub first_seen: u64,
    /// Last occurrence timestamp
    pub last_seen: u64,
    /// Number of occurrences
    pub occurrences: usize,
    /// Representative input (smallest)
    pub representative_input: String,
    /// Representative input size
    pub representative_size: usize,
    /// Has been minimized
    pub minimized: bool,
    /// Bug report generated
    pub reported: bool,
}

impl Default for CrashTriager {
    fn default() -> Self {
        Self::new()
    }
}

impl CrashTriager {
    /// Maximum number of crash IDs to keep per signature (prevents unbounded memory growth)
    const MAX_CRASH_IDS_PER_SIGNATURE: usize = 100;
    /// Maximum size for representative input (prevents large memory usage)
    const MAX_REPRESENTATIVE_SIZE: usize = 10_000;

    /// Create a new crash triager
    pub fn new() -> Self {
        Self {
            known_signatures: HashMap::new(),
            signature_to_crashes: HashMap::new(),
            total_crashes: 0,
            unique_crashes: 0,
        }
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.known_signatures.clear();
        self.signature_to_crashes.clear();
        self.total_crashes = 0;
        self.unique_crashes = 0;
    }

    /// Triage a new crash
    /// Returns (is_new, signature_hash)
    pub fn triage(
        &mut self,
        crash_id: &str,
        input: &str,
        error: &str,
        backtrace: &str,
    ) -> (bool, String) {
        self.total_crashes += 1;

        let signature = CrashSignature::from_crash(error, backtrace);
        let hash = signature.hash.clone();

        // Check if we've seen this signature before
        if let Some(info) = self.known_signatures.get_mut(&hash) {
            info.last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            info.occurrences += 1;

            // Keep the smallest input as representative (with size limit)
            if input.len() < info.representative_size {
                let truncated = if input.len() > Self::MAX_REPRESENTATIVE_SIZE {
                    format!("{}...[truncated]", &input[..Self::MAX_REPRESENTATIVE_SIZE])
                } else {
                    input.to_string()
                };
                info.representative_input = truncated;
                info.representative_size = input.len().min(Self::MAX_REPRESENTATIVE_SIZE);
            }

            // Track crash IDs with this signature (with limit to prevent unbounded growth)
            let crash_ids = self.signature_to_crashes.entry(hash.clone()).or_default();
            crash_ids.push(crash_id.to_string());
            // Keep only the last N crash IDs to prevent memory leak
            if crash_ids.len() > Self::MAX_CRASH_IDS_PER_SIGNATURE {
                crash_ids.remove(0);
            }

            return (false, hash);
        }

        // New unique crash
        self.unique_crashes += 1;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Truncate input if too large to prevent memory issues
        let truncated_input = if input.len() > Self::MAX_REPRESENTATIVE_SIZE {
            format!("{}...[truncated]", &input[..Self::MAX_REPRESENTATIVE_SIZE])
        } else {
            input.to_string()
        };

        let info = CrashInfo {
            signature: signature.clone(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            representative_input: truncated_input,
            representative_size: input.len().min(Self::MAX_REPRESENTATIVE_SIZE),
            minimized: false,
            reported: false,
        };

        self.known_signatures.insert(hash.clone(), info);
        self.signature_to_crashes
            .entry(hash.clone())
            .or_default()
            .push(crash_id.to_string());

        (true, hash)
    }

    /// Get crash info by signature hash
    pub fn get_crash_info(&self, hash: &str) -> Option<&CrashInfo> {
        self.known_signatures.get(hash)
    }

    /// Get all unique signatures
    pub fn unique_signatures(&self) -> Vec<&CrashSignature> {
        self.known_signatures
            .values()
            .map(|info| &info.signature)
            .collect()
    }

    /// Get statistics
    pub fn stats(&self) -> TriageStats {
        let mut by_class: HashMap<String, usize> = HashMap::new();

        for info in self.known_signatures.values() {
            let class_name = format!("{:?}", info.signature.class);
            *by_class.entry(class_name).or_default() += info.occurrences;
        }

        TriageStats {
            total_crashes: self.total_crashes,
            unique_crashes: self.unique_crashes,
            dedup_ratio: if self.total_crashes > 0 {
                1.0 - (self.unique_crashes as f64 / self.total_crashes as f64)
            } else {
                0.0
            },
            by_class,
        }
    }

    /// Mark a crash as minimized
    pub fn mark_minimized(&mut self, hash: &str, minimized_input: &str) {
        if let Some(info) = self.known_signatures.get_mut(hash) {
            info.minimized = true;
            info.representative_input = minimized_input.to_string();
            info.representative_size = minimized_input.len();
        }
    }

    /// Generate a bug report
    pub fn generate_report(&self, hash: &str) -> Option<BugReport> {
        let info = self.known_signatures.get(hash)?;

        Some(BugReport {
            id: hash.to_string(),
            title: self.generate_title(info),
            classification: info.signature.class.clone(),
            stack_trace: info.signature.top_frames.clone(),
            reproducer: info.representative_input.clone(),
            occurrences: info.occurrences,
            first_seen: info.first_seen,
            severity: self.assess_severity(&info.signature.class),
        })
    }

    /// Generate a bug title
    fn generate_title(&self, info: &CrashInfo) -> String {
        let class_str = match &info.signature.class {
            CrashClass::CompilerPanic(kind) => format!("Compiler panic: {:?}", kind),
            CrashClass::LexerCrash => "Lexer crash".to_string(),
            CrashClass::ParserCrash => "Parser crash".to_string(),
            CrashClass::TypeCheckerCrash => "Type checker crash".to_string(),
            CrashClass::CbgrViolation => "CBGR violation".to_string(),
            CrashClass::SmtCrash => "SMT solver crash".to_string(),
            CrashClass::CodegenCrash => "Code generator crash".to_string(),
            CrashClass::RuntimeCrash(kind) => format!("Runtime crash: {:?}", kind),
            CrashClass::OutOfMemory => "Out of memory".to_string(),
            CrashClass::StackOverflow => "Stack overflow".to_string(),
            CrashClass::Segfault => "Segmentation fault".to_string(),
            CrashClass::AssertionFailure => "Assertion failure".to_string(),
            CrashClass::Unknown => "Unknown crash".to_string(),
        };

        if let Some(ref frame) = info.signature.top_frames.first() {
            format!("{} in {}", class_str, frame.function)
        } else {
            class_str
        }
    }

    /// Assess severity of a crash
    fn assess_severity(&self, class: &CrashClass) -> Severity {
        match class {
            CrashClass::Segfault | CrashClass::RuntimeCrash(_) | CrashClass::CbgrViolation => {
                Severity::Critical
            }
            CrashClass::CompilerPanic(_)
            | CrashClass::TypeCheckerCrash
            | CrashClass::CodegenCrash => Severity::High,
            CrashClass::StackOverflow | CrashClass::OutOfMemory | CrashClass::SmtCrash => {
                Severity::Medium
            }
            CrashClass::LexerCrash | CrashClass::ParserCrash | CrashClass::AssertionFailure => {
                Severity::Low
            }
            CrashClass::Unknown => Severity::Medium,
        }
    }

    /// Export all crashes as JSON
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.known_signatures).unwrap_or_default()
    }
}

/// Triage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageStats {
    /// Total crashes triaged
    pub total_crashes: usize,
    /// Unique crashes found
    pub unique_crashes: usize,
    /// Deduplication ratio (0.0 - 1.0)
    pub dedup_ratio: f64,
    /// Crashes by class
    pub by_class: HashMap<String, usize>,
}

/// Severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Severity {
    /// Critical security issue or memory corruption
    Critical,
    /// High-impact bug
    High,
    /// Medium-impact bug
    Medium,
    /// Low-impact bug
    Low,
}

/// A generated bug report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugReport {
    /// Unique ID
    pub id: String,
    /// Bug title
    pub title: String,
    /// Classification
    pub classification: CrashClass,
    /// Top stack frames
    pub stack_trace: Vec<StackFrame>,
    /// Reproducer input
    pub reproducer: String,
    /// Number of occurrences
    pub occurrences: usize,
    /// First seen timestamp
    pub first_seen: u64,
    /// Severity assessment
    pub severity: Severity,
}

impl BugReport {
    /// Format as markdown
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!("# {}\n\n", self.title));
        md.push_str(&format!("**ID:** {}\n", self.id));
        md.push_str(&format!("**Severity:** {:?}\n", self.severity));
        md.push_str(&format!("**Classification:** {:?}\n", self.classification));
        md.push_str(&format!("**Occurrences:** {}\n\n", self.occurrences));

        md.push_str("## Stack Trace\n\n");
        md.push_str("```\n");
        for (i, frame) in self.stack_trace.iter().enumerate() {
            md.push_str(&format!("{}: {}", i, frame.function));
            if let (Some(file), Some(line)) = (&frame.file, frame.line) {
                md.push_str(&format!("\n   at {}:{}", file, line));
            }
            md.push('\n');
        }
        md.push_str("```\n\n");

        md.push_str("## Reproducer\n\n");
        md.push_str("```verum\n");
        md.push_str(&self.reproducer);
        md.push_str("\n```\n");

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_signature() {
        let error =
            "thread 'main' panicked at 'index out of bounds: the len is 0 but the index is 1'";
        let backtrace = r#"
   0: verum_parser::parse_expr
             at src/parser.rs:123
   1: verum_parser::parse_statement
             at src/parser.rs:456
        "#;

        let sig = CrashSignature::from_crash(error, backtrace);

        // Should be classified as ParserCrash since backtrace has verum_parser
        assert!(matches!(sig.class, CrashClass::ParserCrash));
        assert!(!sig.top_frames.is_empty());
    }

    #[test]
    fn test_crash_signature_panic_type() {
        // Test crash classification by panic type when no component in backtrace
        let error =
            "thread 'main' panicked at 'index out of bounds: the len is 0 but the index is 1'";
        let backtrace = r#"
   0: some_function
             at src/unknown.rs:123
        "#;

        let sig = CrashSignature::from_crash(error, backtrace);

        assert!(matches!(
            sig.class,
            CrashClass::CompilerPanic(PanicKind::IndexOutOfBounds)
        ));
    }

    #[test]
    fn test_triage_deduplication() {
        let mut triager = CrashTriager::new();

        let error = "panicked at 'unwrap()` on a `None`'";
        let backtrace = "0: some_function at file.rs:10";

        let (is_new1, hash1) = triager.triage("crash1", "input1", error, backtrace);
        assert!(is_new1);

        let (is_new2, hash2) = triager.triage("crash2", "input2", error, backtrace);
        assert!(!is_new2);
        assert_eq!(hash1, hash2);

        let stats = triager.stats();
        assert_eq!(stats.total_crashes, 2);
        assert_eq!(stats.unique_crashes, 1);
    }

    #[test]
    fn test_crash_classification() {
        let cases = vec![
            (
                "panicked at 'explicit panic'",
                "verum_lexer::lex",
                CrashClass::LexerCrash,
            ),
            (
                "panicked at 'oops'",
                "verum_parser::parse",
                CrashClass::ParserCrash,
            ),
            (
                "panicked at 'oops'",
                "verum_types::infer",
                CrashClass::TypeCheckerCrash,
            ),
            ("stack overflow", "some_func", CrashClass::StackOverflow),
            ("segmentation fault", "some_func", CrashClass::Segfault),
        ];

        for (error, bt_func, expected_class) in cases {
            let backtrace = format!("0: {}", bt_func);
            let sig = CrashSignature::from_crash(error, &backtrace);
            assert_eq!(sig.class, expected_class, "Failed for error: {}", error);
        }
    }

    #[test]
    fn test_bug_report_generation() {
        let mut triager = CrashTriager::new();

        let error = "panicked at 'index out of bounds'";
        let backtrace = "0: verum_parser::parse at src/parser.rs:123";
        let input = "fn main() { x[100] }";

        let (_, hash) = triager.triage("crash1", input, error, backtrace);

        let report = triager.generate_report(&hash).unwrap();
        assert!(!report.title.is_empty());
        assert_eq!(report.reproducer, input);

        let md = report.to_markdown();
        assert!(md.contains("# "));
        assert!(md.contains("## Reproducer"));
    }
}

// ============================================================================
// Enhanced Deduplication
// ============================================================================

/// Configuration for deduplication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    /// Number of top stack frames to consider
    pub top_frames: usize,
    /// Enable fuzzy matching
    pub fuzzy_matching: bool,
    /// Similarity threshold for fuzzy matching (0.0 - 1.0)
    pub similarity_threshold: f64,
    /// Ignore line numbers in deduplication
    pub ignore_line_numbers: bool,
    /// Group by component only
    pub component_only: bool,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            top_frames: 5,
            fuzzy_matching: true,
            similarity_threshold: 0.8,
            ignore_line_numbers: false,
            component_only: false,
        }
    }
}

/// Enhanced crash deduplicator with fuzzy matching
pub struct CrashDeduplicator {
    /// Configuration
    config: DedupConfig,
    /// Known signatures with their buckets
    signatures: HashMap<String, SignatureBucket>,
    /// Statistics
    stats: DedupStats,
}

/// A bucket of similar signatures
#[derive(Debug, Clone)]
pub struct SignatureBucket {
    /// Primary signature
    primary: CrashSignature,
    /// Similar signatures that were merged
    merged: Vec<CrashSignature>,
    /// All crash IDs in this bucket
    crash_ids: Vec<String>,
    /// Representative input
    representative: String,
}

/// Deduplication statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DedupStats {
    /// Total crashes processed
    pub total: usize,
    /// Exact duplicates found
    pub exact_duplicates: usize,
    /// Fuzzy matches found
    pub fuzzy_matches: usize,
    /// Unique crashes
    pub unique: usize,
}

impl Default for CrashDeduplicator {
    fn default() -> Self {
        Self::new(DedupConfig::default())
    }
}

impl CrashDeduplicator {
    /// Maximum number of merged signatures to keep per bucket
    const MAX_MERGED_SIGNATURES: usize = 10;
    /// Maximum number of crash IDs to keep per bucket
    const MAX_CRASH_IDS_PER_BUCKET: usize = 100;
    /// Maximum representative input size
    const MAX_REPRESENTATIVE_SIZE: usize = 10_000;

    /// Create a new deduplicator
    pub fn new(config: DedupConfig) -> Self {
        Self {
            config,
            signatures: HashMap::new(),
            stats: DedupStats::default(),
        }
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.signatures.clear();
        self.stats = DedupStats::default();
    }

    /// Process a crash and return whether it's new
    pub fn process(&mut self, crash_id: &str, signature: CrashSignature, input: &str) -> bool {
        self.stats.total += 1;

        // Check for exact match
        if let Some(bucket) = self.signatures.get_mut(&signature.hash) {
            // Limit crash IDs to prevent unbounded growth
            if bucket.crash_ids.len() < Self::MAX_CRASH_IDS_PER_BUCKET {
                bucket.crash_ids.push(crash_id.to_string());
            }
            if input.len() < bucket.representative.len() {
                let truncated = if input.len() > Self::MAX_REPRESENTATIVE_SIZE {
                    format!("{}...[truncated]", &input[..Self::MAX_REPRESENTATIVE_SIZE])
                } else {
                    input.to_string()
                };
                bucket.representative = truncated;
            }
            self.stats.exact_duplicates += 1;
            return false;
        }

        // Check for fuzzy match if enabled
        if self.config.fuzzy_matching {
            // First, find matching hash by checking similarity
            let matching_hash = self
                .signatures
                .iter()
                .find(|(_hash, bucket)| self.is_similar_signatures(&signature, &bucket.primary))
                .map(|(hash, _)| hash.clone());

            if let Some(hash) = matching_hash {
                if let Some(bucket) = self.signatures.get_mut(&hash) {
                    // Limit merged signatures to prevent unbounded growth
                    if bucket.merged.len() < Self::MAX_MERGED_SIGNATURES {
                        bucket.merged.push(signature);
                    }
                    // Limit crash IDs to prevent unbounded growth
                    if bucket.crash_ids.len() < Self::MAX_CRASH_IDS_PER_BUCKET {
                        bucket.crash_ids.push(crash_id.to_string());
                    }
                    if input.len() < bucket.representative.len() {
                        let truncated = if input.len() > Self::MAX_REPRESENTATIVE_SIZE {
                            format!("{}...[truncated]", &input[..Self::MAX_REPRESENTATIVE_SIZE])
                        } else {
                            input.to_string()
                        };
                        bucket.representative = truncated;
                    }
                    self.stats.fuzzy_matches += 1;
                    return false;
                }
            }
        }

        // New unique crash
        self.stats.unique += 1;
        // Truncate input if too large
        let truncated_input = if input.len() > Self::MAX_REPRESENTATIVE_SIZE {
            format!("{}...[truncated]", &input[..Self::MAX_REPRESENTATIVE_SIZE])
        } else {
            input.to_string()
        };
        let bucket = SignatureBucket {
            primary: signature.clone(),
            merged: Vec::new(),
            crash_ids: vec![crash_id.to_string()],
            representative: truncated_input,
        };
        self.signatures.insert(signature.hash.clone(), bucket);

        true
    }

    /// Check if two signatures are similar
    fn is_similar(&self, a: &CrashSignature, b: &CrashSignature) -> bool {
        self.is_similar_signatures(a, b)
    }

    /// Check if two signatures are similar (takes only immutable borrows)
    fn is_similar_signatures(&self, a: &CrashSignature, b: &CrashSignature) -> bool {
        // Must be same class
        if a.class != b.class {
            return false;
        }

        // Calculate stack trace similarity
        let similarity = self.stack_similarity(&a.top_frames, &b.top_frames);
        similarity >= self.config.similarity_threshold
    }

    /// Calculate similarity between two stack traces
    fn stack_similarity(&self, a: &[StackFrame], b: &[StackFrame]) -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let mut matches = 0;
        let max_len = a.len().max(b.len());

        for (frame_a, frame_b) in a.iter().zip(b.iter()) {
            if self.frames_match(frame_a, frame_b) {
                matches += 1;
            }
        }

        matches as f64 / max_len as f64
    }

    /// Check if two frames match
    fn frames_match(&self, a: &StackFrame, b: &StackFrame) -> bool {
        if a.function != b.function {
            return false;
        }

        if self.config.ignore_line_numbers {
            return true;
        }

        // If we have file info, compare it
        match (&a.file, &b.file) {
            (Some(fa), Some(fb)) => fa == fb && a.line == b.line,
            _ => true, // No file info, consider matching
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &DedupStats {
        &self.stats
    }

    /// Get all signature buckets
    pub fn buckets(&self) -> &HashMap<String, SignatureBucket> {
        &self.signatures
    }

    /// Get deduplication ratio
    pub fn dedup_ratio(&self) -> f64 {
        if self.stats.total == 0 {
            return 0.0;
        }
        1.0 - (self.stats.unique as f64 / self.stats.total as f64)
    }
}

// ============================================================================
// Enhanced Severity Classification
// ============================================================================

/// Detailed severity assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityAssessment {
    /// Overall severity level
    pub level: Severity,
    /// Score (0-100, higher is more severe)
    pub score: u32,
    /// Contributing factors
    pub factors: Vec<SeverityFactor>,
    /// Recommended priority
    pub priority: Priority,
    /// Security implications
    pub security_relevant: bool,
}

/// Factor contributing to severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityFactor {
    /// Factor name
    pub name: String,
    /// Factor contribution to score
    pub contribution: u32,
    /// Description
    pub description: String,
}

/// Priority level for fixing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Fix immediately (P0)
    Immediate,
    /// Fix in current milestone (P1)
    High,
    /// Fix when possible (P2)
    Medium,
    /// Nice to fix (P3)
    Low,
    /// Won't fix / by design
    WontFix,
}

/// Enhanced severity classifier
pub struct SeverityClassifier {
    /// Configuration
    config: SeverityConfig,
}

/// Configuration for severity classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityConfig {
    /// Base scores by crash class
    pub class_scores: HashMap<String, u32>,
    /// Modifiers for specific conditions
    pub modifiers: Vec<SeverityModifier>,
}

/// A modifier that adjusts severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityModifier {
    /// Name
    pub name: String,
    /// Pattern to match (in error or stack trace)
    pub pattern: String,
    /// Score adjustment (can be negative)
    pub adjustment: i32,
}

impl Default for SeverityConfig {
    fn default() -> Self {
        let mut class_scores = HashMap::new();
        class_scores.insert("Segfault".to_string(), 95);
        class_scores.insert("CbgrViolation".to_string(), 90);
        class_scores.insert("RuntimeCrash".to_string(), 85);
        class_scores.insert("TypeCheckerCrash".to_string(), 75);
        class_scores.insert("CodegenCrash".to_string(), 70);
        class_scores.insert("CompilerPanic".to_string(), 60);
        class_scores.insert("SmtCrash".to_string(), 50);
        class_scores.insert("StackOverflow".to_string(), 45);
        class_scores.insert("OutOfMemory".to_string(), 40);
        class_scores.insert("ParserCrash".to_string(), 35);
        class_scores.insert("LexerCrash".to_string(), 30);
        class_scores.insert("AssertionFailure".to_string(), 25);
        class_scores.insert("Unknown".to_string(), 50);

        let modifiers = vec![
            SeverityModifier {
                name: "Use After Free".to_string(),
                pattern: "use.after.free".to_string(),
                adjustment: 20,
            },
            SeverityModifier {
                name: "Double Free".to_string(),
                pattern: "double.free".to_string(),
                adjustment: 20,
            },
            SeverityModifier {
                name: "Buffer Overflow".to_string(),
                pattern: "buffer.overflow".to_string(),
                adjustment: 25,
            },
            SeverityModifier {
                name: "Small Input".to_string(),
                pattern: "small_input".to_string(),
                adjustment: 10,
            },
            SeverityModifier {
                name: "Common Pattern".to_string(),
                pattern: "common_pattern".to_string(),
                adjustment: -5,
            },
        ];

        Self {
            class_scores,
            modifiers,
        }
    }
}

impl Default for SeverityClassifier {
    fn default() -> Self {
        Self::new(SeverityConfig::default())
    }
}

impl SeverityClassifier {
    /// Create a new severity classifier
    pub fn new(config: SeverityConfig) -> Self {
        Self { config }
    }

    /// Assess severity of a crash
    pub fn assess(&self, class: &CrashClass, error: &str, input_size: usize) -> SeverityAssessment {
        let mut score = self.base_score(class);
        let mut factors = Vec::new();

        // Add base class factor
        factors.push(SeverityFactor {
            name: "Crash Class".to_string(),
            contribution: score,
            description: format!("{:?}", class),
        });

        // Apply modifiers
        let error_lower = error.to_lowercase();
        for modifier in &self.config.modifiers {
            let pattern = modifier.pattern.to_lowercase().replace('.', ".*");
            if let Ok(re) = regex::Regex::new(&pattern) {
                if re.is_match(&error_lower) {
                    let adj = modifier.adjustment;
                    score = (score as i32 + adj).clamp(0, 100) as u32;
                    factors.push(SeverityFactor {
                        name: modifier.name.clone(),
                        contribution: adj.unsigned_abs(),
                        description: format!("Adjustment: {:+}", adj),
                    });
                }
            }
        }

        // Small input bonus (easier to reproduce)
        if input_size < 100 {
            score = (score as i32 + 5).clamp(0, 100) as u32;
            factors.push(SeverityFactor {
                name: "Small Reproducer".to_string(),
                contribution: 5,
                description: format!("Input size: {} bytes", input_size),
            });
        }

        // Determine security relevance
        let security_relevant = matches!(
            class,
            CrashClass::Segfault
                | CrashClass::CbgrViolation
                | CrashClass::RuntimeCrash(RuntimeCrashKind::UseAfterFree)
                | CrashClass::RuntimeCrash(RuntimeCrashKind::DoubleFree)
                | CrashClass::RuntimeCrash(RuntimeCrashKind::BufferOverflow)
        );

        // Determine level and priority
        let (level, priority) = self.score_to_level_priority(score, security_relevant);

        SeverityAssessment {
            level,
            score,
            factors,
            priority,
            security_relevant,
        }
    }

    /// Get base score for a crash class
    fn base_score(&self, class: &CrashClass) -> u32 {
        let class_name = match class {
            CrashClass::Segfault => "Segfault",
            CrashClass::CbgrViolation => "CbgrViolation",
            CrashClass::RuntimeCrash(_) => "RuntimeCrash",
            CrashClass::TypeCheckerCrash => "TypeCheckerCrash",
            CrashClass::CodegenCrash => "CodegenCrash",
            CrashClass::CompilerPanic(_) => "CompilerPanic",
            CrashClass::SmtCrash => "SmtCrash",
            CrashClass::StackOverflow => "StackOverflow",
            CrashClass::OutOfMemory => "OutOfMemory",
            CrashClass::ParserCrash => "ParserCrash",
            CrashClass::LexerCrash => "LexerCrash",
            CrashClass::AssertionFailure => "AssertionFailure",
            CrashClass::Unknown => "Unknown",
        };

        *self.config.class_scores.get(class_name).unwrap_or(&50)
    }

    /// Convert score to level and priority
    fn score_to_level_priority(&self, score: u32, security: bool) -> (Severity, Priority) {
        let level = match score {
            80..=100 => Severity::Critical,
            60..=79 => Severity::High,
            40..=59 => Severity::Medium,
            _ => Severity::Low,
        };

        let priority = if security && score >= 60 {
            Priority::Immediate
        } else {
            match score {
                80..=100 => Priority::Immediate,
                60..=79 => Priority::High,
                40..=59 => Priority::Medium,
                _ => Priority::Low,
            }
        };

        (level, priority)
    }
}

// ============================================================================
// Regression Detection
// ============================================================================

/// Baseline for regression detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashBaseline {
    /// Version or commit hash
    pub version: String,
    /// Timestamp when baseline was created
    pub timestamp: u64,
    /// Known crash signatures
    pub signatures: HashSet<String>,
    /// Signature details
    pub details: HashMap<String, BaselineCrash>,
}

/// A crash in the baseline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineCrash {
    /// Crash class
    pub class: CrashClass,
    /// Top frame function
    pub top_function: String,
    /// First seen in this version
    pub introduced_in: Option<String>,
    /// Fixed in version (if any)
    pub fixed_in: Option<String>,
}

/// Regression detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReport {
    /// Current version
    pub current_version: String,
    /// Baseline version
    pub baseline_version: String,
    /// New crashes (not in baseline)
    pub new_crashes: Vec<RegressionCrash>,
    /// Fixed crashes (in baseline but not current)
    pub fixed_crashes: Vec<RegressionCrash>,
    /// Persistent crashes (in both)
    pub persistent_crashes: Vec<RegressionCrash>,
    /// Summary statistics
    pub summary: RegressionSummary,
}

/// A crash in the regression report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionCrash {
    /// Signature hash
    pub hash: String,
    /// Crash class
    pub class: CrashClass,
    /// Title
    pub title: String,
    /// Severity
    pub severity: Severity,
}

/// Regression summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionSummary {
    /// Total new crashes
    pub new_count: usize,
    /// Total fixed crashes
    pub fixed_count: usize,
    /// Total persistent crashes
    pub persistent_count: usize,
    /// Regression score (negative is good)
    pub regression_score: i32,
    /// Pass/fail based on thresholds
    pub passed: bool,
}

/// Regression detector
pub struct RegressionDetector {
    /// Configuration
    config: RegressionConfig,
}

/// Configuration for regression detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionConfig {
    /// Maximum allowed new crashes
    pub max_new_crashes: usize,
    /// Maximum allowed new critical crashes
    pub max_new_critical: usize,
    /// Fail on any new security-relevant crash
    pub fail_on_security: bool,
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self {
            max_new_crashes: 10,
            max_new_critical: 0,
            fail_on_security: true,
        }
    }
}

impl Default for RegressionDetector {
    fn default() -> Self {
        Self::new(RegressionConfig::default())
    }
}

impl RegressionDetector {
    /// Create a new regression detector
    pub fn new(config: RegressionConfig) -> Self {
        Self { config }
    }

    /// Compare current crashes against baseline
    pub fn compare(
        &self,
        current_version: &str,
        baseline: &CrashBaseline,
        current_triager: &CrashTriager,
        severity_classifier: &SeverityClassifier,
    ) -> RegressionReport {
        let current_signatures: HashSet<_> =
            current_triager.known_signatures.keys().cloned().collect();

        let baseline_signatures = &baseline.signatures;

        // Find new crashes
        let new_hashes: Vec<_> = current_signatures
            .difference(baseline_signatures)
            .cloned()
            .collect();

        // Find fixed crashes
        let fixed_hashes: Vec<_> = baseline_signatures
            .difference(&current_signatures)
            .cloned()
            .collect();

        // Find persistent crashes
        let persistent_hashes: Vec<_> = current_signatures
            .intersection(baseline_signatures)
            .cloned()
            .collect();

        // Build crash lists with details
        let new_crashes: Vec<_> = new_hashes
            .iter()
            .filter_map(|hash| {
                let info = current_triager.get_crash_info(hash)?;
                let assessment =
                    severity_classifier.assess(&info.signature.class, "", info.representative_size);
                Some(RegressionCrash {
                    hash: hash.clone(),
                    class: info.signature.class.clone(),
                    title: current_triager.generate_title(info),
                    severity: assessment.level,
                })
            })
            .collect();

        let fixed_crashes: Vec<_> = fixed_hashes
            .iter()
            .filter_map(|hash| {
                let detail = baseline.details.get(hash)?;
                Some(RegressionCrash {
                    hash: hash.clone(),
                    class: detail.class.clone(),
                    title: detail.top_function.clone(),
                    severity: Severity::Medium, // Historical, unknown
                })
            })
            .collect();

        let persistent_crashes: Vec<_> = persistent_hashes
            .iter()
            .filter_map(|hash| {
                let info = current_triager.get_crash_info(hash)?;
                let assessment =
                    severity_classifier.assess(&info.signature.class, "", info.representative_size);
                Some(RegressionCrash {
                    hash: hash.clone(),
                    class: info.signature.class.clone(),
                    title: current_triager.generate_title(info),
                    severity: assessment.level,
                })
            })
            .collect();

        // Calculate summary
        let new_critical = new_crashes
            .iter()
            .filter(|c| c.severity == Severity::Critical)
            .count();

        let regression_score = new_crashes.len() as i32 * 2 - fixed_crashes.len() as i32;

        let passed = new_crashes.len() <= self.config.max_new_crashes
            && new_critical <= self.config.max_new_critical
            && (!self.config.fail_on_security
                || !new_crashes.iter().any(|c| c.severity == Severity::Critical));

        let summary = RegressionSummary {
            new_count: new_crashes.len(),
            fixed_count: fixed_crashes.len(),
            persistent_count: persistent_crashes.len(),
            regression_score,
            passed,
        };

        RegressionReport {
            current_version: current_version.to_string(),
            baseline_version: baseline.version.clone(),
            new_crashes,
            fixed_crashes,
            persistent_crashes,
            summary,
        }
    }

    /// Create a baseline from current triager state
    pub fn create_baseline(&self, version: &str, triager: &CrashTriager) -> CrashBaseline {
        let signatures: HashSet<_> = triager.known_signatures.keys().cloned().collect();

        let details: HashMap<_, _> = triager
            .known_signatures
            .iter()
            .map(|(hash, info)| {
                let top_function = info
                    .signature
                    .top_frames
                    .first()
                    .map(|f| f.function.clone())
                    .unwrap_or_default();

                (
                    hash.clone(),
                    BaselineCrash {
                        class: info.signature.class.clone(),
                        top_function,
                        introduced_in: Some(version.to_string()),
                        fixed_in: None,
                    },
                )
            })
            .collect();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        CrashBaseline {
            version: version.to_string(),
            timestamp,
            signatures,
            details,
        }
    }
}

/// Format a regression report as markdown
impl RegressionReport {
    /// Convert to markdown
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!("# Regression Report\n\n"));
        md.push_str(&format!(
            "Comparing **{}** against baseline **{}**\n\n",
            self.current_version, self.baseline_version
        ));

        // Summary
        md.push_str("## Summary\n\n");
        md.push_str(&format!("| Metric | Count |\n"));
        md.push_str(&format!("|--------|-------|\n"));
        md.push_str(&format!("| New crashes | {} |\n", self.summary.new_count));
        md.push_str(&format!(
            "| Fixed crashes | {} |\n",
            self.summary.fixed_count
        ));
        md.push_str(&format!(
            "| Persistent crashes | {} |\n",
            self.summary.persistent_count
        ));
        md.push_str(&format!(
            "| Regression score | {:+} |\n",
            self.summary.regression_score
        ));
        md.push_str(&format!(
            "| **Result** | {} |\n\n",
            if self.summary.passed { "PASS" } else { "FAIL" }
        ));

        // New crashes
        if !self.new_crashes.is_empty() {
            md.push_str("## New Crashes\n\n");
            for crash in &self.new_crashes {
                md.push_str(&format!(
                    "- **[{:?}]** `{}` - {} ({})\n",
                    crash.severity,
                    crash.hash[..8].to_string(),
                    crash.title,
                    format!("{:?}", crash.class)
                ));
            }
            md.push('\n');
        }

        // Fixed crashes
        if !self.fixed_crashes.is_empty() {
            md.push_str("## Fixed Crashes\n\n");
            for crash in &self.fixed_crashes {
                md.push_str(&format!(
                    "- `{}` - {} ({})\n",
                    crash.hash[..8].to_string(),
                    crash.title,
                    format!("{:?}", crash.class)
                ));
            }
            md.push('\n');
        }

        md
    }
}

// ============================================================================
// Auto-categorization
// ============================================================================

/// Automatic bug categorization
pub struct AutoCategorizer {
    /// Category rules
    rules: Vec<CategoryRule>,
}

/// A rule for categorizing bugs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    /// Category name
    pub category: String,
    /// Patterns to match in error message
    pub error_patterns: Vec<String>,
    /// Patterns to match in stack trace
    pub stack_patterns: Vec<String>,
    /// Crash classes that match
    pub classes: Vec<CrashClass>,
    /// Priority of this rule (higher wins)
    pub priority: u32,
}

/// Bug category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugCategory {
    /// Category name
    pub name: String,
    /// Component (e.g., lexer, parser, type checker)
    pub component: String,
    /// Labels for issue tracking
    pub labels: Vec<String>,
    /// Assignee suggestion
    pub suggested_assignee: Option<String>,
}

impl Default for AutoCategorizer {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoCategorizer {
    /// Create a new auto-categorizer with default rules
    pub fn new() -> Self {
        let rules = vec![
            CategoryRule {
                category: "lexer".to_string(),
                error_patterns: vec!["lex".to_string(), "token".to_string()],
                stack_patterns: vec!["verum_lexer".to_string()],
                classes: vec![CrashClass::LexerCrash],
                priority: 10,
            },
            CategoryRule {
                category: "parser".to_string(),
                error_patterns: vec!["parse".to_string(), "syntax".to_string()],
                stack_patterns: vec!["verum_parser".to_string()],
                classes: vec![CrashClass::ParserCrash],
                priority: 10,
            },
            CategoryRule {
                category: "type-system".to_string(),
                error_patterns: vec!["type".to_string(), "infer".to_string(), "unify".to_string()],
                stack_patterns: vec!["verum_types".to_string()],
                classes: vec![CrashClass::TypeCheckerCrash],
                priority: 10,
            },
            CategoryRule {
                category: "memory-safety".to_string(),
                error_patterns: vec!["cbgr".to_string(), "generation".to_string()],
                stack_patterns: vec!["verum_cbgr".to_string()],
                classes: vec![CrashClass::CbgrViolation],
                priority: 15,
            },
            CategoryRule {
                category: "verification".to_string(),
                error_patterns: vec!["smt".to_string(), "z3".to_string(), "verify".to_string()],
                stack_patterns: vec!["verum_smt".to_string()],
                classes: vec![CrashClass::SmtCrash],
                priority: 10,
            },
            CategoryRule {
                category: "codegen".to_string(),
                error_patterns: vec![
                    "llvm".to_string(),
                    "codegen".to_string(),
                    "inkwell".to_string(),
                ],
                stack_patterns: vec!["verum_codegen".to_string()],
                classes: vec![CrashClass::CodegenCrash],
                priority: 10,
            },
        ];

        Self { rules }
    }

    /// Categorize a crash
    pub fn categorize(&self, crash: &CrashSignature, error: &str, backtrace: &str) -> BugCategory {
        let error_lower = error.to_lowercase();
        let bt_lower = backtrace.to_lowercase();

        let mut best_match: Option<(&CategoryRule, u32)> = None;

        for rule in &self.rules {
            let mut score = 0;

            // Check crash class
            if rule.classes.contains(&crash.class) {
                score += rule.priority;
            }

            // Check error patterns
            for pattern in &rule.error_patterns {
                if error_lower.contains(&pattern.to_lowercase()) {
                    score += 5;
                }
            }

            // Check stack patterns
            for pattern in &rule.stack_patterns {
                if bt_lower.contains(&pattern.to_lowercase()) {
                    score += 10;
                }
            }

            if score > 0 {
                if let Some((_, best_score)) = best_match {
                    if score > best_score {
                        best_match = Some((rule, score));
                    }
                } else {
                    best_match = Some((rule, score));
                }
            }
        }

        if let Some((rule, _)) = best_match {
            BugCategory {
                name: rule.category.clone(),
                component: rule.category.clone(),
                labels: vec!["bug".to_string(), format!("component/{}", rule.category)],
                suggested_assignee: None,
            }
        } else {
            BugCategory {
                name: "unknown".to_string(),
                component: "unknown".to_string(),
                labels: vec!["bug".to_string(), "needs-triage".to_string()],
                suggested_assignee: None,
            }
        }
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    #[test]
    fn test_crash_deduplicator() {
        let mut dedup = CrashDeduplicator::default();

        let sig1 = CrashSignature::from_crash(
            "panicked at 'index out of bounds'",
            "0: some_function at file.rs:10",
        );

        let sig2 = CrashSignature::from_crash(
            "panicked at 'index out of bounds'",
            "0: some_function at file.rs:10",
        );

        assert!(dedup.process("crash1", sig1.clone(), "input1"));
        assert!(!dedup.process("crash2", sig2.clone(), "input2"));

        assert_eq!(dedup.stats().unique, 1);
        assert_eq!(dedup.stats().exact_duplicates, 1);
    }

    #[test]
    fn test_severity_classifier() {
        let classifier = SeverityClassifier::default();

        // Critical: Segfault
        let assessment = classifier.assess(&CrashClass::Segfault, "", 50);
        assert_eq!(assessment.level, Severity::Critical);
        assert!(assessment.security_relevant);

        // Low: Lexer crash
        let assessment = classifier.assess(&CrashClass::LexerCrash, "", 500);
        assert_eq!(assessment.level, Severity::Low);
        assert!(!assessment.security_relevant);
    }

    #[test]
    fn test_regression_detector() {
        let mut triager = CrashTriager::new();
        let detector = RegressionDetector::default();
        let classifier = SeverityClassifier::default();

        // Add some crashes
        triager.triage(
            "crash1",
            "input1",
            "panicked at 'oops'",
            "0: verum_parser::parse",
        );

        // Create baseline
        let baseline = detector.create_baseline("v1.0.0", &triager);
        assert_eq!(baseline.signatures.len(), 1);

        // Add new crash
        triager.triage(
            "crash2",
            "input2",
            "panicked at 'different'",
            "0: verum_lexer::lex",
        );

        // Compare
        let report = detector.compare("v1.1.0", &baseline, &triager, &classifier);
        assert_eq!(report.summary.new_count, 1);
        assert_eq!(report.summary.persistent_count, 1);
    }

    #[test]
    fn test_auto_categorizer() {
        let categorizer = AutoCategorizer::new();

        let sig = CrashSignature::from_crash(
            "panicked at 'type mismatch'",
            "0: verum_types::infer at types.rs:100",
        );

        let category = categorizer.categorize(&sig, "type mismatch", "verum_types::infer");
        assert_eq!(category.component, "type-system");
    }

    #[test]
    fn test_regression_report_markdown() {
        let report = RegressionReport {
            current_version: "v1.1.0".to_string(),
            baseline_version: "v1.0.0".to_string(),
            new_crashes: vec![RegressionCrash {
                hash: "abc12345def67890".to_string(),
                class: CrashClass::ParserCrash,
                title: "Parser crash in parse_expr".to_string(),
                severity: Severity::Medium,
            }],
            fixed_crashes: vec![],
            persistent_crashes: vec![],
            summary: RegressionSummary {
                new_count: 1,
                fixed_count: 0,
                persistent_count: 0,
                regression_score: 2,
                passed: true,
            },
        };

        let md = report.to_markdown();
        assert!(md.contains("# Regression Report"));
        assert!(md.contains("New Crashes"));
        assert!(md.contains("abc12345"));
    }
}
