//! Cog distribution registry — reproducibility chain + multi-mirror
//! trust model.
//!

//! ## Goal
//!

//! Make Verum's package manager production-grade so verified
//! mathematics can be published, depended-on, and audit-traced
//! like Cargo / npm but with **cryptographic proof-integrity**:
//!

//!  1. **Per-cog reproducibility hash chain**: every published
//!  cog ships with a blake3 chain over (source files,
//!  verum.lock, audit reports, certificates). Downstream
//!  consumers verify the entire dependency closure.
//!  2. **Cog signing** (Ed25519): the registry verifies
//!  signatures on publish + serve.
//!  3. **Verified-build attestations**: CI runs
//!  `make audit-honesty-gate` + `make audit`, attests the
//!  result into the registry; consumers see "audited by
//!  VERIFIED-CI on date X" badges.
//!  4. **Math content discovery**: tag cogs by paper-DOI /
//!  framework lineage / theorem catalogue.
//!  5. **Multi-mirror trust**: the registry protocol supports N
//!  independent mirrors; a cog is trusted only when every
//!  mirror agrees on its content hash.
//!

//! ## Architectural pattern
//!

//! Same single-trait-boundary pattern as the rest of the
//! integration arc:
//!

//!  * [`CogManifest`] — typed metadata (name, version, deps,
//!  content hash, attestations, framework lineage).
//!  * [`CogReproEnvelope`] — typed reproducibility chain
//!  (`input_hash` over source files + lockfile + audit reports
//!  ⟶ `build_env_hash` over toolchain pinning ⟶ `output_hash`
//!  over compiled artefacts).
//!  * [`AttestationKind`] — VerifiedCi / Honesty / Coord /
//!  CrossFormat / FrameworkSoundness.
//!  * [`Attestation`] — typed `(kind, signer, signature_bytes,
//!  timestamp)`.
//!  * [`PublishOutcome`] / [`LookupOutcome`] — typed registry
//!  verdicts.
//!  * [`RegistryClient`] trait — single dispatch interface.
//!  * Reference impls: [`MemoryRegistry`] (deterministic, in-
//!  process), [`LocalFilesystemRegistry`] (V0 disk-backed
//!  reference — every cog stored as JSON under
//!  `<root>/<name>/<version>.json`).
//!  * [`MultiMirrorClient`] — composite that fans out to multiple
//!  registries and requires consensus on content hashes.

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

/// Per-cog reproducibility chain. Three blake3 hashes:
///

///  * `input_hash` — blake3 over (sorted source-file hashes +
///  lockfile + audit-report hashes).
///  * `build_env_hash` — blake3 over the pinned toolchain
///  (Verum kernel version, SMT-solver versions, foreign-tool
///  versions). Drift here invalidates the build.
///  * `output_hash` — blake3 over the compiled artefacts (.vbc
///  archives + cert files).
///

/// A consumer fetches the cog, recomputes each hash from the
/// downloaded payload, and compares against the envelope. Any
/// mismatch ⇒ tampering or build-env drift.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CogReproEnvelope {
    pub input_hash: Text,
    pub build_env_hash: Text,
    pub output_hash: Text,
    /// Blake3 chain hash: `chain_hash = blake3(input_hash ‖
    /// build_env_hash ‖ output_hash)`. This is the single
    /// canonical identifier for the cog version's content.
    pub chain_hash: Text,
}

impl CogReproEnvelope {
    /// Build an envelope from raw component bytes. Each hash is
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
    /// the three component hashes. Tampering with any field
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
    pub fn new(name: impl Into<Text>, version: CogVersion, envelope: CogReproEnvelope) -> Self {
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
    /// chain hash. This is a hard failure (immutable releases).
    VersionConflict {
        existing_chain_hash: Text,
        proposed_chain_hash: Text,
    },
}

/// Discriminator for [`PublishOutcome`] — zero-sized projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PublishOutcomeKind {
    Accepted,
    Rejected,
    VersionConflict,
}

/// Per-variant projection for [`PublishOutcomeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishOutcomeKindMeta {
    /// PascalCase wire form — matches the existing `name()`
    /// surface that downstream JSON/audit consumers parse on.
    pub name: &'static str,
    /// Whether the outcome is *positive* (the publish succeeded
    /// — Accepted singleton).
    pub is_accepted: bool,
    /// Whether the outcome is a *hard collision* — the
    /// `(name, version)` already exists with a different
    /// chain hash.  Singleton on `VersionConflict`.  Pinned
    /// because immutable-release semantics mean this is the
    /// only outcome that fundamentally cannot be retried with
    /// the same version.
    pub is_collision: bool,
    /// Whether the variant carries a *chain-hash* payload
    /// (Accepted + VersionConflict — both reference at least
    /// one chain hash).  Decouples downstream display logic
    /// from per-variant matching.
    pub carries_chain_hash: bool,
    /// Whether the variant carries a free-form *reason*
    /// payload — Rejected singleton.
    pub carries_reason: bool,
}

