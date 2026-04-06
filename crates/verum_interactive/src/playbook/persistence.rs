//! Persistence for Playbook files (.vrbook format)

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::session::Cell;

/// Playbook file format version
const FORMAT_VERSION: u32 = 1;

/// Playbook settings persisted with the file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybookSettings {
    #[serde(default)]
    pub auto_save_interval_secs: u64,
    #[serde(default = "default_keybinding_mode")]
    pub keybinding_mode: String,
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_timeout")]
    pub execution_timeout_ms: u64,
}

fn default_keybinding_mode() -> String { "standard".into() }
fn default_true() -> bool { true }
fn default_timeout() -> u64 { 5000 }

/// Root structure for .vrbook files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookFile {
    /// Format version
    pub version: u32,
    /// Playbook metadata
    pub metadata: PlaybookMetadata,
    /// Cells in order
    pub cells: Vec<Cell>,
    /// Persisted settings
    #[serde(default)]
    pub settings: Option<PlaybookSettings>,
}

/// Playbook metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaybookMetadata {
    /// Playbook title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Creation timestamp (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Last modified timestamp (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// Verum version used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verum_version: Option<String>,
}

/// Load a playbook from a file
pub fn load_playbook(path: &Path) -> io::Result<(Vec<Cell>, Option<PlaybookSettings>)> {
    let content = fs::read_to_string(path)?;

    // Try to parse as PlaybookFile
    let playbook: PlaybookFile = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // Check version compatibility
    if playbook.version > FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Playbook version {} is newer than supported version {}",
                playbook.version, FORMAT_VERSION
            ),
        ));
    }

    Ok((playbook.cells, playbook.settings))
}

/// Save a playbook to a file
pub fn save_playbook(path: &Path, cells: &[Cell], settings: Option<&PlaybookSettings>) -> io::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();

    let playbook = PlaybookFile {
        version: FORMAT_VERSION,
        metadata: PlaybookMetadata {
            title: path.file_stem().and_then(|s| s.to_str()).map(String::from),
            author: None,
            created: None, // Would be set from existing file if loading
            modified: Some(now),
            verum_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
        cells: cells.to_vec(),
        settings: settings.cloned(),
    };

    let content = serde_json::to_string_pretty(&playbook)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    fs::write(path, content)
}

/// Export playbook to plain Verum source
pub fn export_to_verum(cells: &[Cell]) -> String {
    let mut output = String::new();

    for cell in cells {
        if cell.is_code() {
            output.push_str(cell.source.as_str());
            output.push_str("\n\n");
        } else {
            // Convert markdown to comments
            for line in cell.source.as_str().lines() {
                output.push_str("// ");
                output.push_str(line);
                output.push('\n');
            }
            output.push('\n');
        }
    }

    output
}

/// Export playbook to Markdown format
pub fn export_to_markdown(cells: &[Cell]) -> String {
    use super::session::CellOutput;
    let mut output = String::new();

    for cell in cells {
        if cell.is_code() {
            output.push_str("```verum\n");
            output.push_str(cell.source.as_str());
            output.push_str("\n```\n\n");

            if let Some(cell_output) = &cell.output {
                let formatted = format_output_for_export(cell_output);
                if !formatted.is_empty() {
                    for line in formatted.lines() {
                        output.push_str("> ");
                        output.push_str(line);
                        output.push('\n');
                    }
                    output.push('\n');
                }
            }
        } else {
            output.push_str(cell.source.as_str());
            output.push_str("\n\n");
        }
    }

    output
}

