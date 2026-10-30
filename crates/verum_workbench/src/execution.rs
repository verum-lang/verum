//! Validated operation contracts shared by browser, agent tools and receipts.
use crate::workspace::{SOURCE_LIMIT, digest};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use verum_common::Text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationKind {
    Run,
    Check,
    Proof,
}

/// The CLI owns permission parsing and enforcement. These are its override
/// modes, not an OS sandbox or a second permission implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunPermissions {
    Source,
    DenyAll,
    AllowAll,
}

impl RunPermissions {
    pub fn cli_flag(self) -> Option<&'static str> {
        match self {
            Self::Source => None,
            Self::DenyAll => Some("--deny-all"),
            Self::AllowAll => Some("--allow-all"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub kind: OperationKind,
    pub path: Text,
    pub source_hash: Text,
    pub executable: PathBuf,
    pub timeout_ms: u64,
    // Missing on old receipts means unrecorded, not a known permission mode.
    #[serde(default)]
    pub permissions: Option<RunPermissions>,
    #[serde(default)]
    pub declaration: Option<Text>,
}

impl ExecutionPlan {
    pub fn parse(params: &Value, verum: &Path, proof: &Path) -> Result<Self> {
        let kind: OperationKind = serde_json::from_value(params["kind"].clone())
            .context("Expected operation kind: run, check or proof")?;
        let source = params["source"].as_str().context("Expected source")?;
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds limit");
        let path = params["path"].as_str().context("Expected source path")?;
        let timeout_ms = match params.get("timeout_ms") {
            None => 60_000,
            Some(value) => value.as_u64().context("Timeout must be an integer")?,
        };
        ensure!(
            (1..=300_000).contains(&timeout_ms),
            "Timeout must be 1..300000 milliseconds"
        );
        let permissions = if kind == OperationKind::Run {
            Some(match params.get("permissions") {
                None => RunPermissions::Source,
                Some(value) => serde_json::from_value(value.clone())
                    .context("Run permissions must be source, deny-all or allow-all")?,
            })
        } else {
            ensure!(
                params.get("permissions").is_none(),
                "Permissions apply only to script runs"
            );
            None
        };
        let declaration = if kind == OperationKind::Proof {
            Some(
                params["declaration"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty())
                    .context("Choose a theorem declaration")?
                    .into(),
            )
        } else {
            ensure!(
                params.get("declaration").is_none(),
                "A declaration applies only to proof checks"
            );
            None
        };
        Ok(Self {
            kind,
            path: path.into(),
            source_hash: digest(source.as_bytes()),
            executable: if kind == OperationKind::Proof {
                proof
            } else {
                verum
            }
            .into(),
            timeout_ms,
            permissions,
            declaration,
        })
    }
}
