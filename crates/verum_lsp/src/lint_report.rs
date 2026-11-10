//! Saved-source lint inspection shared by language clients and Studio's agent tools.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock, Weak},
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};
use tower_lsp::lsp_types::{DiagnosticSeverity, Position, TextEdit};

#[derive(Debug, Serialize, Deserialize)]
pub struct LintReport {
    pub schema_version: u32,
    pub files: Vec<LintDocument>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LintDocument {
    pub path: PathBuf,
    pub source_hash: String,
    pub parsed: bool,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub position: Option<Position>,
    pub edits: Vec<TextEdit>,
}

async fn bounded(
    pipe: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(
            "Lint output exceeds the inspection limit",
        ));
    }
    Ok(bytes)
}

type InspectionKey = (PathBuf, PathBuf, Option<String>);
type Inspection = tokio::sync::OnceCell<Result<Arc<LintReport>, String>>;
static RUNNING: LazyLock<tokio::sync::Mutex<HashMap<InspectionKey, Weak<Inspection>>>> =
    LazyLock::new(Default::default);

/// Concurrent consumers of the same workspace share a child. After consumers
/// receive the result the weak entry expires, so a later run reads fresh files.
pub async fn inspect(binary: &Path, root: &Path, profile: Option<&str>) -> Result<Arc<LintReport>> {
    let key = (
        binary.to_owned(),
        root.to_owned(),
        profile.map(str::to_owned),
    );
    let scan = {
        let mut runs = RUNNING.lock().await;
        runs.retain(|_, entry| entry.strong_count() != 0);
        if let Some(scan) = runs.get(&key).and_then(Weak::upgrade) {
            scan
        } else {
            let scan = Arc::new(Inspection::new());
            runs.insert(key, Arc::downgrade(&scan));
            scan
        }
    };
    scan.get_or_init(|| async {
        run(binary, root, profile)
            .await
            .map(Arc::new)
            .map_err(|error| format!("{error:#}"))
    })
    .await
    .clone()
    .map_err(anyhow::Error::msg)
}

async fn run(binary: &Path, root: &Path, profile: Option<&str>) -> Result<LintReport> {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .args(["lint", "--inspect"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(profile) = profile {
        command.arg("--profile").arg(profile);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Launch {}", binary.display()))?;
    let stdout = child.stdout.take().context("Lint stdout unavailable")?;
    let stderr = child.stderr.take().context("Lint stderr unavailable")?;
    let (output, errors, status) = tokio::time::timeout(Duration::from_secs(90), async {
        tokio::try_join!(
            bounded(stdout, 16 << 20),
            bounded(stderr, 128 << 10),
            child.wait()
        )
    })
    .await
    .context("Workspace lint exceeded 90 seconds")??;
    ensure!(
        status.success(),
        "Workspace lint failed ({status}): {}",
        String::from_utf8_lossy(&errors).trim()
    );
    let report: LintReport =
        serde_json::from_slice(&output).context("Invalid workspace lint report")?;
    ensure!(
        report.schema_version == 1,
        "Unsupported workspace lint schema {}",
        report.schema_version
    );
    Ok(report)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[tokio::test]
    async fn simultaneous_clients_share_the_scan_and_a_later_failure_is_visible() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let command = root.join("lint-host");
        fs::write(
            &command,
            "#!/bin/sh\necho run >> runs\nsleep 0.05\ncat report\n",
        )
        .unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("report"), r#"{"schema_version":1,"files":[]}"#).unwrap();
        let (a, b) = tokio::join!(inspect(&command, root, None), inspect(&command, root, None));
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(fs::read_to_string(root.join("runs")).unwrap(), "run\n");
        fs::write(root.join("report"), r#"{"schema_version":2,"files":[]}"#).unwrap();
        assert!(
            inspect(&command, root, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("schema 2")
        );
        assert_eq!(fs::read_to_string(root.join("runs")).unwrap(), "run\nrun\n");
    }
}
