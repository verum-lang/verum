//! REPL (Read-Eval-Print-Loop) Integration for JIT.
//!
//! Provides interactive execution environment for Verum code with:
//!
//! - Session state management
//! - Expression evaluation and variable binding
//! - Persistent scope across evaluations
//! - History tracking
//! - Auto-completion hooks
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           REPL Execution Flow                                │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//!   User Input (expression/statement)
//!         │
//!         ▼
//! ┌─────────────────┐
//! │  Parser         │  Parse input
//! │  (incremental)  │
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐    ┌─────────────────┐
//! │  Type Check     │───▶│   Session Ctx   │  Resolve bindings
//! │  (w/ context)   │    │   (variables)   │
//! └────────┬────────┘    └─────────────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  Codegen        │  AST → MLIR → LLVM
//! │  (JIT compile)  │
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐    ┌─────────────────┐
//! │  Execute        │───▶│  Update Session │  Store new bindings
//! │  (JIT run)      │    │  State          │
//! └────────┬────────┘    └─────────────────┘
//!          │
//!          ▼
//!   Result (display)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{ReplSession, ReplConfig};
//!
//! let mut session = ReplSession::new(ReplConfig::default())?;
//!
//! // Evaluate expressions
//! let result = session.eval("let x = 42")?;
//! let result = session.eval("x + 1")?;
//! println!("Result: {:?}", result); // 43
//!
//! // Check session state
//! println!("Bindings: {:?}", session.bindings());
//! ```

use crate::mlir::error::{MlirError, Result};
use crate::mlir::jit::engine::{JitConfig, JitEngine, JitCompiler};
use crate::mlir::jit::incremental::{IncrementalCache, CacheConfig};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use verum_common::Text;

// ============================================================================
// REPL Configuration
// ============================================================================

/// Configuration for REPL session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplConfig {
    /// JIT configuration.
    pub jit_config: JitConfig,

    /// Enable incremental compilation.
    pub incremental: bool,

    /// Maximum history entries.
    pub max_history: usize,

    /// Enable auto-completion.
    pub auto_complete: bool,

    /// Show timing information.
    pub show_timing: bool,

    /// Pretty-print results.
    pub pretty_print: bool,

    /// Maximum expression result size to display.
    pub max_display_size: usize,

    /// Enable verbose output.
    pub verbose: bool,

    /// Session timeout in seconds (0 = no timeout).
    pub timeout_seconds: u64,
}

impl ReplConfig {
    /// Create new REPL configuration.
    pub fn new() -> Self {
        Self {
            jit_config: JitConfig::development(),
            incremental: true,
            max_history: 1000,
            auto_complete: true,
            show_timing: false,
            pretty_print: true,
            max_display_size: 4096,
            verbose: false,
            timeout_seconds: 0,
        }
    }

    /// Builder: set JIT config.
    pub fn jit_config(mut self, config: JitConfig) -> Self {
        self.jit_config = config;
        self
    }

    /// Builder: enable/disable incremental compilation.
    pub fn incremental(mut self, enabled: bool) -> Self {
        self.incremental = enabled;
        self
    }

    /// Builder: set max history.
    pub fn max_history(mut self, max: usize) -> Self {
        self.max_history = max;
        self
    }

    /// Builder: enable/disable auto-completion.
    pub fn auto_complete(mut self, enabled: bool) -> Self {
        self.auto_complete = enabled;
        self
    }

    /// Builder: enable/disable timing display.
    pub fn show_timing(mut self, enabled: bool) -> Self {
        self.show_timing = enabled;
        self
    }

    /// Builder: set verbose mode.
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }

    /// Create development configuration.
    pub fn development() -> Self {
        Self::new()
            .show_timing(true)
            .verbose(true)
    }

    /// Create minimal configuration.
    pub fn minimal() -> Self {
        Self::new()
            .incremental(false)
            .auto_complete(false)
            .max_history(100)
    }
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Evaluation Result
// ============================================================================

/// Result of evaluating an expression in the REPL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    /// The evaluated value (as displayable string).
    pub value: Text,

    /// The type of the result.
    pub type_name: Text,

    /// Whether the result is a unit type.
    pub is_unit: bool,

    /// Compilation time in microseconds.
    pub compile_time_us: u64,

    /// Execution time in microseconds.
    pub exec_time_us: u64,

    /// New bindings created by this evaluation.
    pub new_bindings: Vec<Text>,
}

impl EvalResult {
    /// Create a unit result.
    pub fn unit() -> Self {
        Self {
            value: Text::from("()"),
            type_name: Text::from("Unit"),
            is_unit: true,
            compile_time_us: 0,
            exec_time_us: 0,
            new_bindings: vec![],
        }
    }

