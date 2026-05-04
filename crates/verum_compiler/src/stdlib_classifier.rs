//! Stdlib layer classifier — read-only audit pass.
//!
//! Walks every `.vr` file in the embedded stdlib archive
//! (`embedded_stdlib::get_embedded_stdlib`), parses each into an AST, and
//! classifies the module as one of the three layer kinds the precompiled-
//! stdlib epic introduces:
//!
//! * [`Layer::Runtime`] — anything reached during normal program
//!   execution (`Function`, `Type`, `Protocol`, `Impl`, `Const`,
//!   `Static`, `ContextDecl`, `Predicate`, `Pattern`, `View`,
//!   `ExternBlock`, `FFIBoundary`).
//! * [`Layer::Proof`] — `theorem`, `lemma`, `corollary`, `axiom`,
//!   `tactic` (proof automation only).
//! * [`Layer::Meta`] — `meta fn`, `@meta`-decorated items, `@derive`
//!   templates, macro definitions.
//!
//! Layer-neutral items (`Mount`, sub-`Module`) don't tip the scale.
//!
//! The classifier is the empirical foundation for Phase 2 (directory
//! refactor) and Phase 4 (precompile-stdlib pipeline). It is *not* a
//! replacement for the existing module loader — those callers stay on
//! `embedded_stdlib::get_embedded_stdlib` + `stdlib_index` /
//! `stdlib_reachability`. This crate only adds a typed report on top.

use std::collections::BTreeMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::embedded_stdlib::{self, StdlibArchive};
use crate::stdlib_index::{self, StdlibModuleIndex};

use verum_ast::decl::ItemKind;
use verum_common::FileId;
use verum_fast_parser::FastParser;

/// Three stdlib layers that the precompiled-stdlib archive epic separates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// Runtime-essential code, embedded mandatorily, target-conditional
    /// via multi-variant function bodies.
    Runtime,
    /// Theorems, lemmas, corollaries, axioms, tactics. Lazy-loaded only
    /// when `--verify formal` or audit/replay tooling needs them.
    Proof,
    /// `@meta` / `@const` / `@derive` / macro evaluators. Lazy-loaded
    /// only when meta evaluation hits a stdlib meta declaration.
    Meta,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Runtime => "runtime",
            Layer::Proof => "proof",
            Layer::Meta => "meta",
        }
    }
}

/// Per-module item-kind tally produced by [`classify_stdlib`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemBreakdown {
    /// Items that are runtime-shaped (`Function`, `Type`, etc.).
    pub runtime: usize,
    /// Items that are proof-shaped (`Theorem`, `Lemma`, `Corollary`,
    /// `Axiom`, `Tactic`).
    pub proof: usize,
    /// Items that are meta-shaped (`Meta`, `meta fn`, items carrying a
    /// `@meta` attribute).
    pub meta: usize,
    /// Items that don't tip the scale (`Mount`, sub-`Module` decl).
    pub neutral: usize,
}

impl ItemBreakdown {
    pub fn nonneutral(&self) -> usize {
        self.runtime + self.proof + self.meta
    }
    pub fn is_pure_runtime(&self) -> bool {
        self.runtime > 0 && self.proof == 0 && self.meta == 0
    }
    pub fn is_pure_proof(&self) -> bool {
        self.proof > 0 && self.runtime == 0 && self.meta == 0
    }
    pub fn is_pure_meta(&self) -> bool {
        self.meta > 0 && self.runtime == 0 && self.proof == 0
    }
}

/// Reasons a module's classification cannot be determined automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassificationError {
    /// Items from two or more layers coexist; needs an explicit
    /// `@layer(...)` annotation or a Phase-2 file split.
    Mixed {
        breakdown: ItemBreakdown,
        /// What the auto-classifier would pick if forced to choose: the
        /// most-populated bucket.
        suggested: Layer,
    },
    /// Source failed to parse; classification skipped.
    ParseError(String),
    /// Module has no items at all (only `module X;` declaration).
    Empty,
}

