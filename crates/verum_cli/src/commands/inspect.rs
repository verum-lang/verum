//! `verum inspect` — read the Shape manifest a binary carries (T0854).
//!
//! An AOT binary embeds its own capability surface at build time:
//! `MAGIC ++ u32-le length ++ json` in a dedicated section. This
//! command finds the MAGIC by scanning the file bytes — which
//! survives every object format and symbol stripping — and prints
//! the manifest. The point: a supply-chain auditor learns what an
//! artifact MAY DO without sources, and the T0851 enforcement layer
//! derives its filter from the same record.

use anyhow::{bail, Context, Result};

const MAGIC: &[u8] = b"VERUM_SHAPE_MANIFEST_v1\0";

pub fn execute(binary: &std::path::Path, json: bool) -> Result<()> {
    let bytes = std::fs::read(binary)
        .with_context(|| format!("reading {}", binary.display()))?;
    let pos = bytes
        .windows(MAGIC.len())
        .position(|w| w == MAGIC)
        .with_context(|| {
            format!(
                "{} carries no Verum shape manifest (magic not found) — \
                 either it is not a Verum AOT binary, or it was built \
                 before manifests existed",
                binary.display()
            )
        })?;
    let after = pos + MAGIC.len();
    if bytes.len() < after + 4 {
        bail!("truncated manifest header");
    }
    let len = u32::from_le_bytes(bytes[after..after + 4].try_into().unwrap()) as usize;
    let body = bytes
        .get(after + 4..after + 4 + len)
        .context("truncated manifest body")?;
    let text = std::str::from_utf8(body).context("manifest is not UTF-8")?;

    if json {
        println!("{text}");
        return Ok(());
    }
    // Human rendering: parse and lay it out.
    let v: serde_json::Value =
        serde_json::from_str(text).context("manifest is not valid JSON")?;
    println!("shape manifest of {}", binary.display());
    println!(
        "  schema v{}, built by verum {}",
        v["schema_version"], v["tool_version"]
    );
    if let Some(mods) = v["modules"].as_array() {
        println!("  pinned modules: {}", mods.len());
        for m in mods.iter().take(20) {
            let caps = m["pinned"].as_array().map(|a| a.len()).unwrap_or(0);
            println!("    {} ({} capabilities)", m["module"], caps);
        }
        if mods.len() > 20 {
            println!("    … and {} more", mods.len() - 20);
        }
    }
    if let Some(surface) = v["inferred_surface"].as_array() {
        println!("  inferred surface ({} atoms):", surface.len());
        for a in surface.iter().take(20) {
            println!("    {}", a.as_str().unwrap_or("?"));
        }
        if surface.len() > 20 {
            println!("    … and {} more", surface.len() - 20);
        }
    }
    Ok(())
}
