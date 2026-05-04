pub mod analyze;
pub mod audit_gate;
pub mod bench;
/// `verum benchmark` subcommand — head-to-head comparison surface
/// (#83). Runs the configured suite against one or more systems
/// (Verum / Coq / Lean4 / Isabelle / Agda) and emits a typed
/// comparison matrix.
pub mod benchmark;
pub mod build;
pub mod cache;
/// `verum cache-closure` subcommand — surfaces
/// `verum_verification::closure_cache::FilesystemCacheStore` so
/// users / IDE / CI can inspect / list / clear / probe the
/// per-theorem closure-hash incremental verification cache.
pub mod cache_closure;
/// `verum cert-replay` subcommand — multi-backend SMT certificate
/// cross-validation. Kernel-only structural check + per-backend
/// replay + multi-backend consensus gate.
pub mod cert_replay;
pub mod check;
pub mod check_proof;
pub mod clean;
/// `verum cog precompile` — Phase 12 of the precompiled-stdlib
/// archive epic. Wires `verum_compiler::precompile::precompile_cog`
/// through the CLI surface to produce a `.vbca` artefact for a
/// single cog directory ready for registry distribution.
pub mod cog_precompile;
/// `verum cog-registry` subcommand — interact with the cog
/// distribution registry: publish / lookup / search / verify /
/// multi-mirror consensus check.
pub mod cog_registry;
pub mod config;
/// `verum cubical` subcommand — typed cubical/HoTT primitive
/// catalogue + computation-rule registry + face-formula validator.
pub mod cubical;
pub mod dap;
pub mod deps;
pub mod diagnose;
pub mod doc;
/// `verum doc-render` subcommand — auto-paper generator. Walks
/// every `.vr` file, projects @theorem / @lemma / @corollary /
/// @axiom to typed `DocItem`s, and renders Markdown / LaTeX / HTML
/// via `verum_verification::doc_render::DefaultDocRenderer`.
pub mod doc_render;
pub mod doctor;
pub mod elaborate_proof;
pub mod explain;
pub mod file;
pub mod fmt;
/// `verum foreign-import` subcommand — read Coq / Lean4 / Mizar /
/// Isabelle source files and emit a Verum `.vr` skeleton with one
/// `@axiom`-bodied declaration per imported theorem, attributed
/// back to the source via `@framework(<system>, "<source>:<line>")`.
pub mod foreign_import;
pub mod hooks;
pub mod init;
pub mod lex_mask;
pub mod lint;
pub mod lint_baseline;
pub mod lint_cache;
pub mod lint_engine;
pub mod lint_human;
/// `verum llm-tactic` subcommand — LCF-style fail-closed LLM proof
/// proposer. The LLM may propose tactic sequences but the kernel
/// re-checks every step before committing.
pub mod llm_tactic;
pub mod lsp;
pub mod new;
pub mod playbook;
pub mod profile;
/// `verum proof-draft` subcommand — surfaces the
/// `verum_verification::proof_drafting::SuggestionEngine` for
/// IDE / REPL / CLI proof-drafting integration.
pub mod proof_draft;
/// `verum proof-repair` subcommand — surfaces the
/// `verum_diagnostics::proof_repair::RepairEngine` so IDE / LSP /
/// REPL consumers can request structured repair suggestions for a
/// typed `ProofFailureKind` without depending on the Rust API.
pub mod proof_repair;
/// `verum proof-repl` subcommand — non-interactive batch driver
/// for the proof REPL state machine. Apply tactics, undo / redo,
/// hint, visualise the proof tree.
pub mod proof_repl;
pub mod repl;
pub mod run;
/// `verum tactic` subcommand — surfaces
/// `verum_verification::tactic_combinator::DefaultTacticCatalog` so
/// IDE / REPL / docs-generator consumers can ask the canonical
/// combinator catalogue what its 15 entries are, what their
/// algebraic laws look like, and what a single combinator's full
/// doc record is.
pub mod tactic;
// NOTE: stdlib command removed - stdlib is now compiled automatically via cache system.
// The stdlib.rs file is kept for reference but not exposed in the CLI.
// pub mod stdlib;
/// `verum stdlib precompile` — Phase 4 of the precompiled-stdlib epic.
/// Drives `CompilationPipeline::compile_core` and writes a `.vbca`
/// archive that the compiler binary embeds at build time (Phase 5).
pub mod stdlib_precompile;
pub mod fuzz;
pub mod property;
#[cfg(feature = "verification")]
pub mod smt_info;
pub mod smt_stats;
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
pub mod bridge_discharge;
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
