//! Comprehensive quick fix generation for refinement violations
//!
//! Quick Fix Generation for Refinement Violations:
//!
//! This module generates actionable quick fixes for refinement type violations
//! detected by the SMT solver. It provides 6 categories of fixes with different
//! priorities and safety guarantees:
//!
//! 1. Runtime Check Wrapping (priority 1, safe)
//! 2. Inline Refinement (priority 2, breaking)
//! 3. Sigma Type Conversion (priority 3, breaking)
//! 4. Runtime Assertion (priority 3, safe)
//! 5. Weaken Refinement (priority 4, maybe breaking)
//! 6. Promote to &checked (priority 5, safe)
//!
//! Each quick fix includes:
//! - Human-readable title and description
//! - Impact analysis (safe/breaking/unsafe)
//! - Concrete TextEdit operations
//! - Priority ordering

use tower_lsp::lsp_types::*;
use verum_common::{List, Maybe, Text};
use verum_smt::counterexample::{CounterExample, FailureCategory};

use crate::document::DocumentState;

// ==================== Core Types ====================

/// Kind of quick fix action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFixKind {
    /// Wrap with Result<T, E> using runtime check
    RuntimeCheck,
    /// Add refinement constraint to parameter type
    InlineRefinement,
    /// Convert to dependent pair (Sigma type)
    SigmaType,
    /// Add runtime assertion (assert!)
    Assertion,
    /// Relax overly strict constraint
    WeakenRefinement,
    /// Promote reference from &T to &checked T
    PromoteToChecked,
}

impl QuickFixKind {
    /// Get the LSP CodeActionKind for this fix
    pub fn to_lsp_kind(&self) -> CodeActionKind {
        match self {
            Self::RuntimeCheck
            | Self::InlineRefinement
            | Self::Assertion
            | Self::PromoteToChecked => CodeActionKind::QUICKFIX,
            Self::SigmaType | Self::WeakenRefinement => CodeActionKind::REFACTOR,
        }
    }
}

/// Impact level of applying a quick fix
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixImpact {
    /// Safe: no breaking changes, only local modifications
    Safe,
    /// Breaking: changes function signature, affects callers
    Breaking,
    /// Maybe: might be breaking depending on usage
    MaybeBreaking,
    /// Unsafe: introduces unsafe code
    Unsafe,
}

impl FixImpact {
    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Breaking => "breaking",
            Self::MaybeBreaking => "maybe breaking",
            Self::Unsafe => "unsafe",
        }
    }
}

/// A quick fix for a refinement violation
#[derive(Debug, Clone)]
pub struct QuickFix {
    /// Human-readable title
    pub title: Text,
    /// Kind of fix
    pub kind: QuickFixKind,
    /// Priority (1 = highest, 5 = lowest)
    pub priority: u8,
    /// Impact analysis
    pub impact: FixImpact,
    /// Detailed description
    pub description: Text,
    /// Text edits to apply
    pub edits: List<TextEdit>,
}

impl QuickFix {
    /// Create a new quick fix
    pub fn new(
        title: impl Into<Text>,
        kind: QuickFixKind,
        priority: u8,
        impact: FixImpact,
        description: impl Into<Text>,
        edits: List<TextEdit>,
    ) -> Self {
        Self {
            title: title.into(),
            kind,
            priority,
            impact,
            description: description.into(),
            edits,
        }
    }

    /// Convert to LSP CodeAction
    pub fn to_code_action(&self, uri: &Url, diagnostics: Vec<Diagnostic>) -> CodeAction {
        let mut changes = std::collections::HashMap::new();
        changes.insert(uri.clone(), self.edits.to_vec());

        CodeAction {
            title: format!("{} [{}]", self.title, self.impact.description()),
            kind: Some(self.kind.to_lsp_kind()),
            diagnostics: Some(diagnostics),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: Some(self.priority == 1),
            disabled: None,
            data: None,
        }
    }
}

// ==================== Quick Fix Generator ====================

/// Generates quick fixes for refinement violations
pub struct QuickFixGenerator<'a> {
    document: &'a DocumentState,
    uri: &'a Url,
}

impl<'a> QuickFixGenerator<'a> {
    /// Create a new generator
    pub fn new(document: &'a DocumentState, uri: &'a Url) -> Self {
        Self { document, uri }
    }

