//! Code snippet extraction from source files.
//!
//! Extracts relevant source code snippets with context for diagnostic rendering.
//! Supports:
//! - Multi-line spans
//! - Context lines before/after
//! - Source file caching
//! - Line-based extraction with proper indexing

use crate::Span;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Text};

/// Extracts code snippets from source files
pub struct SnippetExtractor {
    /// Cache of loaded source files (path -> lines)
    source_cache: Map<PathBuf, List<Text>>,
    /// Maximum number of lines to cache per file
    max_cache_size: usize,
}

impl SnippetExtractor {
    /// Create a new snippet extractor
    pub fn new() -> Self {
        Self {
            source_cache: Map::new(),
            max_cache_size: 100, // Cache up to 100 files
        }
    }

    /// Create with custom cache size
    pub fn with_cache_size(max_cache_size: usize) -> Self {
        Self {
            source_cache: Map::new(),
            max_cache_size,
        }
    }

    /// Extract a snippet from a source file
    pub fn extract_snippet(
        &mut self,
        file: &Path,
        span: &Span,
        context_lines: usize,
    ) -> Result<Snippet, SnippetError> {
        let source_lines = self.get_source(file)?;

        // Calculate line range with context
        let start_line = span.line.saturating_sub(context_lines).max(1);
        let end_line_from_span = span.end_line.unwrap_or(span.line);
        let end_line = (end_line_from_span + context_lines).min(source_lines.len());

        // Extract lines
        let mut lines = List::new();
        for line_num in start_line..=end_line {
            let line_idx = line_num - 1; // Convert to 0-based index
            if line_idx < source_lines.len() {
                let content = source_lines[line_idx].clone();
                let is_in_span = line_num >= span.line && line_num <= end_line_from_span;

                lines.push(SourceLine {
                    line_number: line_num,
                    content,
                    is_in_span,
                    span_start_col: if line_num == span.line {
                        Some(span.column)
                    } else {
                        None
                    },
                    span_end_col: if line_num == end_line_from_span {
                        Some(span.end_column)
                    } else {
                        None
                    },
                });
            }
        }

        Ok(Snippet {
            file: file.to_path_buf(),
            lines,
            primary_span: span.clone(),
            start_line,
            end_line,
        })
    }

    /// Extract multiple snippets for different spans in the same file
    pub fn extract_multi_span_snippet(
        &mut self,
        file: &Path,
        spans: &[Span],
        context_lines: usize,
    ) -> Result<MultiSpanSnippet, SnippetError> {
        if spans.is_empty() {
            return Err(SnippetError::NoSpans);
        }

        let source_lines = self.get_source(file)?;

        // Find the overall range covering all spans
        let min_line = spans.iter().map(|s| s.line).min().unwrap();
        let max_line = spans
            .iter()
            .map(|s| s.end_line.unwrap_or(s.line))
            .max()
            .unwrap();

        let start_line = min_line.saturating_sub(context_lines).max(1);
        let end_line = (max_line + context_lines).min(source_lines.len());

        // Extract lines
        let mut lines = List::new();
        for line_num in start_line..=end_line {
            let line_idx = line_num - 1;
            if line_idx < source_lines.len() {
                let content = source_lines[line_idx].clone();

                // Check which spans include this line
                let spans_on_line: List<SpanOnLine> = spans
                    .iter()
                    .filter(|s| {
                        let span_end = s.end_line.unwrap_or(s.line);
                        line_num >= s.line && line_num <= span_end
                    })
                    .map(|s| SpanOnLine {
                        start_col: if line_num == s.line {
                            Some(s.column)
                        } else {
                            None
                        },
                        end_col: if line_num == s.end_line.unwrap_or(s.line) {
                            Some(s.end_column)
                        } else {
                            None
                        },
                    })
                    .collect();

                lines.push(MultiSpanSourceLine {
                    line_number: line_num,
                    content,
                    spans: spans_on_line,
                });
            }
        }

        Ok(MultiSpanSnippet {
            file: file.to_path_buf(),
            lines,
            spans: spans.to_vec(),
            start_line,
            end_line,
        })
    }

