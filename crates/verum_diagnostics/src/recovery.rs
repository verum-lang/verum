//! Compiler error recovery with intelligent suggestions.
//!
//! This module implements advanced error recovery strategies for the Verum compiler,
//! enabling partial compilation and intelligent fix suggestions for IDE support.
//!
//! # Features
//!
//! - **Type error recovery**: Continue type checking with placeholder types
//! - **Name resolution recovery**: Suggest similar names on lookup failure
//! - **Partial compilation**: Generate partial IR for incremental compilation
//! - **Fix suggestions**: Multiple ranked suggestions with confidence scores
//! - **Context-aware recovery**: Different strategies based on compilation phase
//! - **Syntax error recovery**: Smart insertion/deletion for common syntax errors
//! - **Semantic error recovery**: Type coercion and implicit conversion suggestions
//!
//! # Example
//!
//! ```rust
//! use verum_diagnostics::recovery::{ErrorRecovery, RecoveryAction};
//!
//! let recovery = ErrorRecovery::new();
//! let actions = recovery.suggest_fixes_for_type_mismatch(
//!     "Int",
//!     "Text",
//!     "assignment"
//! );
//!
//! for action in actions {
//!     println!("Suggestion: {}", action.description);
//!     println!("Confidence: {}%", action.confidence);
//! }
//! ```

#[allow(unused_imports)]
use crate::{Applicability, CodeSnippet, Diagnostic, Suggestion, SuggestionBuilder};
use verum_common::{List, Maybe, Text};

/// Error recovery coordinator for the compiler.
///
/// Provides intelligent recovery strategies for various compiler errors,
/// enabling continued compilation and useful error messages even in the
/// presence of errors.
#[derive(Debug, Clone)]
pub struct ErrorRecovery {
    /// Maximum Levenshtein distance for name suggestions
    max_edit_distance: usize,
    /// Minimum confidence score for suggestions (0-100)
    min_confidence: u8,
    /// Enable experimental recovery strategies
    experimental: bool,
    /// Known type conversions for suggesting fixes
    type_conversions: List<TypeConversion>,
    /// Common syntax patterns for recovery
    syntax_patterns: List<SyntaxPattern>,
}

/// A known type conversion that can be suggested
#[derive(Debug, Clone)]
pub struct TypeConversion {
    /// Source type
    pub from: Text,
    /// Target type
    pub to: Text,
    /// Code template for the conversion
    pub template: Text,
    /// Human-readable description
    pub description: Text,
    /// Confidence level (0-100)
    pub confidence: u8,
    /// Whether this conversion is infallible
    pub infallible: bool,
}

/// A common syntax pattern for recovery
#[derive(Debug, Clone)]
pub struct SyntaxPattern {
    /// Description of the pattern
    pub description: Text,
    /// Missing element that would fix the error
    pub fix: Text,
    /// Confidence level (0-100)
    pub confidence: u8,
}

impl ErrorRecovery {
    /// Create a new error recovery coordinator with default settings.
    pub fn new() -> Self {
        Self {
            max_edit_distance: 3,
            min_confidence: 30,
            experimental: false,
            type_conversions: Self::default_type_conversions(),
            syntax_patterns: Self::default_syntax_patterns(),
        }
    }

    /// Create a recovery coordinator with custom settings.
    pub fn with_config(max_edit_distance: usize, min_confidence: u8, experimental: bool) -> Self {
        Self {
            max_edit_distance,
            min_confidence,
            experimental,
            type_conversions: Self::default_type_conversions(),
            syntax_patterns: Self::default_syntax_patterns(),
        }
    }