    /// Generate all applicable quick fixes for a refinement violation
    pub fn generate_for_refinement_violation(
        &self,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        violated_constraint: &str,
    ) -> List<QuickFix> {
        let mut fixes = List::new();

        // Categorize the failure for targeted suggestions
        let category = if let Maybe::Some(ce) = counterexample {
            categorize_failure(ce, violated_constraint)
        } else {
            FailureCategory::Other
        };

        // Generate fixes based on category and context
        match category {
            FailureCategory::DivisionByZero => {
                self.generate_division_by_zero_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
            FailureCategory::IndexOutOfBounds => {
                self.generate_bounds_check_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
            FailureCategory::NegativeValue => {
                self.generate_negative_value_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
            FailureCategory::ArithmeticOverflow => {
                self.generate_overflow_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
            FailureCategory::NullDereference => {
                self.generate_null_check_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
            FailureCategory::Other => {
                self.generate_generic_fixes(
                    &mut fixes,
                    diagnostic,
                    counterexample,
                    violated_constraint,
                );
            }
        }

        // Always add promote to &checked if applicable
        if self.is_reference_related(violated_constraint) {
            self.generate_promote_to_checked_fix(&mut fixes, diagnostic);
        }

        // Sort by priority
        fixes.sort_by_key(|f| f.priority);

        fixes
    }

    // ==================== Division by Zero Fixes ====================

    fn generate_division_by_zero_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        _constraint: &str,
    ) {
        // Extract variable name from counterexample
        let var_name: Maybe<Text> = if let Maybe::Some(ce) = counterexample {
            ce.assignments
                .iter()
                .find(|(_, v)| {
                    if let Maybe::Some(i) = Maybe::Some(v.as_int()) {
                        i == Some(0)
                    } else {
                        false
                    }
                })
                .map(|(k, _)| Text::from(k.as_str()))
        } else {
            Maybe::None
        };

        let var_name = var_name.unwrap_or(Text::from("y"));

        // Fix 1: Wrap with runtime check (Result)
        let runtime_check_fix = self.create_runtime_check_fix(
            diagnostic,
            &var_name,
            "NonZero.try_from",
            "Wraps division with runtime validation using Result<T, E>. \
                 Caller must handle error case with ? or match.",
        );
        fixes.push(runtime_check_fix);

        // Fix 2: Add inline refinement
        let inline_fix = self.create_inline_refinement_fix(
            diagnostic,
            &var_name,
            &format!("{} != 0", var_name.as_str()),
            &format!(
                "Adds refinement constraint to parameter, requiring callers \
                 to prove {} is non-zero at call site.",
                var_name.as_str()
            ),
        );
        fixes.push(inline_fix);

        // Fix 3: Runtime assertion
        let assertion_fix = self.create_assertion_fix(
            diagnostic,
            &format!("{} != 0", var_name.as_str()),
            &format!("Divisor {} cannot be zero", var_name.as_str()),
            "Adds runtime assertion that panics if divisor is zero.",
        );
        fixes.push(assertion_fix);
    }

    // ==================== Bounds Check Fixes ====================

    fn generate_bounds_check_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        constraint: &str,
    ) {
        // Extract collection and index variable names
        let (collection, index) = self.extract_collection_index(constraint, counterexample);

        let collection_text: Text = collection.clone().into();
        let index_text: Text = index.clone().into();

        // Fix 1: Sigma type conversion
        let sigma_fix = self.create_sigma_type_fix(
            diagnostic,
            &collection_text,
            &format!("len({}) > 0", collection),
            &format!(
                "Converts parameter to dependent pair that proves {} is non-empty. \
                 Callers must provide proof of non-emptiness.",
                collection
            ),
        );
        fixes.push(sigma_fix);

        // Fix 2: Inline refinement with bounds
        let inline_fix = self.create_inline_refinement_fix(
            diagnostic,
            &index_text,
            &format!("{} >= 0 && {} < len({})", index, index, collection),
            "Adds bounds constraint to index parameter, requiring callers \
                 to prove index is within valid range.",
        );
        fixes.push(inline_fix);

        // Fix 3: Runtime check with .get()
        let get_fix = self.create_safe_access_fix(
            diagnostic,
            &collection_text,
            &index_text,
            "Use safe accessor .get() which returns Maybe<T> instead of panicking.",
        );
        fixes.push(get_fix);

        // Fix 4: Runtime assertion
        let assertion_fix = self.create_assertion_fix(
            diagnostic,
            &format!("{} < {}.len()", index, collection),
            &format!("Index {} out of bounds", index),
            "Adds runtime assertion before array access.",
        );
        fixes.push(assertion_fix);
    }

    // ==================== Negative Value Fixes ====================

    fn generate_negative_value_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        constraint: &str,
    ) {
        // Extract variable that should be positive
        let var_name: Text = if let Maybe::Some(ce) = counterexample {
            ce.assignments
                .keys()
                .next()
                .map(|k| Text::from(k.as_str()))
                .unwrap_or_else(|| Text::from("x"))
        } else {
            Text::from("x")
        };

        // Determine the constraint (> 0 or >= 0)
        let is_positive = constraint.contains("> 0");
        let constraint_str = if is_positive {
            format!("{} > 0", var_name.as_str())
        } else {
            format!("{} >= 0", var_name.as_str())
        };

        // Fix 1: Runtime check with validation
        let runtime_fix = self.create_runtime_check_fix(
            diagnostic,
            &var_name,
            if is_positive {
                "Positive.try_from"
            } else {
                "NonNegative.try_from"
            },
            &format!(
                "Validates that {} is {} at runtime using Result<T, E>.",
                var_name.as_str(),
                if is_positive {
                    "positive"
                } else {
                    "non-negative"
                }
            ),
        );
        fixes.push(runtime_fix);

        // Fix 2: Inline refinement
        let inline_fix = self.create_inline_refinement_fix(
            diagnostic,
            &var_name,
            &constraint_str,
            &format!(
                "Requires callers to prove {} is {}.",
                var_name.as_str(),
                if is_positive {
                    "positive"
                } else {
                    "non-negative"
                }
            ),
        );
        fixes.push(inline_fix);

        // Fix 3: Runtime assertion
        let assertion_fix = self.create_assertion_fix(
            diagnostic,
            &constraint_str,
            &format!(
                "{} must be {}",
                var_name.as_str(),
                if is_positive {
                    "positive"
                } else {
                    "non-negative"
                }
            ),
            "Adds runtime assertion for value constraint.",
        );
        fixes.push(assertion_fix);
    }

    // ==================== Overflow Fixes ====================

    fn generate_overflow_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        _counterexample: Maybe<&CounterExample>,
        _constraint: &str,
    ) {
        // Fix 1: Use checked arithmetic
        let checked_fix = QuickFix::new(
            "Use checked arithmetic",
            QuickFixKind::RuntimeCheck,
            1,
            FixImpact::Safe,
            "Replace operation with checked_add/checked_mul that returns Maybe<T> on overflow.",
            List::from(vec![TextEdit {
                range: diagnostic.range,
                new_text: "checked_add".to_string(),
            }]),
        );
        fixes.push(checked_fix);

        // Fix 2: Weaken refinement to allow overflow
        let weaken_fix = self.create_weaken_refinement_fix(
            diagnostic,
            "overflow",
            "Remove overflow constraint, accepting wrapping behavior.",
        );
        fixes.push(weaken_fix);
    }

    // ==================== Null Check Fixes ====================

    fn generate_null_check_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        _constraint: &str,
    ) {
        let var_name: Text = if let Maybe::Some(ce) = counterexample {
            ce.assignments
                .keys()
                .next()
                .map(|k| Text::from(k.as_str()))
                .unwrap_or_else(|| Text::from("value"))
        } else {
            Text::from("value")
        };

        // Fix 1: Use Maybe<T> with safe unwrap
        let maybe_fix = QuickFix::new(
            "Use safe Maybe<T> unwrap",
            QuickFixKind::RuntimeCheck,
            1,
            FixImpact::Safe,
            format!(
                "Replace with {}.unwrap_or_default() or {}.is_some() check.",
                var_name.as_str(),
                var_name.as_str()
            ),
            List::from(vec![TextEdit {
                range: diagnostic.range,
                new_text: format!("{}.unwrap_or_default()", var_name.as_str()),
            }]),
        );
        fixes.push(maybe_fix);

        // Fix 2: Inline refinement for non-None
        let inline_fix = self.create_inline_refinement_fix(
            diagnostic,
            &var_name,
            &format!("{}.is_some()", var_name.as_str()),
            "Requires callers to prove value is Some before passing.",
        );
        fixes.push(inline_fix);

        // Fix 3: Runtime assertion
        let assertion_fix = self.create_assertion_fix(
            diagnostic,
            &format!("{}.is_some()", var_name.as_str()),
            &format!("{} must be Some", var_name.as_str()),
            "Adds runtime assertion for non-None value.",
        );
        fixes.push(assertion_fix);
    }

