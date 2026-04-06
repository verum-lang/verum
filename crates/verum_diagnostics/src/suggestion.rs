//! Suggestion system for providing actionable fixes to diagnostics.
//!
//! This module provides types for representing fix suggestions with code snippets,
//! applicability levels, and rich descriptions.

use crate::Span;
use serde::{Deserialize, Serialize};
use verum_common::{List, Text};

/// Applicability level of a suggestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Applicability {
    /// This fix is the recommended approach
    Recommended,
    /// This fix is a valid alternative
    Alternative,
    /// This fix may work but requires manual verification
    MaybeIncorrect,
    /// This fix is for demonstration only
    HasPlaceholders,
}

impl Applicability {
    /// Check if this suggestion can be automatically applied
    pub fn is_safe_to_apply(&self) -> bool {
        matches!(
            self,
            Applicability::Recommended | Applicability::Alternative
        )
    }
}

/// A code snippet showing example code
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSnippet {
    /// The language of the snippet (usually "verum")
    pub language: Text,
    /// The code content
    pub code: Text,
    /// Optional span where this should be applied
    pub span: Option<Span>,
}

impl CodeSnippet {
    /// Create a new code snippet
    pub fn new(code: impl Into<Text>) -> Self {
        Self {
            language: "verum".into(),
            code: code.into(),
            span: None,
        }
    }

    /// Create a code snippet with a specific language
    pub fn with_language(code: impl Into<Text>, language: impl Into<Text>) -> Self {
        Self {
            language: language.into(),
            code: code.into(),
            span: None,
        }
    }

    /// Create a code snippet with a span for application
    pub fn with_span(code: impl Into<Text>, span: Span) -> Self {
        Self {
            language: "verum".into(),
            code: code.into(),
            span: Some(span),
        }
    }
}

/// A suggestion for fixing a diagnostic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Title/summary of the suggestion
    pub title: Text,
    /// Detailed description
    pub description: Option<Text>,
    /// Code snippet showing the fix
    pub snippet: Option<CodeSnippet>,
    /// Applicability level
    pub applicability: Applicability,
    /// Multiple code snippets for multi-part suggestions
    pub snippets: List<CodeSnippet>,
}

impl Suggestion {
    /// Create a new suggestion
    pub fn new(title: impl Into<Text>) -> Self {
        Self {
            title: title.into(),
            description: None,
            snippet: None,
            applicability: Applicability::Alternative,
            snippets: List::new(),
        }
    }

    /// Get the title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Get the description
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Get the primary snippet
    pub fn snippet(&self) -> Option<&CodeSnippet> {
        self.snippet.as_ref()
    }

    /// Get all snippets
    pub fn all_snippets(&self) -> impl Iterator<Item = &CodeSnippet> {
        self.snippet.iter().chain(self.snippets.iter())
    }

    /// Get the applicability
    pub fn applicability(&self) -> Applicability {
        self.applicability
    }

    /// Get the code from the primary snippet (convenience method)
    pub fn code(&self) -> &str {
        self.snippet.as_ref().map(|s| s.code.as_str()).unwrap_or("")
    }
}

/// Builder for constructing suggestions
pub struct SuggestionBuilder {
    title: Text,
    description: Option<Text>,
    snippet: Option<CodeSnippet>,
    applicability: Applicability,
    snippets: List<CodeSnippet>,
}

impl SuggestionBuilder {
    /// Create a new suggestion builder
    pub fn new(title: impl Into<Text>) -> Self {
        Self {
            title: title.into(),
            description: None,
            snippet: None,
            applicability: Applicability::Alternative,
            snippets: List::new(),
        }
    }

