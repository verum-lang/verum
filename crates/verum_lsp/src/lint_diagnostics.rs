//! Project lint diagnostics from the same revision-bound report as Studio.
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Diagnostic, NumberOrString, Position, Range, Url};

/// Standalone callers ask about the saved revision. Editor callers supply their
/// already captured source so a concurrent file change cannot retarget findings.
pub async fn lint_diagnostics(uri: &Url, settings: &LintSettings) -> Vec<Diagnostic> {
    if !settings.enabled {
        return Vec::new();
    }
    let Ok(path) = uri.to_file_path() else {
        return Vec::new();
    };
    let Ok(source) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    for_source(uri, &source, settings)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "Workspace lint unavailable");
            Vec::new()
        })
}

pub(crate) async fn for_source(
    uri: &Url,
    source: &str,
    settings: &LintSettings,
) -> Result<Vec<Diagnostic>> {
    let Ok(path) = uri.to_file_path() else {
        return Ok(Vec::new());
    };
    let Some(root) = path.ancestors().skip(1).find(|dir| {
        [
            ".verum.toml",
            "verum.toml",
            "Verum.toml",
            ".verum/lint.toml",
        ]
        .iter()
        .any(|name| dir.join(name).is_file())
    }) else {
        return Ok(Vec::new());
    };
    let binary = settings.binary.as_deref().unwrap_or(Path::new("verum"));
    let report = crate::lint_report::inspect(binary, root, settings.profile.as_deref()).await?;
    let relative = path.strip_prefix(root)?;
    let hash = hex::encode(Sha256::digest(source.as_bytes()));
    let Some(doc) = report
        .files
        .iter()
        .find(|doc| doc.path == relative && doc.source_hash == hash)
    else {
        return Ok(Vec::new());
    };
    Ok(doc
        .findings
        .iter()
        .map(|issue| {
            let start = issue.position.unwrap_or(Position::new(0, 0));
            Diagnostic {
                range: Range::new(start, start),
                severity: Some(issue.severity),
                code: Some(NumberOrString::String(issue.rule.clone())),
                source: Some("verum-lint".into()),
                message: issue.message.clone(),
                ..Default::default()
            }
        })
        .collect())
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
}
