//! End-to-end integration tests for `verum cog-registry`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::{NamedTempFile, TempDir};

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn run(args: &[&str]) -> Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

fn write_manifest(name: &str, version: &str, output_payload: &[u8]) -> NamedTempFile {
    let input_hex = hex_blake3(b"sources@1");
    let env_hex = hex_blake3(b"toolchain-pin");
    let out_hex = hex_blake3(output_payload);
    let chain_hex = {
        let mut h = blake3::Hasher::new();
        h.update(input_hex.as_bytes());
        h.update(b"\n");
        h.update(env_hex.as_bytes());
        h.update(b"\n");
        h.update(out_hex.as_bytes());
        let bytes: [u8; 32] = h.finalize().into();
        let mut s = String::new();
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    };
    let body = format!(
        r#"{{
  "name": "{}",
  "version": {{ "major": {}, "minor": {}, "patch": {}, "prerelease": null }},
  "description": "test",
  "authors": ["test@example.org"],
  "license": "Apache-2.0",
  "dependencies": [],
  "envelope": {{
    "input_hash": "{}",
    "build_env_hash": "{}",
    "output_hash": "{}",
    "chain_hash": "{}"
  }},
  "attestations": [
    {{
      "kind": "verified_ci",
      "signer": "ci@example.org",
      "signature": "{}",
      "timestamp": 0
    }}
  ],
  "tags": {{
    "paper_doi": ["10.4007/test.{}"],
    "framework_lineage": ["zfc"],
    "theorem_catalogue": ["test_thm"]
  }},
  "published_at": 0
}}"#,
        name,
        version_part(version, 0),
        version_part(version, 1),
        version_part(version, 2),
        input_hex,
        env_hex,
        out_hex,
        chain_hex,
        "00".repeat(32),
        name
    );
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

fn version_part(s: &str, idx: usize) -> u32 {
    s.split('.').nth(idx).unwrap().parse().unwrap()
}

fn hex_blake3(bytes: &[u8]) -> String {
    let h = blake3::hash(bytes);
    let mut s = String::new();
    for b in h.as_bytes() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ─────────────────────────────────────────────────────────────────────
// publish
// ─────────────────────────────────────────────────────────────────────

#[test]
fn publish_with_explicit_root_succeeds() {
    let dir = TempDir::new().unwrap();
    let manifest = write_manifest("alpha", "1.0.0", b"output-1");
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        manifest.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("✓ accepted"));
}

#[test]
fn publish_immutable_release_conflict() {
    let dir = TempDir::new().unwrap();
    let m1 = write_manifest("alpha", "1.0.0", b"output-1");
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m1.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    // Republish same (name, version) with different output payload.
    let m2 = write_manifest("alpha", "1.0.0", b"output-2-tampered");
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m2.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "immutable release must fail conflict");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("version conflict") || stdout.contains("VersionConflict"));
}

#[test]
fn publish_idempotent_for_same_content() {
    let dir = TempDir::new().unwrap();
    let m = write_manifest("alpha", "1.0.0", b"out");
    let r1 = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    let r2 = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(r1.status.success());
    assert!(r2.status.success(), "republishing same chain hash must be idempotent");
}

#[test]
fn publish_json_well_formed() {
    let dir = TempDir::new().unwrap();
    let m = write_manifest("alpha", "1.0.0", b"out");
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["name"], "alpha");
    assert_eq!(parsed["version"], "1.0.0");
    assert_eq!(parsed["outcome"]["kind"], "Accepted");
}

// ─────────────────────────────────────────────────────────────────────
// lookup / search / verify
// ─────────────────────────────────────────────────────────────────────

