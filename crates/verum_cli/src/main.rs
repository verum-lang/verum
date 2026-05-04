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
mod cog;
mod cog_manager;
mod commands;
mod config;
mod error;
mod feature_overrides;
pub mod registry;
mod repl;
mod script;
mod templates;
mod tier;
mod ui;

use error::{CliError, Result};

#[derive(Parser)]
#[clap(
    name = "verum",
    version = env!("CARGO_PKG_VERSION"),
    about = "The Verum language compiler \u{2014} semantic honesty, cost transparency, zero-cost safety",
    // Industrial-grade help layout:
    //   * `term-width = 0` lets clap auto-detect the terminal width and use it
    //     in full, instead of capping at 100 cols (the default).  Long
    //     descriptions then wrap to the actual screen width without bleeding
    //     into the command-name column.
    //   * `max-term-width = 110` caps the description column on very wide
    //     screens so the description line doesn't run beyond a reading-
    //     comfortable width.
    //   * `disable-help-subcommand = true` hides the noisy auto-generated
    //     `help` subcommand from the catalogue (still reachable via
    //     `verum --help` and `verum <cmd> --help`).
    term_width = 0,
    max_term_width = 110,
    disable_help_subcommand = true,
    help_template = "\
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}",
    after_help = "\
QUICK START
    verum new my_project --profile application   Create a new project
    verum build                                  Build the current project
    verum run                                    Build and run the current project
    verum run file.vr                            Run a single file (AOT by default)
    verum run --interp file.vr                   Run via interpreter
    verum check file.vr                          Type-check without building
    verum playbook                               Launch the interactive notebook

COMMANDS BY GROUP
    Project           new   init   clean   deps   tree   workspace   package   config
    Build & Run       build   run   check   test   bench   watch   profile
    Code Quality      fmt   lint   analyze   doc   audit
    Verification      verify   check-proof   elaborate-proof   proof-draft
                      proof-repair   proof-repl   llm-tactic   tactic
                      cert-replay   cache-closure   cubical   smt-info   smt-stats
    Interop           export   export-proofs   extract   import   foreign-import   doc-render
    Cog Distribution  cog   cog-registry   stdlib   vbc-version
    Diagnostics       doctor   diagnose   cache   info   version   explain   arch   benchmark
    Interactive/IDE   repl   playbook   playbook-convert   lsp   dap   hooks   completions

Run `verum <COMMAND> --help` for detailed help on any command.
"
)]
struct Cli {
    #[clap(subcommand)]
    command: Option<Commands>,

    /// Enable verbose output
    #[clap(short, long, global = true)]
    verbose: bool,

    /// Suppress non-essential output
    #[clap(short, long, global = true)]
    quiet: bool,

    /// Colour mode: auto / always / never
    #[clap(long, global = true, default_value = "auto", value_name = "MODE")]
    color: Text,

    /// Print the verification-architecture version stamp and exit
    ///
    /// The kernel constant `verum_kernel::VVA_VERSION` is the single source
    /// of truth — bump on every kernel-rule acceptance.
    #[clap(long = "vva-version")]
    vva_version: bool,
}

#[derive(Subcommand)]
enum Commands {

    /// Create a new Verum project
    #[command(display_order = 0)]
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

    /// Initialise a Verum project in the current directory
    #[command(display_order = 1)]
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

    /// Build the project (AOT compilation)
    #[command(display_order = 100)]
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
 /// runtime — runtime assertion only (no formal proof)
 /// static — type-level check only
 /// formal — balanced default (compiler picks best technique)
 /// fast — prefer speed over completeness
 /// thorough — maximum completeness (parallel strategies)
 /// certified — produce exportable proof certificate
 /// synthesize — synthesis problem (generate term from spec)
 ///