    // ==================== Generic Fixes ====================

    fn generate_generic_fixes(
        &self,
        fixes: &mut List<QuickFix>,
        diagnostic: &Diagnostic,
        counterexample: Maybe<&CounterExample>,
        constraint: &str,
    ) {
        // Fix 1: Runtime check wrapper
        let var_name: Text = if let Maybe::Some(ce) = counterexample {
            ce.assignments
                .keys()
                .next()
                .map(|k| Text::from(k.as_str()))
                .unwrap_or_else(|| Text::from("value"))
        } else {
            Text::from("value")
        };

        let runtime_fix = self.create_runtime_check_fix(
            diagnostic,
            &var_name,
            "validate",
            "Adds runtime validation with Result<T, E> error handling.",
        );
        fixes.push(runtime_fix);

        // Fix 2: Inline refinement
        let constraint_string = constraint.to_string();
        let inline_fix = self.create_inline_refinement_fix(
            diagnostic,
            &var_name,
            &constraint_string,
            "Adds constraint to parameter type, proving at compile time.",
        );
        fixes.push(inline_fix);

        // Fix 3: Runtime assertion
        let constraint_string2 = constraint.to_string();
        let assertion_fix = self.create_assertion_fix(
            diagnostic,
            &constraint_string2,
            &"Constraint violation".to_string(),
            "Adds runtime assertion that panics on constraint violation.",
        );
        fixes.push(assertion_fix);

        // Fix 4: Weaken refinement
        let weaken_fix = self.create_weaken_refinement_fix(
            diagnostic,
            &constraint_string2,
            "Weakens the refinement constraint to accept more values.",
        );
        fixes.push(weaken_fix);
    }

    // ==================== Fix Constructors ====================

    fn create_runtime_check_fix(
        &self,
        diagnostic: &Diagnostic,
        var_name: &Text,
        wrapper_fn: &str,
        description: &str,
    ) -> QuickFix {
        // Generate proper runtime check code based on wrapper function
        let new_text = if wrapper_fn.contains("try_from") {
            // Pattern: let validated = Type::try_from(value)?;
            format!(
                "let {} = {}({})?",
                var_name.as_str(),
                wrapper_fn,
                var_name.as_str()
            )
        } else if wrapper_fn.contains("checked_") {
            // Pattern: x.checked_add(y).ok_or(...)?
            format!("checked_{}", var_name.as_str())
        } else if wrapper_fn == "validate" {
            // Generic validation pattern
            format!(
                "if !validate({}) {{\n    return Err(ValidationError::InvalidInput);\n}}",
                var_name.as_str()
            )
        } else {
            // Default: wrap with ? operator
            format!("{}({})?", wrapper_fn, var_name.as_str())
        };

        let edit = TextEdit {
            range: diagnostic.range,
            new_text,
        };

        QuickFix::new(
            format!("Wrap with runtime check ({})", wrapper_fn),
            QuickFixKind::RuntimeCheck,
            1,
            FixImpact::Safe,
            description,
            List::from(vec![edit]),
        )
    }

    fn create_inline_refinement_fix(
        &self,
        diagnostic: &Diagnostic,
        var_name: &Text,
        constraint: &String,
        description: &str,
    ) -> QuickFix {
        // Generate proper refinement type based on base type and constraint
        let new_type = self.generate_refinement_type(var_name, constraint);

        let edit = TextEdit {
            range: diagnostic.range,
            new_text: new_type.clone(),
        };

        QuickFix::new(
            format!("Add inline refinement: {}", constraint),
            QuickFixKind::InlineRefinement,
            2,
            FixImpact::Breaking,
            description,
            List::from(vec![edit]),
        )
    }

    /// Generate a refinement type from base type and constraint
    fn generate_refinement_type(&self, var_name: &Text, constraint: &str) -> String {
        // Determine base type from context (default to Int)
        let base_type = self.infer_base_type_from_constraint(constraint);

        // Format refinement type: BaseType{constraint}
        // Examples:
        //   Int{i != 0}
        //   List<T>{len(it) > 0}
        //   Int{i > 0 && i < 100}

        // Check if constraint already references the refinement variable
        if constraint.contains("it") || constraint.contains(var_name.as_str()) {
            // Constraint already has variable reference
            format!("{}{{{}}}", base_type, constraint)
        } else {
            // Need to add variable reference based on type
            let var_ref = if base_type.starts_with("List") || base_type.starts_with("Array") {
                "it" // Collections use 'it'
            } else {
                var_name.as_str() // Scalars use the variable name
            };

            // Rewrite constraint to use proper variable
            let refined_constraint = self.rewrite_constraint_with_var(constraint, var_ref);
            format!("{}{{{}}}", base_type, refined_constraint)
        }
    }

    /// Infer base type from constraint patterns
    fn infer_base_type_from_constraint(&self, constraint: &str) -> String {
        if constraint.contains("len(") || constraint.contains("size(") {
            "List<T>".to_string()
        } else if constraint.contains("/") || constraint.contains("+") || constraint.contains("-") {
            "Int".to_string()
        } else if constraint.contains("is_some") || constraint.contains("is_none") {
            "Maybe<T>".to_string()
        } else {
            "Int".to_string() // Default
        }
    }

    /// Rewrite constraint to use the given variable
    fn rewrite_constraint_with_var(&self, constraint: &str, var: &str) -> String {
        // Simple implementation: if constraint is a comparison, ensure variable is used
        if constraint.contains("!=") || constraint.contains(">") || constraint.contains("<") {
            constraint.to_string()
        } else {
            format!("{} {}", var, constraint)
        }
    }

