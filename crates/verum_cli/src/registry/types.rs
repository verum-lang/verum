// Cog registry types: package metadata, version resolution, dependency graphs
// Registry types for cog distribution

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use verum_common::{List, Map, Text};

/// Cog metadata from registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CogMetadata {
    pub name: Text,
    pub version: Text,
    pub description: Option<Text>,
    pub authors: List<Text>,
    pub license: Option<Text>,
    pub repository: Option<Text>,
    pub homepage: Option<Text>,
    pub keywords: List<Text>,
    pub categories: List<Text>,
    pub readme: Option<Text>,

    /// Dependencies
    pub dependencies: Map<Text, DependencySpec>,

    /// Features
    pub features: Map<Text, List<Text>>,

    /// Tier-specific artifacts
    pub artifacts: TierArtifacts,

    /// Verification proofs
    pub proofs: Option<VerificationProofs>,

    /// CBGR performance profiles
    pub cbgr_profiles: Option<CbgrProfiles>,

    /// Cog signature (Ed25519)
    pub signature: Option<CogSignature>,

    /// IPFS hash for decentralized distribution
    pub ipfs_hash: Option<Text>,

    /// Checksum (SHA-256)
    pub checksum: Text,

    /// Published timestamp
    pub published_at: i64,
}

/// Dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    Simple(Text),
    Detailed {
        version: Option<Text>,
        features: Option<List<Text>>,
        optional: Option<bool>,
        default_features: Option<bool>,
    },
}

/// Tier-specific build artifacts
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TierArtifacts {
    /// Tier 0: AST cache
    pub tier0: Option<ArtifactInfo>,

    /// Tier 1: JIT compiled code
    pub tier1: Option<ArtifactInfo>,

    /// Tier 2: AOT debug binary
    pub tier2: Option<ArtifactInfo>,

    /// Tier 3: AOT optimized binary
    pub tier3: Option<ArtifactInfo>,
}

/// Artifact information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactInfo {
    /// File path in cog
    pub path: Text,

    /// SHA-256 checksum
    pub checksum: Text,

    /// Size in bytes
    pub size: u64,

    /// Target triple (for tier 2/3)
    pub target: Option<Text>,
}

/// Verification proofs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationProofs {
    /// SMT solver used
    pub solver: Text,

    /// Proof files
    pub proofs: List<ProofInfo>,

    /// Verification level
    pub level: VerificationLevel,
}

/// Proof information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofInfo {
    pub function: Text,
    pub status: ProofStatus,
    pub time_ms: u64,
    pub file: Option<Text>,
}

/// Proof status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProofStatus {
    Verified,
    Runtime,
    Failed,
}

/// Verification level
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationLevel {
    None,
    Runtime,
    Proof,
}

/// CBGR performance profiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CbgrProfiles {
    /// Default profile
    pub default: CbgrProfile,

    /// Optimized profile (more escape analysis)
    pub optimized: Option<CbgrProfile>,

    /// Minimal profile (fewer checks)
    pub minimal: Option<CbgrProfile>,
}

/// CBGR profile data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CbgrProfile {
    /// Average check overhead in nanoseconds
    pub avg_check_ns: f64,

    /// Peak memory overhead percentage
    pub memory_overhead_pct: f64,

    /// Number of optimizable references
    pub optimizable_refs: usize,

    /// Total CBGR checks
    pub total_checks: usize,
}

/// Cog signature using Ed25519
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CogSignature {
    /// Public key (hex-encoded)
    pub public_key: Text,

    /// Signature (hex-encoded)
    pub signature: Text,

    /// Timestamp
    pub signed_at: i64,
}

/// Cog source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CogSource {
    Registry {
        registry: Text,
        version: Text,
    },
    Git {
        url: Text,
        branch: Option<Text>,
        tag: Option<Text>,
        rev: Option<Text>,
    },
    Ipfs {
        hash: Text,
    },
    Path {
        path: PathBuf,
    },
}

/// Cog search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: Text,
    pub version: Text,
    pub description: Option<Text>,
    pub downloads: u64,
    pub verified: bool,
    pub cbgr_optimized: bool,
}

/// Registry index entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: Text,
    pub versions: List<VersionEntry>,
}

/// Version entry in index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version: Text,
    pub checksum: Text,
    pub yanked: bool,
    pub features: Map<Text, List<Text>>,
}

/// Bundle manifest for offline distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub format_version: u32,
    pub created_at: i64,
    pub packages: List<BundleCog>,
    pub include_toolchain: bool,
    pub tier: Option<u8>,
}

/// Cog in bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleCog {
    pub name: Text,
    pub version: Text,
    pub checksum: Text,
    pub path: Text,
}

/// Vulnerability report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnerabilityReport {
    pub package: Text,
    pub version: Text,
    pub vulnerabilities: List<Vulnerability>,
}

/// Vulnerability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: Text,
    pub severity: Severity,
    pub title: Text,
    pub description: Text,
    pub patched_versions: List<Text>,
}

/// Vulnerability severity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
