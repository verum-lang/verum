//! Verifier-selected resource policy. These limits are never read from a proof.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionLimits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}

impl AdmissionLimits {
    pub const DEFAULT: Self = Self {
        max_bytes: 1024 * 1024,
        max_nodes: 65_536,
        max_depth: 128,
    };

    /// The JSON representation adds wrappers around logical term nodes.
    /// Check arithmetic rather than silently saturating an invalid policy.
    pub fn json_nesting_limit(self) -> Option<usize> {
        self.max_depth
            .checked_add(1)?
            .checked_mul(8)?
            .checked_add(8)
    }

    pub fn read_limit(self) -> Option<u64> {
        u64::try_from(self.max_bytes).ok()?.checked_add(1)
    }

    pub fn is_valid(self) -> bool {
        self.json_nesting_limit().is_some() && self.read_limit().is_some()
    }
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLimits {
    pub wall_time_ms: u64,
    pub cpu_seconds: u64,
    pub memory_bytes: u64,
    pub stack_bytes: u64,
    pub output_bytes: u64,
}

impl ProcessLimits {
    pub const DEFAULT: Self = Self {
        wall_time_ms: 5000,
        cpu_seconds: 3,
        memory_bytes: 512 * 1024 * 1024,
        stack_bytes: 16 * 1024 * 1024,
        output_bytes: 64 * 1024,
    };

    pub fn is_valid(self) -> bool {
        self.cpu_seconds > 0
            && self.memory_bytes > 0
            && self.stack_bytes > 0
            && self.output_bytes > 0
            && self.output_bytes.checked_add(1).is_some()
    }
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayLimits {
    pub admission: AdmissionLimits,
    pub process: ProcessLimits,
}

impl ReplayLimits {
    pub fn is_valid(self) -> bool {
        self.admission.is_valid() && self.process.is_valid()
    }
}

/// Actual OS ceilings may be stricter than the requested policy because a
/// worker inherits its supervisor's limits. RSS monitoring remains active
/// when the platform cannot enforce an address-space ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledLimits {
    pub cpu_seconds: u64,
    pub address_space_bytes: Option<u64>,
    pub stack_bytes: u64,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReceipt {
    pub requested: ReplayLimits,
    /// Absent when installation failed or the supervisor received no valid
    /// worker receipt. A child may omit it only for sandbox refusal.
    pub installed: Option<InstalledLimits>,
}
