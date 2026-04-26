// Lint command - static analysis for code quality issues.
// Checks for unused imports, dead code, style violations, and common mistakes.
// Implements 10 Verum-specific lint rules using fast text-based scanning.

use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use verum_common::{List, Text};
use walkdir::WalkDir;

/// Lint severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LintLevel {
    Error,
    Warning,
    Info,
    Hint,
    /// Suppressed — same as `disabled`. Lets `[lint.severity]` set
    /// `rule = "off"` per-rule.
    Off,
}

impl LintLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LintLevel::Error => "error",
            LintLevel::Warning => "warn",
            LintLevel::Info => "info",
            LintLevel::Hint => "hint",
            LintLevel::Off => "off",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "error" | "deny" => Some(LintLevel::Error),
            "warn" | "warning" => Some(LintLevel::Warning),
            "info" => Some(LintLevel::Info),
            "hint" => Some(LintLevel::Hint),
            "off" | "allow" | "disabled" => Some(LintLevel::Off),
            _ => None,
        }
    }
}

/// Verum-specific lint rule
#[derive(Debug, Clone)]
struct LintRule {
    name: &'static str,
    level: LintLevel,
    description: &'static str,
    category: LintCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintCategory {
    Performance,
    Safety,
    Style,
    Verification,
}

/// Lint groups — opt-in rule families exposed via
/// `extends = "verum::<name>"` in the manifest.
///
/// Group membership is computed from the LintRules table on the
/// fly so adding a rule to a group doesn't require a table-wide
/// edit. The deliberate trade-off is that groups become slightly
/// less explicit — the recipe for membership is in `lint_groups()`,
/// not the rule definition.
pub fn lint_groups() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut out: Vec<(&'static str, Vec<&'static str>)> = Vec::new();

    // verum::correctness — the can't-disable layer. Errors only.
    // What you'd ship in `extends = "minimal"` if you wanted the
    // bare minimum that still catches actual bugs.
    let correctness: Vec<&'static str> = LINT_RULES
        .iter()
        .filter(|r| matches!(r.level, LintLevel::Error))
        .map(|r| r.name)
        .collect();
    out.push(("verum::correctness", correctness));

    // verum::strict — everything safety + verification + errors,
    // promoted to error severity in the preset.
    let strict: Vec<&'static str> = LINT_RULES
        .iter()
        .filter(|r| {
            matches!(
                r.category,
                LintCategory::Safety | LintCategory::Verification
            ) || matches!(r.level, LintLevel::Error)
        })
        .map(|r| r.name)
        .collect();
    out.push(("verum::strict", strict));

    // verum::pedantic — every hint-level rule. Useful for
    // refactor-the-codebase sessions when you want to see every
    // suggestion the linter can offer.
    let pedantic: Vec<&'static str> = LINT_RULES
        .iter()
        .filter(|r| matches!(r.level, LintLevel::Hint))
        .map(|r| r.name)
        .collect();
    out.push(("verum::pedantic", pedantic));

    // verum::nursery — explicitly-listed experimental rules. Off
    // by default in every other preset; opt-in via this group.
    let nursery: Vec<&'static str> = vec![
        "inconsistent-public-doc",
        "unused-public",
        "mount-cycle-via-stdlib",
    ];
    out.push(("verum::nursery", nursery));

    // verum::deprecated — placeholder for the rule deprecation
    // framework (#54). Empty today; populated as rules are
    // deprecated.
    out.push(("verum::deprecated", Vec::new()));

    out
}

/// Lint issue found in code
#[derive(Debug, Clone)]
pub struct LintIssue {
    pub rule: &'static str,
    pub level: LintLevel,
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub suggestion: Option<Text>,
    pub fixable: bool,
}

/// Verum-specific lint rules
const LINT_RULES: &[LintRule] = &[
    LintRule {
        name: "unchecked-refinement",
        level: LintLevel::Warning,
        description: "Function with refinement types lacks @verify annotation",
        category: LintCategory::Verification,
    },
    LintRule {
        name: "missing-context-decl",
        level: LintLevel::Error,
        description: "Function uses context without declaration in scope",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "unused-import",
        level: LintLevel::Warning,
        description: "Unused mount/import statement",
        category: LintCategory::Style,
    },
    LintRule {
        name: "unnecessary-heap",
        level: LintLevel::Warning,
        description: "Heap allocation for small type that could be stack-allocated",
        category: LintCategory::Performance,
    },
    LintRule {
        name: "missing-error-context",
        level: LintLevel::Warning,
        description: "Error propagation without context",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "large-copy",
        level: LintLevel::Warning,
        description: "Large struct passed by value instead of by reference",
        category: LintCategory::Performance,
    },
    LintRule {
        name: "unused-result",
        level: LintLevel::Warning,
        description: "Function result value is unused",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "missing-cleanup",
        level: LintLevel::Warning,
        description: "Type with resource management lacks Cleanup protocol",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "deprecated-syntax",
        level: LintLevel::Error,
        description: "Rust-ism or deprecated syntax used",
        category: LintCategory::Style,
    },
    LintRule {
        name: "cbgr-hotspot",
        level: LintLevel::Info,
        description: "Tight loop with reference dereferences could use &checked",
        category: LintCategory::Performance,
    },
    // ── Extended rules (Verum-specific, beyond Clippy) ──
    LintRule {
        name: "mutable-capture-in-spawn",
        level: LintLevel::Error,
        description: "Mutable variable captured by spawn closure (data race risk)",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "unbounded-channel",
        level: LintLevel::Warning,
        description: "Channel.new() without capacity limit can cause OOM",
        category: LintCategory::Performance,
    },
    LintRule {
        name: "missing-timeout",
        level: LintLevel::Warning,
        description: "Blocking operation without timeout (recv, await, join)",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "redundant-clone",
        level: LintLevel::Warning,
        description: "Clone on value that is not used after this point",
        category: LintCategory::Performance,
    },
    LintRule {
        name: "empty-match-arm",
        level: LintLevel::Warning,
        description: "Match arm with empty body or just unit ()",
        category: LintCategory::Style,
    },
    LintRule {
        name: "single-variant-match",
        level: LintLevel::Hint,
        description: "Match with single pattern could be `if let`",
        category: LintCategory::Style,
    },
    LintRule {
        name: "todo-in-code",
        level: LintLevel::Warning,
        description: "TODO/FIXME/HACK comment in production code",
        category: LintCategory::Style,
    },
    LintRule {
        name: "unsafe-ref-in-public",
        level: LintLevel::Warning,
        description: "Public function exposes &unsafe reference in API",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "missing-type-annotation",
        level: LintLevel::Hint,
        description: "Complex expression could benefit from explicit type annotation",
        category: LintCategory::Style,
    },
    LintRule {
        name: "shadow-binding",
        level: LintLevel::Info,
        description: "Variable shadows an existing binding in outer scope",
        category: LintCategory::Style,
    },
    // ── AST-driven passes (Phase B.1+) — implementations live in
    //    `commands/lint_engine.rs`. They appear in --list-rules,
    //    --explain, --validate-config exactly like text-scan rules. ──
    LintRule {
        name: "redundant-refinement",
        level: LintLevel::Hint,
        description: "Refinement predicate evaluates to a tautology — base type would do",
        category: LintCategory::Verification,
    },
    LintRule {
        name: "empty-refinement-bound",
        level: LintLevel::Error,
        description: "Refinement bound has no inhabitants (e.g. `it > 100 && it < 50`)",
        category: LintCategory::Verification,
    },
    LintRule {
        name: "naming-convention",
        level: LintLevel::Warning,
        description: "Identifier doesn't match the project's [lint.naming] convention",
        category: LintCategory::Style,
    },
    // Phase C.1 — refinement-policy enforcement (off by default;
    // projects opt in via [lint.refinement_policy].* flags).
    LintRule {
        name: "unrefined-public-int",
        level: LintLevel::Warning,
        description:
            "Public fn parameter or return is Int/Text without a refinement — \
             tighten the type to express valid usage at the type level",
        category: LintCategory::Verification,
    },
    LintRule {
        name: "verify-implied-by-refinement",
        level: LintLevel::Warning,
        description:
            "Function uses refinement types but lacks @verify — \
             the type-level obligation will only be checked at runtime",
        category: LintCategory::Verification,
    },
    LintRule {
        name: "public-must-have-verify",
        level: LintLevel::Hint,
        description:
            "Public function lacks @verify(...) — declare its verification \
             strategy explicitly (runtime | static | formal | …)",
        category: LintCategory::Verification,
    },
    // Phase C.3 — context-policy enforcement (off by default; opt in
    // via [lint.context_policy.modules].<glob>).
    LintRule {
        name: "forbidden-context",
        level: LintLevel::Error,
        description:
            "Function uses a context (`using [X]`) that the project's \
             [lint.context_policy.modules] forbids in this module path",
        category: LintCategory::Safety,
    },
    // Phase B.4 — architecture layering + bans (off by default; opt
    // in via [lint.architecture.layers] and/or .bans).
    LintRule {
        name: "architecture-violation",
        level: LintLevel::Error,
        description:
            "`mount` crosses a layer boundary or matches an explicit ban — \
             the project's [lint.architecture] forbids this import",
        category: LintCategory::Style,
    },
    // Phase C.4 — CBGR-budget enforcement. Off by default; opt in via
    // [lint.cbgr_budgets].default_check_ns < 15 or per-module overrides.
    LintRule {
        name: "cbgr-budget-exceeded",
        level: LintLevel::Warning,
        description:
            "Managed CBGR reference (`&` / `&mut`) used in a module whose \
             [lint.cbgr_budgets].max_check_ns budget is below the static \
             per-deref cost — promote to `&checked` (0ns) or `&unsafe`",
        category: LintCategory::Performance,
    },
    // Phase C.6 — style ceilings (off by default; opt in via
    // [lint.style] config).
    LintRule {
        name: "max-line-length",
        level: LintLevel::Hint,
        description: "Source line exceeds [lint.style].max_line_length characters",
        category: LintCategory::Style,
    },
    LintRule {
        name: "max-fn-lines",
        level: LintLevel::Hint,
        description: "Function body exceeds [lint.style].max_fn_lines",
        category: LintCategory::Style,
    },
    LintRule {
        name: "max-fn-params",
        level: LintLevel::Hint,
        description: "Function takes more parameters than [lint.style].max_fn_params",
        category: LintCategory::Style,
    },
    LintRule {
        name: "max-match-arms",
        level: LintLevel::Hint,
        description: "match expression has more arms than [lint.style].max_match_arms",
        category: LintCategory::Style,
    },
    // Phase C.5 — documentation policy.
    LintRule {
        name: "public-must-have-doc",
        level: LintLevel::Hint,
        description:
            "Public item lacks a doc comment (`///`) — add one or set \
             [lint.documentation].public_must_have_doc = false",
        category: LintCategory::Style,
    },
    // Phase C.2 — capability-policy enforcement (off by default;
    // opt in via [lint.capability_policy].require_cap_for_*).
    LintRule {
        name: "unsafe-without-capability",
        level: LintLevel::Warning,
        description:
            "Function uses `unsafe { … }` but lacks @cap(...) — declare \
             the capability explicitly so the trust boundary is auditable",
        category: LintCategory::Safety,
    },
    LintRule {
        name: "ffi-without-capability",
        level: LintLevel::Warning,
        description:
            "FFI item (`@ffi` / `@extern`) lacks @cap(...) — declare \
             the foreign-boundary capability explicitly",
        category: LintCategory::Safety,
    },
    // Phase D — meta-rule for [[lint.custom]] entries with ast_match.
    // Per-user-rule diagnostics carry the user-chosen rule name; this
    // entry exists so `--list-rules` and `--explain` can describe the
    // mechanism for users who haven't yet authored their own rules.
    LintRule {
        name: "custom-ast-rule",
        level: LintLevel::Warning,
        description:
            "User-authored AST-pattern rule from [[lint.custom]] (Phase D) — \
             accepts kind = \"method_call|call|unsafe_block|attribute\" with \
             extra fields: method | path | name",
        category: LintCategory::Style,
    },
    // Cross-file rules — operate on the assembled corpus rather than
    // a single file at a time. Cycle detection and orphan detection
    // require the full mount graph; unused-public requires the full
    // identifier-reference set.
    LintRule {
        name: "circular-import",
        level: LintLevel::Error,
        description:
            "Module graph contains a cycle — module A mounts B, \
             B (transitively) mounts A. Break the cycle by extracting \
             the shared types into a leaf module.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "orphan-module",
        level: LintLevel::Hint,
        description:
            "File under src/ that no other corpus file mounts. \
             Excludes main.vr / lib.vr / mod.vr entry points.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "unused-public",
        level: LintLevel::Hint,
        description:
            "Public symbol whose name does not appear in any other file. \
             Heuristic — opt-in via [lint.rules.unused-public].enabled = true.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "unused-private",
        level: LintLevel::Hint,
        description:
            "Non-public symbol with no callers in its own file — \
             dead code that the type-checker doesn't catch because the \
             visibility is technically fine.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "dead-module",
        level: LintLevel::Hint,
        description:
            "File not reachable from any entry point (main.vr / lib.vr / mod.vr) \
             along the mount graph.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "inconsistent-public-doc",
        level: LintLevel::Hint,
        description:
            "Module exports K public symbols, M of them documented; fires \
             when 0 < M < K. Opt-in via [lint.rules.inconsistent-public-doc].enabled.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "mount-cycle-via-stdlib",
        level: LintLevel::Warning,
        description:
            "Module graph contains a back-edge through a stdlib path — \
             cycle hidden by re-export. Refactor to break the round trip.",
        category: LintCategory::Style,
    },
    LintRule {
        name: "pub-exports-unsafe",
        level: LintLevel::Warning,
        description:
            "Public symbol's signature mentions `&unsafe` or `unsafe fn` — \
             unsafe surface leaked across the project boundary.",
        category: LintCategory::Safety,
    },
];

pub fn execute(fix: bool, deny_warnings: bool) -> Result<()> {
    ui::step("Running Verum linter");
    println!();

    // Load full config (covers `disabled` + `severity_map` + `extends`).
    let cfg = load_full_lint_config();

    let mut all_issues = List::new();
    // Find all .vr files in src/ and core/
    let search_dirs: Vec<PathBuf> = ["src", "core"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    if search_dirs.is_empty() {
        return Err(CliError::Custom("No src/ or core/ directory found".into()));
    }

    let (parallel_issues, total_files) = lint_paths_parallel(&search_dirs, &cfg);
    for mut i in parallel_issues {
        if let Some(lvl) = cfg.effective_level(i.rule, i.level) {
            i.level = lvl;
            all_issues.push(i);
        }
    }

    // Group issues by severity
    let mut errors = List::new();
    let mut warnings = List::new();
    let mut info_issues = List::new();
    let mut hints = List::new();

    for issue in &all_issues {
        match issue.level {
            LintLevel::Error => errors.push(issue),
            LintLevel::Warning => warnings.push(issue),
            LintLevel::Info => info_issues.push(issue),
            LintLevel::Hint => hints.push(issue),
            LintLevel::Off => continue,
        }
    }

    // Print issues
    for issue in &errors {
        print_issue(issue, deny_warnings);
    }

    for issue in &warnings {
        print_issue(issue, deny_warnings);
    }

    for issue in &info_issues {
        print_issue(issue, deny_warnings);
    }

    for issue in &hints {
        print_issue(issue, deny_warnings);
    }

    println!();

    // Summary
    let total_issues = all_issues.len();
    if total_issues == 0 {
        ui::success(&format!("No lint issues found in {} files", total_files));
        return Ok(());
    }

    println!("{}", "Lint Summary:".bold());
    println!("  Files checked: {}", total_files);

    if !errors.is_empty() {
        println!("  {}: {}", "Errors".red().bold(), errors.len());
    }
    if !warnings.is_empty() {
        println!("  {}: {}", "Warnings".yellow().bold(), warnings.len());
    }
    if !info_issues.is_empty() {
        println!("  {}: {}", "Info".blue().bold(), info_issues.len());
    }
    if !hints.is_empty() {
        println!("  {}: {}", "Hints".dimmed(), hints.len());
    }
    println!();

    // Count fixable issues
    let fixable_count = all_issues.iter().filter(|i| i.fixable).count();
    if fixable_count > 0 && !fix {
        println!(
            "{} issues can be auto-fixed with {}",
            fixable_count,
            "verum lint --fix".cyan().bold()
        );
    }

    // Apply fixes if requested
    if fix && fixable_count > 0 {
        ui::step(&format!("Applying {} automatic fixes", fixable_count));

        // Group fixable issues by file
        let mut fixes_by_file: HashMap<PathBuf, List<&LintIssue>> = HashMap::new();
        for issue in all_issues.iter().filter(|i| i.fixable) {
            fixes_by_file
                .entry(issue.file.clone())
                .or_default()
                .push(issue);
        }

        // Apply fixes to each file
        let mut fixed_count = 0;
        for (file_path, issues) in fixes_by_file {
            if let Ok(content) = fs::read_to_string(&file_path) {
                let fixed_content = apply_fixes(&content, &issues);
                if fixed_content != content && fs::write(&file_path, &fixed_content).is_ok() {
                    fixed_count += issues.len();
                }
            }
        }

        ui::success(&format!("Fixed {} issues", fixed_count));
    }

    // Determine exit status
    if !errors.is_empty() {
        Err(CliError::Custom(format!(
            "Found {} lint errors",
            errors.len()
        )))
    } else if deny_warnings && !warnings.is_empty() {
        Err(CliError::Custom(format!(
            "Found {} lint warnings (denied)",
            warnings.len()
        )))
    } else {
        Ok(())
    }
}

/// Check if file is a Verum source file (.vr).
fn is_verum_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "vr")
        .unwrap_or(false)
}

