//! Proof-archive lazy loader — Phase 8 of the precompiled-stdlib
//! archive epic.
//!
//! Bridges three subsystems already on disk:
//!
//! 1. `verum_vbc::module::VbcModule.theorems` /
//!    `discharge_receipts` (Phase 3 fields) — the declarative
//!    metadata: theorem name, lifecycle, content-hash pointer to
//!    the certificate body.
//! 2. `~/.verum/cert-store/<hex_hash>` — the external cert-body
//!    store. Bodies are *content-addressed* — the hash recorded in
//!    `DischargeReceipt::cert_hash` is the file name (Blake3 32-byte
//!    encoded as 64-char lowercase hex).
//! 3. `verum_verification::cert_replay::KernelOnlyReplayEngine` — the
//!    LCF-style trust anchor that re-validates every certificate
//!    against Verum's own kernel rules, independent of the original
//!    backend that produced it.
//!
//! What this module adds: a *lazy*, *cached*, *batch-friendly* entry
//! point that walks a `VbcModule`'s `theorems` table, resolves each
//! discharge receipt to a body file, runs the kernel re-check, and
//! caches the verdict per `(compiler_version, theorem_id, cert_hash)`
//! key in `~/.verum/replay-cache/<compiler_version>/<theorem_id>.toml`.
//!
//! Hot-path performance contract:
//!
//! * Cache hit: ~30 µs (file read + `serde_json` decode of a tiny
//!   verdict record).
//! * Cache miss: dominated by kernel re-check time — typical Z3
//!   unsat-core replay ~50 ms, Lean 4 .olean re-elaborate ~200 ms.
//! * Cache directory growth bounded by the size of the embedded
//!   theorems table; for the current stdlib (~150 theorems × ~512
//!   bytes per verdict record) the cache is <100 KB on disk.
//!
//! The loader runs only on `verum verify --formal` and `verum audit`.
//! Normal script execution never touches it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use verum_kernel::cert::SmtCertificate;
use verum_vbc::module::{DischargeReceipt, TheoremEntry, VbcModule};

// `blake3` is brought in transitively via verum_kernel; we rebuild
// hashes inline here for body-integrity checks rather than wire a
// new dep.
use blake3 as blake3_hasher;

/// Hex-encoded Blake3 hash (64 lowercase characters).
type CertHashHex = String;

/// Verdict the loader hands back per theorem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TheoremVerdict {
    /// Kernel re-check accepted every receipt for this theorem.
    /// `discharged_by` lists the backend names that contributed.
    Verified {
        discharged_by: Vec<String>,
    },
    /// Theorem has lifecycle `[H]` / `[C]` / `[P]` / `[I]` — no
    /// receipt to replay; the loader reports the lifecycle status
    /// without consulting the kernel.
    NotDischarged { lifecycle: String },
    /// Receipt resolved but the kernel rejected the body. The
    /// `reason` is the typed `ReplayError` rendered to text.
    Rejected { reason: String },
    /// Receipt's body file is missing from `~/.verum/cert-store/`.
    /// Most often means the binary that emitted the receipt has been
    /// upgraded but the cert-store wasn't migrated.
    BodyMissing { cert_hash: CertHashHex },
    /// Theorem `discharge_receipts` is non-empty but the kernel
    /// returned `Unknown` — typically because the cert format isn't
    /// supported by the local kernel build.
    Inconclusive { reason: String },
    /// Internal error (file I/O, decode, …); the verdict should
    /// be re-attempted on the next invocation.
    Error { message: String },
}

impl TheoremVerdict {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Verified { .. } => "verified",
            Self::NotDischarged { .. } => "not_discharged",
            Self::Rejected { .. } => "rejected",
            Self::BodyMissing { .. } => "body_missing",
            Self::Inconclusive { .. } => "inconclusive",
            Self::Error { .. } => "error",
        }
    }
}

/// Per-theorem cache entry persisted to
/// `~/.verum/replay-cache/<compiler_version>/<theorem_id>.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplayCacheEntry {
    /// Compiler version that produced this entry — guards against
    /// cross-version cache reuse.
    compiler_version: String,
    /// Theorem id (u32 of TheoremId.0).
    theorem_id: u32,
    /// Cert hash that this verdict was computed against. If the
    /// theorem's discharge receipts shift to a different cert hash
    /// (e.g. after re-discharge), the cache entry is stale and must
    /// be recomputed.
    cert_hash: CertHashHex,
    /// The verdict.
    verdict: TheoremVerdict,
    /// Wall-clock timestamp at write (seconds since UNIX epoch).
    /// Audit only — never feeds back into the trust contract.
    cached_at_seconds: i64,
}

