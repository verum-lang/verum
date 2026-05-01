//! `kernel_v0` bootstrap-meta-theory manifest (#154 / Phase 3 of
//! Milawa-style trust-base shrinkage).
//!

//! # Architectural role
//!

//! Verum's trusted base shrinks across stages:
//!

//! ```text
//!  Pre-#157: 10K LOC `verum_kernel` Rust + 38 rules / 34 admits
//!  Post-#157: 796 LOC `proof_checker.rs` Rust + 7 rules
//!  Phase 3 : 500 LOC Verum + 10 rules (this manifest's target)
//!  Phase 3 ✓: 100 LOC bootstrap shim (Rust interpreter of kernel_v0)
//! ```
//!

//! The Verum-side mirror lives at
//! [`core/verify/kernel_v0/`](https://github.com/oldman/verum/tree/main/core/verify/kernel_v0).
//! It carries one file per kernel rule (`rules/k_*.vr`) plus
//! supporting infrastructure (`core_term.vr`, `context.vr`,
//! `judgment.vr`, `soundness.vr`).
//!

//! This manifest is the **Rust-side static record** of that
//! directory's structure. It serves three load-bearing purposes:
//!

//! 1. **Drift gate**: if `proof_checker.rs` adds a kernel rule
//!  that `kernel_v0/rules/` doesn't mirror, the audit fails.
//!  Symmetric: a rule file under `kernel_v0/rules/` that has no
//!  counterpart in the manifest also fails.
//! 2. **Roster surface**: `verum audit --kernel-v0-roster` walks
//!  this manifest + checks the corresponding `.vr` files exist
//!  on disk + classifies each rule's discharge status.
//! 3. **Bootstrap claim**: third-party reviewers ask "what do I
//!  need to trust?" — this manifest is the canonical answer for
//!  the Verum-side trusted-base contents.
//!

//! ## What this manifest is NOT
//!

//! It's not the kernel logic itself — that lives in the .vr files
//! and (currently) in `proof_checker.rs`. The manifest is a
//! cross-cutting record that lets audit gates verify the Rust ↔
//! Verum mirror invariant without parsing the .vr files.
//!

//! Once the Verum compiler matures enough that `kernel_v0/` is
//! fully self-checking (the parse errors at
//! `kernel_v0/soundness.vr` are tracked separately), this manifest
//! can be regenerated from the .vr source and pinned by content
//! hash — closing the loop between the Verum source and the Rust
//! shim's view of it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// =============================================================================
// KernelV0Status — discharge classification
// =============================================================================

/// Discharge status for one bootstrap-meta-theory rule. Mirrors
/// [`super::LemmaStatus`] but is **manifest-local**: pre-this-module
/// the canonical-rules roster lived only in
/// [`super::canonical_rules`] (38 rules of the broader kernel),
/// while `kernel_v0/`'s 10-rule subset had no programmatic surface.
/// Adding it here keeps the bootstrap-manifest data layer
/// self-contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelV0Status {
    /// Soundness lemma proved structurally — no IOU.
    Proved,
    /// Soundness lemma admitted with a structural-property IOU
    /// (substitution-lemma, β-confluence, etc.).
    Admitted,
}

impl KernelV0Status {
    /// Stable diagnostic tag — matches the serde representation.
    pub fn tag(self) -> &'static str {
        match self {
            KernelV0Status::Proved => "proved",
            KernelV0Status::Admitted => "admitted",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            KernelV0Status::Proved => "Proved",
            KernelV0Status::Admitted => "Admitted",
        }
    }
}

impl std::fmt::Display for KernelV0Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// =============================================================================
// KernelV0Rule — one row in the manifest
// =============================================================================

/// One bootstrap-meta-theory rule + its file location + its
/// discharge status. The `file_path` is RELATIVE to a Verum project
/// root (e.g., `core/verify/kernel_v0/rules/k_var.vr`); the audit
/// gate joins it with the project's `manifest_dir` to verify
/// existence on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelV0Rule {
    /// Stable rule identifier — `"K-Var"` / `"K-Univ"` / `"K-Pi-Form"`
    /// / etc. Matches the rule-name column in
    /// [`super::canonical_rules`].
    pub name: String,
    /// Verum-side soundness-lemma symbol — what `kernel_v0/soundness.vr`
    /// imports from each `rules/k_*.vr` file (e.g., `"k_var_sound"`).
    pub lemma_symbol: String,
    /// Project-relative path to the rule's source file.
    pub file_path: PathBuf,
    /// Discharge status.
    pub status: KernelV0Status,
    /// One-line description of what the rule asserts.
    pub description: String,
    /// IOU citation when `Admitted`; empty when `Proved`.
    pub iou_citation: String,
}

