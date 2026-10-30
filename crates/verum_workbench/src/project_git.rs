//! Read-only Git adapter; no shell, external diff driver, hooks or index writes.
use crate::{project_files::exposed, workspace::Workspace};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};
use verum_common::{List, Text};

const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Change {
    pub path: Text,
    pub index: Text,
    pub worktree: Text,
    pub original_path: Option<Text>,
}
#[derive(Serialize)]
pub struct Changes {
    pub state: &'static str,
    pub branch: Option<Text>,
    pub entries: List<Change>,
    pub error: Option<Text>,
}
async fn git(root: &Path, args: &[&str]) -> Result<List<u8>> {
    let mut command = Command::new("git");
    command
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // A parent shell's Git targeting variables must not redirect the workspace.
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
            command.env_remove(name);
        }
    }
    let mut child = command.spawn().context("Git is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .context("Git stdout")?
        .take(OUTPUT_LIMIT + 1);
    tokio::time::timeout(Duration::from_secs(5), async {
        // Tokio's byte reader takes a Vec at the process boundary.
        let mut bytes = vec![];
        stdout.read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() as u64 <= OUTPUT_LIMIT,
            "Git output exceeds the review limit"
        );
        ensure!(
            child.wait().await?.success(),
            "Git could not inspect this workspace"
        );
        Ok(bytes.into())
    })
    .await
    .context("Git inspection exceeded five seconds")?
}
pub fn parse_status(bytes: &[u8], prefix: &str) -> Result<(Option<Text>, List<Change>)> {
    let mut fields = bytes.split(|b| *b == 0).filter(|field| !field.is_empty());
    let mut branch = None;
    let mut entries = List::new();
    while let Some(field) = fields.next() {
        let record = std::str::from_utf8(field).context("Git paths must be UTF-8")?;
        if let Some(header) = record.strip_prefix("## ") {
            branch = Some(header.into());
            continue;
        }
        ensure!(
            field.len() >= 4 && field[2] == b' ' && field[..2].is_ascii(),
            "Malformed Git status"
        );
        let original = if matches!(field[0], b'R' | b'C') || matches!(field[1], b'R' | b'C') {
            Some(std::str::from_utf8(
                fields.next().context("Missing Git rename source")?,
            )?)
        } else {
            None
        };
        let Some(path) = record[3..]
            .strip_prefix(prefix)
            .filter(|path| exposed(path))
        else {
            continue;
        };
        entries.push(Change {
            path: path.into(),
            index: record[..1].into(),
            worktree: record[1..2].into(),
            original_path: original
                .and_then(|path| path.strip_prefix(prefix))
                .filter(|path| exposed(path))
                .map(Into::into),
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((branch, entries))
}
pub async fn changes(root: &Path) -> Changes {
    if !root.ancestors().any(|path| path.join(".git").exists()) {
        return Changes {
            state: "not_repository",
            branch: None,
            entries: List::new(),
            error: None,
        };
    }
    let result = async {
        let prefix = git(root, &["rev-parse", "--show-prefix"]).await?;
        let prefix = std::str::from_utf8(&prefix)?.trim_end_matches('\n');
        let bytes = git(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--",
                ".",
            ],
        )
        .await?;
        parse_status(&bytes, prefix)
    }
    .await;
    match result {
        Ok((branch, entries)) => Changes {
            state: "ready",
            branch,
            entries,
            error: None,
        },
        Err(error) => Changes {
            state: "unavailable",
            branch: None,
            entries: List::new(),
            error: Some(format!("{error:#}").into()),
        },
    }
}
pub async fn diff(root: &Path, path: &str, side: &str) -> Result<Text> {
    ensure!(exposed(path), "Select an exposed project path");
    Workspace::open(root)?.document_path(path)?;
    let mut args: List<_> = [
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
    ]
    .into_iter()
    .collect();
    match side {
        "index" => args.push("--cached"),
        "worktree" => {}
        _ => bail!("Diff side must be index or worktree"),
    }
    args.extend(["--", path]);
    Ok(Text::from_utf8_slice(&git(root, &args).await?).context("Diff is not UTF-8 text")?)
}