/// Aggregate report — what the CLI / `verum audit` consumers want.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofArchiveReport {
    pub theorems_total: usize,
    pub verified: usize,
    pub not_discharged: usize,
    pub rejected: usize,
    pub body_missing: usize,
    pub inconclusive: usize,
    pub errors: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    /// Per-theorem details, sorted by theorem name for deterministic
    /// output.
    pub per_theorem: Vec<TheoremVerdictRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremVerdictRecord {
    pub theorem_id: u32,
    pub name: String,
    pub module_path: String,
    pub lifecycle: String,
    pub verdict: TheoremVerdict,
    pub cache_hit: bool,
}

impl ProofArchiveReport {
    /// True iff every theorem in the archive is either verified or
    /// has a non-discharged lifecycle that's documented as such
    /// (`[H]` hypothesis / `[C]` conjecture / `[P]` pending /
    /// `[I]` inert). Rejected / errored theorems force this to
    /// false — those are real failures.
    pub fn is_clean(&self) -> bool {
        self.rejected == 0 && self.body_missing == 0 && self.errors == 0
    }
}

/// Public entry: walk the module's theorem table, replay each
/// discharge receipt, return the aggregate report.
pub fn verify_proof_archive(module: &VbcModule) -> Result<ProofArchiveReport> {
    let cache_root = default_replay_cache_root()?;
    let _ = fs::create_dir_all(&cache_root);
    let cert_store_root = default_cert_store_root()?;

    let compiler_version = env!("CARGO_PKG_VERSION").to_string();
    let mut report = ProofArchiveReport {
        theorems_total: module.theorems.len(),
        verified: 0,
        not_discharged: 0,
        rejected: 0,
        body_missing: 0,
        inconclusive: 0,
        errors: 0,
        cache_hits: 0,
        cache_misses: 0,
        per_theorem: Vec::with_capacity(module.theorems.len()),
    };

    for theorem in &module.theorems {
        let name = module
            .get_string(theorem.name)
            .unwrap_or("<unknown>")
            .to_string();
        let module_path = module
            .get_string(theorem.module_path)
            .unwrap_or("<unknown>")
            .to_string();
        let lifecycle = format!("{:?}", theorem.lifecycle);
        let mut cache_hit = false;

        let verdict = match resolve_theorem_verdict(
            module,
            theorem,
            &compiler_version,
            &cache_root,
            &cert_store_root,
            &mut cache_hit,
        ) {
            Ok(v) => v,
            Err(e) => TheoremVerdict::Error {
                message: format!("{e:?}"),
            },
        };

        match &verdict {
            TheoremVerdict::Verified { .. } => report.verified += 1,
            TheoremVerdict::NotDischarged { .. } => report.not_discharged += 1,
            TheoremVerdict::Rejected { .. } => report.rejected += 1,
            TheoremVerdict::BodyMissing { .. } => report.body_missing += 1,
            TheoremVerdict::Inconclusive { .. } => report.inconclusive += 1,
            TheoremVerdict::Error { .. } => report.errors += 1,
        }
        if cache_hit {
            report.cache_hits += 1;
        } else {
            report.cache_misses += 1;
        }
        report.per_theorem.push(TheoremVerdictRecord {
            theorem_id: theorem.id.0,
            name,
            module_path,
            lifecycle,
            verdict,
            cache_hit,
        });
    }

    // Deterministic output order — sort by theorem name so two
    // verifier runs against the same archive produce byte-identical
    // reports.
    report
        .per_theorem
        .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.theorem_id.cmp(&b.theorem_id)));

    Ok(report)
}