    fn create_sigma_type_fix(
        &self,
        diagnostic: &Diagnostic,
        var_name: &Text,
        proof_constraint: &str,
        description: &str,
    ) -> QuickFix {
        // Generate sigma type (dependent pair) with value and proof
        // Pattern: (v: BaseType, proof: constraint(v))

        let base_type = self.infer_base_type_from_constraint(proof_constraint);

        // Generate sigma type signature
        let new_type = format!("(v: {}, proof: {})", base_type, proof_constraint);

        // Also need to update usage sites to access .0 for the value
        // This would require additional edits, but for now we just change the type
        let mut edits = List::new();

        // Edit 1: Change parameter type to sigma type
        edits.push(TextEdit {
            range: diagnostic.range,
            new_text: new_type.clone(),
        });

        // Edit 2: Add usage note (as a comment) about accessing the value
        let comment_pos = Position {
            line: diagnostic.range.start.line,
            character: diagnostic.range.end.character,
        };
        edits.push(TextEdit {
            range: Range {
                start: comment_pos,
                end: comment_pos,
            },
            new_text: format!(
                "  // Access value with {}.0, proof with {}.1",
                var_name.as_str(),
                var_name.as_str()
            ),
        });

        QuickFix::new(
            "Convert to dependent pair (Sigma type)",
            QuickFixKind::SigmaType,
            3,
            FixImpact::Breaking,
            description,
            edits,
        )
    }

    fn create_assertion_fix(
        &self,
        diagnostic: &Diagnostic,
        condition: &String,
        message: &String,
        description: &str,
    ) -> QuickFix {
        // Insert assertion before the problematic line
        let insert_pos = Position {
            line: diagnostic.range.start.line,
            character: 0,
        };

        let indent = self.get_line_indent(diagnostic.range.start.line);
        let assertion_code = format!("{}assert!({}, \"{}\");\n", indent, condition, message);

        let edit = TextEdit {
            range: Range {
                start: insert_pos,
                end: insert_pos,
            },
            new_text: assertion_code,
        };

        QuickFix::new(
            format!("Add runtime assertion: {}", condition),
            QuickFixKind::Assertion,
            3,
            FixImpact::Safe,
            description,
            List::from(vec![edit]),
        )
    }

    fn create_safe_access_fix(
        &self,
        diagnostic: &Diagnostic,
        collection: &Text,
        index: &Text,
        description: &str,
    ) -> QuickFix {
        // Replace arr[i] with arr.get(i)
        let new_text = format!("{}.get({})", collection.as_str(), index.as_str());

        let edit = TextEdit {
            range: diagnostic.range,
            new_text,
        };

        QuickFix::new(
            "Use safe accessor .get()",
            QuickFixKind::RuntimeCheck,
            1,
            FixImpact::Safe,
            description,
            List::from(vec![edit]),
        )
    }

    fn create_weaken_refinement_fix(
        &self,
        diagnostic: &Diagnostic,
        constraint: &str,
        description: &str,
    ) -> QuickFix {
        // Parse and weaken the constraint by removing the most restrictive part
        let weakened = self.weaken_constraint(constraint);

        let edit = TextEdit {
            range: diagnostic.range,
            new_text: weakened.clone(),
        };

        QuickFix::new(
            format!("Relax constraint to: {}", weakened),
            QuickFixKind::WeakenRefinement,
            4,
            FixImpact::MaybeBreaking,
            description,
            List::from(vec![edit]),
        )
    }

    /// Weaken a constraint by removing the most restrictive part
    fn weaken_constraint(&self, constraint: &str) -> String {
        // Handle compound constraints with &&
        if constraint.contains("&&") {
            let parts: Vec<&str> = constraint.split("&&").map(|s| s.trim()).collect();
            if parts.len() > 1 {
                // Remove the last constraint (often the most restrictive)
                return parts[..parts.len() - 1].join(" && ");
            }
        }

        // Handle range constraints like "x > 0 && x < 100" -> "x > 0"
        if constraint.contains(">") && constraint.contains("<") {
            // Keep only the lower bound
            if let Some(pos) = constraint.find("&&") {
                return constraint[..pos].trim().to_string();
            }
        }

        // Handle strict inequalities -> non-strict
        if constraint.contains(">") && !constraint.contains(">=") {
            return constraint.replace(">", ">=");
        }
        if constraint.contains("<") && !constraint.contains("<=") {
            return constraint.replace("<", "<=");
        }

        // Handle != -> allow all values (remove constraint)
        if constraint.contains("!=") {
            return "true".to_string(); // Always satisfied
        }

        // Fallback: return original
        constraint.to_string()
    }

    fn generate_promote_to_checked_fix(&self, fixes: &mut List<QuickFix>, diagnostic: &Diagnostic) {
        // Promote &T to &checked T with proper type extraction
        // Try to extract the actual type from the diagnostic context
        let type_text = self.extract_reference_type_from_diagnostic(diagnostic);

        let new_text = if type_text.starts_with('&') {
            // Already a reference, insert 'checked' keyword
            type_text.replacen("&", "&checked ", 1)
        } else {
            // Add &checked prefix
            format!("&checked {}", type_text)
        };

        let mut edits = List::new();

        // Edit 1: Change reference type
        edits.push(TextEdit {
            range: diagnostic.range,
            new_text: new_text.clone(),
        });

        // Edit 2: Add SAFETY comment explaining why this is safe
        let comment_line = Position {
            line: diagnostic.range.start.line.saturating_sub(1),
            character: 0,
        };
        let indent = self.get_line_indent(diagnostic.range.start.line);
        edits.push(TextEdit {
            range: Range {
                start: comment_line,
                end: comment_line,
            },
            new_text: format!(
                "{}// SAFETY: Escape analysis proves this reference does not outlive its referent\n",
                indent
            ),
        });

        let fix = QuickFix::new(
            "Promote to &checked reference",
            QuickFixKind::PromoteToChecked,
            5,
            FixImpact::Safe,
            "Converts to statically-verified reference with 0ns overhead. \
             Requires escape analysis to prove safety.",
            edits,
        );

        fixes.push(fix);
    }

