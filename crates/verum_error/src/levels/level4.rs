//! Level 4: Security Containment (Isolation Boundaries)
//!
//! The outermost layer of the 5-Level Error Defense Architecture. Level 4 limits
//! the blast radius of failures through capability-based security and isolation
//! boundaries. The context system (`using [...]`) tracks required capabilities at
//! the type level, and sandboxed code can only invoke operations for which it
//! holds explicit capabilities. Four isolation levels are defined:
//!
//! - **Trusted**: Full access, no isolation
//! - **Sandbox**: Limited capabilities, isolated from system resources
//! - **Restricted**: Minimal capabilities, ephemeral execution
//! - **Hardened**: Maximum restrictions, formal verification required
//!
//! Fine-grained capabilities control access to file I/O, networking, process
//! spawning, and other system resources. Sandbox enforcement prevents escalation.
//!
//! Provides error types for **security violations and isolation boundaries**. This level
//! protects against malicious or compromised code through capability-based isolation.
//!
//! # Error Categories
//!
//! - **Authentication failures** - identity verification failed
//! - **Authorization violations** - insufficient permissions
//! - **Capability violations** - missing required capabilities
//! - **Sandbox violations** - attempted escape from isolation boundary
//! - **Resource limit exceeded** - quota or rate limit exceeded
//!
//! # Capability-Based Security
//!
//! Level 4 uses a capability-based security model:
//!
//! ```rust,ignore
//! // Code is restricted to specific capabilities
//! fn process_file(file: &File, creds: Capability<FileIO>) -> Result<()> {
//!     // Can only access file-related operations
//!     // Cannot escape sandbox or access other resources
//!     file.read()
//! }
//! ```
//!
//! # Isolation Levels
//!
//! - **Level 1 (Trusted)**: Full access, no isolation
//! - **Level 2 (Sandbox)**: Limited capabilities, isolated from system
//! - **Level 3 (Restricted)**: Minimal capabilities, ephemeral execution
//! - **Level 4 (Hardened)**: Maximum restrictions, formal verification required
//!
//! # Use Cases
//!
//! - Running untrusted plugins safely
//! - Executing user-provided scripts
//! - Multi-tenant isolation
//! - Defense-in-depth security
//! - Zero-trust architecture
//!
//! Provides error types for security and isolation violations.

use crate::{ErrorKind, VerumError};
use verum_common::Text;

/// Security error
///
/// Indicates an authentication or authorization failure.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Security violation: {message}")]
pub struct SecurityError {
    /// Error message
    pub message: Text,
    /// Violated policy (if known)
    pub policy: Option<Text>,
}

impl SecurityError {
    /// Create a new security error
    pub fn new(message: impl Into<Text>) -> Self {
        Self {
            message: message.into(),
            policy: None,
        }
    }

    /// Add violated policy
    pub fn with_policy(mut self, policy: impl Into<Text>) -> Self {
        self.policy = Some(policy.into());
        self
    }
}

impl From<SecurityError> for VerumError {
    fn from(err: SecurityError) -> Self {
        let mut message = format!("Security violation: {}", err.message);
        if let Some(policy) = err.policy {
            message.push_str(&format!(" (policy: {})", policy));
        }
        VerumError::new(message, ErrorKind::Security)
    }
}

/// Capability violation
///
/// Indicates an operation was attempted without required capabilities.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Capability violation: {capability}")]
pub struct CapabilityError {
    /// Required capability
    pub capability: Text,
    /// Operation that required it
    pub operation: Option<Text>,
}

impl CapabilityError {
    /// Create a new capability error
    pub fn new(capability: impl Into<Text>) -> Self {
        Self {
            capability: capability.into(),
            operation: None,
        }
    }

    /// Add operation name
    pub fn with_operation(mut self, operation: impl Into<Text>) -> Self {
        self.operation = Some(operation.into());
        self
    }
}

impl From<CapabilityError> for VerumError {
    fn from(err: CapabilityError) -> Self {
        let mut message = format!("Capability violation: {}", err.capability);
        if let Some(op) = err.operation {
            message.push_str(&format!(" (operation: {})", op));
        }
        VerumError::new(message, ErrorKind::Capability)
    }
}

/// Sandbox escape attempt
///
/// Indicates code attempted to escape its sandbox.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Sandbox violation: {message}")]
pub struct SandboxError {
    /// Error message
    pub message: Text,
    /// Attempted operation
    pub operation: Option<Text>,
}

impl SandboxError {
    /// Create a new sandbox error
    pub fn new(message: impl Into<Text>) -> Self {
        Self {
            message: message.into(),
            operation: None,
        }
    }

    /// Add operation name
    pub fn with_operation(mut self, operation: impl Into<Text>) -> Self {
        self.operation = Some(operation.into());
        self
    }
}

impl From<SandboxError> for VerumError {
    fn from(err: SandboxError) -> Self {
        let mut message = format!("Sandbox violation: {}", err.message);
        if let Some(op) = err.operation {
            message.push_str(&format!(" (operation: {})", op));
        }
        VerumError::new(message, ErrorKind::Sandbox)
    }
}
