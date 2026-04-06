//! Quote Macro Implementation - quote! and unquote! procedural macros
//!
//! This module provides the implementation of the quote! and unquote! macros
//! for generating Verum code at compile-time within meta functions.
//!
//! Procedural macro support: quote! macro for AST construction in meta functions.
//! Meta functions use quote! to generate Verum code at compile-time.
//!
//! # Overview
//!
//! The quote! macro allows meta functions to generate Verum code using
//! quasi-quotation syntax with interpolation support:
//!
//! ```verum
//! meta fn generate_getter(field: Text) -> TokenStream {
//!     quote! {
//!         fn get_#field(&self) -> &Self::#field {
//!             &self.#field
//!         }
//!     }
//! }
//! ```
//!
//! # Interpolation Syntax
//!
//! - `#ident` - Single interpolation (substitutes a variable)
//! - `#(#items),*` - Repetition with comma separator
//! - `#(#items)*` - Repetition without separator
//! - `#(#items);+` - Repetition with semicolon (at least one)
//!
//! # Implementation Strategy
//!
//! Since Rust procedural macros cannot be used directly in the compiler,
//! this module provides runtime equivalents that can be called from
//! meta function execution.

use crate::meta::{ConstValue, MetaError};
use crate::quote::{MetaContext, Quote, QuoteError, ToTokens, TokenStream};
use verum_ast::{Expr, Span, ty::Type};
use verum_common::{List, Text as QuoteText};

/// Execute a quote! invocation at compile time
///
/// This function is called by the meta function evaluator when it encounters
/// a quote! expression. It takes the quoted AST expression and converts it
/// to a TokenStream.
///
/// # Arguments
///
/// * `expr` - The AST expression to quote
///
/// # Returns
///
/// A TokenStream representing the quoted code
///
/// # Example
///
/// ```rust
/// use verum_compiler::quote_macro::quote_expr;
/// use verum_ast::{Expr, Span, expr::ExprKind};
///
/// let expr = Expr::new(
///     ExprKind::Path(verum_ast::ty::Path::single(
///         verum_ast::ty::Ident::new("x", Span::default())
///     )),
///     Span::default()
/// );
///
/// let ts = quote_expr(&expr);
/// assert!(!ts.is_empty());
/// ```
pub fn quote_expr(expr: &Expr) -> TokenStream {
    expr.into_token_stream()
}

/// Execute a quote! invocation with a template string
///
/// This parses the template string and expands it with the given context.
///
/// # Arguments
///
/// * `template` - The template string containing Verum code with interpolations
/// * `context` - The context containing variable bindings for interpolation
///
/// # Returns
///
/// Result containing the expanded TokenStream or an error
///
/// # Example
///
/// ```rust
/// use verum_compiler::quote_macro::quote_with_context;
/// use verum_compiler::quote::MetaContext;
/// use verum_compiler::quote::ident;
/// use verum_ast::Span;
///
/// let mut ctx = MetaContext::new();
/// ctx.bind_single(
///     "name".into(),
///     ident("my_var", Span::default())
/// );
///
/// let result = quote_with_context("let #name = 42;", &ctx).unwrap();
/// assert!(!result.is_empty());
/// ```
pub fn quote_with_context(
    template: &str,
    context: &MetaContext,
) -> Result<TokenStream, QuoteError> {
    let quote = Quote::parse(template)?;
    quote.expand(context)
}

/// Execute an unquote! invocation at compile time
///
/// This function is called by the meta function evaluator when it encounters
/// an unquote! expression. It takes a TokenStream and parses it back into
/// an AST expression.
///
/// # Arguments
///
/// * `stream` - The TokenStream to unquote
///
/// # Returns
///
/// Result containing the parsed AST expression or an error
///
/// # Example
///
/// ```rust
/// use verum_compiler::quote_macro::{quote_expr, unquote_stream};
/// use verum_ast::{Expr, Span, expr::ExprKind};
///
/// let expr = Expr::new(
///     ExprKind::Path(verum_ast::ty::Path::single(
///         verum_ast::ty::Ident::new("x", Span::default())
///     )),
///     Span::default()
/// );
///
/// let ts = quote_expr(&expr);
/// let result = unquote_stream(&ts).unwrap();
/// ```
pub fn unquote_stream(stream: &TokenStream) -> Result<Expr, crate::quote::ParseError> {
    stream.parse_as_expr()
}

/// Convert a ConstValue containing a TokenStream to an Expr
///
/// This is a helper for the meta evaluator to convert TokenStream values
/// back into AST expressions.
pub fn const_value_to_expr(value: &ConstValue) -> Result<Expr, MetaError> {
    match value {
        ConstValue::Expr(expr) => Ok(expr.clone()),
        _ => Err(MetaError::TypeMismatch {
            expected: "Expr or TokenStream".to_string().into(),
            found: value.type_name(),
        }),
    }
}

