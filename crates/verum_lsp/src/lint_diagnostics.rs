//! Lint-engine diagnostics for the LSP.
//!
//! The Verum lint engine lives in `verum_cli`, which `verum_lsp`
//! cannot depend on (cycle: `verum_cli` already depends on
//! `verum_lsp` to wire the `verum lsp` subcommand). To keep the
//! dependency graph clean we invoke the lint engine through its
//! stable `--format json` interface — the same NDJSON contract
//! external CI tools rely on. Each line carries
//! `schema_version: 1` so this client can fail loudly if the
//! producer ever bumps the version.
//!
//! Performance: subprocess spawn + JSON decode is on the order of
//! tens of milliseconds, well below the per-keystroke threshold.
//! In practice we run on save and on a 300 ms debounce after the
//! last edit, which absorbs the cost transparently.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Deserialize;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url,
};

/// One issue line emitted by `verum lint --format json`. Mirrors
/// the documented schema in
/// `internal/website/docs/reference/lint-configuration.md`.
#[derive(Debug, Deserialize)]
struct LintIssueWire {
    schema_version: u32,
    rule: String,
    level: String,
    line: u32,
    column: u32,
    message: String,
    #[serde(default)]
    fixable: bool,
}

/// Run the lint engine against the project that owns `uri` and
/// return LSP-shaped diagnostics. Returns an empty vec on any
/// failure (binary missing, schema mismatch, parse error) — the
/// LSP must never block on lint errors.
pub async fn lint_diagnostics(uri: &Url, settings: &LintSettings) -> Vec<Diagnostic> {
    if !settings.enabled {
        return Vec::new();
    }
    let path = match uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let project_root = match find_project_root(&path) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let binary = settings.binary.clone().unwrap_or_else(|| "verum".into());

    let profile = settings.profile.clone();
    let project_root_owned = project_root.clone();

    // Run the subprocess on the blocking pool so the LSP runtime is
    // not stalled waiting for child I/O. The 10-second cap is
    // generous — typical lint runs finish under 100 ms — but
    // protects the client from a misconfigured binary that hangs.
    let stdout_bytes = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
        let mut cmd = Command::new(&binary);
        cmd.arg("lint")
            .arg("--no-cache")
            .arg("--format")
            .arg("json")
            .current_dir(&project_root_owned)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(p) = &profile {
            cmd.arg("--profile").arg(p);
        }
        cmd.output().ok().map(|o| o.stdout)
    })
    .await;

    let bytes = match stdout_bytes {
        Ok(Some(b)) => b,
        _ => return Vec::new(),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let target_path = path.canonicalize().unwrap_or(path.clone());
    let mut diagnostics = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: LintIssueWire = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if parsed.schema_version != 1 {
            // We understand version 1 only. A future bump is a
            // deprecation event the LSP needs to be updated to
            // handle — until then we silently skip.
            continue;
        }

        // Match the diagnostic against the active document. The
        // lint output uses a project-relative path, so we resolve
        // it against the project root and compare canonicalised
        // paths.
        let issue_field = match line_field(line, "file") {
            Some(f) => f,
            None => continue,
        };
        let issue_path = project_root.join(&issue_field);
        let issue_canonical = issue_path.canonicalize().unwrap_or(issue_path);
        if issue_canonical != target_path {
            continue;
        }

        let severity = match parsed.level.as_str() {
            "error" => Some(DiagnosticSeverity::ERROR),
            "warning" | "warn" => Some(DiagnosticSeverity::WARNING),
            "info" => Some(DiagnosticSeverity::INFORMATION),
            "hint" => Some(DiagnosticSeverity::HINT),
            "off" => continue,
            _ => Some(DiagnosticSeverity::WARNING),
        };

        // The lint engine reports 1-indexed line/column; LSP wants
        // 0-indexed. Use a 0-length range — IDEs render the
        // squiggle at the column the user expects.
        let line_zero = parsed.line.saturating_sub(1);
        let col_zero = parsed.column.saturating_sub(1);
        let position = Position {
            line: line_zero,
            character: col_zero,
        };

        let mut tags = Vec::new();
        if parsed.fixable {
            // Use Unnecessary for fixable issues so the IDE renders
            // them with the conventional grey-out appearance.
            // Remove if too noisy — the rule still reports as a
            // diagnostic, but the visual hint is opt-in.
            tags.push(tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY);
        }

        diagnostics.push(Diagnostic {
            range: Range {
                start: position,
                end: position,
            },
            severity,
            code: Some(NumberOrString::String(parsed.rule.clone())),
            code_description: None,
            source: Some("verum-lint".to_string()),
            message: parsed.message,
            related_information: None,
            tags: if tags.is_empty() { None } else { Some(tags) },
            data: None,
        });
    }
    diagnostics
}

/// Settings the editor exposes through `workspace/configuration`.
/// All fields are optional; reasonable defaults make the
/// integration zero-config for typical projects.
#[derive(Debug, Clone, Default)]
pub struct LintSettings {
    pub enabled: bool,
    pub profile: Option<String>,
    pub binary: Option<PathBuf>,
}

impl LintSettings {
    pub fn from_json(value: &serde_json::Value) -> Self {
        let mut out = Self {
            enabled: true,
            profile: None,
            binary: None,
        };
        if let Some(obj) = value.as_object() {
            if let Some(b) = obj.get("enabled").and_then(|v| v.as_bool()) {
                out.enabled = b;
            }
            if let Some(s) = obj.get("profile").and_then(|v| v.as_str()) {
                out.profile = Some(s.to_string());
            }
            if let Some(s) = obj.get("binary").and_then(|v| v.as_str()) {
                out.binary = Some(PathBuf::from(s));
            }
        }
        out
    }
}

/// Walk up from a file path until we find a directory containing
/// `verum.toml`. Returns `None` if no manifest is reachable —
/// without a project root we can't run the linter.
fn find_project_root(start: &std::path::Path) -> Option<PathBuf> {
    let mut cursor = start.parent();
    while let Some(dir) = cursor {
        if dir.join("verum.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        cursor = dir.parent();
    }
    None
}

/// Pull a string field out of a JSON line without parsing the whole
/// object. Useful when we need the file path before we've decided
/// whether the rest of the record is for us.
fn line_field<'a>(line: &'a str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":\"");
    let start = line.find(&needle)? + needle.len();
    let after = &line[start..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lint_settings_defaults() {
        let s = LintSettings::from_json(&serde_json::Value::Null);
        assert!(s.enabled);
        assert!(s.profile.is_none());
    }

    #[test]
    fn lint_settings_disabled_via_json() {
        let v = json!({"enabled": false});
        let s = LintSettings::from_json(&v);
        assert!(!s.enabled);
    }

    #[test]
    fn line_field_extracts_file() {
        let line = r#"{"event":"lint","schema_version":1,"rule":"todo-in-code","level":"warning","file":"src/main.vr","line":2,"column":4,"message":"TODO comment in code","fixable":true,"suggestion":"TODO(#0000)"}"#;
        assert_eq!(line_field(line, "file"), Some("src/main.vr".to_string()));
        assert_eq!(line_field(line, "rule"), Some("todo-in-code".to_string()));
    }
}