// =============================================================================
// Manifest
// =============================================================================

/// The canonical 10-rule kernel_v0 manifest. Mirrors
/// `core/verify/kernel_v0/README.md`'s "10 minimal rules" table.
///

/// **Stable contract**: this list is the single source of truth for
/// the Rust ↔ Verum bootstrap-mirror invariant. Adding a rule
/// requires:
///

///  1. Creating `core/verify/kernel_v0/rules/k_<name>.vr` with the
///  `introduce_<name>` constructor + `k_<name>_sound` soundness
///  lemma.
///  2. Adding the rule to `proof_checker.rs` (or another suitable
///  Rust-side trusted base).
///  3. Adding the entry to this manifest.
///

/// Drift across these three sites is the audit failure mode.
pub fn manifest() -> Vec<KernelV0Rule> {
    // Path relative to the Verum project root (the directory
    // containing `verum.toml`). For the canonical Verum stdlib
    // project, that's `core/`, and the kernel_v0 tree lives at
    // `verify/kernel_v0/rules/` underneath it. The audit gate
    // joins this against `Manifest::find_manifest_dir()`.
    let rules_dir = PathBuf::from("verify/kernel_v0/rules");
    vec![
        KernelV0Rule {
            name: "K-Var".to_string(),
            lemma_symbol: "k_var_sound".to_string(),
            file_path: rules_dir.join("k_var.vr"),
            status: KernelV0Status::Proved,
            description: "Variable lookup in context".to_string(),
            iou_citation: String::new(),
        },
        KernelV0Rule {
            name: "K-Univ".to_string(),
            lemma_symbol: "k_univ_sound".to_string(),
            file_path: rules_dir.join("k_univ.vr"),
            status: KernelV0Status::Proved,
            description: "Universe stratification: Universe(n) : Universe(n+1)".to_string(),
            iou_citation: String::new(),
        },
        KernelV0Rule {
            name: "K-Pi-Form".to_string(),
            lemma_symbol: "k_pi_form_sound".to_string(),
            file_path: rules_dir.join("k_pi_form.vr"),
            status: KernelV0Status::Admitted,
            description: "Pi-type formation: (A:U(n))→(B:U(m)) lives in U(max(n,m))".to_string(),
            iou_citation: "kind-preservation lemma for Π-forms (textbook CIC)".to_string(),
        },
        KernelV0Rule {
            name: "K-Lam-Intro".to_string(),
            lemma_symbol: "k_lam_intro_sound".to_string(),
            file_path: rules_dir.join("k_lam_intro.vr"),
            status: KernelV0Status::Admitted,
            description: "Lambda introduction: body-type-under-binder gives Pi type".to_string(),
            iou_citation: "context-extension lemma (substitution-stable)".to_string(),
        },
        KernelV0Rule {
            name: "K-App-Elim".to_string(),
            lemma_symbol: "k_app_elim_sound".to_string(),
            file_path: rules_dir.join("k_app_elim.vr"),
            status: KernelV0Status::Admitted,
            description: "Apply elimination + substitution".to_string(),
            iou_citation: "substitution-lemma (textbook CIC)".to_string(),
        },
        KernelV0Rule {
            name: "K-Beta".to_string(),
            lemma_symbol: "k_beta_sound".to_string(),
            file_path: rules_dir.join("k_beta.vr"),
            status: KernelV0Status::Admitted,
            description: "Beta-reduction (λx.M) N ⤳ M[N/x] is type-preserving".to_string(),
            iou_citation: "β-confluence + type-preservation (Church-Rosser)".to_string(),
        },
        KernelV0Rule {
            name: "K-Eta".to_string(),
            lemma_symbol: "k_eta_sound".to_string(),
            file_path: rules_dir.join("k_eta.vr"),
            status: KernelV0Status::Admitted,
            description: "Eta-equivalence λx.(f x) ≡ f when x ∉ FV(f)".to_string(),
            iou_citation: "function-extensionality (Hofmann-Streicher 1996)".to_string(),
        },
        KernelV0Rule {
            name: "K-Sub".to_string(),
            lemma_symbol: "k_sub_sound".to_string(),
            file_path: rules_dir.join("k_sub.vr"),
            status: KernelV0Status::Admitted,
            description: "Subtyping (universe cumulativity)".to_string(),
            iou_citation: "cumulativity preservation across reduction".to_string(),
        },
        KernelV0Rule {
            name: "K-FwAx".to_string(),
            lemma_symbol: "k_fwax_sound".to_string(),
            file_path: rules_dir.join("k_fwax.vr"),
            status: KernelV0Status::Proved,
            description: "Foundation-aware axiom admission (Prop-only)".to_string(),
            iou_citation: String::new(),
        },
        KernelV0Rule {
            name: "K-Pos".to_string(),
            lemma_symbol: "k_pos_sound".to_string(),
            file_path: rules_dir.join("k_pos.vr"),
            status: KernelV0Status::Proved,
            description: "Positivity check (Berardi 1998 — non-positive ⇒ ⊥)".to_string(),
            iou_citation: String::new(),
        },
    ]
}