    /// Set the description
    pub fn description(mut self, description: impl Into<Text>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the code snippet
    pub fn snippet(mut self, snippet: CodeSnippet) -> Self {
        self.snippet = Some(snippet);
        self
    }

    /// Set the code snippet from a string
    pub fn code(mut self, code: impl Into<Text>) -> Self {
        self.snippet = Some(CodeSnippet::new(code));
        self
    }

    /// Add an additional snippet
    pub fn add_snippet(mut self, snippet: CodeSnippet) -> Self {
        self.snippets.push(snippet);
        self
    }

    /// Set the applicability level
    pub fn applicability(mut self, applicability: Applicability) -> Self {
        self.applicability = applicability;
        self
    }

    /// Mark as recommended
    pub fn recommended(mut self) -> Self {
        self.applicability = Applicability::Recommended;
        self
    }

    /// Mark as alternative
    pub fn alternative(mut self) -> Self {
        self.applicability = Applicability::Alternative;
        self
    }

    /// Build the suggestion
    pub fn build(self) -> Suggestion {
        Suggestion {
            title: self.title,
            description: self.description,
            snippet: self.snippet,
            applicability: self.applicability,
            snippets: self.snippets,
        }
    }
}

/// Trait for types that can provide suggestions
pub trait Suggestible {
    /// Get suggestions for fixing this issue
    fn suggestions(&self) -> List<Suggestion>;
}

/// Common suggestion templates for Verum
pub mod templates {
    use super::*;

    /// Suggest adding a refinement type constraint
    pub fn add_refinement_constraint(param_name: &str, constraint: &str) -> Suggestion {
        SuggestionBuilder::new("Add refinement type constraint")
            .description(format!(
                "Add a refinement constraint to parameter '{}' to ensure the condition at compile time",
                param_name
            ))
            .code(format!("{}: Type{{{}}}", param_name, constraint))
            .recommended()
            .build()
    }

    /// Suggest using a runtime check
    pub fn runtime_check(check_expr: &str, error_handling: &str) -> Suggestion {
        SuggestionBuilder::new("Use runtime check")
            .description("Validate the condition at runtime and handle errors explicitly")
            .code(format!(
                "if {} {{\n    // safe code\n}} else {{\n    {}\n}}",
                check_expr, error_handling
            ))
            .alternative()
            .build()
    }

    /// Suggest using Option type
    pub fn use_option_type(type_name: &str) -> Suggestion {
        SuggestionBuilder::new("Return Option type")
            .description("Return None when the operation cannot succeed")
            .code(format!("-> Option<{}>", type_name))
            .alternative()
            .build()
    }

    /// Suggest using Result type
    pub fn use_result_type(type_name: &str, error_type: &str) -> Suggestion {
        SuggestionBuilder::new("Return Result type")
            .description("Return an error when the operation fails")
            .code(format!("-> Result<{}, {}>", type_name, error_type))
            .alternative()
            .build()
    }

    /// Suggest adding an assertion
    pub fn add_assertion(condition: &str, message: &str) -> Suggestion {
        SuggestionBuilder::new("Add assertion")
            .description("Add a runtime assertion that panics if violated")
            .code(format!("assert!({}, \"{}\");", condition, message))
            .applicability(Applicability::MaybeIncorrect)
            .build()
    }

    /// Suggest using a safe method
    pub fn use_safe_method(method_name: &str, explanation: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Use safe method: {}", method_name))
            .description(explanation)
            .code(format!(".{}()", method_name))
            .recommended()
            .build()
    }

    /// Suggest compile-time proof
    pub fn compile_time_proof(condition: &str) -> Suggestion {
        SuggestionBuilder::new("Add compile-time proof")
            .description("Use @verify to prove the condition at compile time")
            .code(format!("@verify {}", condition))
            .recommended()
            .build()
    }

    /// Suggest strengthening precondition
    pub fn strengthen_precondition(param: &str, constraint: &str) -> Suggestion {
        SuggestionBuilder::new("Strengthen precondition")
            .description("Add a stronger constraint to the parameter")
            .code(format!("{}: Type{{{}}}", param, constraint))
            .recommended()
            .build()
    }

    /// Suggest weakening postcondition
    pub fn weaken_postcondition(return_type: &str) -> Suggestion {
        SuggestionBuilder::new("Weaken postcondition")
            .description("Relax the return type constraint")
            .code(format!("-> {}", return_type))
            .alternative()
            .build()
    }

    // Context-specific suggestion templates.
    // Verum's context system requires all dependencies to be declared via 'using [ContextName]'
    // in function signatures. Common fixes: add missing context to 'using' clause, install
    // provider via 'provide', or pass dependency explicitly as a parameter.

