pub mod analyze;
pub mod dap;
pub mod bench;
pub mod build;
pub mod check;
pub mod clean;
pub mod config;
pub mod deps;
pub mod diagnose;
pub mod doc;
pub mod explain;
pub mod file;
pub mod fmt;
pub mod init;
pub mod lint;
pub mod lint_engine;
pub mod lsp;
pub mod new;
pub mod playbook;
pub mod profile;
pub mod repl;
pub mod run;
// NOTE: stdlib command removed - stdlib is now compiled automatically via cache system.
// The stdlib.rs file is kept for reference but not exposed in the CLI.
// pub mod stdlib;
#[cfg(feature = "verification")]
pub mod smt_info;
pub mod smt_stats;
pub mod property;
pub mod test;
pub mod verify;
pub mod version;
pub mod watch;
pub mod workspace;

// Cog management commands
pub mod add;
pub mod audit;
pub mod export;
// Shared OWL 2 graph + walker — consumed by `audit::audit_owl2_classify_*`
// and `export` (B5 owl2-fs emitter). Single source of truth for the
// Owl2*Attr → Owl2Graph projection.
pub mod owl2;
pub mod publish;
pub mod remove;
pub mod search;
pub mod smt_check;
pub mod tree;
pub mod update;
