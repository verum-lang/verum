#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unexpected_cfgs)]
#![allow(unused_imports)]

// Force LLVM static libraries to be available at link time.
// On MSVC, transitive static lib dependencies are resolved in single-pass
// order — this direct reference ensures symbols remain available.
extern crate verum_llvm_sys;

// Main entry point for the Verum language toolchain

use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use std::process;
use verum_common::{List, Text};

mod cache;
mod commands;
mod config;
mod error;
mod feature_overrides;
mod cog;
mod cog_manager;
pub mod registry;
mod repl;
mod templates;
mod ui;

use error::{CliError, Result};

#[derive(Parser)]
#[clap(
    name = "verum",
    version = env!("CARGO_PKG_VERSION"),
    about = "The Verum language toolchain \u{2014} semantic honesty, cost transparency, zero-cost safety",
    after_help = "\
QUICK START:
  verum new my_project --profile application   Create a new project
  verum build                       Build the current project
  verum run                         Build and run the current project
  verum run file.vr                 Run a single file (AOT by default)
  verum run --interp file.vr        Run via interpreter
  verum check file.vr               Type-check without building
  verum playbook                    Launch interactive notebook"
)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,

    #[clap(short, long, global = true)]
    verbose: bool,

    #[clap(short, long, global = true)]
    quiet: bool,

    #[clap(long, global = true, default_value = "auto")]
    color: Text,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Verum project
    New {
        /// Project name (used as directory name and cog name)
        name: String,

        /// Language profile controlling available features
        #[clap(
            short, long,
            value_name = "PROFILE",
            value_parser = ["application", "systems", "research"],
        )]
        profile: Option<String>,

        /// Project template to scaffold
        #[clap(
            short, long,
            default_value = "binary",
            value_parser = ["binary", "library", "web-api", "cli-app"],
        )]
        template: String,

        /// Create a library project (shorthand for --template library)
        #[clap(long)]
        lib: bool,

        /// Version control system to initialize
        #[clap(
            long,
            default_value = "git",
            value_parser = ["git", "none"],
        )]
        vcs: String,

        /// Create project at a custom path instead of ./<name>
        #[clap(long, value_name = "DIR")]
        path: Option<String>,
    },

    /// Initialize a Verum project in the current directory
    Init {
        /// Language profile controlling available features (required)
        #[clap(
            short, long,
            value_name = "PROFILE",
            required = true,
            value_parser = ["application", "systems", "research"],
        )]
        profile: String,

        /// Project template to scaffold
        #[clap(
            short, long,
            default_value = "binary",
            value_parser = ["binary", "library", "web-api", "cli-app"],
        )]
        template: String,

        /// Create a library project (shorthand for --template library)
        #[clap(long)]
        lib: bool,

        /// Overwrite existing verum.toml
        #[clap(long)]
        force: bool,

        /// Override project name (default: current directory name)
        #[clap(long, value_name = "NAME")]
        name: Option<String>,
    },

    /// Build the project (always AOT compilation)
    Build {
        /// Optional path to project directory or .vr file
        #[clap(value_name = "PATH")]
        path: Option<Text>,
        #[clap(long, value_name = "NAME")]
        profile: Option<Text>,
        /// Reference mode: managed (~15ns), checked (0ns), mixed (smart)
        #[clap(
            long,
            value_name = "MODE",
            help = "Reference mode: managed|checked|mixed"
        )]
        refs: Option<Text>,
        /// Verification strategy controlling formal-verification behavior.
        ///
        /// Semantic strategies (backend-agnostic):
        ///   runtime     — runtime assertion only (no formal proof)
        ///   static      — type-level check only
        ///   formal      — balanced default (compiler picks best technique)
        ///   fast        — prefer speed over completeness
        ///   thorough    — maximum completeness (parallel strategies)
        ///   certified   — produce exportable proof certificate
        ///   synthesize  — synthesis problem (generate term from spec)
        ///
        /// Legacy values "none", "proof" are aliases for "runtime" and "formal".
        #[clap(long, value_name = "STRATEGY",
               help = "Verification strategy: runtime|static|formal|fast|thorough|certified|synthesize")]
        verify: Option<Text>,

        /// Print SMT routing statistics after compilation.
        #[clap(long, help = "Show SMT solver routing telemetry after build")]
        smt_stats: bool,
        #[clap(short, long)]
        release: bool,
        #[clap(long)]
        target: Option<Text>,
        #[clap(short, long)]
        jobs: Option<usize>,
        #[clap(long)]
        keep_temps: bool,
        #[clap(long)]
        all_features: bool,
        #[clap(long)]
        no_default_features: bool,
        #[clap(long)]
        features: Option<Text>,
        #[clap(long)]
        timings: bool,

        // Advanced linking options
        /// Enable Link-Time Optimization: thin (fast) or full (slower, better)
        #[clap(long, value_name = "MODE", help = "LTO mode: thin|full")]
        lto: Option<Text>,

        /// Enable static linking (no runtime dependencies)
        #[clap(long, help = "Static linking for portable binary")]
        static_link: bool,

        /// Strip all symbols from output binary
        #[clap(long, help = "Strip symbols for smaller binary")]
        strip: bool,

        /// Strip debug info only (keep function names)
        #[clap(long, help = "Strip debug info only")]
        strip_debug: bool,

        /// Output assembly instead of binary
        #[clap(long, help = "Emit assembly (.s) file")]
        emit_asm: bool,

        /// Output LLVM IR instead of binary
        #[clap(long, help = "Emit LLVM IR (.ll) file")]
        emit_llvm: bool,

        /// Output LLVM bitcode for LTO
        #[clap(long, help = "Emit LLVM bitcode (.bc) file")]
        emit_bc: bool,

        /// Emit type metadata (.vtyp) file for separate compilation
        #[clap(long, help = "Emit type metadata (.vtyp) file")]
        emit_types: bool,

        /// Emit VBC bytecode dump (human-readable disassembly)
        #[clap(long, help = "Emit VBC bytecode dump (.vbc.txt)")]
        emit_vbc: bool,

        // Lint configuration options
        /// Treat all warnings as errors
        #[clap(long, help = "Treat all warnings as errors")]
        deny_warnings: bool,

        /// Treat missing intrinsics as errors (default: warnings)
        #[clap(long, help = "Missing intrinsics become errors")]
        strict_intrinsics: bool,

        /// Set a lint to deny level (e.g., -D missing_intrinsic)
        #[clap(short = 'D', long = "deny", value_name = "LINT")]
        deny_lint: Vec<Text>,

        /// Set a lint to warn level (e.g., -W missing_intrinsic)
        #[clap(short = 'W', long = "warn", value_name = "LINT")]
        warn_lint: Vec<Text>,

        /// Set a lint to allow level (e.g., -A missing_intrinsic)
        #[clap(short = 'A', long = "allow", value_name = "LINT")]
        allow_lint: Vec<Text>,

        /// Set a lint to forbid level (e.g., -F missing_intrinsic)
        #[clap(short = 'F', long = "forbid", value_name = "LINT")]
        forbid_lint: Vec<Text>,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Run a Verum program (interpreter by default, --aot for native)
    Run {
        /// .vr file to run (or project directory)
        #[clap(value_name = "FILE")]
        file: Option<Text>,
        /// Run via interpreter (default, can be omitted)
        #[clap(long, conflicts_with = "aot")]
        interp: bool,
        /// Compile to native and run (LLVM AOT)
        #[clap(long, conflicts_with = "interp")]
        aot: bool,
        #[clap(short, long)]
        release: bool,
        /// Show compilation phase timings
        #[clap(long)]
        timings: bool,
        #[clap(last = true)]
        args: Vec<String>,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Run tests
    Test {
        #[clap(long)]
        filter: Option<Text>,
        #[clap(short, long)]
        release: bool,
        #[clap(long)]
        nocapture: bool,
        #[clap(long)]
        test_threads: Option<usize>,
        /// Enable code coverage instrumentation and report generation
        #[clap(long)]
        coverage: bool,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Run benchmarks
    Bench {
        #[clap(long)]
        filter: Option<Text>,
        #[clap(long)]
        save_baseline: Option<Text>,
        #[clap(long)]
        baseline: Option<Text>,
    },

    /// Check without building (works with projects or single .vr files)
    Check {
        /// Optional path to project directory or .vr file
        #[clap(value_name = "PATH")]
        path: Option<Text>,
        #[clap(long)]
        workspace: bool,
        /// Only parse, don't type check (for VCS parse-pass tests)
        #[clap(long)]
        parse_only: bool,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Format source code
    Fmt {
        #[clap(long)]
        check: bool,
        #[clap(long)]
        verbose: bool,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Run linter
    Lint {
        #[clap(long)]
        fix: bool,
        #[clap(long)]
        deny_warnings: bool,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Generate documentation
    Doc {
        #[clap(long)]
        open: bool,
        #[clap(long)]
        document_private_items: bool,
        #[clap(long)]
        no_deps: bool,
        /// Output format: html, markdown, json (default: html)
        #[clap(long, default_value = "html")]
        format: Text,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Remove build artifacts
    Clean {
        #[clap(long)]
        all: bool,
    },

    /// Inspect, export, or clean crash reports written by the Verum
    /// crash reporter (panics and fatal signals). Reports live under
    /// `~/.verum/crashes/` and are safe to attach to issue reports —
    /// environment variables that look sensitive are redacted.
    #[clap(subcommand)]
    Diagnose(commands::diagnose::DiagnoseCommands),

    /// Watch for changes and rebuild
    Watch {
        #[clap(default_value = "build")]
        command: Text,
        #[clap(long)]
        clear: bool,
    },

    /// Manage dependencies
    #[clap(subcommand)]
    Deps(DepsCommands),

    /// Start interactive REPL (optionally preload a file)
    Repl {
        /// Optional file to preload
        #[clap(long)]
        preload: Option<Text>,
        #[clap(long)]
        skip_verify: bool,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Start Verum Playbook - Interactive notebook environment
    ///
    /// The Playbook provides a Jupyter-like experience in your terminal,
    /// optimized for exploring Verum's capabilities including:
    /// - 9-layer math stack (tensors, autodiff, neural networks)
    /// - CBGR memory safety with configurable tiers
    /// - Async programming with structured concurrency
    /// - Full access to core/ standard library
    Playbook {
        /// Optional .vrbook file to open
        #[clap(value_name = "FILE")]
        file: Option<Text>,

        /// Execution tier: 0=interpreter (safe), 1=AOT (fast)
        #[clap(long, short = 't', value_name = "TIER", default_value = "0")]
        tier: u8,

        /// Enable vim-style keybindings
        #[clap(long)]
        vim: bool,

        /// Preload definitions from a .vr file
        #[clap(long, value_name = "FILE")]
        preload: Option<Text>,

        /// Start with interactive language tutorial
        #[clap(long)]
        tutorial: bool,

        /// Enable performance profiling display
        #[clap(long)]
        profile: bool,

        /// Export to .vr script on exit
        #[clap(long, value_name = "FILE")]
        export: Option<Text>,

        /// Disable ANSI colors
        #[clap(long)]
        no_color: bool,
    },

    /// Playbook format conversion utilities
    #[clap(subcommand, name = "playbook-convert")]
    PlaybookConvert(PlaybookConvertCommands),

    /// Show version information
    Version {
        #[clap(long)]
        verbose: bool,
    },

    /// Cog management
    #[clap(subcommand)]
    Package(PackageCommands),

    /// Profile performance (works with projects or single .vr files)
    Profile {
        /// Optional .vr file to profile
        #[clap(value_name = "FILE")]
        file: Option<Text>,
        #[clap(long)]
        memory: bool,
        #[clap(long)]
        cpu: bool,
        #[clap(long)]
        cache: bool,
        /// Profile compilation pipeline phases
        #[clap(long)]
        compilation: bool,
        /// Profile everything — memory, cpu, cache, compilation — in one run.
        /// Equivalent to `--memory --cpu --cache --compilation` and produces
        /// the unified dashboard described in docs/detailed/25-developer-tooling.md §6.
        #[clap(long, conflicts_with_all = ["memory", "cpu", "cache", "compilation"])]
        all: bool,
        #[clap(long, default_value = "5.0")]
        hot_threshold: f64,
        /// Sampling rate for CBGR profiling, as a percentage (0.0–100.0).
        /// Lower values reduce overhead; 1.0 is a safe default for hot paths.
        #[clap(long, value_name = "PERCENT", default_value = "1.0")]
        sample_rate: f64,
        /// Comma-separated list of function names to restrict profiling to.
        /// When set, only samples from these functions are reported.
        #[clap(long, value_name = "NAMES", value_delimiter = ',')]
        functions: Vec<Text>,
        /// Timing precision: `us` (microseconds, default) or `ns` (RDTSC-based,
        /// more expensive but distinguishes sub-microsecond checks).
        #[clap(long, value_name = "UNIT", default_value = "us")]
        precision: Text,
        #[clap(short, long)]
        output: Option<Text>,
        #[clap(long)]
        suggest: bool,
    },

    /// Formal verification (works with projects or single .vr files)
    Verify {
        /// Optional .vr file to verify
        #[clap(value_name = "FILE")]
        file: Option<Text>,
        #[clap(long, short = 'm', default_value = "proof")]
        mode: Text,
        /// Enable the verification profiler: per-function timings, bottleneck
        /// diagnostics, cache stats, ranked recommendations. Results are
        /// printed to stdout unless `--export` is given.
        #[clap(long)]
        profile: bool,
        /// Enable per-obligation profiling: breaks down each function's
        /// verification time into its individual proof obligations
        /// (preconditions, postconditions, refinement checks, loop
        /// invariants, …) and surfaces the slowest obligations
        /// across the whole run. Implies `--profile`. Output joins the
        /// standard profile report under a "Per-obligation breakdown"
        /// section. See `docs/verification/performance.md §5`.
        #[clap(long)]
        profile_obligation: bool,
        /// Fail the build if total verification time exceeds this budget.
        /// Accepts human-readable durations: `120s`, `2m`, `90`, `1h`.
        #[clap(long, value_name = "DURATION")]
        budget: Option<Text>,
        /// Export the profile report as JSON to the given path (implies `--profile`).
        /// Intended for CI/CD integration and trend tracking.
        #[clap(long, value_name = "PATH")]
        export: Option<PathBuf>,
        /// URL of a distributed verification cache (e.g. `s3://bucket/path`).
        /// Reads/writes proof results so that CI reuses proofs across runs.
        #[clap(long, value_name = "URL")]
        distributed_cache: Option<Text>,
        #[clap(long)]
        show_cost: bool,
        #[clap(long)]
        compare_modes: bool,
        #[clap(long, default_value = "z3")]
        solver: Text,
        /// Named `[verify.profiles.<name>]` profile from `verum.toml`
        /// to apply. Profile fields inherit from the base `[verify]`
        /// block; CLI flags still win over both. Unknown profile name
        /// surfaces as a warning and falls back to the base block
        /// (the downstream merge layer is tolerant). See
        /// `docs/verification/cli-workflow.md §9`.
        #[clap(long, value_name = "NAME")]
        verify_profile: Option<Text>,
        /// Preferred backend for exporting SMT proof traces when the
        /// `Certified` strategy races a portfolio. CVC5's ALETHE proof
        /// format is more stable than Z3's native `(proof …)` format
        /// across releases, so the default is `cvc5` — matches the
        /// recommendation in `docs/verification/proof-export.md §7`.
        /// Only affects proof export; does not change which solver
        /// closes an obligation.
        #[clap(long, value_name = "BACKEND", default_value = "cvc5")]
        smt_proof_preference: Text,
        // Default 120s: generous enough for induction and coinduction
        // proofs on realistic programs; too-short default (30s) was
        // causing spurious timeouts on legitimate verifications.
        #[clap(long, default_value = "120")]
        timeout: u64,
        #[clap(long)]
        cache: bool,
        #[clap(long)]
        interactive: bool,
        /// Launch the interactive-tactic REPL after loading. Unlike
        /// plain `--interactive`, this drops straight into a tactic
        /// console (Ltac2-style): the current goal is printed, the
        /// user enters tactics one at a time, and the prompt updates
        /// with the resulting sub-goals. Useful for proof
        /// debugging. See `docs/verification/tactic-dsl.md §9.2`.
        #[clap(long)]
        interactive_tactic: bool,
        /// Limit verification to functions whose source has changed
        /// since the given git reference. Accepts any `git`-parseable
        /// ref: `HEAD~1`, `HEAD~5`, `main`, `abc123`, … The diff is
        /// computed against the current working tree; only functions
        /// whose body lines fall in the changed range are verified.
        /// Use in CI: `verum verify --diff origin/main` verifies only
        /// what a PR changed. Docs: `docs/verification/cli-workflow.md §11`.
        #[clap(long, value_name = "GIT_REF")]
        diff: Option<Text>,
        #[clap(long)]
        function: Option<Text>,
    },

    /// Static analysis
    Analyze {
        #[clap(long)]
        escape: bool,
        #[clap(long)]
        context: bool,
        #[clap(long)]
        refinement: bool,
        #[clap(long)]
        all: bool,
    },

    /// Explain error codes
    Explain {
        /// Error code to explain (e.g., E0312 or 0312)
        code: Text,
        #[clap(long)]
        no_color: bool,
    },

    /// Display compiler information
    Info {
        #[clap(long)]
        features: bool,
        #[clap(long)]
        llvm: bool,
        #[clap(long)]
        all: bool,
    },

    /// Start Debug Adapter Protocol server for IDE debugging
    Dap {
        /// Transport mode: stdio (default), socket
        #[clap(long, value_name = "TRANSPORT", default_value = "stdio")]
        transport: Text,
        /// Port for socket transport (required when transport=socket)
        #[clap(long, value_name = "PORT")]
        port: Option<u16>,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Start Language Server Protocol server for IDE integration
    Lsp {
        /// Transport mode: stdio (default), socket, pipe
        #[clap(long, value_name = "TRANSPORT", default_value = "stdio")]
        transport: Text,
        /// Port for socket transport (required when transport=socket)
        #[clap(long, value_name = "PORT")]
        port: Option<u16>,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Security audit of dependencies
    ///
    /// Default mode: supply-chain audit (vulns, checksums, signatures).
    ///
    /// With `--framework-axioms`: enumerate the trusted-framework boundary
    /// of the current project — every `@framework(name, "citation")` marker
    /// on an axiom / theorem / lemma is collected, grouped by framework,
    /// and printed as a structured report so external reviewers see the
    /// exact set of Lurie HTT / Schreiber DCCT / Connes / Petz / Arnold /
    /// Baez-Dolan results the proofs rely on.
    Audit {
        /// Show vulnerability details
        #[clap(long)]
        details: bool,
        /// Only check direct dependencies
        #[clap(long)]
        direct_only: bool,
        /// Enumerate the trusted-framework-axiom boundary of this project.
        /// Prints every `@framework(name, "citation")` marker found in
        /// .vr sources, grouped by framework. Exits non-zero if any
        /// malformed `@framework(...)` attribute is found.
        #[clap(long)]
        framework_axioms: bool,

        /// Enumerate the 18 primitive inference rules implemented by
        /// `verum_kernel`. Useful for auditors verifying the kernel's
        /// trust boundary corresponds to its documented rule set.
        #[clap(long)]
        kernel_rules: bool,

        /// Output format for the audit report: `plain` (default, human-
        /// readable) or `json` (machine-parseable, stable schema).
        ///
        /// The `json` format is suitable for CI dashboards and
        /// supply-chain enforcement — e.g. fail the build if a PR
        /// introduces a new framework-axiom dependency.
        #[clap(long, value_name = "FORMAT", default_value = "plain")]
        format: String,
    },

    /// Export the project's theorems / lemmas / axioms to an external
    /// proof assistant's certificate format.
    ///
    /// Walks every .vr file in the project, collects every top-level
    /// axiom / theorem / lemma / corollary declaration, and emits a
    /// per-format file containing statement-only entries (proofs are
    /// admitted). `@framework(name, "citation")` markers ride along
    /// so the trusted boundary is visible in the exported artefact.
    ///
    /// Full proof-term export through verum_kernel is a follow-up
    /// — it requires SMT proof-replay, which lands per-backend.
    Export {
        /// Target format: `dedukti`, `coq`, or `lean`.
        #[clap(long, value_name = "FORMAT")]
        to: String,
        /// Output file path (defaults to
        /// `certificates/<format>/export.<ext>`).
        #[clap(long, short, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
    },

    /// Display dependency tree
    Tree {
        /// Show duplicate dependencies
        #[clap(long)]
        duplicates: bool,
        /// Maximum depth to display
        #[clap(long)]
        depth: Option<usize>,
    },

    /// Manage workspace
    #[clap(subcommand)]
    Workspace(WorkspaceCommands),

    // NOTE: stdlib command removed - stdlib is now compiled automatically via cache system.
    // Use `verum info` with --stdlib flag for stdlib information if needed.

    /// Generate shell completion scripts for bash, zsh, fish, or PowerShell.
    ///
    /// Usage: `verum completions bash > ~/.bash_completion.d/verum`
    Completions {
        /// Shell to generate completions for.
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Show the resolved language-feature set for the current project.
    ///
    /// Loads `verum.toml`, applies any CLI overrides (`--tier`, `-Z …`),
    /// runs the feature validator, and prints the final effective
    /// configuration. Useful for debugging flag interactions.
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
    },

    /// Show formal-verification engine capabilities and backends.
    ///
    /// This command diagnoses the toolchain itself: which verification
    /// techniques are available, which advanced features (interpolation,
    /// synthesis, abduction, …) the current build supports. It does not
    /// touch user code.
    #[clap(name = "smt-info")]
    SmtInfo {
        /// Output as machine-readable JSON instead of human-readable text.
        #[clap(long)]
        json: bool,
    },

    /// Show routing statistics from the most recent verification session.
    ///
    /// Reads telemetry: which strategy ran for each theory, portfolio race
    /// winners, cross-validation agreement rate, divergence events, and
    /// per-theory success rates.
    #[clap(name = "smt-stats")]
    SmtStats {
        /// Output as JSON instead of formatted report.
        #[clap(long)]
        json: bool,
        /// Reset statistics after printing.
        #[clap(long)]
        reset: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Print the resolved feature set (human-readable or JSON).
    Show {
        /// Emit machine-readable JSON.
        #[clap(long)]
        json: bool,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Validate verum.toml without building — check for invalid values,
    /// inconsistent combinations, and typos.
    Validate {
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },
}

#[derive(Subcommand)]
enum DepsCommands {
    Add {
        name: Text,
        #[clap(long)]
        version: Option<Text>,
        #[clap(long)]
        dev: bool,
        #[clap(long)]
        build: bool,
    },
    Remove {
        name: Text,
        #[clap(long)]
        dev: bool,
        #[clap(long)]
        build: bool,
    },
    Update {
        package: Option<Text>,
    },
    List {
        #[clap(long)]
        tree: bool,
    },
}

#[derive(Subcommand)]
enum PackageCommands {
    Publish {
        #[clap(long)]
        dry_run: bool,
        #[clap(long)]
        allow_dirty: bool,
    },
    Search {
        query: Text,
        #[clap(long, default_value = "10")]
        limit: usize,
    },
    Install {
        name: Text,
        #[clap(long)]
        version: Option<Text>,
    },
}

/// Playbook conversion utilities
#[derive(Subcommand)]
enum PlaybookConvertCommands {
    /// Export playbook to Verum script (.vr)
    #[clap(name = "to-script")]
    ToScript {
        /// Input .vrbook file
        #[clap(value_name = "INPUT")]
        input: Text,
        /// Output .vr file (defaults to same name with .vr extension)
        #[clap(short, long, value_name = "OUTPUT")]
        output: Option<Text>,
        /// Include output comments in exported script
        #[clap(long)]
        include_outputs: bool,
    },

    /// Import Verum script into playbook format
    #[clap(name = "from-script")]
    FromScript {
        /// Input .vr file
        #[clap(value_name = "INPUT")]
        input: Text,
        /// Output .vrbook file (defaults to same name with .vrbook extension)
        #[clap(short, long, value_name = "OUTPUT")]
        output: Option<Text>,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    /// List workspace members
    List,
    /// Add a new member to workspace
    Add {
        /// Path to the new member
        path: Text,
    },
    /// Remove a member from workspace
    Remove {
        /// Name of the member to remove
        name: Text,
    },
    /// Run command in all workspace members
    Exec {
        /// Command to execute
        #[clap(last = true)]
        command: Vec<String>,
    },
}

fn main() {
    // Industrial crash reporter: panic hook + fatal-signal handlers +
    // breadcrumb-enriched structured reports to `~/.verum/crashes/`.
    verum_error::crash::install(verum_error::crash::CrashReporterConfig::default());

    // Eagerly initialise the LLVM native target on the main thread,
    // before any rayon worker or Z3 context exists.
    //
    // WHY: phase_generate_native was hitting a ~70% SIGSEGV on arm64
    // macOS. The crash always landed in LLVM pass-registry init
    // (TargetLibraryInfoWrapperPass / CFIFixup / CallBase) under
    // __cxa_guard_acquire → __os_semaphore_wait. The cxa guards behind
    // LLVM's first-use pass-constructor registration are not robust
    // against other threads' TLS teardown running in parallel. By
    // registering the native target here — ~zero work, one call, no
    // rayon workers alive yet — the guards are fully released before
    // any stdlib parse spawns rayon workers or verify spawns Z3.
    //
    // The underlying `Target::initialize_native` is idempotent via an
    // internal `Once`; the later call inside `VbcToLlvmLowering::new`
    // becomes a no-op.
    //
    // Diagnosed by running `./target/release/verum build
    // ./examples/cbgr_demo.vr` 20 times: 14/20 segfaults, all in
    // phase=compiler.phase.generate_native at 307–350ms, always on
    // verum-main, all stacks top-heavy with LLVM pass constructors.
    let _ = verum_llvm::targets::Target::initialize_native(
        &verum_llvm::targets::InitializationConfig::default(),
    );

    // Windows default stack is 1 MB — insufficient for deep recursive
    // compiler data structures. Spawn on a thread with 16 MB stack.
    const STACK_SIZE: usize = 16 * 1024 * 1024;
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder.spawn(main_inner).expect("failed to spawn main thread");
    if let Err(e) = handler.join() {
        std::panic::resume_unwind(e);
    }
}

fn main_inner() {
    let cli = Cli::parse();

    if let Err(e) = ui::init(cli.verbose, cli.quiet, cli.color.as_str()) {
        eprintln!("{} {}", "Error:".red().bold(), e);
        process::exit(1);
    }

    // Set VERUM_VERBOSE environment variable based on CLI flags
    // 0 = quiet, 1 = normal (default), 2 = verbose (debug output enabled)
    let verbose_level = if cli.quiet {
        0
    } else if cli.verbose {
        2
    } else {
        1
    };
    // SAFETY: Setting environment variable at program startup before any threads are spawned
    unsafe {
        std::env::set_var("VERUM_VERBOSE", verbose_level.to_string());
    }

    let verbose = cli.verbose;

    // Run main command in a thread with large stack size (64MB) to prevent
    // stack overflow on deeply nested expressions during type checking.
    // Deep expression nesting in select/if/match and files with many nested
    // imports (e.g., 400+ intrinsics) require a larger stack than the default.
    const STACK_SIZE: usize = 256 * 1024 * 1024; // 256MB - needed for deep type checking with full stdlib

    let result = std::thread::Builder::new()
        .name("verum-main".into())
        .stack_size(STACK_SIZE)
        .spawn(move || run_command(cli))
        .expect("Failed to spawn main thread")
        .join()
        .expect("Main thread panicked");

    match result {
        Ok(()) => {}
        Err(e) => {
            ui::error(&format!("{}", e));
            if verbose {
                eprintln!("\n{}: {:?}", "Debug info".yellow(), e);
            }
            process::exit(e.exit_code());
        }
    }
}

/// Resolved target from a user-supplied path argument.
enum PathTarget {
    /// A Verum project directory (cwd has been changed to it).
    Project,
    /// A single source file (.vr).
    SingleFile(Text),
}

/// Resolve an optional path argument into either a project directory or a single file.
///
/// For directories: validates that Verum.toml exists and changes the working directory.
/// For files: returns the path for single-file commands.
/// When no path is given, assumes the current directory is already a project.
fn resolve_path(path: Option<&Text>) -> Result<PathTarget> {
    let project_path = match path {
        Some(p) => p,
        None => return Ok(PathTarget::Project),
    };

    let fs_path = std::path::Path::new(project_path.as_str());

    if fs_path.is_dir() {
        let manifest = fs_path.join("Verum.toml");
        if manifest.exists() {
            std::env::set_current_dir(fs_path).map_err(|e| {
                CliError::Custom(format!("Failed to change to project directory: {}", e))
            })?;
            Ok(PathTarget::Project)
        } else {
            Err(CliError::FileNotFound(format!(
                "{}: Not a Verum project (no Verum.toml found)",
                project_path
            )))
        }
    } else if fs_path.exists() {
        Ok(PathTarget::SingleFile(project_path.clone()))
    } else {
        Err(CliError::FileNotFound(project_path.to_string()))
    }
}

fn run_command(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::New {
            name,
            profile,
            template,
            lib,
            vcs,
            path,
        } => {
            let final_template: &str = if lib { "library" } else { template.as_str() };
            let git = vcs == "git";
            commands::new::execute(
                name.as_str(),
                profile.as_ref().map(|t| t.as_str()),
                final_template,
                git,
                path.as_ref().map(|p| p.as_str()),
            )
        }
        Commands::Init {
            profile,
            template,
            lib,
            force,
            name,
        } => {
            let final_template: &str = if lib { "library" } else { template.as_str() };
            commands::init::execute(
                profile.as_str(),
                final_template,
                force,
                name.as_ref().map(|n| n.as_str()),
            )
        }
        Commands::Build {
            path,
            profile,
            refs,
            verify,
            smt_stats,
            release,
            target,
            jobs,
            keep_temps,
            all_features,
            no_default_features,
            features,
            timings,
            lto,
            static_link,
            strip,
            strip_debug,
            emit_asm,
            emit_llvm,
            emit_bc,
            emit_types,
            emit_vbc,
            deny_warnings,
            strict_intrinsics,
            deny_lint,
            warn_lint,
            allow_lint,
            forbid_lint,
            feature_overrides,
        } => {
            let _smt_stats = smt_stats; // Will be plumbed into session options
            feature_overrides::install(feature_overrides);
            verum_error::crash::set_command("build");
            match resolve_path(path.as_ref())? {
                PathTarget::SingleFile(file_path) => {
                    verum_error::crash::set_input_file(file_path.as_str());
                    ui::status("Building", file_path.as_str());
                    return commands::file::build(
                        file_path.as_str(),
                        None,
                        if release { 3 } else { 2 },
                        "auto",
                        30,
                        false,
                        emit_vbc,
                    );
                }
                PathTarget::Project => {}
            }
            commands::build::execute(
                profile,
                refs,
                verify,
                release,
                target,
                jobs,
                keep_temps,
                all_features,
                no_default_features,
                features,
                timings,
                // Advanced linking options
                lto,
                static_link,
                strip,
                strip_debug,
                emit_asm,
                emit_llvm,
                emit_bc,
                emit_types,
                emit_vbc,
                // Lint options
                deny_warnings,
                strict_intrinsics,
                deny_lint,
                warn_lint,
                allow_lint,
                forbid_lint,
                smt_stats,
            )
        }
        Commands::Run {
            file,
            interp,
            aot,
            release,
            timings,
            args,
            feature_overrides,
        } => {
            // Tier resolution precedence:
            //   1. `--interp` / `--aot` shortcuts on the Run command
            //   2. `--tier` from LanguageFeatureOverrides
            //      (accepts interpret|aot|check; "check" is invalid
            //      for `run` and yields an error)
            //   3. default: interpreter
            let tier_from_override = feature_overrides.tier.as_ref()
                .map(|t| t.as_str().to_string());
            feature_overrides::install(feature_overrides);

            let tier_num: Option<u8> = if interp {
                Some(0)
            } else if aot {
                Some(1)
            } else {
                match tier_from_override.as_deref() {
                    Some("interpret") | Some("interpreter") => Some(0),
                    Some("aot") => Some(1),
                    Some("check") => {
                        return Err(CliError::InvalidArgument(
                            "--tier check is for `verum check`, not `verum run`".into(),
                        ));
                    }
                    Some(other) => {
                        return Err(CliError::InvalidArgument(format!(
                            "unknown tier `{}` (expected interpret|aot)",
                            other
                        )));
                    }
                    None => Some(0), // default = interpreter
                }
            };

            let args_list: List<Text> = args.into_iter().map(|s| s.into()).collect();
            let tier_label = if tier_num == Some(1) { "aot" } else { "interpreter" };

            verum_error::crash::set_command("run");
            verum_error::crash::set_tier(tier_label);
            match resolve_path(file.as_ref())? {
                PathTarget::SingleFile(file_path) => {
                    verum_error::crash::set_input_file(file_path.as_str());
                    ui::status("Running", &format!("{} ({})", file_path, tier_label));
                    commands::file::run_with_tier(file_path.as_str(), args_list, false, tier_num, timings)
                }
                PathTarget::Project => {
                    commands::run::execute(tier_num, None, release, None, None, args_list)
                }
            }
        }
        Commands::Test {
            filter,
            release,
            nocapture,
            test_threads,
            coverage,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            commands::test::execute(filter, release, nocapture, test_threads, coverage, None)
        }
        Commands::Bench {
            filter,
            save_baseline,
            baseline,
        } => commands::bench::execute(filter, save_baseline, baseline, false, false),
        Commands::Check {
            path,
            workspace,
            parse_only,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            match resolve_path(path.as_ref())? {
                PathTarget::SingleFile(file_path) => {
                    ui::status("Checking", file_path.as_str());
                    commands::file::check(file_path.as_str(), false, parse_only)
                }
                PathTarget::Project => {
                    commands::check::execute(workspace, false, false)
                }
            }
        }
        Commands::Fmt { check, verbose, feature_overrides } => {
            feature_overrides::install(feature_overrides);
            commands::fmt::execute(check, verbose)
        }
        Commands::Lint { fix, deny_warnings, feature_overrides } => {
            feature_overrides::install(feature_overrides);
            commands::lint::execute(fix, deny_warnings)
        }
        Commands::Doc {
            open,
            document_private_items,
            no_deps,
            format,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            commands::doc::execute(open, document_private_items, no_deps, format.as_str())
        }
        Commands::Clean { all } => commands::clean::execute(all),
        Commands::Diagnose(cmd) => commands::diagnose::execute(cmd),
        Commands::Watch { command, clear } => commands::watch::execute(command.as_str(), clear),
        Commands::Deps(deps_cmd) => match deps_cmd {
            DepsCommands::Add {
                name,
                version,
                dev,
                build,
            } => commands::deps::add(name.as_str(), version, dev, build),
            DepsCommands::Remove { name, dev, build } => {
                commands::deps::remove(name.as_str(), dev, build)
            }
            DepsCommands::Update { package } => commands::deps::update(package),
            DepsCommands::List { tree } => commands::deps::list(tree),
        },
        Commands::Repl {
            preload,
            skip_verify,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            commands::file::repl(preload.as_ref().map(|s| s.as_str()), skip_verify)
        }
        Commands::Playbook {
            file,
            tier,
            vim,
            preload,
            tutorial,
            profile,
            export,
            no_color,
        } => {
            commands::playbook::execute(commands::playbook::PlaybookOptions {
                file: file.as_ref().map(|s| s.as_str()),
                tier,
                vim_mode: vim,
                preload: preload.as_ref().map(|s| s.as_str()),
                tutorial,
                profile,
                export: export.as_ref().map(|s| s.as_str()),
                no_color,
            })
        }
        Commands::PlaybookConvert(convert_cmd) => match convert_cmd {
            PlaybookConvertCommands::ToScript {
                input,
                output,
                include_outputs,
            } => {
                commands::playbook::export_to_script(
                    input.as_str(),
                    output.as_ref().map(|s| s.as_str()),
                    include_outputs,
                )
            }
            PlaybookConvertCommands::FromScript { input, output } => {
                commands::playbook::import_from_script(
                    input.as_str(),
                    output.as_ref().map(|s| s.as_str()),
                )
            }
        }
        Commands::Version { verbose } => commands::version::execute(verbose),
        Commands::Package(pkg_cmd) => match pkg_cmd {
            PackageCommands::Publish {
                dry_run,
                allow_dirty,
            } => cog::publish(dry_run, allow_dirty),
            PackageCommands::Search { query, limit } => cog::search(query.as_str(), limit),
            PackageCommands::Install { name, version } => cog::install(name.as_str(), version),
        },
        Commands::Profile {
            file,
            memory,
            cpu,
            cache,
            compilation,
            all,
            hot_threshold,
            sample_rate,
            functions,
            precision,
            output,
            suggest,
        } => {
            // Validate sampling knobs at the CLI boundary so the rest of the
            // profiler can trust its inputs.
            if !(0.0..=100.0).contains(&sample_rate) {
                eprintln!(
                    "{} --sample-rate must be in [0, 100], got {}",
                    "error:".red().bold(),
                    sample_rate
                );
                process::exit(2);
            }
            let precision_kind = match precision.as_str() {
                "us" | "micro" | "microseconds" => commands::profile::PrecisionKind::Microseconds,
                "ns" | "nano" | "nanoseconds" => commands::profile::PrecisionKind::Nanoseconds,
                other => {
                    eprintln!(
                        "{} unknown --precision '{}' (use `us` or `ns`)",
                        "error:".red().bold(),
                        other
                    );
                    process::exit(2);
                }
            };
            let function_filter: Vec<String> = functions
                .iter()
                .map(|t| t.as_str().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            // `--all` expands to every slice — spec §6 unified dashboard.
            let (memory, cpu, cache, compilation) = if all {
                (true, true, true, true)
            } else {
                (memory, cpu, cache, compilation)
            };

            let sampling = commands::profile::SamplingConfig {
                sample_rate_percent: sample_rate,
                function_filter,
                precision: precision_kind,
            };

            if let Some(file_path) = file {
                // Profile single file
                commands::file::profile(
                    file_path.as_str(),
                    memory,
                    hot_threshold,
                    output.as_ref().map(|s| s.as_str()),
                    suggest,
                )
            } else {
                // Profile project
                let output_str = output.as_ref().map(|s| s.as_str()).unwrap_or("text");
                commands::profile::execute_with_sampling(
                    memory,
                    cpu,
                    cache,
                    compilation,
                    output_str,
                    sampling,
                )
            }
        }
        Commands::Verify {
            file,
            mode,
            profile,
            profile_obligation,
            budget,
            export,
            distributed_cache,
            show_cost,
            compare_modes,
            solver,
            verify_profile,
            smt_proof_preference,
            timeout,
            cache,
            interactive,
            interactive_tactic,
            diff,
            function,
        } => {
            // `--export` implies `--profile` — you can't dump a profile you
            // didn't collect. `--profile-obligation` also implies `--profile`
            // (per-obligation breakdown is rendered as a detail view under
            // the main profile report). Normalise so downstream sees a
            // single `profile` flag plus a granularity hint.
            let profile = profile || export.is_some() || profile_obligation;

            // Validate --smt-proof-preference (cvc5 | z3). Down-stream
            // passes the value to the Certified strategy's export path;
            // when unrecognised, fail fast rather than silently picking
            // an arbitrary default.
            match smt_proof_preference.as_str() {
                "cvc5" | "z3" => {}
                other => {
                    return Err(CliError::InvalidArgument(
                        format!(
                            "--smt-proof-preference must be 'cvc5' or 'z3', got '{}'",
                            other
                        )
                        .into(),
                    ));
                }
            }
            // The preference flag is consumed by the Certified-strategy
            // export path, not the solver selection path. For now,
            // record it in telemetry; the export wiring (task #65)
            // will read it from the session config when it lands.
            tracing::debug!(
                "smt_proof_preference = {}",
                smt_proof_preference.as_str()
            );
            let _ = smt_proof_preference;

            let budget_duration = match budget.as_deref() {
                None => None,
                Some(raw) => match commands::verify::parse_duration(raw) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        eprintln!(
                            "{} invalid --budget: {}",
                            "error:".red().bold(),
                            e
                        );
                        process::exit(2);
                    }
                },
            };

            if let Some(file_path) = file {
                // Single-file mode uses the non-project path; the profile /
                // budget / export hooks live in the project executor below.
                if profile || budget_duration.is_some() || export.is_some() {
                    eprintln!(
                        "{} --profile / --budget / --export are supported \
                         only when verifying a whole project (omit FILE).",
                        "warning:".yellow().bold()
                    );
                }
                let _ = distributed_cache;
                commands::file::verify(
                    file_path.as_str(),
                    mode.as_str(),
                    show_cost,
                    timeout,
                    solver.as_str(),
                    function.as_ref().map(|s| s.as_str()),
                )
            } else {
                let profile_cfg = commands::verify::ProfileConfig {
                    enabled: profile,
                    budget: budget_duration,
                    export_path: export,
                    distributed_cache: distributed_cache.map(|t| t.to_string()),
                    profile_name: verify_profile.map(|t| t.to_string()),
                    profile_obligation,
                };
                // Verify project
                commands::verify::execute(
                    profile_cfg,
                    show_cost,
                    compare_modes,
                    mode.as_str(),
                    solver.as_str(),
                    timeout,
                    cache,
                    interactive || interactive_tactic,
                    diff.as_ref().map(|s| s.as_str().to_string()),
                )
            }
        }
        Commands::Analyze {
            escape,
            context,
            refinement,
            all,
        } => commands::analyze::execute(escape, context, refinement, all),
        Commands::Explain { code, no_color } => commands::explain::execute(code.as_str(), no_color),
        Commands::Info {
            features,
            llvm,
            all,
        } => commands::file::info(features, llvm, all),
        Commands::Dap { transport, port, feature_overrides } => {
            feature_overrides::install(feature_overrides);

            // Gate on [debug].dap_enabled. Resolve from the project
            // manifest if present; otherwise defaults apply (enabled).
            // A missing manifest is not an error — `verum dap` can run
            // outside a Verum project (e.g. for stand-alone IDE use).
            let (dap_enabled, default_port) =
                match config::Manifest::find_manifest_dir().ok() {
                    Some(dir) => {
                        let path = config::Manifest::manifest_path(&dir);
                        let mut m = config::Manifest::from_file(&path)
                            .unwrap_or_else(|_| {
                                config::create_default_manifest(
                                    "scratch",
                                    false,
                                    config::LanguageProfile::Application,
                                )
                            });
                        feature_overrides::apply_global(&mut m)?;
                        (m.debug.dap_enabled, m.debug.port)
                    }
                    None => (true, 0),
                };

            if !dap_enabled {
                return Err(CliError::Custom(
                    "DAP server is disabled by `[debug] dap_enabled = false` \
                     in verum.toml. Set `dap_enabled = true` or override \
                     with `-Z debug.dap_enabled=true`."
                        .into(),
                ));
            }

            let transport_mode = match transport.as_str() {
                "stdio" => commands::dap::Transport::Stdio,
                "socket" => {
                    // Precedence: --port > [debug].port > error if both 0.
                    let resolved = port.unwrap_or(default_port);
                    if resolved == 0 {
                        return Err(CliError::InvalidArgument(
                            "--port is required for socket transport \
                             (or set `[debug] port = NNNN` in verum.toml)"
                                .into(),
                        ));
                    }
                    commands::dap::Transport::Socket(resolved)
                }
                _ => {
                    return Err(CliError::InvalidArgument(
                        "transport must be: stdio or socket".into(),
                    ));
                }
            };
            commands::dap::execute(transport_mode)
        }
        Commands::Lsp { transport, port, feature_overrides } => {
            feature_overrides::install(feature_overrides);
            let transport_mode = match transport.as_str() {
                "stdio" => commands::lsp::Transport::Stdio,
                "socket" => match port {
                    Some(p) => commands::lsp::Transport::Socket(p),
                    None => {
                        return Err(CliError::InvalidArgument(
                            "--port required for socket transport".into(),
                        ));
                    }
                },
                "pipe" => commands::lsp::Transport::Pipe,
                _ => {
                    return Err(CliError::InvalidArgument(
                        "transport must be: stdio, socket, or pipe".into(),
                    ));
                }
            };
            commands::lsp::execute(transport_mode)
        }
        Commands::Audit {
            details,
            direct_only,
            framework_axioms,
            kernel_rules,
            format,
        } => {
            let output_format = match format.as_str() {
                "plain" => commands::audit::AuditFormat::Plain,
                "json" => commands::audit::AuditFormat::Json,
                other => {
                    return Err(CliError::InvalidArgument(
                        format!(
                            "--format must be 'plain' or 'json', got '{}'",
                            other
                        )
                        .into(),
                    ));
                }
            };
            if kernel_rules {
                commands::audit::audit_kernel_rules(output_format)
            } else if framework_axioms {
                commands::audit::audit_framework_axioms_with_format(output_format)
            } else {
                let options = commands::audit::AuditOptions {
                    verify_checksums: true,
                    verify_signatures: details,
                    verify_proofs: false,
                    cbgr_profiles: false,
                    fix: false,
                    direct_only,
                };
                commands::audit::audit(options)
            }
        }
        Commands::Export { to, output } => {
            let format = commands::export::ExportFormat::parse(&to)?;
            let options = commands::export::ExportOptions {
                format,
                output: match output {
                    Some(p) => verum_common::Maybe::Some(p),
                    None => verum_common::Maybe::None,
                },
            };
            commands::export::run(options)
        }
        Commands::Tree { duplicates, depth } => {
            let options = commands::tree::TreeOptions {
                duplicates,
                depth,
                all_features: false,
            };
            commands::tree::tree(options)
        }
        Commands::Workspace(ws_cmd) => match ws_cmd {
            WorkspaceCommands::List => commands::workspace::list(),
            WorkspaceCommands::Add { path } => commands::workspace::add(path),
            WorkspaceCommands::Remove { name } => commands::workspace::remove(name),
            WorkspaceCommands::Exec { command } => commands::workspace::exec(command),
        },

        Commands::SmtInfo { json } => {
            #[cfg(feature = "verification")]
            {
                commands::smt_info::execute(json)
                    .map_err(|e| CliError::Custom(e.to_string()))
            }
            #[cfg(not(feature = "verification"))]
            {
                let _ = json;
                eprintln!(
                    "{} this build of verum does not include formal-verification support.",
                    "note:".cyan().bold()
                );
                eprintln!(
                    "      rebuild with: {}",
                    "cargo build --features verum_cli/verification".dimmed()
                );
                Ok(())
            }
        }

        Commands::SmtStats { json, reset } => {
            commands::smt_stats::execute(json, reset)
                .map_err(|e| CliError::Custom(e.to_string()))
        }

        Commands::Config { command } => match command {
            ConfigCommands::Show { json, feature_overrides } => {
                feature_overrides::install(feature_overrides);
                commands::config::execute(json)
                    .map_err(|e| CliError::Custom(e.to_string()))
            }
            ConfigCommands::Validate { feature_overrides } => {
                feature_overrides::install(feature_overrides);
                commands::config::validate()
                    .map_err(|e| CliError::Custom(e.to_string()))
            }
        },
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "verum",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        // NOTE: stdlib command removed - stdlib is now compiled automatically via cache system
    }
}
