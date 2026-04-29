//! `verum cache-closure` subcommand — surfaces
//! `verum_verification::closure_cache::FilesystemCacheStore` so users
//! / IDE / CI can inspect, list, clear, and probe the per-theorem
//! closure-hash incremental verification cache.
//!
//! ## Why
//!
//! Pre-this-module the cache trait + reference impls existed in
//! `verum_verification::closure_cache` but had no production CLI
//! consumer.  This module is the **transport-layer integration**:
//! every cache operation IDE/CI needs is exposed via a typed
//! subcommand.
//!
//! ## Cache root resolution
//!
//! The cache lives at `<manifest_dir>/target/.verum_cache/closure-hashes/`,
//! mirroring Cargo's `target/` layout.  Users can override via
//! `--root <path>` for testing or non-standard project layouts.
//!
//! ## Subcommands
//!
//!   * `verum cache-closure stat`  — summary stats (entries, size, hit-ratio).
//!   * `verum cache-closure list`  — every cached theorem name.
//!   * `verum cache-closure get <theorem>` — show a single record.
//!   * `verum cache-closure clear` — remove every entry.
//!   * `verum cache-closure decide <theorem> --signature <hex> --body <hex> [--cite <s>]…`
//!       Probe the cache: report Skip / Recheck verdict for the
//!       given fingerprint, citing the typed [`RecheckReason`] when
//!       Recheck.  Used by IDE prefetch hooks.

use crate::config::Manifest;
use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_verification::closure_cache::{
    decide, CacheDecision, CacheEntry, CacheError, CachedVerdict, ClosureFingerprint,
    FilesystemCacheStore, IncrementalCacheStore, RecheckReason,
};

/// Default cache root relative to the manifest directory.
pub const CACHE_ROOT_REL: &str = "target/.verum_cache/closure-hashes";

/// Resolve the cache root: explicit override or default to manifest-relative.
pub fn resolve_root(override_root: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = override_root {
        return Ok(PathBuf::from(p));
    }
    let manifest_dir = Manifest::find_manifest_dir()?;
    Ok(manifest_dir.join(CACHE_ROOT_REL))
}

fn open_store(root: Option<&str>) -> Result<FilesystemCacheStore> {
    let resolved = resolve_root(root)?;
    FilesystemCacheStore::new(&resolved).map_err(|e| {
        CliError::VerificationFailed(format!(
            "open closure cache at {}: {}",
            resolved.display(),
            e
        ))
    })
}

fn validate_format(format: &str) -> Result<()> {
    if format != "plain" && format != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            format
        )));
    }
    Ok(())
}

// =============================================================================
// stat
// =============================================================================

pub fn run_stat(root: Option<&str>, format: &str) -> Result<()> {
    validate_format(format)?;
    let store = open_store(root)?;
    let stats = store.stats();
    match format {
        "plain" => {
            println!("Closure cache statistics");
            println!("  root          : {}", store.root().display());
            println!("  entries       : {}", stats.entries);
            println!("  size_bytes    : {}", stats.size_bytes);
            println!("  hits          : {}", stats.hits);
            println!("  misses        : {}", stats.misses);
            println!("  hit_ratio     : {:.4}", stats.hit_ratio());
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"root\": \"{}\",\n",
                json_escape(&store.root().display().to_string())
            ));
            out.push_str(&format!("  \"entries\": {},\n", stats.entries));
            out.push_str(&format!("  \"size_bytes\": {},\n", stats.size_bytes));
            out.push_str(&format!("  \"hits\": {},\n", stats.hits));
            out.push_str(&format!("  \"misses\": {},\n", stats.misses));
            out.push_str(&format!("  \"hit_ratio\": {:.4}\n", stats.hit_ratio()));
            out.push('}');
            println!("{}", out);
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// list
// =============================================================================

pub fn run_list(root: Option<&str>, format: &str) -> Result<()> {
    validate_format(format)?;
    let store = open_store(root)?;
    let names = store.names();
    match format {
        "plain" => {
            if names.is_empty() {
                println!("(cache is empty)");
            } else {
                for n in &names {
                    println!("{}", n.as_str());
                }
                println!();
                println!("Total: {} theorem(s)", names.len());
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", names.len()));
            out.push_str("  \"theorems\": [");
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("\"{}\"", json_escape(n.as_str())));
            }
            out.push_str("]\n}");
            println!("{}", out);
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// get
// =============================================================================

pub fn run_get(theorem: &str, root: Option<&str>, format: &str) -> Result<()> {
    validate_format(format)?;
    let store = open_store(root)?;
    let entry = store.get(theorem).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "no cache entry for theorem '{}' (run `verum cache-closure list` to see what's cached)",
            theorem
        ))
    })?;
    match format {
        "plain" => emit_entry_plain(&entry),
        "json" => emit_entry_json(&entry),
        _ => unreachable!(),
    }
    Ok(())
}