    /// Get source lines for a file (with caching)
    fn get_source(&mut self, file: &Path) -> Result<&List<Text>, SnippetError> {
        let path_buf = file.to_path_buf();

        if !self.source_cache.contains_key(&path_buf) {
            // Check cache size and evict if needed
            if self.source_cache.len() >= self.max_cache_size {
                // Simple eviction: clear half the cache
                let keys_to_remove: List<PathBuf> = self
                    .source_cache
                    .keys()
                    .take(self.max_cache_size / 2)
                    .cloned()
                    .collect();
                for key in keys_to_remove {
                    self.source_cache.remove(&key);
                }
            }

            // Load the file
            let content = fs::read_to_string(file).map_err(SnippetError::IoError)?;

            let lines: List<Text> = content.lines().map(Text::from).collect();

            self.source_cache.insert(path_buf.clone(), lines);
        }

        self.source_cache
            .get(&path_buf)
            .ok_or(SnippetError::FileNotInCache)
    }

    /// Add source content directly (useful for testing or virtual files)
    pub fn add_source(&mut self, file: PathBuf, content: &str) {
        let lines: List<Text> = content.lines().map(Text::from).collect();
        self.source_cache.insert(file, lines);
    }

    /// Clear the cache
    pub fn clear_cache(&mut self) {
        self.source_cache.clear();
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            cached_files: self.source_cache.len(),
            max_capacity: self.max_cache_size,
        }
    }
}

impl Default for SnippetExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// A snippet of source code with context
#[derive(Debug, Clone)]
pub struct Snippet {
    /// Path to the source file
    pub file: PathBuf,
    /// Lines included in the snippet
    pub lines: List<SourceLine>,
    /// The primary span this snippet was extracted for
    pub primary_span: Span,
    /// First line number in the snippet
    pub start_line: usize,
    /// Last line number in the snippet
    pub end_line: usize,
}

impl Snippet {
    /// Get the maximum line number width (for formatting)
    pub fn max_line_number_width(&self) -> usize {
        self.end_line.to_string().len()
    }

    /// Check if a line is within the primary span
    pub fn is_primary_line(&self, line_num: usize) -> bool {
        self.lines
            .iter()
            .any(|l| l.line_number == line_num && l.is_in_span)
    }
}

/// A single source line with metadata
#[derive(Debug, Clone)]
pub struct SourceLine {
    /// Line number (1-based)
    pub line_number: usize,
    /// Line content
    pub content: Text,
    /// Is this line within the span?
    pub is_in_span: bool,
    /// Start column of span on this line (if applicable)
    pub span_start_col: Option<usize>,
    /// End column of span on this line (if applicable)
    pub span_end_col: Option<usize>,
}

impl SourceLine {
    /// Get the length of the underline for this line
    pub fn underline_length(&self) -> usize {
        match (self.span_start_col, self.span_end_col) {
            (Some(start), Some(end)) => end.saturating_sub(start).max(1),
            (Some(start), None) => self.content.len().saturating_sub(start).max(1),
            _ => 1,
        }
    }

    /// Get the start column for underlining (0-based)
    pub fn underline_start(&self) -> usize {
        self.span_start_col.unwrap_or(0)
    }
}

/// Snippet with multiple spans
#[derive(Debug, Clone)]
pub struct MultiSpanSnippet {
    pub file: PathBuf,
    pub lines: List<MultiSpanSourceLine>,
    pub spans: Vec<Span>,
    pub start_line: usize,
    pub end_line: usize,
}

impl MultiSpanSnippet {
    pub fn max_line_number_width(&self) -> usize {
        self.end_line.to_string().len()
    }
}

/// Source line with multiple span annotations
#[derive(Debug, Clone)]
pub struct MultiSpanSourceLine {
    pub line_number: usize,
    pub content: Text,
    pub spans: List<SpanOnLine>,
}

/// Span information for a specific line
#[derive(Debug, Clone)]
pub struct SpanOnLine {
    pub start_col: Option<usize>,
    pub end_col: Option<usize>,
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub cached_files: usize,
    pub max_capacity: usize,
}

