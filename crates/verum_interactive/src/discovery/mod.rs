//! Discovery system for exploring Verum's core/ capabilities
//!
//! This module provides interactive documentation, examples, and exploration
//! tools for the Verum standard library (core/).

pub mod completions;
pub mod docs;
pub mod examples;
pub mod index;
pub mod search;
pub mod tutorials;

pub use completions::{CompletionContext, CompletionItem, CompletionKind, CompletionProvider, InlineHelp, get_inline_help};
pub use docs::{DocEntry, DocKind};
pub use examples::{Example, ExampleCategory};
pub use index::{DiscoveryIndex, ModuleInfo, ModuleTree};
pub use search::{SearchQuery, SearchResult};
pub use tutorials::{
    Tutorial, TutorialStep, Challenge, TestCase,
    PlaybookTemplate, TemplateCell,
    builtin_tutorials, builtin_challenges, builtin_templates,
};