    /// Create a result with value.
    pub fn with_value(value: impl Into<Text>, type_name: impl Into<Text>) -> Self {
        Self {
            value: value.into(),
            type_name: type_name.into(),
            is_unit: false,
            compile_time_us: 0,
            exec_time_us: 0,
            new_bindings: vec![],
        }
    }

    /// Set timing information.
    pub fn with_timing(mut self, compile_us: u64, exec_us: u64) -> Self {
        self.compile_time_us = compile_us;
        self.exec_time_us = exec_us;
        self
    }

    /// Set new bindings.
    pub fn with_bindings(mut self, bindings: Vec<Text>) -> Self {
        self.new_bindings = bindings;
        self
    }

    /// Format result for display.
    pub fn display(&self, config: &ReplConfig) -> String {
        let mut output = String::new();

        if !self.is_unit {
            if config.pretty_print {
                output.push_str(&format!("{}: {}", self.value, self.type_name));
            } else {
                output.push_str(self.value.as_str());
            }
        }

        if config.show_timing {
            output.push_str(&format!(
                "\n[compile: {}µs, exec: {}µs]",
                self.compile_time_us, self.exec_time_us
            ));
        }

        if !self.new_bindings.is_empty() {
            output.push_str(&format!("\nNew bindings: {:?}", self.new_bindings));
        }

        output
    }
}

// ============================================================================
// Session Binding
// ============================================================================

/// A binding in the REPL session.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Variable name.
    pub name: Text,

    /// Type name.
    pub type_name: Text,

    /// Value (as raw bytes for storage).
    pub value: Vec<u8>,

    /// Whether this is mutable.
    pub mutable: bool,

    /// Evaluation number when created.
    pub created_at: u64,

    /// Address in JIT memory (for reference types).
    pub address: Option<*mut ()>,
}

// SAFETY: Binding contains raw pointers that point to JIT-managed memory
unsafe impl Send for Binding {}
unsafe impl Sync for Binding {}

impl Binding {
    /// Create a new binding.
    pub fn new(
        name: impl Into<Text>,
        type_name: impl Into<Text>,
        value: Vec<u8>,
        mutable: bool,
        eval_num: u64,
    ) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            value,
            mutable,
            created_at: eval_num,
            address: None,
        }
    }

    /// Create binding with address.
    pub fn with_address(mut self, addr: *mut ()) -> Self {
        self.address = Some(addr);
        self
    }
}

// ============================================================================
// History Entry
// ============================================================================

/// A history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Entry number.
    pub number: u64,

    /// Input text.
    pub input: Text,

    /// Result (if successful).
    pub result: Option<EvalResult>,

    /// Error message (if failed).
    pub error: Option<Text>,

    /// Timestamp.
    pub timestamp: u64,
}

// ============================================================================
// Session Statistics
// ============================================================================

/// Statistics for a REPL session.
#[derive(Debug, Default)]
pub struct SessionStats {
    /// Number of evaluations.
    pub evaluations: AtomicU64,

    /// Number of successful evaluations.
    pub successes: AtomicU64,

    /// Number of failed evaluations.
    pub failures: AtomicU64,

    /// Total compilation time.
    pub total_compile_time_us: AtomicU64,

    /// Total execution time.
    pub total_exec_time_us: AtomicU64,

    /// Number of bindings created.
    pub bindings_created: AtomicU64,

    /// Cache hits (for incremental).
    pub cache_hits: AtomicU64,
}

impl SessionStats {
    /// Create new statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get success rate.
    pub fn success_rate(&self) -> f64 {
        let total = self.evaluations.load(Ordering::Relaxed);
        if total == 0 {
            1.0
        } else {
            self.successes.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Get average compilation time.
    pub fn avg_compile_time_us(&self) -> f64 {
        let count = self.evaluations.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            self.total_compile_time_us.load(Ordering::Relaxed) as f64 / count as f64
        }
    }

    /// Get summary.
    pub fn summary(&self) -> SessionStatsSummary {
        SessionStatsSummary {
            evaluations: self.evaluations.load(Ordering::Relaxed),
            success_rate: self.success_rate(),
            avg_compile_time_us: self.avg_compile_time_us(),
            bindings: self.bindings_created.load(Ordering::Relaxed),
        }
    }
}

/// Summary of session statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatsSummary {
    pub evaluations: u64,
    pub success_rate: f64,
    pub avg_compile_time_us: f64,
    pub bindings: u64,
}

