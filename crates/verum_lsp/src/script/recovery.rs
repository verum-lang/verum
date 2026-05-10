//! Error recovery strategies optimized for script/REPL environments
//!

//! Script contexts have different error recovery needs than batch compilation:
//! - More lenient on incomplete input (user is still typing)
//! - Better suggestions for common REPL mistakes
//! - Context-aware completions
//! - Graceful handling of partial expressions
//!

//! # Recovery Strategies
//!

//! 1. **Incomplete Input Detection**: Recognize when user needs to continue typing
//! 2. **Missing Semicolon Recovery**: REPL often doesn't require semicolons
//! 3. **Unbalanced Delimiter Recovery**: Track and suggest closing braces/brackets
//! 4. **Identifier Typo Correction**: Suggest similar names from context
//! 5. **Type Annotation Inference**: Suggest types when inference fails
//!

//! Moved from verum_parser::script_recovery

use verum_ast::Expr;
use verum_common::{List, Text};
use verum_lexer::TokenKind;

use super::context::ScriptContext;
use verum_parser::ParseError;
use verum_parser::error::ParseErrorKind;

/// Result of attempting error recovery in script mode
#[derive(Debug, Clone)]
pub enum RecoveryResult {
    /// Recovery succeeded with a suggestion
    Recovered {
        suggestion: Text,
        recovered_node: Option<Expr>,
    },
    /// Input is incomplete, user should continue
    Incomplete { expected: List<Text> },
    /// Error cannot be recovered, but here's a helpful message
    Failed { message: Text, help: Text },
}

/// Discriminator for [`RecoveryResult`] — zero-sized projection
/// classifying the three recovery outcomes for script-mode
/// parse errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecoveryResultKind {
    Recovered,
    Incomplete,
    Failed,
}

/// Per-variant projection for [`RecoveryResultKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryResultKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// Recovery succeeded (Recovered singleton).  The IDE
    /// should apply the suggestion (or surface it as a
    /// quick-fix).
    pub is_success: bool,
    /// The input is *partial* (Incomplete singleton).  The
    /// IDE should keep the buffer open and prompt the user
    /// to continue typing — pinned because the REPL/script
    /// surfaces hold off emitting an error here.
    pub is_partial: bool,
    /// Recovery *gave up* (Failed singleton).  The IDE
    /// should surface the message + help text as a hard
    /// diagnostic.
    pub is_terminal_failure: bool,
    /// The variant carries an actionable *recovered_node*
    /// payload — Recovered singleton.  IDE applies the node
    /// directly when present.
    pub carries_recovered_node: bool,
    /// The variant carries an *expectations list* —
    /// Incomplete singleton.  IDE renders the expected-token
    /// hints in the completion popup.
    pub carries_expectations: bool,
}

impl RecoveryResultKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] =
        &[Self::Recovered, Self::Incomplete, Self::Failed];

    /// Static fact-pack.
    pub const fn meta(self) -> RecoveryResultKindMeta {
        match self {
            RecoveryResultKind::Recovered => RecoveryResultKindMeta {
                name: "recovered",
                is_success: true,
                is_partial: false,
                is_terminal_failure: false,
                carries_recovered_node: true,
                carries_expectations: false,
            },
            RecoveryResultKind::Incomplete => RecoveryResultKindMeta {
                name: "incomplete",
                is_success: false,
                is_partial: true,
                is_terminal_failure: false,
                carries_recovered_node: false,
                carries_expectations: true,
            },
            RecoveryResultKind::Failed => RecoveryResultKindMeta {
                name: "failed",
                is_success: false,
                is_partial: false,
                is_terminal_failure: true,
                carries_recovered_node: false,
                carries_expectations: false,
            },
        }
    }
}

impl RecoveryResult {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> RecoveryResultKind {
        match self {
            RecoveryResult::Recovered { .. } => RecoveryResultKind::Recovered,
            RecoveryResult::Incomplete { .. } => RecoveryResultKind::Incomplete,
            RecoveryResult::Failed { .. } => RecoveryResultKind::Failed,
        }
    }
}

/// Script-specific error recovery engine
pub struct ScriptRecovery {
    /// Threshold for fuzzy matching (0.0-1.0)
    similarity_threshold: f64,
}

impl ScriptRecovery {
    /// Create a new script recovery engine
    pub fn new() -> Self {
        Self {
            similarity_threshold: 0.75,
        }
    }