    /// Suggest adding a context to function signature
    pub fn add_context_to_signature(context_name: &str, function_name: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Add '{}' to function signature", context_name))
            .description(format!(
                "Declare that '{}' requires the '{}' context",
                function_name, context_name
            ))
            .code(format!(
                "fn {}(...) -> ReturnType\n    using [{}]  // <-- add this",
                function_name, context_name
            ))
            .recommended()
            .build()
    }

    /// Suggest providing a context before function call
    pub fn provide_context_before_call(context_name: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Provide '{}' context before calling", context_name))
            .description("Install the context provider before the function call")
            .code(format!(
                "provide {} = create_{}();",
                context_name,
                context_name.to_lowercase()
            ))
            .alternative()
            .build()
    }

    /// Suggest creating a context group
    pub fn create_context_group(group_name: &str, contexts: &[Text]) -> Suggestion {
        SuggestionBuilder::new("Create a context group for reusability")
            .description("Define a context group to avoid repeating context lists")
            .code(format!("using {} = [{}];", group_name, contexts.join(", ")))
            .alternative()
            .build()
    }

    /// Suggest using an existing context group
    pub fn use_context_group(group_name: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Use context group '{}'", group_name))
            .description("Replace individual contexts with the context group")
            .code(format!("using {}", group_name))
            .alternative()
            .build()
    }

    /// Suggest implementing a context interface
    pub fn implement_context_interface(context_name: &str, interface: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Implement '{}' context", context_name))
            .description(format!(
                "Create an implementation of the '{}' interface",
                interface
            ))
            .code(format!(
                "type My{} is {} {{\n    // implement interface methods\n}}",
                context_name, interface
            ))
            .applicability(Applicability::HasPlaceholders)
            .build()
    }

    /// Suggest adding module-level context annotation
    pub fn add_module_level_context(contexts: &[Text]) -> Suggestion {
        SuggestionBuilder::new("Add module-level context annotation")
            .description("Declare contexts for all functions in this module")
            .code(format!(
                "@using([{}])\nmodule my_module {{",
                contexts.join(", ")
            ))
            .alternative()
            .build()
    }

    /// Suggest using context in async function
    pub fn use_context_in_async(context_name: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Add '{}' to async function", context_name))
            .description("Contexts work seamlessly with async functions")
            .code(format!(
                "async fn my_async_function() -> Result<T, E>\n    using [{}]",
                context_name
            ))
            .recommended()
            .build()
    }

    /// Suggest testing context with mock implementation
    pub fn mock_context_for_testing(context_name: &str) -> Suggestion {
        SuggestionBuilder::new("Use mock context for testing")
            .description("Provide a test implementation of the context")
            .code(format!(
                "#[test]\nfn test_my_function() {{\n    provide {} = Mock{}.new();\n    // test code\n}}",
                context_name, context_name
            ))
            .alternative()
            .build()
    }

    /// Suggest propagating context requirement up call chain
    pub fn propagate_context_up(context_name: &str, caller_function: &str) -> Suggestion {
        SuggestionBuilder::new("Propagate context requirement upward")
            .description(format!(
                "Add '{}' to the calling function '{}'",
                context_name, caller_function
            ))
            .code(format!(
                "fn {}(...) -> ReturnType\n    using [{}]",
                caller_function, context_name
            ))
            .alternative()
            .build()
    }