impl PublishOutcomeKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Accepted,
        Self::Rejected,
        Self::VersionConflict,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> PublishOutcomeKindMeta {
        match self {
            PublishOutcomeKind::Accepted => PublishOutcomeKindMeta {
                name: "Accepted",
                is_accepted: true,
                is_collision: false,
                carries_chain_hash: true,
                carries_reason: false,
            },
            PublishOutcomeKind::Rejected => PublishOutcomeKindMeta {
                name: "Rejected",
                is_accepted: false,
                is_collision: false,
                carries_chain_hash: false,
                carries_reason: true,
            },
            PublishOutcomeKind::VersionConflict => PublishOutcomeKindMeta {
                name: "VersionConflict",
                is_accepted: false,
                is_collision: true,
                carries_chain_hash: true,
                carries_reason: false,
            },
        }
    }
}

impl PublishOutcome {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> PublishOutcomeKind {
        match self {
            PublishOutcome::Accepted { .. } => PublishOutcomeKind::Accepted,
            PublishOutcome::Rejected { .. } => PublishOutcomeKind::Rejected,
            PublishOutcome::VersionConflict { .. } => PublishOutcomeKind::VersionConflict,
        }
    }

    /// Whether the publish succeeded.  Routes through
    /// `meta().is_accepted` — single source of truth.
    pub fn is_accepted(&self) -> bool {
        self.kind().meta().is_accepted
    }

    /// PascalCase variant name — preserved for downstream JSON
    /// consumers.  Routes through `meta().name`.
    pub fn name(&self) -> &'static str {
        self.kind().meta().name
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LookupOutcome {
    Found { manifest: CogManifest },
    NotFound { name: Text, version: CogVersion },
    Error { message: Text },
}

/// Discriminator for [`LookupOutcome`] — zero-sized projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LookupOutcomeKind {
    Found,
    NotFound,
    Error,
}

/// Per-variant projection for [`LookupOutcomeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookupOutcomeKindMeta {
    /// PascalCase wire form.
    pub name: &'static str,
    /// Whether the lookup *succeeded* — Found singleton.
    pub is_found: bool,
    /// Whether the lookup *successfully concluded the manifest
    /// is absent* — NotFound singleton.  Distinct from `Error`
    /// which is a registry-side failure.
    pub is_definite_absence: bool,
    /// Whether the outcome carries a *manifest* payload —
    /// Found singleton.
    pub carries_manifest: bool,
    /// Whether the outcome carries a free-form *error message*
    /// — Error singleton.
    pub carries_error: bool,
}

impl LookupOutcomeKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[Self::Found, Self::NotFound, Self::Error];

    /// Static fact-pack.
    pub const fn meta(self) -> LookupOutcomeKindMeta {
        match self {
            LookupOutcomeKind::Found => LookupOutcomeKindMeta {
                name: "Found",
                is_found: true,
                is_definite_absence: false,
                carries_manifest: true,
                carries_error: false,
            },
            LookupOutcomeKind::NotFound => LookupOutcomeKindMeta {
                name: "NotFound",
                is_found: false,
                is_definite_absence: true,
                carries_manifest: false,
                carries_error: false,
            },
            LookupOutcomeKind::Error => LookupOutcomeKindMeta {
                name: "Error",
                is_found: false,
                is_definite_absence: false,
                carries_manifest: false,
                carries_error: true,
            },
        }
    }
}

impl LookupOutcome {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> LookupOutcomeKind {
        match self {
            LookupOutcome::Found { .. } => LookupOutcomeKind::Found,
            LookupOutcome::NotFound { .. } => LookupOutcomeKind::NotFound,
            LookupOutcome::Error { .. } => LookupOutcomeKind::Error,
        }
    }

    /// Whether the lookup succeeded.  Routes through
    /// `meta().is_found`.
    pub fn is_found(&self) -> bool {
        self.kind().meta().is_found
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

/// Discriminator for [`RegistryError`] — zero-sized projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegistryErrorKind {
    Io,
    Parse,
    Auth,
    Other,
}

/// Per-variant projection for [`RegistryErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryErrorKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// Display-prefix used by the `Display` impl
    /// (`"I/O: <msg>"` etc.).  Single source of truth — pre-
    /// collapse the four prefix strings lived inline as match
    /// arms.
    pub display_prefix: &'static str,
    /// Whether this error originates from the *transport / I/O*
    /// layer — Io singleton.
    pub is_io_failure: bool,
    /// Whether this error originates from *parsing* the
    /// registry payload — Parse singleton.
    pub is_parse_failure: bool,
    /// Whether this error originates from the *authentication*
    /// surface (key check, token expiry) — Auth singleton.
    pub is_auth_failure: bool,
    /// Whether this is the catch-all *other* band — Other
    /// singleton.  Pinned so adding a new variant doesn't
    /// silently land in the catch-all.
    pub is_catch_all: bool,
}

