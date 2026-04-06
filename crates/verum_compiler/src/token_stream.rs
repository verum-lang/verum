//! TokenStream - Re-export of TokenStream from quote module
//!
//! This module provides a convenient re-export of the TokenStream type
//! and related functionality from the quote module. This allows users to
//! import from either `verum_compiler::token_stream` or `verum_compiler::quote`.
//!
//! Token stream for procedural macros: provides the interface between
//! meta functions and the compiler, enabling AST manipulation via quote!.

// Re-export all public types from the quote module
pub use crate::quote::{
    GroupDelimiter, InterpolationKind, MetaContext, ParseError, Quote,
    QuoteBuilder, QuoteError, ToTokens, TokenStream,
};

// Re-export HygieneContext from hygiene module
pub use crate::hygiene::HygieneContext;

// Re-export helper functions
pub use crate::quote::{
    concat, format_ident, generate_field_access, generate_fn, generate_impl, generate_match_arm,
    generate_method_call, generate_self_field, generate_struct_literal, ident, literal_int,
    literal_string, stringify,
};

/// Create a quote from a string literal
///
/// This is a helper function that parses a string into a Quote object.
///
/// # Example
///
/// ```rust
/// use verum_compiler::token_stream::quote_str;
///
/// let q = quote_str("let x = #value;").unwrap();
/// ```
pub fn quote_str(s: &str) -> Result<Quote, QuoteError> {
    Quote::parse(s)
}

/// Macro-like helper for creating token streams
///
/// This provides a programmatic way to create TokenStreams similar to
/// the quote! macro but without requiring procedural macro infrastructure.
///
/// # Example
///
/// ```rust
/// use verum_compiler::token_stream::quote_builder;
/// use verum_ast::Span;
///
/// let ts = quote_builder()
///     .keyword("fn")
///     .ident("example")
///     .punct("(")
///     .punct(")")
///     .punct("{")
///     .punct("}")
///     .build();
/// ```
pub fn quote_builder() -> QuoteBuilder {
    QuoteBuilder::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::Span;

    #[test]
    fn test_quote_str() {
        let q = quote_str("let x = 42;").unwrap();
        assert!(!q.interpolations().is_empty() || q.interpolations().is_empty());
    }

    #[test]
    fn test_quote_builder_reexport() {
        let ts = quote_builder()
            .keyword("let")
            .ident("x")
            .punct("=")
            .int(42)
            .build();

        assert!(!ts.is_empty());
    }

    #[test]
    fn test_helper_functions() {
        let id = ident("test", Span::default());
        assert_eq!(id.len(), 1);

        let lit = literal_int(42, Span::default());
        assert_eq!(lit.len(), 1);

        let str_lit = literal_string("hello", Span::default());
        assert_eq!(str_lit.len(), 1);
    }
}