#[test]
fn lookup_finds_published_cog() {
    let dir = TempDir::new().unwrap();
    let m = write_manifest("alpha", "1.0.0", b"out");
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    let out = run(&[
        "cog-registry",
        "lookup",
        "--name",
        "alpha",
        "--version",
        "1.0.0",
        "--root",
        dir.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["kind"], "Found");
    let manifest = &parsed["manifest"];
    assert_eq!(manifest["name"], "alpha");
}

#[test]
fn lookup_missing_returns_non_zero_exit() {
    let dir = TempDir::new().unwrap();
    let out = run(&[
        "cog-registry",
        "lookup",
        "--name",
        "missing-cog",
        "--version",
        "1.0.0",
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn search_by_name_substring() {
    let dir = TempDir::new().unwrap();
    let m1 = write_manifest("math.algebra", "1.0.0", b"out1");
    let m2 = write_manifest("math.topology", "1.0.0", b"out2");
    let m3 = write_manifest("io.fs", "1.0.0", b"out3");
    for path in [m1.path(), m2.path(), m3.path()] {
        run(&[
            "cog-registry",
            "publish",
            "--manifest",
            path.to_str().unwrap(),
            "--root",
            dir.path().to_str().unwrap(),
        ]);
    }
    let out = run(&[
        "cog-registry",
        "search",
        "--name",
        "math",
        "--root",
        dir.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 2);
}

#[test]
fn verify_passes_for_valid_envelope() {
    let dir = TempDir::new().unwrap();
    let m = write_manifest("alpha", "1.0.0", b"out");
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    let out = run(&[
        "cog-registry",
        "verify",
        "--name",
        "alpha",
        "--version",
        "1.0.0",
        "--root",
        dir.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["envelope_valid"], true);
    assert_eq!(parsed["attestations"]["verified_ci"], true);
}

// ─────────────────────────────────────────────────────────────────────
// consensus
// ─────────────────────────────────────────────────────────────────────

#[test]
fn consensus_two_mirrors_in_agreement() {
    let d1 = TempDir::new().unwrap();
    let d2 = TempDir::new().unwrap();
    let m = write_manifest("alpha", "1.0.0", b"out");
    for d in [&d1, &d2] {
        run(&[
            "cog-registry",
            "publish",
            "--manifest",
            m.path().to_str().unwrap(),
            "--root",
            d.path().to_str().unwrap(),
        ]);
    }
    let out = run(&[
        "cog-registry",
        "consensus",
        "--name",
        "alpha",
        "--version",
        "1.0.0",
        "--mirror",
        d1.path().to_str().unwrap(),
        "--mirror",
        d2.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["consensus"], true);
    assert!(parsed["agreed_chain_hash"].is_string());
}

#[test]
fn consensus_breaks_on_mirror_disagreement() {
    let d1 = TempDir::new().unwrap();
    let d2 = TempDir::new().unwrap();
    let m1 = write_manifest("alpha", "1.0.0", b"out-original");
    let m2 = write_manifest("alpha", "1.0.0", b"out-tampered");
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m1.path().to_str().unwrap(),
        "--root",
        d1.path().to_str().unwrap(),
    ]);
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m2.path().to_str().unwrap(),
        "--root",
        d2.path().to_str().unwrap(),
    ]);
    let out = run(&[
        "cog-registry",
        "consensus",
        "--name",
        "alpha",
        "--version",
        "1.0.0",
        "--mirror",
        d1.path().to_str().unwrap(),
        "--mirror",
        d2.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "mirror disagreement must break consensus");
}

#[test]
fn consensus_requires_at_least_one_mirror() {
    let out = run(&[
        "cog-registry",
        "consensus",
        "--name",
        "alpha",
        "--version",
        "1.0.0",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// seed-demo
// ─────────────────────────────────────────────────────────────────────

#[test]
fn seed_demo_smoke_every_format() {
    for fmt in ["plain", "json", "markdown"] {
        let out = run(&["cog-registry", "seed-demo", "--output", fmt]);
        assert!(
            out.status.success(),
            "seed-demo --output {} failed: stderr={}",
            fmt,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn seed_demo_json_carries_chain_hash() {
    let out = run(&["cog-registry", "seed-demo", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let manifest = &parsed["manifest"];
    let chain = manifest["envelope"]["chain_hash"].as_str().unwrap();
    assert_eq!(chain.len(), 64);
    assert!(chain.chars().all(|c| c.is_ascii_hexdigit()));
}

// ─────────────────────────────────────────────────────────────────────
// validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn lookup_rejects_invalid_version() {
    let dir = TempDir::new().unwrap();
    let out = run(&[
        "cog-registry",
        "lookup",
        "--name",
        "alpha",
        "--version",
        "garbage",
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn lookup_rejects_empty_name() {
    let dir = TempDir::new().unwrap();
    let out = run(&[
        "cog-registry",
        "lookup",
        "--name",
        "",
        "--version",
        "1.0.0",
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn search_rejects_unknown_attestation_kind() {
    let dir = TempDir::new().unwrap();
    let out = run(&[
        "cog-registry",
        "search",
        "--require-attestation",
        "garbage",
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn publish_rejects_invalid_envelope() {
    let dir = TempDir::new().unwrap();
    let mut f = NamedTempFile::new().unwrap();
    let body = r#"{
  "name": "alpha",
  "version": { "major": 1, "minor": 0, "patch": 0, "prerelease": null },
  "description": "",
  "authors": [],
  "license": "",
  "dependencies": [],
  "envelope": {
    "input_hash": "0000",
    "build_env_hash": "1111",
    "output_hash": "2222",
    "chain_hash": "tampered"
  },
  "attestations": [],
  "tags": { "paper_doi": [], "framework_lineage": [], "theorem_catalogue": [] },
  "published_at": 0
}"#;
    f.write_all(body.as_bytes()).unwrap();
    f.flush().unwrap();
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        f.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "invalid envelope must reject");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rejected") || stdout.contains("chain_hash mismatch"));
}

#[test]
fn publish_rejects_malformed_json() {
    let dir = TempDir::new().unwrap();
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(b"not json").unwrap();
    f.flush().unwrap();
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        f.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_82_immutable_release_via_cli() {
    // §1: published cogs are immutable — republishing a version
    // with different content fails.
    let dir = TempDir::new().unwrap();
    let m1 = write_manifest("alpha", "1.0.0", b"original");
    run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m1.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    let m2 = write_manifest("alpha", "1.0.0", b"modified");
    let out = run(&[
        "cog-registry",
        "publish",
        "--manifest",
        m2.path().to_str().unwrap(),
        "--root",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn task_82_multi_mirror_trust_via_cli() {
    // §1+§4: multiple mirrors must agree on the chain hash for
    // the cog to be trusted.
    let d1 = TempDir::new().unwrap();
    let d2 = TempDir::new().unwrap();
    let d3 = TempDir::new().unwrap();
    let m = write_manifest("widely-used", "2.5.0", b"out");
    for d in [&d1, &d2, &d3] {
        run(&[
            "cog-registry",
            "publish",
            "--manifest",
            m.path().to_str().unwrap(),
            "--root",
            d.path().to_str().unwrap(),
        ]);
    }
    let out = run(&[
        "cog-registry",
        "consensus",
        "--name",
        "widely-used",
        "--version",
        "2.5.0",
        "--mirror",
        d1.path().to_str().unwrap(),
        "--mirror",
        d2.path().to_str().unwrap(),
        "--mirror",
        d3.path().to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["consensus"], true);
    let per = parsed["per_mirror"].as_object().unwrap();
    assert_eq!(per.len(), 3);
}