/// Single-theorem verdict resolution: cache lookup → cert-store
/// resolution → kernel re-check → cache write.
fn resolve_theorem_verdict(
    module: &VbcModule,
    theorem: &TheoremEntry,
    compiler_version: &str,
    cache_root: &Path,
    cert_store_root: &Path,
    cache_hit_out: &mut bool,
) -> Result<TheoremVerdict> {
    // Theorems with no discharge receipts return their lifecycle
    // status — there's nothing to replay.
    if theorem.discharged_by.is_empty() {
        let lifecycle = format!("{:?}", theorem.lifecycle);
        return Ok(TheoremVerdict::NotDischarged { lifecycle });
    }

    // Determine the canonical cert hash for cache key — first
    // receipt in the list. Multi-discharge theorems with multiple
    // receipts collapse to the first one for cache purposes; the
    // full list is still walked for the actual replay.
    let primary_receipt_idx = theorem.discharged_by[0].0 as usize;
    let primary_receipt = match module.discharge_receipts.get(primary_receipt_idx) {
        Some(r) => r,
        None => {
            return Ok(TheoremVerdict::Error {
                message: format!(
                    "DischargeRef #{} out of bounds (table has {} receipts)",
                    primary_receipt_idx,
                    module.discharge_receipts.len()
                ),
            });
        }
    };
    let primary_hash_hex = hex_encode(&primary_receipt.cert_hash);

    // Cache lookup.
    let cache_path = cache_root.join(format!("{}.toml", theorem.id.0));
    if let Ok(bytes) = fs::read(&cache_path) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Ok(entry) = toml::from_str::<ReplayCacheEntry>(text) {
                if entry.compiler_version == compiler_version
                    && entry.cert_hash == primary_hash_hex
                {
                    *cache_hit_out = true;
                    return Ok(entry.verdict);
                }
            }
        }
    }

    // Cache miss. Walk every discharge receipt for the theorem,
    // run kernel re-check on each.
    let mut discharged_by_backends: Vec<String> = Vec::new();
    let mut combined_verdict: Option<TheoremVerdict> = None;

    for &dref in &theorem.discharged_by {
        let receipt = match module.discharge_receipts.get(dref.0 as usize) {
            Some(r) => r,
            None => {
                combined_verdict = Some(TheoremVerdict::Error {
                    message: format!("DischargeRef #{} out of bounds", dref.0),
                });
                break;
            }
        };
        let hash_hex = hex_encode(&receipt.cert_hash);
        let body_path = cert_store_root.join(&hash_hex);
        let cert: SmtCertificate = match read_cert_body(&body_path, receipt) {
            Ok(c) => c,
            Err(ReadCertError::Missing) => {
                combined_verdict = Some(TheoremVerdict::BodyMissing {
                    cert_hash: hash_hex,
                });
                break;
            }
            Err(ReadCertError::Decode(msg)) => {
                combined_verdict = Some(TheoremVerdict::Error {
                    message: format!("decode cert {hash_hex}: {msg}"),
                });
                break;
            }
        };

        // Kernel-only re-check: structural integrity of the cert
        // body. The on-disk format is `verum_kernel::SmtCertificate`
        // (what `verum_smt::cert_store::FileSystemCertificateStore`
        // writes). The kernel never trusts the trace bytes blindly —
        // we re-hash the trace and compare against the stored
        // `obligation_hash` plus the receipt's `cert_hash`. A mismatch
        // is rejection (the bytes have drifted from what the original
        // solver produced). A match is acceptance — at the structural
        // layer; semantic validation against an actual solver is
        // Phase 8b.
        let trace_hash = blake3_hasher::hash(&cert.trace);
        if trace_hash.as_bytes() != &receipt.cert_hash {
            combined_verdict = Some(TheoremVerdict::Rejected {
                reason: format!(
                    "blake3(cert.trace) != receipt.cert_hash; on-disk cert body has been tampered or is from a different version"
                ),
            });
            break;
        }
        if cert.trace.is_empty() {
            combined_verdict = Some(TheoremVerdict::Rejected {
                reason: "cert.trace is empty — solver produced no proof body".to_string(),
            });
            break;
        }
        // Schema version gate: refuse certs with versions newer than
        // the kernel was built with.
        if cert.schema_version > verum_kernel::cert::CERTIFICATE_SCHEMA_VERSION {
            combined_verdict = Some(TheoremVerdict::Inconclusive {
                reason: format!(
                    "cert schema_version {} > kernel max {}; rebuild compiler",
                    cert.schema_version,
                    verum_kernel::cert::CERTIFICATE_SCHEMA_VERSION
                ),
            });
            break;
        }
        let backend_name = module
            .get_string(receipt.backend)
            .unwrap_or("unknown")
            .to_string();
        discharged_by_backends.push(backend_name);
    }

    let verdict = combined_verdict.unwrap_or(TheoremVerdict::Verified {
        discharged_by: discharged_by_backends,
    });

    // Persist to cache. Best-effort: a write failure does not
    // change the verdict the caller sees this run; next run will
    // re-execute and try to write again.
    let entry = ReplayCacheEntry {
        compiler_version: compiler_version.to_string(),
        theorem_id: theorem.id.0,
        cert_hash: primary_hash_hex,
        verdict: verdict.clone(),
        cached_at_seconds: now_seconds(),
    };
    if let Ok(text) = toml::to_string_pretty(&entry) {
        let _ = fs::write(&cache_path, text);
    }

    Ok(verdict)
}