/// Canonical count of bootstrap-meta-theory rules. Matches the
/// "10 minimal rules" table in `core/verify/kernel_v0/README.md`.
pub const KERNEL_V0_RULE_COUNT: usize = 10;

/// Number of rules currently in `Proved` status (no IOU).
pub fn proved_count() -> usize {
    manifest()
        .iter()
        .filter(|r| r.status == KernelV0Status::Proved)
        .count()
}

/// Number of rules currently in `Admitted` status (with structural
/// IOU).
pub fn admitted_count() -> usize {
    manifest()
        .iter()
        .filter(|r| r.status == KernelV0Status::Admitted)
        .count()
}

// =============================================================================
// Manifest verification
// =============================================================================

/// One issue found by the manifest-verification pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestIssue {
    /// The manifest names a file that doesn't exist on disk. Either
    /// the rule was deleted without updating the manifest, or the
    /// manifest entry has the wrong path.
    MissingSourceFile {
        /// Rule name.
        rule_name: String,
        /// Expected file path (project-relative).
        expected_path: PathBuf,
    },
    /// A `kernel_v0/rules/k_*.vr` file exists on disk but no manifest
    /// entry references it. Either a new rule was added without
    /// updating the manifest, or the file is stale.
    OrphanSourceFile {
        /// Path of the orphan file.
        path: PathBuf,
    },
}

/// Walk the project's `core/verify/kernel_v0/rules/` directory and
/// cross-reference against the manifest. Returns one
/// [`ManifestIssue`] per drift point detected. An empty result
/// means the Rust ↔ Verum mirror is consistent.
///