 /// Legacy values "none", "proof" are aliases for "runtime" and "formal".
        #[clap(
            long,
            value_name = "STRATEGY",
            help = "Verification strategy: runtime|static|formal|fast|thorough|certified|synthesize"
        )]
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

 /// Windows PE subsystem. `console` (default) produces a
 /// standard CLI app that allocates a console window when
 /// launched. `gui` produces a Win32 GUI app — no console
 /// flashes on launch, suitable for desktop applications.
 /// Ignored on non-Windows targets. Overrides the manifest
 /// `[build].windows_subsystem` setting if present.
        #[clap(
            long,
            value_name = "MODE",
            help = "Windows subsystem: console|gui (Windows targets only)"
        )]
        windows_subsystem: Option<Text>,

 // Lint configuration options
 /// Treat all warnings as errors
        #[clap(long, help = "Treat all warnings as errors")]
        deny_warnings: bool,

 /// Treat missing intrinsics as errors (default: warnings)
        #[clap(long, help = "Missing intrinsics become errors")]
        strict_intrinsics: bool,

 /// Promote bug-class lenient skips (#110) to hard errors. When set,
 /// `compile_item_lenient` no longer demotes UndefinedFunction /
 /// WrongArgumentCount / TypeMismatch / NonExhaustivePattern to
 /// warn-level traces — they fail the build. Irreducible skips
 /// (FFI prototypes, unimplemented language features, GPU shaders)
 /// remain tolerated. Recommended for CI / release pipelines.
        #[clap(
            long,
            help = "Bug-class lenient skips become hard errors (#110 strict mode)"
        )]
        strict_codegen: bool,

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

    /// Run a Verum program (interpreter, --aot for native)
    #[command(display_order = 101)]
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
    #[command(display_order = 103)]
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
    #[command(display_order = 104)]
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

    /// Type-check without building
    #[command(display_order = 102)]
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

    /// Re-verify a proof certificate via the kernel
    #[command(display_order = 301)]
    CheckProof {
 /// Path to the `.vproof` certificate file.
        #[clap(value_name = "FILE")]
        file: Text,

 /// **Meta-mode universe lift** (#158 V2 — kernel reflection).
 /// Re-runs the kernel with every `Universe(n)` interpreted as
 /// `Universe(n + LIFT)`. Equivalent to running the proof at a
 /// strictly stronger universe, which is what Gödel's 2nd
 /// Incompleteness Theorem demands for self-soundness claims.
 ///
 /// `--meta-mode` (no value) is equivalent to `--meta-lift 1`.
 ///
 /// Soundness invariant: a closed certificate accepts at lift 0
 /// iff it accepts at any lift k > 0 (universe-cumulativity:
 /// HTT 1.4, U_n ⊂ U_{n+1}). Disagreements indicate either a
 /// shift-implementation bug or an unsound universe-identity
 /// dependence in the proof structure.
 ///
 /// Use case: `verum check-proof foo.vproof --meta-mode` to
 /// confirm a proof survives meta-level interpretation; useful
 /// as a sanity check before publishing.
        #[clap(long, conflicts_with = "meta_lift")]
        meta_mode: bool,

 /// Explicit universe-lift level for meta-mode (#158 V2).
 /// `--meta-lift 0` is identical to default; higher values
 /// run the kernel at progressively stronger universes.
        #[clap(long, value_name = "LIFT")]
        meta_lift: Option<u32>,
    },

    /// Elaborate theorems into kernel-checkable certificates
    #[command(display_order = 302)]
    ElaborateProof {
 /// Path to the `.vr` source file.
        #[clap(value_name = "FILE")]
        file: Text,
 /// Output directory for emitted `.vproof` files. Default:
 /// `<source-dir>/elaborated/`.
        #[clap(long, value_name = "DIR")]
        output_dir: Option<Text>,
    },

    /// Format source code
    #[command(display_order = 200)]
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
 /// - `fallback` (default): silently apply whitespace
 /// normalisation; warn.
 /// - `skip`: leave the file untouched; warn.
 /// - `error`: leave the file untouched; fail the run.
        #[clap(long, value_name = "MODE")]
        on_parse_error: Option<Text>,
 /// Language-feature overrides (applied on top of verum.toml).
        #[clap(flatten)]
        feature_overrides: feature_overrides::LanguageFeatureOverrides,
    },

    /// Static analysis suite
    #[command(display_order = 201)]
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
    #[command(display_order = 203)]
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
    #[command(display_order = 2)]
    Clean {
        #[clap(long)]
        all: bool,
    },

    /// Inspect / export / clean crash reports
    #[command(display_order = 601)]
    #[clap(subcommand)]
    Diagnose(commands::diagnose::DiagnoseCommands),

    /// Manage the script-mode VBC cache
    #[command(display_order = 602)]
    #[clap(subcommand)]
    Cache(commands::cache::CacheCommands),

    /// Run installation health check
    #[command(display_order = 600)]
    Doctor(commands::doctor::DoctorArgs),

    /// Watch for changes and rebuild
    #[command(display_order = 105)]
    Watch {
        #[clap(default_value = "build")]
        command: Text,
        #[clap(long)]
        clear: bool,
    },

    /// Manage git hooks for the current project
    #[command(display_order = 705)]
    #[clap(subcommand)]
    Hooks(HooksCommands),

    /// Manage dependencies
    #[command(display_order = 3)]
    #[clap(subcommand)]
    Deps(DepsCommands),

    /// Start interactive REPL
    #[command(display_order = 700)]
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

    /// Start interactive notebook (Playbook)
    #[command(display_order = 701)]
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

    /// Convert between Playbook formats
    #[command(display_order = 702)]
    #[clap(subcommand, name = "playbook-convert")]
    PlaybookConvert(PlaybookConvertCommands),

    /// Show version information
    #[command(display_order = 604)]
    Version {
        #[clap(long)]
        verbose: bool,
    },

    /// Inspect a VBC archive header
    #[command(display_order = 503)]
    #[clap(name = "vbc-version")]
    VbcVersion {
 /// Path to the .vbc archive.
        #[clap(value_name = "ARCHIVE")]
        archive: std::path::PathBuf,
 /// Emit a single-line key=value form for scripting.
        #[clap(long)]
        raw: bool,
    },

    /// Manage cog packages (build / publish / install)
    #[command(display_order = 6)]
    #[clap(subcommand)]
    Package(PackageCommands),

    /// Profile performance
    #[command(display_order = 106)]
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

    /// Run formal verification
    #[command(display_order = 300)]
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
 /// verification cache (#79). When set, theorem proofs whose
 /// closure hash is in the cache and whose cached verdict was
 /// Ok are skipped without invoking the SMT / kernel re-check.
 /// Cache root defaults to
 /// `<input.parent>/target/.verum_cache/closure-hashes/`;
 /// override with `--closure-cache-root <PATH>`.
        #[clap(long)]
        closure_cache: bool,

 /// Override the closure-cache root directory. Implies
 /// `--closure-cache` if set. Standard CI use is to point
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

    /// Static analysis (legacy alias)
    #[command(display_order = 202)]
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
    #[command(display_order = 605)]
    Explain {
 /// Error code to explain (e.g., E0312 or 0312)
        code: Text,
        #[clap(long)]
        no_color: bool,
    },

    /// Display compiler information
    #[command(display_order = 603)]
    Info {
        #[clap(long)]
        features: bool,
        #[clap(long)]
        llvm: bool,
        #[clap(long)]
        all: bool,
    },

    /// Start Debug Adapter Protocol server
    #[command(display_order = 704)]
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

    /// Start Language Server Protocol server
    #[command(display_order = 703)]
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

    /// Draft proof obligations from a Verum source file
    #[command(display_order = 303)]
    ProofDraft {
 /// Theorem name (the proof body's owner — used for diagnostic
 /// labelling and history attribution).
        #[clap(long)]
        theorem: String,

 /// The focused goal's proposition rendering (what needs to be
 /// proved). Pipe via stdin with `--goal -` for multi-line
 /// goals.
        #[clap(long)]
        goal: String,

 /// Available lemmas in scope as `name:::signature` lines (one
 /// per `--lemma` flag, repeatable). Or use `--lemmas-from
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

    /// Suggest repairs for failing proofs
    #[command(display_order = 304)]
    ProofRepair {
 /// Failure-kind tag — see command help for the full set.
        #[clap(long)]
        kind: String,

 /// Per-kind structured fields as `key=value`. Repeatable.
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

    /// Import theorems from Coq / Lean / Isabelle / Mizar
    #[command(display_order = 404)]
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

    /// Cubical / HoTT primitive catalogue
    #[command(display_order = 310)]
    Cubical {
        #[clap(subcommand)]
        sub: CubicalSub,
    },

    /// Cog distribution registry operations
    #[command(display_order = 501)]
    CogRegistry {
        #[clap(subcommand)]
        sub: CogRegistrySub,
    },

    /// SMT certificate replay (multi-backend cross-validation)
    #[command(display_order = 308)]
    CertReplay {
        #[clap(subcommand)]
        sub: CertReplaySub,
    },

    /// Continuous benchmarking vs other proof systems
    #[command(display_order = 607)]
    Benchmark {
        #[clap(subcommand)]
        sub: BenchmarkSub,
    },

    /// Live proof REPL with stepwise tactic feedback
    #[command(display_order = 305)]
    ProofRepl {
        #[clap(subcommand)]
        sub: ProofReplSub,
    },

    /// LCF-style fail-closed LLM tactic proposer
    #[command(display_order = 306)]
    LlmTactic {
        #[clap(subcommand)]
        sub: LlmTacticSub,
    },

    /// Auto-paper documentation generator
    #[command(display_order = 405)]
    DocRender {
        #[clap(subcommand)]
        sub: DocRenderSub,
    },

    /// Closure-hash incremental verification cache
    #[command(display_order = 309)]
    CacheClosure {
        #[clap(subcommand)]
        sub: CacheClosureSub,
    },

    /// Tactic combinator catalogue surface
    #[command(display_order = 307)]
    Tactic {
        #[clap(subcommand)]
        sub: TacticSub,
    },

    /// Audit framework axioms / dependencies
    #[command(display_order = 204)]
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

 /// Run the kernel re-check (K-Refine-omega / K-Universe-Ascent /
 /// K-Eps-Mu / K-Round-Trip) against every theorem-shaped +
 /// axiom + function declaration in the project, surfacing
 /// per-name admitted / rejected outcomes. Useful as a fast
 /// first gate in CI pipelines before the verifier dispatcher
 /// runs. Exits non-zero on any rejection. (#122)
        #[clap(long)]
        kernel_recheck: bool,

 /// Run the kernel-soundness corpus check (task #80 / VERUM-TRUST-1).
 /// Verifies the Rust-side rule list matches the .vr corpus's
 /// declared rule count, enumerates per-rule proved / admitted
 /// status, and emits parallel Coq + Lean theory files into
 /// `target/audit-reports/kernel-soundness/` for independent
 /// re-checking. Exits non-zero only on Rust↔.vr drift;
 /// admitted lemmas are accountability surface, not failures.
        #[clap(long = "kernel-soundness")]
        kernel_soundness: bool,

 /// kernel_v0 roster audit (task #154 / Phase 3).
 /// Walks the canonical 10-rule kernel_v0 manifest and the
 /// `core/verify/kernel_v0/rules/` directory on disk.
 /// Reports per-rule (Proved / Admitted with IOU) and the
 /// proved-vs-admitted split. Exits non-zero on
 /// manifest↔filesystem drift (missing or orphan source
 /// files). Output:
 /// `target/audit-reports/kernel-v0-roster.json`.
        #[clap(long = "kernel-v0-roster")]
        kernel_v0_roster: bool,

 /// dependent-theorems query (task #188). Given an axiom
 /// name, walks the workspace apply-graph and lists every
 /// theorem whose transitive proof depends on the axiom.
 /// Mathematician-facing utility — when an axiom rejects or
 /// is admitted under audit, "which of my theorems lose
 /// their discharge?" is answered without manual dependency
 /// tracing. Output:
 /// `target/audit-reports/dependent-theorems-<axiom>.json`.
        #[clap(long = "dependent-theorems", value_name = "AXIOM")]
        dependent_theorems: Option<String>,

 /// Codegen-pass kernel-discharge attestation audit
 /// (task #162 / CompCert-style verified compilation).
 /// Walks the canonical 6-pass codegen manifest from
 /// `verum_kernel::codegen_attestation` and reports per-pass
 /// status (Discharged / AdmittedWithIOU / NotYetAttested).
 /// Each pass entry carries its semantic invariant + the
 /// concrete proof obligation that would discharge it.
 /// Foundation surface: V0 has every entry NotYetAttested;
 /// future per-pass discharge work flips entries individually.
 /// Output: `target/audit-reports/codegen-attestation.json`.
        #[clap(long = "codegen-attestation")]
        codegen_attestation: bool,

 /// Differential-kernel cross-implementation audit
 /// (task #159 / Rust↔Verum self-hosted kernel agreement).
 /// Runs every kernel_v0 rule's canonical certificate through
 /// the Rust trusted base (`verum_kernel::proof_checker`) AND
 /// the Verum-self-hosted kernel (`core/verify/kernel_v0/`).
 /// Reports per-rule agreement (`both_accept` / `both_reject` /
 /// `disagreement` / `not_yet_self_hosting`). Exits non-zero
 /// on any disagreement; `not_yet_self_hosting` is observability
 /// (current state — Verum side awaits parser-blocker fix).
 /// Output: `target/audit-reports/differential-kernel.json`.
        #[clap(long = "differential-kernel")]
        differential_kernel: bool,

 /// Run the differential-kernel fuzz audit. Mutation-based
 /// property fuzzing: takes the canonical-certificate roster,
 /// applies structural mutations (universe lifts, subterm
 /// swaps, binder rewrites, application injections), runs every
 /// mutant through every registered kernel. The property
 /// invariant is that every mutant produces a unanimous
 /// agreement; any disagreement is a kernel-implementation
 /// bug and exits non-zero. Bounded deterministic campaign
 /// (default 500 iterations, fixed seed) — disagreements are
 /// reproducible across runs.
 /// Output: `target/audit-reports/differential-kernel-fuzz.json`.
        #[clap(long = "differential-kernel-fuzz")]
        differential_kernel_fuzz: bool,

 /// Run the reflection-tower audit. Walks every level in the
 /// ordinal-indexed meta-soundness tower (REF^0 through REF^4
 /// plus the REF^ω limit), reports per-level discharge verdict
 /// and citation (Gödel 1931 + Feferman 1989, Pohlers 2009,
 /// Beklemishev 2003, Schütte 1965, Feferman 1962). Auto-derives
 /// the minimum reflection level required by the current
 /// kernel-rule roster from per-rule footprint enumeration. Exits
 /// non-zero if any finite level fails to discharge.
 /// Output: `target/audit-reports/reflection-tower.json`.
        #[clap(long = "reflection-tower")]
        reflection_tower: bool,

 /// Run the ATS-V Architectural Type System discharge audit.
 /// Walks the kernel-side architectural intrinsic registry
 /// (capability discipline, boundary check, composition algebra,
 /// lifecycle integrity, foundation consistency, anti-pattern
 /// catalog, CVE-closure, end-to-end soundness witness) and
 /// reports stable RFC error codes ATS-V-AP-001..010 from the
 /// canonical anti-pattern catalog (10 patterns; 22
 /// remaining per `internal/specs/ats-v.md` §7).
 /// Output: `target/audit-reports/arch-discharges.json`.
        #[clap(long = "arch-discharges")]
        arch_discharges: bool,

 /// ATS-V Counterfactual reasoning audit. Runs the
 /// counterfactual reasoning engine over a synthetic
 /// `CounterfactualPair` battery (one per canonical
 /// `ArchProposition` × baseline metric set) against the
 /// default Shape, exercising every entry in the engine's
 /// dispatch table. Verifies the engine's per-arm soundness
 /// contracts (HoldsBoth/HoldsBaseOnly/...) at audit time and
 /// surfaces the comparative report as JSON. Output:
 /// `target/audit-reports/counterfactual.json`.
        #[clap(long = "counterfactual")]
        counterfactual: bool,

 /// ATS-V Adjunction analyzer audit. Runs the
 /// adjunction analyzer over a synthetic battery covering each
 /// of the four canonical adjunctions (Inline⊣Extract /
 /// Specialise⊣Generalise / Decompose⊣Compose /
 /// Strengthen⊣Weaken) plus a chain composition pin and a
 /// failure case. Verifies recogniser soundness +
 /// preservation/gain coverage at audit time. Output:
 /// `target/audit-reports/adjunctions.json`.
        #[clap(long = "adjunctions")]
        adjunctions: bool,

 /// ATS-V Yoneda-equivalence checker audit. Per
 /// spec §20.7 + §23, two architectures are equivalent iff
 /// every Observer in the canonical roster (EndUser /
 /// PeerCog / Stakeholder / Auditor / Adversary) projects
 /// the same observation. Runs a synthetic battery covering
 /// identity (trivially equivalent), per-observer
 /// distinguishability cases (Auditor sees foundation,
 /// Adversary sees network, EndUser sees exposes), and the
 /// trivially-safe refactoring entry. Output:
 /// `target/audit-reports/yoneda.json`.
        #[clap(long = "yoneda")]
        yoneda: bool,

        /// ATS-V `@arch_module(...)` adoption coverage audit. Walks
        /// every `.vr` file in the project + stdlib, reports which
        /// modules carry `@arch_module(...)` declarations vs. the
        /// total module count, and surfaces a per-module status list
        /// as JSON. Observability gate — does not fail the build;
        /// coverage grows incrementally per spec §17.5 backward-
        /// compat. Output: `target/audit-reports/arch-coverage.json`.
        #[clap(long = "arch-coverage")]
        arch_coverage: bool,

        /// ATS-V whole-corpus cross-cog architectural analysis.
        /// Walks every `@arch_module(...)`-annotated `.vr` file in
        /// the project + stdlib, builds the global cog → mounts
        /// graph, populates each cog's `DiagnosticContext` with
        /// `composed_foundations` + `composes_graph` edges, then
        /// runs the canonical 32-pattern checker.  Activates AP-003
        /// DependencyCycle and AP-005 FoundationDrift on real
        /// cross-cog architecture.  Output:
        /// `target/audit-reports/arch-corpus.json`.
        #[clap(long = "arch-corpus")]
        arch_corpus: bool,

 /// Run the bridge-discharge audit (task #134 / MSFS-L4.1).
 /// Walks every `apply kernel_*_strict(args)` invocation in the
 /// corpus's proof bodies and replays each literal-arg call
 /// through `verum_kernel::dispatch_intrinsic`. Reports
 /// per-bridge: callsite count, literal vs non-literal split,
 /// dispatcher decisions, and the count of false discharges
 /// (cases where the dispatcher rejected the args). Exits
 /// non-zero on any false discharge or on bridges cited
 /// without a dispatcher entry. This is the observability
 /// layer for L4 promotion; the elaborator-time wiring that
 /// makes the verdict load-bearing at compile time is task #135.
        #[clap(long = "bridge-discharge")]
        bridge_discharge: bool,

        /// Stdlib layer-classification audit (Phase 1 of the
        /// precompiled-stdlib epic). Walks every `.vr` file in the
        /// embedded stdlib archive, classifies each module as one of
        /// `runtime` / `proof` / `meta` based on the items it declares,
        /// and reports per-module + per-subtree counts plus a list of
        /// mixed-layer modules that need explicit `@layer(...)` or a
        /// file split before Phase 2 (directory refactor). Read-only
        /// audit — does not modify any source. Output:
        /// `target/audit-reports/stdlib-layers.{md,json}`.
        #[clap(long = "stdlib-layers")]
        stdlib_layers: bool,

        /// Proof-archive verification audit (Phase 8 of the
        /// precompiled-stdlib epic). Decodes the embedded VBC
        /// archive's `theorems` table, resolves each discharge
        /// receipt to its body in `~/.verum/cert-store/`, runs the
        /// kernel-only structural re-check (blake3 integrity +
        /// schema-version gate), caches per-theorem verdicts in
        /// `~/.verum/replay-cache/<compiler-version>/`, and prints
        /// a per-theorem report. Read-only audit — does not modify
        /// any source. Exits non-zero on `rejected` or `error`
        /// theorem verdicts; `not_discharged` and `inconclusive`
        /// are observability and don't fail the build.
        #[clap(long = "proof-archive")]
        proof_archive: bool,

 /// Run the runtime ν-monotonicity drive (task #139 / MSFS-L4.6).
 /// For every theorem-shaped item with a `@verify(<strategy>)`
 /// annotation, dispatches the obligation at every backbone
 /// strategy from `Runtime` up to and including the declared
 /// strategy, then verifies the strict-ν-monotonicity
 /// invariant: `Closes` at any strategy `S_strict` MUST imply
 /// `Closes` at every coarser backbone strategy `S_coarser ≤
 /// S_strict`. Exits non-zero on any inversion. This is the
 /// architectural promise the verification ladder makes by
 /// design — this audit keeps the promise honest at runtime
 /// across the live corpus.
        #[clap(long = "ladder-monotonicity")]
        ladder_monotonicity: bool,

 /// Run the cross-format roundtrip audit (task #138 / MSFS-L4.5).
 /// Walks every `@theorem`/`@lemma`/`@corollary` in the
 /// project, emits per-theorem `.v` (Coq) and `.lean` (Lean 4)
 /// files into `target/audit-reports/cross-format-roundtrip/`,
 /// and invokes `coqc` / `lean` against each emitted file.
 /// Aggregates per-theorem foreign-tool verdicts. Exits
 /// non-zero only when an AVAILABLE foreign tool reports a
 /// real failure on at least one emitted file. Hosts without
 /// the foreign tools installed get `tool_missing`
 /// observability without failing the gate.
        #[clap(long = "cross-format-roundtrip")]
        cross_format_roundtrip: bool,

 /// Force docker backend for the cross-format gate (#149 / MSFS-L4.15).
 /// Without this flag the gate uses host PATH-resolved coqc/lean,
 /// surfacing `tool_missing` if absent. With `--docker`, foreign
 /// tools run inside their canonical container images
 /// (coqorg/coq:8.18.0-flambda, leanprovercommunity/lean4:4.5.0
 /// by default; override via VERUM_DOCKER_IMAGE_COQ /
 /// VERUM_DOCKER_IMAGE_LEAN env vars). Each emitted .v / .lean
 /// file is mounted read-only into the container, the foreign
 /// tool runs against it, and the verdict surfaces as
 /// `passed`/`failed` instead of `tool_missing`. Equivalent to
 /// setting `VERUM_FOREIGN_TOOL_BACKEND=docker`.
        #[clap(long = "docker")]
        docker: bool,

 /// Verify the canonical proof-term certificate library (#157
 /// follow-up). Walks `core/verify/proof_term_examples/*.vproof`
 /// (or any directory pointed at by `VERUM_PROOF_TERM_EXAMPLES`),
 /// runs `proof_checker::Certificate::verify()` on each, exits
 /// non-zero on any rejection. This is the trust-base
 /// regression suite — the canonical proofs (identity,
 /// polymorphic identity, K combinator, transitivity) that
 /// every kernel implementation claiming Verum compatibility
 /// must accept.
        #[clap(long = "proof-term-library")]
        proof_term_library: bool,

 /// Verify provenance signatures on emitted cross-format files
 /// (#174). Walks the corpus, recomputes each theorem's
 /// expected `verum_signature` header, and compares it to the
 /// signature actually present in
 /// `target/audit-reports/cross-format-roundtrip/{coq,lean}/*`.
 /// Exits non-zero on any mismatch. Reproducibility primitive:
 /// a third-party reviewer pulls the published .v / .lean files
 /// out of supplementary material and runs this gate to confirm
 /// the files were emitted by the named kernel version against
 /// the named corpus state.
        #[clap(long = "signatures")]
        signatures: bool,

 /// Run the kernel-soundness IOU dashboard. Enumerates
 /// every kernel rule whose
 /// soundness lemma is admitted with an IOU reason in
 /// `core/verify/kernel_soundness/`; groups by RuleCategory
 /// (Structural / Cubical / Refinement / Quotient / Inductive /
 /// SmtAxiom / Diakrisis); emits structured JSON + plain summary.
 /// Drives discharge prioritisation: high-priority admits surface
 /// at the top. This is the metric-driven foundation for the
 /// path to "constructively verified from first principles" —
 /// each admit closed shrinks Verum's trusted base.
        #[clap(long = "soundness-iou")]
        soundness_iou: bool,

 /// Run the unified audit-bundle (#151). Executes each of the
 /// load-bearing L1+L2+L3+L4 gates in dependency order
 /// (`--bridge-discharge`, `--kernel-discharged-axioms`,
 /// `--apply-graph`, `--cross-format-roundtrip`) and aggregates
 /// their JSON outputs into a single `target/audit-reports/
 /// bundle.json`. Top-level `l4_load_bearing: bool` summarises
 /// the corpus's L4 verdict in one boolean. The bundle is the
 /// user-facing UX for the verum-corpus L4-readiness gate: one
 /// command, one verdict, all evidence in one place.
        #[clap(long = "bundle")]
        bundle: bool,

 /// Run the apply-graph transitive bridge-discharge audit
 /// (task #150 / MSFS-L4.13). Walks every theorem in the
 /// project and classifies its TRANSITIVE apply-chain leaves
 /// — each `apply <symbol>(args)` resolves through the
 /// workspace symbol table to its body; the recursion
 /// terminates at axiom leaves classified as
 /// `kernel_strict` / `framework_axiom` / `placeholder_axiom`
 /// / `unresolved`. This is the load-bearing complement to
 /// `--bridge-discharge` (which only checks the immediate
 /// apply): `--apply-graph` follows the chain across `_full`
 /// forms and stdlib delegates so a placeholder leak deep in
 /// the chain surfaces. Exits non-zero when any theorem's
 /// composition has `placeholder_axiom > 0` or
 /// `unresolved > 0` — those theorems are not yet L4 load-
 /// bearing.
        #[clap(long = "apply-graph")]
        apply_graph: bool,

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

 /// foundation-profiles audit:
 /// classify every `@framework(<name>, "<citation>")` marker
 /// by its underlying logical foundation (ZFC family / MLTT
 /// / HoTT / Cubical / CIC) using the `verum_kernel::
 /// foundation_profile::FoundationDistribution` analyzer.
 /// Reports per-foundation citation counts, lists unresolved
 /// framework names, and detects cross-foundation conflicts
 /// (UIP ⊥ univalence). Exits non-zero on any conflict.
 ///

 /// Output: `target/audit-reports/foundation-profiles.json`.
        #[clap(long)]
        foundation_profiles: bool,

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
 /// strict-ν-monotonicity invariant. Exits non-zero on any
 /// monotonicity violation.
        #[clap(long)]
        verify_ladder: bool,

 /// Manifest-coverage audit (#290): enumerate every
 /// `Verum.toml` manifest field, report its wiring status
 /// (load-bearing / partial / forward-looking), and emit a
 /// JSON report at `target/audit-reports/manifest-coverage.json`.
 /// Load-bearing inert-defense gate: any future PR that adds
 /// a manifest field without wiring it produces a
 /// `forward-looking` row pointing at the closure follow-up
 /// task. CI consumers can grep `is_wired:false` rows to
 /// know what's pending.
        #[clap(long = "manifest-coverage")]
        manifest_coverage: bool,

 /// MLS-coverage audit (#296): walk the project's .vr files
 /// counting classified functions, classified parameters,
 /// `@declassify` boundaries, and sink-context consumers
 /// (Logger / FS / Network / etc.). Emits JSON to
 /// `target/audit-reports/mls-coverage.json`. Useful for
 /// security-review dashboards and CI gates that track
 /// classification growth in regulated-environment codebases.
        #[clap(long = "mls-coverage")]
        mls_coverage: bool,

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

 /// Bridge-admit footprint audit (M-EXPORT V2 / K-Round-Trip):
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

    /// Export proofs to an external assistant's format
    #[command(display_order = 400)]
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

    /// Alias for `export --to <format>`
    #[command(display_order = 401)]
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

    /// Extract executable programs from constructive proofs
    #[command(display_order = 402)]
    Extract {
 /// Optional explicit input `.vr` path. When absent, all `.vr`
 /// files under the project's manifest directory are scanned.
        input: Option<std::path::PathBuf>,
 /// Output directory (defaults to `extracted/`).
        #[clap(long, short, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
    },

    /// Import knowledge-base formats (e.g. OWL 2)
    #[command(display_order = 403)]
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

    /// Display the dependency tree
    #[command(display_order = 4)]
    Tree {
 /// Show duplicate dependencies
        #[clap(long)]
        duplicates: bool,
 /// Maximum depth to display
        #[clap(long)]
        depth: Option<usize>,
    },

    /// Manage the workspace
    #[command(display_order = 5)]
    #[clap(subcommand)]
    Workspace(WorkspaceCommands),

    /// Stdlib precompile / inspection
    #[command(display_order = 502)]
    Stdlib {
        #[clap(subcommand)]
        sub: StdlibSub,
    },

    /// Cog precompile / inspection
    #[command(display_order = 500)]
    Cog {
        #[clap(subcommand)]
        sub: CogSub,
    },

    /// Generate shell completion scripts
    #[command(display_order = 706)]
    Completions {
 /// Shell to generate completions for.
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Show the resolved language-feature set
    #[command(display_order = 7)]
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
    },

    /// Show formal-verification engine capabilities
    #[command(display_order = 311)]
    #[clap(name = "smt-info")]
    SmtInfo {
 /// Output as machine-readable JSON instead of human-readable text.
        #[clap(long)]
        json: bool,
    },

    /// Show recent verification routing statistics
    #[command(display_order = 312)]
    #[clap(name = "smt-stats")]
    SmtStats {
 /// Output as JSON instead of formatted report.
        #[clap(long)]
        json: bool,
 /// Reset statistics after printing.
        #[clap(long)]
        reset: bool,
    },

    /// ATS-V Architectural Type System operations
    #[command(display_order = 606)]
    Arch {
        #[clap(subcommand)]
        cmd: ArchCommands,
    },
}

#[derive(Subcommand)]
enum ArchCommands {
 /// Show structured architectural type information for a cog.
 /// Per spec §32.4: outputs `Shape` + anti-pattern violations
 /// + suggestions in human-friendly plain text or
 /// machine-readable JSON.
 ///
 /// In (current), without ATS-V phase wiring, the
 /// command returns the default Shape with the canonical
 /// anti-pattern catalog roster. After , it consumes
 /// the cog's actual `@arch_module(...)` declaration and runs
 /// per-cog dispatch.
    Explain {
 /// Cog or module path to explain. Currently a stub
 /// argument — wiring resolves it against the
 /// project's cog graph.
        cog: Option<String>,
 /// Output format: `plain` (human) or `json` (agent).
        #[clap(long, default_value = "plain")]
        format: String,
    },
 /// List the canonical anti-pattern catalog with stable RFC
 /// codes ATS-V-AP-NNN. Equivalent to `verum audit
 /// --arch-discharges` filtered to the catalog table.
    Catalog {
 /// Output format: `plain` (human) or `json` (agent).
        #[clap(long, default_value = "plain")]
        format: String,
 /// Filter to MTAC patterns only (AP-027..032).
        #[clap(long)]
        mtac_only: bool,
 /// Filter by season (1 or 2).
        #[clap(long, value_name = "N")]
        season: Option<u8>,
    },
 /// Check ATS-V architectural type invariants on a .vr file.
 /// end-to-end: parses the file, walks every module
 /// declaration, extracts @arch_module(...) attributes, runs
 /// the canonical 32-pattern catalog, and reports violations.
 /// Per spec §11.4 — backward-compat: modules без аннотации
 /// pass vacuously (default Shape).
    Check {
 /// Path to a .vr file (or `-` for stdin).
        file: String,
 /// Output format: `plain` (human) or `json` (agent).
        #[clap(long, default_value = "plain")]
        format: String,
 /// Strict mode: warnings → errors, missing CVE-closure → error.
        #[clap(long)]
        strict: bool,
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
enum CubicalSub {
 /// List every primitive with a one-line semantics summary.
 /// Optional `--category` filters to one of: identity / path_ops /
 /// induction / transport / composition / glue / universe.
    Primitives {
        #[clap(long)]
        category: Option<String>,
        #[clap(long, default_value = "plain")]
        output: String,
    },
 /// Full structured doc for a single primitive.
    Explain {
        name: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
 /// List every computation / reduction rule.
    Rules {
        #[clap(long, default_value = "plain")]
        output: String,
    },
 /// Parse + validate a face formula (e.g. `i = 0 ∧ j = 1`).
    Face {
        formula: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
}

#[derive(Subcommand)]
enum CogRegistrySub {
    Publish {
        #[clap(long, value_name = "FILE")]
        manifest: PathBuf,
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        #[clap(long, value_name = "ID", default_value = "local")]
        registry_id: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Lookup {
        #[clap(long)]
        name: String,
        #[clap(long)]
        version: String,
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        #[clap(long, value_name = "ID", default_value = "local")]
        registry_id: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Search {
        #[clap(long, value_name = "SUBSTRING")]
        name: Option<String>,
        #[clap(long, value_name = "DOI")]
        paper_doi: Option<String>,
        #[clap(long, value_name = "TAG")]
        framework: Option<String>,
        #[clap(long, value_name = "NAME")]
        theorem: Option<String>,
        #[clap(long, value_name = "KIND")]
        require_attestation: Option<String>,
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        #[clap(long, value_name = "ID", default_value = "local")]
        registry_id: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Verify {
        #[clap(long)]
        name: String,
        #[clap(long)]
        version: String,
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        #[clap(long, value_name = "ID", default_value = "local")]
        registry_id: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Consensus {
        #[clap(long)]
        name: String,
        #[clap(long)]
        version: String,
 /// Repeatable: each mirror is a separate registry root.
        #[clap(long, value_name = "DIR")]
        mirror: Vec<PathBuf>,
        #[clap(long, default_value = "plain")]
        output: String,
    },
 /// Seed an in-process registry with a demo cog and dump its
 /// metadata. Useful for the docs generator + tutorial walks.
    SeedDemo {
        #[clap(long, default_value = "plain")]
        output: String,
    },
}

#[derive(Subcommand)]
enum CertReplaySub {
    Replay {
        #[clap(long)]
        backend: String,
 /// Read the cert from a JSON file (preferred for real
 /// proofs). Mutually exclusive with `--format` /
 /// `--theory` / `--conclusion` / `--body`.
        #[clap(long, value_name = "FILE")]
        cert: Option<PathBuf>,
 /// Inline cert: format tag.
        #[clap(long, default_value = "")]
        format: String,
 /// Inline cert: SMT-LIB theory.
        #[clap(long, default_value = "")]
        theory: String,
 /// Inline cert: theorem-shaped conclusion.
        #[clap(long, default_value = "")]
        conclusion: String,
 /// Inline cert: raw cert body.
        #[clap(long, default_value = "")]
        body: String,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    CrossCheck {
 /// Backend to invoke. Repeatable. When omitted, runs
 /// every external backend (Z3 / CVC5 / Verit / OpenSmt /
 /// Mathsat) plus the always-on kernel-only baseline.
        #[clap(long, value_name = "NAME")]
        backend: Vec<String>,
        #[clap(long, value_name = "FILE")]
        cert: Option<PathBuf>,
        #[clap(long, default_value = "")]
        format: String,
        #[clap(long, default_value = "")]
        theory: String,
        #[clap(long, default_value = "")]
        conclusion: String,
        #[clap(long, default_value = "")]
        body: String,
 /// Fail with non-zero exit when any available backend
 /// disagrees with the others. This is the
 /// `@verify(certified)` semantics: every available solver
 /// must accept.
        #[clap(long)]
        require_consensus: bool,
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Formats {
        #[clap(long, default_value = "plain")]
        output: String,
    },
    Backends {
        #[clap(long, default_value = "plain")]
        output: String,
    },
}

#[derive(Subcommand)]
enum BenchmarkSub {
 /// Run the suite against a single system and emit raw results.
    Run {
        #[clap(long)]
        system: String,
        #[clap(long, default_value = "default")]
        suite_name: String,
        #[clap(long, value_name = "NAME")]
        theorem: Vec<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
 /// Run the suite against multiple systems and emit a
 /// comparison matrix. Without any `--system` flag, runs all
 /// five canonical systems.
    Compare {
        #[clap(long, value_name = "NAME")]
        system: Vec<String>,
        #[clap(long, default_value = "default")]
        suite_name: String,
        #[clap(long, value_name = "NAME")]
        theorem: Vec<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
 /// List every supported metric with its `higher_is_better`
 /// direction.
    Metrics {
        #[clap(long, default_value = "plain")]
        format: String,
    },
}

#[derive(Subcommand)]
enum ProofReplSub {
 /// Run a batch of REPL commands non-interactively. Commands
 /// can come from a file (`--commands`) and / or repeated
 /// `--cmd` flags; both are concatenated in CLI order.
 ///

 /// Command-script syntax (one per line):
 /// - `apply <tactic>` — apply a tactic. Bare lines are
 /// also treated as `apply <line>`.
 /// - `undo` / `redo` / `status` / `show-goals` /
 /// `show-context` / `visualise` — REPL navigation.
 /// - `hint` (default 5) / `hint <N>` — proposed next steps.
 /// - `# comment` — skipped.
 /// - blank line — skipped.
    Batch {
        #[clap(long)]
        theorem: String,
        #[clap(long)]
        goal: String,
 /// Lemmas in scope (`name:::signature[:::lineage]`,
 /// repeatable).
        #[clap(long, value_name = "NAME:::SIGNATURE[:::LINEAGE]")]
        lemma: Vec<String>,
 /// Read commands from a file.
        #[clap(long, value_name = "FILE")]
        commands: Option<PathBuf>,
 /// Inline command. Repeatable — concatenated after
 /// `--commands` content.
        #[clap(long = "cmd", value_name = "LINE")]
        cmd: Vec<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
 /// Apply a sequence of tactics and emit the resulting proof
 /// tree as Graphviz DOT (suitable for `dot -Tsvg`). Non-zero
 /// exit on any kernel rejection.
    Tree {
        #[clap(long)]
        theorem: String,
        #[clap(long)]
        goal: String,
        #[clap(long, value_name = "NAME:::SIGNATURE[:::LINEAGE]")]
        lemma: Vec<String>,
 /// Tactic to apply. Repeatable; ordered as on the CLI.
        #[clap(long, value_name = "TACTIC")]
        apply: Vec<String>,
    },
}

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
 /// citation. CI-friendly.
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
 /// Remove every cache entry. Idempotent.
    Clear {
        #[clap(long)]
        root: Option<String>,
        #[clap(long, default_value = "plain")]
        format: String,
    },
 /// Probe the cache: report Skip / Recheck for the given fingerprint.
    Decide {
        theorem: String,
 /// Theorem signature payload (hashed by the cache). Pass any
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
enum StdlibSub {
    /// Precompile `core/` to a `.vbca` archive embedded into the
    /// compiler binary at build time (Phase 4 of the precompiled-
    /// stdlib epic). Default output path is
    /// `target/precompiled-stdlib/runtime.vbca`. Stdlib path defaults
    /// to the current workspace's `core/` directory; pass
    /// `--stdlib-path` to override.
    Precompile {
        /// Override `core/` path. Defaults to the workspace's
        /// `core/` directory (auto-detected by walking up from cwd).
        #[clap(long, value_name = "DIR")]
        stdlib_path: Option<std::path::PathBuf>,

        /// Output `.vbca` path. Defaults to
        /// `<workspace>/target/precompiled-stdlib/runtime.vbca`.
        #[clap(long, short = 'o', value_name = "FILE")]
        out: Option<std::path::PathBuf>,

        /// Target triple to compile for. `None` = host triple.
        /// Phase 4b will read this and emit per-target variants for
        /// cfg-conditional functions; today the value is recorded
        /// but selection is host-only.
        #[clap(long, value_name = "TRIPLE")]
        target: Option<String>,

        /// Verbose progress output.
        #[clap(long)]
        verbose: bool,
    },
}

#[derive(Subcommand)]
enum CogSub {
    /// Precompile a local Verum cog to a `.vbca` archive (Phase 12
    /// of the precompiled-stdlib epic). Reads `Verum.toml` for cog
    /// name + version, walks the source tree, runs the same global-
    /// registration pipeline used for stdlib precompile, and writes
    /// the archive to the canonical registry-naming-convention path
    /// `<cog>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`.
    /// Override the output via `--out`.
    Precompile {
        /// Cog directory containing `Verum.toml`. Defaults to the
        /// current working directory.
        #[clap(long, value_name = "DIR")]
        cog_dir: Option<std::path::PathBuf>,

        /// Output `.vbca` path. Defaults to the canonical
        /// `target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`
        /// inside the cog directory.
        #[clap(long, short = 'o', value_name = "FILE")]
        out: Option<std::path::PathBuf>,

        /// Target triple to compile for. `None` = host triple.
        /// Cross-compile matrix builds (Phase 12b) iterate this axis.
        #[clap(long, value_name = "TRIPLE")]
        target: Option<String>,

        /// Verbose progress output.
        #[clap(long)]
        verbose: bool,
    },

    /// Verify a registry-distributed `.vbca` is byte-identical to
    /// the output of locally precompiling its source (Phase 15 of
    /// the precompiled-stdlib epic).  Detects registry tampering
    /// and reproducibility regressions.
    ///
    /// Three modes:
    ///
    ///   1. Fully local: `--source-dir DIR --reference-vbca FILE`.
    ///   2. Tarball-mode: `--source-tar PATH --reference-vbca FILE`.
    ///   3. Remote: `<name>@<version> [--registry URL]` — fetch
    ///      both the source tarball and reference VBCA from the
    ///      registry, precompile locally, byte-compare.
    Reproduce {
        /// Cog spec `<name>@<version>`.  When set, missing source
        /// or reference paths are fetched from the registry.
        #[clap(value_name = "SPEC")]
        spec: Option<String>,

        /// Local pre-extracted source tree (must contain
        /// `Verum.toml`).  Mutually exclusive with `--source-tar`.
        #[clap(long, value_name = "DIR", conflicts_with = "source_tar")]
        source_dir: Option<std::path::PathBuf>,

        /// Local source tarball (`.tar.gz`) to extract before
        /// precompiling.  Mutually exclusive with `--source-dir`.
        #[clap(long, value_name = "TARBALL", conflicts_with = "source_dir")]
        source_tar: Option<std::path::PathBuf>,

        /// Reference `.vbca` archive to byte-compare against.
        /// When omitted, falls through to the registry fetcher
        /// using the spec.
        #[clap(long, value_name = "FILE")]
        reference_vbca: Option<std::path::PathBuf>,

        /// Override the registry base URL (defaults to
        /// `https://vcogs.io`).
        #[clap(long, value_name = "URL")]
        registry: Option<String>,

        /// Keep the working directory after the check completes
        /// (useful for post-mortem inspection).
        #[clap(long)]
        keep_workdir: bool,

        /// Verbose progress output.
        #[clap(long)]
        verbose: bool,
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
    let handler = builder
        .spawn(main_inner)
        .expect("failed to spawn main thread");
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

    let spawned = match std::thread::Builder::new()
        .name("verum-main".into())
        .stack_size(STACK_SIZE)
        .spawn(move || run_command(cli))
    {
        Ok(handle) => handle,
        Err(e) => {
            // #113 — graceful spawn failure. Pre-fix this panicked
            // with `expect("Failed to spawn main thread")`, which the
            // crash reporter would intercept and emit as an internal
            // crash report. That misleads the user: failure to spawn
            // a worker thread is an OS resource issue (typically
            // EAGAIN under thread / address-space exhaustion), not a
            // compiler bug. Surface it as a typed CLI error and
            // exit with the conventional `74 EX_IOERR` code so CI
            // wrappers can distinguish "OS refused us a thread"
            // from "compilation failed".
            ui::error(&format!(
                "failed to spawn compiler worker thread: {}\n\
                 hint: this is usually OS thread / address-space exhaustion, \
                 not a verum bug. retry with fewer concurrent processes, or \
                 check `ulimit -u` / process count.",
                e
            ));
            process::exit(74);
        }
    };
    let result = match spawned.join() {
        Ok(r) => r,
        Err(panic_payload) => {
            // The worker thread panicked. The crash reporter
            // installed at the top of `main()` already wrote a
            // structured report to ~/.verum/crashes/; the panic hook
            // also printed a backtrace. Render a clear summary here
            // and exit with `70 EX_SOFTWARE` so wrappers can
            // distinguish internal-crash from compilation-failure
            // (which exits with 1) and from OS errors (74 above).
            //
            // We deliberately don't `resume_unwind` — that would
            // hand the panic back to the runtime and re-trigger the
            // crash hook output, doubling the user's noise.
            let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "<non-string panic payload>".to_string()
            };
            ui::error(&format!(
                "internal compiler error (worker thread panicked): {}\n\
                 a structured crash report has been written to \
                 `~/.verum/crashes/`. please file a bug at \
                 https://github.com/verum-lang/verum/issues with that \
                 report attached.",
                msg
            ));
            process::exit(70);
        }
    };

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
            windows_subsystem,
            deny_warnings,
            strict_intrinsics,
            strict_codegen,
            deny_lint,
            warn_lint,
            allow_lint,
            forbid_lint,
            feature_overrides,
        } => {
            let _smt_stats = smt_stats; // Will be plumbed into session options
            // #110 strict-codegen propagation. The flag is process-scoped so
            // every entry point — `commands::file::build`, `commands::build::execute`,
            // sub-process compilations — picks it up via `LintConfig::default()`
            // without needing to thread the field through every signature.
            // Mirrors `VERUM_FULL_STDLIB` (#109) and `VERUM_NO_PARALLEL_ANALYZE`.
            if strict_codegen {
                // SAFETY: writing to the process environment is safe in single-
                // threaded CLI startup; the flag is set before the pipeline
                // constructs any threads. Rust 2024 unsafe-edition compatibility:
                // wrap in unsafe block.
                unsafe {
                    std::env::set_var("VERUM_STRICT_CODEGEN", "1");
                }
            }
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
                        target.as_ref().map(|t| t.as_str()),
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
                windows_subsystem,
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
 // 1. `--interp` / `--aot` shortcuts on the Run command
 // 2. `--tier` from LanguageFeatureOverrides
 // (accepts interpret|aot|check; "check" is invalid
 // for `run` and yields an error)
 // 3. default: interpreter
            let tier_from_override = feature_overrides
                .tier
                .as_ref()
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
            let tier_label = if tier_num == Some(1) {
                "aot"
            } else {
                "interpreter"
            };

            verum_error::crash::set_command("run");
            verum_error::crash::set_tier(tier_label);

 // Inline-eval and stdin sources synthesise a temporary
 // script file with a shebang prefix so they flow through
 // the same script-mode pipeline as on-disk scripts —
 // identical parser, identical permission model, identical
 // exit-code semantics. The temp file is removed on drop.
            if let Some(expr) = eval {
                let tmp =
                    commands::file::synthesize_script_temp(&format!("print({});\n", expr), "eval")
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
 // The inner `run_script_interpreted` /
 // `run_native_compilation` paths print their own
 // `Running <file> (interpreter|cached VBC|aot)`
 // status with the cache-state-aware label.
 // Printing here too would emit a duplicate
 // `Running` line on every invocation.
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
            let resolved = tier::resolve(interp, aot, tier_override.as_ref(), tier::Tier::Aot)?;
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
            let resolved = tier::resolve(interp, aot, tier_override.as_ref(), tier::Tier::Aot)?;
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
                PathTarget::Project => commands::check::execute(workspace, false, false),
            }
        }
        Commands::CheckProof {
            file,
            meta_mode,
            meta_lift,
        } => {
 // Meta-mode dispatch (#158 V2): `--meta-mode` (no value) →
 // lift=1; `--meta-lift N` → lift=N; default → lift=0.
            let lift = meta_lift.unwrap_or(if meta_mode { 1 } else { 0 });
            commands::check_proof::execute_with_universe_lift(file.as_str(), lift)
        }
        Commands::ElaborateProof { file, output_dir } => commands::elaborate_proof::execute(
            file.as_str(),
            output_dir.as_ref().map(|s| s.as_str()),
        ),
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
                    || {
                        CliError::InvalidArgument(format!(
                            "unknown --severity `{}` (expected: error|warn|info|hint)",
                            level
                        ))
                    },
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
        } => commands::playbook::execute(commands::playbook::PlaybookOptions {
            file: file.as_ref().map(|s| s.as_str()),
            tier,
            vim_mode: vim,
            preload: preload.as_ref().map(|s| s.as_str()),
            tutorial,
            profile,
            export: export.as_ref().map(|s| s.as_str()),
            no_color,
        }),
        Commands::PlaybookConvert(convert_cmd) => match convert_cmd {
            PlaybookConvertCommands::ToScript {
                input,
                output,
                include_outputs,
            } => commands::playbook::export_to_script(
                input.as_str(),
                output.as_ref().map(|s| s.as_str()),
                include_outputs,
            ),
            PlaybookConvertCommands::FromScript { input, output } => {
                commands::playbook::import_from_script(
                    input.as_str(),
                    output.as_ref().map(|s| s.as_str()),
                )
            }
        },
        Commands::Version { verbose } => commands::version::execute(verbose),
        Commands::VbcVersion { archive, raw } => commands::vbc_version::execute(&archive, raw),
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
                return commands::verify_ladder::run_verify_ladder(ladder_format.as_str());
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
                unsafe {
                    std::env::set_var("VERUM_LSP_MODE", "1");
                }
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
                    std::env::set_var("VERUM_DUMP_SMT_DIR", dir.display().to_string());
                }
            }
            if solver_protocol {
                unsafe {
                    std::env::set_var("VERUM_SOLVER_PROTOCOL", "1");
                }
            }
            if let Some(ref smt_file) = check_smt_formula {
 // --check-smt-formula short-circuits: read the
 // file, dispatch to the configured solver, print
 // sat/unsat/unknown. Skips the whole verify
 // pipeline because the input is raw SMT-LIB — the
 // Verum AST / type-checker / VC generator don't
 // need to run.
                return commands::smt_check::run(smt_file, solver.as_str(), timeout);
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
            tracing::debug!("smt_proof_preference = {}", smt_proof_preference.as_str());
            let _ = smt_proof_preference;

            let budget_duration = match budget.as_deref() {
                None => None,
                Some(raw) => match commands::verify::parse_duration(raw) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        eprintln!("{} invalid --budget: {}", "error:".red().bold(), e);
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
                    distributed_cache_trust: None,
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
        Commands::Dap {
            transport,
            port,
            feature_overrides,
        } => {
            feature_overrides::install(feature_overrides);

 // Gate on [debug].dap_enabled. Resolve from the project
 // manifest if present; otherwise defaults apply (enabled).
 // A missing manifest is not an error — `verum dap` can run
 // outside a Verum project (e.g. for stand-alone IDE use).
            let (dap_enabled, default_port) = match config::Manifest::find_manifest_dir().ok() {
                Some(dir) => {
                    let path = config::Manifest::manifest_path(&dir);
                    let mut m = config::Manifest::from_file(&path).unwrap_or_else(|_| {
                        config::create_default_manifest(
                            "scratch",
                            false,
                            config::LanguageProfile::Application,
                        )
                    });
                    feature_overrides::apply_global(&mut m)?;
 // Surface inert DebugConfig fields not yet wired
 // through to the DAP server. `dap_enabled` and
 // `port` reach the dispatch logic below; the
 // remaining three (`step_granularity`,
 // `inspect_depth`, `show_erased_proofs`) flow
 // from the manifest into LanguageFeatures but
 // verum_dap doesn't consult them at session
 // setup. Trace the values at the dispatch entry
 // so embedders writing
 // `[debug].step_granularity = "instruction"`
 // see the value was observed at the CLI
 // boundary, gated on any non-default value.
                    if m.debug.step_granularity.as_str() != "statement"
                        || m.debug.inspect_depth != 8
                        || m.debug.show_erased_proofs
                    {
                        tracing::debug!(
                            "dap dispatch: step_granularity={:?}, inspect_depth={}, \
                                 show_erased_proofs={} — these fields land on the manifest \
                                 but verum_dap does not yet consult them at session setup; \
                                 forward-looking infra",
                            m.debug.step_granularity.as_str(),
                            m.debug.inspect_depth,
                            m.debug.show_erased_proofs,
                        );
                    }
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
        Commands::Lsp {
            transport,
            port,
            feature_overrides,
        } => {
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
        } => commands::proof_draft::run_proof_draft(&theorem, &goal, &lemma, max, &format),
        Commands::ProofRepair {
            kind,
            field,
            max,
            format,
        } => commands::proof_repair::run_proof_repair(&kind, &field, max, &format),
        Commands::ForeignImport {
            from,
            file,
            out,
            format,
        } => commands::foreign_import::run_import(&from, &file, out.as_ref(), &format),
        Commands::Cubical { sub } => match sub {
            CubicalSub::Primitives { category, output } => {
                commands::cubical::run_primitives(category.as_deref(), &output)
            }
            CubicalSub::Explain { name, output } => commands::cubical::run_explain(&name, &output),
            CubicalSub::Rules { output } => commands::cubical::run_rules(&output),
            CubicalSub::Face { formula, output } => commands::cubical::run_face(&formula, &output),
        },
        Commands::CogRegistry { sub } => match sub {
            CogRegistrySub::Publish {
                manifest,
                root,
                registry_id,
                output,
            } => {
                commands::cog_registry::run_publish(&manifest, root.as_ref(), &registry_id, &output)
            }
            CogRegistrySub::Lookup {
                name,
                version,
                root,
                registry_id,
                output,
            } => commands::cog_registry::run_lookup(
                &name,
                &version,
                root.as_ref(),
                &registry_id,
                &output,
            ),
            CogRegistrySub::Search {
                name,
                paper_doi,
                framework,
                theorem,
                require_attestation,
                root,
                registry_id,
                output,
            } => commands::cog_registry::run_search(
                name.as_deref(),
                paper_doi.as_deref(),
                framework.as_deref(),
                theorem.as_deref(),
                require_attestation.as_deref(),
                root.as_ref(),
                &registry_id,
                &output,
            ),
            CogRegistrySub::Verify {
                name,
                version,
                root,
                registry_id,
                output,
            } => commands::cog_registry::run_verify(
                &name,
                &version,
                root.as_ref(),
                &registry_id,
                &output,
            ),
            CogRegistrySub::Consensus {
                name,
                version,
                mirror,
                output,
            } => commands::cog_registry::run_consensus(&name, &version, &mirror, &output),
            CogRegistrySub::SeedDemo { output } => commands::cog_registry::run_seed_demo(&output),
        },
        Commands::CertReplay { sub } => match sub {
            CertReplaySub::Replay {
                backend,
                cert,
                format,
                theory,
                conclusion,
                body,
                output,
            } => commands::cert_replay::run_replay(
                &backend,
                cert.as_ref(),
                &format,
                &theory,
                &conclusion,
                &body,
                &output,
            ),
            CertReplaySub::CrossCheck {
                backend,
                cert,
                format,
                theory,
                conclusion,
                body,
                require_consensus,
                output,
            } => commands::cert_replay::run_cross_check(
                &backend,
                cert.as_ref(),
                &format,
                &theory,
                &conclusion,
                &body,
                require_consensus,
                &output,
            ),
            CertReplaySub::Formats { output } => commands::cert_replay::run_formats(&output),
            CertReplaySub::Backends { output } => commands::cert_replay::run_backends(&output),
        },
        Commands::Benchmark { sub } => match sub {
            BenchmarkSub::Run {
                system,
                suite_name,
                theorem,
                format,
            } => commands::benchmark::run_run(&system, &suite_name, &theorem, &format),
            BenchmarkSub::Compare {
                system,
                suite_name,
                theorem,
                format,
            } => commands::benchmark::run_compare(&system, &suite_name, &theorem, &format),
            BenchmarkSub::Metrics { format } => commands::benchmark::run_metrics(&format),
        },
        Commands::ProofRepl { sub } => match sub {
            ProofReplSub::Batch {
                theorem,
                goal,
                lemma,
                commands,
                cmd,
                format,
            } => commands::proof_repl::run_batch_cli(
                &theorem,
                &goal,
                &lemma,
                commands.as_ref(),
                &cmd,
                &format,
            ),
            ProofReplSub::Tree {
                theorem,
                goal,
                lemma,
                apply,
            } => commands::proof_repl::run_tree_cli(&theorem, &goal, &lemma, &apply),
        },
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
            TacticSub::Explain { name, format } => commands::tactic::run_explain(&name, &format),
            TacticSub::Laws { format } => commands::tactic::run_laws(&format),
        },
        Commands::Audit {
            details,
            direct_only,
            framework_axioms,
            kernel_rules,
            kernel_recheck,
            kernel_soundness,
            kernel_v0_roster,
            dependent_theorems,
            codegen_attestation,
            differential_kernel,
            differential_kernel_fuzz,
            reflection_tower,
            arch_discharges,
            counterfactual,
            adjunctions,
            yoneda,
            arch_coverage,
            arch_corpus,
            bridge_discharge,
            stdlib_layers,
            proof_archive,
            ladder_monotonicity,
            cross_format_roundtrip,
            docker,
            bundle,
            signatures,
            proof_term_library,
            soundness_iou,
            apply_graph,
            epsilon,
            coord,
            no_coord,
            hygiene,
            hygiene_strict,
            owl2_classify,
            framework_conflicts,
            foundation_profiles,
            accessibility,
            round_trip,
            coherent,
            proof_honesty,
            bridge_admits,
            framework_soundness,
            coord_consistency,
            htt_roadmap,
            ar_roadmap,
            cross_format,
            kernel_intrinsics,
            kernel_discharged_axioms,
            verify_ladder,
            manifest_coverage,
            mls_coverage,
            format,
        } => {
            let output_format = match format.as_str() {
                "plain" => commands::audit::AuditFormat::Plain,
                "json" => commands::audit::AuditFormat::Json,
                other => {
                    return Err(CliError::InvalidArgument(
                        format!("--format must be 'plain' or 'json', got '{}'", other).into(),
                    ));
                }
            };
            if kernel_rules {
                commands::audit::audit_kernel_rules(output_format)
            } else if kernel_recheck {
                commands::audit::audit_kernel_recheck_with_format(output_format)
            } else if kernel_soundness {
                commands::audit::audit_kernel_soundness_with_format(output_format)
            } else if kernel_v0_roster {
                commands::audit::audit_kernel_v0_roster_with_format(output_format)
            } else if let Some(axiom_name) = dependent_theorems.as_deref() {
                commands::audit::audit_dependent_theorems_with_format(
                    axiom_name,
                    output_format,
                )
            } else if codegen_attestation {
                commands::audit::audit_codegen_attestation_with_format(output_format)
            } else if differential_kernel {
                commands::audit::audit_differential_kernel_with_format(output_format)
            } else if differential_kernel_fuzz {
                commands::audit::audit_differential_kernel_fuzz_with_format(output_format)
            } else if reflection_tower {
                commands::audit::audit_reflection_tower_with_format(output_format)
            } else if arch_discharges {
                commands::audit::audit_arch_discharges_with_format(output_format)
            } else if counterfactual {
                commands::audit::audit_counterfactual_with_format(output_format)
            } else if adjunctions {
                commands::audit::audit_adjunctions_with_format(output_format)
            } else if yoneda {
                commands::audit::audit_yoneda_with_format(output_format)
            } else if arch_coverage {
                commands::audit::audit_arch_coverage_with_format(output_format)
            } else if arch_corpus {
                commands::audit::audit_arch_corpus_with_format(output_format)
            } else if bridge_discharge {
                commands::audit::audit_bridge_discharge_with_format(output_format)
            } else if ladder_monotonicity {
                commands::audit::audit_ladder_monotonicity_with_format(output_format)
            } else if cross_format_roundtrip {
                commands::audit::audit_cross_format_roundtrip_with_backend(
                    output_format,
                    if docker {
                        verum_smt::cross_format_runner::CheckerBackend::Docker
                    } else {
                        verum_smt::cross_format_runner::CheckerBackend::from_env()
                    },
                )
            } else if bundle {
                commands::audit::audit_bundle_with_format(output_format)
            } else if signatures {
                commands::audit::audit_signatures_with_format(output_format)
            } else if proof_term_library {
                commands::audit::audit_proof_term_library_with_format(output_format)
            } else if soundness_iou {
                commands::audit::audit_soundness_iou_with_format(output_format)
            } else if apply_graph {
                commands::audit::audit_apply_graph_with_format(output_format)
            } else if proof_archive {
                commands::audit::audit_proof_archive_with_format(output_format)
            } else if stdlib_layers {
                commands::audit::audit_stdlib_layers_with_format(output_format)
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
            } else if foundation_profiles {
                commands::audit::audit_foundation_profiles_with_format(output_format)
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
            } else if cross_format {
                commands::audit::audit_cross_format(output_format)
            } else if kernel_intrinsics {
                commands::audit::audit_kernel_intrinsics(output_format)
            } else if kernel_discharged_axioms {
                commands::audit::audit_kernel_discharged_axioms(output_format)
            } else if verify_ladder {
                commands::audit::audit_verify_ladder(output_format)
            } else if manifest_coverage {
                commands::audit::audit_manifest_coverage(output_format)
            } else if mls_coverage {
                commands::audit::audit_mls_coverage(output_format)
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
                    let coord_result = commands::audit::audit_coord_with_format(output_format);
 // Surface either failure; prefer the dep-audit
 // error for backwards compatibility.
                    dep_result.and(coord_result)
                } else {
                    dep_result
                }
            }
        }
        Commands::Export {
            to,
            output,
            with_provenance,
        }
        | Commands::ExportProofs {
            to,
            output,
            with_provenance,
        } => {
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
        Commands::Import {
            from,
            input,
            output,
        } => {
            let format = commands::import::ImportFormat::parse(&from)?;
            commands::import::run(commands::import::ImportOptions {
                format,
                input,
                output,
            })
        }
        Commands::Extract { input, output } => {
            commands::extract::run(commands::extract::ExtractOptions { input, output })
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

        Commands::Stdlib { sub } => match sub {
            StdlibSub::Precompile {
                stdlib_path,
                out,
                target,
                verbose,
            } => commands::stdlib_precompile::run(stdlib_path, out, target, verbose),
        },

        Commands::Cog { sub } => match sub {
            CogSub::Precompile {
                cog_dir,
                out,
                target,
                verbose,
            } => commands::cog_precompile::run(cog_dir, out, target, verbose),
            CogSub::Reproduce {
                spec,
                source_dir,
                source_tar,
                reference_vbca,
                registry,
                keep_workdir,
                verbose,
            } => commands::cog_reproduce::run(
                spec,
                source_dir,
                source_tar,
                reference_vbca,
                registry,
                keep_workdir,
                verbose,
            ),
        },

        Commands::SmtInfo { json } => {
            #[cfg(feature = "verification")]
            {
                commands::smt_info::execute(json).map_err(|e| CliError::Custom(e.to_string()))
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
            commands::smt_stats::execute(json, reset).map_err(|e| CliError::Custom(e.to_string()))
        }

        Commands::Config { command } => match command {
            ConfigCommands::Show {
                json,
                feature_overrides,
            } => {
                feature_overrides::install(feature_overrides);
                commands::config::execute(json).map_err(|e| CliError::Custom(e.to_string()))
            }
            ConfigCommands::Validate { feature_overrides } => {
                feature_overrides::install(feature_overrides);
                commands::config::validate().map_err(|e| CliError::Custom(e.to_string()))
            }
        },
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "verum", &mut std::io::stdout());
            Ok(())
        } // NOTE: stdlib command removed - stdlib is now compiled automatically via cache system
        Commands::Arch { cmd } => match cmd {
            ArchCommands::Explain { cog, format } => {
                let output_format = match format.as_str() {
                    "plain" => commands::audit::AuditFormat::Plain,
                    "json" => commands::audit::AuditFormat::Json,
                    other => {
                        return Err(CliError::InvalidArgument(
                            format!("--format must be 'plain' or 'json', got '{}'", other).into(),
                        ));
                    }
                };
                commands::audit::arch_explain(cog.as_deref(), output_format)
            }
            ArchCommands::Catalog {
                format,
                mtac_only,
                season,
            } => {
                let output_format = match format.as_str() {
                    "plain" => commands::audit::AuditFormat::Plain,
                    "json" => commands::audit::AuditFormat::Json,
                    other => {
                        return Err(CliError::InvalidArgument(
                            format!("--format must be 'plain' or 'json', got '{}'", other).into(),
                        ));
                    }
                };
                commands::audit::arch_catalog(output_format, mtac_only, season)
            }
            ArchCommands::Check {
                file,
                format,
                strict,
            } => {
                let output_format = match format.as_str() {
                    "plain" => commands::audit::AuditFormat::Plain,
                    "json" => commands::audit::AuditFormat::Json,
                    other => {
                        return Err(CliError::InvalidArgument(
                            format!("--format must be 'plain' or 'json', got '{}'", other).into(),
                        ));
                    }
                };
                commands::audit::arch_check(&file, output_format, strict)
            }
        },
    }
}