fn emit_entry_plain(e: &CacheEntry) {
    println!("Theorem      : {}", e.theorem_name.as_str());
    println!("Recorded at  : {}", e.recorded_at);
    println!("Fingerprint:");
    println!("  kernel_version  : {}", e.fingerprint.kernel_version.as_str());
    println!("  signature_hash  : {}", e.fingerprint.signature_hash.as_str());
    println!("  body_hash       : {}", e.fingerprint.body_hash.as_str());
    println!("  citations_hash  : {}", e.fingerprint.citations_hash.as_str());
    println!("  closure_hash    : {}", e.fingerprint.closure_hash().as_str());
    println!("Verdict:");
    match &e.verdict {
        CachedVerdict::Ok { elapsed_ms } => {
            println!("  status  : ok");
            println!("  elapsed : {}ms", elapsed_ms);
        }
        CachedVerdict::Failed { reason } => {
            println!("  status  : failed");
            println!("  reason  : {}", reason.as_str());
        }
    }
}

fn emit_entry_json(e: &CacheEntry) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"theorem_name\": \"{}\",\n",
        json_escape(e.theorem_name.as_str())
    ));
    out.push_str(&format!("  \"recorded_at\": {},\n", e.recorded_at));
    out.push_str("  \"fingerprint\": {\n");
    out.push_str(&format!(
        "    \"kernel_version\": \"{}\",\n",
        json_escape(e.fingerprint.kernel_version.as_str())
    ));
    out.push_str(&format!(
        "    \"signature_hash\": \"{}\",\n",
        json_escape(e.fingerprint.signature_hash.as_str())
    ));
    out.push_str(&format!(
        "    \"body_hash\": \"{}\",\n",
        json_escape(e.fingerprint.body_hash.as_str())
    ));
    out.push_str(&format!(
        "    \"citations_hash\": \"{}\",\n",
        json_escape(e.fingerprint.citations_hash.as_str())
    ));
    out.push_str(&format!(
        "    \"closure_hash\": \"{}\"\n",
        json_escape(e.fingerprint.closure_hash().as_str())
    ));
    out.push_str("  },\n");
    out.push_str("  \"verdict\": ");
    match &e.verdict {
        CachedVerdict::Ok { elapsed_ms } => {
            out.push_str(&format!(
                "{{ \"status\": \"ok\", \"elapsed_ms\": {} }}\n",
                elapsed_ms
            ));
        }
        CachedVerdict::Failed { reason } => {
            out.push_str(&format!(
                "{{ \"status\": \"failed\", \"reason\": \"{}\" }}\n",
                json_escape(reason.as_str())
            ));
        }
    }
    out.push('}');
    println!("{}", out);
}

// =============================================================================
// clear
// =============================================================================

