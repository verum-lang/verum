#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(unexpected_cfgs)]
// Library interface for verum_cli
// Exposes modules for testing purposes
#![allow(dead_code)]

pub mod cache;
pub mod commands;
// NOTE: Old compiler module removed - now using verum_compiler crate
// pub mod compiler;
pub mod config;
pub mod error;
pub mod feature_overrides;
pub mod script;
pub mod tier;
pub mod cog;
pub mod cog_manager;
pub mod registry;
pub mod templates;
pub mod ui;

// Re-export commonly used types
pub use error::{CliError, Result};

// Re-export core types from verum_common
pub use verum_common::{List, Map, Text};

// Re-export registry types for tests
pub use registry::types::{DependencySpec, CogMetadata, CogSource, TierArtifacts};

// Re-export command modules for backward compatibility with tests
pub use commands::add;
pub use commands::audit;
pub use commands::publish;
pub use commands::remove;
pub use commands::search;
pub use commands::tree;
pub use commands::update;

// Re-export registry modules for backward compatibility with tests
pub use registry::{
    cache_manager, client, enterprise, ipfs, lockfile, mirror, resolver, sat_resolver, security,
    signing,
};

// Re-export command option types for tests
pub use commands::audit::AuditOptions;
pub use commands::publish::PublishOptions;
pub use commands::remove::RemoveOptions;
pub use commands::search::SearchOptions;
pub use commands::tree::TreeOptions;

// Re-export enterprise types for tests
pub use registry::enterprise::{
    EnterpriseClient, EnterpriseConfig, MirrorConfig, SbomFormat, SbomGenerator,
};

// Re-export security types for tests
pub use registry::security::{LicenseIssueType, RiskLevel, SecurityReport, SecurityScanner};

// Re-export toolchain for compiler integration
#[allow(deprecated)]
pub use verum_toolchain::{RuntimeArtifacts, Toolchain, ToolchainManager};