    /// Build default type conversions for common Verum types
    fn default_type_conversions() -> List<TypeConversion> {
        vec![
            // Numeric conversions
            TypeConversion {
                from: "Int".into(),
                to: "Text".into(),
                template: "{value}.to_string()".into(),
                description: "Convert integer to text".into(),
                confidence: 90,
                infallible: true,
            },
            TypeConversion {
                from: "Text".into(),
                to: "Int".into(),
                template: "{value}.parse::<Int>()?".into(),
                description: "Parse text as integer".into(),
                confidence: 85,
                infallible: false,
            },
            TypeConversion {
                from: "Int".into(),
                to: "Float".into(),
                template: "{value} as Float".into(),
                description: "Convert integer to float".into(),
                confidence: 95,
                infallible: true,
            },
            TypeConversion {
                from: "Float".into(),
                to: "Int".into(),
                template: "{value} as Int".into(),
                description: "Truncate float to integer".into(),
                confidence: 75,
                infallible: true,
            },
            TypeConversion {
                from: "Float".into(),
                to: "Int".into(),
                template: "{value}.round() as Int".into(),
                description: "Round float to nearest integer".into(),
                confidence: 80,
                infallible: true,
            },
            // Bool conversions
            TypeConversion {
                from: "Bool".into(),
                to: "Int".into(),
                template: "if {value} {{ 1 }} else {{ 0 }}".into(),
                description: "Convert bool to integer (1 or 0)".into(),
                confidence: 90,
                infallible: true,
            },
            TypeConversion {
                from: "Int".into(),
                to: "Bool".into(),
                template: "{value} != 0".into(),
                description: "Convert integer to bool (non-zero = true)".into(),
                confidence: 85,
                infallible: true,
            },
            // Char conversions
            TypeConversion {
                from: "Char".into(),
                to: "Text".into(),
                template: "{value}.to_string()".into(),
                description: "Convert char to text".into(),
                confidence: 95,
                infallible: true,
            },
            TypeConversion {
                from: "Text".into(),
                to: "Char".into(),
                template: "{value}.chars().next()?".into(),
                description: "Get first char from text".into(),
                confidence: 70,
                infallible: false,
            },
            // List/Array conversions
            TypeConversion {
                from: "List<T>".into(),
                to: "Array<T>".into(),
                template: "{value}.into_array()".into(),
                description: "Convert list to fixed-size array".into(),
                confidence: 75,
                infallible: false,
            },
            TypeConversion {
                from: "Array<T>".into(),
                to: "List<T>".into(),
                template: "{value}.to_list()".into(),
                description: "Convert array to list".into(),
                confidence: 95,
                infallible: true,
            },
            // Option/Maybe conversions
            TypeConversion {
                from: "T".into(),
                to: "Maybe<T>".into(),
                template: "Maybe::Some({value})".into(),
                description: "Wrap value in Maybe".into(),
                confidence: 95,
                infallible: true,
            },
            TypeConversion {
                from: "Maybe<T>".into(),
                to: "T".into(),
                template: "{value}?".into(),
                description: "Unwrap Maybe value (may fail)".into(),
                confidence: 80,
                infallible: false,
            },
            TypeConversion {
                from: "Maybe<T>".into(),
                to: "T".into(),
                template: "{value}.unwrap_or_default()".into(),
                description: "Unwrap Maybe or use default".into(),
                confidence: 85,
                infallible: true,
            },
            // Result conversions
            TypeConversion {
                from: "Result<T, E>".into(),
                to: "T".into(),
                template: "{value}?".into(),
                description: "Unwrap Result (propagate error)".into(),
                confidence: 90,
                infallible: false,
            },
            TypeConversion {
                from: "Result<T, E>".into(),
                to: "Maybe<T>".into(),
                template: "{value}.ok()".into(),
                description: "Convert Result to Maybe".into(),
                confidence: 85,
                infallible: true,
            },
            TypeConversion {
                from: "T".into(),
                to: "Result<T, E>".into(),
                template: "Ok({value})".into(),
                description: "Wrap value in Ok".into(),
                confidence: 95,
                infallible: true,
            },
        ]
        .into()
    }

    /// Build default syntax patterns for common errors
    fn default_syntax_patterns() -> List<SyntaxPattern> {
        vec![
            SyntaxPattern {
                description: "Missing semicolon".into(),
                fix: ";".into(),
                confidence: 95,
            },
            SyntaxPattern {
                description: "Missing closing brace".into(),
                fix: "}".into(),
                confidence: 90,
            },
            SyntaxPattern {
                description: "Missing closing parenthesis".into(),
                fix: ")".into(),
                confidence: 90,
            },
            SyntaxPattern {
                description: "Missing closing bracket".into(),
                fix: "]".into(),
                confidence: 90,
            },
            SyntaxPattern {
                description: "Missing colon in type annotation".into(),
                fix: ":".into(),
                confidence: 85,
            },
            SyntaxPattern {
                description: "Missing arrow in function return type".into(),
                fix: "->".into(),
                confidence: 80,
            },
            SyntaxPattern {
                description: "Missing equals in assignment".into(),
                fix: "=".into(),
                confidence: 85,
            },
            SyntaxPattern {
                description: "Missing comma in list".into(),
                fix: ",".into(),
                confidence: 80,
            },
        ]
        .into()
    }