impl RegistryErrorKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] =
        &[Self::Io, Self::Parse, Self::Auth, Self::Other];

    /// Static fact-pack.
    pub const fn meta(self) -> RegistryErrorKindMeta {
        match self {
            RegistryErrorKind::Io => RegistryErrorKindMeta {
                name: "io",
                display_prefix: "I/O: ",
                is_io_failure: true,
                is_parse_failure: false,
                is_auth_failure: false,
                is_catch_all: false,
            },
            RegistryErrorKind::Parse => RegistryErrorKindMeta {
                name: "parse",
                display_prefix: "parse: ",
                is_io_failure: false,
                is_parse_failure: true,
                is_auth_failure: false,
                is_catch_all: false,
            },
            RegistryErrorKind::Auth => RegistryErrorKindMeta {
                name: "auth",
                display_prefix: "auth: ",
                is_io_failure: false,
                is_parse_failure: false,
                is_auth_failure: true,
                is_catch_all: false,
            },
            RegistryErrorKind::Other => RegistryErrorKindMeta {
                name: "other",
                // Other has no display prefix — the message is
                // surfaced verbatim.
                display_prefix: "",
                is_io_failure: false,
                is_parse_failure: false,
                is_auth_failure: false,
                is_catch_all: true,
            },
        }
    }
}

impl RegistryError {
    /// Discriminator projection — strip the message, keep tag.
    pub const fn kind(&self) -> RegistryErrorKind {
        match self {
            RegistryError::Io(_) => RegistryErrorKind::Io,
            RegistryError::Parse(_) => RegistryErrorKind::Parse,
            RegistryError::Auth(_) => RegistryErrorKind::Auth,
            RegistryError::Other(_) => RegistryErrorKind::Other,
        }
    }

    /// Returns the inner message text — every variant carries
    /// one (all four are `Variant(Text)`).
    pub fn message(&self) -> &Text {
        match self {
            RegistryError::Io(t) => t,
            RegistryError::Parse(t) => t,
            RegistryError::Auth(t) => t,
            RegistryError::Other(t) => t,
        }
    }
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display prefix lives in the meta() table — single
        // source of truth replacing the previous 4-arm match.
        write!(
            f,
            "{}{}",
            self.kind().meta().display_prefix,
            self.message().as_str()
        )
    }
}

impl std::error::Error for RegistryError {}

// =============================================================================
// Ed25519 attestation signing / verification (#96 hardening)
// =============================================================================
//

// Pre-this-module, `Attestation::signature` was an opaque `Text`
// blob that the registry stored verbatim — `MemoryRegistry::publish`
// accepted any string as a "signature". An adversary could publish
// a manifest with a fabricated `signature` field and the registry
// would happily serve it to consumers.
//

// Hardening: deterministic Ed25519 over a stable canonical message
// (`name + version + envelope.chain_hash + kind.name`). Every
// `RegistryClient` implementation is encouraged to call
// `verify_attestation` against the publisher's pinned public key
// before accepting; consumers (e.g. the CLI's `verum cog install`)
// run the same check on download.
//

// Key encoding follows the standard 64-hex-character convention
// (32 bytes hex-encoded). Signatures are 128 hex characters
// (64 bytes hex-encoded) — both round-trip through `serde` cleanly.

use ed25519_dalek::{
    SECRET_KEY_LENGTH, Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey,
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

/// Discriminator for [`AttestationCryptoError`] — zero-sized
/// projection.  Splits the four error variants into (decode
/// failure × key-material kind) plus the unique
/// SignatureMismatch verification-failure singleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AttestationCryptoErrorKind {
    InvalidPublicKey,
    InvalidSignature,
    SignatureMismatch,
    InvalidSecretKey,
}

/// Per-variant projection for [`AttestationCryptoErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttestationCryptoErrorKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// Display message — single source of truth replacing the
    /// previous 4-arm match in the `Display` impl.  For the
    /// payload-bearing variants this is the *prefix* before
    /// the inner Text; for `SignatureMismatch` it's the full
    /// (no-payload) message.
    pub display_text: &'static str,
    /// Whether this error is a *decode failure* (the hex blob
    /// couldn't be parsed as the expected length).
    /// InvalidPublicKey + InvalidSignature + InvalidSecretKey.
    /// `SignatureMismatch` is the unique non-decode failure —
    /// the bytes parsed but the cryptographic check failed.
    pub is_decode_failure: bool,
    /// Whether this error involves a *secret key* — pinned so
    /// secret-key-handling code paths can branch on the kind
    /// without per-variant matching, and operators can gate
    /// secret-key telemetry separately from public-key telemetry.
    pub touches_secret_key: bool,
    /// Whether the variant carries a *hex-input* payload
    /// (Text with the bad hex blob) — the three Invalid* kinds.
    /// SignatureMismatch carries no payload.
    pub carries_hex_payload: bool,
}

