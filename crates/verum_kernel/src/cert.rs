//! SMT certificate envelope — `SmtCertificate` + schema versioning.
//! Split per #198 V7.
//!
//! The kernel consumes this via [`crate::replay_smt_cert`] and
//! reconstructs a [`crate::CoreTerm`] witness. This is the primary
//! mechanism that takes Z3 / CVC5 / E / Vampire / Alt-Ergo **out of
//! the TCB**: a bug in a solver that produced a spurious "proof"
//! will fail the replay there, not leak into accepted theorems.

use serde::{Deserialize, Serialize};
use verum_common::{List, Text};

use crate::KernelError;

/// A proof certificate produced by an SMT backend.
///
/// The kernel consumes this via [`crate::replay_smt_cert`] and
/// reconstructs a [`crate::CoreTerm`] witness. This is the primary
/// mechanism that takes Z3 / CVC5 / E / Vampire / Alt-Ergo **out of
/// the TCB**: a bug in a solver that produced a spurious "proof"
/// will fail the replay here, not leak into accepted theorems.
///
/// The certificate format is backend-neutral: each backend's native
/// proof trace is normalized into the common shape by
/// `verum_smt::proof_extraction` before landing here.
///
/// # Envelope versioning
///
/// `schema_version` identifies the certificate envelope format. The
/// kernel rejects any certificate whose `schema_version` is greater
/// than [`CERTIFICATE_SCHEMA_VERSION`] — this lets forward
/// compatibility be negotiated explicitly rather than silently
/// accepting unknown-shape envelopes. Version `0` is treated as
/// "legacy unversioned" for backward compatibility with pre-envelope
/// certificates on disk.
///
/// # Metadata
///
/// `metadata` is a free-form key/value store for non-trust-relevant
/// annotations (tactics used, solver options, timing, obligation
/// provenance, …). The kernel never reads these fields — they are
/// carried end-to-end so tooling (`verum audit --framework-axioms`,
/// proof export, cross-tool replay) can preserve diagnostic context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtCertificate {
    /// Envelope schema version. Zero means "legacy unversioned";
    /// current shipping version is [`CERTIFICATE_SCHEMA_VERSION`].
    #[serde(default)]
    pub schema_version: u32,
    /// Which backend produced the certificate (for routing the replay).
    pub backend: Text,
    /// Backend version — certificates are keyed by version because
    /// different solver versions can have different proof-rule sets.
    pub backend_version: Text,
    /// Serialized proof trace. Format is backend-specific; the replay
    /// routine knows how to parse each known backend.
    pub trace: List<u8>,
    /// Hash of the obligation so callers can cross-check that the
    /// certificate belongs to the goal they were trying to prove.
    pub obligation_hash: Text,
    /// Verum compiler version that produced the certificate. Used by
    /// the cross-tool replay matrix (task #90) to key CI runs.
    #[serde(default)]
    pub verum_version: Text,
    /// ISO-8601 timestamp of certificate creation (UTC). Allows
    /// disk-cached certificates to be invalidated by age without
    /// re-hashing.
    #[serde(default)]
    pub created_at: Text,
    /// Free-form non-trust-relevant annotations. Not inspected by the
    /// kernel.
    #[serde(default)]
    pub metadata: List<(Text, Text)>,
}

/// Current SmtCertificate envelope schema version.
///
/// Bump this constant whenever the envelope shape changes
/// incompatibly. The kernel rejects any certificate whose
/// `schema_version` exceeds this value, which gives tooling a clean
/// error path on version skew.
pub const CERTIFICATE_SCHEMA_VERSION: u32 = 1;

impl SmtCertificate {
    /// Construct a new certificate with the current schema version
    /// and [`verum_version`] filled in from the crate metadata.
    ///
    /// `created_at` is left empty; callers that want timestamps
    /// should populate them via [`with_created_at`] (the kernel
    /// crate is intentionally free of `chrono`/`std::time::SystemTime`
    /// dependencies to keep the TCB minimal).
    ///
    /// [`verum_version`]: Self::verum_version
    /// [`with_created_at`]: Self::with_created_at
    pub fn new(
        backend: Text,
        backend_version: Text,
        trace: List<u8>,
        obligation_hash: Text,
    ) -> Self {
        Self {
            schema_version: CERTIFICATE_SCHEMA_VERSION,
            backend,
            backend_version,
            trace,
            obligation_hash,
            verum_version: Text::from(env!("CARGO_PKG_VERSION")),
            created_at: Text::new(),
            metadata: List::new(),
        }
    }

    /// Attach an ISO-8601 timestamp to the certificate. The kernel
    /// does not parse this field — it is carried end-to-end for
    /// tooling use.
    pub fn with_created_at(mut self, ts: Text) -> Self {
        self.created_at = ts;
        self
    }

    /// Attach a single metadata key/value pair. See the struct-level
    /// docs for what metadata is used for.
    pub fn with_metadata(mut self, key: Text, value: Text) -> Self {
        self.metadata.push((key, value));
        self
    }

    /// Validate the envelope schema. Returns [`Err`] if the schema
    /// version is newer than this kernel build understands.
    ///
    /// Version `0` is accepted as "legacy unversioned" for backward
    /// compatibility with pre-1.0 on-disk certificates.
    pub fn validate_schema(&self) -> Result<(), KernelError> {
        if self.schema_version > CERTIFICATE_SCHEMA_VERSION {
            return Err(KernelError::UnsupportedCertificateSchema {
                found: self.schema_version,
                max_supported: CERTIFICATE_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}