    /// Register a custom type conversion
    pub fn register_type_conversion(&mut self, conversion: TypeConversion) {
        self.type_conversions.push(conversion);
    }

    /// Register a custom syntax pattern
    pub fn register_syntax_pattern(&mut self, pattern: SyntaxPattern) {
        self.syntax_patterns.push(pattern);
    }

    /// Suggest fixes for a type mismatch error.
    pub fn suggest_fixes_for_type_mismatch(
        &self,
        expected: &str,
        found: &str,
        context: &str,
    ) -> List<RecoveryAction> {
        let mut suggestions = List::new();

        // Common type conversions
        if let Some(conversion) = self.suggest_type_conversion(expected, found) {
            suggestions.push(RecoveryAction {
                description: conversion.0,
                code_change: Maybe::Some(conversion.1),
                confidence: 80,
                applicability: Applicability::Recommended,
            });
        }

        // Check for refinement type mismatch
        if self.is_refinement_mismatch(expected, found) {
            suggestions.push(RecoveryAction {
                description: Text::from(format!(
                    "Add runtime check: {}::try_from(value)?",
                    expected
                )),
                code_change: Maybe::Some(Text::from(format!(
                    "{}::try_from({})?",
                    expected, "value"
                ))),
                confidence: 90,
                applicability: Applicability::HasPlaceholders,
            });

            suggestions.push(RecoveryAction {
                description: Text::from(format!(
                    "Use @verify annotation: @verify(value: {})",
                    expected
                )),
                code_change: Maybe::Some(Text::from(format!("@verify(value: {})", expected))),
                confidence: 75,
                applicability: Applicability::MaybeIncorrect,
            });
        }

        // Check for reference mode mismatch
        if self.is_reference_mismatch(expected, found)
            && let Maybe::Some(fix) = self.suggest_reference_fix(expected, found)
        {
            suggestions.push(RecoveryAction {
                description: fix.0,
                code_change: Maybe::Some(fix.1),
                confidence: 85,
                applicability: Applicability::Recommended,
            });
        }

        // Context-specific suggestions
        match context {
            "assignment" => {
                suggestions.push(RecoveryAction {
                    description: Text::from("Consider changing the variable's type annotation"),
                    code_change: Maybe::None,
                    confidence: 60,
                    applicability: Applicability::MaybeIncorrect,
                });
            }
            "return" => {
                suggestions.push(RecoveryAction {
                    description: Text::from("Consider changing the function's return type"),
                    code_change: Maybe::None,
                    confidence: 65,
                    applicability: Applicability::MaybeIncorrect,
                });
            }
            "function_call" => {
                suggestions.push(RecoveryAction {
                    description: Text::from("Check the function signature for parameter types"),
                    code_change: Maybe::None,
                    confidence: 70,
                    applicability: Applicability::MaybeIncorrect,
                });
            }
            _ => {}
        }

        suggestions
    }

    /// Suggest similar names for undefined identifiers.
    pub fn suggest_similar_names(&self, name: &str, available: &[Text]) -> List<Text> {
        let mut suggestions = List::new();

        for candidate in available {
            let distance = levenshtein_distance(name, candidate);
            if distance <= self.max_edit_distance {
                suggestions.push(candidate.clone());
            }
        }

        // Sort by edit distance
        let mut suggestions_vec: List<Text> = suggestions.clone();
        suggestions_vec.sort_by_key(|s: &Text| levenshtein_distance(name, s.as_str()));
        suggestions_vec
    }

    /// Generate a placeholder type for error recovery.
    pub fn placeholder_type(&self) -> Text {
        Text::from("_")
    }

    /// Check if types are compatible after inserting conversion.
    fn suggest_type_conversion(&self, expected: &str, found: &str) -> Maybe<(Text, Text)> {
        // Common conversions
        match (expected, found) {
            ("Text", "Int") => Maybe::Some((
                Text::from("Convert to string: value.to_string()"),
                Text::from("value.to_string()"),
            )),
            ("Int", "Text") => Maybe::Some((
                Text::from("Parse to integer: value.parse::<Int>()?"),
                Text::from("value.parse::<Int>()?"),
            )),
            ("Float", "Int") => Maybe::Some((
                Text::from("Convert to float: value as Float"),
                Text::from("value as Float"),
            )),
            ("Int", "Float") => Maybe::Some((
                Text::from("Truncate to integer: value as Int"),
                Text::from("value as Int"),
            )),
            (e, f) if e.starts_with("Maybe<") && !f.starts_with("Maybe<") => Maybe::Some((
                Text::from("Wrap in Maybe: Maybe::Some(value)"),
                Text::from("Maybe::Some(value)"),
            )),
            (e, f) if !e.starts_with("Maybe<") && f.starts_with("Maybe<") => Maybe::Some((
                Text::from("Unwrap Maybe: value? or value.unwrap()"),
                Text::from("value?"),
            )),
            _ => Maybe::None,
        }
    }