// ============================================================================
// REPL Session
// ============================================================================

/// Unique session identifier.
pub type SessionId = uuid::Uuid;

/// REPL session for interactive code execution.
pub struct ReplSession {
    /// Unique session ID.
    id: SessionId,

    /// Configuration.
    config: ReplConfig,

    /// Current bindings.
    bindings: DashMap<Text, Binding>,

    /// History entries.
    history: RwLock<Vec<HistoryEntry>>,

    /// Evaluation counter.
    eval_count: AtomicU64,

    /// Statistics.
    stats: Arc<SessionStats>,

    /// Incremental cache (if enabled).
    cache: Option<IncrementalCache>,

    /// Session creation time.
    created_at: instant::Instant,
}

impl ReplSession {
    /// Create a new REPL session.
    pub fn new(config: ReplConfig) -> Result<Self> {
        let cache = if config.incremental {
            Some(IncrementalCache::new(CacheConfig::memory_only())?)
        } else {
            None
        };

        Ok(Self {
            id: uuid::Uuid::new_v4(),
            config,
            bindings: DashMap::new(),
            history: RwLock::new(Vec::new()),
            eval_count: AtomicU64::new(0),
            stats: Arc::new(SessionStats::new()),
            cache,
            created_at: instant::Instant::now(),
        })
    }

    /// Get session ID.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Get configuration.
    pub fn config(&self) -> &ReplConfig {
        &self.config
    }

