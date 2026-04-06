//! Phase 1: Lexical Analysis & Parsing
//!
//! Converts source text to Abstract Syntax Tree (AST).
//!
//! ## Responsibilities
//!
//! 1. **Tokenization**: Convert source text to token stream
//!    - Profile-aware keyword recognition
//!    - Tagged literal recognition (contract#"...", sql#"...")
//!    - Numeric suffix recognition (100_km, 5_seconds)
//! 2. **Parsing**: Build AST from tokens
//!    - LL(k) predictive recursive descent
//!    - Error recovery with synchronization
//!    - Preserve meta annotations
//!
//! ## Performance Targets
//!
//! - Lexing + Parsing: ~50-100ms per 10K LOC
//! - Parallel parsing of independent modules
//!
//! Phase 1: Lexical analysis and parsing. Tokenizes source with profile awareness,
//! parses LL(k) grammar, recognizes tagged literals and numeric suffixes.
//! Output: AST with preserved meta annotations.

use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use verum_ast::{FileId, Module};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;
use verum_common::{List, Text};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

/// Lexical analysis and parsing phase
pub struct LexicalParsingPhase {
    /// Source file manager
    sources: SourceManager,
}

impl LexicalParsingPhase {
    pub fn new() -> Self {
        Self {
            sources: SourceManager::new(),
        }
    }

    /// Parse a single source file
    pub fn parse_file(&mut self, path: PathBuf, source: &str) -> Result<Module, List<Diagnostic>> {
        let start = Instant::now();

        // Register source
        let file_id = self.sources.add_source(path.clone(), Text::from(source));

        // Tokenize
        let lexer = Lexer::new(source, file_id);

        // Parse
        let parser = VerumParser::new();
        let module = parser
            .parse_module(lexer, file_id)
            .map_err(|errors| self.convert_parse_errors(errors.into()))?;

        let duration = start.elapsed();
        tracing::debug!(
            "Parsed {} in {:.2}ms ({} items)",
            path.display(),
            duration.as_millis(),
            module.items.len()
        );

        Ok(module)
    }

    /// Parse multiple source files in parallel
    pub fn parse_files(
        &mut self,
        sources: List<(PathBuf, Text)>,
    ) -> Result<List<Module>, List<Diagnostic>> {
        let start = Instant::now();
        let count = sources.len();

        // Pre-allocate file IDs sequentially (cannot do this in parallel)
        let sources_with_ids: List<_> = sources
            .into_iter()
            .map(|(path, source)| {
                let file_id = self.sources.add_source(path.clone(), source.clone());
                (path, source, file_id)
            })
            .collect();

        // Parse in parallel using Rayon for CPU-bound work
        use rayon::prelude::*;
        let results: Vec<_> = sources_with_ids
            .into_par_iter() // Parallel parsing - each file is independent
            .map(|(path, source, file_id)| {
                let lexer = Lexer::new(source.as_str(), *file_id);
                let parser = VerumParser::new();
                parser.parse_module(lexer, *file_id).map_err(|errors| {
                    errors
                        .into_iter()
                        .map(|e| {
                            DiagnosticBuilder::new(Severity::Error)
                                .message(format!("Parse error in {}: {}", path.display(), e))
                                .build()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        // Collect errors and modules
        let mut modules = List::new();
        let mut all_errors: Vec<Diagnostic> = Vec::new();

        for result in results {
            match result {
                Ok(module) => modules.push(module),
                Err(errors) => all_errors.extend(errors),
            }
        }

        let all_errors: List<Diagnostic> = List::from(all_errors);

        if !all_errors.is_empty() {
            return Err(all_errors);
        }

        let duration = start.elapsed();
        tracing::info!("Parsed {} files in {:.2}ms", count, duration.as_millis());

        Ok(modules)
    }

    /// Convert parser errors to diagnostics
    fn convert_parse_errors(&self, errors: List<verum_fast_parser::ParseError>) -> List<Diagnostic> {
        errors
            .into_iter()
            .map(|e| {
                let mut builder = DiagnosticBuilder::new(Severity::Error)
                    .message(e.to_string());
                // Include error code if present (e.g., M401 for splice outside quote)
                if let Some(ref code) = e.code {
                    builder = builder.code(code.clone());
                }
                builder.build()
            })
            .collect()
    }
}

impl Default for LexicalParsingPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for LexicalParsingPhase {
    fn name(&self) -> &str {
        "Phase 1: Lexical Analysis & Parsing"
    }

    fn description(&self) -> &str {
        "Tokenization and syntax parsing with profile awareness"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        let source_files = match &input.data {
            PhaseData::SourceFiles(files) => files,
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for lexical parsing phase")
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Parse all source files into AST modules
        let mut modules = List::new();
        let mut all_errors = List::new();

        for source_path in source_files {
            // Read source file
            let source = match std::fs::read_to_string(source_path) {
                Ok(s) => s,
                Err(e) => {
                    let diag = DiagnosticBuilder::new(Severity::Error)
                        .message(format!("Failed to read source file {}: {}", source_path, e))
                        .build();
                    all_errors.push(diag);
                    continue;
                }
            };

            // Parse the file
            let file_id = FileId::new(modules.len() as u32);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            match parser.parse_module(lexer, file_id) {
                Ok(module) => {
                    tracing::debug!(
                        "Successfully parsed {} ({} items)",
                        source_path,
                        module.items.len()
                    );
                    modules.push(module);
                }
                Err(errors) => {
                    // Convert parse errors to diagnostics
                    for error in errors {
                        let mut builder = DiagnosticBuilder::new(Severity::Error)
                            .message(format!("Parse error: {}", error));
                        // Include error code if present (e.g., M401 for splice outside quote)
                        if let Some(ref code) = error.code {
                            builder = builder.code(code.clone());
                        }
                        all_errors.push(builder.build());
                    }
                }
            }
        }

        // If we have errors and no successful modules, fail
        if !all_errors.is_empty() && modules.is_empty() {
            return Err(all_errors);
        }

        let duration = start.elapsed();
        let metrics = PhaseMetrics::new(self.name())
            .with_duration(duration)
            .with_items_processed(modules.len());

        tracing::info!(
            "Lexical parsing complete: {} modules, {:.2}ms",
            modules.len(),
            duration.as_millis()
        );

        Ok(PhaseOutput {
            data: PhaseData::AstModules(modules),
            warnings: all_errors, // Report errors as warnings if we got some modules
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Files can be parsed in parallel
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

/// Source file manager
#[derive(Debug, Clone)]
pub struct SourceManager {
    sources: List<SourceFile>,
    next_id: usize,
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            sources: List::new(),
            next_id: 0,
        }
    }

    pub fn add_source(&mut self, path: PathBuf, source: Text) -> FileId {
        let id = FileId::new(self.next_id as u32);

        // Register with global source file registry for error diagnostics
        // This allows global_span_to_line_col conversion to work correctly across the compiler
        verum_common::register_source_file(id, path.display().to_string(), source.as_str());

        self.sources.push(SourceFile { path, source, id });
        self.next_id += 1;
        id
    }

    pub fn get_source(&self, id: FileId) -> Option<&SourceFile> {
        self.sources.iter().find(|s| s.id == id)
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: Text,
    pub id: FileId,
}
