#![allow(unexpected_cfgs)]
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::if_same_then_else)]
//! Production-ready parser for the Verum language with LSP support.
//!
//! This crate extends `verum_fast_parser` with LSP-specific features:
//!
//! - **Lossless parsing**: Preserves all trivia (whitespace, comments) for formatting
//! - **Event-based parsing**: Marker/precede pattern for retroactive tree building
//! - **Incremental parsing**: Efficient reparsing for IDE document synchronization
//! - **Enhanced error recovery**: IDE-optimized recovery with structured ERROR nodes
//!
//! For compilation (direct AST construction), use `verum_fast_parser` directly.
//! For IDE features, use this crate.
//!
//! # Architecture
//!
//! ```text
//! verum_fast_parser (core parsing engine)
//!          ↑
//!     verum_parser (this crate: LSP extensions, re-exports core)
//!          ↑
//!     verum_lsp (script parsing, completion, etc.)
//! ```
//!
//! # Example
//!
//! ```rust
//! use verum_parser::VerumParser;
//! use verum_lexer::Lexer;
//! use verum_ast::span::FileId;
//!
//! let source = r#"
//!     fn factorial(n: Int{>= 0}) -> Int {
//!         match n {
//!             0 => 1,
//!             n => n * factorial(n - 1)
//!         }
//!     }
//! "#;
//!
//! let file_id = FileId::new(0);
//! let lexer = Lexer::new(source, file_id);
//! let parser = VerumParser::new();
//! let result = parser.parse_module(lexer, file_id);
//!
//! match result {
//!     Ok(module) => println!("Parsed successfully: {} items", module.items.len()),
//!     Err(errors) => {
//!         for error in errors {
//!             eprintln!("Parse error: {}", error);
//!         }
//!     }
//! }
//! ```
//!
//! # LSP-Specific Features
//!
//! ## Lossless Parsing
//!
//! ```rust,ignore
//! use verum_parser::{LosslessParser, LosslessParse};
//! use verum_ast::FileId;
//!
//! let parser = LosslessParser::new();
//! let result = parser.parse("fn foo() { }", FileId::new(0));
//!
//! // Access both AST and green tree
//! let module = result.module;
//! let green = result.green;
//!
//! // Reconstruct original source (lossless)
//! assert_eq!(result.text(), "fn foo() { }");
//! ```
//!
//! ## Incremental Parsing
//!
//! ```rust,ignore
//! use verum_parser::IncrementalParserEngine;
//!
//! let mut engine = IncrementalParserEngine::new();
//! engine.parse_full("fn foo() { }");
//!
//! // Apply incremental edit
//! engine.apply_lsp_change(lsp_change);
//! ```

#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

// =============================================================================
// RE-EXPORT CORE PARSING ENGINE FROM verum_fast_parser
// =============================================================================

// Re-export everything from verum_fast_parser
pub use verum_fast_parser::{
    // Main parser types
    FastParser, Parser, VerumParser,
    // Core parsing infrastructure
    RecursiveParser, TokenStream, merge_spans, span_from_tokens,
    // Error handling
    ParseError, ParseResult,
    // Attribute validation
    AttributeValidationWarning, AttributeValidator, AttributeValidatorTrait, ValidationConfig,
    validate_field_attributes, validate_function_attributes, validate_match_arm_attributes,
    validate_parsed_attributes, validate_type_attributes,
    // Base recovery types
    Delimiter, RecoveryContext, RecoveryStrategy, SyncPoint,
    can_start_expression, can_start_item, can_start_statement, is_statement_terminator,
    missing_token_message, unexpected_token_message,
};

// Re-export error module for full access to error types
pub use verum_fast_parser::error;

// =============================================================================
// LSP-SPECIFIC MODULES
// =============================================================================

pub mod ast_sink;
pub mod incremental;
pub mod recovery;
pub mod recovery_parser;
pub mod syntax_bridge;

// =============================================================================
// LSP-SPECIFIC EXPORTS
// =============================================================================

// Export LSP recovery types (extends base recovery from verum_fast_parser)
pub use recovery::{
    EventRecovery, Recoverable, RecoveryResult as RecoveryOpResult,
    RecoverySet, recovery_sets, token_kind_to_syntax_kind,
};

// Export incremental parsing infrastructure
pub use incremental::{
    BenchmarkResult, IncrementalDocument, IncrementalParserEngine,
    benchmark_incremental_vs_full, run_benchmark_suite,
};

// Export lossless parsing infrastructure
pub use syntax_bridge::{LosslessParser, LosslessParse, IncrementalParser};

// Export event-based parsing infrastructure
pub use syntax_bridge::{EventBasedParser, EventBasedParse};

// Export recovering event-based parser with full error recovery
pub use recovery_parser::{RecoveringEventParser, RecoveringParse};

// Export AST sink for converting green tree to semantic AST
pub use ast_sink::{AstSink, AstSinkResult, syntax_to_ast};