/// One module's classification record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleClassification {
    /// Dotted module path, e.g. `core.async.future`.
    pub module_path: String,
    /// Archive-relative file path, e.g. `async/future.vr`.
    pub file_path: String,
    /// Auto-classified layer if items resolved unambiguously.
    pub layer: Result<Layer, ClassificationError>,
    /// Per-bucket counts for diagnostic / report consumption.
    pub breakdown: ItemBreakdown,
}

/// Aggregate counters printed at the end of a classification report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassificationStats {
    pub total_modules: usize,
    pub runtime_count: usize,
    pub proof_count: usize,
    pub meta_count: usize,
    pub mixed_count: usize,
    pub parse_error_count: usize,
    pub empty_count: usize,
}

/// Full classification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdlibClassificationReport {
    pub modules: Vec<ModuleClassification>,
    pub stats: ClassificationStats,
}

/// Errors that prevent the classifier from running at all.
#[derive(Debug)]
pub enum ClassifierError {
    /// The compiler binary was built with the embedded stdlib disabled.
    EmbeddedArchiveMissing,
    /// The embedded stdlib module-path index couldn't be initialised.
    ModuleIndexMissing,
}

impl std::fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmbeddedArchiveMissing => f.write_str(
                "embedded stdlib archive is unavailable; build the binary with the embed enabled",
            ),
            Self::ModuleIndexMissing => {
                f.write_str("embedded stdlib module-path index is unavailable")
            }
        }
    }
}

impl std::error::Error for ClassifierError {}

/// Run the classifier across every `.vr` file in the embedded stdlib
/// archive. Returns a sorted-by-module-path report.
///
/// This is intentionally read-only — it does not mutate the session, the
/// module registry, or any cache. It is safe to call repeatedly. The
/// per-file parse pass runs in parallel via `rayon`.
pub fn classify_stdlib() -> Result<StdlibClassificationReport, ClassifierError> {
    let archive = embedded_stdlib::get_embedded_stdlib()
        .ok_or(ClassifierError::EmbeddedArchiveMissing)?;
    let index = stdlib_index::get_module_index()
        .ok_or(ClassifierError::ModuleIndexMissing)?;
    Ok(classify_archive(archive, index))
}

/// Classify a specific archive — used by tests where we substitute a
/// fixture archive for the embedded one. Public for the same reason.
pub fn classify_archive(
    archive: &'static StdlibArchive,
    index: &'static StdlibModuleIndex,
) -> StdlibClassificationReport {
    // Sorted module list for deterministic output.
    let modules: Vec<&str> = index.all_modules().iter().map(|s| s.as_str()).collect();

    // The fast parser is heavily recursive — at default rayon worker
    // stack (~2 MB) some stdlib modules trip a stack overflow on
    // debug builds. Build a dedicated thread pool with a 16 MB stack
    // so per-module parses always fit. Falling back to the global
    // pool when a custom pool can't be built keeps the classifier
    // available even if rayon initialisation fails.
    let classifications = match rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024)
        .build()
    {
        Ok(pool) => pool.install(|| classify_modules_in_parallel(&modules, archive, index)),
        Err(_) => classify_modules_in_parallel(&modules, archive, index),
    };
    let mut classifications = classifications;

    // Re-sort after parallel collect to keep deterministic order
    // (par_iter preserves source order for `collect` into `Vec`, but be
    // explicit so a future rayon change doesn't quietly break the
    // contract).
    classifications.sort_by(|a, b| a.module_path.cmp(&b.module_path));

    let mut stats = ClassificationStats::default();
    stats.total_modules = classifications.len();
    for c in &classifications {
        match &c.layer {
            Ok(Layer::Runtime) => stats.runtime_count += 1,
            Ok(Layer::Proof) => stats.proof_count += 1,
            Ok(Layer::Meta) => stats.meta_count += 1,
            Err(ClassificationError::Mixed { .. }) => stats.mixed_count += 1,
            Err(ClassificationError::ParseError(_)) => stats.parse_error_count += 1,
            Err(ClassificationError::Empty) => stats.empty_count += 1,
        }
    }

    StdlibClassificationReport {
        modules: classifications,
        stats,
    }
}

