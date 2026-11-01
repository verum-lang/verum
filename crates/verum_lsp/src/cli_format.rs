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

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

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
    // Cancellation drops and kills the child. Concurrent input/output avoids
    // pipe deadlocks; bounded output and time also cover misconfigured binaries.
    let operation = async {
        const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
        let mut cmd = Command::new(&binary);
        cmd.arg("fmt").arg("--stdin");
        if let Some(h) = filename_hint {
            cmd.arg("--stdin-filename").arg(h);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().ok()?;
        let mut stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (_, output, status) = tokio::try_join!(
            async {
                stdin.write_all(text.as_bytes()).await?;
                drop(stdin);
                Ok::<_, std::io::Error>(())
            },
            async {
                let mut output = Vec::new();
                stdout
                    .take(OUTPUT_LIMIT + 1)
                    .read_to_end(&mut output)
                    .await?;
                if output.len() as u64 > OUTPUT_LIMIT {
                    return Err(std::io::Error::other("Formatter output exceeds limit"));
                }
                Ok(output)
            },
            child.wait(),
        )
        .ok()?;
        status
            .success()
            .then(|| String::from_utf8(output).ok())
            .flatten()
    };
    tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .ok()?
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
            start: Position {
                line: 0,
                character: 0,
            },
            end: verum_common::span_utils::lsp::offset_to_lsp_position(original.len(), original),
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