    /// Extract reference type from diagnostic using AST analysis
    ///
    /// Performs accurate extraction of reference types by:
    /// 1. First trying to get the code text at the diagnostic range
    /// 2. Analyzing the AST to find the exact type at that location
    /// 3. Handling various reference forms: &T, &mut T, &checked T, &unsafe T
    fn extract_reference_type_from_diagnostic(&self, diagnostic: &Diagnostic) -> String {
        // First, try to read the actual code at the diagnostic range.
        // Stale or malformed diagnostic ranges can land start/end
        // inside a multi-byte UTF-8 sequence; clamp both DOWN to char
        // boundaries via the shared verum_common primitive.
        if let Some(line) = self.document.get_line(diagnostic.range.start.line) {
            let start = verum_common::text_utf8::clamp_to_char_boundary(
                line,
                diagnostic.range.start.character as usize,
            );
            let end = verum_common::text_utf8::clamp_to_char_boundary(
                line,
                diagnostic.range.end.character as usize,
            );
            if start < end {
                let code_fragment = &line[start..end];
                if code_fragment.starts_with('&') {
                    return code_fragment.to_string();
                }
            }
        }

        // Try AST-based extraction from the document's parsed module
        if let Some(ref module) = self.document.module {
            if let Some(ref_type) = self.find_reference_type_at_position(
                module,
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            ) {
                return ref_type;
            }
        }

        // Try to extract from diagnostic message
        if let Some(ref_type) = self.extract_reference_from_message(&diagnostic.message) {
            return ref_type;
        }

        // Fallback to generic reference
        "&T".to_string()
    }

