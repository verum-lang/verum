//! `verum cog-registry` subcommand — interact with the cog
//! distribution registry (publish / lookup / search / verify).

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_common::Text;
use verum_verification::cog_registry::{
    AttestationKind, CogManifest, CogVersion, LocalFilesystemRegistry, LookupOutcome,
    MemoryRegistry, MultiMirrorClient, PublishOutcome, RegistryClient, SearchQuery,
};

fn validate_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" && s != "markdown" {
        return Err(CliError::InvalidArgument(format!(
            "--output must be 'plain', 'json', or 'markdown', got '{}'",
            s
        )));
    }
    Ok(())
}

fn parse_version(s: &str) -> Result<CogVersion> {
    CogVersion::parse(s).map_err(|e| CliError::InvalidArgument(e.as_str().to_string()))
}

fn parse_attestation(s: &str) -> Result<AttestationKind> {
    AttestationKind::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--require-attestation must be one of verified_ci / honesty / coord / cross_format / framework_soundness, got '{}'",
            s
        ))
    })
}

fn open_registry(root: Option<&PathBuf>, id: &str) -> Result<LocalFilesystemRegistry> {
    let path = match root {
        Some(p) => p.clone(),
        None => crate::config::Manifest::find_manifest_dir()?
            .join("target")
            .join(".verum_cache")
            .join("cog-registry"),
    };
    LocalFilesystemRegistry::new(&path, id).map_err(|e| {
        CliError::VerificationFailed(format!("registry open: {}", e))
    })
}

fn load_manifest(path: &PathBuf) -> Result<CogManifest> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        CliError::VerificationFailed(format!(
            "reading manifest {}: {}",
            path.display(),
            e
        ))
    })?;
    serde_json::from_str::<CogManifest>(&raw).map_err(|e| {
        CliError::InvalidArgument(format!(
            "manifest {} must be valid CogManifest JSON: {}",
            path.display(),
            e
        ))
    })
}

// =============================================================================
// run_publish
// =============================================================================

pub fn run_publish(
    manifest_path: &PathBuf,
    root: Option<&PathBuf>,
    registry_id: &str,
    output: &str,
) -> Result<()> {
    validate_format(output)?;
    let manifest = load_manifest(manifest_path)?;
    let registry = open_registry(root, registry_id)?;
    let outcome = registry.publish(&manifest).map_err(|e| {
        CliError::VerificationFailed(format!("publish: {}", e))
    })?;
    match output {
        "plain" => emit_publish_plain(&outcome, &manifest),
        "json" => emit_publish_json(&outcome, &manifest),
        "markdown" => emit_publish_markdown(&outcome, &manifest),
        _ => unreachable!(),
    }
    if !outcome.is_accepted() {
        return Err(CliError::VerificationFailed(format!(
            "publish rejected: {}",
            outcome.name()
        )));
    }
    Ok(())
}

fn emit_publish_plain(o: &PublishOutcome, m: &CogManifest) {
    println!("Cog publish");
    println!("  name        : {}", m.name.as_str());
    println!("  version     : {}", m.version);
    println!("  chain_hash  : {}", m.envelope.chain_hash.as_str());
    println!();
    match o {
        PublishOutcome::Accepted { .. } => println!("  ✓ accepted"),
        PublishOutcome::Rejected { reason } => {
            println!("  ✗ rejected — {}", reason.as_str())
        }
        PublishOutcome::VersionConflict {
            existing_chain_hash,
            proposed_chain_hash,
        } => {
            println!("  ✗ version conflict — immutable releases enforced");
            println!("    existing  : {}", existing_chain_hash.as_str());
            println!("    proposed  : {}", proposed_chain_hash.as_str());
        }
    }
}

fn emit_publish_json(o: &PublishOutcome, m: &CogManifest) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"name\": \"{}\",\n",
        json_escape(m.name.as_str())
    ));
    out.push_str(&format!("  \"version\": \"{}\",\n", m.version));
    let body = serde_json::to_string(o).unwrap_or_default();
    out.push_str(&format!("  \"outcome\": {}\n", body));
    out.push('}');
    println!("{}", out);
}

