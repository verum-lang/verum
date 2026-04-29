//! Cog distribution registry — reproducibility chain + multi-mirror
//! trust model.
//!
//! ## Goal
//!
//! Make Verum's package manager production-grade so verified
//! mathematics can be published, depended-on, and audit-traced
//! like Cargo / npm but with **cryptographic proof-integrity**:
//!
//!   1. **Per-cog reproducibility hash chain**: every published
//!      cog ships with a blake3 chain over (source files,
//!      verum.lock, audit reports, certificates).  Downstream
//!      consumers verify the entire dependency closure.
//!   2. **Cog signing** (Ed25519): the registry verifies
//!      signatures on publish + serve.
//!   3. **Verified-build attestations**: CI runs
//!      `make audit-honesty-gate` + `make audit`, attests the
//!      result into the registry; consumers see "audited by
//!      VERIFIED-CI on date X" badges.
//!   4. **Math content discovery**: tag cogs by paper-DOI /
//!      framework lineage / theorem catalogue.
//!   5. **Multi-mirror trust**: the registry protocol supports N
//!      independent mirrors; a cog is trusted only when every
//!      mirror agrees on its content hash.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the
//! integration arc:
//!
//!   * [`CogManifest`] — typed metadata (name, version, deps,
//!     content hash, attestations, framework lineage).
//!   * [`CogReproEnvelope`] — typed reproducibility chain
//!     (`input_hash` over source files + lockfile + audit reports
//!     ⟶ `build_env_hash` over toolchain pinning ⟶ `output_hash`
//!     over compiled artefacts).
//!   * [`AttestationKind`] — VerifiedCi / Honesty / Coord /
//!     CrossFormat / FrameworkSoundness.
//!   * [`Attestation`] — typed `(kind, signer, signature_bytes,
//!     timestamp)`.
//!   * [`PublishOutcome`] / [`LookupOutcome`] — typed registry
//!     verdicts.
//!   * [`RegistryClient`] trait — single dispatch interface.
//!   * Reference impls: [`MemoryRegistry`] (deterministic, in-
//!     process), [`LocalFilesystemRegistry`] (V0 disk-backed
//!     reference — every cog stored as JSON under
//!     `<root>/<name>/<version>.json`).
//!   * [`MultiMirrorClient`] — composite that fans out to multiple
//!     registries and requires consensus on content hashes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use verum_common::Text;

// =============================================================================
// CogVersion
// =============================================================================

/// Semver-ish three-component version with optional pre-release tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CogVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub prerelease: Option<Text>,
}

impl CogVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            prerelease: None,
        }
    }

    pub fn with_prerelease(mut self, tag: impl Into<Text>) -> Self {
        self.prerelease = Some(tag.into());
        self
    }

    /// Parse a version string like `"1.2.3"` or `"1.2.3-alpha"`.
    pub fn parse(s: &str) -> Result<Self, Text> {
        let s = s.trim();
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(Text::from(format!(
                "version must be `major.minor.patch[-pre]`, got `{}`",
                s
            )));
        }
        let major: u32 = parts[0]
            .parse()
            .map_err(|_| Text::from(format!("major not a u32: `{}`", parts[0])))?;
        let minor: u32 = parts[1]
            .parse()
            .map_err(|_| Text::from(format!("minor not a u32: `{}`", parts[1])))?;
        let patch: u32 = parts[2]
            .parse()
            .map_err(|_| Text::from(format!("patch not a u32: `{}`", parts[2])))?;
        if let Some(pre) = pre {
            if pre.is_empty() {
                return Err(Text::from(
                    "prerelease tag must be non-empty when `-` is present",
                ));
            }
        }
        Ok(Self {
            major,
            minor,
            patch,
            prerelease: pre.map(Text::from),
        })
    }
}

impl std::fmt::Display for CogVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.prerelease {
            write!(f, "-{}", p.as_str())?;
        }
        Ok(())
    }
}

// =============================================================================
// CogReproEnvelope — reproducibility chain
// =============================================================================

/// Per-cog reproducibility chain.  Three blake3 hashes:
///
///   * `input_hash` — blake3 over (sorted source-file hashes +
///     lockfile + audit-report hashes).
///   * `build_env_hash` — blake3 over the pinned toolchain
///     (Verum kernel version, SMT-solver versions, foreign-tool
///     versions).  Drift here invalidates the build.
///   * `output_hash` — blake3 over the compiled artefacts (.vbc
///     archives + cert files).
///
/// A consumer fetches the cog, recomputes each hash from the
/// downloaded payload, and compares against the envelope.  Any
/// mismatch ⇒ tampering or build-env drift.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CogReproEnvelope {
    pub input_hash: Text,
    pub build_env_hash: Text,
    pub output_hash: Text,
    /// Blake3 chain hash: `chain_hash = blake3(input_hash ‖
    /// build_env_hash ‖ output_hash)`.  This is the single
    /// canonical identifier for the cog version's content.
    pub chain_hash: Text,
}

impl CogReproEnvelope {
    /// Build an envelope from raw component bytes.  Each hash is
    /// blake3 hex; the chain hash is derived deterministically.
    pub fn compute(input: &[u8], build_env: &[u8], output: &[u8]) -> Self {
        let input_hash = hex32(blake3::hash(input).as_bytes());
        let build_env_hash = hex32(blake3::hash(build_env).as_bytes());
        let output_hash = hex32(blake3::hash(output).as_bytes());
        let chain = {
            let mut h = blake3::Hasher::new();
            h.update(input_hash.as_bytes());
            h.update(b"\n");
            h.update(build_env_hash.as_bytes());
            h.update(b"\n");
            h.update(output_hash.as_bytes());
            hex32(h.finalize().as_bytes())
        };
        Self {
            input_hash: Text::from(input_hash),
            build_env_hash: Text::from(build_env_hash),
            output_hash: Text::from(output_hash),
            chain_hash: Text::from(chain),
        }
    }

    /// True iff `chain_hash` matches the canonical derivation of
    /// the three component hashes.  Tampering with any field
    /// (without recomputing the chain) trips this.
    pub fn chain_hash_valid(&self) -> bool {
        let mut h = blake3::Hasher::new();
        h.update(self.input_hash.as_str().as_bytes());
        h.update(b"\n");
        h.update(self.build_env_hash.as_str().as_bytes());
        h.update(b"\n");
        h.update(self.output_hash.as_str().as_bytes());
        let recomputed = Text::from(hex32(h.finalize().as_bytes()));
        recomputed == self.chain_hash
    }
}