impl AttestationCryptoErrorKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::InvalidPublicKey,
        Self::InvalidSignature,
        Self::SignatureMismatch,
        Self::InvalidSecretKey,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> AttestationCryptoErrorKindMeta {
        match self {
            AttestationCryptoErrorKind::InvalidPublicKey => AttestationCryptoErrorKindMeta {
                name: "invalid_public_key",
                display_text: "invalid Ed25519 public key: ",
                is_decode_failure: true,
                touches_secret_key: false,
                carries_hex_payload: true,
            },
            AttestationCryptoErrorKind::InvalidSignature => AttestationCryptoErrorKindMeta {
                name: "invalid_signature",
                display_text: "invalid Ed25519 signature: ",
                is_decode_failure: true,
                touches_secret_key: false,
                carries_hex_payload: true,
            },
            AttestationCryptoErrorKind::SignatureMismatch => AttestationCryptoErrorKindMeta {
                name: "signature_mismatch",
                display_text: "Ed25519 signature did not verify",
                is_decode_failure: false,
                touches_secret_key: false,
                carries_hex_payload: false,
            },
            AttestationCryptoErrorKind::InvalidSecretKey => AttestationCryptoErrorKindMeta {
                name: "invalid_secret_key",
                display_text: "invalid Ed25519 secret key: ",
                is_decode_failure: true,
                touches_secret_key: true,
                carries_hex_payload: true,
            },
        }
    }
}

impl AttestationCryptoError {
    /// Discriminator projection — strip the hex-payload, keep
    /// the tag.
    pub const fn kind(&self) -> AttestationCryptoErrorKind {
        match self {
            AttestationCryptoError::InvalidPublicKey(_) => {
                AttestationCryptoErrorKind::InvalidPublicKey
            }
            AttestationCryptoError::InvalidSignature(_) => {
                AttestationCryptoErrorKind::InvalidSignature
            }
            AttestationCryptoError::SignatureMismatch => {
                AttestationCryptoErrorKind::SignatureMismatch
            }
            AttestationCryptoError::InvalidSecretKey(_) => {
                AttestationCryptoErrorKind::InvalidSecretKey
            }
        }
    }

    /// Returns the offending hex blob for the three decode-
    /// failure variants.  `SignatureMismatch` returns `None`.
    pub fn hex_payload(&self) -> Option<&Text> {
        match self {
            AttestationCryptoError::InvalidPublicKey(t)
            | AttestationCryptoError::InvalidSignature(t)
            | AttestationCryptoError::InvalidSecretKey(t) => Some(t),
            AttestationCryptoError::SignatureMismatch => None,
        }
    }
}