    /// Find reference type at a specific position in the AST
    fn find_reference_type_at_position(
        &self,
        module: &verum_ast::Module,
        line: u32,
        character: u32,
    ) -> Option<String> {
        use verum_ast::ItemKind;

        // Convert line/character to byte offset
        let target_offset = self.position_to_offset(line, character);

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                // Check if position is within function
                if func.span.start <= target_offset && target_offset <= func.span.end {
                    // Check function parameters
                    for param in &func.params {
                        if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind
                        {
                            if ty.span.start <= target_offset && target_offset <= ty.span.end {
                                if let Some(ref_str) = self.format_reference_type(ty) {
                                    return Some(ref_str);
                                }
                            }
                        }
                    }

                    // Check return type
                    if let Some(ret_ty) = &func.return_type {
                        if ret_ty.span.start <= target_offset && target_offset <= ret_ty.span.end {
                            if let Some(ref_str) = self.format_reference_type(ret_ty) {
                                return Some(ref_str);
                            }
                        }
                    }

                    // Check function body for local variable types
                    if let Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            if let Some(ref_str) =
                                self.find_reference_in_block(block, target_offset)
                            {
                                return Some(ref_str);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Format a reference type from the AST
    fn format_reference_type(&self, ty: &verum_ast::Type) -> Option<String> {
        use verum_ast::ty::TypeKind;

        match &ty.kind {
            TypeKind::Reference { mutable, inner } => {
                let inner_str = self.format_inner_type(inner);
                if *mutable {
                    Some(format!("&mut {}", inner_str))
                } else {
                    Some(format!("&{}", inner_str))
                }
            }
            TypeKind::CheckedReference { mutable, inner } => {
                let inner_str = self.format_inner_type(inner);
                if *mutable {
                    Some(format!("&checked mut {}", inner_str))
                } else {
                    Some(format!("&checked {}", inner_str))
                }
            }
            TypeKind::UnsafeReference { mutable, inner } => {
                let inner_str = self.format_inner_type(inner);
                if *mutable {
                    Some(format!("&unsafe mut {}", inner_str))
                } else {
                    Some(format!("&unsafe {}", inner_str))
                }
            }
            TypeKind::Pointer { mutable, inner } => {
                let inner_str = self.format_inner_type(inner);
                if *mutable {
                    Some(format!("*mut {}", inner_str))
                } else {
                    Some(format!("*const {}", inner_str))
                }
            }
            _ => None,
        }
    }

    /// Format the inner type of a reference
    fn format_inner_type(&self, ty: &verum_ast::Type) -> String {
        use verum_ast::ty::TypeKind;

        if let Some(name) = ty.kind.primitive_name() {
            return name.to_string();
        }
        match &ty.kind {
            TypeKind::Path(path) => path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("::"),
            TypeKind::Tuple(types) => {
                let inner: Vec<String> = types.iter().map(|t| self.format_inner_type(t)).collect();
                format!("({})", inner.join(", "))
            }
            TypeKind::Generic { base, args } => {
                let base_str = self.format_inner_type(base);
                let args_str: Vec<String> = args
                    .iter()
                    .filter_map(|arg| match arg {
                        verum_ast::ty::GenericArg::Type(t) => Some(self.format_inner_type(t)),
                        _ => None,
                    })
                    .collect();
                format!("{}<{}>", base_str, args_str.join(", "))
            }
            TypeKind::Refined { base, .. } => self.format_inner_type(base),
            _ => "T".to_string(),
        }
    }

    /// Find reference type in a block
    fn find_reference_in_block(
        &self,
        block: &verum_ast::expr::Block,
        target_offset: u32,
    ) -> Option<String> {
        for stmt in &block.stmts {
            if let verum_ast::StmtKind::Let { ty, .. } = &stmt.kind {
                if let Some(type_ann) = ty {
                    if type_ann.span.start <= target_offset && target_offset <= type_ann.span.end {
                        return self.format_reference_type(type_ann);
                    }
                }
            }
        }
        None
    }

    /// Extract reference type from diagnostic message
    fn extract_reference_from_message(&self, message: &str) -> Option<String> {
        // Common patterns in refinement error messages
        let patterns = [
            ("reference `", "`"),
            ("type `&", "`"),
            ("type `&mut ", "`"),
            ("type `&checked ", "`"),
            ("type `&unsafe ", "`"),
        ];

        for (start_pattern, end_pattern) in patterns {
            if let Some(start_idx) = message.find(start_pattern) {
                let after_start = &message[start_idx + start_pattern.len()..];
                if let Some(end_idx) = after_start.find(end_pattern) {
                    let extracted = &after_start[..end_idx];
                    if start_pattern.contains("&") {
                        return Some(format!(
                            "&{}",
                            start_pattern.strip_prefix("type `").unwrap_or(extracted)
                        ));
                    }
                    if extracted.starts_with('&') {
                        return Some(extracted.to_string());
                    }
                }
            }
        }

        None
    }

    /// Convert line/character position to byte offset
    fn position_to_offset(&self, line: u32, character: u32) -> u32 {
        let mut offset = 0u32;
        for (line_num, line_text) in self.document.text.lines().enumerate() {
            if line_num == line as usize {
                return offset + character;
            }
            offset += line_text.len() as u32 + 1; // +1 for newline
        }
        offset
    }

    // ==================== Helper Methods ====================

    fn is_reference_related(&self, constraint: &str) -> bool {
        constraint.contains("&")
            || constraint.contains("reference")
            || constraint.contains("borrow")
    }

    /// Calculate the insertion point for a new statement (beginning of line)
    fn find_insertion_point(&self, position: Position) -> Position {
        Position {
            line: position.line,
            character: 0,
        }
    }

    /// Generate a type signature change edit for function returns
    fn calculate_type_signature_change(
        &self,
        diagnostic: &Diagnostic,
        old_type: &str,
        new_type: &str,
    ) -> TextEdit {
        // This would ideally parse the function signature and find the return type
        // For now, we create a simple edit at the diagnostic range
        TextEdit {
            range: diagnostic.range,
            new_text: format!("{} -> {}", old_type, new_type),
        }
    }

    /// Generate code for a specific failure category with context
    fn generate_code_for_category(
        &self,
        category: FailureCategory,
        diagnostic: &Diagnostic,
        var_name: &str,
    ) -> List<TextEdit> {
        let mut edits = List::new();

        match category {
            FailureCategory::DivisionByZero => {
                // Generate runtime check for division
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: format!("if {} == 0 {{ return Err(DivisionByZero); }}", var_name),
                });
            }
            FailureCategory::IndexOutOfBounds => {
                // Generate bounds check
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: format!(
                        "if {} >= data.len() {{ return Err(IndexOutOfBounds); }}",
                        var_name
                    ),
                });
            }
            FailureCategory::NegativeValue => {
                // Generate positive value check
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: format!("if {} < 0 {{ return Err(NegativeValue); }}", var_name),
                });
            }
            FailureCategory::ArithmeticOverflow => {
                // Use checked arithmetic
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: "checked_add".to_string(),
                });
            }
            FailureCategory::NullDereference => {
                // Add null check
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: format!("if {}.is_none() {{ return Err(NullPointer); }}", var_name),
                });
            }
            FailureCategory::Other => {
                // Generate intelligent validation based on constraint analysis
                let validation_code = self.generate_intelligent_validation(diagnostic, var_name);
                edits.push(TextEdit {
                    range: diagnostic.range,
                    new_text: validation_code,
                });
            }
        }

        edits
    }

    fn extract_collection_index(
        &self,
        constraint: &str,
        counterexample: Maybe<&CounterExample>,
    ) -> (String, String) {
        // Try to extract from counterexample first
        if let Maybe::Some(ce) = counterexample {
            let vars: List<_> = ce.assignments.keys().collect();
            if vars.len() >= 2 {
                return (vars[0].to_string(), vars[1].to_string());
            }
        }

        // Fallback to parsing constraint
        if constraint.contains("len(")
            && let Some(start) = constraint.find("len(")
            && let Some(end) = constraint[start..].find(')')
        {
            let collection = &constraint[start + 4..start + end];
            return (collection.to_string(), "index".to_string());
        }

        ("data".to_string(), "index".to_string())
    }

    fn get_line_indent(&self, line: u32) -> String {
        if let Some(line_text) = self.document.get_line(line) {
            let indent_count = line_text.chars().take_while(|c| c.is_whitespace()).count();
            " ".repeat(indent_count)
        } else {
            "    ".to_string()
        }
    }

    /// Generate intelligent validation code based on constraint pattern analysis
    fn generate_intelligent_validation(&self, diagnostic: &Diagnostic, var_name: &str) -> String {
        // Extract the actual code at the diagnostic location to understand context
        let context = self.extract_code_context(diagnostic);

        // Analyze the context to determine what kind of validation is needed

        self.infer_validation_from_context(&context, var_name)
    }

    /// Extract code context from diagnostic location
    fn extract_code_context(&self, diagnostic: &Diagnostic) -> String {
        if let Some(line) = self.document.get_line(diagnostic.range.start.line) {
            // Get the full line for context
            let line_str = line.trim();

            // Also try to get surrounding lines for better context
            let mut context = String::new();

            // Previous line
            if diagnostic.range.start.line > 0
                && let Some(prev_line) = self.document.get_line(diagnostic.range.start.line - 1)
            {
                context.push_str(prev_line.trim());
                context.push(' ');
            }

            // Current line
            context.push_str(line_str);

            // Next line
            if let Some(next_line) = self.document.get_line(diagnostic.range.start.line + 1) {
                context.push(' ');
                context.push_str(next_line.trim());
            }

            context
        } else {
            String::new()
        }
    }

    /// Infer appropriate validation code from context
    fn infer_validation_from_context(&self, context: &str, var_name: &str) -> String {
        // Pattern 1: Collection operations (len, size, contains, etc.)
        // Check BEFORE range constraints since patterns like "len() > 0" contain ">"
        if context.contains("len") || context.contains("size") || context.contains("count") {
            return self.generate_collection_validation(context, var_name);
        }

        // Pattern 2: Range constraints (comparisons)
        if context.contains(">")
            || context.contains("<")
            || context.contains(">=")
            || context.contains("<=")
        {
            return self.generate_range_validation(context, var_name);
        }

        // Pattern 3: Equality/inequality checks
        if context.contains("==") || context.contains("!=") {
            return self.generate_equality_validation(context, var_name);
        }

        // Pattern 4: Boolean properties (is_some, is_none, is_empty, etc.)
        if context.contains("is_some")
            || context.contains("is_none")
            || context.contains("is_empty")
            || context.contains("is_valid")
        {
            return self.generate_boolean_property_validation(context, var_name);
        }

        // Pattern 5: Arithmetic operations
        if context.contains("+") || context.contains("*") || context.contains("-") {
            return format!(
                "if !{}.is_valid() {{ return Err(ValidationError::InvalidValue); }}",
                var_name
            );
        }

        // Pattern 6: Type-specific validation
        if context.contains("String") || context.contains("Text") {
            return format!(
                "if {}.is_empty() {{ return Err(ValidationError::EmptyString); }}",
                var_name
            );
        }

        if context.contains("List") || context.contains("Array") || context.contains("Vec") {
            return format!(
                "if {}.is_empty() {{ return Err(ValidationError::EmptyCollection); }}",
                var_name
            );
        }

        // Default: Generic constraint validation
        format!(
            "if !validate_constraint({}) {{ return Err(ValidationError::ConstraintViolation); }}",
            var_name
        )
    }

    /// Generate range validation code
    fn generate_range_validation(&self, context: &str, var_name: &str) -> String {
        // Try to extract the comparison operators and values
        let has_lower = context.contains(">") || context.contains(">=");
        let has_upper = context.contains("<") || context.contains("<=");

        if has_lower && has_upper {
            // Range constraint
            format!(
                "if {} < MIN_VALUE || {} > MAX_VALUE {{ return Err(ValidationError::OutOfRange); }}",
                var_name, var_name
            )
        } else if has_lower {
            // Lower bound only
            if context.contains(">=") {
                format!(
                    "if {} < 0 {{ return Err(ValidationError::NegativeValue); }}",
                    var_name
                )
            } else {
                format!(
                    "if {} <= 0 {{ return Err(ValidationError::NonPositive); }}",
                    var_name
                )
            }
        } else if has_upper {
            // Upper bound only
            format!(
                "if {} >= MAX_VALUE {{ return Err(ValidationError::ValueTooLarge); }}",
                var_name
            )
        } else {
            // Fallback
            format!(
                "if !is_in_valid_range({}) {{ return Err(ValidationError::OutOfRange); }}",
                var_name
            )
        }
    }

    /// Generate equality/inequality validation code
    fn generate_equality_validation(&self, context: &str, var_name: &str) -> String {
        if context.contains("!= 0") || context.contains("== 0") {
            format!(
                "if {} == 0 {{ return Err(ValidationError::ZeroNotAllowed); }}",
                var_name
            )
        } else if context.contains("!= null") || context.contains("== null") {
            format!(
                "if {}.is_none() {{ return Err(ValidationError::NullValue); }}",
                var_name
            )
        } else {
            format!(
                "if {} == INVALID_VALUE {{ return Err(ValidationError::InvalidValue); }}",
                var_name
            )
        }
    }

    /// Generate collection validation code
    fn generate_collection_validation(&self, context: &str, var_name: &str) -> String {
        if context.contains("len()") && (context.contains("> 0") || context.contains("!= 0")) {
            format!(
                "if {}.is_empty() {{ return Err(ValidationError::EmptyCollection); }}",
                var_name
            )
        } else if context.contains("len()") && context.contains("<") {
            format!(
                "if {}.len() >= MAX_SIZE {{ return Err(ValidationError::CollectionTooLarge); }}",
                var_name
            )
        } else if context.contains("contains") {
            format!(
                "if !{}.contains(&required_element) {{ return Err(ValidationError::MissingElement); }}",
                var_name
            )
        } else {
            format!(
                "if !validate_collection({}) {{ return Err(ValidationError::InvalidCollection); }}",
                var_name
            )
        }
    }

    /// Generate boolean property validation code
    fn generate_boolean_property_validation(&self, context: &str, var_name: &str) -> String {
        if context.contains("is_some()") {
            format!(
                "if {}.is_none() {{ return Err(ValidationError::NoneValue); }}",
                var_name
            )
        } else if context.contains("is_none()") {
            format!(
                "if {}.is_some() {{ return Err(ValidationError::UnexpectedValue); }}",
                var_name
            )
        } else if context.contains("is_empty()") {
            format!(
                "if !{}.is_empty() {{ return Err(ValidationError::NotEmpty); }}",
                var_name
            )
        } else if context.contains("is_valid()") {
            format!(
                "if !{}.is_valid() {{ return Err(ValidationError::InvalidState); }}",
                var_name
            )
        } else {
            format!(
                "if !{}.check_property() {{ return Err(ValidationError::PropertyViolation); }}",
                var_name
            )
        }
    }
}