    /// Attempt to recover from a parse error in script context
    pub fn recover(&self, error: &ParseError, context: &ScriptContext) -> RecoveryResult {
        match &error.kind {
            ParseErrorKind::UnexpectedToken { expected, found } => {
                self.recover_unexpected_token(expected, found, context)
            }
            ParseErrorKind::UnexpectedEof { expected } => {
                self.recover_unexpected_eof(expected, context)
            }
            ParseErrorKind::InvalidSyntax { message } => {
                self.recover_invalid_syntax(message, context)
            }
            _ => RecoveryResult::Failed {
                message: Text::from(format!("{}", error)),
                help: Text::from("Check syntax and try again"),
            },
        }
    }

    /// Recover from unexpected token errors
    fn recover_unexpected_token(
        &self,
        expected: &List<TokenKind>,
        found: &TokenKind,
        context: &ScriptContext,
    ) -> RecoveryResult {
        // Check if we're missing a closing delimiter
        if !context.is_complete() {
            let missing = self.suggest_missing_delimiters(context);
            return RecoveryResult::Incomplete { expected: missing };
        }

        // Check for common REPL mistakes
        if expected.contains(&TokenKind::Semicolon) {
            return RecoveryResult::Recovered {
                suggestion: Text::from("In REPL, semicolons are optional for expressions"),
                recovered_node: None,
            };
        }

        // Check for identifier typos
        if let TokenKind::Ident(found_name) = found
            && let Some(suggestion) = self.suggest_similar_identifier(found_name, context)
        {
            return RecoveryResult::Recovered {
                suggestion: Text::from(format!("Did you mean '{}'?", suggestion)),
                recovered_node: None,
            };
        }

        // General case
        let expected_str = expected
            .iter()
            .map(|k| format!("{:?}", k))
            .collect::<Vec<_>>()
            .join(", ");

        RecoveryResult::Failed {
            message: Text::from(format!("Expected {}, found {:?}", expected_str, found)),
            help: Text::from("Check your syntax"),
        }
    }

    /// Recover from unexpected EOF
    fn recover_unexpected_eof(
        &self,
        expected: &List<TokenKind>,
        _context: &ScriptContext,
    ) -> RecoveryResult {
        // Definitely incomplete input
        let expected_tokens: List<Text> = expected
            .iter()
            .map(|k| Text::from(format!("{:?}", k)))
            .collect();

        RecoveryResult::Incomplete {
            expected: expected_tokens,
        }
    }

    /// Recover from general syntax errors
    fn recover_invalid_syntax(&self, message: &str, context: &ScriptContext) -> RecoveryResult {
        // Check for common patterns
        if message.contains("not found in scope") || message.contains("cannot find") {
            // Extract the identifier that wasn't found
            if let Some(name) = self.extract_identifier_from_message(message)
                && let Some(suggestion) = self.suggest_similar_identifier(&name, context)
            {
                return RecoveryResult::Recovered {
                    suggestion: Text::from(format!(
                        "Name '{}' not found. Did you mean '{}'?",
                        name, suggestion
                    )),
                    recovered_node: None,
                };
            }
        }

        RecoveryResult::Failed {
            message: Text::from(message),
            help: Text::from("Use :help for REPL commands"),
        }
    }

    /// Suggest missing closing delimiters
    pub fn suggest_missing_delimiters(&self, context: &ScriptContext) -> List<Text> {
        let mut suggestions = List::new();

        if context.get_brace_depth() > 0 {
            suggestions.push(Text::from(format!(
                "Missing {} closing brace(s) '{}'",
                context.get_brace_depth(),
                "}"
            )));
        }

        if context.get_bracket_depth() > 0 {
            suggestions.push(Text::from(format!(
                "Missing {} closing bracket(s) ']'",
                context.get_bracket_depth()
            )));
        }

        if context.get_paren_depth() > 0 {
            suggestions.push(Text::from(format!(
                "Missing {} closing paren(s) ')'",
                context.get_paren_depth()
            )));
        }

        if context.is_in_string() {
            suggestions.push(Text::from("Unclosed string literal"));
        }

        if context.is_in_comment() {
            suggestions.push(Text::from("Unclosed block comment"));
        }

        suggestions
    }

