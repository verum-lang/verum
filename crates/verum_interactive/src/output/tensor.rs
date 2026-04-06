//! Tensor output rendering.
//!
//! Provides rich visualization for tensor data including:
//! - Shape and dtype display
//! - Statistical summaries (mean, std, min, max)
//! - Smart truncation for large tensors
//! - 2D matrix formatting

use verum_common::Text;
use verum_vbc::value::Value;

use super::renderer::{OutputFormat, OutputRenderer, RenderedOutput};

/// Tensor statistics for display.
#[derive(Debug, Clone)]
pub struct TensorStats {
    /// Shape of the tensor.
    pub shape: Vec<usize>,
    /// Data type (e.g., "Float32", "Int64").
    pub dtype: Text,
    /// Total number of elements.
    pub numel: usize,
    /// Mean value (for numeric tensors).
    pub mean: Option<f64>,
    /// Standard deviation (for numeric tensors).
    pub std: Option<f64>,
    /// Minimum value.
    pub min: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// Number of NaN values.
    pub nan_count: usize,
    /// Number of Inf values.
    pub inf_count: usize,
}

impl TensorStats {
    /// Creates stats for an empty tensor.
    pub fn empty(dtype: impl Into<Text>) -> Self {
        Self {
            shape: vec![0],
            dtype: dtype.into(),
            numel: 0,
            mean: None,
            std: None,
            min: None,
            max: None,
            nan_count: 0,
            inf_count: 0,
        }
    }

    /// Formats the shape as a string.
    pub fn shape_str(&self) -> String {
        format!(
            "[{}]",
            self.shape
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Formats the full tensor type signature.
    pub fn type_signature(&self) -> String {
        format!("Tensor<{}, {}>", self.dtype.as_str(), self.shape_str())
    }

    /// Formats statistics as a summary string.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(mean) = self.mean {
            parts.push(format!("mean={:.4}", mean));
        }
        if let Some(std) = self.std {
            parts.push(format!("std={:.4}", std));
        }
        if let Some(min) = self.min {
            parts.push(format!("min={:.4}", min));
        }
        if let Some(max) = self.max {
            parts.push(format!("max={:.4}", max));
        }
        if self.nan_count > 0 {
            parts.push(format!("nan={}", self.nan_count));
        }
        if self.inf_count > 0 {
            parts.push(format!("inf={}", self.inf_count));
        }

        parts.join(", ")
    }
}

/// Preview options for tensor display.
#[derive(Debug, Clone)]
pub struct TensorPreview {
    /// Maximum elements to show per dimension.
    pub max_elements_per_dim: usize,
    /// Maximum total elements to display.
    pub max_total_elements: usize,
    /// Number of decimal places for floats.
    pub precision: usize,
    /// Whether to show statistics.
    pub show_stats: bool,
    /// Whether to use scientific notation for large/small values.
    pub scientific_notation_threshold: f64,
}

impl Default for TensorPreview {
    fn default() -> Self {
        Self {
            max_elements_per_dim: 6,
            max_total_elements: 100,
            precision: 4,
            show_stats: true,
            scientific_notation_threshold: 1e6,
        }
    }
}

/// Renders a tensor value to output.
pub fn render_tensor(
    stats: &TensorStats,
    data_preview: Option<&[f64]>,
    options: &TensorPreview,
    format: OutputFormat,
) -> RenderedOutput {
    let ndim = stats.shape.len();

    // Build the text output
    let mut text = String::new();

    // Type signature
    text.push_str(&stats.type_signature());
    text.push('\n');

    // Statistics (if enabled)
    if options.show_stats && stats.numel > 0 {
        text.push_str("  ");
        text.push_str(&stats.summary());
        text.push('\n');
    }

    // Data preview
    if let Some(data) = data_preview {
        text.push_str(&format_tensor_data(data, &stats.shape, options));
    }

    // Preview for collapsed view
    let preview = format!(
        "{}: {} elements",
        stats.type_signature(),
        stats.numel
    );

    // Format-specific rendering
    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_tensor(&text, stats)),
        OutputFormat::Html => Some(html_format_tensor(&text, stats)),
        _ => None,
    };

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(stats.type_signature().as_str()),
        collapsible: stats.numel > options.max_total_elements,
        preview: Some(Text::from(preview.as_str())),
    }
}

/// Formats tensor data as a string.
fn format_tensor_data(data: &[f64], shape: &[usize], options: &TensorPreview) -> String {
    if shape.is_empty() || data.is_empty() {
        return "[]".to_string();
    }

    let ndim = shape.len();

    match ndim {
        1 => format_1d(data, shape[0], options),
        2 => format_2d(data, shape[0], shape[1], options),
        _ => format_nd(data, shape, options),
    }
}

/// Formats a 1D tensor.
fn format_1d(data: &[f64], len: usize, options: &TensorPreview) -> String {
    let show = len.min(options.max_elements_per_dim * 2);
    let half = options.max_elements_per_dim;

    let mut parts = Vec::new();

    if len <= options.max_elements_per_dim * 2 {
        // Show all elements
        for &v in data.iter().take(show) {
            parts.push(format_number(v, options));
        }
    } else {
        // Show first and last
        for &v in data.iter().take(half) {
            parts.push(format_number(v, options));
        }
        parts.push("...".to_string());
        for &v in data.iter().skip(len - half) {
            parts.push(format_number(v, options));
        }
    }

    format!("[{}]", parts.join(", "))
}