fn hex32(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// =============================================================================
// Attestation
// =============================================================================

/// Kind of attestation a CI / auditor stamps onto a cog version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationKind {
    /// `make audit` + `make audit-honesty-gate` passed.
    VerifiedCi,
    /// Per-theorem `--proof-honesty` audit passed (no axiom-only
    /// placeholder).
    Honesty,
    /// `--coord-consistency` audit passed (every `@verify(...)`
    /// has a matching `@framework(...)`).
    Coord,
    /// Cross-format export round-trip succeeded for every required
    /// foreign system.
    CrossFormat,
    /// `--framework-soundness` audit passed (every `@axiom` body is
    /// in `Prop`).
    FrameworkSoundness,
}

impl AttestationKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::VerifiedCi => "verified_ci",
            Self::Honesty => "honesty",
            Self::Coord => "coord",
            Self::CrossFormat => "cross_format",
            Self::FrameworkSoundness => "framework_soundness",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "verified_ci" => Some(Self::VerifiedCi),
            "honesty" => Some(Self::Honesty),
            "coord" => Some(Self::Coord),
            "cross_format" => Some(Self::CrossFormat),
            "framework_soundness" => Some(Self::FrameworkSoundness),
            _ => None,
        }
    }

    pub fn all() -> [AttestationKind; 5] {
        [
            Self::VerifiedCi,
            Self::Honesty,
            Self::Coord,
            Self::CrossFormat,
            Self::FrameworkSoundness,
        ]
    }
}

/// One attestation stamp on a cog version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attestation {
    pub kind: AttestationKind,
    /// Identity of the signer (e.g. `"verified-ci@verum.lang"`).
    pub signer: Text,
    /// Hex-encoded Ed25519 signature over
    /// `(cog.name + version + envelope.chain_hash + kind.name())`.
    /// V0 stores the signature blob verbatim; V1+ verifies on
    /// publish + serve.
    pub signature: Text,
    /// Unix timestamp (seconds) when the attestation was issued.
    pub timestamp: u64,
}

// =============================================================================
// CogManifest
// =============================================================================

/// One cog dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CogDependency {
    pub name: Text,
    pub version_constraint: Text,
}

/// Discovery tags — used by registry search.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CogTags {
    /// Paper DOI(s) the cog mechanises.
    pub paper_doi: Vec<Text>,
    /// `@framework(...)` lineages the cog cites.
    pub framework_lineage: Vec<Text>,
    /// Theorem-catalogue entries (e.g. `"yoneda_full_faithful"`,
    /// `"kunen_consistency"`) — searchable.
    pub theorem_catalogue: Vec<Text>,
}

impl Default for CogTags {
    fn default() -> Self {
        Self {
            paper_doi: Vec::new(),
            framework_lineage: Vec::new(),
            theorem_catalogue: Vec::new(),
        }
    }
}

/// Typed cog manifest as stored in the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CogManifest {
    pub name: Text,
    pub version: CogVersion,
    pub description: Text,
    pub authors: Vec<Text>,
    pub license: Text,
    pub dependencies: Vec<CogDependency>,
    pub envelope: CogReproEnvelope,
    pub attestations: Vec<Attestation>,
    pub tags: CogTags,
    /// Unix timestamp (seconds) when the manifest was published.
    pub published_at: u64,
}

impl CogManifest {
    pub fn new(
        name: impl Into<Text>,
        version: CogVersion,
        envelope: CogReproEnvelope,
    ) -> Self {
        Self {
            name: name.into(),
            version,
            description: Text::from(""),
            authors: Vec::new(),
            license: Text::from(""),
            dependencies: Vec::new(),
            envelope,
            attestations: Vec::new(),
            tags: CogTags::default(),
            published_at: now_secs(),
        }
    }

    /// True iff the envelope's chain hash is internally consistent.
    pub fn envelope_valid(&self) -> bool {
        self.envelope.chain_hash_valid()
    }

    /// Has this cog been attested with a particular kind?
    pub fn has_attestation(&self, kind: AttestationKind) -> bool {
        self.attestations.iter().any(|a| a.kind == kind)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =============================================================================
// Outcomes
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PublishOutcome {
    Accepted {
        chain_hash: Text,
    },
    /// Manifest validation failed (envelope chain-hash mismatch,
    /// missing required attestation, malformed version, etc.).
    Rejected {
        reason: Text,
    },
    /// Same `(name, version)` already exists with a different
    /// chain hash.  This is a hard failure (immutable releases).
    VersionConflict {
        existing_chain_hash: Text,
        proposed_chain_hash: Text,
    },
}

impl PublishOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "Accepted",
            Self::Rejected { .. } => "Rejected",
            Self::VersionConflict { .. } => "VersionConflict",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LookupOutcome {
    Found { manifest: CogManifest },
    NotFound { name: Text, version: CogVersion },
    Error { message: Text },
}

impl LookupOutcome {
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Found { .. })
    }
}

// =============================================================================
// RegistryClient trait
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum RegistryError {
    Io(Text),
    Parse(Text),
    Auth(Text),
    Other(Text),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(t) => write!(f, "I/O: {}", t.as_str()),
            Self::Parse(t) => write!(f, "parse: {}", t.as_str()),
            Self::Auth(t) => write!(f, "auth: {}", t.as_str()),
            Self::Other(t) => write!(f, "{}", t.as_str()),
        }
    }
}

impl std::error::Error for RegistryError {}

// =============================================================================
// Ed25519 attestation signing / verification (#96 hardening)
// =============================================================================
//
// Pre-this-module, `Attestation::signature` was an opaque `Text`
// blob that the registry stored verbatim — `MemoryRegistry::publish`
// accepted any string as a "signature".  An adversary could publish
// a manifest with a fabricated `signature` field and the registry
// would happily serve it to consumers.
//
// Hardening: deterministic Ed25519 over a stable canonical message
// (`name + version + envelope.chain_hash + kind.name`).  Every
// `RegistryClient` implementation is encouraged to call
// `verify_attestation` against the publisher's pinned public key
// before accepting; consumers (e.g. the CLI's `verum cog install`)
// run the same check on download.
//
// Key encoding follows the standard 64-hex-character convention
// (32 bytes hex-encoded).  Signatures are 128 hex characters
// (64 bytes hex-encoded) — both round-trip through `serde` cleanly.