/// Export playbook to standalone HTML with dark theme
pub fn export_to_html(cells: &[Cell]) -> String {
    use super::session::CellOutput;
    let mut html = String::from(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Verum Playbook</title>
<style>
  body { background: #1a1a2e; color: #e0e0e0; font-family: 'Segoe UI', Tahoma, sans-serif; max-width: 900px; margin: 0 auto; padding: 2rem; }
  h1, h2, h3 { color: #00d4ff; }
  pre { background: #16213e; border: 1px solid #0f3460; border-radius: 6px; padding: 1rem; overflow-x: auto; }
  code { font-family: 'Fira Code', 'Consolas', monospace; font-size: 0.9em; }
  .output { background: #0f3460; border-left: 3px solid #e94560; padding: 0.5rem 1rem; margin: 0.5rem 0 1.5rem; border-radius: 0 4px 4px 0; color: #a0e0a0; }
  .error { border-left-color: #ff4444; color: #ff6666; }
  .cell { margin-bottom: 1.5rem; }
  .markdown { line-height: 1.6; }
  blockquote { border-left: 3px solid #0f3460; margin-left: 0; padding-left: 1rem; color: #888; }
</style>
</head>
<body>
<h1>Verum Playbook</h1>
"#);

    for cell in cells {
        html.push_str("<div class=\"cell\">\n");
        if cell.is_code() {
            html.push_str("<pre><code>");
            html.push_str(&html_escape(cell.source.as_str()));
            html.push_str("</code></pre>\n");

            if let Some(cell_output) = &cell.output {
                let formatted = format_output_for_export(cell_output);
                if !formatted.is_empty() {
                    let class = if matches!(cell_output, CellOutput::Error { .. }) {
                        "output error"
                    } else {
                        "output"
                    };
                    html.push_str(&format!("<div class=\"{}\"><pre>", class));
                    html.push_str(&html_escape(&formatted));
                    html.push_str("</pre></div>\n");
                }
            }
        } else {
            html.push_str("<div class=\"markdown\">");
            // Simple markdown: convert headers and paragraphs
            for line in cell.source.as_str().lines() {
                if let Some(rest) = line.strip_prefix("### ") {
                    html.push_str(&format!("<h3>{}</h3>\n", html_escape(rest)));
                } else if let Some(rest) = line.strip_prefix("## ") {
                    html.push_str(&format!("<h2>{}</h2>\n", html_escape(rest)));
                } else if let Some(rest) = line.strip_prefix("# ") {
                    html.push_str(&format!("<h1>{}</h1>\n", html_escape(rest)));
                } else if let Some(rest) = line.strip_prefix("> ") {
                    html.push_str(&format!("<blockquote>{}</blockquote>\n", html_escape(rest)));
                } else if line.trim().is_empty() {
                    html.push_str("<br>\n");
                } else {
                    html.push_str(&format!("<p>{}</p>\n", html_escape(line)));
                }
            }
            html.push_str("</div>\n");
        }
        html.push_str("</div>\n");
    }

    html.push_str("</body>\n</html>\n");
    html
}

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format a CellOutput for text export (markdown/html)
fn format_output_for_export(output: &super::session::CellOutput) -> String {
    use super::session::CellOutput;
    match output {
        CellOutput::Value { repr, type_info, .. } => {
            format!("{} : {}", repr, type_info)
        }
        CellOutput::Stream { stdout, stderr, .. } => {
            let mut s = stdout.to_string();
            if !stderr.is_empty() {
                if !s.is_empty() { s.push('\n'); }
                s.push_str(&format!("[stderr] {}", stderr));
            }
            s
        }
        CellOutput::Error { message, .. } => {
            format!("Error: {}", message)
        }
        CellOutput::Timing { compile_time_ms, execution_time_ms } => {
            format!("compile: {}ms, exec: {}ms", compile_time_ms, execution_time_ms)
        }
        CellOutput::Multi { outputs } => {
            outputs.iter()
                .map(|o| format_output_for_export(o))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        CellOutput::Empty => String::new(),
        CellOutput::Tensor { shape, dtype, preview, .. } => {
            format!("Tensor<{}> {:?}: {}", dtype, shape, preview)
        }
        CellOutput::Structured { type_name, fields, .. } => {
            let field_strs: Vec<String> = fields.iter()
                .map(|(k, v)| format!("{}: {}", k, format_output_for_export(v)))
                .collect();
            format!("{} {{ {} }}", type_name, field_strs.join(", "))
        }
        CellOutput::Collection { len, element_type, .. } => {
            format!("[{} x {}]", element_type, len)
        }
    }
}

/// Import from plain Verum source (splits by blank lines)
pub fn import_from_verum(source: &str) -> Vec<Cell> {
    let mut cells = Vec::new();
    let mut current_block = String::new();
    let mut is_comment_block = false;

    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            // End of block
            if !current_block.trim().is_empty() {
                let cell = if is_comment_block {
                    // Remove comment prefixes and create markdown cell
                    let markdown = current_block
                        .lines()
                        .map(|l| l.trim_start_matches("//").trim_start())
                        .collect::<Vec<_>>()
                        .join("\n");
                    Cell::new_markdown(markdown.as_str())
                } else {
                    Cell::new_code(current_block.trim())
                };
                cells.push(cell);
            }
            current_block.clear();
            is_comment_block = false;
        } else {
            if current_block.is_empty() {
                is_comment_block = trimmed.starts_with("//");
            }
            current_block.push_str(line);
            current_block.push('\n');
        }
    }

    // Handle last block
    if !current_block.trim().is_empty() {
        let cell = if is_comment_block {
            let markdown = current_block
                .lines()
                .map(|l| l.trim_start_matches("//").trim_start())
                .collect::<Vec<_>>()
                .join("\n");
            Cell::new_markdown(markdown.as_str())
        } else {
            Cell::new_code(current_block.trim())
        };
        cells.push(cell);
    }

    if cells.is_empty() {
        cells.push(Cell::new_code(""));
    }

    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_import_roundtrip() {
        let cells = vec![
            Cell::new_code("let x = 42"),
            Cell::new_markdown("# Header"),
            Cell::new_code("let y = x + 1"),
        ];

        let exported = export_to_verum(&cells);
        let imported = import_from_verum(&exported);

        assert_eq!(imported.len(), 3);
        assert!(imported[0].is_code());
        assert!(imported[1].is_markdown());
        assert!(imported[2].is_code());
    }
}