impl std::fmt::Display for AttestationCryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display text lives in the meta() table — single
        // source of truth replacing the previous 4-arm match.
        let m = self.kind().meta();
        if let Some(payload) = self.hex_payload() {
            write!(f, "{}{}", m.display_text, payload.as_str())
        } else {
            f.write_str(m.display_text)
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

/// Sign one attestation with an Ed25519 key. Returns the hex-
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

/// Decode a fixed-size hex string into a byte array. Returns
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

    /// Search by tag. Returns matching `(name, version)` pairs.
    fn search(&self, query: &SearchQuery) -> Result<Vec<(Text, CogVersion)>, RegistryError>;

    /// Publish a manifest. The registry validates the envelope's
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

    fn lookup(&self, name: &str, version: &CogVersion) -> Result<LookupOutcome, RegistryError> {
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

/// Disk-backed registry. One JSON file per cog version under
/// `<root>/<name>/<version>.json`. V0 reference impl; V1+
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
        self.root.join(safe_name).join(format!("{}.json", version))
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

    fn lookup(&self, name: &str, version: &CogVersion) -> Result<LookupOutcome, RegistryError> {
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
                RegistryError::Io(Text::from(format!("creating {}: {}", parent.display(), e)))
            })?;
        }
        let json = serde_json::to_string_pretty(m)
            .map_err(|e| RegistryError::Parse(Text::from(format!("serialise manifest: {}", e))))?;
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
/// consensus on the chain hash. Returns `Found` only when every
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
    /// chain hash. False means at least one mirror disagrees —
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
    ///

    /// **V0 / back-compat behaviour.** Use [`lookup_with_consensus_policy`]
    /// for the production gates (minimum quorum, identity match,
    /// signature verification under publisher pubkeys).
    pub fn lookup_with_consensus(
        &self,
        name: &str,
        version: &CogVersion,
    ) -> MirrorConsensusVerdict {
        self.lookup_with_consensus_policy(name, version, &MirrorConsensusPolicy::default())
    }

    /// Hardened lookup that applies the supplied policy gates on top
    /// of the V0 chain-hash agreement. Returns `consensus = true`
    /// only when every gate passes:
    ///

    ///  1. Every `Found` verdict's manifest `chain_hash` is identical.
    ///  2. The `Found` count is ≥ `policy.min_quorum`.
    ///  3. When `policy.require_identity_match`, every `Found`'s
    ///  manifest name + version match the query.
    ///  4. For every `(kind, pubkey)` pair in
    ///  `policy.required_attestations`, every `Found`'s
    ///  manifest carries an attestation of `kind` whose Ed25519
    ///  signature verifies under one of the configured pubkeys.
    pub fn lookup_with_consensus_policy(
        &self,
        name: &str,
        version: &CogVersion,
        policy: &MirrorConsensusPolicy,
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

        let found_manifests: Vec<&CogManifest> = per_mirror
            .values()
            .filter_map(|o| match o {
                LookupOutcome::Found { manifest } => Some(manifest),
                _ => None,
            })
            .collect();
        let chain_hashes: Vec<&Text> = found_manifests
            .iter()
            .map(|m| &m.envelope.chain_hash)
            .collect();

        // Gate 1 — chain-hash agreement.
        let chain_consensus = match chain_hashes.first() {
            None => true,
            Some(first) => chain_hashes.iter().all(|h| h.as_str() == first.as_str()),
        };

        // Gate 2 — minimum quorum.
        let quorum_ok = found_manifests.len() >= policy.min_quorum;

        // Gate 3 — identity match (when required).
        let identity_ok = !policy.require_identity_match
            || found_manifests
                .iter()
                .all(|m| m.name.as_str() == name && &m.version == version);

        // Gate 4 — Ed25519 attestation verification (when required).
        let attestation_ok = policy.required_attestations.is_empty()
            || found_manifests.iter().all(|m| {
                policy.required_attestations.iter().all(|(kind, pubkeys)| {
                    let chain_hash = m.envelope.chain_hash.as_str();
                    m.attestations.iter().any(|att| {
                        att.kind == *kind
                            && pubkeys.iter().any(|pk| {
                                verify_attestation(pk, m.name.as_str(), &m.version, chain_hash, att)
                                    .is_ok()
                            })
                    })
                })
            });

        let consensus = chain_consensus && quorum_ok && identity_ok && attestation_ok;
        let agreed = if consensus {
            chain_hashes.first().map(|h| (*h).clone())
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

// =============================================================================
// MirrorConsensusPolicy — production-mode gates for #106 hardening
// =============================================================================

/// Policy applied by [`MultiMirrorClient::lookup_with_consensus_policy`]
/// on top of the V0 chain-hash agreement check. Each gate is opt-in
/// so callers can layer them: production verifiers configure all
/// three (quorum + identity + attestation), while the V0 default
/// (`Default::default()`) preserves chain-hash-only behaviour.
#[derive(Debug, Clone, Default)]
pub struct MirrorConsensusPolicy {
    /// Minimum number of mirrors that must return `Found` for
    /// consensus to hold. Default `0` accepts any number ≥ 0
    /// (V0 behaviour).
    pub min_quorum: usize,
    /// When `true`, every `Found` manifest's `name` and `version`
    /// MUST equal the query's. Default `false` (V0 behaviour).
    pub require_identity_match: bool,
    /// Per-attestation-kind list of publisher Ed25519 public keys
    /// (hex-encoded, 64 chars). Every `Found` manifest must carry
    /// an attestation of each listed kind whose signature verifies
    /// under one of the listed pubkeys. Default empty (no gate).
    pub required_attestations: Vec<(AttestationKind, Vec<Text>)>,
}

impl MirrorConsensusPolicy {
    /// Convenience constructor: production policy gating on all
    /// three orthogonal factors.
    pub fn production(
        min_quorum: usize,
        publisher_pubkeys_per_kind: Vec<(AttestationKind, Vec<Text>)>,
    ) -> Self {
        Self {
            min_quorum,
            require_identity_match: true,
            required_attestations: publisher_pubkeys_per_kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift-pin: `PublishOutcomeKind` discriminator projection.
    /// Pins variant count, name uniqueness, four classifier
    /// partitions (accepted / collision / chain-hash-bearing /
    /// reason-bearing), and the cross-cutting invariants binding
    /// them.
    #[test]
    fn meta_pin_publish_outcome_kind_round_trip_and_partitions() {
        assert_eq!(PublishOutcomeKind::ALL.len(), 3);

        // PascalCase wire form, unique names.
        let mut seen = std::collections::HashSet::new();
        for k in PublishOutcomeKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().next().map_or(false, |c| c.is_ascii_uppercase()),
                "{:?}: name not PascalCase",
                k
            );
            assert!(seen.insert(m.name), "{:?}: duplicate", k);
        }

        // is_accepted: Accepted singleton.
        let acc: Vec<_> = PublishOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().is_accepted)
            .copied()
            .collect();
        assert_eq!(acc, vec![PublishOutcomeKind::Accepted]);

        // is_collision: VersionConflict singleton.
        let col: Vec<_> = PublishOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().is_collision)
            .copied()
            .collect();
        assert_eq!(col, vec![PublishOutcomeKind::VersionConflict]);

        // is_accepted ⊕ is_collision (an outcome is at most one
        // of these — the third variant Rejected is neither).
        for k in PublishOutcomeKind::ALL {
            let m = k.meta();
            assert!(!(m.is_accepted && m.is_collision), "{:?}: ⊕", k);
        }

        // carries_chain_hash: Accepted + VersionConflict.
        let ch: Vec<_> = PublishOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().carries_chain_hash)
            .copied()
            .collect();
        assert_eq!(
            ch,
            vec![
                PublishOutcomeKind::Accepted,
                PublishOutcomeKind::VersionConflict,
            ],
        );

        // carries_reason: Rejected singleton.
        let rs: Vec<_> = PublishOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().carries_reason)
            .copied()
            .collect();
        assert_eq!(rs, vec![PublishOutcomeKind::Rejected]);

        // carries_chain_hash ⊕ carries_reason (every variant
        // carries exactly one structured payload).
        for k in PublishOutcomeKind::ALL {
            let m = k.meta();
            assert!(
                m.carries_chain_hash ^ m.carries_reason,
                "{:?}: must carry exactly one of chain_hash / reason",
                k
            );
        }

        // Live-payload kind() + name() routing.
        let acc = PublishOutcome::Accepted {
            chain_hash: Text::from("abc123"),
        };
        assert_eq!(acc.kind(), PublishOutcomeKind::Accepted);
        assert!(acc.is_accepted());
        assert_eq!(acc.name(), "Accepted");

        let rej = PublishOutcome::Rejected {
            reason: Text::from("malformed"),
        };
        assert_eq!(rej.kind(), PublishOutcomeKind::Rejected);
        assert!(!rej.is_accepted());
        assert_eq!(rej.name(), "Rejected");

        let conf = PublishOutcome::VersionConflict {
            existing_chain_hash: Text::from("a"),
            proposed_chain_hash: Text::from("b"),
        };
        assert_eq!(conf.kind(), PublishOutcomeKind::VersionConflict);
        assert!(!conf.is_accepted());
    }

    /// Drift-pin: `LookupOutcomeKind`.
    #[test]
    fn meta_pin_lookup_outcome_kind_round_trip_and_partitions() {
        assert_eq!(LookupOutcomeKind::ALL.len(), 3);

        // Singleton partitions (every variant is its own
        // singleton — the three classifier flags partition the
        // three variants).
        let found: Vec<_> = LookupOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().is_found)
            .copied()
            .collect();
        assert_eq!(found, vec![LookupOutcomeKind::Found]);

        let abs_: Vec<_> = LookupOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().is_definite_absence)
            .copied()
            .collect();
        assert_eq!(abs_, vec![LookupOutcomeKind::NotFound]);

        let err_: Vec<_> = LookupOutcomeKind::ALL
            .iter()
            .filter(|k| k.meta().carries_error)
            .copied()
            .collect();
        assert_eq!(err_, vec![LookupOutcomeKind::Error]);

        // Each variant flips exactly one of {is_found,
        // is_definite_absence, carries_error} — perfect
        // partition pinned.
        for k in LookupOutcomeKind::ALL {
            let m = k.meta();
            let count = (m.is_found as u32)
                + (m.is_definite_absence as u32)
                + (m.carries_error as u32);
            assert_eq!(count, 1, "{:?}: must flip exactly one classifier", k);
        }

        // carries_manifest = is_found (only Found carries the
        // manifest payload).
        for k in LookupOutcomeKind::ALL {
            let m = k.meta();
            assert_eq!(m.carries_manifest, m.is_found);
        }
    }

    /// Drift-pin: `RegistryErrorKind`.  Pins the Display-prefix
    /// table, the four single-flag classifiers, and the
    /// catch-all-singleton invariant.
    #[test]
    fn meta_pin_registry_error_kind_round_trip_and_display() {
        assert_eq!(RegistryErrorKind::ALL.len(), 4);

        // Each variant flips exactly one of the four
        // single-flag classifiers (is_io / is_parse / is_auth
        // / is_catch_all) — perfect partition.
        for k in RegistryErrorKind::ALL {
            let m = k.meta();
            let count = (m.is_io_failure as u32)
                + (m.is_parse_failure as u32)
                + (m.is_auth_failure as u32)
                + (m.is_catch_all as u32);
            assert_eq!(count, 1, "{:?}: must flip exactly one classifier", k);
        }

        // Names are unique snake_case + match expected variant
        // tags.
        let mut seen = std::collections::HashSet::new();
        for k in RegistryErrorKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?}: name not snake_case",
                k
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", k);
        }
        assert_eq!(RegistryErrorKind::Io.meta().name, "io");
        assert_eq!(RegistryErrorKind::Parse.meta().name, "parse");
        assert_eq!(RegistryErrorKind::Auth.meta().name, "auth");
        assert_eq!(RegistryErrorKind::Other.meta().name, "other");

        // Display-prefix table pinned.  Other has empty prefix
        // (catch-all message surfaces verbatim).
        assert_eq!(RegistryErrorKind::Io.meta().display_prefix, "I/O: ");
        assert_eq!(RegistryErrorKind::Parse.meta().display_prefix, "parse: ");
        assert_eq!(RegistryErrorKind::Auth.meta().display_prefix, "auth: ");
        assert_eq!(RegistryErrorKind::Other.meta().display_prefix, "");

        // Display impl routes through the meta() table.
        let io = RegistryError::Io(Text::from("connection lost"));
        assert_eq!(io.kind(), RegistryErrorKind::Io);
        assert_eq!(format!("{}", io), "I/O: connection lost");

        let other = RegistryError::Other(Text::from("misc"));
        assert_eq!(other.kind(), RegistryErrorKind::Other);
        assert_eq!(format!("{}", other), "misc");
    }

    /// Drift-pin: `AttestationCryptoErrorKind`.  Pins the four
    /// classifiers (decode-failure, secret-key-touching,
    /// hex-payload-bearing) and the SignatureMismatch singleton
    /// invariant.
    #[test]
    fn meta_pin_attestation_crypto_error_kind_round_trip_and_partitions() {
        assert_eq!(AttestationCryptoErrorKind::ALL.len(), 4);

        // is_decode_failure — three Invalid* variants.
        let decode: Vec<_> = AttestationCryptoErrorKind::ALL
            .iter()
            .filter(|k| k.meta().is_decode_failure)
            .copied()
            .collect();
        assert_eq!(
            decode,
            vec![
                AttestationCryptoErrorKind::InvalidPublicKey,
                AttestationCryptoErrorKind::InvalidSignature,
                AttestationCryptoErrorKind::InvalidSecretKey,
            ],
        );

        // touches_secret_key — InvalidSecretKey singleton.
        let sk: Vec<_> = AttestationCryptoErrorKind::ALL
            .iter()
            .filter(|k| k.meta().touches_secret_key)
            .copied()
            .collect();
        assert_eq!(sk, vec![AttestationCryptoErrorKind::InvalidSecretKey]);

        // carries_hex_payload = is_decode_failure (the three
        // decode-failure variants are exactly the hex-bearing
        // variants).  SignatureMismatch is the unique non-
        // payload-bearing variant.
        for k in AttestationCryptoErrorKind::ALL {
            let m = k.meta();
            assert_eq!(
                m.carries_hex_payload, m.is_decode_failure,
                "{:?}: hex_payload = decode_failure",
                k
            );
        }

        // SignatureMismatch is the unique verification failure
        // (not a decode failure, no hex payload).
        let mismatch: Vec<_> = AttestationCryptoErrorKind::ALL
            .iter()
            .filter(|k| !k.meta().is_decode_failure)
            .copied()
            .collect();
        assert_eq!(mismatch, vec![AttestationCryptoErrorKind::SignatureMismatch]);

        // touches_secret_key ⇒ is_decode_failure (you can only
        // get a secret-key error during the decode — there's
        // no signature-mismatch path that touches the secret
        // key, by construction Ed25519 verification only uses
        // the public key).
        for k in AttestationCryptoErrorKind::ALL {
            let m = k.meta();
            assert!(
                !m.touches_secret_key || m.is_decode_failure,
                "{:?}: touches_secret_key ⇒ is_decode_failure",
                k
            );
        }

        // Display routing.
        let pk = AttestationCryptoError::InvalidPublicKey(Text::from("bad-hex"));
        assert_eq!(pk.kind(), AttestationCryptoErrorKind::InvalidPublicKey);
        assert_eq!(pk.hex_payload().unwrap().as_str(), "bad-hex");
        assert_eq!(
            format!("{}", pk),
            "invalid Ed25519 public key: bad-hex",
        );

        let mm = AttestationCryptoError::SignatureMismatch;
        assert_eq!(mm.kind(), AttestationCryptoErrorKind::SignatureMismatch);
        assert!(mm.hex_payload().is_none());
        assert_eq!(format!("{}", mm), "Ed25519 signature did not verify");
    }

    fn fixture_envelope() -> CogReproEnvelope {
        CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"compiled-output")
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
        let with_diff_env = CogReproEnvelope::compute(b"sources", b"different", b"compiled-output");
        let with_diff_out = CogReproEnvelope::compute(b"sources", b"toolchain-pin", b"different");
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
        let look = r
            .lookup("does-not-exist", &CogVersion::new(1, 0, 0))
            .unwrap();
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
        let base = attestation_message("alpha", &v1, "abcd", AttestationKind::VerifiedCi);
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
        let sig = sign_attestation(&sk, "alpha", &v, &chain, AttestationKind::VerifiedCi).unwrap();
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
        match sign_attestation("deadbeef", "alpha", &v, "abcd", AttestationKind::VerifiedCi) {
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

    // =========================================================================
    // MirrorConsensusPolicy (#106 hardening)
    // =========================================================================

    fn publish_to(reg: &dyn RegistryClient, manifest: &CogManifest) {
        match reg.publish(manifest).unwrap() {
            PublishOutcome::Accepted { .. } => {}
            other => panic!("publish failed: {:?}", other),
        }
    }

    fn signed_manifest(name: &str, version: CogVersion, sk: &str) -> CogManifest {
        let mut m = fixture_manifest(name, version.clone());
        let sig = sign_attestation(
            sk,
            name,
            &version,
            m.envelope.chain_hash.as_str(),
            AttestationKind::VerifiedCi,
        )
        .unwrap();
        m.attestations.push(Attestation {
            kind: AttestationKind::VerifiedCi,
            signer: Text::from("ci@verum"),
            signature: sig,
            timestamp: 0,
        });
        m
    }

    #[test]
    fn consensus_policy_default_preserves_v0_chain_hash_only() {
        // Default policy = chain-hash agreement only. Single-Found
        // result still passes consensus (same as V0).
        let m1 = MemoryRegistry::new("m1");
        let m2 = MemoryRegistry::new("m2");
        let manifest = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        publish_to(&m1, &manifest);
        // m2 has no entry — `NotFound` does NOT break consensus.
        let composite = MultiMirrorClient::new(vec![Box::new(m1), Box::new(m2)]);
        let v = composite.lookup_with_consensus("alpha", &CogVersion::new(1, 0, 0));
        assert!(v.consensus);
    }

    #[test]
    fn consensus_policy_min_quorum_enforces_minimum_found_count() {
        let m1 = MemoryRegistry::new("m1");
        let m2 = MemoryRegistry::new("m2");
        let manifest = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        publish_to(&m1, &manifest);
        // Only m1 has the entry — quorum=2 should reject.
        let composite = MultiMirrorClient::new(vec![Box::new(m1), Box::new(m2)]);
        let policy = MirrorConsensusPolicy {
            min_quorum: 2,
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(!v.consensus);
    }

    #[test]
    fn consensus_policy_min_quorum_passes_when_enough_mirrors_found() {
        let m1 = MemoryRegistry::new("m1");
        let m2 = MemoryRegistry::new("m2");
        let manifest = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        publish_to(&m1, &manifest);
        publish_to(&m2, &manifest);
        let composite = MultiMirrorClient::new(vec![Box::new(m1), Box::new(m2)]);
        let policy = MirrorConsensusPolicy {
            min_quorum: 2,
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(v.consensus);
    }

    #[test]
    fn consensus_policy_identity_match_rejects_confused_deputy() {
        // Construct a manifest whose name doesn't match the query —
        // a confused-deputy mirror returning the wrong cog.
        let m1 = MemoryRegistry::new("m1");
        let mut wrong_name = fixture_manifest("alpha", CogVersion::new(1, 0, 0));
        wrong_name.name = Text::from("WRONG_COG");
        // Bypass `publish`'s name+version key by manually inserting.
        m1.entries
            .lock()
            .unwrap()
            .insert((Text::from("alpha"), CogVersion::new(1, 0, 0)), wrong_name);
        let composite = MultiMirrorClient::new(vec![Box::new(m1)]);
        let policy = MirrorConsensusPolicy {
            require_identity_match: true,
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(!v.consensus, "identity mismatch must break consensus");
    }

    #[test]
    fn consensus_policy_attestation_gate_rejects_unsigned_manifest() {
        let m1 = MemoryRegistry::new("m1");
        // No attestation attached.
        publish_to(&m1, &fixture_manifest("alpha", CogVersion::new(1, 0, 0)));
        let composite = MultiMirrorClient::new(vec![Box::new(m1)]);
        let (_sk, pk) = test_keypair(0xab);
        let policy = MirrorConsensusPolicy {
            min_quorum: 1,
            required_attestations: vec![(AttestationKind::VerifiedCi, vec![Text::from(pk)])],
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(!v.consensus, "missing attestation must break consensus");
    }

    #[test]
    fn consensus_policy_attestation_gate_admits_correctly_signed_manifest() {
        let (sk, pk) = test_keypair(0xab);
        let m1 = MemoryRegistry::new("m1");
        let manifest = signed_manifest("alpha", CogVersion::new(1, 0, 0), &sk);
        publish_to(&m1, &manifest);
        let composite = MultiMirrorClient::new(vec![Box::new(m1)]);
        let policy = MirrorConsensusPolicy {
            min_quorum: 1,
            required_attestations: vec![(AttestationKind::VerifiedCi, vec![Text::from(pk)])],
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(v.consensus);
    }

    #[test]
    fn consensus_policy_attestation_gate_rejects_wrong_publisher_key() {
        let (sk_legit, _pk_legit) = test_keypair(0x11);
        let (_sk_attacker, pk_attacker) = test_keypair(0x22);
        let m1 = MemoryRegistry::new("m1");
        // Manifest signed by `legit` but policy expects `attacker`.
        let manifest = signed_manifest("alpha", CogVersion::new(1, 0, 0), &sk_legit);
        publish_to(&m1, &manifest);
        let composite = MultiMirrorClient::new(vec![Box::new(m1)]);
        let policy = MirrorConsensusPolicy {
            min_quorum: 1,
            required_attestations: vec![(
                AttestationKind::VerifiedCi,
                vec![Text::from(pk_attacker)],
            )],
            ..Default::default()
        };
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(!v.consensus);
    }

    #[test]
    fn task_106_production_policy_layers_all_three_gates() {
        // Pin: the production policy = quorum + identity + signature.
        // All three must hold simultaneously for consensus to admit.
        let (sk, pk) = test_keypair(0x42);
        let m1 = MemoryRegistry::new("m1");
        let m2 = MemoryRegistry::new("m2");
        let manifest = signed_manifest("alpha", CogVersion::new(1, 0, 0), &sk);
        publish_to(&m1, &manifest);
        publish_to(&m2, &manifest);
        let composite = MultiMirrorClient::new(vec![Box::new(m1), Box::new(m2)]);
        let policy = MirrorConsensusPolicy::production(
            2,
            vec![(AttestationKind::VerifiedCi, vec![Text::from(pk)])],
        );
        let v = composite.lookup_with_consensus_policy("alpha", &CogVersion::new(1, 0, 0), &policy);
        assert!(v.consensus);
        assert!(policy.require_identity_match);
        assert_eq!(policy.min_quorum, 2);
    }
}