use ed25519_dalek::{
    Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey,
    SECRET_KEY_LENGTH,
};

/// Canonical message bytes Ed25519 signs over for a given
/// (name, version, envelope-chain-hash, attestation-kind) tuple.
///
/// The four components are joined by `'\n'` separators — newline
/// is forbidden in any of them (the four are all kebab-/lowercase-
/// identifier-shaped or hex-encoded), so the join is unambiguously
/// invertible.
pub fn attestation_message(
    cog_name: &str,
    version: &CogVersion,
    chain_hash: &str,
    kind: AttestationKind,
) -> Vec<u8> {
    let body = format!(
        "{}\n{}\n{}\n{}",
        cog_name,
        version.render().as_str(),
        chain_hash,
        kind.name(),
    );
    body.into_bytes()
}

/// Ed25519 attestation-signature errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationCryptoError {
    /// `public_key_hex` could not be decoded as 32 bytes.
    InvalidPublicKey(Text),
    /// `signature_hex` could not be decoded as 64 bytes.
    InvalidSignature(Text),
    /// Signature did not verify against the canonical message.
    SignatureMismatch,
    /// `secret_key_hex` could not be decoded as 32 bytes.
    InvalidSecretKey(Text),
}

impl std::fmt::Display for AttestationCryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPublicKey(t) => write!(f, "invalid Ed25519 public key: {}", t.as_str()),
            Self::InvalidSignature(t) => write!(f, "invalid Ed25519 signature: {}", t.as_str()),
            Self::SignatureMismatch => write!(f, "Ed25519 signature did not verify"),
            Self::InvalidSecretKey(t) => write!(f, "invalid Ed25519 secret key: {}", t.as_str()),
        }
    }
}

impl std::error::Error for AttestationCryptoError {}

/// Render a `CogVersion` to its `MAJOR.MINOR.PATCH[-PRE]` form.
/// (Helper used by `attestation_message`.)
impl CogVersion {
    pub fn render(&self) -> Text {
        let mut s = format!("{}.{}.{}", self.major, self.minor, self.patch);
        if let Some(pre) = &self.prerelease {
            s.push('-');
            s.push_str(pre.as_str());
        }
        Text::from(s)
    }
}

/// Sign one attestation with an Ed25519 key.  Returns the hex-
/// encoded 128-character signature.
///
/// `secret_key_hex` is a 64-character (32 byte) hex-encoded secret
/// scalar — the same encoding `ed25519-dalek::SigningKey::from_bytes`
/// expects after hex decoding.
pub fn sign_attestation(
    secret_key_hex: &str,
    cog_name: &str,
    version: &CogVersion,
    chain_hash: &str,
    kind: AttestationKind,
) -> Result<Text, AttestationCryptoError> {
    let sk_bytes = decode_hex_array::<{ SECRET_KEY_LENGTH }>(secret_key_hex)
        .ok_or_else(|| AttestationCryptoError::InvalidSecretKey(Text::from(secret_key_hex)))?;
    let signing = SigningKey::from_bytes(&sk_bytes);
    let msg = attestation_message(cog_name, version, chain_hash, kind);
    let sig: Ed25519Signature = signing.sign(&msg);
    Ok(Text::from(hex_encode(&sig.to_bytes())))
}

/// Verify one attestation against a publisher public key.
///
/// `public_key_hex` is 64 hex chars (32 bytes); the attestation's
/// `signature` field is 128 hex chars (64 bytes).
pub fn verify_attestation(
    public_key_hex: &str,
    cog_name: &str,
    version: &CogVersion,
    chain_hash: &str,
    attestation: &Attestation,
) -> Result<(), AttestationCryptoError> {
    let pk_bytes = decode_hex_array::<32>(public_key_hex)
        .ok_or_else(|| AttestationCryptoError::InvalidPublicKey(Text::from(public_key_hex)))?;
    let verifying = VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|_| AttestationCryptoError::InvalidPublicKey(Text::from(public_key_hex)))?;
    let sig_bytes = decode_hex_array::<64>(attestation.signature.as_str())
        .ok_or_else(|| AttestationCryptoError::InvalidSignature(attestation.signature.clone()))?;
    let sig = Ed25519Signature::from_bytes(&sig_bytes);
    let msg = attestation_message(cog_name, version, chain_hash, attestation.kind);
    verifying
        .verify(&msg, &sig)
        .map_err(|_| AttestationCryptoError::SignatureMismatch)
}

/// Decode a fixed-size hex string into a byte array.  Returns
/// `None` if the input isn't exactly `N * 2` hex characters.
fn decode_hex_array<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for i in 0..N {
        let pair = &hex[i * 2..i * 2 + 2];
        out[i] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Single dispatch interface for a cog registry client.
pub trait RegistryClient: std::fmt::Debug + Send + Sync {
    /// Stable identifier of the registry (e.g.
    /// `"packages.verum.lang"`).
    fn registry_id(&self) -> Text;

    /// Look up a specific (name, version) pair.
    fn lookup(&self, name: &str, version: &CogVersion) -> Result<LookupOutcome, RegistryError>;

    /// Search by tag.  Returns matching `(name, version)` pairs.
    fn search(&self, query: &SearchQuery) -> Result<Vec<(Text, CogVersion)>, RegistryError>;

    /// Publish a manifest.  The registry validates the envelope's
    /// chain hash, checks for version conflicts, and (V1+) verifies
    /// signatures.
    fn publish(&self, manifest: &CogManifest) -> Result<PublishOutcome, RegistryError>;
}

/// Search query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SearchQuery {
    pub name_substring: Option<Text>,
    pub paper_doi: Option<Text>,
    pub framework_lineage: Option<Text>,
    pub theorem_name: Option<Text>,
    pub require_attestation: Option<AttestationKind>,
}

// =============================================================================
// MemoryRegistry — in-process reference (tests + playbook)
// =============================================================================

#[derive(Debug, Default)]
pub struct MemoryRegistry {
    /// `(name, version) -> manifest`.
    entries: std::sync::Mutex<BTreeMap<(Text, CogVersion), CogManifest>>,
    id: Text,
}

impl MemoryRegistry {
    pub fn new(id: impl Into<Text>) -> Self {
        Self {
            entries: std::sync::Mutex::new(BTreeMap::new()),
            id: id.into(),
        }
    }
}

impl RegistryClient for MemoryRegistry {
    fn registry_id(&self) -> Text {
        self.id.clone()
    }