    /// Suggest using 'did you mean' for typo
    pub fn did_you_mean_context(actual: &str, suggested: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Did you mean '{}'?", suggested))
            .description(format!(
                "'{}' is not defined, but '{}' is available",
                actual, suggested
            ))
            .code(format!("using [{}]", suggested))
            .recommended()
            .build()
    }
}

/// Templates for type-related suggestions
pub struct TypeSuggestionTemplates;

impl TypeSuggestionTemplates {
    /// Suggest adding a type annotation
    pub fn add_type_annotation(var_name: &str, suggested_type: &str) -> Suggestion {
        SuggestionBuilder::new("Add explicit type annotation")
            .description(format!(
                "Explicitly annotate '{}' with type '{}'",
                var_name, suggested_type
            ))
            .code(format!("let {}: {} = ...", var_name, suggested_type))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using a type conversion
    pub fn use_type_conversion(from_type: &str, to_type: &str, conversion: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Convert {} to {}", from_type, to_type))
            .description(format!(
                "Use '{}' to convert from '{}' to '{}'",
                conversion, from_type, to_type
            ))
            .code(conversion.to_string())
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using a refinement type
    pub fn use_refinement_type(base_type: &str, constraint: &str) -> Suggestion {
        SuggestionBuilder::new("Use refinement type")
            .description(format!(
                "Create a refinement type with constraint: {}",
                constraint
            ))
            .code(format!(
                "type Refined{} is {} where {{ {} }}",
                base_type, base_type, constraint
            ))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest wrapping in Maybe
    pub fn wrap_in_maybe(value_expr: &str) -> Suggestion {
        SuggestionBuilder::new("Wrap value in Maybe")
            .description("Convert value to Maybe<T> to handle optional case")
            .code(format!("Maybe::Some({})", value_expr))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest unwrapping Maybe with default
    pub fn unwrap_with_default(maybe_expr: &str, default: &str) -> Suggestion {
        SuggestionBuilder::new("Unwrap Maybe with default")
            .description("Use default value when Maybe is None")
            .code(format!("{}.unwrap_or({})", maybe_expr, default))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using Result for fallible operation
    pub fn use_result_type(operation: &str, error_type: &str) -> Suggestion {
        SuggestionBuilder::new("Return Result for fallible operation")
            .description(format!(
                "Use Result<T, {}> to handle potential errors",
                error_type
            ))
            .code(format!(
                "fn {}() -> Result<T, {}> {{\n    // ...\n}}",
                operation, error_type
            ))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest adding generic type parameter
    pub fn add_generic_parameter(
        function: &str,
        param: &str,
        constraint: Option<&str>,
    ) -> Suggestion {
        let constraint_str = constraint.map(|c| format!(": {}", c)).unwrap_or_default();

        SuggestionBuilder::new(format!("Add generic parameter '{}'", param))
            .description("Make the function generic over this type")
            .code(format!("fn {}<{}{}>(..)", function, param, constraint_str))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest using associated type
    pub fn use_associated_type(trait_name: &str, assoc_type: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Use associated type {}", assoc_type))
            .description(format!(
                "Reference the associated type from trait '{}'",
                trait_name
            ))
            .code(format!("<Self as {}>::{}", trait_name, assoc_type))
            .applicability(Applicability::Alternative)
            .build()
    }
}

/// Templates for error handling suggestions
pub struct ErrorHandlingSuggestionTemplates;

impl ErrorHandlingSuggestionTemplates {
    /// Suggest using ? operator
    pub fn use_question_mark_operator(expr: &str) -> Suggestion {
        SuggestionBuilder::new("Propagate error with ? operator")
            .description("Use ? to propagate the error to the caller")
            .code(format!("{}?", expr))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using match for Result
    pub fn match_result(result_expr: &str) -> Suggestion {
        SuggestionBuilder::new("Handle Result with match")
            .description("Pattern match on the Result to handle both cases")
            .code(format!(
                "match {} {{\n    Ok(value) => {{ /* use value */ }}\n    Err(e) => {{ /* handle error */ }}\n}}",
                result_expr
            ))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using if let for optional value
    pub fn use_if_let(maybe_expr: &str, var_name: &str) -> Suggestion {
        SuggestionBuilder::new("Use if let for optional value")
            .description("Conditionally execute code when value is present")
            .code(format!(
                "if let Maybe::Some({}) = {} {{\n    // use {}\n}}",
                var_name, maybe_expr, var_name
            ))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest converting error type
    pub fn convert_error_type(from_err: &str, to_err: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Convert {} to {}", from_err, to_err))
            .description("Use map_err to convert the error type")
            .code(format!(".map_err(|e| {}::from(e))", to_err))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest adding error context
    pub fn add_error_context(context_msg: &str) -> Suggestion {
        SuggestionBuilder::new("Add context to error")
            .description("Wrap error with additional context for better debugging")
            .code(format!(".context(\"{}\")?", context_msg))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest using unwrap_or_else
    pub fn use_unwrap_or_else(result_expr: &str, fallback: &str) -> Suggestion {
        SuggestionBuilder::new("Use unwrap_or_else for fallback")
            .description("Provide a fallback function when Result is Err")
            .code(format!("{}.unwrap_or_else(|_| {})", result_expr, fallback))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest handling @must_handle Result
    pub fn handle_must_handle_result(var_name: &str, error_type: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Handle @must_handle Result<_, {}>", error_type))
            .description("This Result must be explicitly handled due to @must_handle annotation")
            .code(format!(
                "match {} {{\n    Ok(value) => {{ /* success case */ }}\n    Err(e) => {{ /* handle {} */ }}\n}}",
                var_name, error_type
            ))
            .applicability(Applicability::Recommended)
            .build()
    }
}

/// Templates for syntax fix suggestions
pub struct SyntaxSuggestionTemplates;

impl SyntaxSuggestionTemplates {
    /// Suggest adding missing delimiter
    pub fn add_missing_delimiter(delimiter: &str, position: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Add missing '{}'", delimiter))
            .description(format!("Insert '{}' {}", delimiter, position))
            .code(delimiter.to_string())
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest removing extra token
    pub fn remove_extra_token(token: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Remove extra '{}'", token))
            .description(format!("The '{}' token is unexpected here", token))
            .code(String::new()) // Empty replacement
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest replacing token
    pub fn replace_token(from: &str, to: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Replace '{}' with '{}'", from, to))
            .description(format!("Use '{}' instead of '{}'", to, from))
            .code(to.to_string())
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest adding function signature element
    pub fn add_function_element(element: &str, function: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Add {} to function '{}'", element, function))
            .description(format!(
                "The function '{}' is missing {}",
                function, element
            ))
            .code(format!("fn {}() -> ReturnType {{ }}", function))
            .applicability(Applicability::HasPlaceholders)
            .build()
    }

    /// Suggest fixing indentation
    pub fn fix_indentation(expected_indent: usize) -> Suggestion {
        SuggestionBuilder::new("Fix indentation")
            .description(format!(
                "Expected {} spaces of indentation",
                expected_indent
            ))
            .code(" ".repeat(expected_indent))
            .applicability(Applicability::MaybeIncorrect)
            .build()
    }

    /// Suggest adding module declaration
    pub fn add_module_declaration(module_name: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Add module declaration for '{}'", module_name))
            .description("Declare the module to make it accessible")
            .code(format!("mod {};", module_name))
            .applicability(Applicability::Recommended)
            .build()
    }

    /// Suggest importing symbol
    pub fn import_symbol(symbol: &str, module: &str) -> Suggestion {
        SuggestionBuilder::new(format!("Import '{}' from '{}'", symbol, module))
            .description(format!(
                "Add import statement to use '{}' from '{}'",
                symbol, module
            ))
            .code(format!("using {}::{};", module, symbol))
            .applicability(Applicability::Recommended)
            .build()
    }
}

/// Templates for performance-related suggestions
pub struct PerformanceSuggestionTemplates;

impl PerformanceSuggestionTemplates {
    /// Suggest using iterator instead of collecting
    pub fn use_iterator(collection: &str) -> Suggestion {
        SuggestionBuilder::new("Use iterator instead of collecting")
            .description("Avoid intermediate collection allocation")
            .code(format!("{}.iter()", collection))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest using with_capacity for collection
    pub fn use_with_capacity(collection_type: &str, hint: &str) -> Suggestion {
        SuggestionBuilder::new("Pre-allocate collection capacity")
            .description("Avoid reallocations by specifying initial capacity")
            .code(format!("{}::with_capacity({})", collection_type, hint))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest using borrow instead of clone
    pub fn use_borrow_instead_of_clone(expr: &str) -> Suggestion {
        SuggestionBuilder::new("Use borrow instead of clone")
            .description("Avoid unnecessary clone by borrowing the value")
            .code(format!("&{}", expr))
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest moving value instead of cloning
    pub fn move_instead_of_clone(expr: &str) -> Suggestion {
        SuggestionBuilder::new("Move value instead of cloning")
            .description("Transfer ownership instead of copying")
            .code(expr.to_string())
            .applicability(Applicability::Alternative)
            .build()
    }

    /// Suggest using lazy evaluation
    pub fn use_lazy_evaluation(expr: &str) -> Suggestion {
        SuggestionBuilder::new("Use lazy evaluation")
            .description("Defer computation until the value is needed")
            .code(format!("lazy {{ {} }}", expr))
            .applicability(Applicability::Alternative)
            .build()
    }
}