/// Built-in quote! function for meta context
///
/// This can be registered as a built-in meta function that generates
/// a TokenStream from its arguments.
///
/// # Arguments
///
/// * `args` - List of ConstValues representing the code to quote
///
/// # Returns
///
/// Result containing a ConstValue::Expr with the quoted code
pub fn meta_quote(args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: 0,
        });
    }

    // For now, if we get an Expr, convert it to a TokenStream
    if let Some(expr) = args.first().and_then(|v| v.as_expr()) {
        let _stream = quote_expr(expr);
        // Wrap the TokenStream in a ConstValue
        // Since ConstValue doesn't have a TokenStream variant, we keep it as Expr
        Ok(ConstValue::Expr(expr.clone()))
    } else if let Some(text) = args.first().and_then(|v| v.as_text()) {
        // If given a text string, parse it as a template
        let quote = Quote::parse(text.as_str())
            .map_err(|e| MetaError::Other(format!("Quote parse error: {}", e).into()))?;

        // Create an empty context for now
        let ctx = MetaContext::new();
        let stream = quote
            .expand(&ctx)
            .map_err(|e| MetaError::Other(format!("Quote expansion error: {}", e).into()))?;

        // Parse the stream back to an expression
        let expr = stream
            .parse_as_expr()
            .map_err(|e| MetaError::Other(format!("Parse error: {}", e).into()))?;

        Ok(ConstValue::Expr(expr))
    } else {
        Err(MetaError::TypeMismatch {
            expected: "Expr or Text".to_string().into(),
            found: args
                .first()
                .map(|v| v.type_name())
                .unwrap_or("none".into()),
        })
    }
}

/// Built-in unquote! function for meta context
///
/// This can be registered as a built-in meta function that converts
/// a TokenStream back into an AST expression.
///
/// # Arguments
///
/// * `args` - List containing a ConstValue::Expr (TokenStream wrapper)
///
/// # Returns
///
/// Result containing a ConstValue::Expr with the unquoted expression
pub fn meta_unquote(args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: 0,
        });
    }

    // Extract the expression
    if let Some(expr) = args.first().and_then(|v| v.as_expr()) {
        // For now, just return the expression as-is
        // In a full implementation, this would convert a TokenStream to Expr
        Ok(ConstValue::Expr(expr.clone()))
    } else {
        Err(MetaError::TypeMismatch {
            expected: "Expr".into(),
            found: args
                .first()
                .map(|v| v.type_name())
                .unwrap_or("none".into()),
        })
    }
}

/// Create a TokenStream from a string of Verum code
///
/// This is a convenience function for creating TokenStreams from code strings.
///
/// # Arguments
///
/// * `code` - The Verum code as a string
///
/// # Returns
///
/// Result containing the TokenStream or a parse error
///
/// # Example
///
/// ```rust
/// use verum_compiler::quote_macro::tokenstream_from_str;
///
/// let ts = tokenstream_from_str("let x = 42;").unwrap();
/// assert!(!ts.is_empty());
/// ```
pub fn tokenstream_from_str(code: &str) -> Result<TokenStream, crate::quote::ParseError> {
    use verum_ast::FileId;
    TokenStream::from_str(code, FileId::new(0))
}

/// Parse a type from a TokenStream
///
/// This is a helper for meta functions that need to work with types.
pub fn parse_type_from_stream(stream: &TokenStream) -> Result<Type, crate::quote::ParseError> {
    stream.parse_as_type()
}

/// Helper to create a quote context with bindings
///
/// This is a convenience function for creating contexts with pre-populated
/// bindings for use in quote! expansion.
///
/// # Example
///
/// ```rust
/// use verum_compiler::quote_macro::create_quote_context;
/// use verum_compiler::quote::ident;
/// use verum_ast::Span;
///
/// let bindings = vec![
///     ("name", ident("my_var", Span::default())),
///     ("value", ident("42", Span::default())),
/// ];
///
/// let ctx = create_quote_context(&bindings);
/// ```
pub fn create_quote_context(bindings: &[(&str, TokenStream)]) -> MetaContext {
    let mut ctx = MetaContext::new();
    for (name, stream) in bindings {
        ctx.bind_single(QuoteText::from(*name), stream.clone());
    }
    ctx
}

/// Helper to create a quote context with repeated bindings
///
/// This is useful for patterns like `#(#items),*`
pub fn create_quote_context_with_repeats(
    singles: &[(&str, TokenStream)],
    repeats: &[(&str, Vec<TokenStream>)],
) -> MetaContext {
    let mut ctx = MetaContext::new();

    for (name, stream) in singles {
        ctx.bind_single(QuoteText::from(*name), stream.clone());
    }

    for (name, streams) in repeats {
        ctx.bind_repeat(
            QuoteText::from(*name),
            List::from_iter(streams.iter().cloned()),
        );
    }

    ctx
}