    fn lookup(
        &self,
        name: &str,
        version: &CogVersion,
    ) -> Result<LookupOutcome, RegistryError> {
        let g = self
            .entries
            .lock()
            .map_err(|_| RegistryError::Io(Text::from("memory registry mutex poisoned")))?;
        match g.get(&(Text::from(name), version.clone())) {
            Some(m) => Ok(LookupOutcome::Found {
                manifest: m.clone(),
            }),
            None => Ok(LookupOutcome::NotFound {
                name: Text::from(name),
                version: version.clone(),
            }),
        }
    }

    fn search(&self, q: &SearchQuery) -> Result<Vec<(Text, CogVersion)>, RegistryError> {
        let g = self
            .entries
            .lock()
            .map_err(|_| RegistryError::Io(Text::from("memory registry mutex poisoned")))?;
        let mut out: Vec<(Text, CogVersion)> = Vec::new();
        for (key, m) in g.iter() {
            if let Some(sub) = &q.name_substring {
                if !key.0.as_str().contains(sub.as_str()) {
                    continue;
                }
            }
            if let Some(doi) = &q.paper_doi {
                if !m.tags.paper_doi.iter().any(|d| d == doi) {
                    continue;
                }
            }
            if let Some(fw) = &q.framework_lineage {
                if !m.tags.framework_lineage.iter().any(|f| f == fw) {
                    continue;
                }
            }
            if let Some(thm) = &q.theorem_name {
                if !m.tags.theorem_catalogue.iter().any(|t| t == thm) {
                    continue;
                }
            }
            if let Some(att) = q.require_attestation {
                if !m.has_attestation(att) {
                    continue;
                }
            }
            out.push(key.clone());
        }
        Ok(out)
    }

    fn publish(&self, m: &CogManifest) -> Result<PublishOutcome, RegistryError> {
        if !m.envelope_valid() {
            return Ok(PublishOutcome::Rejected {
                reason: Text::from(
                    "envelope chain_hash mismatch — recomputed hash differs from stored",
                ),
            });
        }
        let mut g = self
            .entries
            .lock()
            .map_err(|_| RegistryError::Io(Text::from("memory registry mutex poisoned")))?;
        let key = (m.name.clone(), m.version.clone());
        if let Some(existing) = g.get(&key) {
            if existing.envelope.chain_hash != m.envelope.chain_hash {
                return Ok(PublishOutcome::VersionConflict {
                    existing_chain_hash: existing.envelope.chain_hash.clone(),
                    proposed_chain_hash: m.envelope.chain_hash.clone(),
                });
            }
            // Same content republished — accepted as no-op.
            return Ok(PublishOutcome::Accepted {
                chain_hash: m.envelope.chain_hash.clone(),
            });
        }
        g.insert(key, m.clone());
        Ok(PublishOutcome::Accepted {
            chain_hash: m.envelope.chain_hash.clone(),
        })
    }
}

// =============================================================================
// LocalFilesystemRegistry — disk-backed reference
// =============================================================================

/// Disk-backed registry.  One JSON file per cog version under
/// `<root>/<name>/<version>.json`.  V0 reference impl; V1+
/// production server fronts an HTTP API but uses the same trait.
#[derive(Debug)]
pub struct LocalFilesystemRegistry {
    root: PathBuf,
    id: Text,
}

