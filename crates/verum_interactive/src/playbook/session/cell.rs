//! Cell types for the playbook

use std::time::Duration;
use serde::{Deserialize, Serialize};
use verum_common::Text;
use verum_ast::Span;
use verum_vbc::value::Value;

/// Unique identifier for a cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellId(pub uuid::Uuid);

impl CellId {
    /// Create a new random cell ID
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for CellId {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    /// Code cell that can be executed
    Code,
    /// Markdown cell for documentation
    Markdown,
}

/// A cell in the playbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    /// Unique identifier
    pub id: CellId,
    /// Kind of cell
    pub kind: CellKind,
    /// Source content
    pub source: Text,
    /// Output from execution (only for code cells)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CellOutput>,
    /// Execution count (only for code cells)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_count: Option<u32>,
    /// Whether the cell has been modified since last execution
    #[serde(default)]
    pub dirty: bool,
    /// Cell metadata
    #[serde(default)]
    pub metadata: CellMetadata,
    /// Whether the output display is collapsed
    #[serde(default)]
    pub output_collapsed: bool,
}

impl Cell {
    /// Create a new code cell
    pub fn new_code(source: impl Into<Text>) -> Self {
        Self {
            id: CellId::new(),
            kind: CellKind::Code,
            source: source.into(),
            output: None,
            execution_count: None,
            dirty: true,
            metadata: CellMetadata::default(),
            output_collapsed: false,
        }
    }

    /// Create a new markdown cell
    pub fn new_markdown(source: impl Into<Text>) -> Self {
        Self {
            id: CellId::new(),
            kind: CellKind::Markdown,
            source: source.into(),
            output: None,
            execution_count: None,
            dirty: false,
            metadata: CellMetadata::default(),
            output_collapsed: false,
        }
    }

    /// Toggle output collapse state
    pub fn toggle_output_collapse(&mut self) {
        self.output_collapsed = !self.output_collapsed;
    }

    /// Check if this is a code cell
    pub fn is_code(&self) -> bool {
        matches!(self.kind, CellKind::Code)
    }

    /// Check if this is a markdown cell
    pub fn is_markdown(&self) -> bool {
        matches!(self.kind, CellKind::Markdown)
    }

    /// Mark the cell as executed with the given output
    pub fn set_output(&mut self, output: CellOutput, execution_count: u32) {
        self.output = Some(output);
        self.execution_count = Some(execution_count);
        self.dirty = false;
    }

    /// Clear the output
    pub fn clear_output(&mut self) {
        self.output = None;
        self.dirty = true;
    }

    /// Update the source content
    pub fn set_source(&mut self, source: impl Into<Text>) {
        self.source = source.into();
        self.dirty = true;
    }
}

/// Output from executing a code cell
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CellOutput {
    /// Successfully evaluated to a value
    Value {
        /// String representation of the value
        repr: Text,
        /// Type information
        type_info: Text,
        /// Raw value for inspection (not serialized)
        #[serde(skip)]
        raw: Option<Value>,
    },
    /// Tensor output with rich visualization
    Tensor {
        /// Shape of the tensor
        shape: Vec<usize>,
        /// Data type (e.g., "Float32")
        dtype: Text,
        /// Preview text for display
        preview: Text,
        /// Statistics (mean, std, min, max, etc.)
        #[serde(skip_serializing_if = "Option::is_none")]
        stats: Option<TensorStats>,
    },
    /// Structured data (records, variants)
    Structured {
        /// Type name
        type_name: Text,
        /// Fields as nested outputs
        fields: Vec<(Text, CellOutput)>,
    },
    /// Collection (List, Set, Map)
    Collection {
        /// Number of elements
        len: usize,
        /// Element type
        element_type: Text,
        /// Preview of first few elements
        preview: Vec<CellOutput>,
        /// Whether truncated
        truncated: bool,
    },
    /// Execution resulted in an error
    Error {
        /// Error message
        message: Text,
        /// Optional span for error location
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<SerializableSpan>,
        /// Suggestions for fixing the error
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        suggestions: Vec<Text>,
    },
    /// Stream output (print statements)
    Stream {
        /// Stdout text
        stdout: Text,
        /// Stderr text
        #[serde(default, skip_serializing_if = "Text::is_empty")]
        stderr: Text,
    },
    /// Timing information
    Timing {
        /// Compilation time
        compile_time_ms: u64,
        /// Execution time
        execution_time_ms: u64,
    },
    /// Multiple outputs
    Multi {
        /// List of outputs
        outputs: Vec<CellOutput>,
    },
    /// Empty output (e.g., for statements that don't produce values)
    Empty,
}