/// Macro expansion context for procedural macros
///
/// This provides the environment in which procedural macros execute,
/// including access to the type system, error reporting, and code generation.
#[derive(Debug, Clone)]
pub struct MacroExpansionContext {
    /// The span of the macro invocation (for error reporting)
    pub span: Span,

    /// The module path where the macro is being expanded
    pub module_path: String,

    /// Whether we're in a const context
    pub is_const: bool,

    /// Whether we're in an async context
    pub is_async: bool,
}

impl MacroExpansionContext {
    /// Create a new macro expansion context
    pub fn new(span: Span, module_path: String) -> Self {
        Self {
            span,
            module_path,
            is_const: false,
            is_async: false,
        }
    }

    /// Create a context for const evaluation
    pub fn in_const_context(mut self) -> Self {
        self.is_const = true;
        self
    }

    /// Create a context for async execution
    pub fn in_async_context(mut self) -> Self {
        self.is_async = true;
        self
    }
}

/// Result type for macro expansion
pub type MacroResult = Result<TokenStream, MacroError>;

/// Errors that can occur during macro expansion
#[derive(Debug, Clone)]
pub enum MacroError {
    /// Parse error in the macro input
    ParseError(String),

    /// Type error in generated code
    TypeError(String),

    /// Invalid macro arguments
    InvalidArguments(String),

    /// Macro expansion limit exceeded (prevents infinite recursion)
    ExpansionLimitExceeded,

    /// Other error
    Other(String),
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MacroError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            MacroError::TypeError(msg) => write!(f, "Type error: {}", msg),
            MacroError::InvalidArguments(msg) => write!(f, "Invalid arguments: {}", msg),
            MacroError::ExpansionLimitExceeded => write!(f, "Macro expansion limit exceeded"),
            MacroError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for MacroError {}

impl From<QuoteError> for MacroError {
    fn from(err: QuoteError) -> Self {
        MacroError::ParseError(err.to_string())
    }
}

impl From<crate::quote::ParseError> for MacroError {
    fn from(err: crate::quote::ParseError) -> Self {
        MacroError::ParseError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote::ident;
    use verum_ast::{Expr, Span, expr::ExprKind};

    #[test]
    fn test_quote_expr() {
        let expr = Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "x",
                Span::default(),
            ))),
            Span::default(),
        );

        let ts = quote_expr(&expr);
        assert!(!ts.is_empty());
    }

    #[test]
    fn test_quote_with_context() {
        let mut ctx = MetaContext::new();
        ctx.bind_single(QuoteText::from("name"), ident("my_var", Span::default()));

        let result = quote_with_context("let # name = 42;", &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_unquote_stream() {
        let expr = Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "x",
                Span::default(),
            ))),
            Span::default(),
        );

        let ts = quote_expr(&expr);
        let result = unquote_stream(&ts);
        assert!(result.is_ok());
    }

    #[test]
    fn test_tokenstream_from_str() {
        let ts = tokenstream_from_str("let x = 42;");
        assert!(ts.is_ok());
        assert!(!ts.unwrap().is_empty());
    }

    #[test]
    fn test_create_quote_context() {
        let bindings = vec![
            ("name", ident("my_var", Span::default())),
            ("value", ident("42", Span::default())),
        ];

        let ctx = create_quote_context(&bindings);
        assert!(ctx.get_single("name").is_some());
        assert!(ctx.get_single("value").is_some());
    }

    #[test]
    fn test_create_quote_context_with_repeats() {
        let singles = vec![("prefix", ident("get", Span::default()))];
        let repeats = vec![(
            "fields",
            vec![
                ident("name", Span::default()),
                ident("age", Span::default()),
            ],
        )];

        let ctx = create_quote_context_with_repeats(&singles, &repeats);
        assert!(ctx.get_single("prefix").is_some());
        assert!(ctx.get_repeat("fields").is_some());
    }

    #[test]
    fn test_meta_quote() {
        let expr = Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "x",
                Span::default(),
            ))),
            Span::default(),
        );

        let result = meta_quote(List::from_iter(vec![ConstValue::Expr(expr)]));
        assert!(result.is_ok());
    }

    #[test]
    fn test_macro_expansion_context() {
        let ctx = MacroExpansionContext::new(Span::default(), "my::module".to_string());

        assert_eq!(ctx.module_path.as_str(), "my::module");
        assert!(!ctx.is_const);
        assert!(!ctx.is_async);

        let const_ctx = ctx.clone().in_const_context();
        assert!(const_ctx.is_const);
    }
}