fn emit_publish_markdown(o: &PublishOutcome, m: &CogManifest) {
    println!("# Cog publish\n");
    println!("- **name** — `{}`", m.name.as_str());
    println!("- **version** — `{}`", m.version);
    println!(
        "- **chain_hash** — `{}`\n",
        m.envelope.chain_hash.as_str()
    );
    match o {
        PublishOutcome::Accepted { .. } => println!("**Outcome:** ✓ accepted"),
        PublishOutcome::Rejected { reason } => {
            println!("**Outcome:** ✗ rejected — {}", reason.as_str())
        }
        PublishOutcome::VersionConflict {
            existing_chain_hash,
            proposed_chain_hash,
        } => {
            println!("**Outcome:** ✗ version conflict\n");
            println!("- existing chain: `{}`", existing_chain_hash.as_str());
            println!("- proposed chain: `{}`", proposed_chain_hash.as_str());
        }
    }
}

// =============================================================================
// run_lookup
// =============================================================================

pub fn run_lookup(
    name: &str,
    version: &str,
    root: Option<&PathBuf>,
    registry_id: &str,
    output: &str,
) -> Result<()> {
    validate_format(output)?;
    if name.is_empty() {
        return Err(CliError::InvalidArgument("--name must be non-empty".into()));
    }
    let v = parse_version(version)?;
    let registry = open_registry(root, registry_id)?;
    let outcome = registry.lookup(name, &v).map_err(|e| {
        CliError::VerificationFailed(format!("lookup: {}", e))
    })?;
    match output {
        "plain" => emit_lookup_plain(&outcome, name, &v),
        "json" => emit_lookup_json(&outcome),
        "markdown" => emit_lookup_markdown(&outcome, name, &v),
        _ => unreachable!(),
    }
    if !outcome.is_found() {
        return Err(CliError::VerificationFailed(format!(
            "cog `{}@{}` not found in registry",
            name, v
        )));
    }
    Ok(())
}

fn emit_lookup_plain(o: &LookupOutcome, name: &str, v: &CogVersion) {
    match o {
        LookupOutcome::Found { manifest } => {
            println!("Cog lookup: ✓ found");
            println!("  name           : {}", manifest.name.as_str());
            println!("  version        : {}", manifest.version);
            println!("  description    : {}", manifest.description.as_str());
            if !manifest.authors.is_empty() {
                let a: Vec<&str> = manifest.authors.iter().map(|t| t.as_str()).collect();
                println!("  authors        : {}", a.join(", "));
            }
            println!("  license        : {}", manifest.license.as_str());
            println!("  published_at   : {}", manifest.published_at);
            println!("  envelope:");
            println!(
                "    input_hash     : {}",
                manifest.envelope.input_hash.as_str()
            );
            println!(
                "    build_env_hash : {}",
                manifest.envelope.build_env_hash.as_str()
            );
            println!(
                "    output_hash    : {}",
                manifest.envelope.output_hash.as_str()
            );
            println!(
                "    chain_hash     : {}",
                manifest.envelope.chain_hash.as_str()
            );
            println!(
                "    valid          : {}",
                manifest.envelope_valid()
            );
            if !manifest.dependencies.is_empty() {
                println!("  dependencies:");
                for d in &manifest.dependencies {
                    println!(
                        "    {} {}",
                        d.name.as_str(),
                        d.version_constraint.as_str()
                    );
                }
            }
            if !manifest.attestations.is_empty() {
                println!("  attestations:");
                for a in &manifest.attestations {
                    println!(
                        "    {:<22} signer={} ts={}",
                        a.kind.name(),
                        a.signer.as_str(),
                        a.timestamp
                    );
                }
            }
        }
        LookupOutcome::NotFound { .. } => {
            println!("Cog lookup: — not found");
            println!("  query: {}@{}", name, v);
        }
        LookupOutcome::Error { message } => {
            println!("Cog lookup: ! error");
            println!("  message: {}", message.as_str());
        }
    }
}

fn emit_lookup_json(o: &LookupOutcome) {
    let body = serde_json::to_string_pretty(o).unwrap_or_default();
    println!("{}", body);
}