/// Load lint configuration from `.verum.toml` or `verum.toml`.
/// Returns set of disabled rule names.
/// User-configurable lint settings loaded from verum.toml [lint] section.
///
/// Supports:
/// - `disable = "rule1, rule2"` — Disable specific rules
/// - `deny = "rule1, rule2"` — Promote rules to errors (fail build)
/// - `allow = "rule1, rule2"` — Demote rules to allowed (no output)
/// - `warn = "rule1, rule2"` — Explicitly set rules to warning level
///
/// Custom pattern-based rules in [lint.custom] section:
/// ```toml
/// [lint]
/// disable = "todo-in-code"
/// deny = "unsafe-ref-in-public"
///
/// [[lint.custom]]
/// name = "no-print-in-lib"
/// pattern = "print("
/// message = "Use Logger context instead of print() in library code"
/// level = "warning"
/// paths = ["src/lib/"]
///
/// [[lint.custom]]
/// name = "no-unwrap-in-production"
/// pattern = ".unwrap()"
/// message = "Use ? or handle error explicitly"
/// level = "error"
/// exclude = ["tests/", "benches/"]
/// ```
pub struct LintConfig {
    /// `extends = "minimal" | "recommended" | "strict" | "relaxed"`.
    /// Applied before all other config. None = no preset (= legacy behaviour;
    /// rule levels come from `LINT_RULES.<rule>.level`).
    pub extends: Option<String>,
    /// Rules that are completely disabled (no output)
    pub disabled: HashSet<String>,
    /// Rules promoted to error level
    pub denied: HashSet<String>,
    /// Rules demoted to allow (suppressed)
    pub allowed: HashSet<String>,
    /// Rules explicitly set to warning level
    pub warned: HashSet<String>,
    /// Per-rule severity map from `[lint.severity]` — most precise
    /// per-rule control. Wins over the disabled/denied/allowed/warned
    /// lists when both reference the same rule.
    pub severity_map: HashMap<String, LintLevel>,
    /// Per-rule typed configuration tables from `[lint.rules.<name>]`.
    /// Stored as raw `toml::Value` — each rule deserialises its own
    /// config struct via `cfg.rule_config::<T>("rule-name")`.
    pub rules: HashMap<String, toml::Value>,
    /// Per-file overrides from `[lint.per_file_overrides]`. Each entry
    /// maps a glob pattern (matched against rel-path) to a per-file
    /// override block. Applied after [lint.severity] in the precedence
    /// stack — most-specific (longest pattern) wins.
    pub per_file_overrides: Vec<(String, FileOverride)>,
    /// Named profiles from `[lint.profiles.<name>]`. Selected via
    /// `verum lint --profile NAME` or `$VERUM_LINT_PROFILE`. Profile
    /// values are deep-merged on top of base config; CLI flags still
    /// override profile values.
    pub profiles: HashMap<String, toml::Value>,
    /// Custom pattern-based lint rules
    pub custom_rules: Vec<CustomLintRule>,
}

/// Per-file lint override block from `[lint.per_file_overrides]`.
#[derive(Debug, Clone, Default)]
pub struct FileOverride {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub warn: Vec<String>,
    pub disable: Vec<String>,
}

impl LintConfig {
    /// Typed accessor for a rule's `[lint.rules.<name>]` block.
    /// Returns `None` if the block is absent; an `Err`-shaped Result
    /// would force every call site to unwrap-or-default for what is
    /// genuinely "user didn't set this — use defaults" — `None` keeps
    /// rule code clean.
    ///
    /// Each rule documents its own `T` shape in
    /// `crates/verum_cli/src/commands/lint_engine.rs`.
    pub fn rule_config<T: serde::de::DeserializeOwned>(&self, rule: &str) -> Option<T> {
        let v = self.rules.get(rule)?.clone();
        T::deserialize(v).ok()
    }

    /// Effective level taking per-file overrides into account.
    /// Used by the `--severity` filter and the issue-emission paths
    /// to resolve `(rule, path, default_level) → Option<Level>`.
    ///
    /// Precedence (already-known stack, most-specific wins):
    ///   1. Per-file override matching `path`
    ///   2. severity_map
    ///   3. disabled / allowed / denied / warned lists
    ///   4. default_level
    pub fn effective_level_for_file(
        &self,
        rule_name: &str,
        path: &Path,
        default_level: LintLevel,
    ) -> Option<LintLevel> {
        // Per-file overrides — most specific (longest pattern) wins.
        let rel = path.to_string_lossy();
        let mut best: Option<&FileOverride> = None;
        let mut best_len: usize = 0;
        for (pat, ovr) in &self.per_file_overrides {
            if glob_path_match(pat, &rel) && pat.len() > best_len {
                best = Some(ovr);
                best_len = pat.len();
            }
        }
        if let Some(ovr) = best {
            if ovr.allow.iter().any(|n| n == rule_name)
                || ovr.disable.iter().any(|n| n == rule_name)
            {
                return None;
            }
            if ovr.deny.iter().any(|n| n == rule_name) {
                return Some(LintLevel::Error);
            }
            if ovr.warn.iter().any(|n| n == rule_name) {
                return Some(LintLevel::Warning);
            }
        }
        self.effective_level(rule_name, default_level)
    }
}

/// Glob match for file paths. Supports leading `**/` and trailing
/// `/**` (or `**`), plus `*` as a single-segment wildcard. Examples:
///
///   "tests/**"          ↔ "tests/x.vr", "tests/a/b.vr"     ✓
///   "core/intrinsics/*" ↔ "core/intrinsics/foo.vr"          ✓
///   "**/*.generated.vr" ↔ "src/foo/bar.generated.vr"        ✓
///
/// Heuristic implementation (no full glob crate); good enough for
/// the patterns documented in lint-configuration.md.
fn glob_path_match(pat: &str, path: &str) -> bool {
    if pat == path { return true; }
    // "x/**" → starts_with("x/") OR equals "x"
    if let Some(prefix) = pat.strip_suffix("/**") {
        return path == prefix
            || path.starts_with(&format!("{}/", prefix));
    }
    // "**/x.vr" → ends_with("/x.vr") OR equals "x.vr"
    if let Some(suffix) = pat.strip_prefix("**/") {
        if path == suffix { return true; }
        return path.ends_with(&format!("/{}", suffix));
    }
    // "**" alone matches anything
    if pat == "**" { return true; }
    // Single-segment wildcards: "x/*" → "x/" + non-empty + no extra /
    if let Some(prefix) = pat.strip_suffix("/*") {
        if !path.starts_with(&format!("{}/", prefix)) { return false; }
        let rest = &path[prefix.len() + 1..];
        return !rest.is_empty() && !rest.contains('/');
    }
    // Generic substring fallback for any pattern with `*`.
    if pat.contains('*') {
        let parts: Vec<&str> = pat.split('*').collect();
        let mut pos = 0usize;
        let bytes = path.as_bytes();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() { continue; }
            if i == 0 {
                if !path.starts_with(part) { return false; }
                pos = part.len();
            } else if i == parts.len() - 1 {
                if !path.ends_with(part) { return false; }
                if path.len() < pos + part.len() { return false; }
            } else {
                match path[pos..].find(part) {
                    Some(idx) => { pos = pos + idx + part.len(); let _ = bytes; }
                    None => return false,
                }
            }
        }
        return true;
    }
    false
}

/// Structured AST-pattern matcher for [[lint.custom]] rules.
/// Users author these in TOML for a Verum-aware alternative to the
/// regex `pattern` field. Patterns match a single AST shape; multiple
/// shapes produce one CustomLintRule each.
///
///   [[lint.custom]]
///   name = "no-unwrap-in-prod"
///   message = "use `?` or `expect(\"why\")` instead of unwrap()"
///   level = "warn"
///   [lint.custom.ast_match]            # one of:
///   kind = "method_call"
///   method = "unwrap"                  # specific method name
///
///   # OR
///   [lint.custom.ast_match]
///   kind = "call"
///   path = "core.unsafe.from_raw"      # dotted path to the callee
///
///   # OR
///   [lint.custom.ast_match]
///   kind = "attribute"
///   name = "deprecated"                # any item with @deprecated
///
///   # OR
///   [lint.custom.ast_match]
///   kind = "unsafe_block"              # any `unsafe { ... }` block
///
/// Multiple rules are independent — each runs as a separate AST walk
/// inside CustomAstRulesPass.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct AstMatchSpec {
    /// What AST shape to match. Recognised: "method_call", "call",
    /// "attribute", "unsafe_block".
    pub kind: String,
    /// For method_call: the method name (e.g. "unwrap").
    pub method: Option<String>,
    /// For call: the dotted callee path (e.g. "core.unsafe.from_raw").
    pub path: Option<String>,
    /// For attribute: the attribute name (without @).
    pub name: Option<String>,
}

/// A user-defined pattern-based lint rule from [lint.custom].
#[derive(Debug, Clone)]
pub struct CustomLintRule {
    /// Rule name (used in diagnostics and disable/deny)
    pub name: String,
    /// Text pattern to search for (substring match) — Phase D rules
    /// can leave this empty when `ast_match` is set.
    pub pattern: String,
    /// Diagnostic message
    pub message: String,
    /// Severity level
    pub level: LintLevel,
    /// Only check files under these paths (empty = all files)
    pub paths: Vec<String>,
    /// Exclude files under these paths
    pub exclude: Vec<String>,
    /// Optional fix suggestion
    pub suggestion: Option<String>,
    /// Phase D: structured AST matcher. When present, this rule
    /// fires via the AST-walking custom-rules pass instead of the
    /// regex/text-scan path. Strictly more precise — a `match: {
    /// kind = "method_call", method = "unwrap" }` cannot be fooled
    /// by `unwrap` appearing inside a string literal or comment.
    pub ast_match: Option<AstMatchSpec>,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            extends: None,
            disabled: HashSet::new(),
            denied: HashSet::new(),
            allowed: HashSet::new(),
            warned: HashSet::new(),
            severity_map: HashMap::new(),
            rules: HashMap::new(),
            per_file_overrides: Vec::new(),
            profiles: HashMap::new(),
            custom_rules: Vec::new(),
        }
    }
}

impl LintConfig {
    /// Get effective level for a rule, considering user overrides.
    /// Precedence (highest → lowest):
    ///   1. `[lint.severity].<rule>` map
    ///   2. `disabled` / `allowed` lists  → suppressed
    ///   3. `denied` list                 → error
    ///   4. `warned` list                 → warning
    ///   5. `default_level` (built-in or preset-provided)
    pub fn effective_level(&self, rule_name: &str, default_level: LintLevel) -> Option<LintLevel> {
        if let Some(level) = self.severity_map.get(rule_name).copied() {
            return if matches!(level, LintLevel::Off) {
                None
            } else {
                Some(level)
            };
        }
        if self.disabled.contains(rule_name) || self.allowed.contains(rule_name) {
            return None;
        }
        if self.denied.contains(rule_name) {
            return Some(LintLevel::Error);
        }
        if self.warned.contains(rule_name) {
            return Some(LintLevel::Warning);
        }
        Some(default_level)
    }
}

fn load_lint_config() -> HashSet<String> {
    let config = load_full_lint_config();
    config.disabled
}

fn load_full_lint_config() -> LintConfig {
    let mut config = LintConfig::default();

    // Layer 1: dedicated .verum/lint.toml takes priority over the
    // manifest, matching the design doc — projects that prefer to
    // keep verum.toml clean can drop a separate lint config without
    // touching the manifest.
    let dedicated = PathBuf::from(".verum/lint.toml");
    let manifest_candidates: &[&str] = &[".verum.toml", "verum.toml", "Verum.toml"];

    if dedicated.exists() {
        if let Ok(content) = fs::read_to_string(&dedicated) {
            // For .verum/lint.toml the file IS the [lint] block, so the
            // user can drop the section header.
            parse_lint_config_from_toml_v2(&content, &mut config, /* lint_root = */ true);
        }
    } else {
        for name in manifest_candidates {
            let path = PathBuf::from(name);
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    parse_lint_config_from_toml_v2(&content, &mut config, false);
                }
                break;
            }
        }
    }

    // Apply preset (extends) AFTER the file load so explicit per-rule
    // overrides win over preset defaults. The preset fills the
    // severity_map with built-in defaults appropriate for the chosen
    // preset; explicit `[lint.severity].<rule>` entries from the file
    // already populated the map and are NOT overwritten.
    if let Some(preset) = config.extends.clone() {
        apply_preset(&mut config, &preset);
    }

    config
}