/// `project_root` should be the project's `manifest_dir` — the
/// directory containing `verum.toml`. The function joins
/// project-relative paths in the manifest against this root.
pub fn verify_manifest(project_root: &Path) -> Vec<ManifestIssue> {
    let mut issues = Vec::new();
    let m = manifest();

    // Pass 1: every manifest entry must have a corresponding file.
    for rule in &m {
        let abs = project_root.join(&rule.file_path);
        if !abs.is_file() {
            issues.push(ManifestIssue::MissingSourceFile {
                rule_name: rule.name.clone(),
                expected_path: rule.file_path.clone(),
            });
        }
    }

    // Pass 2: every k_*.vr file under kernel_v0/rules/ must be in
    // the manifest. Surfaces orphans that drifted out of the
    // bootstrap chain without a manifest update.
    let rules_dir = project_root.join("verify/kernel_v0/rules");
    if let Ok(entries) = std::fs::read_dir(&rules_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Only inspect k_*.vr files; mod.vr, README.md, etc.
            // aren't manifest subjects.
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if s.starts_with("k_") => s,
                _ => continue,
            };
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "vr")
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let _ = stem; // stem unused after extension check
            let in_manifest = m.iter().any(|r| {
                project_root.join(&r.file_path).canonicalize().ok() == path.canonicalize().ok()
            });
            if !in_manifest {
                let rel = path
                    .strip_prefix(project_root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|_| path.clone());
                issues.push(ManifestIssue::OrphanSourceFile { path: rel });
            }
        }
    }

    issues
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_exactly_ten_rules() {
        assert_eq!(manifest().len(), KERNEL_V0_RULE_COUNT);
        assert_eq!(KERNEL_V0_RULE_COUNT, 10);
    }

    #[test]
    fn rule_names_are_distinct() {
        let names: std::collections::BTreeSet<_> =
            manifest().iter().map(|r| r.name.clone()).collect();
        assert_eq!(names.len(), manifest().len());
    }

    #[test]
    fn lemma_symbols_are_distinct() {
        let syms: std::collections::BTreeSet<_> =
            manifest().iter().map(|r| r.lemma_symbol.clone()).collect();
        assert_eq!(syms.len(), manifest().len());
    }

    #[test]
    fn lemma_symbols_match_rule_name_pattern() {
        // `k_<name>_sound` — the convention every rule file follows.
        // Drift in the convention surfaces as an audit failure.
        for rule in manifest() {
            assert!(
                rule.lemma_symbol.starts_with("k_"),
                "lemma symbol {:?} should start with `k_`",
                rule.lemma_symbol,
            );
            assert!(
                rule.lemma_symbol.ends_with("_sound"),
                "lemma symbol {:?} should end with `_sound`",
                rule.lemma_symbol,
            );
        }
    }

    #[test]
    fn file_paths_live_under_kernel_v0_rules() {
        for rule in manifest() {
            let path_str = rule.file_path.to_string_lossy();
            assert!(
                path_str.contains("verify/kernel_v0/rules/"),
                "file path {:?} should live under verify/kernel_v0/rules/",
                rule.file_path,
            );
            assert_eq!(
                rule.file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
                "vr",
                ".vr extension required",
            );
        }
    }

    #[test]
    fn proved_admitted_split_matches_readme() {
        // README "10 minimal rules" table: 4 proved, 6 admitted.
        // This split is the soundness-debt headline for #154 — when
        // it changes, the README + audit dashboards must change
        // together.
        assert_eq!(proved_count(), 4);
        assert_eq!(admitted_count(), 6);
        assert_eq!(proved_count() + admitted_count(), KERNEL_V0_RULE_COUNT);
    }

    #[test]
    fn proved_rules_have_no_iou_citation() {
        for rule in manifest() {
            if rule.status == KernelV0Status::Proved {
                assert!(
                    rule.iou_citation.is_empty(),
                    "Proved rule {:?} must not carry an IOU citation",
                    rule.name,
                );
            }
        }
    }

    #[test]
    fn admitted_rules_carry_iou_citation() {
        for rule in manifest() {
            if rule.status == KernelV0Status::Admitted {
                assert!(
                    !rule.iou_citation.is_empty(),
                    "Admitted rule {:?} must carry an IOU citation describing the missing structural lemma",
                    rule.name,
                );
            }
        }
    }

    #[test]
    fn status_serde_round_trip() {
        for status in [KernelV0Status::Proved, KernelV0Status::Admitted] {
            let json = serde_json::to_string(&status).unwrap();
            let restored: KernelV0Status = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, status);
        }
    }

    #[test]
    fn rule_serde_round_trip() {
        let rule = manifest().into_iter().next().unwrap();
        let json = serde_json::to_string(&rule).unwrap();
        let restored: KernelV0Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, rule);
    }

    #[test]
    fn proved_rules_match_canonical_set() {
        // The 4 structurally-proved rules. Mirrors the README's
        // "Proved" rows and `verum_kernel::proof_checker`'s
        // hand-audit set.
        let proved: std::collections::BTreeSet<_> = manifest()
            .iter()
            .filter(|r| r.status == KernelV0Status::Proved)
            .map(|r| r.name.clone())
            .collect();
        let expected: std::collections::BTreeSet<_> = ["K-Var", "K-Univ", "K-FwAx", "K-Pos"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(proved, expected);
    }

    #[test]
    fn verify_manifest_against_real_project_finds_no_missing_files() {
        // Locate the canonical Verum project root (`<workspace>/core`,
        // where `verum.toml` lives) and verify the manifest against
        // the live `verify/kernel_v0/` tree. This is the drift
        // gate: if a rule was deleted or moved without updating the
        // manifest, this test fails.
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // crate_dir = .../verum/crates/verum_kernel
        let workspace_root = crate_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root above crates/verum_kernel/");
        let project_root = workspace_root.join("core");
        let issues = verify_manifest(&project_root);
        let missing: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i, ManifestIssue::MissingSourceFile { .. }))
            .collect();
        assert!(
            missing.is_empty(),
            "Manifest references files that don't exist on disk: {:#?}",
            missing,
        );
    }
}