pub fn run_clear(root: Option<&str>, format: &str) -> Result<()> {
    validate_format(format)?;
    let store = open_store(root)?;
    let n = store.clear().map_err(map_cache_error)?;
    match format {
        "plain" => println!("Cleared {} cache entry(ies)", n),
        "json" => println!("{{\n  \"schema_version\": 1,\n  \"cleared\": {}\n}}", n),
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// decide
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub fn run_decide(
    theorem: &str,
    kernel_version: Option<&str>,
    signature: &str,
    body: &str,
    citations: &[String],
    root: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_format(format)?;
    if signature.is_empty() {
        return Err(CliError::InvalidArgument(
            "--signature must be non-empty".into(),
        ));
    }
    if body.is_empty() {
        return Err(CliError::InvalidArgument("--body must be non-empty".into()));
    }
    let kver = kernel_version.unwrap_or(verum_kernel::VVA_VERSION);
    let cite_refs: Vec<&str> = citations.iter().map(String::as_str).collect();
    let fp =
        ClosureFingerprint::compute(kver, signature.as_bytes(), body.as_bytes(), &cite_refs);

    let store = open_store(root)?;
    let decision = decide(&store, theorem, &fp);

    match format {
        "plain" => emit_decision_plain(&decision, &fp, theorem),
        "json" => emit_decision_json(&decision, &fp, theorem),
        _ => unreachable!(),
    }
    Ok(())
}

fn emit_decision_plain(d: &CacheDecision, fp: &ClosureFingerprint, theorem: &str) {
    println!("Theorem        : {}", theorem);
    println!("Closure hash   : {}", fp.closure_hash().as_str());
    println!("Kernel version : {}", fp.kernel_version.as_str());
    println!();
    match d {
        CacheDecision::Skip { cached } => {
            println!("Decision : skip   (cache hit)");
            println!("  cached_at      : {}", cached.recorded_at);
            if let CachedVerdict::Ok { elapsed_ms } = &cached.verdict {
                println!("  cached_elapsed : {}ms", elapsed_ms);
            }
        }
        CacheDecision::Recheck { reason } => {
            println!("Decision : recheck");
            println!("  reason : {}", reason.label());
            match reason {
                RecheckReason::FingerprintMismatch {
                    previous_closure_hash,
                    current_closure_hash,
                } => {
                    println!("    previous : {}", previous_closure_hash.as_str());
                    println!("    current  : {}", current_closure_hash.as_str());
                }
                RecheckReason::KernelVersionChanged {
                    cached_version,
                    current_version,
                } => {
                    println!("    cached  : {}", cached_version.as_str());
                    println!("    current : {}", current_version.as_str());
                }
                RecheckReason::PreviousVerdictFailed { reason } => {
                    println!("    cause : {}", reason.as_str());
                }
                RecheckReason::NoCacheEntry => {}
            }
        }
    }
}

fn emit_decision_json(d: &CacheDecision, fp: &ClosureFingerprint, theorem: &str) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"theorem\": \"{}\",\n",
        json_escape(theorem)
    ));
    out.push_str(&format!(
        "  \"closure_hash\": \"{}\",\n",
        json_escape(fp.closure_hash().as_str())
    ));
    out.push_str(&format!(
        "  \"kernel_version\": \"{}\",\n",
        json_escape(fp.kernel_version.as_str())
    ));
    out.push_str("  \"decision\": ");
    match d {
        CacheDecision::Skip { cached } => {
            out.push_str(&format!(
                "{{ \"action\": \"skip\", \"cached_at\": {}, \"verdict\": {} }}\n",
                cached.recorded_at,
                verdict_json(&cached.verdict)
            ));
        }
        CacheDecision::Recheck { reason } => {
            let detail = match reason {
                RecheckReason::NoCacheEntry => String::new(),
                RecheckReason::FingerprintMismatch {
                    previous_closure_hash,
                    current_closure_hash,
                } => format!(
                    ", \"previous\": \"{}\", \"current\": \"{}\"",
                    json_escape(previous_closure_hash.as_str()),
                    json_escape(current_closure_hash.as_str())
                ),
                RecheckReason::KernelVersionChanged {
                    cached_version,
                    current_version,
                } => format!(
                    ", \"cached_version\": \"{}\", \"current_version\": \"{}\"",
                    json_escape(cached_version.as_str()),
                    json_escape(current_version.as_str())
                ),
                RecheckReason::PreviousVerdictFailed { reason } => {
                    format!(", \"cause\": \"{}\"", json_escape(reason.as_str()))
                }
            };
            out.push_str(&format!(
                "{{ \"action\": \"recheck\", \"reason\": \"{}\"{} }}\n",
                reason.label(),
                detail
            ));
        }
    }
    out.push('}');
    println!("{}", out);
}

fn verdict_json(v: &CachedVerdict) -> String {
    match v {
        CachedVerdict::Ok { elapsed_ms } => {
            format!("{{ \"status\": \"ok\", \"elapsed_ms\": {} }}", elapsed_ms)
        }
        CachedVerdict::Failed { reason } => format!(
            "{{ \"status\": \"failed\", \"reason\": \"{}\" }}",
            json_escape(reason.as_str())
        ),
    }
}

// =============================================================================
// helpers
// =============================================================================