/// Activate a named profile from `[lint.profiles.<name>]` on top of
/// the loaded config. Profile blocks accept the same keys as the
/// top-level `[lint]` block; non-empty values from the profile
/// override base values. Selected via `--profile NAME` or
/// `$VERUM_LINT_PROFILE`.
pub fn apply_profile(config: &mut LintConfig, name: &str) -> Result<()> {
    use toml::Value;
    let profile = match config.profiles.get(name).cloned() {
        Some(v) => v,
        None => {
            return Err(CliError::Custom(format!(
                "lint profile `{}` is not declared in [lint.profiles.*]",
                name
            )));
        }
    };
    let table = match profile.as_table() {
        Some(t) => t.clone(),
        None => return Ok(()),
    };

    // extends
    if let Some(Value::String(s)) = table.get("extends") {
        config.extends = Some(s.clone());
        apply_preset(config, s);
    }

    // disabled / denied / allowed / warned — extend, don't replace.
    fn extend_list(v: &Value) -> Vec<String> {
        match v {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            Value::String(s) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
    for k in &["disable", "disabled"] {
        if let Some(v) = table.get(*k) {
            config.disabled.extend(extend_list(v));
        }
    }
    for k in &["deny", "denied"] {
        if let Some(v) = table.get(*k) {
            config.denied.extend(extend_list(v));
        }
    }
    for k in &["allow", "allowed"] {
        if let Some(v) = table.get(*k) {
            config.allowed.extend(extend_list(v));
        }
    }
    for k in &["warn", "warned"] {
        if let Some(v) = table.get(*k) {
            config.warned.extend(extend_list(v));
        }
    }

    // [lint.profiles.<name>.severity] — profile severities override
    // base severities.
    if let Some(Value::Table(sev)) = table.get("severity") {
        for (rule, val) in sev {
            if let Some(level_str) = val.as_str() {
                if let Some(lvl) = LintLevel::parse(level_str) {
                    config.severity_map.insert(rule.clone(), lvl);
                }
            }
        }
    }

    Ok(())
}

/// Apply a built-in preset to the config's severity_map. Presets
/// fill in defaults for every rule; explicit `[lint.severity]`
/// entries from user config take precedence and are preserved.
fn apply_preset(config: &mut LintConfig, name: &str) {
    // `verum::<group>` form opts every rule in the group into the
    // severity_map at the rule's default level. Combined with a
    // base preset (`extends = "recommended"`), this enables a
    // family of nursery / pedantic / correctness rules without
    // changing the per-rule defaults.
    if let Some(group_name) = name.strip_prefix("verum::") {
        if let Some((_, members)) = lint_groups()
            .into_iter()
            .find(|(n, _)| *n == name || n.strip_prefix("verum::") == Some(group_name))
        {
            for rule_name in members {
                if config.severity_map.contains_key(rule_name) {
                    continue;
                }
                if let Some(rule) = LINT_RULES.iter().find(|r| r.name == rule_name) {
                    config.severity_map.insert(rule_name.to_string(), rule.level);
                }
            }
        }
        return;
    }

    let preset = match name {
        "minimal" => LintPreset::Minimal,
        "recommended" => LintPreset::Recommended,
        "strict" => LintPreset::Strict,
        "relaxed" => LintPreset::Relaxed,
        _ => {
            // Unknown preset is silently ignored at load-time;
            // --validate-config surfaces it.
            return;
        }
    };
    for rule in LINT_RULES {
        // Don't override explicit user choices.
        if config.severity_map.contains_key(rule.name) {
            continue;
        }
        let preset_level = preset.level_for(rule);
        config.severity_map.insert(rule.name.to_string(), preset_level);
    }
}

#[derive(Debug, Clone, Copy)]
enum LintPreset {
    /// Hard errors only — `deprecated-syntax`, `missing-context-decl`,
    /// `mutable-capture-in-spawn`. Everything else is `Off`. For
    /// porting legacy code without a flood of warnings.
    Minimal,
    /// Default. All Safety + Verification rules at their built-in
    /// level; Performance / Style at warn / info / hint.
    Recommended,
    /// CI-grade. Everything `recommended` ships, plus warnings
    /// promoted to errors for high-stakes categories.
    Strict,
    /// IDE-only suggestions. Errors stay errors, warnings → info,
    /// info → hint.
    Relaxed,
}

impl LintPreset {
    fn level_for(self, rule: &LintRule) -> LintLevel {
        match self {
            LintPreset::Minimal => match rule.level {
                LintLevel::Error => LintLevel::Error,
                _ => LintLevel::Off,
            },
            LintPreset::Recommended => rule.level,
            LintPreset::Strict => match (rule.level, rule.category) {
                (LintLevel::Error, _) => LintLevel::Error,
                (LintLevel::Warning, LintCategory::Safety)
                | (LintLevel::Warning, LintCategory::Verification) => LintLevel::Error,
                (LintLevel::Warning, _) => LintLevel::Warning,
                (LintLevel::Info, _) => LintLevel::Info,
                (LintLevel::Hint, _) => LintLevel::Hint,
                (LintLevel::Off, _) => LintLevel::Off,
            },
            LintPreset::Relaxed => match rule.level {
                LintLevel::Error => LintLevel::Error,
                LintLevel::Warning => LintLevel::Info,
                LintLevel::Info => LintLevel::Hint,
                LintLevel::Hint => LintLevel::Hint,
                LintLevel::Off => LintLevel::Off,
            },
        }
    }
}

/// `toml`-crate-based parser. Replaces the line-based parser for
/// new-form config (severity map, extends preset). Old-form keys
/// (`disabled`, `denied`, `allowed`, `warned` lists at the top of
/// `[lint]`) and `[[lint.custom]]` arrays remain supported via the
/// same parser for backwards compatibility.
///
/// `lint_root = true` means the parsed file IS the `[lint]` block
/// directly (used for `.verum/lint.toml`).
fn parse_lint_config_from_toml_v2(content: &str, config: &mut LintConfig, lint_root: bool) {
    use toml::Value;
    let parsed: Value = match toml::from_str::<Value>(content) {
        Ok(v) => v,
        Err(_) => return, // best-effort
    };
    let lint_table = if lint_root {
        match parsed {
            Value::Table(t) => Value::Table(t),
            _ => return,
        }
    } else {
        match parsed.get("lint") {
            Some(v) => v.clone(),
            None => match parsed.get("linter") {
                Some(v) => v.clone(),
                None => return,
            },
        }
    };
    let lint = match lint_table.as_table() {
        Some(t) => t,
        None => return,
    };

    // extends preset
    if let Some(Value::String(s)) = lint.get("extends") {
        config.extends = Some(s.clone());
    }

    // disabled/denied/allowed/warned — accept array OR comma-separated string
    fn extract_list(v: &Value) -> Vec<String> {
        match v {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            Value::String(s) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
    for key in &["disable", "disabled"] {
        if let Some(v) = lint.get(*key) {
            config.disabled.extend(extract_list(v));
        }
    }
    for key in &["deny", "denied"] {
        if let Some(v) = lint.get(*key) {
            config.denied.extend(extract_list(v));
        }
    }
    for key in &["allow", "allowed"] {
        if let Some(v) = lint.get(*key) {
            config.allowed.extend(extract_list(v));
        }
    }
    for key in &["warn", "warned"] {
        if let Some(v) = lint.get(*key) {
            config.warned.extend(extract_list(v));
        }
    }

    // [lint.rules.<name>] — per-rule typed configuration tables.
    // Each rule's block is stored raw as a `toml::Value`; rule code
    // deserialises its own typed config via `LintConfig::rule_config`.
    if let Some(Value::Table(rules)) = lint.get("rules") {
        for (rule_name, val) in rules {
            // Wrap non-tables (rare) so the rule can still attempt
            // to deserialize a unit/empty config. In practice every
            // documented rule expects a Table.
            config.rules.insert(rule_name.clone(), val.clone());
        }
    }

    // Synthetic rule keys — section blocks that drive multi-rule
    // policy enforcement. Each block lands under one synthetic key
    // so the rules consuming them go through the same
    // `cfg.rule_config::<T>` path as every other rule.
    let synthetic_keys = [
        ("naming",                "naming-convention"),     // Phase B.3
        ("refinement_policy",     "refinement-policy"),     // Phase C.1
        ("capability_policy",     "capability-policy"),     // Phase C.2
        ("context_policy",        "context-policy"),        // Phase C.3
        ("cbgr_budgets",          "cbgr-budgets"),          // Phase C.4
        ("verification_policy",   "verification-policy"),   // Phase C.5
        ("documentation",         "documentation-policy"),  // Phase C.5
        ("style",                 "style-policy"),          // Phase C.6
        ("architecture",          "architecture-policy"),   // Phase B.4
    ];
    for (toml_key, rule_key) in synthetic_keys {
        if let Some(v) = lint.get(toml_key) {
            config.rules.insert(rule_key.to_string(), v.clone());
        }
    }

    // [lint.per_file_overrides] — file-glob → allow/deny/warn/disable.
    if let Some(Value::Table(over)) = lint.get("per_file_overrides") {
        for (pat, val) in over {
            let mut fov = FileOverride::default();
            if let Value::Table(t) = val {
                for (k, v) in t {
                    let list = extract_list(v);
                    match k.as_str() {
                        "allow" => fov.allow.extend(list),
                        "deny" => fov.deny.extend(list),
                        "warn" => fov.warn.extend(list),
                        "disable" => fov.disable.extend(list),
                        _ => {}
                    }
                }
            }
            config.per_file_overrides.push((pat.clone(), fov));
        }
    }

    // [lint.profiles.<name>] — named profile blocks.
    if let Some(Value::Table(profs)) = lint.get("profiles") {
        for (name, val) in profs {
            config.profiles.insert(name.clone(), val.clone());
        }
    }

    // [lint.severity] — per-rule severity map
    if let Some(Value::Table(sev)) = lint.get("severity") {
        for (rule, val) in sev {
            if let Some(level_str) = val.as_str() {
                if let Some(lvl) = LintLevel::parse(level_str) {
                    config.severity_map.insert(rule.clone(), lvl);
                }
            }
        }
    }

    // [[lint.custom]] — array of tables
    if let Some(Value::Array(arr)) = lint.get("custom") {
        for entry in arr {
            if let Value::Table(t) = entry {
                let mut rule = CustomLintRule {
                    name: String::new(),
                    pattern: String::new(),
                    message: String::new(),
                    level: LintLevel::Warning,
                    paths: Vec::new(),
                    exclude: Vec::new(),
                    suggestion: None,
                    ast_match: None,
                };
                if let Some(s) = t.get("name").and_then(|v| v.as_str()) {
                    rule.name = s.to_string();
                }
                if let Some(s) = t.get("pattern").and_then(|v| v.as_str()) {
                    rule.pattern = s.to_string();
                }
                if let Some(s) = t.get("message").and_then(|v| v.as_str())
                    .or_else(|| t.get("description").and_then(|v| v.as_str()))
                {
                    rule.message = s.to_string();
                }
                if let Some(s) = t.get("level").and_then(|v| v.as_str())
                    .or_else(|| t.get("severity").and_then(|v| v.as_str()))
                {
                    if let Some(lvl) = LintLevel::parse(s) {
                        rule.level = lvl;
                    }
                }
                if let Some(s) = t.get("suggestion").and_then(|v| v.as_str())
                    .or_else(|| t.get("fix").and_then(|v| v.as_str()))
                {
                    rule.suggestion = Some(s.to_string());
                }
                if let Some(v) = t.get("paths") {
                    rule.paths = extract_list(v);
                }
                if let Some(v) = t.get("exclude") {
                    rule.exclude = extract_list(v);
                }
                // Phase D: structured AST matcher (alternative to
                // the regex `pattern` field).
                if let Some(am) = t.get("ast_match") {
                    if let Ok(spec) = <AstMatchSpec as serde::Deserialize>::deserialize(am.clone()) {
                        if !spec.kind.is_empty() {
                            rule.ast_match = Some(spec);
                        }
                    }
                }
                // Accept rule when EITHER pattern OR ast_match is set.
                let has_pattern = !rule.pattern.is_empty();
                let has_ast = rule.ast_match.is_some();
                if !rule.name.is_empty() && (has_pattern || has_ast) {
                    config.custom_rules.push(rule);
                }
            }
        }
    }
}


/// Run custom lint rules against a file.
fn lint_custom_rules(path: &Path, content: &str, config: &LintConfig) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    let path_str = path.to_string_lossy();

    for rule in &config.custom_rules {
        // Check path filters
        if !rule.paths.is_empty() {
            let matches_path = rule.paths.iter().any(|p| path_str.contains(p.as_str()));
            if !matches_path { continue; }
        }
        if rule.exclude.iter().any(|p| path_str.contains(p.as_str())) {
            continue;
        }

        // Check if rule is disabled/allowed
        if config.disabled.contains(&rule.name) || config.allowed.contains(&rule.name) {
            continue;
        }

        // Effective level (deny overrides)
        let level = if config.denied.contains(&rule.name) {
            LintLevel::Error
        } else {
            rule.level
        };

        // Scan lines for pattern
        for (line_idx, line) in content.lines().enumerate() {
            if line.contains(&rule.pattern) {
                issues.push(LintIssue {
                    rule: Box::leak(rule.name.clone().into_boxed_str()),
                    level,
                    file: path.to_path_buf(),
                    line: line_idx + 1,
                    column: line.find(&rule.pattern).unwrap_or(0) + 1,
                    message: rule.message.clone(),
                    suggestion: rule.suggestion.as_ref().map(|s| Text::from(s.clone())),
                    fixable: rule.suggestion.is_some(),
                });
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Fix application
// ---------------------------------------------------------------------------

/// Apply automatic fixes to file content
fn apply_fixes(content: &str, issues: &List<&LintIssue>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result_lines: Vec<Option<String>> = lines.iter().map(|l| Some(l.to_string())).collect();

    for issue in issues.iter() {
        let idx = issue.line.wrapping_sub(1);
        if idx >= result_lines.len() {
            continue;
        }
        match issue.rule {
            "unused-import" => {
                // Remove the entire mount line
                result_lines[idx] = None;
            }
            "deprecated-syntax" => {
                if let Some(ref suggestion) = issue.suggestion {
                    if let Some(ref current) = result_lines[idx] {
                        let fixed = apply_deprecated_syntax_fix(current, suggestion);
                        result_lines[idx] = Some(fixed);
                    }
                }
            }
            "unnecessary-heap" => {
                // Replace Heap(literal) with the literal for small types
                if let Some(ref current) = result_lines[idx] {
                    let fixed = fix_unnecessary_heap(current);
                    result_lines[idx] = Some(fixed);
                }
            }
            "redundant-clone" => {
                if let Some(ref current) = result_lines[idx] {
                    result_lines[idx] = Some(fix_redundant_clone(current));
                }
            }
            "single-variant-match" => {
                // Single-line `match e { Pat(a) => body }` → `if let Pat(a) = e { body }`.
                // Only triggers on the easy single-line form to avoid
                // the multi-line block-rewrite hazard.
                if let Some(ref current) = result_lines[idx] {
                    if let Some(rewritten) = fix_single_variant_match_inline(current) {
                        result_lines[idx] = Some(rewritten);
                    }
                }
            }
            "empty-match-arm" => {
                // Drop arms that are `_ => ()` / `Pat => {}` / similar
                // empty bodies — let the match exhaustiveness check
                // surface any breakage.
                if let Some(ref current) = result_lines[idx] {
                    if is_empty_match_arm_line(current) {
                        result_lines[idx] = None;
                    }
                }
            }
            "redundant-refinement" => {
                // `Type{ true }` → `Type`. Strip the always-true predicate.
                if let Some(ref current) = result_lines[idx] {
                    result_lines[idx] = Some(fix_redundant_refinement(current));
                }
            }
            "shadow-binding" => {
                // Conservative fix: rename `let x =` → `let x2 =` on
                // the inner binding. Only applies to `let` / `let mut`
                // forms; downstream uses on the same line are NOT
                // rewritten (the user must update them or the code
                // will fail to compile, which is the desired signal).
                if let Some(ref current) = result_lines[idx] {
                    if let Some(rewritten) = fix_shadow_rename_inner(current) {
                        result_lines[idx] = Some(rewritten);
                    }
                }
            }
            "todo-in-code" => {
                // Auto-append a placeholder issue tag so the
                // require-issue-link convention is satisfied. The
                // user is expected to replace the `0000` with a real
                // tracker number; the next lint pass will keep
                // failing until they do.
                if let Some(ref current) = result_lines[idx] {
                    result_lines[idx] = Some(fix_todo_with_placeholder_issue(current));
                }
            }
            _ => {}
        }
    }

    let mut result = String::new();
    for line in result_lines {
        if let Some(l) = line {
            result.push_str(&l);
            result.push('\n');
        }
    }
    result
}

/// Apply a deprecated syntax fix based on the suggestion
fn apply_deprecated_syntax_fix(line: &str, suggestion: &str) -> String {
    // Suggestions are of the form "Use 'X' instead of 'Y'"
    // Extract the replacement pairs
    if suggestion.contains("'implement'") && suggestion.contains("'impl'") {
        return line.replace("impl ", "implement ");
    }
    if suggestion.contains("'type X is'") && suggestion.contains("'struct'") {
        // struct Name { -> type Name is {
        if let Some(pos) = line.find("struct ") {
            let after = &line[pos + 7..];
            if let Some(brace) = after.find('{') {
                let name = after[..brace].trim();
                let indent = &line[..pos];
                return format!("{}type {} is {{", indent, name);
            }
        }
    }
    if suggestion.contains("'List<T>'") && suggestion.contains("'Vec<T>'") {
        return line.replace("Vec<", "List<");
    }
    if suggestion.contains("without '!'") {
        // Remove ! from macro-style calls: println!(...) -> print(...)
        // The suggestion tells us the correct name
        return line
            .replace("println!(", "print(")
            .replace("format!(", "f\"")
            .replace("panic!(", "panic(")
            .replace("assert!(", "assert(")
            .replace("assert_eq!(", "assert_eq(");
    }
    line.to_string()
}

/// Fix unnecessary Heap allocation by removing the Heap() wrapper
fn fix_unnecessary_heap(line: &str) -> String {
    // Heap(42) -> 42, Heap(true) -> true, Heap(3.14) -> 3.14
    let mut result = line.to_string();
    // Simple pattern: Heap(literal)
    for small in &["Heap(true)", "Heap(false)"] {
        let replacement = &small[5..small.len() - 1]; // extract inner
        result = result.replace(small, replacement);
    }
    // For numeric literals, use a regex-like scan
    let mut output = String::new();
    let chars = result.chars().peekable();
    let mut i = 0;
    let bytes = result.as_bytes();
    while i < bytes.len() {
        if i + 5 <= bytes.len() && &result[i..i + 5] == "Heap(" {
            // Check if content is a simple numeric literal
            let after = &result[i + 5..];
            if let Some(close) = after.find(')') {
                let inner = after[..close].trim();
                if is_small_literal(inner) {
                    output.push_str(inner);
                    i += 5 + close + 1;
                    continue;
                }
            }
        }
        output.push(bytes[i] as char);
        i += 1;
    }
    let _ = chars; // suppress unused
    output
}

/// Fix `expr.clone()` → `expr` on a single line. Targets the simple
/// trailing-clone shape; nested `clone()` chains and method-chain
/// suffix forms are left alone (the user can rerun on shrunken
/// input or fix manually).
fn fix_redundant_clone(line: &str) -> String {
    // Walk left-to-right, collapsing `something.clone()` into
    // `something`. We honour the receiver boundary: the substring
    // immediately to the left of `.clone()` is preserved verbatim,
    // including any whitespace, so layout is intact.
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find(".clone()") {
        out.push_str(&rest[..pos]);
        rest = &rest[pos + ".clone()".len()..];
    }
    out.push_str(rest);
    out
}

/// Rewrite a single-line `match e { Pat(args) => body }` (one arm,
/// no semicolon-separated trailing arms) into the equivalent
/// `if let Pat(args) = e { body }`. Returns `None` when the line
/// doesn't fit the simple shape — a multi-arm or multi-line match
/// is out of scope for the line-based fixer.
fn fix_single_variant_match_inline(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("match ") {
        return None;
    }
    let indent = &line[..line.len() - trimmed.len()];
    // Locate the boundaries: `match <scrutinee> { <Pat> => <body> }`.
    let after_match = &trimmed["match ".len()..];
    let brace = after_match.find('{')?;
    let scrutinee = after_match[..brace].trim();
    let inside = after_match[brace + 1..].trim_end_matches('}').trim();
    // Inside must be exactly one arm: `Pat => body[,]` (single arrow).
    let arrow = inside.find("=>")?;
    let pat = inside[..arrow].trim();
    let body = inside[arrow + 2..].trim().trim_end_matches(',').trim();
    if pat.is_empty() || body.is_empty() {
        return None;
    }
    // Reject the underscore-arm case — `match x { _ => ... }` is
    // already legitimately one-arm; an `if let _ = x` is not the
    // same construct.
    if pat == "_" {
        return None;
    }
    Some(format!("{indent}if let {pat} = {scrutinee} {{ {body} }}"))
}

/// True when the line is a match-arm line whose body is empty —
/// `_ => ()`, `Pat => {}`, `Pat => {},`. The fixer drops these so
/// match exhaustiveness handles the missing arm.
fn is_empty_match_arm_line(line: &str) -> bool {
    let trimmed = line.trim().trim_end_matches(',').trim();
    trimmed.ends_with("=> ()") || trimmed.ends_with("=> {}") || trimmed.ends_with("=>{}")
}

/// `Type{ true }` → `Type`. Strips the always-true predicate while
/// leaving every other refinement shape alone.
fn fix_redundant_refinement(line: &str) -> String {
    // Search for the literal `{ true }` and the variants `{true}` /
    // `{ true}` / `{true }`. The receiver token immediately to the
    // left is whatever the type name is — we don't need to inspect
    // it.
    let mut out = line.to_string();
    for needle in &["{ true }", "{true}", "{true }", "{ true}"] {
        out = out.replace(needle, "");
    }
    out
}

/// Conservative shadow-binding fix: rename the `let x = …` /
/// `let mut x = …` on this line to `let x2 = …`. Downstream uses on
/// later lines are NOT rewritten — the deliberate compile error is
/// the signal for the author to finish the rename.
fn fix_shadow_rename_inner(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];
    let rest = if let Some(r) = trimmed.strip_prefix("let mut ") {
        r
    } else if let Some(r) = trimmed.strip_prefix("let ") {
        r
    } else {
        return None;
    };
    // Find the binding name (first identifier).
    let name_end = rest
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    let suffix = &rest[name_end..];
    let new_name = format!("{name}2");
    let prefix = if trimmed.starts_with("let mut ") {
        "let mut "
    } else {
        "let "
    };
    Some(format!("{indent}{prefix}{new_name}{suffix}"))
}

/// `// TODO` → `// TODO(#0000)` (and the same for FIXME / HACK /
/// XXX). The user is expected to fill in a real tracker number; the
/// next lint pass keeps complaining if they don't.
fn fix_todo_with_placeholder_issue(line: &str) -> String {
    let mut out = line.to_string();
    for marker in &["TODO", "FIXME", "HACK", "XXX"] {
        // Skip lines that already carry an issue tag.
        if out.contains(&format!("{marker}(#")) {
            continue;
        }
        // Replace the bare marker followed by a delimiter (':',
        // ' ', or end-of-line) — but only the first occurrence.
        let needle = format!("{marker}:");
        if let Some(pos) = out.find(&needle) {
            let head = &out[..pos];
            let tail = &out[pos + needle.len()..];
            out = format!("{head}{marker}(#0000):{tail}");
            continue;
        }
        let bare = format!("{marker} ");
        if let Some(pos) = out.find(&bare) {
            let head = &out[..pos];
            let tail = &out[pos + bare.len()..];
            out = format!("{head}{marker}(#0000) {tail}");
        }
    }
    out
}

/// Check if a string is a small literal (Int, Float, Bool)
fn is_small_literal(s: &str) -> bool {
    if s == "true" || s == "false" {
        return true;
    }
    // Integer literal (possibly negative)
    let s = s.strip_prefix('-').unwrap_or(s);
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        return true;
    }
    // Float literal
    if s.contains('.') {
        let parts: Vec<&str> = s.splitn(2, '.').collect();
        if parts.len() == 2
            && parts[0].chars().all(|c| c.is_ascii_digit())
            && parts[1].chars().all(|c| c.is_ascii_digit())
            && !parts[0].is_empty()
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Core linting engine
// ---------------------------------------------------------------------------

/// Parsed information about the file for cross-rule analysis
struct FileInfo {
    lines: Vec<String>,
    /// All context declarations found: "context Name { ... }"
    context_decls: HashSet<String>,
    /// All type declarations: name -> (line, field_count)
    type_decls: HashMap<String, (usize, usize)>,
    /// Types that have close()/free() methods
    types_with_resource_methods: HashSet<String>,
    /// Types that implement Cleanup
    types_with_cleanup: HashSet<String>,
    /// Imported names from mount statements: name -> line_number
    mount_imports: Vec<(String, Vec<String>, usize)>,
    /// Functions that return Result/Maybe types: (line, name)
    result_returning_fns: HashSet<String>,
}

impl FileInfo {
    fn parse(content: &str) -> Self {
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let mut context_decls = HashSet::new();
        let mut type_decls = HashMap::new();
        let mut types_with_resource_methods = HashSet::new();
        let mut types_with_cleanup = HashSet::new();
        let mut mount_imports = Vec::new();
        let mut result_returning_fns = HashSet::new();

        let mut current_type_name: Option<String> = None;
        let mut current_type_line: usize = 0;
        let mut current_field_count: usize = 0;
        let mut brace_depth: i32 = 0;
        let mut in_type_body = false;

        for (line_num, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Context declarations: "context Name { ... }"
            if trimmed.starts_with("context ") {
                if let Some(name) = trimmed
                    .strip_prefix("context ")
                    .and_then(|s| s.split_whitespace().next())
                {
                    context_decls.insert(name.trim_end_matches('{').trim().to_string());
                }
            }

            // Type declarations: "type Name is { field: Type, ... };"
            if trimmed.starts_with("type ") && trimmed.contains(" is ") {
                let after_type = trimmed.strip_prefix("type ").unwrap_or("");
                let name = after_type
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if trimmed.contains('{') {
                    // Count fields in single-line type decl
                    if trimmed.contains('}') {
                        let body = extract_between(trimmed, '{', '}');
                        let field_count = body
                            .split(',')
                            .filter(|f| f.contains(':'))
                            .count();
                        type_decls.insert(name, (line_num, field_count));
                    } else {
                        // Multi-line type decl
                        current_type_name = Some(name);
                        current_type_line = line_num;
                        current_field_count = 0;
                        in_type_body = true;
                        brace_depth = 1;
                    }
                } else if !name.is_empty() {
                    // Simple type alias or newtype
                    type_decls.insert(name, (line_num, 0));
                }
            } else if in_type_body {
                for ch in trimmed.chars() {
                    match ch {
                        '{' => brace_depth += 1,
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                }
                if trimmed.contains(':') && brace_depth == 1 {
                    current_field_count += trimmed
                        .split(',')
                        .filter(|f| f.contains(':'))
                        .count();
                }
                if brace_depth <= 0 {
                    if let Some(ref name) = current_type_name {
                        type_decls.insert(name.clone(), (current_type_line, current_field_count));
                    }
                    in_type_body = false;
                    current_type_name = None;
                }
            }

            // Implement blocks with close()/free() methods
            if trimmed.starts_with("implement ") && !trimmed.contains(" for ") {
                let impl_name = trimmed
                    .strip_prefix("implement ")
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("")
                    .trim_end_matches('{')
                    .trim()
                    .to_string();
                // Scan ahead for close()/free() methods
                for ahead in lines.iter().skip(line_num + 1) {
                    let at = ahead.trim();
                    if at.starts_with("fn close(") || at.starts_with("fn free(") {
                        types_with_resource_methods.insert(impl_name.clone());
                        break;
                    }
                    if at == "}" && !at.contains("fn ") {
                        // Simple heuristic: top-level closing brace ends impl
                        if ahead.starts_with('}') {
                            break;
                        }
                    }
                }
            }

            // implement Cleanup for Type
            if trimmed.starts_with("implement Cleanup for ") {
                let type_name = trimmed
                    .strip_prefix("implement Cleanup for ")
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("")
                    .trim_end_matches('{')
                    .trim()
                    .to_string();
                types_with_cleanup.insert(type_name);
            }

            // Mount statements: "mount module.{name1, name2}" or "mount module.name"
            if trimmed.starts_with("mount ") {
                if let Some(rest) = trimmed.strip_prefix("mount ") {
                    let rest = rest.trim_end_matches(';').trim();
                    let (module_path, names) = parse_mount_statement(rest);
                    mount_imports.push((module_path, names, line_num));
                }
            }

            // Functions returning Result/Maybe
            if trimmed.starts_with("fn ") {
                if let Some(arrow_pos) = trimmed.find("->") {
                    let ret_type = trimmed[arrow_pos + 2..].trim();
                    if ret_type.starts_with("Result")
                        || ret_type.starts_with("Maybe")
                        || ret_type.contains("Result<")
                        || ret_type.contains("Maybe<")
                    {
                        if let Some(fn_name) = extract_fn_name(trimmed) {
                            result_returning_fns.insert(fn_name);
                        }
                    }
                }
            }
        }

        FileInfo {
            lines,
            context_decls,
            type_decls,
            types_with_resource_methods,
            types_with_cleanup,
            mount_imports,
            result_returning_fns,
        }
    }
}

/// Parse a mount statement to extract module path and imported names
fn parse_mount_statement(rest: &str) -> (String, Vec<String>) {
    // "foo.bar.{Baz, Qux}" or "foo.bar.Baz" or "foo.bar.*"
    if let Some(brace_start) = rest.find('{') {
        let module_path = rest[..brace_start].trim_end_matches('.').to_string();
        let brace_end = rest.find('}').unwrap_or(rest.len());
        let names: Vec<String> = rest[brace_start + 1..brace_end]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        (module_path, names)
    } else if rest.contains('.') {
        let parts: Vec<&str> = rest.rsplitn(2, '.').collect();
        let name = parts[0].trim().to_string();
        let module_path = if parts.len() > 1 {
            parts[1].trim().to_string()
        } else {
            String::new()
        };
        if name == "*" {
            (module_path, vec!["*".to_string()])
        } else {
            (module_path, vec![name])
        }
    } else {
        (rest.to_string(), vec![rest.to_string()])
    }
}

/// Extract function name from a line like "fn foo(..."
fn extract_fn_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let after_fn = trimmed
        .strip_prefix("pub fn ")
        .or_else(|| trimmed.strip_prefix("fn "))?;
    let name_end = after_fn.find(|c: char| c == '(' || c == '<' || c == ' ')?;
    Some(after_fn[..name_end].to_string())
}

/// Extract text between matching delimiters (first occurrence)
fn extract_between(s: &str, open: char, close: char) -> &str {
    if let Some(start) = s.find(open) {
        if let Some(end) = s[start + 1..].find(close) {
            return &s[start + 1..start + 1 + end];
        }
    }
    ""
}

// ---------------------------------------------------------------------------
// Individual lint rule checkers
// ---------------------------------------------------------------------------

/// Rule 1: UncheckedRefinement
/// Functions with refinement type parameters that lack @verify annotation.
fn check_unchecked_refinement(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    let mut prev_lines_have_verify = false;

    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("@verify") {
            prev_lines_have_verify = true;
            continue;
        }

        // Look for function signatures bearing a refinement-typed
        // parameter. Verum syntax: `<BaseType>{ <predicate> }` —
        // the `{` follows the type name with no whitespace. Heuristic:
        // an `Int{` / `Text{` / `Float{` / `<Custom>{` token inside
        // the parameter list of an `fn` declaration.
        let is_fn_decl = trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("public fn ");
        if is_fn_decl {
            if let (Some(open), Some(close)) = (trimmed.find('('), trimmed.rfind(')')) {
                if close > open {
                    let params = &trimmed[open + 1..close];
                    if has_refinement_token(params) && !prev_lines_have_verify {
                        let fn_name = extract_fn_name(trimmed).unwrap_or_default();
                        issues.push(LintIssue {
                            rule: "unchecked-refinement",
                            level: LintLevel::Warning,
                            file: path.to_path_buf(),
                            line: line_num + 1,
                            column: 1,
                            message: format!(
                                "Function '{}' has refinement type parameters without @verify annotation",
                                fn_name
                            ),
                            suggestion: Some(
                                "Add @verify(formal) above the function to discharge the refinement obligation"
                                    .into(),
                            ),
                            fixable: false,
                        });
                    }
                }
            }
        }

        // Reset @verify tracking once we leave the annotation block.
        if !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('@') {
            prev_lines_have_verify = false;
        }
    }
}

/// Detect `<TypeName>{` — a refinement-bearing type token. The `{`
/// must immediately follow an alphanumeric character (no
/// intervening whitespace) so that bodies like `fn foo(x: Int) {`
/// don't match.
fn has_refinement_token(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'{' {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'>' {
                return true;
            }
        }
    }
    false
}

/// Rule 2: MissingContextDecl
/// Functions using `using [X]` where X is not declared as a context in scope.
fn check_missing_context_decl(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Look for "using [ContextA, ContextB]" in function signatures
        if let Some(using_pos) = trimmed.find("using [") {
            let after_using = &trimmed[using_pos + 7..];
            if let Some(bracket_end) = after_using.find(']') {
                let context_list = &after_using[..bracket_end];
                for ctx_name in context_list.split(',') {
                    let ctx_name = ctx_name.trim();
                    if ctx_name.is_empty() {
                        continue;
                    }
                    // Check if this context is declared in the file
                    if !info.context_decls.contains(ctx_name) {
                        // Also check mount imports
                        let imported = info
                            .mount_imports
                            .iter()
                            .any(|(_, names, _)| names.iter().any(|n| n == ctx_name || n == "*"));

                        if !imported {
                            let col = line.find("using [").unwrap_or(0) + 1;
                            issues.push(LintIssue {
                                rule: "missing-context-decl",
                                level: LintLevel::Error,
                                file: path.to_path_buf(),
                                line: line_num + 1,
                                column: col,
                                message: format!(
                                    "Context '{}' used but not declared or imported in this file",
                                    ctx_name
                                ),
                                suggestion: Some(
                                    format!(
                                        "Add 'context {} {{ ... }}' or import it via mount",
                                        ctx_name
                                    )
                                    .into(),
                                ),
                                fixable: false,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Rule 3: UnusedImport
/// Mount statements where the imported name is never referenced in the file.
fn check_unused_imports(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (module_path, names, mount_line) in &info.mount_imports {
        // Wildcard imports can't be checked
        if names.iter().any(|n| n == "*") {
            continue;
        }

        for name in names {
            if name.is_empty() {
                continue;
            }

            // Check if the name appears anywhere in the file other than the mount line
            let used = info.lines.iter().enumerate().any(|(idx, line)| {
                if idx == *mount_line {
                    return false;
                }
                let trimmed = line.trim();
                // Skip comments
                if trimmed.starts_with("//") {
                    return false;
                }
                // Check for the name as a word boundary (not just substring)
                contains_word(trimmed, name)
            });

            if !used {
                let display_import = if names.len() == 1 {
                    format!("{}.{}", module_path, name)
                } else {
                    name.clone()
                };
                issues.push(LintIssue {
                    rule: "unused-import",
                    level: LintLevel::Warning,
                    file: path.to_path_buf(),
                    line: mount_line + 1,
                    column: 1,
                    message: format!("Unused import: {}", display_import),
                    suggestion: Some("Remove unused import".into()),
                    fixable: true,
                });
            }
        }
    }
}

/// Check if a line contains a word (not just a substring)
fn contains_word(line: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = line[start..].find(word) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0
            || !line.as_bytes()[abs_pos - 1].is_ascii_alphanumeric()
                && line.as_bytes()[abs_pos - 1] != b'_';
        let after_pos = abs_pos + word.len();
        let after_ok = after_pos >= line.len()
            || !line.as_bytes()[after_pos].is_ascii_alphanumeric()
                && line.as_bytes()[after_pos] != b'_';
        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
        if start >= line.len() {
            break;
        }
    }
    false
}

/// Rule 4: UnnecessaryHeap
/// Heap(x) where x is a small type (Int, Float, Bool) literal.
fn check_unnecessary_heap(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();
        // Skip comments
        if trimmed.starts_with("//") {
            continue;
        }

        // Scan for Heap(literal) patterns
        let mut search_start = 0;
        while let Some(pos) = line[search_start..].find("Heap(") {
            let abs_pos = search_start + pos;
            // Check it's not part of a larger word (e.g., "MinHeap(")
            if abs_pos > 0 && line.as_bytes()[abs_pos - 1].is_ascii_alphanumeric() {
                search_start = abs_pos + 5;
                continue;
            }

            let after = &line[abs_pos + 5..];
            if let Some(close) = find_matching_paren(after) {
                let inner = after[..close].trim();
                if is_small_literal(inner) {
                    issues.push(LintIssue {
                        rule: "unnecessary-heap",
                        level: LintLevel::Warning,
                        file: path.to_path_buf(),
                        line: line_num + 1,
                        column: abs_pos + 1,
                        message: format!(
                            "Unnecessary heap allocation for small value: Heap({})",
                            inner
                        ),
                        suggestion: Some(
                            format!(
                                "Use '{}' directly; Int/Float/Bool fit in registers",
                                inner
                            )
                            .into(),
                        ),
                        fixable: true,
                    });
                } else {
                    // Check if inner is a known small type variable
                    // Heuristic: if the inner expression is a simple identifier and its type
                    // is declared as Int/Float/Bool, flag it
                    check_heap_of_small_type(path, info, line_num, abs_pos, inner, issues);
                }
            }
            search_start = abs_pos + 5;
        }
    }
}

/// Check if a Heap() argument is a known small type
fn check_heap_of_small_type(
    path: &Path,
    _info: &FileInfo,
    line_num: usize,
    col: usize,
    inner: &str,
    issues: &mut List<LintIssue>,
) {
    // If inner is something like "0", "1.0", "true", "false" we already caught it
    // Here check for typed constructor patterns: Heap(Int), Heap(Bool), Heap(Float)
    let small_types = ["Int", "Float", "Bool", "Byte", "Char"];
    for st in &small_types {
        if inner == *st {
            issues.push(LintIssue {
                rule: "unnecessary-heap",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col + 1,
                message: format!("Unnecessary heap allocation for small type: Heap({})", inner),
                suggestion: Some(
                    format!("{} is a small type that fits in a register", inner).into(),
                ),
                fixable: false,
            });
        }
    }
}

/// Find the position of the matching closing paren, accounting for nesting
fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s.chars().enumerate() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Rule 5: MissingErrorContext
/// `?` operator usage without `.with_context()` or `.map_err()`.
fn check_missing_error_context(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") {
            continue;
        }

        // Look for lines ending with `?` or containing `?;` or `?)`
        // but not preceded by `.with_context(` or `.map_err(`
        let mut search_start = 0;
        while let Some(q_pos) = trimmed[search_start..].find('?') {
            let abs_pos = search_start + q_pos;

            // Check this is actually the ? operator (not inside a string or comment)
            if abs_pos > 0 {
                // Must follow a ) or identifier char or ] (expression context)
                let prev_char = trimmed.as_bytes()[abs_pos - 1];
                if prev_char == b')' || prev_char == b']' || prev_char.is_ascii_alphanumeric() || prev_char == b'_' {
                    // Check if preceded by .with_context( or .map_err(
                    let before = &trimmed[..abs_pos];
                    let has_context = before.contains(".with_context(")
                        || before.contains(".map_err(")
                        || before.contains(".context(");

                    if !has_context {
                        // Find the expression before ?
                        let expr_start = before
                            .rfind(|c: char| c == '=' || c == '(' || c == '{' || c == ';')
                            .map(|p| p + 1)
                            .unwrap_or(0);
                        let expr = before[expr_start..].trim();

                        // Don't flag if expr is just a simple variable or short expression
                        // Only flag function calls and method calls
                        if expr.contains('(') || expr.contains('.') {
                            issues.push(LintIssue {
                                rule: "missing-error-context",
                                level: LintLevel::Warning,
                                file: path.to_path_buf(),
                                line: line_num + 1,
                                column: abs_pos + 1,
                                message: "Error propagation with '?' lacks context".to_string(),
                                suggestion: Some(
                                    "Add .with_context(|| \"description\") before '?'".into(),
                                ),
                                fixable: false,
                            });
                        }
                    }
                }
            }
            search_start = abs_pos + 1;
            if search_start >= trimmed.len() {
                break;
            }
        }
    }
}

/// Rule 6: LargeCopy
/// Function parameters taking large struct types by value (>4 fields).
fn check_large_copy(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Look for function signatures
        if !trimmed.starts_with("fn ") && !trimmed.starts_with("pub fn ") {
            continue;
        }

        // Extract parameter list
        if let Some(paren_start) = trimmed.find('(') {
            if let Some(paren_end) = trimmed.rfind(')') {
                let params = &trimmed[paren_start + 1..paren_end];

                // Parse each parameter
                for param in split_params(params) {
                    let param = param.trim();
                    if param.is_empty() || param == "&self" || param == "&mut self" || param == "self" {
                        continue;
                    }

                    // param format: "name: Type"
                    if let Some(colon_pos) = param.find(':') {
                        let param_name = param[..colon_pos].trim();
                        let param_type = param[colon_pos + 1..].trim();

                        // Skip references
                        if param_type.starts_with('&') {
                            continue;
                        }

                        // Check if the type is a known large struct
                        let type_name = param_type
                            .split('<')
                            .next()
                            .unwrap_or(param_type)
                            .trim();

                        if let Some((_, field_count)) = info.type_decls.get(type_name) {
                            if *field_count > 4 {
                                issues.push(LintIssue {
                                    rule: "large-copy",
                                    level: LintLevel::Warning,
                                    file: path.to_path_buf(),
                                    line: line_num + 1,
                                    column: paren_start + 2,
                                    message: format!(
                                        "Parameter '{}' copies type '{}' ({} fields) by value",
                                        param_name, type_name, field_count
                                    ),
                                    suggestion: Some(
                                        format!(
                                            "Pass by reference: '{}: &{}'",
                                            param_name, param_type
                                        )
                                        .into(),
                                    ),
                                    fixable: false,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Split parameter list by commas, respecting nested generics and braces
fn split_params(params: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    for (i, ch) in params.chars().enumerate() {
        match ch {
            '<' | '(' | '{' | '[' => depth += 1,
            '>' | ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&params[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < params.len() {
        result.push(&params[start..]);
    }
    result
}

/// Rule 7: UnusedResult
/// Function calls returning Result/Maybe whose value is not bound or propagated.
fn check_unused_result(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Skip comments, blank lines, control flow
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("return ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("match ")
            || trimmed.starts_with("}")
            || trimmed.starts_with("{")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("mount ")
            || trimmed.starts_with("context ")
            || trimmed.starts_with("implement ")
            || trimmed.starts_with("@")
        {
            continue;
        }

        // Look for bare function/method calls as statements (ending with ;)
        if trimmed.ends_with(';') && !trimmed.contains('=') {
            let stmt = trimmed.trim_end_matches(';').trim();

            // Must contain a function call
            if !stmt.contains('(') {
                continue;
            }

            // Skip if it ends with ? (error is propagated)
            if stmt.ends_with('?') {
                continue;
            }

            // Extract the function name
            let fn_name = if stmt.contains('.') {
                // Method call: extract method name
                // e.g., "foo.bar()" -> "bar"
                if let Some(dot_pos) = stmt.rfind('.') {
                    let after_dot = &stmt[dot_pos + 1..];
                    after_dot
                        .split('(')
                        .next()
                        .unwrap_or("")
                        .trim()
                } else {
                    ""
                }
            } else {
                // Free function call
                stmt.split('(').next().unwrap_or("").trim()
            };

            // Check if this function is known to return Result/Maybe
            if info.result_returning_fns.contains(fn_name) {
                issues.push(LintIssue {
                    rule: "unused-result",
                    level: LintLevel::Warning,
                    file: path.to_path_buf(),
                    line: line_num + 1,
                    column: 1,
                    message: format!(
                        "Return value of '{}' (Result/Maybe) is unused",
                        fn_name
                    ),
                    suggestion: Some(
                        "Use 'let _ = ...' to explicitly discard, or handle the result".into(),
                    ),
                    fixable: false,
                });
            }
        }
    }
}

/// Rule 8: MissingCleanup
/// Types with close()/free() method but no `implement Cleanup for`.
fn check_missing_cleanup(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for type_name in &info.types_with_resource_methods {
        if !info.types_with_cleanup.contains(type_name) {
            // Find the type declaration line for better error reporting
            let decl_line = info
                .type_decls
                .get(type_name)
                .map(|(line, _)| *line + 1)
                .unwrap_or(1);

            issues.push(LintIssue {
                rule: "missing-cleanup",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: decl_line,
                column: 1,
                message: format!(
                    "Type '{}' has close()/free() method but does not implement Cleanup protocol",
                    type_name
                ),
                suggestion: Some(
                    format!(
                        "Add 'implement Cleanup for {} {{ fn cleanup(&mut self) {{ self.close(); }} }}'",
                        type_name
                    )
                    .into(),
                ),
                fixable: false,
            });
        }
    }
}

/// Rule 9: DeprecatedSyntax
/// Check for Rust-isms: struct, impl, !, Vec<T>, etc.
fn check_deprecated_syntax(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") {
            continue;
        }

        // Check for `struct` keyword (should be `type Name is { ... }`)
        if trimmed.starts_with("struct ") || trimmed.starts_with("pub struct ") {
            let col = line.find("struct").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'type Name is { ... }' instead of 'struct'".to_string(),
                suggestion: Some("Use 'type X is { ... }' instead of 'struct X { ... }'".into()),
                fixable: true,
            });
        }

        // Check for bare `impl` instead of `implement`
        // Be careful not to match "implement" itself
        if (trimmed.starts_with("impl ") || trimmed.starts_with("pub impl "))
            && !trimmed.starts_with("implement")
        {
            let col = line.find("impl").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'implement' instead of 'impl'".to_string(),
                suggestion: Some("Use 'implement' instead of 'impl'".into()),
                fixable: true,
            });
        }

        // Check for `trait` keyword (should be `type Name is protocol { ... }`)
        if trimmed.starts_with("trait ") || trimmed.starts_with("pub trait ") {
            let col = line.find("trait").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'type Name is protocol { ... }' instead of 'trait'".to_string(),
                suggestion: Some("Use 'type X is protocol { ... }' instead of 'trait X { ... }'".into()),
                fixable: false,
            });
        }

        // Check for `enum` keyword (should be sum type)
        if trimmed.starts_with("enum ") || trimmed.starts_with("pub enum ") {
            let col = line.find("enum").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'type Name is A | B' instead of 'enum'".to_string(),
                suggestion: Some("Use 'type X is A | B;' instead of 'enum X { A, B }'".into()),
                fixable: false,
            });
        }

        // Check for Vec<T> (should be List<T>)
        if contains_word(trimmed, "Vec") && trimmed.contains("Vec<") {
            let col = line.find("Vec<").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'List<T>' instead of 'Vec<T>'".to_string(),
                suggestion: Some("Use 'List<T>' instead of 'Vec<T>'".into()),
                fixable: true,
            });
        }

        // Check for String type (should be Text)
        // Be careful: only flag standalone "String" as a type, not inside strings
        if contains_word(trimmed, "String") && !trimmed.starts_with("//") {
            // Heuristic: "String" appearing after : or < or as a type annotation
            if trimmed.contains(": String")
                || trimmed.contains("<String>")
                || trimmed.contains("-> String")
                || trimmed.starts_with("String")
            {
                let col = line.find("String").unwrap_or(0) + 1;
                issues.push(LintIssue {
                    rule: "deprecated-syntax",
                    level: LintLevel::Error,
                    file: path.to_path_buf(),
                    line: line_num + 1,
                    column: col,
                    message: "Use 'Text' instead of 'String'".to_string(),
                    suggestion: Some("Use 'Text' instead of 'String'".into()),
                    fixable: false,
                });
            }
        }

        // Check for HashMap (should be Map)
        if contains_word(trimmed, "HashMap") {
            let col = line.find("HashMap").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'Map<K, V>' instead of 'HashMap<K, V>'".to_string(),
                suggestion: Some("Use 'Map<K, V>' instead of 'HashMap<K, V>'".into()),
                fixable: false,
            });
        }

        // Check for HashSet (should be Set)
        if contains_word(trimmed, "HashSet") {
            let col = line.find("HashSet").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'Set<T>' instead of 'HashSet<T>'".to_string(),
                suggestion: Some("Use 'Set<T>' instead of 'HashSet<T>'".into()),
                fixable: false,
            });
        }

        // Check for ! macro syntax: println!(), format!(), panic!(), assert!(), etc.
        let macro_patterns = [
            ("println!(", "print("),
            ("format!(", "f\"...\""),
            ("panic!(", "panic("),
            ("assert!(", "assert("),
            ("assert_eq!(", "assert_eq("),
            ("assert_ne!(", "assert_ne("),
            ("unreachable!(", "unreachable("),
            ("todo!(", "todo("),
            ("vec![", "List ["),
        ];
        for (bad, good) in &macro_patterns {
            if trimmed.contains(bad) {
                let col = line.find(bad).unwrap_or(0) + 1;
                issues.push(LintIssue {
                    rule: "deprecated-syntax",
                    level: LintLevel::Error,
                    file: path.to_path_buf(),
                    line: line_num + 1,
                    column: col,
                    message: format!("Use '{}' instead of '{}' (Verum has no '!' macro syntax)", good, bad.trim_end_matches('(')),
                    suggestion: Some(format!("Use '{}' without '!'", good).into()),
                    fixable: true,
                });
            }
        }

        // Check for `use` keyword (should be `mount`)
        if trimmed.starts_with("use ") && !trimmed.starts_with("using") {
            let col = 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'mount' instead of 'use' for imports".to_string(),
                suggestion: Some("Use 'mount module.name' instead of 'use module::name'".into()),
                fixable: false,
            });
        }

        // Check for Box::new (should be Heap())
        if trimmed.contains("Box::new(") {
            let col = line.find("Box::new(").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use 'Heap(x)' instead of 'Box::new(x)'".to_string(),
                suggestion: Some("Use 'Heap(x)' instead of 'Box::new(x)'".into()),
                fixable: false,
            });
        }

        // Check for :: path separator (should be .)
        // Only flag when it looks like a path, not inside strings
        if trimmed.contains("::") && !trimmed.starts_with("//") && !trimmed.contains("\"") {
            // Heuristic: A::B pattern that's not inside a string literal
            let col = line.find("::").unwrap_or(0) + 1;
            issues.push(LintIssue {
                rule: "deprecated-syntax",
                level: LintLevel::Error,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: col,
                message: "Use '.' path separator instead of '::' (Verum uses dot paths)".to_string(),
                suggestion: Some("Replace '::' with '.' in module paths".into()),
                fixable: false,
            });
        }
    }
}

/// Rule 10: CbgrHotspot
/// Tight loops containing reference dereferences that could use &checked.
fn check_cbgr_hotspot(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    let mut in_loop = false;
    let mut loop_start_line: usize = 0;
    let mut loop_depth: i32 = 0;
    let mut loop_has_deref = false;
    let mut deref_lines: Vec<usize> = Vec::new();

    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // Track loop entry
        if trimmed.starts_with("for ") || trimmed.starts_with("while ") || trimmed == "loop {" {
            if !in_loop {
                in_loop = true;
                loop_start_line = line_num;
                loop_has_deref = false;
                deref_lines.clear();
                loop_depth = 0;
            }
            // Count braces on the loop line itself
            for ch in trimmed.chars() {
                match ch {
                    '{' => loop_depth += 1,
                    '}' => loop_depth -= 1,
                    _ => {}
                }
            }
            continue;
        }

        if in_loop {
            for ch in trimmed.chars() {
                match ch {
                    '{' => loop_depth += 1,
                    '}' => loop_depth -= 1,
                    _ => {}
                }
            }

            // Check for reference dereferences inside the loop
            // Patterns: *ref_name, &variable (tier 0 reference creation), .deref()
            if trimmed.contains('*') && !trimmed.starts_with("//") {
                // Simple heuristic: * followed by an identifier
                let mut chars = trimmed.chars().peekable();
                while let Some(ch) = chars.next() {
                    if ch == '*' {
                        if let Some(&next) = chars.peek() {
                            if next.is_ascii_alphanumeric() || next == '_' {
                                loop_has_deref = true;
                                deref_lines.push(line_num);
                                break;
                            }
                        }
                    }
                }
            }

            // Also flag &T references created inside loops (CBGR overhead per iteration)
            if (trimmed.contains("&") && !trimmed.contains("&&") && !trimmed.contains("&checked") && !trimmed.contains("&unsafe"))
                && (trimmed.contains("let ") || trimmed.contains("= &"))
            {
                loop_has_deref = true;
                deref_lines.push(line_num);
            }

            // Loop end
            if loop_depth <= 0 {
                if loop_has_deref && !deref_lines.is_empty() {
                    issues.push(LintIssue {
                        rule: "cbgr-hotspot",
                        level: LintLevel::Info,
                        file: path.to_path_buf(),
                        line: loop_start_line + 1,
                        column: 1,
                        message: format!(
                            "Tight loop contains {} reference dereference(s) with CBGR overhead (~15ns each)",
                            deref_lines.len()
                        ),
                        suggestion: Some(
                            "Consider using &checked references if escape analysis can prove safety (0ns overhead)".into(),
                        ),
                        fixable: false,
                    });
                }
                in_loop = false;
                deref_lines.clear();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main file linting
// ---------------------------------------------------------------------------

/// Lint a single file by running all rules. Reloads the config
/// per call — used by entry points that don't have a pre-loaded
/// `LintConfig` to share. Hot paths (the parallel runner) should
/// call `lint_file_with` instead so the config is loaded once.
fn lint_file(path: &Path) -> Result<List<LintIssue>> {
    let cfg = load_full_lint_config();
    lint_file_with(path, &cfg)
}

/// Lint a single file with an externally-provided `LintConfig`.
/// Reading the file is the only I/O performed here, which makes
/// this function trivially `Send + Sync` and parallelisable.
fn lint_file_with(path: &Path, cfg: &LintConfig) -> Result<List<LintIssue>> {
    let content = fs::read_to_string(path)?;
    let mut issues = List::new();
    let info = FileInfo::parse(&content);

    check_unchecked_refinement(path, &info, &mut issues);
    check_missing_context_decl(path, &info, &mut issues);
    check_unused_imports(path, &info, &mut issues);
    check_unnecessary_heap(path, &info, &mut issues);
    check_missing_error_context(path, &info, &mut issues);
    check_large_copy(path, &info, &mut issues);
    check_unused_result(path, &info, &mut issues);
    check_missing_cleanup(path, &info, &mut issues);
    check_deprecated_syntax(path, &info, &mut issues);
    check_cbgr_hotspot(path, &info, &mut issues);
    check_extended_rules(path, &info, &mut issues);
    for issue in lint_custom_rules(path, &content, cfg) {
        issues.push(issue);
    }

    // AST-driven passes — parse the source via verum_parser and walk
    // the Module with verum_ast::Visitor. Parse failures fall through
    // silently so text-scan output is still produced.
    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    let fid = FileId::new(0);
    let lexer = Lexer::new(&content, fid);
    let parser = VerumParser::new();
    if let Ok(module) = parser.parse_module(lexer, fid) {
        let ctx = super::lint_engine::LintCtx {
            file: path,
            source: &content,
            module: &module,
            config: Some(cfg),
        };
        for issue in super::lint_engine::run(&ctx) {
            issues.push(issue);
        }

        let scopes = super::lint_engine::collect_suppressions(&module, &content);
        if !scopes.is_empty() {
            let collected: Vec<_> = std::mem::take(&mut issues).into_iter().collect();
            let suppressed = super::lint_engine::apply_suppressions(collected, &scopes);
            for i in suppressed {
                issues.push(i);
            }
        }
    }

    Ok(issues)
}

/// Sort key that gives us deterministic output regardless of which
/// thread reported the issue first.
fn issue_sort_key(i: &LintIssue) -> (String, usize, usize, &'static str) {
    (
        i.file.to_string_lossy().into_owned(),
        i.line,
        i.column,
        i.rule,
    )
}

/// Discover every `.vr` file under each search root, lint them in
/// parallel, return a flat sorted issue list. Errors per-file are
/// surfaced via `eprintln!` rather than failing the whole run — one
/// unreadable fixture should not block the rest of the corpus.
fn lint_paths_parallel(
    search_dirs: &[PathBuf],
    cfg: &LintConfig,
) -> (Vec<LintIssue>, usize) {
    let cache = build_lint_cache(cfg);
    cache.gc();
    lint_paths_parallel_with_cache(search_dirs, cfg, &cache)
}

fn lint_paths_parallel_with_cache(
    search_dirs: &[PathBuf],
    cfg: &LintConfig,
    cache: &super::lint_cache::LintCache,
) -> (Vec<LintIssue>, usize) {
    use rayon::prelude::*;

    let files: Vec<PathBuf> = search_dirs
        .iter()
        .flat_map(|d| {
            WalkDir::new(d)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && is_verum_file(p))
                .collect::<Vec<_>>()
        })
        .collect();
    let total_files = files.len();

    let issues: Vec<LintIssue> = files
        .par_iter()
        .flat_map(|path| lint_one_with_cache(path, cfg, cache))
        .collect();

    let mut issues = issues;
    issues.sort_by(|a, b| issue_sort_key(a).cmp(&issue_sort_key(b)));
    (issues, total_files)
}

/// Process one file: cache hit → reuse; cache miss → lint, persist,
/// return. Read-error files are surfaced as warnings and contribute
/// no issues.
fn lint_one_with_cache(
    path: &Path,
    cfg: &LintConfig,
    cache: &super::lint_cache::LintCache,
) -> Vec<LintIssue> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: failed to read {}: {}", path.display(), e);
            return Vec::new();
        }
    };
    let sh = super::lint_cache::source_hash(&content);
    if let Some(cached) = cache.load(&sh) {
        return cached;
    }
    let issues = match lint_content_with(path, &content, cfg) {
        Ok(i) => i.into_iter().collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("warning: failed to lint {}: {}", path.display(), e);
            Vec::new()
        }
    };
    cache.store(&sh, &issues);
    issues
}