    /// Check if this is a refinement type mismatch.
    ///
    /// A refinement type mismatch occurs when two types have the same base type
    /// but different constraints. For example:
    /// - `Int{x > 0}` vs `Int{x >= 0}` (same base `Int`, different constraints)
    /// - `Float{x != 0}` vs `Float` (one has constraint, one doesn't)
    ///
    /// This function recognizes several refinement type syntaxes:
    /// - `Type{constraint}` - inline refinement
    /// - `Type where constraint` - where-clause refinement
    /// - `Type{pred1, pred2}` - multiple predicates
    fn is_refinement_mismatch(&self, expected: &str, found: &str) -> bool {
        // Extract base type from both, handling multiple refinement syntaxes
        let expected_base = self.extract_base_type(expected);
        let found_base = self.extract_base_type(found);

        // Check if at least one type has refinement syntax
        let expected_has_refinement = self.has_refinement_syntax(expected);
        let found_has_refinement = self.has_refinement_syntax(found);

        // It's a refinement mismatch if:
        // 1. At least one type has refinement syntax
        // 2. The base types are the same
        (expected_has_refinement || found_has_refinement) && expected_base == found_base
    }

    /// Extract the base type from a potentially refined type.
    ///
    /// Handles the following syntaxes:
    /// - `Type{constraint}` -> `Type`
    /// - `Type where constraint` -> `Type`
    /// - `Maybe<Type{constraint}>` -> `Maybe<Type>`
    fn extract_base_type<'a>(&self, type_str: &'a str) -> &'a str {
        let trimmed = type_str.trim();

        // Handle `Type{constraint}` syntax
        if let Some(brace_pos) = trimmed.find('{') {
            // Check for balanced braces - need to handle nested generics like Maybe<Int{x > 0}>
            let before_brace = &trimmed[..brace_pos];
            // Verify this isn't inside a generic parameter
            let open_angles = before_brace.matches('<').count();
            let close_angles = before_brace.matches('>').count();

            if open_angles == close_angles {
                // The brace is for a refinement, not inside a generic
                return before_brace.trim();
            }
        }

        // Handle `Type where constraint` syntax
        if let Some(where_pos) = trimmed.find(" where ") {
            return trimmed[..where_pos].trim();
        }

        // No refinement syntax found, return the whole type
        trimmed
    }

    /// Check if a type string contains refinement syntax.
    fn has_refinement_syntax(&self, type_str: &str) -> bool {
        let trimmed = type_str.trim();

        // Check for `{constraint}` syntax
        if let Some(brace_pos) = trimmed.find('{') {
            // Verify it's not inside a generic parameter
            let before_brace = &trimmed[..brace_pos];
            let open_angles = before_brace.matches('<').count();
            let close_angles = before_brace.matches('>').count();

            if open_angles == close_angles && trimmed.contains('}') {
                return true;
            }
        }

        // Check for `where` syntax
        if trimmed.contains(" where ") {
            return true;
        }

        false
    }

    /// Check if this is a reference mode mismatch.
    fn is_reference_mismatch(&self, expected: &str, found: &str) -> bool {
        expected.starts_with('&') || found.starts_with('&')
    }

    /// Suggest fix for reference mode mismatch.
    fn suggest_reference_fix(&self, expected: &str, found: &str) -> Maybe<(Text, Text)> {
        match (expected.starts_with('&'), found.starts_with('&')) {
            (true, false) => {
                Maybe::Some((Text::from("Add reference: &value"), Text::from("&value")))
            }
            (false, true) => Maybe::Some((Text::from("Dereference: *value"), Text::from("*value"))),
            _ => Maybe::None,
        }
    }

    /// Suggest fixes for a syntax error based on common patterns.
    ///
    /// Analyzes the error context and suggests likely missing tokens.
    pub fn suggest_syntax_fixes(&self, error_context: &SyntaxErrorContext) -> List<RecoveryAction> {
        let mut suggestions = List::new();

        // Match against known patterns
        for pattern in &self.syntax_patterns {
            if self.matches_pattern(error_context, pattern) {
                suggestions.push(RecoveryAction {
                    description: pattern.description.clone(),
                    code_change: Maybe::Some(pattern.fix.clone()),
                    confidence: pattern.confidence,
                    applicability: if pattern.confidence >= 90 {
                        Applicability::Recommended
                    } else {
                        Applicability::MaybeIncorrect
                    },
                });
            }
        }

        // Context-specific recovery
        match &error_context.kind {
            SyntaxErrorKind::UnexpectedToken { expected, found } => {
                if let Some(fix) = self.suggest_token_insertion(expected, found) {
                    suggestions.push(fix);
                }
            }
            SyntaxErrorKind::UnexpectedEof { expected } => {
                for exp in expected {
                    suggestions.push(RecoveryAction {
                        description: Text::from(format!("Insert missing '{}'", exp)),
                        code_change: Maybe::Some(exp.clone()),
                        confidence: 85,
                        applicability: Applicability::Recommended,
                    });
                }
            }
            SyntaxErrorKind::InvalidExpression { hint } => {
                if let Some(h) = hint {
                    suggestions.push(RecoveryAction {
                        description: h.clone(),
                        code_change: Maybe::None,
                        confidence: 70,
                        applicability: Applicability::MaybeIncorrect,
                    });
                }
            }
            SyntaxErrorKind::MismatchedDelimiter { opening, closing } => {
                suggestions.push(RecoveryAction {
                    description: Text::from(format!(
                        "Add closing '{}' to match opening '{}'",
                        closing, opening
                    )),
                    code_change: Maybe::Some(closing.clone()),
                    confidence: 95,
                    applicability: Applicability::Recommended,
                });
            }
        }

        suggestions
    }

    /// Check if a syntax error context matches a known pattern
    fn matches_pattern(&self, context: &SyntaxErrorContext, pattern: &SyntaxPattern) -> bool {
        // Simple keyword-based matching
        let desc_lower = pattern.description.to_lowercase();

        match &context.kind {
            SyntaxErrorKind::UnexpectedToken { expected, .. } => {
                for exp in expected {
                    if desc_lower.contains(&exp.to_lowercase()) {
                        return true;
                    }
                }
            }
            SyntaxErrorKind::UnexpectedEof { expected } => {
                for exp in expected {
                    if desc_lower.contains(&exp.to_lowercase()) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }

    /// Suggest a token insertion based on expected vs found
    fn suggest_token_insertion(&self, expected: &[Text], found: &str) -> Option<RecoveryAction> {
        // Common insertion suggestions
        if expected.iter().any(|e| e == ";") && !found.contains(';') {
            return Some(RecoveryAction {
                description: Text::from("Insert semicolon before this token"),
                code_change: Maybe::Some(Text::from(";")),
                confidence: 95,
                applicability: Applicability::Recommended,
            });
        }

        if expected.iter().any(|e| e == ",") && !found.contains(',') {
            return Some(RecoveryAction {
                description: Text::from("Insert comma to separate items"),
                code_change: Maybe::Some(Text::from(",")),
                confidence: 90,
                applicability: Applicability::Recommended,
            });
        }

        None
    }

    /// Suggest fixes for an undefined identifier.
    ///
    /// Uses Levenshtein distance and other heuristics to find similar names.
    pub fn suggest_fixes_for_undefined_name(
        &self,
        name: &str,
        available_names: &[Text],
        available_types: &[Text],
        context: NameContext,
    ) -> List<RecoveryAction> {
        let mut suggestions = List::new();

        // Find similar names using edit distance
        let similar = self.suggest_similar_names(name, available_names);
        for (i, sim) in similar.iter().enumerate() {
            let confidence = 90 - (i * 5) as u8; // Decrease confidence for later suggestions
            suggestions.push(RecoveryAction {
                description: Text::from(format!("Did you mean '{}'?", sim)),
                code_change: Maybe::Some(sim.clone()),
                confidence: confidence.max(50),
                applicability: Applicability::Recommended,
            });
        }

        // Context-specific suggestions
        match context {
            NameContext::Variable => {
                suggestions.push(RecoveryAction {
                    description: Text::from(format!("Declare variable: let {} = ...", name)),
                    code_change: Maybe::Some(Text::from(format!("let {} = ", name))),
                    confidence: 60,
                    applicability: Applicability::HasPlaceholders,
                });
            }
            NameContext::Function => {
                suggestions.push(RecoveryAction {
                    description: Text::from(format!("Define function: fn {}() {{ ... }}", name)),
                    code_change: Maybe::Some(Text::from(format!("fn {}() {{\n    \n}}", name))),
                    confidence: 55,
                    applicability: Applicability::HasPlaceholders,
                });
            }
            NameContext::Type => {
                // Check if it might be a typo for a built-in type
                let builtin_types = vec![
                    "Int", "Float", "Bool", "Text", "Char", "List", "Map", "Set", "Maybe", "Result",
                ];
                for bt in builtin_types {
                    let dist = levenshtein_distance(name, bt);
                    if dist <= 2 {
                        suggestions.push(RecoveryAction {
                            description: Text::from(format!(
                                "Did you mean built-in type '{}'?",
                                bt
                            )),
                            code_change: Maybe::Some(Text::from(bt)),
                            confidence: 95 - (dist * 10) as u8,
                            applicability: Applicability::Recommended,
                        });
                    }
                }

                // Suggest defining a new type
                suggestions.push(RecoveryAction {
                    description: Text::from(format!("Define type: type {} is ...", name)),
                    code_change: Maybe::Some(Text::from(format!("type {} is", name))),
                    confidence: 50,
                    applicability: Applicability::HasPlaceholders,
                });
            }
            NameContext::Module => {
                suggestions.push(RecoveryAction {
                    description: Text::from(format!("Import module: using {}", name)),
                    code_change: Maybe::Some(Text::from(format!("using {}", name))),
                    confidence: 65,
                    applicability: Applicability::MaybeIncorrect,
                });
            }
            NameContext::Context => {
                // Check available contexts
                let similar_contexts = self.suggest_similar_names(name, available_types);
                for ctx in similar_contexts.iter().take(3) {
                    suggestions.push(RecoveryAction {
                        description: Text::from(format!("Did you mean context '{}'?", ctx)),
                        code_change: Maybe::Some(ctx.clone()),
                        confidence: 85,
                        applicability: Applicability::Recommended,
                    });
                }
            }
        }

        suggestions
    }

    /// Suggest fixes for an arity mismatch (wrong number of arguments).
    pub fn suggest_fixes_for_arity_mismatch(
        &self,
        function_name: &str,
        expected: usize,
        found: usize,
        param_names: &[Text],
    ) -> List<RecoveryAction> {
        let mut suggestions = List::new();

        if found < expected {
            // Too few arguments
            let missing_count = expected - found;
            let missing_params: List<&Text> = param_names.iter().skip(found).collect();

            if !missing_params.is_empty() {
                let missing_str = missing_params
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                suggestions.push(RecoveryAction {
                    description: Text::from(format!(
                        "Add {} missing argument{}: {}",
                        missing_count,
                        if missing_count > 1 { "s" } else { "" },
                        missing_str
                    )),
                    code_change: Maybe::None,
                    confidence: 85,
                    applicability: Applicability::HasPlaceholders,
                });
            }

            suggestions.push(RecoveryAction {
                description: Text::from(format!(
                    "Function '{}' expects {} argument{}, but {} {} provided",
                    function_name,
                    expected,
                    if expected != 1 { "s" } else { "" },
                    found,
                    if found != 1 { "were" } else { "was" }
                )),
                code_change: Maybe::None,
                confidence: 95,
                applicability: Applicability::MaybeIncorrect,
            });
        } else {
            // Too many arguments
            let extra_count = found - expected;
            suggestions.push(RecoveryAction {
                description: Text::from(format!(
                    "Remove {} extra argument{}",
                    extra_count,
                    if extra_count > 1 { "s" } else { "" }
                )),
                code_change: Maybe::None,
                confidence: 85,
                applicability: Applicability::MaybeIncorrect,
            });

            suggestions.push(RecoveryAction {
                description: Text::from(format!(
                    "Function '{}' expects {} argument{}, but {} {} provided",
                    function_name,
                    expected,
                    if expected != 1 { "s" } else { "" },
                    found,
                    if found != 1 { "were" } else { "was" }
                )),
                code_change: Maybe::None,
                confidence: 95,
                applicability: Applicability::MaybeIncorrect,
            });
        }

        suggestions
    }

    /// Create recovery state for continuing compilation after an error.
    pub fn create_recovery_state(&self, error_kind: &ErrorKind) -> RecoveryState {
        match error_kind {
            ErrorKind::Type => RecoveryState {
                placeholder_type: self.placeholder_type(),
                can_continue: true,
                severity: RecoverySeverity::Recoverable,
            },
            ErrorKind::Name => RecoveryState {
                placeholder_type: Text::from("_unknown"),
                can_continue: true,
                severity: RecoverySeverity::Recoverable,
            },
            ErrorKind::Syntax => RecoveryState {
                placeholder_type: Text::from("_error"),
                can_continue: false,
                severity: RecoverySeverity::Fatal,
            },
            ErrorKind::Semantic => RecoveryState {
                placeholder_type: self.placeholder_type(),
                can_continue: true,
                severity: RecoverySeverity::Warning,
            },
        }
    }
}

/// Context for syntax error recovery
#[derive(Debug, Clone)]
pub struct SyntaxErrorContext {
    /// Kind of syntax error
    pub kind: SyntaxErrorKind,
    /// Line number where error occurred
    pub line: usize,
    /// Column number where error occurred
    pub column: usize,
    /// Surrounding source context (if available)
    pub context: Option<Text>,
}

/// Kind of syntax error
#[derive(Debug, Clone)]
pub enum SyntaxErrorKind {
    /// Unexpected token encountered
    UnexpectedToken { expected: List<Text>, found: Text },
    /// Unexpected end of file
    UnexpectedEof { expected: List<Text> },
    /// Invalid expression
    InvalidExpression { hint: Option<Text> },
    /// Mismatched delimiter
    MismatchedDelimiter { opening: Text, closing: Text },
}

/// Context for name resolution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameContext {
    /// Variable name
    Variable,
    /// Function name
    Function,
    /// Type name
    Type,
    /// Module name
    Module,
    /// Context name (for context system)
    Context,
}

/// Kind of error for recovery purposes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Type error
    Type,
    /// Name resolution error
    Name,
    /// Syntax error
    Syntax,
    /// Semantic error
    Semantic,
}

