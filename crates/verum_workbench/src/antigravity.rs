//! Local profile used by the official Antigravity CLI's ACP server.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tokio::process::Command;

pub(crate) const AUTH_METHOD: &str = "oauth-personal";
const AGENT: &str = "verum-conversation";

// Google always includes send_message, even with an empty native tool list.
// Its strict tool policy asks the client before every tool action; ACP cancels
// these requests. No personal/workspace extensions or MCP servers are inherited.
const AGENT_PROFILE: &str = "---\n\
name: verum-conversation\n\
description: A text conversation in Verum Studio.\n\
mainAgent: true\n\
subagent: false\n\
tools: []\n\
skills: []\n\
inheritMcp: false\n\
inheritCustomizations: false\n\
---\n\
Reply with text.\n";

fn write_config(path: &Path, content: &[u8]) -> Result<()> {
    let mut replacement =
        tempfile::NamedTempFile::new_in(path.parent().context("AGY config directory")?)?;
    replacement.write_all(content)?;
    replacement.persist(path)?;
    Ok(())
}

fn profile_record(room: &Path) -> Result<PathBuf> {
    Ok(room
        .parent()
        .context("Isolated AGY runtime directory")?
        .join("agy-policy-home.json"))
}

fn settings() -> Value {
    json!({
        "toolPermission": "strict",
        "enableTerminalSandbox": true,
        "allowNonWorkspaceAccess": false,
        "permissions": {"allow": [], "ask": [], "deny": [
            "read_file(*)", "write_file(*)", "read_url(*)", "execute_url(*)", "command(*)", "mcp(*)"
        ]}
    })
}

pub(crate) fn command(executable: &Path, home: &Path, room: &Path) -> Result<Command> {
    write_config(
        &profile_record(room)?,
        &serde_json::to_vec(&home.canonicalize()?)?,
    )?;
    let data = crate::workspace::private_dir(home, &[".gemini", "antigravity-cli"])?;
    let config = crate::workspace::private_dir(home, &[".gemini", "config"])?;
    let agents = crate::workspace::private_dir(&config, &["agents"])?;
    write_config(
        &agents.join("verum-conversation.md"),
        AGENT_PROFILE.as_bytes(),
    )?;
    write_config(&config.join("mcp_config.json"), br#"{"mcpServers":{}}"#)?;
    write_config(
        &data.join("settings.json"),
        &serde_json::to_vec(&settings())?,
    )?;

    let mut command = Command::new(executable);
    command.env_clear();
    // The official CLI launches the browser itself for interactive Google OAuth.
    for variable in [
        "PATH",
        "LANG",
        "LC_ALL",
        "SystemRoot",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
    ] {
        if let Some(value) = std::env::var_os(variable) {
            command.env(variable, value);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local").join("share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .current_dir(room)
        .args(["acp", "--agent", AGENT, "--disable-telemetry"]);
    Ok(command)
}

fn read_policy_file(home: &Path, relative: &str) -> Result<verum_common::List<u8>> {
    let path = home.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 65536 && path.canonicalize()? == path,
        "Invalid Antigravity policy file"
    );
    Ok(fs::read(path)?.into())
}

/// Recheck Studio's actual settings before supplying the user's prompt to AGY.
/// The peer enforces its native rules; Studio refuses every ACP tool request.
pub(crate) fn qualify_turn(room: &Path) -> Result<()> {
    let home: PathBuf = serde_json::from_slice(&fs::read(profile_record(room)?)?)?;
    ensure!(
        fs::read_dir(room)?.next().is_none(),
        "Antigravity requires an empty conversation directory"
    );
    let current: Value = serde_json::from_slice(&read_policy_file(
        &home,
        ".gemini/antigravity-cli/settings.json",
    )?)?;
    let expected = settings();
    ensure!(
        expected
            .as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| current.get(key) == Some(value)),
        "Antigravity native permission settings changed"
    );
    let mcp: Value =
        serde_json::from_slice(&read_policy_file(&home, ".gemini/config/mcp_config.json")?)?;
    ensure!(
        mcp == json!({"mcpServers":{}}),
        "Antigravity MCP configuration changed"
    );
    ensure!(
        read_policy_file(&home, ".gemini/config/agents/verum-conversation.md")?.as_slice()
            == AGENT_PROFILE.as_bytes(),
        "Antigravity conversation tool profile changed"
    );
    Ok(())
}