/// Lint already-loaded content. Hot-path counterpart to
/// `lint_file_with` for the cache flow that has the bytes in hand.
fn lint_content_with(
    path: &Path,
    content: &str,
    cfg: &LintConfig,
) -> Result<List<LintIssue>> {
    let mut issues = List::new();
    let info = FileInfo::parse(content);

    check_unchecked_refinement(path, &info, &mut issues);
    check_missing_context_decl(path, &info, &mut issues);
    check_unused_imports(path, &info, &mut issues);
    check_unnecessary_heap(path, &info, &mut issues);
    check_missing_error_context(path, &info, &mut issues);
    check_large_copy(path, &info, &mut issues);
    check_unused_result(path, &info, &mut issues);
    check_missing_cleanup(path, &info, &mut issues);
    check_deprecated_syntax(path, &info, &mut issues);
    check_cbgr_hotspot(path, &info, &mut issues);
    check_extended_rules(path, &info, &mut issues);
    for issue in lint_custom_rules(path, content, cfg) {
        issues.push(issue);
    }

    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    let fid = FileId::new(0);
    let lexer = Lexer::new(content, fid);
    let parser = VerumParser::new();
    if let Ok(module) = parser.parse_module(lexer, fid) {
        let ctx = super::lint_engine::LintCtx {
            file: path,
            source: content,
            module: &module,
            config: Some(cfg),
        };
        for issue in super::lint_engine::run(&ctx) {
            issues.push(issue);
        }
        let scopes = super::lint_engine::collect_suppressions(&module, content);
        if !scopes.is_empty() {
            let collected: Vec<_> = std::mem::take(&mut issues).into_iter().collect();
            let suppressed = super::lint_engine::apply_suppressions(collected, &scopes);
            for i in suppressed {
                issues.push(i);
            }
        }
    }
    Ok(issues)
}

