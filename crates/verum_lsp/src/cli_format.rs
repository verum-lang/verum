//! Subprocess-based formatter — single source of truth.
//!
//! For the same architectural reason as `lint_diagnostics` (the
//! formatter lives in `verum_cli`; `verum_cli` depends on
//! `verum_lsp` for the `verum lsp` subcommand; cycle), the LSP
//! defers to the binary through the `verum fmt --stdin` interface.
//! This guarantees `verum fmt` from the CLI and the editor's
//! "format document" command produce *byte-identical* output, not
//! "almost the same" output.
//!
//! Cost: a subprocess spawn (~30 ms cold on macOS, ~10 ms on
//! Linux). Format-on-save is human-paced, so this overhead is
//! invisible. The old in-LSP formatter remains as a fallback for
//! cases where the binary can't be located.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use tower_lsp::lsp_types::{Position, Range, TextEdit};
use verum_common::List;

/// Resolved formatter settings — mirrors the lint side. The
/// `binary` path is overridable so the editor can point at a
/// dev-build binary if needed.
#[derive(Debug, Clone, Default)]
pub struct FmtSettings {
    pub enabled: bool,
    pub binary: Option<PathBuf>,
}

impl FmtSettings {
    pub fn from_json(value: &serde_json::Value) -> Self {
        let mut out = Self {
            enabled: true,
            binary: None,
        };
        if let Some(obj) = value.as_object() {
            if let Some(b) = obj.get("enabled").and_then(|v| v.as_bool()) {
                out.enabled = b;
            }
            if let Some(s) = obj.get("binary").and_then(|v| v.as_str()) {
                out.binary = Some(PathBuf::from(s));
            }
        }
        out
    }
}

/// Format `text` by piping it through `verum fmt --stdin`. Returns
/// `None` on any subprocess failure so the caller can fall back to
/// the in-LSP formatter without breaking the user's save flow.
pub async fn format_via_cli(
    text: &str,
    settings: &FmtSettings,
    filename_hint: Option<&str>,
) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    let binary = settings.binary.clone().unwrap_or_else(|| "verum".into());
    let buffer = text.to_string();
    let hint = filename_hint.map(|s| s.to_string());

    let stdout_bytes = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
        let mut cmd = Command::new(&binary);
        cmd.arg("fmt").arg("--stdin");
        if let Some(h) = &hint {
            cmd.arg("--stdin-filename").arg(h);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().ok()?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(buffer.as_bytes()).ok()?;
        }
        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(output.stdout)
    })
    .await
    .ok()??;

    String::from_utf8(stdout_bytes).ok()
}

/// Build a single full-document `TextEdit` if the formatted output
/// differs from the original; otherwise return an empty list (the
/// LSP convention for "no changes needed").
pub fn diff_to_text_edits(original: &str, formatted: &str) -> List<TextEdit> {
    if original == formatted {
        return List::new();
    }
    let mut edits = List::new();
    edits.push(TextEdit {
        range: Range {
            start: Position { line: 0, character: 0 },
            end: Position {
                line: u32::MAX,
                character: u32::MAX,
            },
        },
        new_text: formatted.to_string(),
    });
    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fmt_settings_defaults_enabled() {
        let s = FmtSettings::from_json(&serde_json::Value::Null);
        assert!(s.enabled);
    }

    #[test]
    fn fmt_settings_disabled_via_json() {
        let s = FmtSettings::from_json(&json!({"enabled": false}));
        assert!(!s.enabled);
    }

    #[test]
    fn diff_returns_empty_for_unchanged() {
        let edits = diff_to_text_edits("fn main() {}\n", "fn main() {}\n");
        assert_eq!(edits.len(), 0);
    }

    #[test]
    fn diff_returns_full_replace_for_changed() {
        let edits = diff_to_text_edits("fn main(){}", "fn main() {}\n");
        assert_eq!(edits.len(), 1);
    }
}