/// Formats a 2D tensor (matrix).
fn format_2d(data: &[f64], rows: usize, cols: usize, options: &TensorPreview) -> String {
    let show_rows = rows.min(options.max_elements_per_dim);
    let show_cols = cols.min(options.max_elements_per_dim * 2);

    let mut lines = Vec::new();
    lines.push("[".to_string());

    for i in 0..show_rows {
        let row_start = i * cols;
        let row_data: Vec<f64> = if cols <= options.max_elements_per_dim * 2 {
            data[row_start..row_start + cols].to_vec()
        } else {
            let half = options.max_elements_per_dim;
            let mut v = data[row_start..row_start + half].to_vec();
            v.extend_from_slice(&data[row_start + cols - half..row_start + cols]);
            v
        };

        let row_str = format_1d(&row_data, row_data.len(), options);
        let prefix = if i == 0 { " " } else { "  " };
        lines.push(format!("{}{}{}",
            prefix,
            row_str,
            if i < show_rows - 1 || rows > show_rows { "," } else { "" }
        ));
    }

    if rows > show_rows {
        lines.push(format!("  ... ({} more rows)", rows - show_rows));
    }

    lines.push("]".to_string());
    lines.join("\n")
}

/// Formats an N-dimensional tensor.
fn format_nd(data: &[f64], shape: &[usize], options: &TensorPreview) -> String {
    // For higher dimensions, just show shape and sample
    let sample_size = 10.min(data.len());
    let sample: Vec<String> = data
        .iter()
        .take(sample_size)
        .map(|&v| format_number(v, options))
        .collect();

    format!(
        "shape={:?}\ndata=[{}, ...]",
        shape,
        sample.join(", ")
    )
}

/// Formats a single number.
fn format_number(v: f64, options: &TensorPreview) -> String {
    if v.is_nan() {
        "nan".to_string()
    } else if v.is_infinite() {
        if v.is_sign_positive() { "inf".to_string() } else { "-inf".to_string() }
    } else if v.abs() >= options.scientific_notation_threshold || (v != 0.0 && v.abs() < 1e-4) {
        format!("{:.*e}", options.precision.min(2), v)
    } else {
        format!("{:.*}", options.precision, v)
    }
}

/// Colorizes tensor output for ANSI terminal.
fn colorize_tensor(text: &str, stats: &TensorStats) -> String {
    // Use cyan for the type signature, dim for stats
    let lines: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("\x1b[36m{}\x1b[0m", line) // Cyan for type
            } else if line.starts_with("  mean=") || line.starts_with("  std=") {
                format!("\x1b[2m{}\x1b[0m", line) // Dim for stats
            } else {
                line.to_string()
            }
        })
        .collect();

    lines.join("\n")
}

/// Formats tensor output as HTML.
fn html_format_tensor(text: &str, stats: &TensorStats) -> String {
    let lines: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let escaped = line
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");

            if i == 0 {
                format!("<span class=\"tensor-type\">{}<!-- raw HTML omitted --></span>", escaped)
            } else if line.starts_with("  mean=") || line.starts_with("  std=") {
                format!("<span class=\"tensor-stats\">{}<!-- raw HTML omitted --></span>", escaped)
            } else {
                escaped
            }
        })
        .collect();

    format!("<pre class=\"tensor\">{}</pre>", lines.join("\n"))
}

/// Tensor renderer for the output registry.
pub struct TensorRenderer {
    options: TensorPreview,
}

impl Default for TensorRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TensorRenderer {
    pub fn new() -> Self {
        Self {
            options: TensorPreview::default(),
        }
    }

    pub fn with_options(options: TensorPreview) -> Self {
        Self { options }
    }
}

impl OutputRenderer for TensorRenderer {
    fn render(&self, value: &Value, type_info: &Text, format: OutputFormat) -> RenderedOutput {
        // Check if this is a tensor type
        // In a real implementation, we'd extract tensor data from the value
        // For now, we create a placeholder

        let stats = TensorStats::empty("Float64");
        render_tensor(&stats, None, &self.options, format)
    }

    fn can_render(&self, _value: &Value, type_info: &Text) -> bool {
        type_info.as_str().starts_with("Tensor<")
    }

    fn priority(&self) -> u32 {
        100 // High priority for tensors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tensor_stats_shape_str() {
        let stats = TensorStats {
            shape: vec![2, 3, 4],
            dtype: Text::from("Float32"),
            numel: 24,
            mean: Some(1.0),
            std: Some(0.5),
            min: Some(0.0),
            max: Some(2.0),
            nan_count: 0,
            inf_count: 0,
        };

        assert_eq!(stats.shape_str(), "[2, 3, 4]");
        assert_eq!(stats.type_signature(), "Tensor<Float32, [2, 3, 4]>");
    }

    #[test]
    fn test_format_1d() {
        let options = TensorPreview::default();
        let data = vec![1.0, 2.0, 3.0];
        let result = format_1d(&data, 3, &options);
        assert!(result.contains("1.0"));
        assert!(result.contains("2.0"));
        assert!(result.contains("3.0"));
    }

    #[test]
    fn test_format_number() {
        let options = TensorPreview::default();

        assert_eq!(format_number(42.0, &options), "42.0000");
        assert_eq!(format_number(f64::NAN, &options), "nan");
        assert_eq!(format_number(f64::INFINITY, &options), "inf");
        assert!(format_number(1e10, &options).contains('e'));
    }

    #[test]
    fn test_tensor_stats_summary() {
        let stats = TensorStats {
            shape: vec![10],
            dtype: Text::from("Float64"),
            numel: 10,
            mean: Some(5.5),
            std: Some(2.87),
            min: Some(1.0),
            max: Some(10.0),
            nan_count: 0,
            inf_count: 0,
        };

        let summary = stats.summary();
        assert!(summary.contains("mean=5.5"));
        assert!(summary.contains("std=2.87"));
    }
}