/// Recovery state after an error
#[derive(Debug, Clone)]
pub struct RecoveryState {
    /// Placeholder type to use for continued type checking
    pub placeholder_type: Text,
    /// Whether compilation can continue
    pub can_continue: bool,
    /// Severity of the recovery
    pub severity: RecoverySeverity,
}

/// Severity of error recovery
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverySeverity {
    /// Error can be recovered from without issues
    Recoverable,
    /// Error causes a warning but compilation can continue
    Warning,
    /// Error is fatal and compilation should stop
    Fatal,
}

impl Default for ErrorRecovery {
    fn default() -> Self {
        Self::new()
    }
}

/// A suggested recovery action for a compiler error.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryAction {
    /// Human-readable description of the fix
    pub description: Text,
    /// Optional code change to apply
    pub code_change: Maybe<Text>,
    /// Confidence score (0-100)
    pub confidence: u8,
    /// How safely this can be auto-applied
    pub applicability: Applicability,
}

impl RecoveryAction {
    /// Convert to a diagnostic suggestion.
    pub fn to_suggestion(&self) -> Suggestion {
        let mut builder = SuggestionBuilder::new(self.description.clone());

        if let Maybe::Some(ref code) = self.code_change {
            let snippet = CodeSnippet::new(code.as_str());
            builder = builder.add_snippet(snippet);
        }

        builder.build()
    }