/// Build the corpus, run every cross-file pass, return the issues.
/// Files that fail to parse are excluded — cross-file rules need a
/// `Module`, and the per-file phase already surfaced the parse
/// error. This is the single place that re-parses for the corpus
/// view; future work could share the per-file parses if memory
/// budget allows.
fn run_cross_file_phase(files: &[PathBuf], cfg: &LintConfig) -> Vec<LintIssue> {
    use rayon::prelude::*;
    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let parsed: Vec<super::lint_engine::CorpusFile> = files
        .par_iter()
        .filter_map(|path| {
            let source = fs::read_to_string(path).ok()?;
            let fid = FileId::new(0);
            let lexer = Lexer::new(&source, fid);
            let parser = VerumParser::new();
            let module = parser.parse_module(lexer, fid).ok()?;
            Some(super::lint_engine::CorpusFile {
                path: path.clone(),
                source,
                module,
            })
        })
        .collect();

    let ctx = super::lint_engine::CorpusCtx {
        files: &parsed,
        config: Some(cfg),
    };
    super::lint_engine::run_cross_file(&ctx)
}

/// CLI entry: wipe the lint cache and exit. Idempotent — calling
/// when the cache doesn't exist is a no-op.
pub fn clean_cache() -> Result<()> {
    let target = std::env::current_dir()
        .map(|cwd| cwd.join("target"))
        .unwrap_or_else(|_| PathBuf::from("target"));
    let cache_root = target.join("lint-cache");
    if cache_root.exists() {
        std::fs::remove_dir_all(&cache_root).map_err(|e| {
            CliError::Custom(format!(
                "failed to remove {}: {}",
                cache_root.display(),
                e
            ))
        })?;
        ui::success(&format!("removed {}", cache_root.display()));
    } else {
        ui::info(&format!("{}: nothing to clean", cache_root.display()));
    }
    Ok(())
}

