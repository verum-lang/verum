pub mod analyze;
pub mod cache;
pub mod dap;
pub mod bench;
pub mod build;
pub mod check;
pub mod clean;
pub mod config;
pub mod deps;
pub mod diagnose;
pub mod doc;
pub mod doctor;
pub mod explain;
pub mod file;
pub mod fmt;
pub mod hooks;
pub mod init;
pub mod lex_mask;
pub mod lint;
pub mod lint_baseline;
pub mod lint_cache;
pub mod lint_engine;
pub mod lint_human;
pub mod lsp;
pub mod new;
pub mod playbook;
pub mod profile;
/// `verum proof-draft` subcommand — surfaces the
/// `verum_verification::proof_drafting::SuggestionEngine` for
/// IDE / REPL / CLI proof-drafting integration.
pub mod proof_draft;
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
pub mod vbc_version;
pub mod verify;
/// `verum verify --ladder` — wires
/// `verum_verification::ladder_dispatch::DefaultLadderDispatcher` into
/// the CLI verify command path so per-theorem `@verify(strategy)`
/// annotations are routed through the typed ladder dispatcher.
pub mod verify_ladder;
pub mod version;
pub mod watch;
pub mod workspace;

// Cog management commands
pub mod add;
pub mod audit;
pub mod export;
// `verum extract <file.vr>` — walk @extract / @extract_witness /
// @extract_contract attrs and emit per-target scaffolds at
// extracted/<name>.{vr,ml,lean,v}.
pub mod extract;
// `verum import --from owl2-fs <file>` — read OWL 2 Functional-Style
// Syntax and emit the corresponding `.vr` source with `@owl2_*` typed
// attributes. Round-trip with `export --to owl2-fs`.
pub mod import;
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
