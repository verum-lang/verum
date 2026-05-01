//! Interactive environment for Verum: REPL, Script Parser, and Playbook TUI
//!

//! This crate provides interactive programming features for Verum:
//!

//! - **Script Parser**: Specialized parsing for REPL/script environments
//! - **Incremental Parsing**: Efficient re-parsing for changed regions
//! - **Error Recovery**: Script-optimized error recovery with suggestions
//! - **Playbook TUI**: Jupyter-like terminal-based notebook interface
//!

//! # Architecture
//!

//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ verum_interactive │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Script parsing is provided by verum_lsp::script │
//! │ ├─ ScriptParser Expression-first parsing │
//! │ ├─ ScriptContext Session state (bindings, imports) │
//! │ └─ ParseMode Parsing mode selection │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Incremental parsing from verum_lsp::script::incremental │
//! │ ├─ IncrementalScriptParser Line-level caching │
//! │ └─ DependencyGraph Smart cache invalidation │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Error recovery from verum_lsp::script::recovery │
//! │ └─ ScriptRecovery Typo correction, completions │
//! ├─────────────────────────────────────────────────────────────┤
//! │ playbook/ TUI notebook interface │
//! │ ├─ app.rs Main application state │
//! │ ├─ ui/ Rendering components │
//! │ └─ session/ Cell and execution management │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!

//! # Example
//!

//! ```rust
//! use verum_interactive::{ScriptParser, ScriptContext, ParseMode};
//! use verum_ast::FileId;
//!

//! let parser = ScriptParser::new();
//! let mut context = ScriptContext::new();
//! let file_id = FileId::new(1);
//!

//! // Parse a line
//! match parser.parse_line("let x = 42", file_id, &mut context) {
//!  Ok(result) => println!("Parsed: {:?}", result),
//!  Err(e) => eprintln!("Error: {:?}", e),
//! }
//! ```

#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

pub mod discovery;
pub mod execution;

#[cfg(feature = "playbook")]
pub mod playbook;

pub mod output;

// Re-export script parsing types from verum_lsp
// This provides a stable API for interactive features while the implementation lives in verum_lsp
pub use verum_lsp::{
    // Incremental parsing
    CachedLine,
    DependencyGraph,
    IncrementalScriptParser,
    IncrementalStats,
    // Script parsing
    ParseMode,
    // Error recovery
    RecoveryResult,
    ScriptContext,
    ScriptParseResult,
    ScriptParser,
    ScriptRecovery,
    detect_dependencies,
    explain_error,
    needs_continuation,
    suggest_autocompletion,
    suggest_completion,
};

#[cfg(feature = "playbook")]
pub use playbook::{
    Cell, CellId, CellKind, CellOutput, PlaybookApp, SessionState,
    TensorStats as PlaybookTensorStats,
};

// Re-export execution types
pub use execution::{
    AsyncExecutor, BindingInfo, CompiledCell, ExecutionContext, ExecutionError, ExecutionHandle,
    ExecutionMessage, ExecutionPipeline, ExecutionResult, ExecutionStatus, OutputLine,
    ProgressDisplay, ProgressStyle, StreamingOutput,
};

// Re-export output types
pub use output::{
    OutputFormat, OutputRenderer, RenderedOutput, TensorPreview, TensorStats, render_collection,
    render_struct, render_tensor, render_variant,
};

// Re-export discovery types
pub use discovery::{
    Challenge,
    CompletionContext,
    CompletionItem,
    CompletionKind,
    CompletionProvider,
    DiscoveryIndex,
    DocEntry,
    DocKind,
    Example,
    ExampleCategory,
    InlineHelp,
    ModuleInfo,
    ModuleTree,
    PlaybookTemplate,
    SearchQuery,
    SearchResult,
    TemplateCell,
    TestCase,
    // Tutorials and templates
    Tutorial,
    TutorialStep,
    builtin_challenges,
    builtin_templates,
    builtin_tutorials,
    get_inline_help,
};