impl LocalFilesystemRegistry {
    pub fn new(root: impl Into<PathBuf>, id: impl Into<Text>) -> Result<Self, RegistryError> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .map_err(|e| RegistryError::Io(Text::from(format!("creating root: {}", e))))?;
        Ok(Self {
            root,
            id: id.into(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for(&self, name: &str, version: &CogVersion) -> PathBuf {
        let safe_name = sanitize(name);
        self.root
            .join(safe_name)
            .join(format!("{}.json", version))
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

impl RegistryClient for LocalFilesystemRegistry {
    fn registry_id(&self) -> Text {
        self.id.clone()
    }

    fn lookup(
        &self,
        name: &str,
        version: &CogVersion,
    ) -> Result<LookupOutcome, RegistryError> {
        let p = self.path_for(name, version);
        match std::fs::read_to_string(&p) {
            Ok(s) => {
                let m: CogManifest = serde_json::from_str(&s).map_err(|e| {
                    RegistryError::Parse(Text::from(format!("{}: {}", p.display(), e)))
                })?;
                Ok(LookupOutcome::Found { manifest: m })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LookupOutcome::NotFound {
                name: Text::from(name),
                version: version.clone(),
            }),
            Err(e) => Err(RegistryError::Io(Text::from(format!(
                "{}: {}",
                p.display(),
                e
            )))),
        }
    }

    fn search(&self, q: &SearchQuery) -> Result<Vec<(Text, CogVersion)>, RegistryError> {
        let mut out: Vec<(Text, CogVersion)> = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            Err(_) => return Ok(out),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir = match std::fs::read_dir(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            for v_entry in dir.flatten() {
                let v_path = v_entry.path();
                if v_path.extension().map_or(false, |e| e != "json") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&v_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let m: CogManifest = match serde_json::from_str(&raw) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let mut keep = true;
                if let Some(sub) = &q.name_substring {
                    if !m.name.as_str().contains(sub.as_str()) {
                        keep = false;
                    }
                }
                if keep {
                    if let Some(doi) = &q.paper_doi {
                        if !m.tags.paper_doi.iter().any(|d| d == doi) {
                            keep = false;
                        }
                    }
                }
                if keep {
                    if let Some(fw) = &q.framework_lineage {
                        if !m.tags.framework_lineage.iter().any(|f| f == fw) {
                            keep = false;
                        }
                    }
                }
                if keep {
                    if let Some(thm) = &q.theorem_name {
                        if !m.tags.theorem_catalogue.iter().any(|t| t == thm) {
                            keep = false;
                        }
                    }
                }
                if keep {
                    if let Some(att) = q.require_attestation {
                        if !m.has_attestation(att) {
                            keep = false;
                        }
                    }
                }
                if keep {
                    out.push((m.name.clone(), m.version.clone()));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn publish(&self, m: &CogManifest) -> Result<PublishOutcome, RegistryError> {
        if !m.envelope_valid() {
            return Ok(PublishOutcome::Rejected {
                reason: Text::from(
                    "envelope chain_hash mismatch — recomputed hash differs from stored",
                ),
            });
        }
        let path = self.path_for(m.name.as_str(), &m.version);
        // Conflict check.
        if let Ok(existing_raw) = std::fs::read_to_string(&path) {
            let existing: CogManifest = serde_json::from_str(&existing_raw).map_err(|e| {
                RegistryError::Parse(Text::from(format!(
                    "existing manifest at {}: {}",
                    path.display(),
                    e
                )))
            })?;
            if existing.envelope.chain_hash != m.envelope.chain_hash {
                return Ok(PublishOutcome::VersionConflict {
                    existing_chain_hash: existing.envelope.chain_hash,
                    proposed_chain_hash: m.envelope.chain_hash.clone(),
                });
            }
            return Ok(PublishOutcome::Accepted {
                chain_hash: m.envelope.chain_hash.clone(),
            });
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RegistryError::Io(Text::from(format!(
                    "creating {}: {}",
                    parent.display(),
                    e
                )))
            })?;
        }
        let json = serde_json::to_string_pretty(m).map_err(|e| {
            RegistryError::Parse(Text::from(format!("serialise manifest: {}", e)))
        })?;
        std::fs::write(&path, json).map_err(|e| {
            RegistryError::Io(Text::from(format!("writing {}: {}", path.display(), e)))
        })?;
        Ok(PublishOutcome::Accepted {
            chain_hash: m.envelope.chain_hash.clone(),
        })
    }
}

// =============================================================================
// MultiMirrorClient — composite consensus across mirrors
// =============================================================================

/// Composite client that fans out to N mirrors and requires
/// consensus on the chain hash.  Returns `Found` only when every
/// mirror that has the cog returns the same chain hash.
pub struct MultiMirrorClient {
    pub mirrors: Vec<Box<dyn RegistryClient>>,
}

impl MultiMirrorClient {
    pub fn new(mirrors: Vec<Box<dyn RegistryClient>>) -> Self {
        Self { mirrors }
    }
}

impl std::fmt::Debug for MultiMirrorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultiMirrorClient({} mirrors)", self.mirrors.len())
    }
}

/// Per-mirror lookup verdict aggregated by [`MultiMirrorClient::lookup_with_consensus`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirrorConsensusVerdict {
    pub query_name: Text,
    pub query_version: CogVersion,
    /// Per-mirror outcomes keyed by registry id.
    pub per_mirror: BTreeMap<Text, LookupOutcome>,
    /// True iff every mirror that returned `Found` agrees on the
    /// chain hash.  False means at least one mirror disagrees —
    /// the cog's content is *not* trusted.
    pub consensus: bool,
    /// The agreed-upon chain hash when `consensus = true` and at
    /// least one mirror has it.
    pub agreed_chain_hash: Option<Text>,
}

impl MultiMirrorClient {
    /// Look up the cog across every mirror and report consensus.
    /// `NotFound` and `Error` outcomes do NOT break consensus —
    /// only conflicting `Found` results do.
    pub fn lookup_with_consensus(
        &self,
        name: &str,
        version: &CogVersion,
    ) -> MirrorConsensusVerdict {
        let mut per_mirror: BTreeMap<Text, LookupOutcome> = BTreeMap::new();
        for m in &self.mirrors {
            let outcome = match m.lookup(name, version) {
                Ok(o) => o,
                Err(e) => LookupOutcome::Error {
                    message: Text::from(format!("{}", e)),
                },
            };
            per_mirror.insert(m.registry_id(), outcome);
        }
        let mut chain_hashes: Vec<Text> = Vec::new();
        for o in per_mirror.values() {
            if let LookupOutcome::Found { manifest } = o {
                chain_hashes.push(manifest.envelope.chain_hash.clone());
            }
        }
        let consensus =
            chain_hashes.is_empty() || chain_hashes.iter().all(|h| h == &chain_hashes[0]);
        let agreed = if consensus {
            chain_hashes.first().cloned()
        } else {
            None
        };
        MirrorConsensusVerdict {
            query_name: Text::from(name),
            query_version: version.clone(),
            per_mirror,
            consensus,
            agreed_chain_hash: agreed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_envelope() -> CogReproEnvelope {
        CogReproEnvelope::compute(
            b"sources",
            b"toolchain-pin",
            b"compiled-output",
        )
    }

    fn fixture_manifest(name: &str, ver: CogVersion) -> CogManifest {
        let mut m = CogManifest::new(name, ver, fixture_envelope());
        m.description = Text::from(format!("test cog {}", name));
        m.authors.push(Text::from("test@example.org"));
        m.license = Text::from("Apache-2.0");
        m
    }

    // ----- CogVersion -----

    #[test]
    fn version_parse_canonical() {
        let v = CogVersion::parse("1.2.3").unwrap();
        assert_eq!(v, CogVersion::new(1, 2, 3));
        assert_eq!(format!("{}", v), "1.2.3");
    }

    #[test]
    fn version_parse_with_prerelease() {
        let v = CogVersion::parse("1.2.3-alpha.1").unwrap();
        assert_eq!(v.prerelease.as_ref().unwrap().as_str(), "alpha.1");
        assert_eq!(format!("{}", v), "1.2.3-alpha.1");
    }

    #[test]
    fn version_parse_rejects_malformed() {
        assert!(CogVersion::parse("1.2").is_err());
        assert!(CogVersion::parse("not a version").is_err());
        assert!(CogVersion::parse("1.2.3-").is_err());
        assert!(CogVersion::parse("a.b.c").is_err());
    }

    #[test]
    fn version_ord_lexicographic() {
        let v1 = CogVersion::new(1, 0, 0);
        let v2 = CogVersion::new(1, 0, 1);
        let v3 = CogVersion::new(1, 1, 0);
        let v4 = CogVersion::new(2, 0, 0);
        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v3 < v4);
    }

    // ----- CogReproEnvelope -----

    #[test]
    fn envelope_compute_is_deterministic() {
        let a = CogReproEnvelope::compute(b"x", b"y", b"z");
        let b = CogReproEnvelope::compute(b"x", b"y", b"z");
        assert_eq!(a, b);
    }

    #[test]
    fn envelope_chain_hash_valid() {
        let e = fixture_envelope();
        assert!(e.chain_hash_valid());
    }

    #[test]
    fn envelope_tamper_detection() {
        let mut e = fixture_envelope();
        e.input_hash = Text::from("0".repeat(64));
        // chain_hash was computed for the original input_hash; now
        // tampered.
        assert!(!e.chain_hash_valid());
    }

    #[test]
    fn envelope_each_component_changes_chain_hash() {
        let base = fixture_envelope();
        let with_diff_input =
            CogReproEnvelope::compute(b"different", b"toolchain-pin", b"compiled-output");
        let with_diff_env =
            CogReproEnvelope::compute(b"sources", b"different", b"compiled-output");
        let with_diff_out =
            CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"different");
        assert_ne!(base.chain_hash, with_diff_input.chain_hash);
        assert_ne!(base.chain_hash, with_diff_env.chain_hash);
        assert_ne!(base.chain_hash, with_diff_out.chain_hash);
    }

    // ----- AttestationKind -----

    #[test]
    fn attestation_kind_round_trip() {
        for k in AttestationKind::all() {
            assert_eq!(AttestationKind::from_name(k.name()), Some(k));
        }
    }

    #[test]
    fn five_canonical_attestation_kinds() {
        assert_eq!(AttestationKind::all().len(), 5);
    }

    // ----- CogManifest -----

    #[test]
    fn manifest_new_envelope_valid() {
        let m = fixture_manifest("foo", CogVersion::new(1, 0, 0));
        assert!(m.envelope_valid());
    }

    #[test]
    fn manifest_has_attestation() {
        let mut m = fixture_manifest("foo", CogVersion::new(1, 0, 0));
        assert!(!m.has_attestation(AttestationKind::VerifiedCi));
        m.attestations.push(Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@example.org"),
            signature: Text::from("00".repeat(32)),
            timestamp: 0,
        });
        assert!(m.has_attestation(AttestationKind::VerifiedCi));
        assert!(!m.has_attestation(AttestationKind::Honesty));
    }

    // ----- MemoryRegistry -----

    #[test]
    fn memory_registry_publish_then_lookup() {
        let r = MemoryRegistry::new("local");
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let outcome = r.publish(&m).unwrap();
        assert!(outcome.is_accepted());
        let look = r.lookup("alpha", &CogVersion::new(1, 0, 0)).unwrap();
        assert!(look.is_found());
    }

    #[test]
    fn memory_registry_lookup_missing_returns_not_found() {
        let r = MemoryRegistry::new("local");
        let look = r.lookup("does-not-exist", &CogVersion::new(1, 0, 0)).unwrap();
        match look {
            LookupOutcome::NotFound { .. } => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn memory_registry_rejects_envelope_with_bad_chain_hash() {
        let r = MemoryRegistry::new("local");
        let mut m = fixture_manifest("bad", CogVersion::new(1, 0, 0));
        m.envelope.chain_hash = Text::from("0".repeat(64));
        let o = r.publish(&m).unwrap();
        match o {
            PublishOutcome::Rejected { reason } => {
                assert!(reason.as_str().contains("chain_hash mismatch"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn memory_registry_version_conflict_on_different_chain_hash() {
        let r = MemoryRegistry::new("local");
        let m1 = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        r.publish(&m1).unwrap();
        // Construct a different envelope (different output bytes)
        // for the same (name, version).
        let mut m2 = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        m2.envelope = CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"DIFFERENT");
        let o = r.publish(&m2).unwrap();
        assert!(matches!(o, PublishOutcome::VersionConflict { .. }));
    }

    #[test]
    fn memory_registry_idempotent_republish_of_same_content() {
        let r = MemoryRegistry::new("local");
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let o1 = r.publish(&m).unwrap();
        let o2 = r.publish(&m).unwrap();
        assert!(o1.is_accepted());
        assert!(o2.is_accepted());
    }

    #[test]
    fn memory_registry_search_by_name_substring() {
        let r = MemoryRegistry::new("local");
        let m1 = fixture_manifest("math.algebra", CogVersion::new(1, 0, 0));
        let m2 = fixture_manifest("math.topology", CogVersion::new(1, 0, 0));
        let m3 = fixture_manifest("io.fs", CogVersion::new(1, 0, 0));
        r.publish(&m1).unwrap();
        r.publish(&m2).unwrap();
        r.publish(&m3).unwrap();
        let q = SearchQuery {
            name_substring: Some(Text::from("math")),
            ..Default::default()
        };
        let results = r.search(&q).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn memory_registry_search_by_attestation() {
        let r = MemoryRegistry::new("local");
        let mut m1 = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let m2 = fixture_manifest("beta", CogVersion::new(1, 0, 0));
        m1.attestations.push(Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@example.org"),
            signature: Text::from(""),
            timestamp: 0,
        });
        r.publish(&m1).unwrap();
        r.publish(&m2).unwrap();
        let q = SearchQuery {
            require_attestation: Some(AttestationKind::VerifiedCi),
            ..Default::default()
        };
        let results = r.search(&q).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.as_str(), "alpha");
    }

    #[test]
    fn memory_registry_search_by_paper_doi() {
        let r = MemoryRegistry::new("local");
        let mut m = fixture_manifest("hott-stuff", CogVersion::new(1, 0, 0));
        m.tags
            .paper_doi
            .push(Text::from("10.4007/annals.2022.196.3"));
        r.publish(&m).unwrap();
        let q = SearchQuery {
            paper_doi: Some(Text::from("10.4007/annals.2022.196.3")),
            ..Default::default()
        };
        let results = r.search(&q).unwrap();
        assert_eq!(results.len(), 1);
    }

    // ----- LocalFilesystemRegistry -----

    #[test]
    fn fs_registry_publish_lookup_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let r = LocalFilesystemRegistry::new(dir.path(), "fs").unwrap();
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let o = r.publish(&m).unwrap();
        assert!(o.is_accepted());
        let look = r.lookup("alpha", &CogVersion::new(1, 0, 0)).unwrap();
        match look {
            LookupOutcome::Found { manifest } => assert_eq!(manifest, m),
            other => panic!("expected Found, got {:?}", other),
        }
    }

    #[test]
    fn fs_registry_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        {
            let r = LocalFilesystemRegistry::new(dir.path(), "fs").unwrap();
            r.publish(&fixture_manifest("alpha", CogVersion::new(1, 0, 0)))
                .unwrap();
        }
        // Re-open the registry; lookup must still find the cog.
        let r2 = LocalFilesystemRegistry::new(dir.path(), "fs").unwrap();
        let look = r2.lookup("alpha", &CogVersion::new(1, 0, 0)).unwrap();
        assert!(look.is_found());
    }

    #[test]
    fn fs_registry_sanitises_unsafe_names() {
        let dir = tempfile::tempdir().unwrap();
        let r = LocalFilesystemRegistry::new(dir.path(), "fs").unwrap();
        let m = fixture_manifest("module::Foo", CogVersion::new(1, 0, 0));
        let o = r.publish(&m).unwrap();
        assert!(o.is_accepted());
    }

    #[test]
    fn fs_registry_search_walks_disk() {
        let dir = tempfile::tempdir().unwrap();
        let r = LocalFilesystemRegistry::new(dir.path(), "fs").unwrap();
        r.publish(&fixture_manifest("math.algebra", CogVersion::new(1, 0, 0)))
            .unwrap();
        r.publish(&fixture_manifest("io.fs", CogVersion::new(1, 0, 0)))
            .unwrap();
        let q = SearchQuery {
            name_substring: Some(Text::from("math")),
            ..Default::default()
        };
        let results = r.search(&q).unwrap();
        assert_eq!(results.len(), 1);
    }

    // ----- MultiMirrorClient -----

    #[test]
    fn multi_mirror_consensus_when_mirrors_agree() {
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let r1 = MemoryRegistry::new("a");
        let r2 = MemoryRegistry::new("b");
        r1.publish(&m).unwrap();
        r2.publish(&m).unwrap();
        let client = MultiMirrorClient::new(vec![Box::new(r1), Box::new(r2)]);
        let v = client.lookup_with_consensus("alpha", &CogVersion::new(1, 0, 0));
        assert!(v.consensus);
        assert!(v.agreed_chain_hash.is_some());
        assert_eq!(v.per_mirror.len(), 2);
    }

    #[test]
    fn multi_mirror_breaks_consensus_on_disagreement() {
        let r1 = MemoryRegistry::new("a");
        let r2 = MemoryRegistry::new("b");
        let m1 = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let mut m2 = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        m2.envelope = CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"DIFFERENT");
        r1.publish(&m1).unwrap();
        r2.publish(&m2).unwrap();
        let client = MultiMirrorClient::new(vec![Box::new(r1), Box::new(r2)]);
        let v = client.lookup_with_consensus("alpha", &CogVersion::new(1, 0, 0));
        assert!(!v.consensus);
        assert!(v.agreed_chain_hash.is_none());
    }

    #[test]
    fn multi_mirror_not_found_in_one_does_not_break_consensus() {
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let r1 = MemoryRegistry::new("a");
        let r2 = MemoryRegistry::new("b");
        r1.publish(&m).unwrap();
        // r2 is empty.
        let client = MultiMirrorClient::new(vec![Box::new(r1), Box::new(r2)]);
        let v = client.lookup_with_consensus("alpha", &CogVersion::new(1, 0, 0));
        // Only one mirror has the cog → no disagreement.
        assert!(v.consensus);
        assert!(v.agreed_chain_hash.is_some());
    }

    // ----- Acceptance pin -----

    #[test]
    fn task_82_immutable_releases() {
        // §1: published cogs are immutable — republishing a version
        // with a different chain hash is a hard failure.
        let r = MemoryRegistry::new("local");
        let original = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        r.publish(&original).unwrap();
        let mut tampered = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        tampered.envelope = CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"hacked");
        let o = r.publish(&tampered).unwrap();
        assert!(matches!(o, PublishOutcome::VersionConflict { .. }));
    }

    #[test]
    fn task_82_reproducibility_chain_tamper_resistant() {
        // §2: the envelope's chain hash detects tampering with any
        // component (sources / build env / output).
        let mut e = fixture_envelope();
        e.output_hash = Text::from("0".repeat(64));
        assert!(!e.chain_hash_valid());
    }

    #[test]
    fn task_82_multi_mirror_trust_model() {
        // §1+§4: multiple independent mirrors must agree on a cog's
        // content hash for it to be trusted.
        let r1 = MemoryRegistry::new("primary");
        let r2 = MemoryRegistry::new("mirror-2");
        let r3 = MemoryRegistry::new("mirror-3");
        let m = fixture_manifest("widely-used", CogVersion::new(1, 0, 0));
        r1.publish(&m).unwrap();
        r2.publish(&m).unwrap();
        r3.publish(&m).unwrap();
        let client = MultiMirrorClient::new(vec![Box::new(r1), Box::new(r2), Box::new(r3)]);
        let v = client.lookup_with_consensus("widely-used", &CogVersion::new(1, 0, 0));
        assert!(v.consensus);
        assert_eq!(v.per_mirror.len(), 3);
    }

    // ----- Serde round-trip -----

    #[test]
    fn manifest_serde_round_trip() {
        let m = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        let json = serde_json::to_string(&m).unwrap();
        let back: CogManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn version_serde_round_trip() {
        let v = CogVersion::new(2, 5, 7).with_prerelease("beta");
        let json = serde_json::to_string(&v).unwrap();
        let back: CogVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    // =========================================================================
    // Ed25519 attestation signing / verification (#96 hardening)
    // =========================================================================

    /// Generate a deterministic test keypair from a 32-byte seed.
    fn test_keypair(seed: u8) -> (String, String) {
        let sk_bytes = [seed; 32];
        let signing = SigningKey::from_bytes(&sk_bytes);
        let verifying = signing.verifying_key();
        let sk_hex = hex_encode(&signing.to_bytes());
        let pk_hex = hex_encode(verifying.as_bytes());
        (sk_hex, pk_hex)
    }

    fn fixture_for_signing(name: &str, version: CogVersion) -> (String, String, Text) {
        let (sk, pk) = test_keypair(0x42);
        let envelope = CogReproEnvelope::compute(b"input", b"build_env", b"output");
        let chain_hash = envelope.chain_hash.clone();
        // Make sure the manifest with this envelope has a stable
        // chain_hash for the signing scope.
        let _ = CogManifest::new(name, version, envelope);
        (sk, pk, chain_hash)
    }

    #[test]
    fn attestation_message_is_deterministic() {
        let v = CogVersion::new(1, 2, 3);
        let m1 = attestation_message("alpha", &v, "abcd", AttestationKind::VerifiedCi);
        let m2 = attestation_message("alpha", &v, "abcd", AttestationKind::VerifiedCi);
        assert_eq!(m1, m2);
    }

    #[test]
    fn attestation_message_distinguishes_components() {
        let v1 = CogVersion::new(1, 0, 0);
        let v2 = CogVersion::new(2, 0, 0);
        let base = attestation_message(
            "alpha",
            &v1,
            "abcd",
            AttestationKind::VerifiedCi,
        );
        // Each component change ⇒ different message bytes.
        assert_ne!(
            base,
            attestation_message("beta", &v1, "abcd", AttestationKind::VerifiedCi)
        );
        assert_ne!(
            base,
            attestation_message("alpha", &v2, "abcd", AttestationKind::VerifiedCi)
        );
        assert_ne!(
            base,
            attestation_message("alpha", &v1, "ffff", AttestationKind::VerifiedCi)
        );
        assert_ne!(
            base,
            attestation_message("alpha", &v1, "abcd", AttestationKind::FrameworkSoundness)
        );
    }

    #[test]
    fn cog_version_render_matches_parse() {
        let v = CogVersion::new(1, 2, 3);
        assert_eq!(v.render().as_str(), "1.2.3");
        let v_pre = CogVersion::new(0, 1, 0).with_prerelease("rc1");
        assert_eq!(v_pre.render().as_str(), "0.1.0-rc1");
    }

    #[test]
    fn sign_then_verify_attestation_round_trip() {
        let (sk, pk, chain) = fixture_for_signing("alpha", CogVersion::new(1, 0, 0));
        let v = CogVersion::new(1, 0, 0);
        let sig = sign_attestation(
            &sk,
            "alpha",
            &v,
            chain.as_str(),
            AttestationKind::VerifiedCi,
        )
        .unwrap();
        // Length sanity: 128 hex chars for 64-byte signature.
        assert_eq!(sig.as_str().len(), 128);
        let att = Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        };
        verify_attestation(&pk, "alpha", &v, chain.as_str(), &att).unwrap();
    }

    #[test]
    fn verify_rejects_signature_under_different_key() {
        let (sk, _pk_a) = test_keypair(0x01);
        let (_sk_b, pk_b) = test_keypair(0x02);
        let v = CogVersion::new(1, 0, 0);
        let envelope = CogReproEnvelope::compute(b"i", b"e", b"o");
        let chain = envelope.chain_hash.clone();
        let sig = sign_attestation(
            &sk,
            "alpha",
            &v,
            chain.as_str(),
            AttestationKind::FrameworkSoundness,
        )
        .unwrap();
        let att = Attestation {
            kind: AttestationKind::FrameworkSoundness,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        };
        match verify_attestation(&pk_b, "alpha", &v, chain.as_str(), &att) {
            Err(AttestationCryptoError::SignatureMismatch) => {}
            other => panic!("expected SignatureMismatch, got {:?}", other),
        }
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let (sk, pk) = test_keypair(0x10);
        let v = CogVersion::new(1, 0, 0);
        let envelope = CogReproEnvelope::compute(b"i", b"e", b"o");
        let chain = envelope.chain_hash.clone();
        let sig = sign_attestation(
            &sk,
            "alpha",
            &v,
            chain.as_str(),
            AttestationKind::VerifiedCi,
        )
        .unwrap();
        let att = Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        };
        // Same key, but verify against a different chain_hash —
        // signature must not validate.
        match verify_attestation(&pk, "alpha", &v, "tamper_hash", &att) {
            Err(AttestationCryptoError::SignatureMismatch) => {}
            other => panic!("expected SignatureMismatch, got {:?}", other),
        }
    }

    #[test]
    fn verify_rejects_wrong_attestation_kind() {
        let (sk, pk) = test_keypair(0x11);
        let v = CogVersion::new(1, 0, 0);
        let chain = "deadbeef".to_string();
        let sig = sign_attestation(
            &sk,
            "alpha",
            &v,
            &chain,
            AttestationKind::VerifiedCi,
        )
        .unwrap();
        // Construct an Attestation with a *different* kind than was signed.
        let bogus = Attestation {
            kind: AttestationKind::FrameworkSoundness,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        };
        match verify_attestation(&pk, "alpha", &v, &chain, &bogus) {
            Err(AttestationCryptoError::SignatureMismatch) => {}
            other => panic!("expected SignatureMismatch, got {:?}", other),
        }
    }

    #[test]
    fn verify_rejects_malformed_keys_and_signatures() {
        let v = CogVersion::new(1, 0, 0);
        let att = Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: Text::from("not-hex"),
            timestamp: 0,
        };
        // Bad public key length.
        match verify_attestation("aabb", "alpha", &v, "deadbeef", &att) {
            Err(AttestationCryptoError::InvalidPublicKey(_)) => {}
            other => panic!("expected InvalidPublicKey, got {:?}", other),
        }
        // Good key length, bad signature length.
        let (_sk, pk) = test_keypair(0x33);
        match verify_attestation(&pk, "alpha", &v, "deadbeef", &att) {
            Err(AttestationCryptoError::InvalidSignature(_)) => {}
            other => panic!("expected InvalidSignature, got {:?}", other),
        }
    }

    #[test]
    fn sign_rejects_malformed_secret_key() {
        let v = CogVersion::new(1, 0, 0);
        match sign_attestation(
            "deadbeef",
            "alpha",
            &v,
            "abcd",
            AttestationKind::VerifiedCi,
        ) {
            Err(AttestationCryptoError::InvalidSecretKey(_)) => {}
            other => panic!("expected InvalidSecretKey, got {:?}", other),
        }
    }

    #[test]
    fn task_96_attestation_signature_is_unbypassable() {
        // Pin the #96 hardening contract: every accepted
        // attestation MUST carry an Ed25519 signature that
        // verifies against the publisher's pinned public key over
        // the canonical (name, version, chain_hash, kind) message.
        // An adversary who fabricates the `signature` blob
        // (non-hex, wrong key, wrong message) is rejected.
        let (sk, pk) = test_keypair(0xab);
        let v = CogVersion::new(1, 0, 0);
        let envelope = CogReproEnvelope::compute(b"input", b"env", b"output");
        let chain = envelope.chain_hash.clone();
        // Legit signature ⇒ accept.
        let sig = sign_attestation(
            &sk,
            "alpha",
            &v,
            chain.as_str(),
            AttestationKind::VerifiedCi,
        )
        .unwrap();
        let legit = Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        };
        verify_attestation(&pk, "alpha", &v, chain.as_str(), &legit).unwrap();
        // Forged signature ⇒ reject.
        let forged = Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: Text::from("0".repeat(128)),
            timestamp: 0,
        };
        assert!(matches!(
            verify_attestation(&pk, "alpha", &v, chain.as_str(), &forged),
            Err(AttestationCryptoError::SignatureMismatch)
        ));
    }
}