/// Build a cache rooted at `target/lint-cache/` for the project.
/// `target/` is the conventional Cargo-style artefact directory; the
/// cache lives under it so a `cargo clean` (or `verum clean`) removes
/// stale entries with the rest of the build artefacts.
///
/// The cache is enabled by default; set `VERUM_LINT_NO_CACHE=1`
/// (or pass `--no-cache` on the CLI, which exports the same var) to
/// bypass it for this run.
fn build_lint_cache(cfg: &LintConfig) -> super::lint_cache::LintCache {
    let enabled = match std::env::var("VERUM_LINT_NO_CACHE") {
        Ok(v) => !matches!(v.as_str(), "1" | "true" | "yes"),
        Err(_) => true,
    };
    let target = std::env::current_dir()
        .map(|cwd| cwd.join("target"))
        .unwrap_or_else(|_| PathBuf::from("target"));
    super::lint_cache::LintCache::new(&target, cfg, enabled)
}

/// Extended lint rules (11-20): Verum-specific analysis beyond Clippy.
fn check_extended_rules(path: &Path, info: &FileInfo, issues: &mut List<LintIssue>) {
    for (line_num, line) in info.lines.iter().enumerate() {
        let trimmed = line.trim();

        // todo-in-code is the one rule that needs to read comment
        // lines — every other rule below skips them, so we run it
        // first and then early-out on comments.
        if trimmed.starts_with("//") {
            for marker in &["TODO", "FIXME", "HACK", "XXX"] {
                if trimmed.contains(marker)
                    // Suppress when the marker carries a tracking ref
                    // such as TODO(#1234) — that's the form
                    // [lint.rules.todo-in-code].require-issue-link
                    // expects.
                    && !trimmed.contains(&format!("{}(#", marker))
                {
                    issues.push(LintIssue {
                        rule: "todo-in-code",
                        level: LintLevel::Warning,
                        file: path.to_path_buf(),
                        line: line_num + 1,
                        column: trimmed.find(marker).unwrap_or(0) + 1,
                        message: format!("{} comment in code", marker),
                        suggestion: Some(format!("{}(#0000)", marker).into()),
                        fixable: true,
                    });
                    break;
                }
            }
            continue;
        }

        // Rule 11: mutable-capture-in-spawn
        // `spawn { ... mut_var ... }` where mut_var is `let mut` in outer scope
        if trimmed.contains("spawn") && trimmed.contains('{') {
            // Check if any `let mut` variables from preceding lines are referenced
            for prev_line in &info.lines[..line_num] {
                let prev = prev_line.trim();
                if prev.starts_with("let mut ") {
                    if let Some(var_name) = prev.strip_prefix("let mut ").and_then(|s| s.split_whitespace().next()) {
                        let var_name = var_name.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
                        // Check if the spawn block references this variable
                        let spawn_block: String = info.lines[line_num..].iter().take(20).cloned().collect::<Vec<_>>().join("\n");
                        if contains_word(&spawn_block, var_name) {
                            issues.push(LintIssue {
                                rule: "mutable-capture-in-spawn",
                                level: LintLevel::Error,
                                file: path.to_path_buf(),
                                line: line_num + 1,
                                column: 1,
                                message: format!("Mutable variable `{}` captured by spawn closure", var_name),
                                suggestion: Some("Use a Channel or Mutex to share mutable state across threads".into()),
                                fixable: false,
                            });
                            break; // one warning per spawn
                        }
                    }
                }
            }
        }

        // Rule 12: unbounded-channel — `Channel.new()` with zero
        // args. The empty parenthesis pair is the literal signature
        // we want; any other shape (`Channel.new(64)` /
        // `Channel.bounded(...)`) is fine.
        if trimmed.contains("Channel.new()") {
            issues.push(LintIssue {
                rule: "unbounded-channel",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: trimmed.find("Channel.new()").unwrap_or(0) + 1,
                message: "Channel created without capacity limit".to_string(),
                suggestion: Some("Use Channel.new(capacity) to prevent OOM under backpressure".into()),
                fixable: false,
            });
        }

        // Rule 13: missing-timeout
        if (trimmed.contains(".recv()") || trimmed.contains(".await") || trimmed.contains("join("))
            && !trimmed.contains("timeout") && !trimmed.contains("try_recv") && !trimmed.contains("select")
        {
            issues.push(LintIssue {
                rule: "missing-timeout",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: 1,
                message: "Blocking operation without timeout".to_string(),
                suggestion: Some("Consider using select { } with a timeout arm for robustness".into()),
                fixable: false,
            });
        }

        // Rule 15: empty-match-arm
        if trimmed.contains("=> ()") || trimmed.contains("=> { }") || trimmed.ends_with("=> {},") {
            issues.push(LintIssue {
                rule: "empty-match-arm",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: 1,
                message: "Match arm has empty body".to_string(),
                suggestion: Some("drop the arm and let exhaustiveness handle it, or add a body".into()),
                fixable: true,
            });
        }

        // todo-in-code on inline trailing comments (`code; // TODO`).
        // Pure comment-line cases are handled at the top of the loop.
        if let Some(comment_at) = trimmed.find("//") {
            let after = &trimmed[comment_at..];
            for marker in &["TODO", "FIXME", "HACK", "XXX"] {
                if after.contains(marker) && !after.contains(&format!("{}(#", marker)) {
                    let col = comment_at
                        + after.find(marker).unwrap_or(0)
                        + 1;
                    issues.push(LintIssue {
                        rule: "todo-in-code",
                        level: LintLevel::Warning,
                        file: path.to_path_buf(),
                        line: line_num + 1,
                        column: col,
                        message: format!("{} comment in code", marker),
                        suggestion: Some(format!("{}(#0000)", marker).into()),
                        fixable: true,
                    });
                    break;
                }
            }
        }

        // Rule 18: unsafe-ref-in-public
        if trimmed.starts_with("pub fn ") && trimmed.contains("&unsafe ") {
            issues.push(LintIssue {
                rule: "unsafe-ref-in-public",
                level: LintLevel::Warning,
                file: path.to_path_buf(),
                line: line_num + 1,
                column: trimmed.find("&unsafe").unwrap_or(0) + 1,
                message: "Public function exposes &unsafe reference in its signature".to_string(),
                suggestion: Some("Consider using &T (tier 0) or &checked T (tier 1) in public APIs".into()),
                fixable: false,
            });
        }

        // Rule 20: shadow-binding
        if trimmed.starts_with("let ") || trimmed.starts_with("let mut ") {
            let var_start = if trimmed.starts_with("let mut ") { "let mut " } else { "let " };
            if let Some(var_name) = trimmed.strip_prefix(var_start).and_then(|s| s.split(|c: char| !c.is_alphanumeric() && c != '_').next()) {
                if !var_name.is_empty() && var_name != "_" {
                    // Check if same name was bound in an earlier line (not in inner scope)
                    for prev in &info.lines[..line_num] {
                        let pt = prev.trim();
                        if (pt.starts_with("let ") || pt.starts_with("let mut ")) && contains_word(pt, var_name) {
                            // Simple heuristic: only flag if same indentation level
                            let cur_indent = line.len() - line.trim_start().len();
                            let prev_indent = prev.len() - prev.trim_start().len();
                            if cur_indent == prev_indent {
                                issues.push(LintIssue {
                                    rule: "shadow-binding",
                                    level: LintLevel::Info,
                                    file: path.to_path_buf(),
                                    line: line_num + 1,
                                    column: 1,
                                    message: format!("Variable `{}` shadows previous binding", var_name),
                                    suggestion: Some(format!("rename the inner binding (e.g. `{var_name}2`)").into()),
                                    fixable: true,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Issue display
// ---------------------------------------------------------------------------

/// Print a lint issue with colored output
fn print_issue(issue: &LintIssue, deny_warnings: bool) {
    let level_str = match issue.level {
        LintLevel::Error => "error".red().bold(),
        LintLevel::Warning => {
            if deny_warnings {
                "error".red().bold()
            } else {
                "warning".yellow().bold()
            }
        }
        LintLevel::Info => "info".blue().bold(),
        LintLevel::Hint => "hint".dimmed(),
        LintLevel::Off => return,
    };

    let rule_display = issue.rule.dimmed();

    println!(
        "{}: {} [{}]",
        level_str, issue.message, rule_display
    );

    println!(
        "  {} {}:{}:{}",
        "-->".blue(),
        issue.file.display().to_string().cyan(),
        issue.line,
        issue.column
    );

    if let Some(ref suggestion) = issue.suggestion {
        println!("  {} {}", "help:".cyan().bold(), suggestion);
    }

    if issue.fixable {
        println!("  {} auto-fixable with {}", "note:".cyan(), "--fix".bold());
    }

    println!();
}

// ---------------------------------------------------------------------------
// Public API for workspace support
// ---------------------------------------------------------------------------

/// Run every text-scan + AST + custom rule against in-memory content
/// and return the raw diagnostics. The caller decides whether to
/// apply `effective_level` filtering (for tests we want to see every
/// issue at its default level).
///
/// Used by integration tests so per-rule fires/silent assertions
/// don't depend on disk I/O. Production code paths still go through
/// `lint_file` to keep the on-disk caching/paralleism story
/// centralised.
pub fn lint_source(
    path: &Path,
    content: &str,
    config: Option<&LintConfig>,
) -> List<LintIssue> {
    let mut issues = List::new();
    let info = FileInfo::parse(content);

    check_unchecked_refinement(path, &info, &mut issues);
    check_missing_context_decl(path, &info, &mut issues);
    check_unused_imports(path, &info, &mut issues);
    check_unnecessary_heap(path, &info, &mut issues);
    check_missing_error_context(path, &info, &mut issues);
    check_large_copy(path, &info, &mut issues);
    check_unused_result(path, &info, &mut issues);
    check_missing_cleanup(path, &info, &mut issues);
    check_deprecated_syntax(path, &info, &mut issues);
    check_cbgr_hotspot(path, &info, &mut issues);
    check_extended_rules(path, &info, &mut issues);

    if let Some(cfg) = config {
        for issue in lint_custom_rules(path, content, cfg) {
            issues.push(issue);
        }
    }

    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    let fid = FileId::new(0);
    let lexer = Lexer::new(content, fid);
    let parser = VerumParser::new();
    if let Ok(module) = parser.parse_module(lexer, fid) {
        let ctx = super::lint_engine::LintCtx {
            file: path,
            source: content,
            module: &module,
            config,
        };
        for issue in super::lint_engine::run(&ctx) {
            issues.push(issue);
        }

        let scopes = super::lint_engine::collect_suppressions(&module, content);
        if !scopes.is_empty() {
            let collected: Vec<_> = std::mem::take(&mut issues).into_iter().collect();
            let suppressed = super::lint_engine::apply_suppressions(collected, &scopes);
            for i in suppressed {
                issues.push(i);
            }
        }
    }

    issues
}

/// Convenience predicate for tests: does the diagnostic stream
/// contain at least one issue under `rule`?
pub fn has_issue(issues: &[LintIssue], rule: &str) -> bool {
    issues.iter().any(|i| i.rule == rule)
}

/// Lint specific path (for workspace support)
pub fn lint_path(path: &Path, fix: bool, deny_warnings: bool) -> Result<()> {
    if !path.exists() {
        return Err(CliError::Custom(format!(
            "Path not found: {}",
            path.display()
        )));
    }

    let mut all_issues = List::new();
    let mut total_files = 0;

    if path.is_file() {
        if is_verum_file(path) {
            total_files += 1;
            let issues = lint_file(path)?;
            all_issues.extend(issues);
        }
    } else if path.is_dir() {
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let file_path = entry.path();
            if file_path.is_file() && is_verum_file(file_path) {
                total_files += 1;
                let issues = lint_file(file_path)?;
                all_issues.extend(issues);
            }
        }
    }

    if all_issues.is_empty() {
        ui::success(&format!("No lint issues found in {} files", total_files));
        return Ok(());
    }

    // Print issues
    for issue in &all_issues {
        print_issue(issue, deny_warnings);
    }

    // Apply fixes if requested
    if fix {
        let fixable: List<&LintIssue> = all_issues.iter().filter(|i| i.fixable).collect();
        if !fixable.is_empty() {
            let mut fixes_by_file: HashMap<PathBuf, List<&LintIssue>> = HashMap::new();
            for issue in fixable.iter() {
                fixes_by_file
                    .entry(issue.file.clone())
                    .or_default()
                    .push(issue);
            }

            let mut fixed_count = 0;
            for (file_path, issues) in fixes_by_file {
                if let Ok(content) = fs::read_to_string(&file_path) {
                    let fixed_content = apply_fixes(&content, &issues);
                    if fixed_content != content && fs::write(&file_path, &fixed_content).is_ok() {
                        fixed_count += issues.len();
                    }
                }
            }
            ui::success(&format!("Fixed {} issues", fixed_count));
        }
    }

    let error_count = all_issues.iter().filter(|i| i.level == LintLevel::Error).count();
    let warning_count = all_issues.iter().filter(|i| i.level == LintLevel::Warning).count();

    if error_count > 0 {
        Err(CliError::Custom(format!(
            "Found {} lint errors",
            error_count
        )))
    } else if deny_warnings && warning_count > 0 {
        Err(CliError::Custom(format!(
            "Found {} lint warnings (denied)",
            warning_count
        )))
    } else {
        Ok(())
    }
}


// ===================================================================
// Phase A.1 extensions: --list-rules, --explain, --validate-config,
// --format json. These are additive on top of the existing surface.
// ===================================================================

/// Print every known built-in lint rule and exit. Used by
/// `verum lint --list-rules`.
pub fn list_rules() -> Result<()> {
    println!("{:<32} {:<8} {:<14} {}",
        "Name".bold(),
        "Level".bold(),
        "Category".bold(),
        "Description".bold());
    println!("{}", "-".repeat(110));
    let mut rules: Vec<&LintRule> = LINT_RULES.iter().collect();
    rules.sort_by_key(|r| r.name);
    for r in rules {
        let level = match r.level {
            LintLevel::Error => "error",
            LintLevel::Warning => "warn",
            LintLevel::Info => "info",
            LintLevel::Hint => "hint",
            LintLevel::Off => "off",
        };
        let cat = match r.category {
            LintCategory::Performance => "performance",
            LintCategory::Safety => "safety",
            LintCategory::Style => "style",
            LintCategory::Verification => "verification",
        };
        println!("{:<32} {:<8} {:<14} {}", r.name, level, cat, r.description);
    }
    println!();
    println!("Total: {} built-in rules.", LINT_RULES.len());
    Ok(())
}

/// `verum lint --list-groups` — print every lint group + its
/// member rules. Use as a discovery tool before adding
/// `extends = "verum::<group>"` to verum.toml.
pub fn list_groups() -> Result<()> {
    println!("{:<24} {}", "Group".bold(), "Members".bold());
    println!("{}", "-".repeat(80));
    for (group, members) in lint_groups() {
        if members.is_empty() {
            println!(
                "{:<24} {}",
                group,
                "(empty — populated as rules join the group)".dimmed()
            );
            continue;
        }
        // First member on the group line; subsequent members
        // wrapped under the same column.
        let mut iter = members.iter();
        if let Some(first) = iter.next() {
            println!("{:<24} {}", group, first);
        }
        for m in iter {
            println!("{:<24} {}", "", m);
        }
        println!();
    }
    println!("Use `extends = \"verum::<group>\"` in verum.toml to opt in.");
    Ok(())
}

/// Print extended documentation for one rule. Used by
/// `verum lint --explain <rule>`.
/// `verum lint --explain RULE --open` — opens the rule's online
/// docs page in the system browser. Resolves to
/// `https://verum-lang.dev/docs/reference/lint-rules#<rule>` and
/// dispatches the platform-appropriate "open URL" command:
///
/// - macOS: `open <url>`
/// - Linux: `xdg-open <url>`
/// - Windows: `cmd /c start "" <url>`
///
/// Verifies the rule name first so unknown rules return a clean
/// error rather than opening a 404 page.
pub fn explain_rule_open(name: &str) -> Result<()> {
    if !LINT_RULES.iter().any(|r| r.name == name) {
        let candidates: Vec<&str> = LINT_RULES
            .iter()
            .map(|r| r.name)
            .filter(|n| levenshtein(n, name) <= 2)
            .collect();
        if let Some(suggest) = candidates.first() {
            return Err(CliError::Custom(format!(
                "unknown lint rule `{}` — did you mean `{}`?",
                name, suggest
            )));
        }
        return Err(CliError::Custom(format!(
            "unknown lint rule `{}` (run `verum lint --list-rules` to see all)",
            name
        )));
    }
    let url = format!(
        "https://verum-lang.dev/docs/reference/lint-rules#{}",
        name
    );
    // Test hatch: when VERUM_OPEN_DRY_RUN=1 is set we just print
    // the URL instead of dispatching a real browser command. Lets
    // CI assert the URL without spawning anything.
    if std::env::var("VERUM_OPEN_DRY_RUN")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        println!("{}", url);
        return Ok(());
    }
    open_url(&url).map_err(|e| {
        CliError::Custom(format!(
            "failed to open `{}`: {}. \
             Open it manually if your environment has no browser.",
            url, e
        ))
    })?;
    ui::success(&format!("Opened {}", url));
    Ok(())
}

/// Cross-platform "open this URL in the user's default app".
/// Doesn't pull in a new crate — uses the platform-standard
/// command. Returns Err on spawn failure or non-zero exit.
fn open_url(url: &str) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    let status = cmd.status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("opener exited {}", status),
        ));
    }
    Ok(())
}

pub fn explain_rule(name: &str) -> Result<()> {
    let r = match LINT_RULES.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => {
            // Suggest a close match — Levenshtein-1 candidates.
            let candidates: Vec<&str> = LINT_RULES
                .iter()
                .map(|r| r.name)
                .filter(|n| levenshtein(n, name) <= 2)
                .collect();
            if let Some(suggest) = candidates.first() {
                return Err(CliError::Custom(format!(
                    "unknown lint rule `{}` — did you mean `{}`?",
                    name, suggest
                )));
            }
            return Err(CliError::Custom(format!(
                "unknown lint rule `{}` (run `verum lint --list-rules` to see all)",
                name
            )));
        }
    };
    let level = match r.level {
        LintLevel::Error => "error",
        LintLevel::Warning => "warn",
        LintLevel::Info => "info",
        LintLevel::Hint => "hint",
        LintLevel::Off => "off",
    };
    let cat = match r.category {
        LintCategory::Performance => "performance",
        LintCategory::Safety => "safety",
        LintCategory::Style => "style",
        LintCategory::Verification => "verification",
    };
    println!("{}", r.name.bold());
    println!();
    println!("  default level : {}", level);
    println!("  category      : {}", cat);
    println!();
    println!("{}", r.description);
    println!();
    println!("To suppress in source: `@allow({}, reason = \"...\")`", r.name);
    println!("To force-error:       `@deny({})`", r.name);
    println!();
    println!("Or in `verum.toml`:");
    println!();
    println!("  [lint.severity]");
    println!("  {} = \"off\"   # off | warn | error | info | hint", r.name);
    println!();
    println!("Full schema: docs/reference/lint-configuration");
    Ok(())
}

/// Run only config validation; useful in pre-commit hooks.
/// Exits 0 if `[lint]` block parses cleanly; non-0 with diagnostics
/// on any unknown key, malformed value, or referenced-but-undeclared
/// rule name.
pub fn validate_config() -> Result<()> {
    let cfg = load_full_lint_config();
    let mut errors: Vec<String> = Vec::new();

    // 1. extends preset must be a known name OR a `verum::<group>`
    //    handle. Both surfaces share the same load-time pipeline.
    if let Some(p) = &cfg.extends {
        let is_preset =
            matches!(p.as_str(), "minimal" | "recommended" | "strict" | "relaxed");
        let is_known_group = lint_groups().iter().any(|(name, _)| name == p);
        if !is_preset && !is_known_group {
            // Build did-you-mean over both surfaces so a typo on
            // either side gets a useful suggestion.
            let mut candidates: Vec<&str> = vec![
                "minimal", "recommended", "strict", "relaxed",
            ];
            for (name, _) in lint_groups() {
                candidates.push(name);
            }
            let suggest = candidates
                .iter()
                .min_by_key(|cand| levenshtein(cand, p))
                .copied()
                .unwrap_or("recommended");
            errors.push(format!(
                "unknown preset `extends = \"{}\"` — did you mean `\"{}\"`? \
                 (valid: minimal | recommended | strict | relaxed | verum::<group>)",
                p, suggest
            ));
        }
    }

    // 2. Collect unknown rule references across every config surface.
    let known: std::collections::HashSet<&'static str> =
        LINT_RULES.iter().map(|r| r.name).collect();
    let mut unknown: Vec<String> = Vec::new();
    for set in [&cfg.disabled, &cfg.denied, &cfg.allowed, &cfg.warned] {
        for name in set {
            if !known.contains(name.as_str())
                && !cfg.custom_rules.iter().any(|r| &r.name == name)
            {
                unknown.push(name.clone());
            }
        }
    }
    for name in cfg.severity_map.keys() {
        if !known.contains(name.as_str())
            && !cfg.custom_rules.iter().any(|r| &r.name == name)
        {
            unknown.push(name.clone());
        }
    }
    if !unknown.is_empty() {
        unknown.sort();
        unknown.dedup();
        let mut msg = String::new();
        msg.push_str("unknown lint rule(s) referenced:
");
        for n in &unknown {
            let suggestion = LINT_RULES
                .iter()
                .map(|r| r.name)
                .min_by_key(|cand| levenshtein(cand, n))
                .unwrap_or("");
            msg.push_str(&format!(
                "  - `{}`{}
",
                n,
                if !suggestion.is_empty() && levenshtein(suggestion, n) <= 3 {
                    format!(" — did you mean `{}`?", suggestion)
                } else {
                    String::new()
                }
            ));
        }
        errors.push(msg.trim_end().to_string());
    }

    if !errors.is_empty() {
        return Err(CliError::Custom(errors.join("\n\n")));
    }

    let preset_summary = cfg
        .extends
        .as_deref()
        .map(|p| format!(" (extends = `{}`)", p))
        .unwrap_or_default();
    ui::success(&format!(
        "lint config is valid ({} built-in + {} custom rules; {} severity overrides{})",
        LINT_RULES.len(),
        cfg.custom_rules.len(),
        cfg.severity_map.len(),
        preset_summary,
    ));
    Ok(())
}

/// Levenshtein distance — used for typo suggestions.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 { return n; }
    if n == 0 { return m; }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i-1] == b[j-1] { 0 } else { 1 };
            curr[j] = std::cmp::min(
                std::cmp::min(curr[j-1] + 1, prev[j] + 1),
                prev[j-1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Output format for `verum lint --format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintOutputFormat {
    Pretty,
    /// Span-underlined human output. The shape of `rustc` /
    /// `clippy` / `ruff`: rule code in brackets, file with `--> `,
    /// the offending source line, a caret underline at the column,
    /// and the help suggestion. Designed for human readers; CI
    /// logs survive ANSI stripping.
    Human,
    Json,
    GithubActions,
    Sarif,
    Tap,
}

impl LintOutputFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pretty" | "" => Ok(Self::Pretty),
            "human" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            "github-actions" | "gha" => Ok(Self::GithubActions),
            "sarif" => Ok(Self::Sarif),
            "tap" => Ok(Self::Tap),
            other => Err(CliError::InvalidArgument(format!(
                "unknown lint format `{}` (expected: pretty | human | json | github-actions | sarif | tap)",
                other
            ))),
        }
    }
}

/// Render a single issue as one NDJSON line.
///
/// `schema_version: 1` is the stable contract — every documented
/// field below MUST be present on every line; new fields may be
/// added in a backward-compatible fashion. A consumer parsing this
/// stream can rely on:
///
/// - `event`: always `"lint"` for issue lines.
/// - `schema_version`: integer; bump = breaking change.
/// - `rule`, `level`, `file`, `line`, `column`, `message`, `fixable`.
/// - `suggestion`: present only when `fixable` is true.
pub fn format_issue_json(issue: &LintIssue) -> String {
    let level = match issue.level {
        LintLevel::Error => "error",
        LintLevel::Warning => "warning",
        LintLevel::Info => "info",
        LintLevel::Hint => "hint",
        LintLevel::Off => "off",
    };
    let suggestion = issue
        .suggestion
        .as_ref()
        .map(|t| format!(",\"suggestion\":{}", json_string(t.as_str())))
        .unwrap_or_default();
    format!(
        "{{\"event\":\"lint\",\"schema_version\":1,\"rule\":{rule},\"level\":\"{lvl}\",\"file\":{file},\"line\":{ln},\"column\":{col},\"message\":{msg},\"fixable\":{fixable}{suggestion}}}",
        rule = json_string(issue.rule),
        lvl = level,
        file = json_string(&issue.file.display().to_string()),
        ln = issue.line,
        col = issue.column,
        msg = json_string(&issue.message),
        fixable = issue.fixable,
        suggestion = suggestion,
    )
}

fn emit_issue_json(issue: &LintIssue) {
    println!("{}", format_issue_json(issue));
}

/// Render a single issue as a GitHub Actions workflow annotation.
/// Format: `::warning file=path,line=N,col=M::message`. Off-level
/// issues return an empty string (no output).
pub fn format_issue_gha(issue: &LintIssue) -> String {
    let level = match issue.level {
        LintLevel::Error => "error",
        LintLevel::Warning => "warning",
        LintLevel::Info => "notice",
        LintLevel::Hint => "notice",
        LintLevel::Off => return String::new(),
    };
    format!(
        "::{lvl} file={file},line={ln},col={col},title={title}::{msg}",
        lvl = level,
        file = issue.file.display(),
        ln = issue.line,
        col = issue.column,
        title = issue.rule,
        msg = issue.message.replace('\n', "%0A"),
    )
}

fn emit_issue_gha(issue: &LintIssue) {
    let line = format_issue_gha(issue);
    if !line.is_empty() {
        println!("{}", line);
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Public entry point: emit all collected issues using the chosen
/// format. Wired from `Commands::Lint` in main.rs when a non-pretty
/// format is requested.
pub fn run_with_format(fix: bool, deny_warnings: bool, format: LintOutputFormat) -> Result<()> {
    if format == LintOutputFormat::Pretty {
        return execute(fix, deny_warnings);
    }

    // Replicate the discovery / lint-collection flow with structured emission.
    let cfg = load_full_lint_config();
    let mut all_issues: Vec<LintIssue> = Vec::new();

    let search_dirs: Vec<PathBuf> = ["src", "core"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    if search_dirs.is_empty() {
        return Err(CliError::Custom("No src/ or core/ directory found".into()));
    }

    let (parallel_issues, _files_seen) = lint_paths_parallel(&search_dirs, &cfg);
    for mut i in parallel_issues {
        if let Some(lvl) = cfg.effective_level(i.rule, i.level) {
            i.level = lvl;
            all_issues.push(i);
        }
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    for issue in &all_issues {
        match issue.level {
            LintLevel::Error => errors += 1,
            LintLevel::Warning => warnings += 1,
            _ => {}
        }
    }
    emit_issues(&all_issues, format)?;

    let _ = fix; // auto-fix in machine modes is handled by the pretty runner.
    if errors > 0 {
        Err(CliError::Custom(format!("{} lint errors", errors)))
    } else if deny_warnings && warnings > 0 {
        Err(CliError::Custom(format!("{} lint warnings (denied)", warnings)))
    } else {
        Ok(())
    }
}

/// Batch-aware issue emission. Per-issue formats (Json, GitHub
/// Actions) iterate; document-level formats (SARIF, TAP) emit a
/// single payload for the whole run.
fn emit_issues(issues: &[LintIssue], format: LintOutputFormat) -> Result<()> {
    match format {
        LintOutputFormat::Pretty => {
            for i in issues {
                print_issue(i, false);
            }
        }
        LintOutputFormat::Human => {
            // Span-underlined output. The renderer caches source
            // file reads so a corpus with N issues across M files
            // touches each file exactly once.
            let mut sources = super::lint_human::SourceMap::new();
            for i in issues {
                print!("{}", super::lint_human::render_issue(i, &mut sources));
            }
        }
        LintOutputFormat::Json => {
            for i in issues {
                emit_issue_json(i);
            }
        }
        LintOutputFormat::GithubActions => {
            for i in issues {
                emit_issue_gha(i);
            }
        }
        LintOutputFormat::Sarif => emit_sarif(issues),
        LintOutputFormat::Tap => emit_tap(issues),
    }
    Ok(())
}

/// SARIF 2.1.0 emitter — one driver "verum-lint", every shipped rule
/// listed under tool.driver.rules, every issue as a result with
/// physicalLocation.artifactLocation + region.startLine/Column.
/// Render the entire batch of issues as a SARIF 2.1.0 document.
/// Single document per run; consumers (GitHub Code Scanning, Azure
/// DevOps) treat one SARIF JSON object as one analysis run.
pub fn format_sarif(issues: &[LintIssue]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"$schema\": \"https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json\",\n");
    s.push_str("  \"version\": \"2.1.0\",\n");
    s.push_str("  \"runs\": [{\n");
    s.push_str("    \"tool\": { \"driver\": {\n");
    s.push_str("      \"name\": \"verum-lint\",\n");
    s.push_str("      \"informationUri\": \"https://verum-lang.dev\",\n");
    s.push_str("      \"rules\": [\n");
    let mut first_rule = true;
    for r in LINT_RULES {
        if !first_rule {
            s.push_str(",\n");
        }
        first_rule = false;
        let _ = write!(
            s,
            "        {{ \"id\": {}, \"shortDescription\": {{ \"text\": {} }} }}",
            json_string(r.name),
            json_string(r.description),
        );
    }
    s.push_str("\n      ]\n    } },\n");
    s.push_str("    \"results\": [\n");
    let mut first_res = true;
    for i in issues {
        if !first_res {
            s.push_str(",\n");
        }
        first_res = false;
        let level = match i.level {
            LintLevel::Error => "error",
            LintLevel::Warning => "warning",
            LintLevel::Info => "note",
            LintLevel::Hint => "note",
            LintLevel::Off => continue,
        };
        let _ = write!(
            s,
            "      {{ \"ruleId\": {}, \"level\": \"{}\", \"message\": {{ \"text\": {} }}, \
             \"locations\": [{{ \"physicalLocation\": {{ \
             \"artifactLocation\": {{ \"uri\": {} }}, \
             \"region\": {{ \"startLine\": {}, \"startColumn\": {} }} }} }}] }}",
            json_string(i.rule),
            level,
            json_string(&i.message),
            json_string(&i.file.display().to_string()),
            i.line,
            i.column,
        );
    }
    s.push_str("\n    ]\n");
    s.push_str("  }]\n");
    s.push_str("}\n");
    s
}

fn emit_sarif(issues: &[LintIssue]) {
    print!("{}", format_sarif(issues));
}

/// TAP v13 — `TAP version 13`, `1..N` plan, `ok N - msg` / `not ok
/// N - msg` per issue with a YAML diagnostic block on failures.
/// `not ok` is emitted for Error and Warning; Info/Hint are `ok`
/// with `# SKIP info` so strict TAP consumers don't fail the run.
pub fn format_tap(issues: &[LintIssue]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "TAP version 13");
    let _ = writeln!(out, "1..{}", issues.len());
    for (idx, i) in issues.iter().enumerate() {
        let n = idx + 1;
        let directive = match i.level {
            LintLevel::Error | LintLevel::Warning => "not ok",
            LintLevel::Info | LintLevel::Hint => "ok",
            LintLevel::Off => continue,
        };
        let skip = matches!(i.level, LintLevel::Info | LintLevel::Hint);
        if skip {
            let _ = writeln!(
                out,
                "{} {} - {} [{}] # SKIP {} ({})",
                directive,
                n,
                i.message,
                i.rule,
                i.level.as_str(),
                i.file.display()
            );
        } else {
            let _ = writeln!(out, "{} {} - {} [{}]", directive, n, i.message, i.rule);
            let _ = writeln!(out, "  ---");
            let _ = writeln!(out, "  rule: {}", i.rule);
            let _ = writeln!(out, "  level: {}", i.level.as_str());
            let _ = writeln!(out, "  file: {}", i.file.display());
            let _ = writeln!(out, "  line: {}", i.line);
            let _ = writeln!(out, "  column: {}", i.column);
            let _ = writeln!(out, "  ...");
        }
    }
    out
}

fn emit_tap(issues: &[LintIssue]) {
    print!("{}", format_tap(issues));
}

/// Phase A.3 extended runner — applies named profiles, --since
/// git-diff filtering, and --severity post-filtering on top of
/// `run_with_format`. Always falls through to the per-format emitter
/// `run_with_format` for json/github-actions, or the existing
/// `execute` for pretty (with the new layers wired in).
/// CLI entry: `verum lint --watch`. Lints once, then enters a debounced
/// file-watch loop that re-lints on every `.vr` change. Uses the same
/// per-file cache as one-shot runs, so untouched files cost ~nothing
/// per iteration.
///
/// The loop never returns an error to the caller — Ctrl-C is the
/// signal to exit. Lint failures are printed and the watch resumes;
/// build crashes never abort the watch.
pub fn run_watch(
    fix: bool,
    deny_warnings: bool,
    format: LintOutputFormat,
    profile: Option<String>,
    since: Option<String>,
    severity_min: Option<LintLevel>,
    clear: bool,
) -> Result<()> {
    use notify::{Event, RecursiveMode, Watcher};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::{Duration, Instant};

    // Initial run — establish the cache and surface every issue.
    print_watch_banner(clear, "initial scan");
    let _ = run_extended(
        fix,
        deny_warnings,
        format,
        profile.clone(),
        since.clone(),
        severity_min,
    );

    let (tx, rx) = channel::<Event>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(ev) = res {
            let _ = tx.send(ev);
        }
    })?;

    // Watch every existing search root recursively.
    for d in ["src", "core"] {
        let p = PathBuf::from(d);
        if p.exists() {
            watcher.watch(&p, RecursiveMode::Recursive)?;
        }
    }
    watcher.watch(Path::new("verum.toml"), RecursiveMode::NonRecursive).ok();

    ui::step("Watching for changes — press Ctrl-C to exit.");

    // Debounce: after a burst of events, wait DEBOUNCE_MS of quiet
    // before triggering a re-lint. Tracks both the last event arrival
    // time and whether at least one event has accumulated.
    const DEBOUNCE: Duration = Duration::from_millis(300);

    loop {
        // Block until the first event arrives.
        let first = match rx.recv() {
            Ok(ev) => ev,
            Err(_) => break,
        };
        if !is_relevant_event(&first) {
            continue;
        }

        // Drain follow-up events for DEBOUNCE_MS.
        let mut last_event = Instant::now();
        let mut saw_any = true;
        while saw_any {
            let remaining = DEBOUNCE.saturating_sub(last_event.elapsed());
            match rx.recv_timeout(remaining) {
                Ok(ev) => {
                    if is_relevant_event(&ev) {
                        last_event = Instant::now();
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            saw_any = last_event.elapsed() < DEBOUNCE;
        }

        print_watch_banner(clear, "change detected, re-running");
        let _ = run_extended(
            fix,
            deny_warnings,
            format,
            profile.clone(),
            since.clone(),
            severity_min,
        );
        ui::step("Watching for changes — press Ctrl-C to exit.");
    }
    Ok(())
}

fn print_watch_banner(clear: bool, label: &str) {
    if clear {
        // ANSI clear-screen + cursor-home. Avoids piling up scrollback
        // in long watch sessions.
        print!("\x1B[2J\x1B[H");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ui::step(&format!("verum lint --watch [{}] {}", now, label));
}

fn is_relevant_event(ev: &notify::Event) -> bool {
    use notify::EventKind;
    matches!(
        ev.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && ev
        .paths
        .iter()
        .any(|p| {
            p.extension().and_then(|s| s.to_str()) == Some("vr")
                || p.file_name().and_then(|s| s.to_str()) == Some("verum.toml")
        })
}

/// Backwards-compatible entry — callers that don't pass a
/// warning budget still work. Forwards to the full version with
/// `max_warnings = None`.
pub fn run_extended(
    fix: bool,
    deny_warnings: bool,
    format: LintOutputFormat,
    profile: Option<String>,
    since: Option<String>,
    severity_min: Option<LintLevel>,
) -> Result<()> {
    run_extended_full(fix, deny_warnings, format, profile, since, severity_min, None)
}

/// Resolved baseline mode for one run.
///
/// The CLI surface has three knobs (`--baseline FILE`, `--no-baseline`,
/// `--write-baseline`) that combine into one of these states.
pub enum BaselineMode {
    /// No baseline applied. Either `--no-baseline` was set, or
    /// none of the baseline flags were used and no default
    /// baseline file exists.
    Disabled,
    /// Read suppressions from PATH; live issues that match a
    /// baseline entry are silenced.
    Read(PathBuf),
    /// Snapshot this run's issue set to PATH and exit 0 regardless
    /// of issue count.
    Write(PathBuf),
}

impl BaselineMode {
    /// Resolve the three CLI flags into a single mode.
    /// Precedence: `--write-baseline` wins over `--baseline FILE`,
    /// which wins over the default-path lookup. `--no-baseline`
    /// disables read-mode (write-mode still works).
    pub fn from_flags(
        baseline: Option<String>,
        no_baseline: bool,
        write_baseline: bool,
    ) -> Self {
        let path = match &baseline {
            Some(p) => PathBuf::from(p),
            None => super::lint_baseline::default_path(),
        };
        if write_baseline {
            return BaselineMode::Write(path);
        }
        if no_baseline {
            return BaselineMode::Disabled;
        }
        if baseline.is_some() || path.exists() {
            BaselineMode::Read(path)
        } else {
            BaselineMode::Disabled
        }
    }
}

/// Full extended runner with the `--max-warnings N` budget +
/// baseline. Forwards from older entry points with the new
/// argument defaulted to `Disabled`.
pub fn run_extended_full_with_baseline(
    fix: bool,
    deny_warnings: bool,
    format: LintOutputFormat,
    profile: Option<String>,
    since: Option<String>,
    severity_min: Option<LintLevel>,
    max_warnings: Option<usize>,
    baseline: BaselineMode,
) -> Result<()> {
    run_extended_inner(
        fix,
        deny_warnings,
        format,
        profile,
        since,
        severity_min,
        max_warnings,
        baseline,
    )
}

/// Full extended runner with the `--max-warnings N` budget.
/// `max_warnings = None` means no cap; `Some(0)` is equivalent to
/// `--deny-warnings` (any warning fails the run); `Some(N>0)` lets
/// up to N warnings through before failing.
pub fn run_extended_full(
    fix: bool,
    deny_warnings: bool,
    format: LintOutputFormat,
    profile: Option<String>,
    since: Option<String>,
    severity_min: Option<LintLevel>,
    max_warnings: Option<usize>,
) -> Result<()> {
    run_extended_inner(
        fix,
        deny_warnings,
        format,
        profile,
        since,
        severity_min,
        max_warnings,
        BaselineMode::Disabled,
    )
}

fn run_extended_inner(
    fix: bool,
    deny_warnings: bool,
    format: LintOutputFormat,
    profile: Option<String>,
    since: Option<String>,
    severity_min: Option<LintLevel>,
    max_warnings: Option<usize>,
    baseline_mode: BaselineMode,
) -> Result<()> {
    let mut cfg = load_full_lint_config();
    if let Some(name) = &profile {
        apply_profile(&mut cfg, name)?;
    }

    // --since: get changed-file allowlist via `git diff --name-only`.
    let changed_files: Option<HashSet<PathBuf>> = match &since {
        Some(git_ref) => match changed_vr_files_since(git_ref) {
            Ok(set) => Some(set),
            Err(e) => {
                ui::warn(&format!("--since {}: {}", git_ref, e));
                Some(HashSet::new())
            }
        },
        None => None,
    };

    let search_dirs: Vec<PathBuf> = ["src", "core"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    if search_dirs.is_empty() {
        return Err(CliError::Custom("No src/ or core/ directory found".into()));
    }

    // Build the file list with the --since filter pre-applied. The
    // parallel runner then sees only the work it needs to do.
    let files: Vec<PathBuf> = search_dirs
        .iter()
        .flat_map(|d| {
            WalkDir::new(d)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && is_verum_file(p))
                .collect::<Vec<_>>()
        })
        .filter(|p| match &changed_files {
            Some(allow) => allow.iter().any(|q| p.ends_with(q) || q == p),
            None => true,
        })
        .collect();

    use rayon::prelude::*;
    let cache = build_lint_cache(&cfg);
    cache.gc();
    let raw_issues: Vec<LintIssue> = files
        .par_iter()
        .flat_map(|path| lint_one_with_cache(path, &cfg, &cache))
        .collect();

    // Cross-file phase. Re-parses each file once to feed the
    // corpus-level passes (circular-import / orphan-module /
    // unused-public). The per-file phase already paid for the
    // parser warm-up so this is incremental work.
    let cross_issues = run_cross_file_phase(&files, &cfg);

    let mut all_issues: Vec<LintIssue> = raw_issues
        .into_iter()
        .chain(cross_issues.into_iter())
        .filter_map(|mut i| {
            let lvl = cfg.effective_level_for_file(i.rule, &i.file, i.level)?;
            if let Some(min) = severity_min {
                if !meets_severity(lvl, min) {
                    return None;
                }
            }
            i.level = lvl;
            Some(i)
        })
        .collect();
    all_issues.sort_by(|a, b| issue_sort_key(a).cmp(&issue_sort_key(b)));

    // --write-baseline path. Snapshot the current set and exit 0,
    // regardless of how many issues there are. Output is suppressed
    // beyond a one-line success because the user is asking us to
    // record state, not report it.
    if let BaselineMode::Write(ref path) = baseline_mode {
        super::lint_baseline::Baseline::write(path, &all_issues)
            .map_err(|e| CliError::Custom(format!("write baseline {}: {}", path.display(), e)))?;
        ui::success(&format!(
            "wrote baseline ({} issues) → {}",
            all_issues.len(),
            path.display()
        ));
        return Ok(());
    }

    // --baseline FILE / default-path lookup. Filter out issues that
    // match a baseline entry. Track suppressed count so the user
    // sees the savings.
    if let BaselineMode::Read(ref path) = baseline_mode {
        if let Some(baseline) = super::lint_baseline::Baseline::load(path) {
            let before = all_issues.len();
            all_issues.retain(|i| !baseline.suppresses(i));
            let suppressed = before - all_issues.len();
            if suppressed > 0 {
                ui::note(&format!(
                    "{} issues suppressed by baseline ({})",
                    suppressed,
                    path.display()
                ));
            }
        } else if path.exists() {
            // The file exists but failed to parse — alert the user
            // rather than silently dropping the suppression set.
            ui::warn(&format!(
                "baseline at {} could not be loaded; running without suppressions",
                path.display()
            ));
        }
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    for issue in &all_issues {
        match issue.level {
            LintLevel::Error => errors += 1,
            LintLevel::Warning => warnings += 1,
            _ => {}
        }
    }
    emit_issues(&all_issues, format)?;

    if format == LintOutputFormat::Pretty {
        // Summary block consistent with execute()'s pretty output.
        println!();
        println!("{}", "Lint Summary:".bold());
        println!("  Issues: {}", all_issues.len());
        if errors > 0 {
            println!("  {}: {}", "Errors".red().bold(), errors);
        }
        if warnings > 0 {
            println!("  {}: {}", "Warnings".yellow().bold(), warnings);
        }
        if let Some(p) = &profile {
            println!("  Profile: {}", p.cyan());
        }
        if let Some(s) = &since {
            println!("  Since: {}", s.cyan());
        }
        println!();
    }

    if fix {
        apply_autofix_run(&all_issues);
    }

    if errors > 0 {
        return Err(CliError::Custom(format!("{} lint errors", errors)));
    }
    // --max-warnings N supersedes --deny-warnings when both are
    // present (N=0 is exactly --deny-warnings semantics, so the
    // user gets identical behaviour either way).
    if let Some(budget) = max_warnings {
        if warnings > budget {
            return Err(CliError::Custom(format!(
                "{} warnings exceeds budget of {} (--max-warnings)",
                warnings, budget
            )));
        }
    } else if deny_warnings && warnings > 0 {
        return Err(CliError::Custom(format!(
            "{} lint warnings (denied)",
            warnings
        )));
    }
    Ok(())
}

/// Apply auto-fixes to disk for every `fixable` issue in `issues`.
/// Issues are grouped per-file so a file is rewritten exactly once
/// regardless of how many fixable issues it carries.
fn apply_autofix_run(issues: &[LintIssue]) {
    let fixable: Vec<&LintIssue> = issues.iter().filter(|i| i.fixable).collect();
    if fixable.is_empty() {
        return;
    }
    let mut by_file: HashMap<PathBuf, List<&LintIssue>> = HashMap::new();
    for issue in &fixable {
        by_file
            .entry(issue.file.clone())
            .or_default()
            .push(*issue);
    }
    let mut fixed = 0usize;
    for (path, file_issues) in by_file {
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let new_content = apply_fixes(&content, &file_issues);
        if new_content != content && fs::write(&path, &new_content).is_ok() {
            fixed += file_issues.len();
        }
    }
    ui::success(&format!("Fixed {} issues", fixed));
}

/// Whether `level` meets the `min` severity bar — error > warn > info > hint > off.
fn meets_severity(level: LintLevel, min: LintLevel) -> bool {
    fn rank(l: LintLevel) -> u8 {
        match l {
            LintLevel::Error => 4,
            LintLevel::Warning => 3,
            LintLevel::Info => 2,
            LintLevel::Hint => 1,
            LintLevel::Off => 0,
        }
    }
    rank(level) >= rank(min)
}

/// Run `git diff --name-only <REF>...HEAD -- '*.vr'` and collect the
/// resulting paths. The `...` form is "in HEAD but not in REF" — i.e.
/// what the current branch added on top of the merge base. For a
/// pre-commit hook against the current working tree, use
/// `git diff --name-only HEAD -- '*.vr'` (no triple-dot).
fn changed_vr_files_since(git_ref: &str) -> std::result::Result<HashSet<PathBuf>, String> {
    let out = std::process::Command::new("git")
        .args([
            "diff",
            "--name-only",
            &format!("{}...HEAD", git_ref),
            "--",
            "*.vr",
        ])
        .output()
        .map_err(|e| format!("failed to spawn git: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git exited with code {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect())
}