    /// Check if this action can be auto-applied.
    pub fn is_auto_applicable(&self) -> bool {
        self.applicability.is_safe_to_apply()
    }

    /// Get a ranking score for sorting suggestions.
    pub fn ranking_score(&self) -> u32 {
        let mut score = self.confidence as u32 * 100;

        // Prefer machine-applicable suggestions
        if self.is_auto_applicable() {
            score += 1000;
        }

        // Prefer suggestions with concrete code changes
        if self.code_change.is_some() {
            score += 500;
        }

        score
    }
}

/// Compute Levenshtein distance between two strings.
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    let s1_chars: List<char> = s1.chars().collect();
    let s2_chars: List<char> = s2.chars().collect();

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] {
                0
            } else {
                1
            };

            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[len1][len2]
}

/// Partial compilation state for error recovery.
#[derive(Debug, Clone)]
pub struct PartialCompilation {
    /// Successfully compiled items
    pub valid_items: List<Text>,
    /// Items with errors (using placeholders)
    pub partial_items: List<Text>,
    /// Total errors encountered
    pub error_count: usize,
    /// Whether compilation can proceed to next phase
    pub can_continue: bool,
}

impl PartialCompilation {
    /// Create a new partial compilation state.
    pub fn new() -> Self {
        Self {
            valid_items: List::new(),
            partial_items: List::new(),
            error_count: 0,
            can_continue: true,
        }
    }