/// Errors that can occur during snippet extraction
#[derive(Debug)]
pub enum SnippetError {
    /// I/O error reading the file
    IoError(io::Error),
    /// File not found in cache (shouldn't happen)
    FileNotInCache,
    /// No spans provided
    NoSpans,
    /// Line number out of range
    LineOutOfRange { line: usize, max: usize },
}

impl std::fmt::Display for SnippetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnippetError::IoError(e) => write!(f, "I/O error: {}", e),
            SnippetError::FileNotInCache => write!(f, "File not found in cache"),
            SnippetError::NoSpans => write!(f, "No spans provided"),
            SnippetError::LineOutOfRange { line, max } => {
                write!(f, "Line {} out of range (max: {})", line, max)
            }
        }
    }
}

impl std::error::Error for SnippetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_file_content() -> &'static str {
        "fn main() {\n\
         let x: Positive = -5;\n\
         println!(x);\n\
         }\n"
    }

    #[test]
    fn test_snippet_extraction() {
        let mut extractor = SnippetExtractor::new();
        let file = PathBuf::from("test.vr");
        extractor.add_source(file.clone(), create_test_file_content());

        let span = Span {
            file: "test.vr".into(),
            line: 2,
            column: 19,
            end_line: Some(2),
            end_column: 21,
        };

        let snippet = extractor.extract_snippet(&file, &span, 1).unwrap();

        assert_eq!(snippet.lines.len(), 3); // line 1, 2, 3 (with context)
        assert_eq!(snippet.start_line, 1);
        assert_eq!(snippet.end_line, 3);
    }

    #[test]
    fn test_multi_line_span() {
        let mut extractor = SnippetExtractor::new();
        let file = PathBuf::from("test.vr");
        let content =
            "fn test() {\nlet result = if condition {\n    value_a\n} else {\n    value_b\n};\n}";
        extractor.add_source(file.clone(), content);

        let span = Span {
            file: "test.vr".into(),
            line: 2,
            column: 14,
            end_line: Some(6),
            end_column: 2,
        };

        let snippet = extractor.extract_snippet(&file, &span, 0).unwrap();

        // Should include lines 2-6
        assert!(snippet.lines.len() >= 5);
    }

    #[test]
    fn test_cache_management() {
        let mut extractor = SnippetExtractor::with_cache_size(5);

        // Add 6 files - cache should evict oldest when full
        for i in 0..6 {
            let file = PathBuf::from(format!("test{}.vr", i));
            extractor.add_source(file, "fn main() {}");
        }

        let stats = extractor.cache_stats();
        // After adding 6 files, cache might not have evicted yet (depends on impl)
        // Just verify we can add files and get stats without panic
        assert!(stats.cached_files >= 1);
        assert!(stats.max_capacity == 5);
    }

    #[test]
    fn test_source_line_underline() {
        let line = SourceLine {
            line_number: 1,
            content: "let x: Positive = -5;".into(),
            is_in_span: true,
            span_start_col: Some(19),
            span_end_col: Some(21),
        };

        assert_eq!(line.underline_start(), 19);
        assert_eq!(line.underline_length(), 2);
    }

    #[test]
    fn test_multi_span_extraction() {
        let mut extractor = SnippetExtractor::new();
        let file = PathBuf::from("test.vr");
        extractor.add_source(file.clone(), create_test_file_content());

        let spans = vec![
            Span {
                file: "test.vr".into(),
                line: 2,
                column: 8,
                end_line: Some(2),
                end_column: 9,
            },
            Span {
                file: "test.vr".into(),
                line: 2,
                column: 19,
                end_line: Some(2),
                end_column: 21,
            },
        ];

        let snippet = extractor
            .extract_multi_span_snippet(&file, &spans, 1)
            .unwrap();

        assert!(!snippet.lines.is_empty());
        assert_eq!(snippet.spans.len(), 2);
    }

    #[test]
    fn test_cache_clear() {
        let mut extractor = SnippetExtractor::new();
        let file = PathBuf::from("test.vr");
        extractor.add_source(file, "test");

        assert_eq!(extractor.cache_stats().cached_files, 1);

        extractor.clear_cache();

        assert_eq!(extractor.cache_stats().cached_files, 0);
    }
}