    /// Find similar identifiers in the context using fuzzy matching
    fn suggest_similar_identifier(&self, name: &str, context: &ScriptContext) -> Option<String> {
        let mut best_match: Option<(String, f64)> = None;

        for binding in context.bindings.keys() {
            let similarity = self.calculate_similarity(name, binding.as_str());
            if similarity >= self.similarity_threshold {
                match best_match {
                    None => best_match = Some((binding.as_str().to_string(), similarity)),
                    Some((_, best_sim)) => {
                        if similarity > best_sim {
                            best_match = Some((binding.as_str().to_string(), similarity));
                        }
                    }
                }
            }
        }

        best_match.map(|(name, _)| name)
    }

    /// Calculate similarity between two strings (Levenshtein-based)
    fn calculate_similarity(&self, a: &str, b: &str) -> f64 {
        let distance = self.levenshtein_distance(a, b);
        let max_len = a.len().max(b.len());
        if max_len == 0 {
            1.0
        } else {
            1.0 - (distance as f64 / max_len as f64)
        }
    }

    /// Levenshtein distance between two strings
    fn levenshtein_distance(&self, a: &str, b: &str) -> usize {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let a_len = a_chars.len();
        let b_len = b_chars.len();

        let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

        for (i, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
            row[0] = i;
        }
        for (j, val) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
            *val = j;
        }

        for i in 1..=a_len {
            for j in 1..=b_len {
                let cost = if a_chars[i - 1] == b_chars[j - 1] {
                    0
                } else {
                    1
                };
                matrix[i][j] = (matrix[i - 1][j] + 1)
                    .min(matrix[i][j - 1] + 1)
                    .min(matrix[i - 1][j - 1] + cost);
            }
        }

        matrix[a_len][b_len]
    }

    /// Extract identifier name from error message
    fn extract_identifier_from_message(&self, message: &str) -> Option<String> {
        // Simple heuristic: look for quoted identifiers
        if let Some(start) = message.find('`')
            && let Some(end) = message[start + 1..].find('`')
        {
            return Some(message[start + 1..start + 1 + end].to_string());
        }
        None
    }
}

impl Default for ScriptRecovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Suggest auto-completions for partial input
pub fn suggest_autocompletion(partial: &str, context: &ScriptContext) -> List<(Text, Text)> {
    let mut suggestions: List<(Text, Text)> = List::new();

    // Add bindings
    for (name, type_hint) in context.bindings.iter() {
        if name.as_str().starts_with(partial) {
            suggestions.push((name.clone(), type_hint.clone()));
        }
    }

    // Add keywords
    let keywords = [
        ("let", "variable binding"),
        ("fn", "function definition"),
        ("type", "type definition"),
        ("protocol", "protocol definition"),
        ("impl", "implementation"),
        ("match", "pattern matching"),
        ("if", "conditional"),
        ("else", "else branch"),
        ("for", "for loop"),
        ("while", "while loop"),
        ("loop", "infinite loop"),
        ("return", "return value"),
        ("break", "break loop"),
        ("continue", "continue loop"),
        ("async", "async function"),
        ("await", "await future"),
        ("using", "context usage"),
        ("provide", "context provision"),
        ("context", "context declaration"),
    ];

    for (kw, desc) in &keywords {
        if kw.starts_with(partial) {
            suggestions.push((Text::from(*kw), Text::from(*desc)));
        }
    }

    suggestions
}