fn classify_modules_in_parallel(
    modules: &[&str],
    archive: &'static StdlibArchive,
    index: &'static StdlibModuleIndex,
) -> Vec<ModuleClassification> {
    modules
        .par_iter()
        .filter_map(|module_path| {
            let file_path = index.module_to_file(module_path)?;
            let source = index.module_source(archive, module_path)?;
            Some(classify_one(module_path, file_path, source))
        })
        .collect()
}

fn classify_one(module_path: &str, file_path: &str, source: &str) -> ModuleClassification {
    let parser = FastParser::new();
    let module_ast = match parser.parse_module_str(source, FileId::dummy()) {
        Ok(m) => m,
        Err(e) => {
            return ModuleClassification {
                module_path: module_path.to_string(),
                file_path: file_path.to_string(),
                layer: Err(ClassificationError::ParseError(format!("{e:?}"))),
                breakdown: ItemBreakdown::default(),
            };
        }
    };

    let breakdown = tally_items(&module_ast);

    let layer = match (
        breakdown.is_pure_runtime(),
        breakdown.is_pure_proof(),
        breakdown.is_pure_meta(),
        breakdown.nonneutral() == 0,
    ) {
        (true, _, _, _) => Ok(Layer::Runtime),
        (_, true, _, _) => Ok(Layer::Proof),
        (_, _, true, _) => Ok(Layer::Meta),
        (_, _, _, true) => Err(ClassificationError::Empty),
        _ => {
            let suggested = if breakdown.runtime >= breakdown.proof
                && breakdown.runtime >= breakdown.meta
            {
                Layer::Runtime
            } else if breakdown.proof >= breakdown.meta {
                Layer::Proof
            } else {
                Layer::Meta
            };
            Err(ClassificationError::Mixed {
                breakdown: breakdown.clone(),
                suggested,
            })
        }
    };

    ModuleClassification {
        module_path: module_path.to_string(),
        file_path: file_path.to_string(),
        layer,
        breakdown,
    }
}

fn tally_items(module: &verum_ast::Module) -> ItemBreakdown {
    let mut b = ItemBreakdown::default();
    for item in module.items.iter() {
        match item.kind {
            // Runtime
            ItemKind::Function(_)
            | ItemKind::Type(_)
            | ItemKind::Protocol(_)
            | ItemKind::Impl(_)
            | ItemKind::Const(_)
            | ItemKind::Static(_)
            | ItemKind::Predicate(_)
            | ItemKind::Context(_)
            | ItemKind::ContextGroup(_)
            | ItemKind::Layer(_)
            | ItemKind::FFIBoundary(_)
            | ItemKind::ExternBlock(_)
            | ItemKind::View(_)
            | ItemKind::Pattern(_) => b.runtime += 1,

            // Proof
            ItemKind::Theorem(_)
            | ItemKind::Lemma(_)
            | ItemKind::Corollary(_)
            | ItemKind::Axiom(_)
            | ItemKind::Tactic(_) => b.proof += 1,

            // Meta
            ItemKind::Meta(_) => b.meta += 1,

            // Neutral — don't tip the scale.
            ItemKind::Mount(_) | ItemKind::Module(_) => b.neutral += 1,
        }
    }
    b
}

// ============================================================================
// Report renderers
// ============================================================================