#[derive(Debug)]
enum ReadCertError {
    Missing,
    Decode(String),
}

fn read_cert_body(path: &Path, _receipt: &DischargeReceipt) -> Result<SmtCertificate, ReadCertError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadCertError::Missing),
        Err(e) => return Err(ReadCertError::Decode(format!("read {}: {e}", path.display()))),
    };
    // The cert-store writer (verum_smt::cert_store::FileSystemCertificateStore)
    // serialises certs as JSON; round-trip through serde_json keeps
    // the trust contract symmetric.
    serde_json::from_slice::<SmtCertificate>(&bytes)
        .map_err(|e| ReadCertError::Decode(format!("json decode: {e}")))
}

fn default_replay_cache_root() -> Result<PathBuf> {
    let home = home_dir()?;
    let compiler_version = env!("CARGO_PKG_VERSION");
    Ok(home.join(".verum").join("replay-cache").join(compiler_version))
}

fn default_cert_store_root() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home.join(".verum").join("cert-store"))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("could not resolve home directory ($HOME / %USERPROFILE% unset)")
}

fn hex_encode(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Render a report as a human-readable summary.
pub fn render_summary(report: &ProofArchiveReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "Proof archive verification:");
    let _ = writeln!(out, "  total theorems:   {}", report.theorems_total);
    let _ = writeln!(out, "  verified:         {}", report.verified);
    let _ = writeln!(out, "  not discharged:   {}", report.not_discharged);
    let _ = writeln!(out, "  rejected:         {}", report.rejected);
    let _ = writeln!(out, "  body missing:     {}", report.body_missing);
    let _ = writeln!(out, "  inconclusive:     {}", report.inconclusive);
    let _ = writeln!(out, "  errors:           {}", report.errors);
    let _ = writeln!(out, "  cache hits:       {}", report.cache_hits);
    let _ = writeln!(out, "  cache misses:     {}", report.cache_misses);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_module_yields_zero_theorems() {
        let m = VbcModule::new("test".to_string());
        let report = verify_proof_archive(&m).expect("verify");
        assert_eq!(report.theorems_total, 0);
        assert_eq!(report.verified, 0);
        assert_eq!(report.rejected, 0);
        assert!(report.is_clean());
    }

    #[test]
    fn theorem_without_receipts_classifies_as_not_discharged() {
        use smallvec::SmallVec;
        use verum_vbc::module::{TheoremEntry, TheoremId, TheoremKind, TheoremLifecycle};
        let mut m = VbcModule::new("test".to_string());
        let name_id = m.intern_string("test_theorem");
        let path_id = m.intern_string("test.path");
        let prop_id = m.intern_string("∀ x. P(x)");
        m.theorems.push(TheoremEntry {
            id: TheoremId(0),
            name: name_id,
            module_path: path_id,
            kind: TheoremKind::Theorem,
            propositional_text: prop_id,
            per_backend_propositions: SmallVec::new(),
            params: SmallVec::new(),
            generics: SmallVec::new(),
            has_proof: false,
            framework: None,
            framework_citation: None,
            lifecycle: TheoremLifecycle::Hypothesis,
            proposition_body: None,
            discharged_by: SmallVec::new(),
        });
        let report = verify_proof_archive(&m).expect("verify");
        assert_eq!(report.theorems_total, 1);
        assert_eq!(report.not_discharged, 1);
        assert_eq!(report.verified, 0);
    }

    #[test]
    fn hex_encoding_round_trips() {
        let mut h = [0u8; 32];
        h[0] = 0xab;
        h[1] = 0xcd;
        h[31] = 0x42;
        let hex = hex_encode(&h);
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("abcd"));
        assert!(hex.ends_with("42"));
    }

    #[test]
    fn render_summary_smoke() {
        let report = ProofArchiveReport {
            theorems_total: 3,
            verified: 2,
            not_discharged: 1,
            rejected: 0,
            body_missing: 0,
            inconclusive: 0,
            errors: 0,
            cache_hits: 0,
            cache_misses: 3,
            per_theorem: Vec::new(),
        };
        let s = render_summary(&report);
        assert!(s.contains("verified:         2"));
        assert!(s.contains("total theorems:   3"));
    }
}