/// Generate helpful error message for common REPL mistakes
pub fn explain_error(error: &ParseError, context: &ScriptContext) -> Text {
    let recovery = ScriptRecovery::new();
    match recovery.recover(error, context) {
        RecoveryResult::Recovered { suggestion, .. } => suggestion,
        RecoveryResult::Incomplete { expected } => {
            if expected.is_empty() {
                Text::from("Input is incomplete. Continue typing...")
            } else {
                Text::from(format!(
                    "Input is incomplete. Expected: {}",
                    expected
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
        RecoveryResult::Failed { message, help } => Text::from(format!("{}\n{}", message, help)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::{FileId, Span};

    /// Drift-pin: `RecoveryResultKind` discriminator
    /// projection.  Perfect partition over the three outcomes
    /// (success / partial / terminal-failure).
    #[test]
    fn meta_pin_recovery_result_kind_round_trip_and_partitions() {
        assert_eq!(RecoveryResultKind::ALL.len(), 3);

        // Perfect partition: every variant flips exactly one
        // of {is_success, is_partial, is_terminal_failure}.
        for k in RecoveryResultKind::ALL {
            let m = k.meta();
            let count = (m.is_success as u32)
                + (m.is_partial as u32)
                + (m.is_terminal_failure as u32);
            assert_eq!(count, 1, "{:?}: must flip exactly one outcome", k);
        }

        // carries_recovered_node = is_success (Recovered
        // singleton).
        for k in RecoveryResultKind::ALL {
            let m = k.meta();
            assert_eq!(m.carries_recovered_node, m.is_success);
        }

        // carries_expectations = is_partial (Incomplete
        // singleton).
        for k in RecoveryResultKind::ALL {
            let m = k.meta();
            assert_eq!(m.carries_expectations, m.is_partial);
        }

        // Live-payload kind() projection.
        let r = RecoveryResult::Recovered {
            suggestion: Text::from("did you mean foo?"),
            recovered_node: None,
        };
        assert_eq!(r.kind(), RecoveryResultKind::Recovered);

        let i = RecoveryResult::Incomplete {
            expected: List::from(vec![Text::from(")")]),
        };
        assert_eq!(i.kind(), RecoveryResultKind::Incomplete);

        let f = RecoveryResult::Failed {
            message: Text::from("unrecoverable"),
            help: Text::from("try X"),
        };
        assert_eq!(f.kind(), RecoveryResultKind::Failed);
    }

    /// Test file ID used for unit tests
    fn test_file_id() -> FileId {
        FileId::new(999)
    }

    #[test]
    fn test_levenshtein_distance() {
        let recovery = ScriptRecovery::new();

        assert_eq!(recovery.levenshtein_distance("cat", "cat"), 0);
        assert_eq!(recovery.levenshtein_distance("cat", "bat"), 1);
        assert_eq!(recovery.levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_similarity_calculation() {
        let recovery = ScriptRecovery::new();

        assert!(recovery.calculate_similarity("test", "test") > 0.99);
        // "test" -> "tset" has Levenshtein distance of 2, similarity = 1 - 2/4 = 0.5
        assert!(recovery.calculate_similarity("test", "tset") >= 0.50);
        assert!(recovery.calculate_similarity("hello", "world") < 0.50);
    }

    #[test]
    fn test_suggest_similar_identifier() {
        let mut context = ScriptContext::new();
        context.add_binding(Text::from("value"), Text::from("Int"));
        context.add_binding(Text::from("variable"), Text::from("Text"));

        let recovery = ScriptRecovery::new();

        // Typo: "valu" should suggest "value"
        let suggestion = recovery.suggest_similar_identifier("valu", &context);
        assert!(suggestion.is_some());
        assert_eq!(suggestion.unwrap(), "value");

        // Typo: "variabl" should suggest "variable"
        let suggestion = recovery.suggest_similar_identifier("variabl", &context);
        assert!(suggestion.is_some());
        assert_eq!(suggestion.unwrap(), "variable");
    }

    #[test]
    fn test_missing_delimiter_suggestions() {
        let recovery = ScriptRecovery::new();
        let mut context = ScriptContext::new();

        context.add_line("fn test() {");
        let suggestions = recovery.suggest_missing_delimiters(&context);
        assert!(!suggestions.is_empty());
        assert!(suggestions[0].as_str().contains("closing brace"));
    }

    #[test]
    fn test_autocompletion_suggestions() {
        let mut context = ScriptContext::new();
        context.add_binding(Text::from("value"), Text::from("Int"));
        context.add_binding(Text::from("variable"), Text::from("Text"));

        let suggestions = suggest_autocompletion("val", &context);
        assert!(!suggestions.is_empty());

        // Should suggest "value"
        let names: Vec<&str> = suggestions.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"value"));
    }

    #[test]
    fn test_keyword_autocompletion() {
        let context = ScriptContext::new();

        let suggestions = suggest_autocompletion("le", &context);
        let names: Vec<&str> = suggestions.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"let"));
    }

    #[test]
    fn test_incomplete_input_detection() {
        let recovery = ScriptRecovery::new();
        let mut context = ScriptContext::new();

        context.add_line("if x > 0 {");

        // Create a proper span for the test error at the end of input
        let error_span = Span::new(10, 10, test_file_id());
        let error = ParseError::unexpected_eof(&[TokenKind::RBrace], error_span);
        let result = recovery.recover(&error, &context);

        match result {
            RecoveryResult::Incomplete { .. } => {
                // Expected
            }
            _ => panic!("Expected incomplete result"),
        }
    }
}