// ==================== Categorization ====================

/// Categorize a refinement failure based on counterexample and constraint
fn categorize_failure(counterexample: &CounterExample, _constraint: &str) -> FailureCategory {
    use verum_smt::counterexample::CounterExampleCategorizer;

    // Use SMT categorizer for pattern matching
    CounterExampleCategorizer::categorize(counterexample)
}

// ==================== Public API ====================

/// Generate quick fixes for a refinement violation diagnostic
pub fn generate_refinement_quick_fixes(
    document: &DocumentState,
    uri: &Url,
    diagnostic: &Diagnostic,
    counterexample: Maybe<&CounterExample>,
    violated_constraint: &str,
) -> List<CodeAction> {
    let generator = QuickFixGenerator::new(document, uri);

    let fixes = generator.generate_for_refinement_violation(
        diagnostic,
        counterexample,
        violated_constraint,
    );

    // Convert to CodeActions
    fixes
        .into_iter()
        .map(|fix| fix.to_code_action(uri, vec![diagnostic.clone()]))
        .collect()
}

/// Generate quick fixes for all diagnostics in a document
pub fn generate_all_quick_fixes(
    document: &DocumentState,
    uri: &Url,
    diagnostics: &[Diagnostic],
) -> std::collections::HashMap<String, List<CodeAction>> {
    let mut result = std::collections::HashMap::new();

    for diagnostic in diagnostics {
        // Check if this is a refinement violation
        if is_refinement_diagnostic(diagnostic) {
            // Extract constraint from diagnostic message
            let constraint = extract_constraint_from_message(&diagnostic.message);

            // Generate fixes (no counterexample available here)
            let fixes = generate_refinement_quick_fixes(
                document,
                uri,
                diagnostic,
                Maybe::None,
                &constraint,
            );

            // Use range as a string key for HashMap
            let key = format!(
                "{}:{}-{}:{}",
                diagnostic.range.start.line,
                diagnostic.range.start.character,
                diagnostic.range.end.line,
                diagnostic.range.end.character
            );
            result.insert(key, fixes);
        }
    }

    result
}

/// Check if a diagnostic is a refinement violation
fn is_refinement_diagnostic(diagnostic: &Diagnostic) -> bool {
    diagnostic.message.contains("refinement")
        || diagnostic.message.contains("constraint")
        || diagnostic
            .code
            .as_ref()
            .map(|c| matches!(c, NumberOrString::String(s) if s.starts_with("E03")))
            .unwrap_or(false)
}