fn emit_lookup_markdown(o: &LookupOutcome, name: &str, v: &CogVersion) {
    match o {
        LookupOutcome::Found { manifest } => {
            println!("# Cog `{}@{}`\n", manifest.name.as_str(), manifest.version);
            println!("- **description** — {}", manifest.description.as_str());
            println!("- **license** — `{}`", manifest.license.as_str());
            println!(
                "- **chain_hash** — `{}`",
                manifest.envelope.chain_hash.as_str()
            );
            println!(
                "- **envelope valid** — {}\n",
                manifest.envelope_valid()
            );
            if !manifest.attestations.is_empty() {
                println!("## Attestations\n");
                println!("| Kind | Signer | Timestamp |");
                println!("|---|---|---|");
                for a in &manifest.attestations {
                    println!(
                        "| `{}` | `{}` | {} |",
                        a.kind.name(),
                        a.signer.as_str(),
                        a.timestamp
                    );
                }
            }
        }
        LookupOutcome::NotFound { .. } => {
            println!("# Cog lookup\n\n**Outcome:** not found (`{}@{}`)", name, v);
        }
        LookupOutcome::Error { message } => {
            println!("# Cog lookup\n\n**Error:** {}", message.as_str());
        }
    }
}

// =============================================================================
// run_search
// =============================================================================