    /// Add a successfully compiled item.
    pub fn add_valid(&mut self, item: Text) {
        self.valid_items.push(item);
    }

    /// Add an item with errors.
    pub fn add_partial(&mut self, item: Text) {
        self.partial_items.push(item);
        self.error_count += 1;
    }

    /// Check if too many errors to continue.
    pub fn should_stop(&self) -> bool {
        !self.can_continue || self.error_count > 100
    }

    /// Get completion percentage.
    pub fn completion_percentage(&self) -> u8 {
        let total = self.valid_items.len() + self.partial_items.len();
        if total == 0 {
            return 0;
        }
        ((self.valid_items.len() * 100) / total) as u8
    }
}

impl Default for PartialCompilation {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Rust-to-Verum Migration Helpers
// ============================================================================

/// Mapping from Rust keywords to their Verum equivalents.
/// Used by the parser and type checker to generate helpful migration messages.
pub static RUST_KEYWORD_MAP: &[(&str, &str)] = &[
    ("struct", "type Name is { ... }"),
    ("enum", "type Name is A | B(T) | C { ... }"),
    ("trait", "type Name is protocol { ... }"),
    ("impl", "implement"),
    ("use", "mount"),
    ("mod", "module"),
    ("crate", "cog"),
    ("pub(crate)", "internal"),
    ("pub(super)", "protected"),
    ("derive", "@derive(...)"),
    ("repr", "@repr(...)"),
    ("cfg", "@cfg(...)"),
];

/// Mapping from Rust type names to Verum semantic type names.
pub static RUST_TYPE_MAP: &[(&str, &str)] = &[
    ("String", "Text"),
    ("&str", "Text"),
    ("Vec", "List"),
    ("HashMap", "Map"),
    ("HashSet", "Set"),
    ("BTreeMap", "Map"),
    ("BTreeSet", "Set"),
    ("Box", "Heap"),
    ("Rc", "Shared"),
    ("Arc", "Shared"),
    ("Option", "Maybe"),
    ("Cell", "Mut"),
    ("RefCell", "Mut"),
];

/// Mapping from Rust macro calls to Verum equivalents.
pub static RUST_MACRO_MAP: &[(&str, &str)] = &[
    ("println!", "print(...)"),
    ("print!", "print(...) (without !)"),
    ("eprintln!", "eprint(...)"),
    ("eprint!", "eprint(...) (without !)"),
    ("format!", "f\"...\" (format string literal)"),
    ("panic!", "panic(...)"),
    ("assert!", "assert(...)"),
    ("assert_eq!", "assert_eq(...)"),
    ("assert_ne!", "assert_ne(...)"),
    ("unreachable!", "unreachable()"),
    ("unimplemented!", "unimplemented()"),
    ("todo!", "todo()"),
    ("vec!", "List(...) or [a, b, c]"),
    ("dbg!", "debug(...)"),
    ("matches!", "x is Pattern (is operator)"),
    ("include_str!", "@include_str(...)"),
    ("include_bytes!", "@include_bytes(...)"),
];

/// Check if a type name is a Rust type and return the Verum equivalent.
pub fn rust_type_suggestion(name: &str) -> Option<&'static str> {
    for (rust_name, verum_name) in RUST_TYPE_MAP {
        if *rust_name == name {
            return Some(verum_name);
        }
    }
    None
}

/// Check if a macro name is a Rust macro and return the Verum equivalent.
pub fn rust_macro_suggestion(name: &str) -> Option<&'static str> {
    for (rust_name, verum_name) in RUST_MACRO_MAP {
        if *rust_name == name {
            return Some(verum_name);
        }
    }
    None
}

/// Check if a keyword is a Rust keyword and return the Verum equivalent.
pub fn rust_keyword_suggestion(name: &str) -> Option<&'static str> {
    for (rust_kw, verum_kw) in RUST_KEYWORD_MAP {
        if *rust_kw == name {
            return Some(verum_kw);
        }
    }
    None
}

/// Find the closest matching name from a list of candidates.
/// Returns None if no candidate is within the edit distance threshold.
pub fn find_closest_name<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    max_distance: usize,
) -> Option<&'a str> {
    let mut best_match: Option<&str> = None;
    let mut best_distance = usize::MAX;

    for candidate in candidates {
        let distance = levenshtein_distance(name, candidate);
        if distance < best_distance && distance <= max_distance {
            best_distance = distance;
            best_match = Some(candidate);
        }
    }

    best_match
}