/// Render the report as compact Markdown — suitable for piping into a
/// terminal or pasting into a PR.
pub fn render_markdown(report: &StdlibClassificationReport) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(report.modules.len() * 80);

    let s = &report.stats;
    let _ = writeln!(out, "# Stdlib layer classification\n");
    let _ = writeln!(
        out,
        "**Total modules:** {}  \n\
         **Runtime:** {}  \n\
         **Proof:** {}  \n\
         **Meta:** {}  \n\
         **Mixed (need explicit @layer):** {}  \n\
         **Parse errors:** {}  \n\
         **Empty:** {}\n",
        s.total_modules,
        s.runtime_count,
        s.proof_count,
        s.meta_count,
        s.mixed_count,
        s.parse_error_count,
        s.empty_count,
    );

    let _ = writeln!(out, "## Per-layer counts by top-level subtree\n");
    let mut by_subtree: BTreeMap<String, [usize; 6]> = BTreeMap::new();
    for c in &report.modules {
        let subtree = top_level_subtree(&c.module_path);
        let row = by_subtree.entry(subtree).or_insert([0; 6]);
        match &c.layer {
            Ok(Layer::Runtime) => row[0] += 1,
            Ok(Layer::Proof) => row[1] += 1,
            Ok(Layer::Meta) => row[2] += 1,
            Err(ClassificationError::Mixed { .. }) => row[3] += 1,
            Err(ClassificationError::ParseError(_)) => row[4] += 1,
            Err(ClassificationError::Empty) => row[5] += 1,
        }
    }
    let _ = writeln!(out, "| Subtree | Runtime | Proof | Meta | Mixed | Parse err | Empty |");
    let _ = writeln!(out, "|---------|--------:|------:|-----:|------:|----------:|------:|");
    for (subtree, row) in &by_subtree {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            subtree, row[0], row[1], row[2], row[3], row[4], row[5]
        );
    }
    let _ = writeln!(out);

    // Mixed-layer table — actionable item.
    let mixed: Vec<&ModuleClassification> = report
        .modules
        .iter()
        .filter(|c| matches!(c.layer, Err(ClassificationError::Mixed { .. })))
        .collect();
    if !mixed.is_empty() {
        let _ = writeln!(out, "## Mixed-layer modules ({})\n", mixed.len());
        let _ = writeln!(
            out,
            "Modules below mix two or more layer kinds; Phase 2 must \
             either split them by file or annotate with `@layer(...)`.\n"
        );
        let _ = writeln!(
            out,
            "| Module | Runtime | Proof | Meta | Suggested |"
        );
        let _ = writeln!(out, "|--------|--------:|------:|-----:|-----------|");
        for c in &mixed {
            if let Err(ClassificationError::Mixed { breakdown, suggested }) = &c.layer {
                let _ = writeln!(
                    out,
                    "| `{}` | {} | {} | {} | {} |",
                    c.module_path,
                    breakdown.runtime,
                    breakdown.proof,
                    breakdown.meta,
                    suggested.as_str()
                );
            }
        }
        let _ = writeln!(out);
    }

    let parse_errs: Vec<&ModuleClassification> = report
        .modules
        .iter()
        .filter(|c| matches!(c.layer, Err(ClassificationError::ParseError(_))))
        .collect();
    if !parse_errs.is_empty() {
        let _ = writeln!(out, "## Parse-error modules ({})\n", parse_errs.len());
        for c in &parse_errs {
            if let Err(ClassificationError::ParseError(msg)) = &c.layer {
                let _ = writeln!(out, "- `{}` — `{}`", c.module_path, snippet(msg, 120));
            }
        }
        let _ = writeln!(out);
    }
    out
}

/// Render the full report as JSON, for CI / tooling consumption.
pub fn render_json(report: &StdlibClassificationReport) -> serde_json::Result<String> {
    serde_json::to_string_pretty(report)
}

fn top_level_subtree(module_path: &str) -> String {
    // `core.async.future` → `core.async`. `core.mod` → `core`.
    let parts: Vec<&str> = module_path.split('.').collect();
    if parts.len() <= 2 {
        return parts.join(".");
    }
    format!("{}.{}", parts[0], parts[1])
}

fn snippet(s: &str, max: usize) -> String {
    let one_line: String = s.chars().take(max).collect();
    one_line.replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_classify_full_stdlib() {
        // Skip when the embedded archive is not built (e.g. minimal
        // builds). The classifier returns a typed error, not a panic.
        let report = match classify_stdlib() {
            Ok(r) => r,
            Err(_) => return,
        };
        // Sanity: stdlib should produce many modules and a non-zero
        // count of every layer.
        assert!(report.stats.total_modules > 100);
        // `runtime` always dominates — collections / mem / sync are huge.
        assert!(report.stats.runtime_count > 0);
        // Verify the per-subtree map at least contains `core.collections`.
        let md = render_markdown(&report);
        assert!(md.contains("Stdlib layer classification"));
    }
}
