#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unexpected_cfgs)]
#![allow(unused_imports)]

// Force LLVM static libraries to be available at link time.
// On MSVC, transitive static lib dependencies are resolved in single-pass
// order — this direct reference ensures symbols remain available.
extern crate verum_llvm_sys;

// Main entry point for the Verum language compiler

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
mod script;
mod tier;
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
    about = "The Verum language compiler \u{2014} semantic honesty, cost transparency, zero-cost safety",
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
    command: Option<Commands>,

    #[clap(short, long, global = true)]
    verbose: bool,

    /// Print the verification-architecture version stamp and exit.
    /// The kernel constant `verum_kernel::VVA_VERSION` is the
    /// single source of truth — bump on every kernel-rule acceptance.
    #[clap(long = "vva-version")]
    vva_version: bool,

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
        /// .vr file to run, project directory, or `-` to read from stdin.
        #[clap(value_name = "FILE")]
        file: Option<Text>,
        /// Evaluate an inline expression. The expression is wrapped in
        /// a `print(...)` so its value is shown; pass `-` instead for
        /// raw stdin without auto-print.
        #[clap(long, short = 'e', value_name = "EXPR", conflicts_with = "file")]
        eval: Option<String>,
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

        /// Script-mode permission overrides. `--allow <scope>`,
        /// `--allow-all`, `--deny-all`. Applied on top of any
        /// `permissions = [...]` declaration in the script's
        /// frontmatter. No-op for non-script invocations.
        #[clap(flatten)]
        permission_flags: crate::script::permission_flags::PermissionFlags,
    },

    /// Run tests
    Test {
        /// Substring match on test name (use `--exact` for equality).
        #[clap(long)]
        filter: Option<Text>,
        #[clap(short, long)]
        release: bool,
        /// Don't capture stdout/stderr of test binaries.
        #[clap(long)]
        nocapture: bool,
        /// Max parallel test workers (default: num CPUs when
        /// [test].parallel = true).
        #[clap(long)]
        test_threads: Option<usize>,
        /// Enable code coverage instrumentation and report generation.
        #[clap(long)]
        coverage: bool,
        /// Run via interpreter (Tier 0, in-process).
        #[clap(long, conflicts_with = "aot")]
        interp: bool,
        /// Compile each test to native and spawn (Tier 1, default).
        #[clap(long, conflicts_with = "interp")]
        aot: bool,
        /// Presentation: pretty | terse | json (libtest convention).
        #[clap(long, value_name = "FMT", default_value = "pretty")]
        format: Text,
        /// Print discovered tests and exit without running them.
        #[clap(long)]
        list: bool,
        /// Run all tests, including @ignore'd.
        #[clap(long, conflicts_with = "ignored")]
        include_ignored: bool,
        /// Run ONLY @ignore'd tests.
        #[clap(long)]
        ignored: bool,
        /// Require filter to match the full test name.
        #[clap(long)]
        exact: bool,
        /// Skip tests whose name contains this pattern (repeatable).
        #[clap(long)]
        skip: Vec<Text>,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Run benchmarks
    Bench {
        /// Substring match on bench name.
        #[clap(long)]
        filter: Option<Text>,
        /// Save current run as a named baseline (target/bench/NAME.json).
        #[clap(long, value_name = "NAME")]
        save_baseline: Option<Text>,
        /// Diff against a previously saved baseline.
        #[clap(long, value_name = "NAME")]
        baseline: Option<Text>,
        /// Run each @bench via interpreter (Tier 0, in-process, no spawn).
        #[clap(long, conflicts_with = "aot")]
        interp: bool,
        /// Compile each @bench to a native driver and spawn (Tier 1, default).
        #[clap(long, conflicts_with = "interp")]
        aot: bool,
        /// Warm-up budget in seconds before timed samples.
        #[clap(long, value_name = "SECS", default_value = "3.0")]
        warm_up_time: f64,
        /// Measurement budget in seconds (terminates after min-samples).
        #[clap(long, value_name = "SECS", default_value = "5.0")]
        measurement_time: f64,
        /// Lower bound on samples per bench (ignored if --sample-size set).
        #[clap(long, value_name = "N", default_value = "10")]
        min_samples: usize,
        /// Upper bound on samples per bench (or fixed count with --sample-size).
        #[clap(long, value_name = "N", default_value = "100")]
        max_samples: usize,
        /// Run exactly this many samples, skipping time-budget logic.
        #[clap(long, value_name = "N")]
        sample_size: Option<usize>,
        /// Percent change below which a diff vs baseline is "noise".
        #[clap(long, value_name = "PCT", default_value = "2.0")]
        noise_threshold: f64,
        /// Output format: table | json | csv | markdown.
        #[clap(long, value_name = "FMT", default_value = "table")]
        format: Text,

        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
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
        /// Read source from stdin and write the formatted output to
        /// stdout. The standard editor format-on-save plumbing
        /// (rustfmt, gofmt, prettier, ruff format -). Mutually
        /// exclusive with `--check`.
        #[clap(long)]
        stdin: bool,
        /// Filename hint for stdin mode — used for diagnostics and
        /// future config resolution. The file at this path is *not*
        /// read.
        #[clap(long, value_name = "PATH")]
        stdin_filename: Option<Text>,
        /// Worker thread count for the parallel file scanner. `0`
        /// = sequential. Default uses `rayon::current_num_threads()`.
        #[clap(long, value_name = "N")]
        threads: Option<usize>,
        /// Behaviour when a file fails to parse:
        ///   - `fallback` (default): silently apply whitespace
        ///     normalisation; warn.
        ///   - `skip`: leave the file untouched; warn.
        ///   - `error`: leave the file untouched; fail the run.
        #[clap(long, value_name = "MODE")]
        on_parse_error: Option<Text>,
        /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Static analysis suite — see [Reference → Lint configuration]
    /// for the full schema in verum.toml.
    Lint {
        /// Apply auto-fixes where available; honours [lint.policy].auto_fix.
        #[clap(long)]
        fix: bool,
        /// Treat warnings as errors (CI gate).
        #[clap(long)]
        deny_warnings: bool,
        /// Print every known built-in lint rule and exit.
        #[clap(long)]
        list_rules: bool,
        /// Print every known lint group (`verum::strict`,
        /// `verum::nursery`, etc.) and the rules they include.
        #[clap(long)]
        list_groups: bool,
        /// Print extended documentation for one rule and exit.
        #[clap(long, value_name = "RULE")]
        explain: Option<Text>,
        /// Open the rule's online documentation page in the system
        /// browser. Requires `--explain RULE`.
        #[clap(long, requires = "explain")]
        open: bool,
        /// Run only config-validator; exits 0 / non-zero. Useful in pre-commit hooks.
        #[clap(long)]
        validate_config: bool,
        /// Output format: pretty (default) | json | github-actions.
        #[clap(long, value_name = "FMT", default_value = "pretty")]
        format: Text,
        /// Apply named profile from `[lint.profiles.<name>]`. Falls
        /// back to `$VERUM_LINT_PROFILE` env var.
        #[clap(long, value_name = "NAME")]
        profile: Option<Text>,
        /// Lint only files changed since the given git ref. Calls
        /// `git diff --name-only <REF>...HEAD -- '*.vr'`.
        #[clap(long, value_name = "GIT_REF")]
        since: Option<Text>,
        /// Report only NEW issues introduced since GIT_REF.
        /// Differs from `--since`: --since lints changed FILES (and
        /// reports every issue in them, including pre-existing).
        /// --new-only-since lints HEAD and REF, then reports issues
        /// present in HEAD but absent from REF. Mutually exclusive
        /// with --since.
        #[clap(long, value_name = "GIT_REF", conflicts_with = "since")]
        new_only_since: Option<Text>,
        /// Filter to issues at this level or higher: error | warn | info | hint.
        #[clap(long, value_name = "LEVEL")]
        severity: Option<Text>,
        /// Worker thread count for the parallel file scanner. `0`
        /// falls back to a sequential single-threaded run (useful
        /// for debugging non-deterministic output). The default is
        /// `rayon::current_num_threads()`, which honours
        /// `RAYON_NUM_THREADS` if set.
        #[clap(long, value_name = "N")]
        threads: Option<usize>,
        /// Bypass the per-file digest cache. Forces every file to
        /// re-lint on this run; useful when you suspect the cache
        /// is stale or you want a clean reproduction of CI output.
        #[clap(long)]
        no_cache: bool,
        /// Wipe the lint cache (`target/lint-cache/`) and exit
        /// without running any rules.
        #[clap(long)]
        clean_cache: bool,
        /// Read suppressions from FILE (default
        /// `.verum/lint-baseline.json` if present). Issues that match
        /// a baseline entry are silenced. Use to adopt strict rules
        /// incrementally on legacy code.
        #[clap(long, value_name = "FILE")]
        baseline: Option<Text>,
        /// Disable baseline lookup for this run, even if the
        /// default `.verum/lint-baseline.json` exists.
        #[clap(long)]
        no_baseline: bool,
        /// Snapshot the current run's issue set to FILE (or to the
        /// default baseline path) and exit 0 even if there are
        /// issues. Use to seed or refresh the baseline.
        #[clap(long)]
        write_baseline: bool,
        /// Fail the run when more than N warnings are emitted (after
        /// severity_map / per-file overrides / --severity / baseline
        /// filtering). `0` is equivalent to `--deny-warnings`. Errors
        /// always fail the run regardless of N.
        #[clap(long, value_name = "N")]
        max_warnings: Option<usize>,
        /// Watch the project for changes and re-lint affected files.
        /// The first run lints everything; subsequent runs use the
        /// per-file cache so untouched files cost ~nothing. Press
        /// Ctrl-C to exit.
        #[clap(long)]
        watch: bool,
        /// When `--watch` is set, clear the screen between runs so
        /// stale output doesn't pile up. No effect without `--watch`.
        #[clap(long)]
        watch_clear: bool,

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

    /// Manage the script-mode VBC cache (`~/.verum/script-cache/`).
    /// Subcommands: `path`, `list`, `clear`, `gc`, `show`. The cache
    /// is content-addressed by source + compiler + flags, so a hit is
    /// always a valid reuse — there is no "stale cache" failure mode.
    /// `gc` evicts least-recently-accessed entries until under a budget;
    /// `clear` removes everything.
    #[clap(subcommand)]
    Cache(commands::cache::CacheCommands),

    /// Run a health-check survey of the Verum installation. Verifies
    /// the home directory is writable, surveys the script cache and
    /// content store, parses any `verum.lock` in the cwd, and probes
    /// the permission-grammar surface. `--json` emits NDJSON for
    /// scripting; `--strict` exits non-zero on warnings as well as
    /// failures.
    Doctor(commands::doctor::DoctorArgs),

    /// Watch for changes and rebuild
    Watch {
        #[clap(default_value = "build")]
        command: Text,
        #[clap(long)]
        clear: bool,
    },

    /// Manage git hooks for the current project. The `install`
    /// subcommand wires `verum lint --since HEAD --severity error`
    /// + `verum fmt --check` into `.git/hooks/pre-commit`. Each
    /// generated hook carries a header marker so `uninstall` only
    /// touches files we wrote.
    #[clap(subcommand)]
    Hooks(HooksCommands),

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

    /// Inspect a VBC archive header (magic, version, sections, hashes).
    /// Tracks #175.
    #[clap(name = "vbc-version")]
    VbcVersion {
        /// Path to the .vbc archive.
        #[clap(value_name = "ARCHIVE")]
        archive: std::path::PathBuf,
        /// Emit a single-line key=value form for scripting.
        #[clap(long)]
        raw: bool,
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
        /// Emit verification diagnostics in Language Server Protocol
        /// format (one JSON `Diagnostic` per line, newline-delimited)
        /// on stdout instead of the human-readable report. Intended
        /// for IDE integrations that pipe `verum verify` through a
        /// JSON-RPC adapter. Implies no human output on stdout;
        /// errors still go to stderr. See
        /// `docs/verification/cli-workflow.md §13`.
        #[clap(long)]
        lsp_mode: bool,
        /// Dump every SMT-LIB query the verifier generates to the
        /// given directory. One file per obligation, named
        /// `<function>-<obligation-idx>.smt2`. Intended for
        /// debugging "why did Z3 time out on this specific goal".
        /// The verifier still runs normally; dumping is a
        /// side-effect. Docs: `docs/verification/cli-workflow.md §14`.
        #[clap(long, value_name = "DIR")]
        dump_smt: Option<std::path::PathBuf>,
        /// Read an SMT-LIB 2 file from disk and dispatch it to the
        /// configured solver; print `sat` / `unsat` / `unknown` on
        /// stdout. Used to replay a dumped query (from
        /// `--dump-smt`) against a different solver / timeout /
        /// backend configuration. Incompatible with `FILE` — when
        /// `--check-smt-formula` is set the positional FILE is
        /// ignored.
        #[clap(long, value_name = "SMT_FILE")]
        check_smt_formula: Option<std::path::PathBuf>,
        /// Log every solver command / response to stderr as it
        /// happens. Useful for diagnosing solver-quirk issues
        /// (e.g. Z3 accepting a command CVC5 rejects). One line
        /// per send/recv, prefixed with `[→]` / `[←]`.
        #[clap(long)]
        solver_protocol: bool,
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
        /// Enable the per-theorem closure-hash incremental
        /// verification cache (#79).  When set, theorem proofs whose
        /// closure hash is in the cache and whose cached verdict was
        /// Ok are skipped without invoking the SMT / kernel re-check.
        /// Cache root defaults to
        /// `<input.parent>/target/.verum_cache/closure-hashes/`;
        /// override with `--closure-cache-root <PATH>`.
        #[clap(long)]
        closure_cache: bool,

        /// Override the closure-cache root directory.  Implies
        /// `--closure-cache` if set.  Standard CI use is to point
        /// this at a shared NFS path so multiple agents reuse cached
        /// verdicts.
        #[clap(long, value_name = "PATH")]
        closure_cache_root: Option<PathBuf>,

        /// Route every `@verify(strategy)` obligation through the
        /// typed 13-strategy ladder dispatcher
        /// (`verum_verification::ladder_dispatch::DefaultLadderDispatcher`)
        /// and emit the per-theorem verdict (Closed / Open /
        /// DispatchPending / Timeout) plus a totals summary. Exits
        /// non-zero on any Open / Timeout verdict (real verification
        /// failure); DispatchPending is advisory because the V0 ladder
        /// only implements the 2 coarsest strategies. Use
        /// `--ladder-format=json` for CI / IDE consumption.
        #[clap(long)]
        ladder: bool,
        /// Output format for `--ladder`: `plain` (default) or `json`.
        #[clap(long, value_name = "FORMAT", default_value = "plain")]
        ladder_format: Text,
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
    /// Interactive proof-drafting helper.  Given a theorem name and
    /// a description of the focused goal, emits ranked next-step
    /// tactic suggestions (lemma applications + tactic invocations +
    /// state navigation) via
    /// `verum_verification::proof_drafting::SuggestionEngine`.
    ///
    /// Output format:
    ///   - `--format plain` (default) — human-readable with rationales.
    ///   - `--format json`            — structured (LSP-friendly).
    ProofDraft {
        /// Theorem name (the proof body's owner — used for diagnostic
        /// labelling and history attribution).
        #[clap(long)]
        theorem: String,

        /// The focused goal's proposition rendering (what needs to be
        /// proved).  Pipe via stdin with `--goal -` for multi-line
        /// goals.
        #[clap(long)]
        goal: String,

        /// Available lemmas in scope as `name:::signature` lines (one
        /// per `--lemma` flag, repeatable).  Or use `--lemmas-from
        /// <file>` to load a `\n`-separated list from a file.
        #[clap(long, value_name = "NAME:::SIGNATURE")]
        lemma: Vec<String>,

        /// Maximum number of suggestions to emit.
        #[clap(long, default_value = "5")]
        max: usize,

        /// Output format: `plain` or `json`.
        #[clap(long, default_value = "plain")]
        format: String,
    },

    /// Structured repair suggestions for a typed proof / kernel
    /// failure.  Wires
    /// `verum_diagnostics::proof_repair::DefaultRepairEngine` so IDE /
    /// LSP / REPL consumers can request ranked drop-in code-snippet
    /// repairs without depending on the Rust API.
    ///
    /// Usage:
    ///   verum proof-repair --kind unbound-name --field name=foo
    ///   verum proof-repair --kind refine-depth \
    ///         --field refined_type=CategoricalLevel \
    ///         --field predicate_depth=ω·2 --max 3 --format json
    ///
    /// Valid `--kind` values: refine-depth, positivity, universe,
    /// fwax-not-prop, adjunction, type-mismatch, unbound-name,
    /// apply-mismatch, tactic-open.
    ProofRepair {
        /// Failure-kind tag — see command help for the full set.
        #[clap(long)]
        kind: String,

        /// Per-kind structured fields as `key=value`.  Repeatable.
        /// Required keys differ per kind; missing required keys
        /// surface as an InvalidArgument error naming the missing key.
        #[clap(long, value_name = "KEY=VALUE")]
        field: Vec<String>,

        /// Maximum number of suggestions to emit.
        #[clap(long, default_value = "5")]
        max: usize,

        /// Output format: `plain` or `json`.
        #[clap(long, default_value = "plain")]
        format: String,
    },

    /// Foreign-system theorem import (#85) — inverse of cross-format
    /// export.  Reads a Coq / Lean4 / Mizar / Isabelle source file
    /// and emits a Verum `.vr` skeleton with one `@axiom`-bodied
    /// declaration per imported theorem, attributed back to the
    /// source via `@framework(<system>, "<source>:<line>")`.  The
    /// user fills in the proof body with Verum tactics, or keeps the
    /// `@axiom` and treats the foreign system as the trust boundary.
    ///
    /// Usage:
    ///   verum foreign-import --from <coq|lean4|mizar|isabelle> <FILE>
    ///                        [--out <PATH>] [--format skeleton|json|summary]
    ForeignImport {
        /// Foreign system: coq / rocq / lean4 / lean / mathlib /
        /// mizar / isabelle / hol.
        #[clap(long)]
        from: String,

        /// Source file to parse.
        #[clap(value_name = "FILE")]
        file: PathBuf,

        /// Write rendered output to this path instead of stdout.
        #[clap(long)]
        out: Option<PathBuf>,

        /// Output format: `skeleton` (default — emit `.vr` source),
        /// `json` (structured payload for tooling), `summary`
        /// (human-readable list).
        #[clap(long, default_value = "skeleton")]
        format: String,
    },

    /// LCF-style fail-closed LLM tactic proposer (#77).  An LLM may
    /// propose tactic sequences but the kernel re-checks every step
    /// before committing.  Any rejection discards the proposal.
    /// Every invocation is captured in an append-only audit trail
    /// keyed by model id + prompt hash + completion hash so the
    /// proof is reproducible from the log.
    LlmTactic {
        #[clap(subcommand)]
        sub: LlmTacticSub,
    },

    /// Auto-paper documentation generator (#84).  Walks every
    /// `.vr` file in the project, projects every public @theorem /
    /// @lemma / @corollary / @axiom into a typed `DocItem`, and
    /// renders Markdown / LaTeX / HTML directly from the corpus.
    /// Eliminates the duplicate-source problem (paper.tex +
    /// verum-corpus): the corpus IS the paper draft.
    ///
    /// Subcommands:
    ///   verum doc-render render [--format md|latex|html] [--out <PATH>] [--public]
    ///   verum doc-render graph [--format dot|json] [--public]
    ///   verum doc-render check-refs [--format plain|json] [--public]
    DocRender {
        #[clap(subcommand)]
        sub: DocRenderSub,
    },

    /// Closure-hash incremental verification cache surface.  Wires
    /// `verum_verification::closure_cache::FilesystemCacheStore` to
    /// the CLI so IDE / CI / users can inspect, list, clear, and
    /// probe the per-theorem cache without depending on the Rust
    /// API.
    ///
    /// Subcommands:
    ///   verum cache-closure stat    [--root <P>] [--format <F>]
    ///   verum cache-closure list    [--root <P>] [--format <F>]
    ///   verum cache-closure get     <theorem> [--root <P>] [--format <F>]
    ///   verum cache-closure clear   [--root <P>] [--format <F>]
    ///   verum cache-closure decide  <theorem> --signature <s> --body <s> \
    ///       [--cite <c>]… [--kernel-version <v>] [--root <P>] [--format <F>]
    CacheClosure {
        #[clap(subcommand)]
        sub: CacheClosureSub,
    },

    /// Industrial-grade tactic combinator catalogue surface.  Wires
    /// `verum_verification::tactic_combinator::DefaultTacticCatalog`
    /// to the CLI so IDE / docs-generator / CI consumers can read
    /// the canonical 15-combinator catalogue + its algebraic laws
    /// without depending on the Rust API.
    ///
    /// Subcommands:
    ///   verum tactic list [--category <C>] [--format <F>]
    ///   verum tactic explain <name> [--format <F>]
    ///   verum tactic laws [--format <F>]
    Tactic {
        #[clap(subcommand)]
        sub: TacticSub,
    },

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

        /// Enumerate the ε-distribution (Actic / DC coordinate) of the
        /// corpus — the dual of `--framework-axioms`. Prints every
        /// `@enact(epsilon = "...")` marker grouped by ε-primitive
        /// . Exits non-zero if any malformed marker is
        /// found (unknown primitive or missing `epsilon = ...` arg).
        #[clap(long)]
        epsilon: bool,

        /// Project the @framework markers to their MSFS coordinate
        /// (Framework, ν, τ). Reads the same source as
        /// `--framework-axioms` and additionally annotates each
        /// framework with its Diakrisis ν-rank and intensional flag.
        ///
        /// the per-theorem coord audit is
        /// **default-on** per `verification-architecture.md` §A.Z.4.
        /// Bare `verum audit` runs dependency-audit + coord-audit
        /// together; pass `--no-coord` to skip the coord pass.
        /// `--coord` (this flag) keeps its legacy meaning of
        /// "coord-only" mode (skips dependency audit).
        #[clap(long)]
        coord: bool,

        /// skip the per-theorem coord audit
        /// that bare `verum audit` runs by default. Honoured only
        /// in the default (no-specific-audit-flag) dispatch path —
        /// other specific audit modes ignore it.
        #[clap(long)]
        no_coord: bool,

        /// Articulation Hygiene audit: walk every type and function
        /// declaration, classify each self-referential surface form
        /// against the hygiene table, and report the (Φ, κ, t)
        /// factorisation for each. Detects inductive, coinductive,
        /// higher-inductive, newtype, @recursive, and @corecursive
        /// surfaces.
        #[clap(long)]
        hygiene: bool,

        /// Articulation Hygiene strict enforcement: walk every
        /// top-level free function body and reject raw `self`
        /// occurrences with `E_HYGIENE_UNFACTORED_SELF`. Methods
        /// (functions with a self-receiver param) are skipped —
        /// `self` is bound there. Exits non-zero on any violation;
        /// safe to wire into CI.
        #[clap(long)]
        hygiene_strict: bool,

        /// OWL 2 classification hierarchy audit: walk every
        /// Owl2*Attr in the project, build the
        /// classification graph (subclass closure + equivalence
        /// partition + disjointness pairs + property characteristics
        /// + has-key constraints), detect cycles and disjoint /
        /// subclass conflicts, and emit a graph-aware report. Exits
        /// non-zero on any cycle or violation.
        #[clap(long)]
        owl2_classify: bool,

        /// Framework-compatibility audit walk every
        /// `@framework(corpus, ...)` marker in the project, collect
        /// the distinct corpus identifiers, and check the well-known
        /// incompatibility matrix (uip ⊥ univalence, anti_classical
        /// ⊥ classical_lem, etc.). Each match prints the conflict
        /// reason + literature citation. Exits non-zero if any
        /// incompatible pair is found — the project's axiom bundle
        /// would derive False, breaking every theorem.
        #[clap(long)]
        framework_conflicts: bool,

        /// accessibility audit (item 4):
        /// walk every `@enact(...)` / EpsilonOf marker in the
        /// project, cross-reference against `@accessibility(λ)`
        /// annotations (per Diakrisis Axi-4 λ-accessibility
        /// premise), and surface any unannotated EpsilonOf site.
        /// Exit non-zero when at least one missing annotation is
        /// found (CI gate). This closes the Axi-4 defect from
        /// by making the framework-author's
        /// accessibility certification a checkable invariant.
        #[clap(long)]
        accessibility: bool,

        /// 108.T round-trip audit: for every theorem citing the
        /// AC/OC duality (Diakrisis 108.T) or its `core.theory_interop.
        /// bridges.oc_dc_bridge` round-trip, classify the canonical
        /// round-trip status as Decidable / SemiDecidable / Undecidable
        /// per the operational coherence layer. Emits one entry per
        /// theorem to `audit-reports/round-trip.json`. Finitely-
        /// axiomatized theorems must be 100% Decidable to clear the
        /// corpus acceptance gate (T5.2).
        #[clap(long)]
        round_trip: bool,

        /// Operational coherence audit: for every `@verify(coherent)`
        /// or `@verify(coherent_static / coherent_runtime)` theorem,
        /// validate the bidirectional α-cert ⟺ ε-cert correspondence
        /// per the Coherent verification rule family. Emits one entry
        /// per theorem to `audit-reports/coherent.json`. Currently
        /// reports a Status::Pending verdict pending the full coherent-
        /// rule kernel implementation (T2.2); the audit surface itself
        /// is stable so CI dashboards can pre-wire it.
        #[clap(long)]
        coherent: bool,

        /// Coord-consistency audit (M4.B): walks every public
        /// theorem / axiom and validates the (Fw, ν, τ) supremum
        /// invariant — every theorem's inferred coordinate must
        /// be ≥ max(cited frameworks' coordinates). Flags
        /// `missing-framework` violations (theorem has `@verify(...)`
        /// but no `@framework(...)` citation). Mirrors V8.1 #232's
        /// kernel-side `check_coord_cite` at corpus-audit time.
        /// Schema_v=1 JSON to `audit-reports/coord-consistency.json`;
        /// non-zero exit on any missing-framework violation.
        #[clap(long)]
        coord_consistency: bool,

        /// Framework-soundness audit (M4.A): walks every
        /// `public axiom` in the project and classifies its
        /// proposition (the parser's requires-AND-ensures
        /// conjunction) as `sound` (has propositional content) or
        /// `trivial-placeholder` (just `true` literal).
        ///
        /// Mirrors the kernel-side K-FwAx
        /// `SubsingletonRegime::ClosedPropositionOnly` gate at
        /// corpus-audit time. Emits to
        /// `audit-reports/framework-soundness.json` (schema_v=1) +
        /// non-zero exit if any axiom is misclassified.
        #[clap(long)]
        framework_soundness: bool,

        /// HTT (Lurie 2009) mechanisation roadmap audit. Emits
        /// per-section coverage table sourced from
        /// `verum_kernel::mechanisation_roadmap::htt_roadmap()`.
        /// Each entry has status Mechanised / Partial / AxiomCited /
        /// Pending plus the kernel module(s) that discharge it.
        #[clap(long)]
        htt_roadmap: bool,

        /// Adámek-Rosický 1994 mechanisation roadmap audit. Emits
        /// per-section coverage table sourced from
        /// `verum_kernel::mechanisation_roadmap::adamek_rosicky_roadmap()`.
        #[clap(long)]
        ar_roadmap: bool,

        /// Kernel self-recognition audit. Decomposes each of the
        /// seven kernel rules (K-Refine, K-Univ, K-Pos, K-Norm,
        /// K-FwAx, K-Adj-Unit, K-Adj-Counit) into its required ZFC
        /// axioms + Grothendieck universes per
        /// `verum_kernel::zfc_self_recognition::required_meta_theory`.
        /// Reports the trusted-base ZFC + κ_n union and exits
        /// non-zero if any rule fails the ZFC + 2-inacc provability
        /// invariant.
        #[clap(long)]
        self_recognition: bool,

        /// Cross-format CI hard gate audit. Lists the four
        /// required export formats (Coq, Lean4, Isabelle, Dedukti)
        /// with their replay commands per
        /// `verum_kernel::cross_format_gate::format_replay_command`.
        #[clap(long)]
        cross_format: bool,

        /// Kernel intrinsic dispatch audit. Lists every kernel_*
        /// dispatcher backing a `kernel_*` axiom in
        /// `core/proof/kernel_bridge.vr`. Used to verify the
        /// Verum-side bridge ↔ Rust-side kernel-function coupling
        /// is complete.
        #[clap(long)]
        kernel_intrinsics: bool,

        /// Kernel-discharged-axioms audit. Walks every
        /// `@kernel_discharge("<intrinsic>")` attribute in the
        /// project, verifies the cited dispatcher name appears in
        /// `verum_kernel::intrinsic_dispatch::available_intrinsics()`.
        /// Exits non-zero on any unmatched citation. Surfaces the
        /// trusted-base-shrinkage cross-link in machine-checkable form.
        #[clap(long)]
        kernel_discharged_axioms: bool,

        /// Verify-ladder audit. Walks every `@verify(strategy)`
        /// annotation, projects to its ν-ordinal, classifies dispatch
        /// status (implemented / fallback / pending), and verifies the
        /// strict-ν-monotonicity invariant.  Exits non-zero on any
        /// monotonicity violation.
        #[clap(long)]
        verify_ladder: bool,

        /// Proof-honesty audit (M0.G): walk every public theorem /
        /// axiom in the project and classify each by proof-body shape
        /// — `axiom-placeholder` / `theorem-no-proof-body` /
        /// `theorem-trivial-true` / `theorem-axiom-only` /
        /// `theorem-multi-step`. Emits per-row classification + by-
        /// lineage totals (msfs / diakrisis subpath partition) to
        /// `audit-reports/proof-honesty.json` (schema_version=1).
        ///
        /// Mirrors the stand-alone Python walker
        /// `verum-msfs-corpus/tools/proof_honesty_audit.py` (M0.E),
        /// now first-class via the verum CLI.
        #[clap(long)]
        proof_honesty: bool,

        /// Bridge-admit footprint audit (M-EXPORT V2 / K-Round-Trip V2):
        /// walk every public theorem / lemma / corollary, lift the
        /// proof body to a CoreTerm, run
        /// `verum_kernel::round_trip::enumerate_bridge_admits`, and
        /// emit a per-theorem footprint of which Diakrisis preprint
        /// admits the proof relies on (16.10 confluence, 16.7
        /// quotient canonical-rep, 14.3 cohesive-adjunction unit/counit).
        ///
        /// Empty footprint = decidable corpus. Non-empty = trusted-
        /// boundary surface: external reviewers see every reliance
        /// on a preprint-blocked result without re-walking the
        /// kernel by hand. Schema_v=1 JSON when --format json.
        #[clap(long)]
        bridge_admits: bool,

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
        /// Target format: `dedukti` | `coq` | `lean` | `metamath`.
        #[clap(long, value_name = "FORMAT")]
        to: String,
        /// Output file path (defaults to
        /// `certificates/<format>/export.<ext>`).
        #[clap(long, short, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
        /// Emit a per-declaration provenance JSON sidecar at
        /// `<output>.provenance.json`. The sidecar lists every
        /// exported declaration with its kind / source-file /
        /// framework citation / discharge_strategy; downstream
        /// tools use it to drive SMT replay or fill in proof terms.
        /// Statement-level export (sorry / Admitted / `?`) is
        /// unchanged when this flag is absent.
        #[clap(long)]
        with_provenance: bool,
    },

    /// Alias for `export --to <format>` — matches the wording in
    /// `docs/verification/proof-export.md` and the CLI reference at
    /// `docs/verification/cli-workflow.md §12`. Behaviour is
    /// identical to `verum export --to FORMAT`; the alias keeps
    /// docs-cli parity without duplicating semantics.
    ExportProofs {
        /// Target format: `dedukti` | `coq` | `lean` | `metamath`.
        #[clap(long, value_name = "FORMAT")]
        to: String,
        /// Output file path (defaults to
        /// `certificates/<format>/export.<ext>`).
        #[clap(long, short, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
        /// See `Export::with_provenance`.
        #[clap(long)]
        with_provenance: bool,
    },

    /// extract executable
    /// programs from constructive proofs marked with `@extract` /
    /// `@extract_witness` / `@extract_contract`. Walks the project
    /// for marked declarations, dispatches to the program-extraction
    /// pipeline at the attribute's `ExtractTarget` (Verum / OCaml /
    /// Lean / Coq), and emits per-target files at
    /// `<output>/<decl>.{vr,ml,lean,v}`. Default output dir is
    /// `extracted/`.
    Extract {
        /// Optional explicit input `.vr` path. When absent, all `.vr`
        /// files under the project's manifest directory are scanned.
        input: Option<std::path::PathBuf>,
        /// Output directory (defaults to `extracted/`).
        #[clap(long, short, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
    },

    /// import an external knowledge-
    /// base format and emit a `.vr` file with the corresponding typed
    /// attributes. Currently supports OWL 2 Functional-Style Syntax
    /// (`--from owl2-fs`); round-trips with `verum export --to owl2-fs`.
    Import {
        /// Source format: `owl2-fs` (also `ofn`).
        #[clap(long, value_name = "FORMAT")]
        from: String,
        /// Input path. Required.
        input: std::path::PathBuf,
        /// Output `.vr` path (defaults to `<input>.vr`).
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
    /// This command diagnoses the verifier itself: which verification
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

/// `verum hooks <subcommand>` — manage git hooks for the project.
#[derive(Subcommand)]
enum LlmTacticSub {
    Propose {
        #[clap(long)]
        theorem: String,
        #[clap(long)]
        goal: String,
        #[clap(long, value_name = "NAME:::SIGNATURE")]
        lemma: Vec<String>,
        #[clap(long, value_name = "NAME:TYPE")]
        hyp: Vec<String>,
        #[clap(long, value_name = "STEP")]
        history: Vec<String>,
        #[clap(long, default_value = "mock")]
        model: String,
        #[clap(long)]
        hint: Option<String>,
        #[clap(long)]
        persist: bool,
        #[clap(long, value_name = "PATH")]
        audit: Option<PathBuf>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    AuditTrail {
        #[clap(long, value_name = "PATH")]
        audit: Option<PathBuf>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    Models {
        #[clap(long, default_value = "plain")]
        format: String,
    },
}

#[derive(Subcommand)]
enum DocRenderSub {
    /// Render the corpus as a single document.
    Render {
        /// Output format: `markdown`, `md`, `latex`, `tex`, `html`.
        #[clap(long, default_value = "markdown")]
        format: String,
        /// Write to this path instead of stdout.
        #[clap(long)]
        out: Option<PathBuf>,
        /// Restrict to public-visibility declarations only.
        #[clap(long)]
        public: bool,
    },
    /// Export the citation graph (citing → cited).
    Graph {
        /// `dot` (Graphviz) or `json` (edge list).
        #[clap(long, default_value = "dot")]
        format: String,
        #[clap(long)]
        public: bool,
    },
    /// Audit broken cross-references; non-zero exit on any dangling
    /// citation.  CI-friendly.
    CheckRefs {
        #[clap(long, default_value = "plain")]
        format: String,
        #[clap(long)]
        public: bool,
    },
}

#[derive(Subcommand)]
enum CacheClosureSub {
    /// Show summary stats: entries, size, hit ratio.
    Stat {
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    /// List every theorem name currently cached.
    List {
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    /// Print a single record (fingerprint + verdict).
    Get {
        theorem: String,
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    /// Remove every cache entry.  Idempotent.
    Clear {
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    /// Probe the cache: report Skip / Recheck for the given fingerprint.
    Decide {
        theorem: String,
        /// Theorem signature payload (hashed by the cache).  Pass any
        /// stable serialisation of the elaborated signature.
        #[clap(long)]
        signature: String,
        /// Proof body payload.
        #[clap(long)]
        body: String,
        /// Repeated `--cite <citation>` for transitive @framework
        /// citations; sorted+deduped before hashing.
        #[clap(long = "cite")]
        cite: Vec<String>,
        /// Override the kernel version reported in the fingerprint
        /// (defaults to the running `verum_kernel::VVA_VERSION`).
        #[clap(long)]
        kernel_version: Option<String>,
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
}

#[derive(Subcommand)]
enum TacticSub {
    /// List every combinator in the canonical catalogue with a
    /// one-line semantics summary.
    List {
        #[clap(long, default_value = "plain")]
        format: String,
        /// Restrict to a single category (identity / composition /
        /// control / focus / forward).
        #[clap(long)]
        category: Option<String>,
    },
    /// Print the full structured doc for a single combinator.
    Explain {
        name: String,
        #[clap(long, default_value = "plain")]
        format: String,
    },
    /// List the canonical algebraic-law inventory.
    Laws {
        #[clap(long, default_value = "plain")]
        format: String,
    },
}

#[derive(Subcommand)]
enum HooksCommands {
    /// Install `.git/hooks/pre-commit` running `verum lint --since
    /// HEAD --severity error` and `verum fmt --check`.
    Install {
        /// Overwrite an existing hook even if it isn't
        /// verum-managed. Without this flag, install refuses to
        /// clobber a hand-authored or third-party hook.
        #[clap(long)]
        force: bool,
    },
    /// Remove the verum-managed pre-commit hook. Refuses to remove
    /// hooks that don't carry our header marker.
    Uninstall,
    /// Report whether the hook is installed and whether we manage it.
    Status,
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
    // Script-mode dispatch: rewrite `verum path/to/file.vr [args…]` into
    // `verum run path/to/file.vr [args…]` BEFORE clap sees the argv. See
    // crate::script for the full rationale and invariants. The rewrite is
    // a no-op for any normal subcommand invocation.
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    // Productivity advisory: catch the `verum file.vr` (no `run`) form
    // when `file.vr` lacks the mandatory shebang and surface a precise,
    // actionable error before clap's generic "unknown subcommand" fires.
    // The Verum execution-mode contract reserves the no-`run` shorthand
    // for shebang scripts; non-script `.vr` files must use `verum run`.
    if let Some(msg) = script::missing_shebang_advisory(&argv) {
        eprintln!("error: {}", msg);
        process::exit(2);
    }
    let argv = script::rewrite_argv_for_script_mode(argv);
    let cli = Cli::parse_from(argv);

    // B14 --vva-version short-circuit. Print the kernel
    // version stamp and exit cleanly without dispatching a
    // subcommand. Tooling integrations (CI, certificate emitters,
    // cross-tool replay matrix) read this single line as their
    // verification-architecture version source of truth.
    if cli.vva_version {
        println!("{}", verum_kernel::VVA_VERSION);
        process::exit(0);
    }

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
        let manifest = config::Manifest::manifest_path(fs_path);
        if manifest.exists() {
            std::env::set_current_dir(fs_path).map_err(|e| {
                CliError::Custom(format!("Failed to change to project directory: {}", e))
            })?;
            Ok(PathTarget::Project)
        } else {
            Err(CliError::FileNotFound(format!(
                "{}: Not a Verum project (no verum.toml found)",
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
    // After --vva-version short-circuit in main_inner, command is
    // required. Anything reaching here without one is a clap mis-
    // configuration; surface it as a user error rather than panic.
    let command = match cli.command {
        Some(c) => c,
        None => {
            return Err(CliError::InvalidArgument(
                "no subcommand given (run with --help or --vva-version)".into(),
            ));
        }
    };
    match command {
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
            eval,
            interp,
            aot,
            release,
            timings,
            args,
            feature_overrides,
            permission_flags,
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

            // Inline-eval and stdin sources synthesise a temporary
            // script file with a shebang prefix so they flow through
            // the same script-mode pipeline as on-disk scripts —
            // identical parser, identical permission model, identical
            // exit-code semantics. The temp file is removed on drop.
            if let Some(expr) = eval {
                let tmp = commands::file::synthesize_script_temp(
                    &format!("print({});\n", expr),
                    "eval",
                )
                .map_err(|e| CliError::Custom(format!("synthesize -e: {e}")))?;
                ui::status("Running", &format!("-e ({})", tier_label));
                let result = commands::file::run_with_tier_and_flags(
                    tmp.path().to_str().expect("temp path is utf-8"),
                    args_list,
                    false,
                    tier_num,
                    timings,
                    permission_flags.clone(),
                );
                drop(tmp);
                return result;
            }
            if file.as_ref().map(|f| f.as_str()) == Some("-") {
                let mut buf = String::new();
                use std::io::Read;
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| CliError::Custom(format!("read stdin: {e}")))?;
                let tmp = commands::file::synthesize_script_temp(&buf, "stdin")
                    .map_err(|e| CliError::Custom(format!("synthesize stdin: {e}")))?;
                ui::status("Running", &format!("- ({})", tier_label));
                let result = commands::file::run_with_tier_and_flags(
                    tmp.path().to_str().expect("temp path is utf-8"),
                    args_list,
                    false,
                    tier_num,
                    timings,
                    permission_flags.clone(),
                );
                drop(tmp);
                return result;
            }

            match resolve_path(file.as_ref())? {
                PathTarget::SingleFile(file_path) => {
                    verum_error::crash::set_input_file(file_path.as_str());
                    ui::status("Running", &format!("{} ({})", file_path, tier_label));
                    commands::file::run_with_tier_and_flags(
                        file_path.as_str(),
                        args_list,
                        false,
                        tier_num,
                        timings,
                        permission_flags,
                    )
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
            interp,
            aot,
            format,
            list,
            include_ignored,
            ignored,
            exact,
            skip,
            feature_overrides,
        } => {
            let tier_override = feature_overrides.tier.clone();
            feature_overrides::install(feature_overrides);
            let resolved =
                tier::resolve(interp, aot, tier_override.as_ref(), tier::Tier::Aot)?;
            let opts = commands::test::TestOptions {
                filter,
                release,
                nocapture,
                test_threads,
                coverage,
                verify: None,
                tier: resolved.tier,
                format: commands::test::TestFormat::parse(format.as_str())?,
                list,
                include_ignored,
                ignored_only: ignored,
                exact,
                skip,
            };
            commands::test::execute(opts)
        }
        Commands::Bench {
            filter,
            save_baseline,
            baseline,
            interp,
            aot,
            warm_up_time,
            measurement_time,
            min_samples,
            max_samples,
            sample_size,
            noise_threshold,
            format,
            feature_overrides,
        } => {
            let tier_override = feature_overrides.tier.clone();
            feature_overrides::install(feature_overrides);
            let resolved =
                tier::resolve(interp, aot, tier_override.as_ref(), tier::Tier::Aot)?;
            let opts = commands::bench::BenchOptions {
                filter,
                save_baseline,
                baseline,
                tier: resolved.tier,
                format: commands::bench::ReportFormat::parse(format.as_str())?,
                warm_up_time: std::time::Duration::from_secs_f64(warm_up_time),
                measurement_time: std::time::Duration::from_secs_f64(measurement_time),
                min_samples,
                max_samples,
                sample_size,
                noise_threshold_pct: noise_threshold,
                no_color: false,
            };
            commands::bench::execute(opts)
        }
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
        Commands::Fmt {
            check,
            verbose,
            stdin,
            stdin_filename,
            threads,
            on_parse_error,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            let parse_policy = match on_parse_error {
                Some(t) => Some(commands::fmt::OnParseError::parse(t.as_str()).ok_or_else(
                    || {
                        CliError::InvalidArgument(format!(
                            "unknown --on-parse-error `{}` (expected: fallback | skip | error)",
                            t
                        ))
                    },
                )?),
                None => None,
            };
            if stdin {
                if check {
                    return Err(CliError::InvalidArgument(
                        "`--check` cannot be combined with `--stdin`; \
                         pipe the buffer through and diff externally"
                            .into(),
                    ));
                }
                return commands::fmt::execute_stdin(stdin_filename.map(|t| t.to_string()));
            }
            if let Some(n) = threads {
                let pool_threads = if n == 0 { 1 } else { n };
                let _ = rayon::ThreadPoolBuilder::new()
                    .num_threads(pool_threads)
                    .build_global();
            }
            commands::fmt::execute_with_policy(check, verbose, parse_policy)
        }
        Commands::Lint {
            fix,
            deny_warnings,
            list_rules,
            list_groups,
            explain,
            open,
            validate_config,
            format,
            profile,
            since,
            new_only_since,
            severity,
            threads,
            no_cache,
            clean_cache,
            baseline,
            no_baseline,
            write_baseline,
            max_warnings,
            watch,
            watch_clear,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);
            if clean_cache {
                return commands::lint::clean_cache();
            }
            if list_rules {
                return commands::lint::list_rules();
            }
            if list_groups {
                return commands::lint::list_groups();
            }
            if let Some(rule) = explain {
                if open {
                    return commands::lint::explain_rule_open(rule.as_str());
                }
                return commands::lint::explain_rule(rule.as_str());
            }
            if validate_config {
                return commands::lint::validate_config();
            }
            if no_cache {
                // SAFETY: env mutation occurs before any worker
                // thread is spawned by the lint pipeline, so no
                // other thread can be reading the environment in
                // parallel.
                unsafe {
                    std::env::set_var("VERUM_LINT_NO_CACHE", "1");
                }
            }
            let fmt = commands::lint::LintOutputFormat::parse(format.as_str())?;
            // Profile selection: explicit --profile flag wins over the
            // VERUM_LINT_PROFILE env var.
            let profile_name: Option<String> = profile
                .map(|t| t.as_str().to_string())
                .or_else(|| std::env::var("VERUM_LINT_PROFILE").ok());
            let severity_filter: Option<commands::lint::LintLevel> = match severity {
                Some(level) => Some(commands::lint::LintLevel::parse(level.as_str()).ok_or_else(
                    || CliError::InvalidArgument(format!(
                        "unknown --severity `{}` (expected: error|warn|info|hint)",
                        level
                    )),
                )?),
                None => None,
            };
            let since_ref: Option<String> = since.map(|t| t.as_str().to_string());

            // Thread-pool sizing: `--threads 0` forces a sequential
            // run (single-thread rayon pool); any other value passes
            // straight through. We build a *global* pool because
            // rayon's API only allows it once per process.
            if let Some(n) = threads {
                let pool_threads = if n == 0 { 1 } else { n };
                // Best-effort: if a pool is already initialised
                // (e.g. by an earlier subcommand), the second call
                // is a no-op — the existing pool is left in place.
                let _ = rayon::ThreadPoolBuilder::new()
                    .num_threads(pool_threads)
                    .build_global();
            }

            if let Some(ref_name) = new_only_since {
                return commands::lint::run_new_only_since(
                    fix,
                    deny_warnings,
                    fmt,
                    profile_name.clone(),
                    severity_filter,
                    ref_name.to_string(),
                );
            }
            if watch {
                commands::lint::run_watch(
                    fix,
                    deny_warnings,
                    fmt,
                    profile_name,
                    since_ref,
                    severity_filter,
                    watch_clear,
                )
            } else {
                let baseline_opt = commands::lint::BaselineMode::from_flags(
                    baseline.map(|t| t.to_string()),
                    no_baseline,
                    write_baseline,
                );
                commands::lint::run_extended_full_with_baseline(
                    fix,
                    deny_warnings,
                    fmt,
                    profile_name,
                    since_ref,
                    severity_filter,
                    max_warnings,
                    baseline_opt,
                )
            }
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
        Commands::Cache(cmd) => commands::cache::execute(cmd),
        Commands::Doctor(args) => commands::doctor::execute(args),
        Commands::Watch { command, clear } => commands::watch::execute(command.as_str(), clear),
        Commands::Hooks(cmd) => match cmd {
            HooksCommands::Install { force } => commands::hooks::install(force),
            HooksCommands::Uninstall => commands::hooks::uninstall(),
            HooksCommands::Status => commands::hooks::status(),
        },
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
        Commands::VbcVersion { archive, raw } => {
            commands::vbc_version::execute(&archive, raw)
        }
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
            lsp_mode,
            dump_smt,
            check_smt_formula,
            solver_protocol,
            ladder,
            ladder_format,
            closure_cache,
            closure_cache_root,
        } => {
            // --ladder short-circuits the standard verify pipeline:
            // route every @verify(strategy) annotation through the
            // typed dispatcher and emit per-theorem verdicts. Honest
            // integration of #71's LadderDispatcher trait surface.
            if ladder {
                return commands::verify_ladder::run_verify_ladder(
                    ladder_format.as_str(),
                );
            }
            // --lsp-mode implies no human output; set an env var
            // the downstream report-renderer reads to switch from
            // human-readable prose to LSP-JSON. Environment is the
            // loose-coupling channel the renderer already uses for
            // output-format toggles (--no-color, --format=json, …).
            if lsp_mode {
                // SAFETY: The env var is used by the downstream
                // verify_cmd's report-renderer as a loose-coupling
                // format toggle. Single-threaded context at CLI
                // entry — no TOCTOU hazard.
                unsafe { std::env::set_var("VERUM_LSP_MODE", "1"); }
            }

            // SMT debugging flags — propagated to the solver via
            // env vars the backend-switcher / Z3 / CVC5 wrappers
            // consult at solver-construction time. Same
            // loose-coupling pattern as VERUM_LSP_MODE; avoids
            // plumbing per-flag knobs through every options
            // struct from CLI → session → solver factory.
            if let Some(ref dir) = dump_smt {
                std::fs::create_dir_all(dir).map_err(|e| {
                    CliError::Custom(
                        format!("creating --dump-smt dir {}: {}", dir.display(), e).into(),
                    )
                })?;
                // SAFETY: single-threaded CLI entry; see --lsp-mode
                // rationale above.
                unsafe {
                    std::env::set_var(
                        "VERUM_DUMP_SMT_DIR",
                        dir.display().to_string(),
                    );
                }
            }
            if solver_protocol {
                unsafe { std::env::set_var("VERUM_SOLVER_PROTOCOL", "1"); }
            }
            if let Some(ref smt_file) = check_smt_formula {
                // --check-smt-formula short-circuits: read the
                // file, dispatch to the configured solver, print
                // sat/unsat/unknown. Skips the whole verify
                // pipeline because the input is raw SMT-LIB — the
                // Verum AST / type-checker / VC generator don't
                // need to run.
                return commands::smt_check::run(
                    smt_file,
                    solver.as_str(),
                    timeout,
                );
            }
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
                    closure_cache_enabled: closure_cache,
                    closure_cache_root,
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
        Commands::ProofDraft {
            theorem,
            goal,
            lemma,
            max,
            format,
        } => {
            commands::proof_draft::run_proof_draft(
                &theorem, &goal, &lemma, max, &format,
            )
        }
        Commands::ProofRepair {
            kind,
            field,
            max,
            format,
        } => {
            commands::proof_repair::run_proof_repair(&kind, &field, max, &format)
        }
        Commands::ForeignImport {
            from,
            file,
            out,
            format,
        } => commands::foreign_import::run_import(&from, &file, out.as_ref(), &format),
        Commands::LlmTactic { sub } => match sub {
            LlmTacticSub::Propose {
                theorem,
                goal,
                lemma,
                hyp,
                history,
                model,
                hint,
                persist,
                audit,
                format,
            } => commands::llm_tactic::run_propose(
                &theorem,
                &goal,
                &lemma,
                &hyp,
                &history,
                &model,
                hint.as_deref(),
                audit.as_ref(),
                persist,
                &format,
            ),
            LlmTacticSub::AuditTrail { audit, format } => {
                commands::llm_tactic::run_audit_trail(audit.as_ref(), &format)
            }
            LlmTacticSub::Models { format } => commands::llm_tactic::run_models(&format),
        },
        Commands::DocRender { sub } => match sub {
            DocRenderSub::Render {
                format,
                out,
                public,
            } => commands::doc_render::run_render(&format, out.as_ref(), public),
            DocRenderSub::Graph { format, public } => {
                commands::doc_render::run_graph(&format, public)
            }
            DocRenderSub::CheckRefs { format, public } => {
                commands::doc_render::run_check_refs(&format, public)
            }
        },
        Commands::CacheClosure { sub } => match sub {
            CacheClosureSub::Stat { root, format } => {
                commands::cache_closure::run_stat(root.as_deref(), &format)
            }
            CacheClosureSub::List { root, format } => {
                commands::cache_closure::run_list(root.as_deref(), &format)
            }
            CacheClosureSub::Get {
                theorem,
                root,
                format,
            } => commands::cache_closure::run_get(&theorem, root.as_deref(), &format),
            CacheClosureSub::Clear { root, format } => {
                commands::cache_closure::run_clear(root.as_deref(), &format)
            }
            CacheClosureSub::Decide {
                theorem,
                signature,
                body,
                cite,
                kernel_version,
                root,
                format,
            } => commands::cache_closure::run_decide(
                &theorem,
                kernel_version.as_deref(),
                &signature,
                &body,
                &cite,
                root.as_deref(),
                &format,
            ),
        },
        Commands::Tactic { sub } => match sub {
            TacticSub::List { format, category } => {
                commands::tactic::run_list(&format, category.as_deref())
            }
            TacticSub::Explain { name, format } => {
                commands::tactic::run_explain(&name, &format)
            }
            TacticSub::Laws { format } => commands::tactic::run_laws(&format),
        },
        Commands::Audit {
            details,
            direct_only,
            framework_axioms,
            kernel_rules,
            epsilon,
            coord,
            no_coord,
            hygiene,
            hygiene_strict,
            owl2_classify,
            framework_conflicts,
            accessibility,
            round_trip,
            coherent,
            proof_honesty,
            bridge_admits,
            framework_soundness,
            coord_consistency,
            htt_roadmap,
            ar_roadmap,
            self_recognition,
            cross_format,
            kernel_intrinsics,
            kernel_discharged_axioms,
            verify_ladder,
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
            } else if epsilon {
                commands::audit::audit_epsilon_with_format(output_format)
            } else if coord {
                commands::audit::audit_coord_with_format(output_format)
            } else if hygiene {
                commands::audit::audit_hygiene_with_format(output_format)
            } else if hygiene_strict {
                commands::audit::audit_hygiene_strict_with_format(output_format)
            } else if owl2_classify {
                commands::audit::audit_owl2_classify_with_format(output_format)
            } else if framework_conflicts {
                commands::audit::audit_framework_conflicts_with_format(output_format)
            } else if accessibility {
                commands::audit::audit_accessibility_with_format(output_format)
            } else if round_trip {
                commands::audit::audit_round_trip_with_format(output_format)
            } else if coherent {
                commands::audit::audit_coherent_with_format(output_format)
            } else if proof_honesty {
                commands::audit::audit_proof_honesty_with_format(output_format)
            } else if bridge_admits {
                commands::audit::audit_bridge_admits_with_format(output_format)
            } else if framework_soundness {
                commands::audit::audit_framework_soundness_with_format(output_format)
            } else if coord_consistency {
                commands::audit::audit_coord_consistency_with_format(output_format)
            } else if htt_roadmap {
                commands::audit::audit_htt_roadmap(output_format)
            } else if ar_roadmap {
                commands::audit::audit_ar_roadmap(output_format)
            } else if self_recognition {
                commands::audit::audit_self_recognition(output_format)
            } else if cross_format {
                commands::audit::audit_cross_format(output_format)
            } else if kernel_intrinsics {
                commands::audit::audit_kernel_intrinsics(output_format)
            } else if kernel_discharged_axioms {
                commands::audit::audit_kernel_discharged_axioms(output_format)
            } else if verify_ladder {
                commands::audit::audit_verify_ladder(output_format)
            } else {
                let options = commands::audit::AuditOptions {
                    verify_checksums: true,
                    verify_signatures: details,
                    verify_proofs: false,
                    cbgr_profiles: false,
                    fix: false,
                    direct_only,
                };
                let dep_result = commands::audit::audit(options);
                // per-theorem coord audit is
                // default-on. Skip with --no-coord.
                if !no_coord {
                    let coord_result =
                        commands::audit::audit_coord_with_format(output_format);
                    // Surface either failure; prefer the dep-audit
                    // error for backwards compatibility.
                    dep_result.and(coord_result)
                } else {
                    dep_result
                }
            }
        }
        Commands::Export { to, output, with_provenance }
        | Commands::ExportProofs { to, output, with_provenance } => {
            let format = commands::export::ExportFormat::parse(&to)?;
            let options = commands::export::ExportOptions {
                format,
                output: match output {
                    Some(p) => verum_common::Maybe::Some(p),
                    None => verum_common::Maybe::None,
                },
                with_provenance,
            };
            commands::export::run(options)
        }
        Commands::Import { from, input, output } => {
            let format = commands::import::ImportFormat::parse(&from)?;
            commands::import::run(commands::import::ImportOptions {
                format,
                input,
                output,
            })
        }
        Commands::Extract { input, output } => {
            commands::extract::run(commands::extract::ExtractOptions {
                input,
                output,
            })
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