    /// Get statistics.
    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }

    /// Get evaluation count.
    pub fn eval_count(&self) -> u64 {
        self.eval_count.load(Ordering::Relaxed)
    }

    /// Evaluate source code.
    ///
    /// This is a placeholder that would integrate with the parser and codegen.
    /// In a full implementation, this would:
    /// 1. Parse the input
    /// 2. Type-check with session context
    /// 3. Compile to MLIR
    /// 4. Execute via JIT
    /// 5. Update session state
    pub fn eval(&self, input: &str) -> Result<EvalResult> {
        let eval_num = self.eval_count.fetch_add(1, Ordering::Relaxed);
        let start = instant::Instant::now();

        self.stats.evaluations.fetch_add(1, Ordering::Relaxed);

        // Record in history
        let history_entry = HistoryEntry {
            number: eval_num,
            input: Text::from(input),
            result: None,
            error: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Placeholder implementation
        // In a real implementation, this would:
        // 1. Parse input
        // 2. Check for declarations (let, fn)
        // 3. Compile and execute
        // 4. Update bindings

        let result = self.eval_internal(input, eval_num)?;

        // Update history with result
        {
            let mut history = self.history.write();
            let mut entry = history_entry;
            entry.result = Some(result.clone());

            if history.len() >= self.config.max_history {
                history.remove(0);
            }
            history.push(entry);
        }

        self.stats.successes.fetch_add(1, Ordering::Relaxed);
        self.stats
            .total_compile_time_us
            .fetch_add(result.compile_time_us, Ordering::Relaxed);
        self.stats
            .total_exec_time_us
            .fetch_add(result.exec_time_us, Ordering::Relaxed);

        Ok(result)
    }

    /// Internal evaluation (placeholder).
    fn eval_internal(&self, input: &str, eval_num: u64) -> Result<EvalResult> {
        let compile_start = instant::Instant::now();

        // Detect declaration vs expression
        let trimmed = input.trim();

        if trimmed.starts_with("let ") {
            // Handle let binding
            self.handle_let(trimmed, eval_num)
        } else if trimmed.starts_with("fn ") {
            // Handle function definition
            self.handle_fn(trimmed, eval_num)
        } else {
            // Handle expression
            self.handle_expr(trimmed, eval_num)
        }
    }

    /// Handle let binding (placeholder).
    fn handle_let(&self, input: &str, eval_num: u64) -> Result<EvalResult> {
        // Parse: let <name> = <expr>
        // This is a simplified placeholder
        let parts: Vec<&str> = input.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(MlirError::ReplError {
                message: Text::from("Invalid let syntax"),
            });
        }

        let name_part = parts[0].trim().strip_prefix("let").unwrap().trim();
        let name = Text::from(name_part.split(':').next().unwrap().trim());

        // Create binding (placeholder - would need actual evaluation)
        let binding = Binding::new(
            name.clone(),
            "i64", // Would be inferred
            vec![],
            false,
            eval_num,
        );

        self.bindings.insert(name.clone(), binding);
        self.stats.bindings_created.fetch_add(1, Ordering::Relaxed);

        Ok(EvalResult::unit().with_bindings(vec![name]))
    }

    /// Handle function definition (placeholder).
    fn handle_fn(&self, input: &str, eval_num: u64) -> Result<EvalResult> {
        // Parse: fn <name>(...) { ... }
        // This is a simplified placeholder
        let name_start = input.find(char::is_alphabetic).unwrap_or(3);
        let name_end = input[name_start..].find(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + name_start)
            .unwrap_or(input.len());

        let name = Text::from(&input[name_start..name_end]);

        // Create function binding
        let binding = Binding::new(
            name.clone(),
            "Fn",
            vec![],
            false,
            eval_num,
        );

        self.bindings.insert(name.clone(), binding);
        self.stats.bindings_created.fetch_add(1, Ordering::Relaxed);

        Ok(EvalResult::unit().with_bindings(vec![name]))
    }

    /// Handle expression (placeholder).
    fn handle_expr(&self, input: &str, _eval_num: u64) -> Result<EvalResult> {
        // Placeholder - would compile and execute expression
        let compile_time = 100; // Simulated
        let exec_time = 50; // Simulated

        // For now, return a placeholder result
        Ok(EvalResult::with_value(
            format!("<result of: {}>", input),
            "Unknown",
        ).with_timing(compile_time, exec_time))
    }

    /// Get a binding by name.
    pub fn get_binding(&self, name: &str) -> Option<Binding> {
        self.bindings.get(&Text::from(name)).map(|r| r.clone())
    }

    /// Get all binding names.
    pub fn binding_names(&self) -> Vec<Text> {
        self.bindings.iter().map(|r| r.key().clone()).collect()
    }

    /// Get all bindings.
    pub fn all_bindings(&self) -> Vec<Binding> {
        self.bindings.iter().map(|r| r.value().clone()).collect()
    }

    /// Remove a binding.
    pub fn remove_binding(&self, name: &str) -> Option<Binding> {
        self.bindings.remove(&Text::from(name)).map(|(_, v)| v)
    }

    /// Clear all bindings.
    pub fn clear_bindings(&self) {
        self.bindings.clear();
    }

    /// Get history.
    pub fn history(&self) -> Vec<HistoryEntry> {
        self.history.read().clone()
    }

    /// Get last N history entries.
    pub fn recent_history(&self, n: usize) -> Vec<HistoryEntry> {
        let history = self.history.read();
        let start = history.len().saturating_sub(n);
        history[start..].to_vec()
    }

    /// Get history entry by number.
    pub fn get_history(&self, number: u64) -> Option<HistoryEntry> {
        self.history
            .read()
            .iter()
            .find(|e| e.number == number)
            .cloned()
    }

    /// Clear history.
    pub fn clear_history(&self) {
        self.history.write().clear();
    }

    /// Get completions for input (auto-complete).
    pub fn completions(&self, prefix: &str) -> Vec<Text> {
        if !self.config.auto_complete {
            return vec![];
        }

        let mut completions = Vec::new();

        // Complete from bindings
        for binding in self.bindings.iter() {
            if binding.name.as_str().starts_with(prefix) {
                completions.push(binding.name.clone());
            }
        }

        // Complete from keywords (Verum keywords)
        let keywords = [
            "let", "fn", "if", "else", "match", "for", "while", "loop",
            "return", "break", "continue", "type", "implement", "using",
            "context", "provide", "async", "await", "spawn", "join",
        ];

        for kw in keywords {
            if kw.starts_with(prefix) {
                completions.push(Text::from(kw));
            }
        }

        completions.sort();
        completions.dedup();
        completions
    }

    /// Reset the session.
    pub fn reset(&self) -> Result<()> {
        self.bindings.clear();
        self.history.write().clear();
        self.eval_count.store(0, Ordering::Relaxed);

        if let Some(ref cache) = self.cache {
            cache.clear()?;
        }

        Ok(())
    }

    /// Get session uptime.
    pub fn uptime(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Serialize session state.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let state = SessionState {
            id: self.id,
            eval_count: self.eval_count.load(Ordering::Relaxed),
            history: self.history.read().clone(),
            // Note: bindings are not fully serializable due to raw pointers
        };

        serde_json::to_vec(&state).map_err(|e| MlirError::ReplError {
            message: Text::from(format!("Serialization failed: {}", e)),
        })
    }
}

/// Serializable session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionState {
    id: SessionId,
    eval_count: u64,
    history: Vec<HistoryEntry>,
}

// ============================================================================
// REPL Command
// ============================================================================