/// Extract the violated constraint from a diagnostic message
pub fn extract_constraint_from_message(message: &str) -> String {
    // Look for pattern: "constraint 'X' violated"
    if let Some(start) = message.find('\'')
        && let Some(end) = message[start + 1..].find('\'')
    {
        return message[start + 1..start + 1 + end].to_string();
    }

    // Look for pattern: "violates: X"
    if let Some(start) = message.find("violates:") {
        let rest = message[start + 9..].trim();
        if let Some(end) = rest.find('\n') {
            return rest[..end].to_string();
        }
        return rest.to_string();
    }

    // Fallback
    "constraint".to_string()
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_smt::{CounterExample, CounterExampleValue};
    use verum_common::{Map, Text};

    #[test]
    fn test_categorize_division_by_zero() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("y"), CounterExampleValue::Int(0));

        let ce = CounterExample::new(assignments, Text::from("y != 0"));
        let category = categorize_failure(&ce, "y != 0");

        assert_eq!(category, FailureCategory::DivisionByZero);
    }

    #[test]
    fn test_categorize_negative_value() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("x"), CounterExampleValue::Int(-5));

        let ce = CounterExample::new(assignments, Text::from("x > 0"));
        let category = categorize_failure(&ce, "x > 0");

        assert_eq!(category, FailureCategory::NegativeValue);
    }

    #[test]
    fn test_extract_constraint_from_message() {
        let message = "Refinement violation: constraint 'x != 0' violated";
        let constraint = extract_constraint_from_message(message);
        assert_eq!(constraint, "x != 0");

        let message2 = "Value violates: x > 0\nCounterexample: x = -5";
        let constraint2 = extract_constraint_from_message(message2);
        assert_eq!(constraint2, "x > 0");
    }

    #[test]
    fn test_is_refinement_diagnostic() {
        let diag = Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E0304".to_string())),
            source: Some("verum".to_string()),
            message: "Refinement violation".to_string(),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        };

        assert!(is_refinement_diagnostic(&diag));
    }

    #[test]
    fn test_quick_fix_priority_ordering() {
        let mut fixes = List::new();

        fixes.push(QuickFix::new(
            "Fix 3",
            QuickFixKind::Assertion,
            3,
            FixImpact::Safe,
            "Description",
            List::new(),
        ));

        fixes.push(QuickFix::new(
            "Fix 1",
            QuickFixKind::RuntimeCheck,
            1,
            FixImpact::Safe,
            "Description",
            List::new(),
        ));

        fixes.push(QuickFix::new(
            "Fix 2",
            QuickFixKind::InlineRefinement,
            2,
            FixImpact::Breaking,
            "Description",
            List::new(),
        ));

        fixes.sort_by_key(|f| f.priority);

        assert_eq!(fixes[0].priority, 1);
        assert_eq!(fixes[1].priority, 2);
        assert_eq!(fixes[2].priority, 3);
    }

    #[test]
    fn test_intelligent_validation_range_constraint() {
        use crate::document::DocumentState;

        let source = "fn test(x: Int) -> Int {\n    if x > 0 && x < 100 { x } else { 0 }\n}";
        let doc = DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(0));
        let uri = Url::parse("file:///test.vr").unwrap();

        let generator = QuickFixGenerator::new(&doc, &uri);

        // Test range validation generation
        let context = "x > 0 && x < 100";
        let validation = generator.infer_validation_from_context(context, "x");

        assert!(validation.contains("MIN_VALUE"));
        assert!(validation.contains("MAX_VALUE"));
        assert!(validation.contains("OutOfRange"));
    }

    #[test]
    fn test_intelligent_validation_equality_constraint() {
        use crate::document::DocumentState;

        let source = "fn divide(x: Int, y: Int) -> Int {\n    x / y\n}";
        let doc = DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(0));
        let uri = Url::parse("file:///test.vr").unwrap();

        let generator = QuickFixGenerator::new(&doc, &uri);

        // Test zero check validation
        let context = "y != 0";
        let validation = generator.infer_validation_from_context(context, "y");

        assert!(validation.contains("== 0"));
        assert!(validation.contains("ZeroNotAllowed") || validation.contains("Err"));
    }

    #[test]
    fn test_intelligent_validation_collection_constraint() {
        use crate::document::DocumentState;

        let source = "fn first(arr: List<Int>) -> Int {\n    arr.len() > 0\n}";
        let doc = DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(0));
        let uri = Url::parse("file:///test.vr").unwrap();

        let generator = QuickFixGenerator::new(&doc, &uri);

        // Test collection validation
        let context = "arr.len() > 0";
        let validation = generator.infer_validation_from_context(context, "arr");

        assert!(validation.contains("is_empty") || validation.contains("EmptyCollection"));
    }

    #[test]
    fn test_intelligent_validation_boolean_property() {
        use crate::document::DocumentState;

        let source = "fn unwrap(opt: Maybe<Int>) -> Int {\n    opt.is_some()\n}";
        let doc = DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(0));
        let uri = Url::parse("file:///test.vr").unwrap();

        let generator = QuickFixGenerator::new(&doc, &uri);

        // Test boolean property validation
        let context = "opt.is_some()";
        let validation = generator.infer_validation_from_context(context, "opt");

        assert!(validation.contains("is_none"));
        assert!(validation.contains("NoneValue") || validation.contains("Err"));
    }

    #[test]
    fn test_no_todo_in_other_category() {
        use crate::document::DocumentState;

        let source = "fn test(x: Int) -> Int {\n    x + 42\n}";
        let doc = DocumentState::new(source.to_string(), 1, verum_ast::FileId::new(0));
        let uri = Url::parse("file:///test.vr").unwrap();

        let generator = QuickFixGenerator::new(&doc, &uri);
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: 1,
                    character: 4,
                },
                end: Position {
                    line: 1,
                    character: 10,
                },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            source: Some("verum".to_string()),
            message: "Constraint violation".to_string(),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        };

        // Generate validation for "Other" category
        let validation = generator.generate_intelligent_validation(&diagnostic, "x");

        // Should NOT contain TODO
        assert!(!validation.contains("TODO"));
        // Should contain actual validation logic
        assert!(
            validation.contains("validate")
                || validation.contains("Err")
                || validation.contains("if")
        );
    }
}