/// Statistics for tensor display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorStats {
    /// Mean value
    pub mean: Option<f64>,
    /// Standard deviation
    pub std: Option<f64>,
    /// Minimum value
    pub min: Option<f64>,
    /// Maximum value
    pub max: Option<f64>,
    /// Number of NaN values
    pub nan_count: usize,
    /// Number of Inf values
    pub inf_count: usize,
}

impl CellOutput {
    /// Create a value output
    pub fn value(repr: impl Into<Text>, type_info: impl Into<Text>) -> Self {
        Self::Value {
            repr: repr.into(),
            type_info: type_info.into(),
            raw: None,
        }
    }

    /// Create a value output with raw value
    pub fn value_with_raw(repr: impl Into<Text>, type_info: impl Into<Text>, raw: Value) -> Self {
        Self::Value {
            repr: repr.into(),
            type_info: type_info.into(),
            raw: Some(raw),
        }
    }

    /// Create a tensor output
    pub fn tensor(
        shape: Vec<usize>,
        dtype: impl Into<Text>,
        preview: impl Into<Text>,
        stats: Option<TensorStats>,
    ) -> Self {
        Self::Tensor {
            shape,
            dtype: dtype.into(),
            preview: preview.into(),
            stats,
        }
    }

    /// Create a structured output
    pub fn structured(type_name: impl Into<Text>, fields: Vec<(Text, CellOutput)>) -> Self {
        Self::Structured {
            type_name: type_name.into(),
            fields,
        }
    }

    /// Create a collection output
    pub fn collection(
        len: usize,
        element_type: impl Into<Text>,
        preview: Vec<CellOutput>,
        truncated: bool,
    ) -> Self {
        Self::Collection {
            len,
            element_type: element_type.into(),
            preview,
            truncated,
        }
    }

    /// Create an error output
    pub fn error(message: impl Into<Text>) -> Self {
        Self::Error {
            message: message.into(),
            span: None,
            suggestions: Vec::new(),
        }
    }

    /// Create an error output with span
    pub fn error_with_span(message: impl Into<Text>, span: Span) -> Self {
        Self::Error {
            message: message.into(),
            span: Some(SerializableSpan::from(span)),
            suggestions: Vec::new(),
        }
    }

    /// Create an error output with suggestions
    pub fn error_with_suggestions(
        message: impl Into<Text>,
        span: Option<Span>,
        suggestions: Vec<Text>,
    ) -> Self {
        Self::Error {
            message: message.into(),
            span: span.map(SerializableSpan::from),
            suggestions,
        }
    }

    /// Create a stream output
    pub fn stream(stdout: impl Into<Text>) -> Self {
        Self::Stream {
            stdout: stdout.into(),
            stderr: Text::from(""),
        }
    }

    /// Create a stream output with stdout and stderr
    pub fn stream_with_stderr(stdout: impl Into<Text>, stderr: impl Into<Text>) -> Self {
        Self::Stream {
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    /// Create a timing output
    pub fn timing(compile_time: Duration, execution_time: Duration) -> Self {
        Self::Timing {
            compile_time_ms: compile_time.as_millis() as u64,
            execution_time_ms: execution_time.as_millis() as u64,
        }
    }

    /// Create a multi output
    pub fn multi(outputs: Vec<CellOutput>) -> Self {
        Self::Multi { outputs }
    }

    /// Check if this is an error
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Check if this is empty or unit
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Get the raw value if available
    pub fn raw_value(&self) -> Option<&Value> {
        match self {
            Self::Value { raw, .. } => raw.as_ref(),
            _ => None,
        }
    }
}

/// Serializable version of Span
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableSpan {
    pub start: u32,
    pub end: u32,
    pub file_id: u32,
}

impl From<Span> for SerializableSpan {
    fn from(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
            file_id: span.file_id.raw(),
        }
    }
}

/// Cell metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CellMetadata {
    /// Optional name/label for the cell
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Text>,
    /// Whether the cell is collapsed in the UI
    #[serde(default)]
    pub collapsed: bool,
    /// Custom tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Text>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_creation() {
        let code = Cell::new_code("let x = 42");
        assert!(code.is_code());
        assert!(code.dirty);

        let md = Cell::new_markdown("# Hello");
        assert!(md.is_markdown());
    }

    #[test]
    fn test_cell_output() {
        let mut cell = Cell::new_code("42");
        cell.set_output(CellOutput::value("42", "Int"), 1);

        assert!(!cell.dirty);
        assert!(cell.output.is_some());
        assert_eq!(cell.execution_count, Some(1));
    }
}