/// REPL meta-commands (prefixed with ':').
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// Show help.
    Help,
    /// Clear screen.
    Clear,
    /// Show bindings.
    Bindings,
    /// Show history.
    History,
    /// Load file.
    Load(String),
    /// Save session.
    Save(String),
    /// Reset session.
    Reset,
    /// Quit REPL.
    Quit,
    /// Show type of expression.
    Type(String),
    /// Show statistics.
    Stats,
    /// Set option.
    Set(String, String),
    /// Unknown command.
    Unknown(String),
}

impl ReplCommand {
    /// Parse a command from input.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if !input.starts_with(':') {
            return None;
        }

        let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim().to_string());

        Some(match cmd.as_str() {
            "help" | "h" | "?" => ReplCommand::Help,
            "clear" | "cls" => ReplCommand::Clear,
            "bindings" | "b" => ReplCommand::Bindings,
            "history" | "hist" => ReplCommand::History,
            "load" | "l" => ReplCommand::Load(arg.unwrap_or_default()),
            "save" | "s" => ReplCommand::Save(arg.unwrap_or_default()),
            "reset" => ReplCommand::Reset,
            "quit" | "q" | "exit" => ReplCommand::Quit,
            "type" | "t" => ReplCommand::Type(arg.unwrap_or_default()),
            "stats" => ReplCommand::Stats,
            "set" => {
                let arg = arg.unwrap_or_default();
                let parts: Vec<&str> = arg.splitn(2, '=').collect();
                if parts.len() == 2 {
                    ReplCommand::Set(parts[0].trim().to_string(), parts[1].trim().to_string())
                } else {
                    ReplCommand::Unknown(input.to_string())
                }
            }
            _ => ReplCommand::Unknown(input.to_string()),
        })
    }

    /// Get help text for commands.
    pub fn help_text() -> &'static str {
        r#"REPL Commands:
  :help, :h, :?      Show this help
  :clear, :cls       Clear screen
  :bindings, :b      Show all bindings
  :history, :hist    Show history
  :load <file>       Load file
  :save <file>       Save session
  :reset             Reset session
  :quit, :q, :exit   Exit REPL
  :type <expr>       Show type of expression
  :stats             Show statistics
  :set <key>=<val>   Set option"#
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repl_config_default() {
        let config = ReplConfig::default();
        assert!(config.incremental);
        assert!(config.auto_complete);
        assert!(config.max_history > 0);
    }

    #[test]
    fn test_eval_result() {
        let result = EvalResult::with_value("42", "i64")
            .with_timing(100, 50);

        assert_eq!(result.value.as_str(), "42");
        assert_eq!(result.type_name.as_str(), "i64");
        assert!(!result.is_unit);
        assert_eq!(result.compile_time_us, 100);
        assert_eq!(result.exec_time_us, 50);
    }

    #[test]
    fn test_repl_session_create() -> Result<()> {
        let session = ReplSession::new(ReplConfig::default())?;
        assert_eq!(session.eval_count(), 0);
        assert!(session.binding_names().is_empty());
        Ok(())
    }

    #[test]
    fn test_repl_command_parse() {
        assert_eq!(ReplCommand::parse(":help"), Some(ReplCommand::Help));
        assert_eq!(ReplCommand::parse(":quit"), Some(ReplCommand::Quit));
        assert_eq!(ReplCommand::parse(":q"), Some(ReplCommand::Quit));
        assert_eq!(
            ReplCommand::parse(":load test.vr"),
            Some(ReplCommand::Load("test.vr".to_string()))
        );
        assert!(ReplCommand::parse("let x = 1").is_none());
    }

    #[test]
    fn test_completions() -> Result<()> {
        let session = ReplSession::new(ReplConfig::default())?;

        // Add some bindings
        session.bindings.insert(
            Text::from("my_var"),
            Binding::new("my_var", "i64", vec![], false, 0),
        );
        session.bindings.insert(
            Text::from("my_func"),
            Binding::new("my_func", "Fn", vec![], false, 1),
        );

        let completions = session.completions("my_");
        assert!(completions.contains(&Text::from("my_var")));
        assert!(completions.contains(&Text::from("my_func")));

        let kw_completions = session.completions("le");
        assert!(kw_completions.contains(&Text::from("let")));

        Ok(())
    }

    #[test]
    fn test_session_stats() {
        let stats = SessionStats::new();

        stats.evaluations.fetch_add(10, Ordering::Relaxed);
        stats.successes.fetch_add(9, Ordering::Relaxed);
        stats.failures.fetch_add(1, Ordering::Relaxed);

        assert_eq!(stats.success_rate(), 0.9);
    }
}