pub fn run_search(
    name_substring: Option<&str>,
    paper_doi: Option<&str>,
    framework: Option<&str>,
    theorem: Option<&str>,
    require_attestation: Option<&str>,
    root: Option<&PathBuf>,
    registry_id: &str,
    output: &str,
) -> Result<()> {
    validate_format(output)?;
    let mut q = SearchQuery::default();
    q.name_substring = name_substring.map(Text::from);
    q.paper_doi = paper_doi.map(Text::from);
    q.framework_lineage = framework.map(Text::from);
    q.theorem_name = theorem.map(Text::from);
    if let Some(s) = require_attestation {
        q.require_attestation = Some(parse_attestation(s)?);
    }
    let registry = open_registry(root, registry_id)?;
    let results = registry.search(&q).map_err(|e| {
        CliError::VerificationFailed(format!("search: {}", e))
    })?;
    match output {
        "plain" => {
            println!("Search results: {} match(es)", results.len());
            for (n, v) in &results {
                println!("  {}@{}", n.as_str(), v);
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", results.len()));
            out.push_str("  \"matches\": [\n");
            for (i, (n, v)) in results.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"version\": \"{}\" }}{}\n",
                    json_escape(n.as_str()),
                    v,
                    if i + 1 < results.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Search results ({} match(es))\n", results.len());
            println!("| Name | Version |");
            println!("|---|---|");
            for (n, v) in &results {
                println!("| `{}` | `{}` |", n.as_str(), v);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// run_verify
// =============================================================================

pub fn run_verify(
    name: &str,
    version: &str,
    root: Option<&PathBuf>,
    registry_id: &str,
    output: &str,
) -> Result<()> {
    validate_format(output)?;
    if name.is_empty() {
        return Err(CliError::InvalidArgument("--name must be non-empty".into()));
    }
    let v = parse_version(version)?;
    let registry = open_registry(root, registry_id)?;
    let look = registry.lookup(name, &v).map_err(|e| {
        CliError::VerificationFailed(format!("lookup: {}", e))
    })?;
    let manifest = match look {
        LookupOutcome::Found { manifest } => manifest,
        LookupOutcome::NotFound { .. } => {
            return Err(CliError::VerificationFailed(format!(
                "cog `{}@{}` not found",
                name, v
            )));
        }
        LookupOutcome::Error { message } => {
            return Err(CliError::VerificationFailed(format!(
                "lookup error: {}",
                message.as_str()
            )));
        }
    };
    let envelope_ok = manifest.envelope_valid();
    let mut attestations: std::collections::BTreeMap<&str, bool> =
        std::collections::BTreeMap::new();
    for k in AttestationKind::all() {
        attestations.insert(k.name(), manifest.has_attestation(k));
    }
    match output {
        "plain" => {
            println!("Verify cog `{}@{}`", manifest.name.as_str(), manifest.version);
            println!();
            println!(
                "  envelope chain_hash valid : {}",
                if envelope_ok { "✓" } else { "✗" }
            );
            println!("  attestations:");
            for (k, present) in &attestations {
                println!(
                    "    {:<22} {}",
                    k,
                    if *present { "✓" } else { "—" }
                );
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"name\": \"{}\",\n",
                json_escape(manifest.name.as_str())
            ));
            out.push_str(&format!(
                "  \"version\": \"{}\",\n",
                manifest.version
            ));
            out.push_str(&format!(
                "  \"chain_hash\": \"{}\",\n",
                json_escape(manifest.envelope.chain_hash.as_str())
            ));
            out.push_str(&format!("  \"envelope_valid\": {},\n", envelope_ok));
            out.push_str("  \"attestations\": {\n");
            let entries: Vec<(&&str, &bool)> = attestations.iter().collect();
            for (i, (k, v)) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "    \"{}\": {}{}\n",
                    k,
                    v,
                    if i + 1 < entries.len() { "," } else { "" }
                ));
            }
            out.push_str("  }\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Verify `{}@{}`\n", manifest.name.as_str(), manifest.version);
            println!(
                "- **chain_hash** — `{}`",
                manifest.envelope.chain_hash.as_str()
            );
            println!(
                "- **envelope valid** — {}\n",
                if envelope_ok { "✓" } else { "✗" }
            );
            println!("## Attestations\n");
            println!("| Kind | Present |");
            println!("|---|---|");
            for (k, present) in &attestations {
                println!("| `{}` | {} |", k, if *present { "✓" } else { "—" });
            }
        }
        _ => unreachable!(),
    }
    if !envelope_ok {
        return Err(CliError::VerificationFailed(
            "envelope chain_hash invalid — cog content has been tampered with".into(),
        ));
    }
    Ok(())
}

// =============================================================================
// run_consensus
// =============================================================================

pub fn run_consensus(
    name: &str,
    version: &str,
    mirror_roots: &[PathBuf],
    output: &str,
) -> Result<()> {
    validate_format(output)?;
    if name.is_empty() {
        return Err(CliError::InvalidArgument("--name must be non-empty".into()));
    }
    let v = parse_version(version)?;
    if mirror_roots.is_empty() {
        return Err(CliError::InvalidArgument(
            "consensus requires at least one --mirror".into(),
        ));
    }
    let mut clients: Vec<Box<dyn RegistryClient>> = Vec::new();
    for (i, root) in mirror_roots.iter().enumerate() {
        let id = format!("mirror-{}", i + 1);
        let r = LocalFilesystemRegistry::new(root, id.as_str()).map_err(|e| {
            CliError::VerificationFailed(format!("opening mirror {}: {}", id, e))
        })?;
        clients.push(Box::new(r));
    }
    let multi = MultiMirrorClient::new(clients);
    let verdict = multi.lookup_with_consensus(name, &v);
    match output {
        "plain" => {
            println!(
                "Consensus check: `{}@{}` across {} mirror(s)",
                name,
                v,
                verdict.per_mirror.len()
            );
            for (mirror_id, outcome) in &verdict.per_mirror {
                let badge = match outcome {
                    LookupOutcome::Found { manifest } => {
                        format!("✓ found chain={}", manifest.envelope.chain_hash.as_str())
                    }
                    LookupOutcome::NotFound { .. } => "— not found".to_string(),
                    LookupOutcome::Error { message } => format!("! error {}", message.as_str()),
                };
                println!("  {:<22} {}", mirror_id.as_str(), badge);
            }
            println!();
            println!(
                "Consensus      : {}",
                if verdict.consensus { "✓" } else { "✗" }
            );
            if let Some(h) = &verdict.agreed_chain_hash {
                println!("Agreed hash    : {}", h.as_str());
            }
        }
        "json" => {
            let body = serde_json::to_string_pretty(&verdict).unwrap_or_default();
            println!("{}", body);
        }
        "markdown" => {
            println!("# Consensus check `{}@{}`\n", name, v);
            println!("| Mirror | Outcome |");
            println!("|---|---|");
            for (mirror_id, outcome) in &verdict.per_mirror {
                let badge = match outcome {
                    LookupOutcome::Found { manifest } => {
                        format!("✓ found `{}`", manifest.envelope.chain_hash.as_str())
                    }
                    LookupOutcome::NotFound { .. } => "— not found".to_string(),
                    LookupOutcome::Error { message } => {
                        format!("! error: {}", message.as_str())
                    }
                };
                println!("| `{}` | {} |", mirror_id.as_str(), badge);
            }
            println!();
            println!(
                "**Consensus:** {}",
                if verdict.consensus {
                    "✓ achieved"
                } else {
                    "✗ broken"
                }
            );
            if let Some(h) = &verdict.agreed_chain_hash {
                println!("\n**Agreed hash:** `{}`", h.as_str());
            }
        }
        _ => unreachable!(),
    }
    if !verdict.consensus {
        return Err(CliError::VerificationFailed(
            "mirror consensus broken — at least one mirror disagrees on chain_hash".into(),
        ));
    }
    Ok(())
}

// =============================================================================
// run_seed_demo — populate an in-memory registry for demo / docs.
//
// Not strictly part of the production protocol, but useful for the
// auto-paper docs generator when it needs sample data.
// =============================================================================

pub fn run_seed_demo(output: &str) -> Result<()> {
    validate_format(output)?;
    use verum_verification::cog_registry::{
        Attestation, CogDependency, CogManifest, CogReproEnvelope, CogTags,
    };
    let envelope = CogReproEnvelope::compute(
        b"sources@hello-world",
        b"verum-2.6.0+z3-4.13.0",
        b"compiled-vbc",
    );
    let mut m = CogManifest::new("verum.demo.hello-world", CogVersion::new(0, 1, 0), envelope);
    m.description = Text::from("Demo cog used by registry documentation");
    m.authors.push(Text::from("verum-docs@verum.lang"));
    m.license = Text::from("Apache-2.0");
    m.dependencies.push(CogDependency {
        name: Text::from("core.proof"),
        version_constraint: Text::from(">=1.0,<2.0"),
    });
    m.attestations.push(Attestation {
        kind: AttestationKind::VerifiedCi,
        signer: Text::from("verified-ci@verum.lang"),
        signature: Text::from("00".repeat(32)),
        timestamp: 0,
    });
    m.tags = CogTags {
        paper_doi: vec![Text::from("10.4007/annals.demo.hello-world")],
        framework_lineage: vec![Text::from("zfc"), Text::from("lurie_htt")],
        theorem_catalogue: vec![Text::from("hello_world_thm")],
    };
    let r = MemoryRegistry::new("demo");
    r.publish(&m).map_err(|e| {
        CliError::VerificationFailed(format!("demo publish: {}", e))
    })?;
    let look = r
        .lookup("verum.demo.hello-world", &CogVersion::new(0, 1, 0))
        .map_err(|e| CliError::VerificationFailed(format!("demo lookup: {}", e)))?;
    match output {
        "plain" => {
            println!("Demo registry seeded with 1 cog.  Inspecting:");
            println!();
            emit_lookup_plain(
                &look,
                "verum.demo.hello-world",
                &CogVersion::new(0, 1, 0),
            );
        }
        "json" => emit_lookup_json(&look),
        "markdown" => emit_lookup_markdown(
            &look,
            "verum.demo.hello-world",
            &CogVersion::new(0, 1, 0),
        ),
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// helpers
// =============================================================================

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use verum_verification::cog_registry::{CogReproEnvelope, RegistryClient};

    fn fixture_manifest(name: &str) -> CogManifest {
        CogManifest::new(
            name,
            CogVersion::new(1, 0, 0),
            CogReproEnvelope::compute(b"sources", b"toolchain", b"output"),
        )
    }

    fn write_temp_manifest(m: &CogManifest) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let body = serde_json::to_string(m).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    // ----- parsers -----

    #[test]
    fn parse_version_canonical() {
        assert!(parse_version("1.2.3").is_ok());
        assert!(parse_version("garbage").is_err());
    }

    #[test]
    fn parse_attestation_canonical() {
        for s in [
            "verified_ci",
            "honesty",
            "coord",
            "cross_format",
            "framework_soundness",
        ] {
            assert!(parse_attestation(s).is_ok());
        }
        assert!(parse_attestation("garbage").is_err());
    }

    #[test]
    fn validate_format_round_trip() {
        for f in ["plain", "json", "markdown"] {
            assert!(validate_format(f).is_ok());
        }
        assert!(validate_format("yaml").is_err());
    }

    // ----- run_publish -----

    #[test]
    fn publish_round_trip_with_explicit_root() {
        let dir = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        let r = run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn publish_invalid_envelope_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = fixture_manifest("alpha");
        m.envelope.chain_hash = Text::from("0".repeat(64));
        let f = write_temp_manifest(&m);
        let r = run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn publish_invalid_format_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        let r = run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "yaml",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn publish_malformed_manifest_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"not json").unwrap();
        f.flush().unwrap();
        let r = run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    // ----- run_lookup -----

    #[test]
    fn lookup_finds_published_cog() {
        let dir = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        )
        .unwrap();
        let r = run_lookup(
            "alpha",
            "1.0.0",
            Some(&dir.path().to_path_buf()),
            "test",
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn lookup_missing_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let r = run_lookup(
            "missing",
            "1.0.0",
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn lookup_validates_inputs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            run_lookup("", "1.0.0", Some(&dir.path().to_path_buf()), "test", "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_lookup(
                "alpha",
                "garbage",
                Some(&dir.path().to_path_buf()),
                "test",
                "plain"
            ),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- run_search -----

    #[test]
    fn search_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        )
        .unwrap();
        let r = run_search(
            Some("alpha"),
            None,
            None,
            None,
            None,
            Some(&dir.path().to_path_buf()),
            "test",
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn search_rejects_unknown_attestation() {
        let dir = tempfile::tempdir().unwrap();
        let r = run_search(
            None,
            None,
            None,
            None,
            Some("garbage"),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    // ----- run_verify -----

    #[test]
    fn verify_accepts_valid_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        run_publish(
            &f.path().to_path_buf(),
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        )
        .unwrap();
        let r = run_verify(
            "alpha",
            "1.0.0",
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn verify_missing_cog_errors() {
        let dir = tempfile::tempdir().unwrap();
        let r = run_verify(
            "missing",
            "1.0.0",
            Some(&dir.path().to_path_buf()),
            "test",
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    // ----- run_consensus -----

    #[test]
    fn consensus_requires_at_least_one_mirror() {
        let r = run_consensus("alpha", "1.0.0", &[], "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn consensus_two_mirrors_in_agreement() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let m = fixture_manifest("alpha");
        let f = write_temp_manifest(&m);
        run_publish(
            &f.path().to_path_buf(),
            Some(&dir1.path().to_path_buf()),
            "mirror-1",
            "plain",
        )
        .unwrap();
        run_publish(
            &f.path().to_path_buf(),
            Some(&dir2.path().to_path_buf()),
            "mirror-2",
            "plain",
        )
        .unwrap();
        let r = run_consensus(
            "alpha",
            "1.0.0",
            &[dir1.path().to_path_buf(), dir2.path().to_path_buf()],
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn consensus_breaks_on_mirror_disagreement() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let m1 = fixture_manifest("alpha");
        let mut m2 = fixture_manifest("alpha");
        m2.envelope =
            CogReproEnvelope::compute(b"sources", b"toolchain", b"DIFFERENT");
        let f1 = write_temp_manifest(&m1);
        let f2 = write_temp_manifest(&m2);
        run_publish(
            &f1.path().to_path_buf(),
            Some(&dir1.path().to_path_buf()),
            "mirror-1",
            "plain",
        )
        .unwrap();
        run_publish(
            &f2.path().to_path_buf(),
            Some(&dir2.path().to_path_buf()),
            "mirror-2",
            "plain",
        )
        .unwrap();
        let r = run_consensus(
            "alpha",
            "1.0.0",
            &[dir1.path().to_path_buf(), dir2.path().to_path_buf()],
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    // ----- run_seed_demo -----

    #[test]
    fn seed_demo_smoke() {
        for o in ["plain", "json", "markdown"] {
            assert!(run_seed_demo(o).is_ok());
        }
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_handles_quotes_newlines() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }
}
