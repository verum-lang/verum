pub mod analyze;
pub mod dap;
pub mod bench;
pub mod build;
pub mod check;
pub mod clean;
pub mod deps;
pub mod doc;
pub mod explain;
pub mod file;
pub mod fmt;
pub mod init;
pub mod lint;
pub mod lsp;
pub mod new;
pub mod playbook;
pub mod profile;
pub mod repl;
pub mod run;
// NOTE: stdlib command removed - stdlib is now compiled automatically via cache system.
// The stdlib.rs file is kept for reference but not exposed in the CLI.
// pub mod stdlib;
pub mod test;
pub mod verify;
pub mod version;
pub mod watch;
pub mod workspace;

// Cog management commands
pub mod add;
pub mod audit;
pub mod publish;
pub mod remove;
pub mod search;
pub mod tree;
pub mod update;