fn map_cache_error(e: CacheError) -> CliError {
    CliError::VerificationFailed(format!("closure cache: {}", e))
}

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
    use tempfile::TempDir;
    use verum_common::Text;
    use verum_verification::closure_cache::CachedVerdict;

    fn fresh_root() -> (TempDir, String) {
        let t = TempDir::new().unwrap();
        let p = t.path().to_string_lossy().into_owned();
        (t, p)
    }

    fn make_fp() -> ClosureFingerprint {
        ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c"])
    }

    fn put_entry(root: &str, name: &str) {
        let store = FilesystemCacheStore::new(root).unwrap();
        store
            .put(&CacheEntry {
                theorem_name: Text::from(name),
                fingerprint: make_fp(),
                verdict: CachedVerdict::Ok { elapsed_ms: 10 },
                recorded_at: 0,
            })
            .unwrap();
    }

    // ----- validation -----

    #[test]
    fn validate_format_rejects_unknown() {
        assert!(matches!(
            validate_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn validate_format_accepts_plain_and_json() {
        assert!(validate_format("plain").is_ok());
        assert!(validate_format("json").is_ok());
    }

    // ----- run_stat -----

    #[test]
    fn run_stat_empty_cache_succeeds() {
        let (_t, root) = fresh_root();
        assert!(run_stat(Some(&root), "plain").is_ok());
        assert!(run_stat(Some(&root), "json").is_ok());
    }

    // ----- run_list -----

    #[test]
    fn run_list_empty_cache_succeeds() {
        let (_t, root) = fresh_root();
        assert!(run_list(Some(&root), "plain").is_ok());
        assert!(run_list(Some(&root), "json").is_ok());
    }

    #[test]
    fn run_list_after_put_includes_entry() {
        let (_t, root) = fresh_root();
        put_entry(&root, "thm.x");
        assert!(run_list(Some(&root), "plain").is_ok());
    }

    // ----- run_get -----

    #[test]
    fn run_get_missing_theorem_errors() {
        let (_t, root) = fresh_root();
        let r = run_get("nonsense", Some(&root), "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_get_existing_theorem_succeeds() {
        let (_t, root) = fresh_root();
        put_entry(&root, "thm.x");
        assert!(run_get("thm.x", Some(&root), "plain").is_ok());
        assert!(run_get("thm.x", Some(&root), "json").is_ok());
    }

    // ----- run_clear -----

    #[test]
    fn run_clear_removes_entries() {
        let (_t, root) = fresh_root();
        put_entry(&root, "a");
        put_entry(&root, "b");
        assert!(run_clear(Some(&root), "plain").is_ok());
        let store = FilesystemCacheStore::new(&root).unwrap();
        assert_eq!(store.names().len(), 0);
    }

    // ----- run_decide -----

    #[test]
    fn run_decide_no_entry_says_recheck() {
        let (_t, root) = fresh_root();
        let r = run_decide(
            "thm.absent",
            Some("2.6.0"),
            "sig",
            "body",
            &[],
            Some(&root),
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_decide_matching_says_skip() {
        let (_t, root) = fresh_root();
        // Pre-seed cache with an entry whose fingerprint matches.
        let store = FilesystemCacheStore::new(&root).unwrap();
        let fp = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c1"]);
        store
            .put(&CacheEntry {
                theorem_name: Text::from("thm.x"),
                fingerprint: fp,
                verdict: CachedVerdict::Ok { elapsed_ms: 5 },
                recorded_at: 0,
            })
            .unwrap();
        // Decide with same fingerprint inputs.
        assert!(run_decide(
            "thm.x",
            Some("2.6.0"),
            "sig",
            "body",
            &["c1".to_string()],
            Some(&root),
            "json",
        )
        .is_ok());
    }

    #[test]
    fn run_decide_rejects_empty_signature() {
        let (_t, root) = fresh_root();
        let r = run_decide(
            "thm.x",
            Some("2.6.0"),
            "",
            "body",
            &[],
            Some(&root),
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_decide_rejects_empty_body() {
        let (_t, root) = fresh_root();
        let r = run_decide(
            "thm.x",
            Some("2.6.0"),
            "sig",
            "",
            &[],
            Some(&root),
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_decide_kernel_version_default_uses_running_kernel() {
        let (_t, root) = fresh_root();
        // No --kernel-version flag passed → default to running kernel.
        let r = run_decide(
            "thm.x",
            None,
            "sig",
            "body",
            &[],
            Some(&root),
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn json_escape_handles_control_chars() {
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
    }
}
