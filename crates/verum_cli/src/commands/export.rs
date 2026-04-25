// `verum export` — certificate and cross-prover exchange.
//
// Walks every `.vr` file in the current project, collects every
// theorem / lemma / axiom / corollary declaration (plus its
// `@framework(name, "citation")` attribution if present), and emits
// a per-format file containing one entry per declaration.
//
// Supported formats:
//   dedukti   neutral exchange format — `<name> : <ty>.` statements.
//   coq       `Axiom` / `Theorem ... Admitted.` scaffolds.
//   lean      `axiom` / `theorem ... := sorry` scaffolds.
//
// The current MVP emits each declaration's STATEMENT — proof bodies
// are admitted (`Admitted.` / `sorry` / `(;;)` depending on target).
// The statement carries the `@framework` citation as an inline
// comment so external reviewers see the provenance without needing
// the separate audit report.
//
// Full proof-term export (re-derivation through `verum_kernel` and
// serialisation of the resulting CoreTerm) is a follow-up — it
// requires the SMT proof-replay path which lands per-backend.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use verum_ast::attr::FrameworkAttr;
use verum_ast::decl::ItemKind;
use verum_ast::Item;
use verum_common::{List, Maybe, Text};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use verum_compiler::CompilerOptions;

/// The target format a user can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Dedukti,
    Coq,
    Lean,
    /// Agda — statement-only `postulate` scaffold in `.agda` format.
    /// Each Verum declaration emits as a single-entry postulate block
    /// so per-declaration framework citations stay attached. Mirrors
    /// the admitted-proof semantics of Coq / Lean / Dedukti / Metamath.
    Agda,
    /// Metamath — statement-only axiom scaffold in `.mm` format.
    /// Proof bodies are left as proof-step placeholders
    /// (`$= ? $.`), mirroring the admitted-proof semantics of the
    /// Coq / Lean / Dedukti targets.
    Metamath,
}

impl ExportFormat {
    /// Parse the `--to <format>` CLI argument.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "dedukti" | "dk" => Ok(Self::Dedukti),
            "coq" | "v" => Ok(Self::Coq),
            "lean" | "lean4" => Ok(Self::Lean),
            "agda" => Ok(Self::Agda),
            "metamath" | "mm" => Ok(Self::Metamath),
            other => Err(CliError::InvalidArgument(
                format!(
                    "unknown export format: `{}` (expected `dedukti`, \
                     `coq`, `lean`, `agda`, or `metamath`)",
                    other
                )
                .into(),
            )),
        }
    }

    /// Canonical short name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dedukti => "dedukti",
            Self::Coq => "coq",
            Self::Lean => "lean",
            Self::Agda => "agda",
            Self::Metamath => "metamath",
        }
    }

    /// The file extension the emitted certificate uses.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Dedukti => "dk",
            Self::Coq => "v",
            Self::Lean => "lean",
            Self::Agda => "agda",
            Self::Metamath => "mm",
        }
    }
}

/// A single declaration collected from the project's AST.
#[derive(Debug, Clone)]
struct Declaration {
    /// Keyword that introduced it — `axiom`, `theorem`, `lemma`, `corollary`.
    kind: &'static str,
    /// Declared name.
    name: Text,
    /// Relative path of the source file it came from.
    source: PathBuf,
    /// Framework attribution if the declaration carries
    /// `@framework(name, "citation")`.
    framework: Maybe<FrameworkAttr>,
}

/// Options for the `verum export` command.
pub struct ExportOptions {
    pub format: ExportFormat,
    pub output: Maybe<PathBuf>,
}

/// Entry point for `verum export --to <format> [--output <path>]`.
pub fn run(options: ExportOptions) -> Result<()> {
    ui::step(&format!(
        "Exporting to {} certificate",
        options.format.as_str()
    ));

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("no .vr files found under the current project");
        return Ok(());
    }

    let mut declarations = Vec::new();
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_export(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };

        for item in &module.items {
            if let Some(decl) = collect_declaration(item, &rel_path) {
                declarations.push(decl);
            }
        }
    }

    let body = match options.format {
        ExportFormat::Dedukti => emit_dedukti(&declarations),
        ExportFormat::Coq => emit_coq(&declarations),
        ExportFormat::Lean => emit_lean(&declarations),
        ExportFormat::Agda => emit_agda(&declarations),
        ExportFormat::Metamath => emit_metamath(&declarations),
    };

    let output_path = match options.output {
        Maybe::Some(p) => p,
        Maybe::None => default_output_path(&manifest_dir, options.format),
    };

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CliError::Custom(
                    format!("creating output directory {}: {}", parent.display(), e)
                        .into(),
                )
            })?;
        }
    }

    std::fs::write(&output_path, &body).map_err(|e| {
        CliError::Custom(
            format!("writing certificate to {}: {}", output_path.display(), e)
                .into(),
        )
    })?;

    print_summary(
        options.format,
        &declarations,
        &output_path,
        skipped_files,
    );

    Ok(())
}

// -----------------------------------------------------------------------------
// Walking the project AST
// -----------------------------------------------------------------------------

fn discover_vr_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "target" && name != "node_modules"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_file()
            && entry.path().extension().map_or(false, |e| e == "vr")
        {
            out.push(entry.into_path());
        }
    }
    out
}

fn parse_file_for_export(path: &Path) -> std::result::Result<verum_ast::Module, String> {
    let mut options = CompilerOptions::default();
    options.input = path.to_path_buf();
    let mut session = Session::new(options);
    let file_id = session
        .load_file(path)
        .map_err(|e| format!("load: {}", e))?;
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .phase_parse(file_id)
        .map_err(|e| format!("parse: {}", e))
}

fn collect_declaration(item: &Item, rel_path: &Path) -> Option<Declaration> {
    let (kind, name, decl_attrs) = match &item.kind {
        ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
        ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
        ItemKind::Corollary(decl) => {
            ("corollary", decl.name.name.clone(), &decl.attributes)
        }
        ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
        _ => return None,
    };

    // Framework attribution: check both the item's outer attributes
    // and the inner decl.attributes — the parser can place the
    // marker on either.
    let framework = first_framework(&item.attributes).or_else(|| first_framework(decl_attrs));

    Some(Declaration {
        kind,
        name,
        source: rel_path.to_path_buf(),
        framework,
    })
}

fn first_framework(attrs: &List<verum_ast::attr::Attribute>) -> Maybe<FrameworkAttr> {
    for attr in attrs.iter() {
        if !attr.is_named("framework") {
            continue;
        }
        if let Maybe::Some(fw) = FrameworkAttr::from_attribute(attr) {
            return Maybe::Some(fw);
        }
    }
    Maybe::None
}

// -----------------------------------------------------------------------------
// Framework-lineage → target-library mapping (VUVA §8.5)
//
// When a `@framework(<lineage>, "...")` marker has a known mapping in a
// target ecosystem, the exporter emits the corresponding `import` /
// `Require` / dependency stanza so the resulting file is ready to check
// against the target assistant's standard library. Unmapped lineages
// fall through to the plain-axiom scaffolding (current MVP behaviour)
// and are flagged in the output as comments so reviewers see the
// missing hook.
//
// The table is intentionally small and curated — stdlib "standard
// six-pack" per VUVA §6.2 plus a handful of widely-cited foundations.
// User-authored packages extend this by shipping a `@lineage_map`
// attribute on their `@framework` declarations (Phase 3 work).
// -----------------------------------------------------------------------------

/// Dependency stanza for a specific exporter target.
#[derive(Debug, Clone, Copy)]
struct LineageImport {
    lean: Option<&'static str>,
    coq: Option<&'static str>,
    dedukti: Option<&'static str>,
    metamath: Option<&'static str>,
    agda: Option<&'static str>,
}

/// Curated map from Verum framework lineage slug → target-ecosystem
/// dependency stanza. Ordering: lineage slug alphabetical for
/// deterministic output.
const LINEAGE_IMPORTS: &[(&str, LineageImport)] = &[
    (
        "arnold_mather",
        LineageImport {
            // Not yet mapped to any mainstream library; emitted as
            // plain axiom with citation comment.
            lean: None,
            coq: None,
            dedukti: None,
            metamath: None,
            agda: None,
        },
    ),
    (
        "baez_dolan",
        LineageImport {
            lean: Some("import Mathlib.CategoryTheory.Monoidal.Category"),
            coq: Some("Require Import Category.Category.CategoryTheory."),
            dedukti: None,
            metamath: None,
            agda: Some("open import Categories.Category"),
        },
    ),
    (
        "connes_reconstruction",
        LineageImport {
            lean: Some("import Mathlib.Analysis.NormedSpace.OperatorNorm"),
            coq: None,
            dedukti: None,
            metamath: None,
            agda: None,
        },
    ),
    (
        "lurie_htt",
        LineageImport {
            lean: Some("import Mathlib.CategoryTheory.Category.Basic"),
            coq: Some("Require Import Category.Theory.Category."),
            dedukti: None,
            metamath: None,
            agda: Some("open import Categories.Category"),
        },
    ),
    (
        "petz_classification",
        LineageImport {
            lean: Some("import Mathlib.Analysis.InnerProductSpace.Basic"),
            coq: None,
            dedukti: None,
            metamath: None,
            agda: None,
        },
    ),
    (
        "schreiber_dcct",
        LineageImport {
            lean: Some("import Mathlib.CategoryTheory.Sites.Sheaf"),
            coq: None,
            dedukti: None,
            metamath: None,
            agda: None,
        },
    ),
    (
        "univalence",
        LineageImport {
            lean: Some("import Mathlib.Logic.Equiv.Defs"),
            coq: Some("Require Import HoTT.Univalence."),
            dedukti: None,
            metamath: None,
            agda: Some("open import Cubical.Foundations.Univalence"),
        },
    ),
];

/// Look up a lineage's target-library dependency stanza, if any.
fn lineage_import(lineage: &str) -> Option<&'static LineageImport> {
    LINEAGE_IMPORTS
        .iter()
        .find(|(slug, _)| *slug == lineage)
        .map(|(_, li)| li)
}

/// Collect every distinct framework lineage appearing in `decls` —
/// used to emit the per-file import header.
fn distinct_lineages(decls: &[Declaration]) -> Vec<Text> {
    let mut seen = std::collections::BTreeSet::new();
    for d in decls {
        if let Maybe::Some(fw) = &d.framework {
            seen.insert(fw.name.clone());
        }
    }
    seen.into_iter().collect()
}

// -----------------------------------------------------------------------------
// Dedukti emitter
// -----------------------------------------------------------------------------

fn emit_dedukti(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("(; Exported by `verum export --to dedukti`. ;)\n");
    out.push_str("(; One entry per top-level axiom / theorem / lemma / corollary. ;)\n");
    out.push_str(
        "(; Types are currently opaque — statement surface only. Proof-term\n",
    );
    out.push_str(
        "   re-derivation through verum_kernel lands with per-backend SMT replay. ;)\n\n",
    );
    out.push_str("Prop : Type.\n\n");

    let by_framework = group_by_framework(decls);
    for (framework_key, group) in &by_framework {
        if let Some(fw) = framework_key {
            out.push_str(&format!(
                "(; ---- framework: {} ---- ;)\n",
                fw.as_str()
            ));
        } else {
            out.push_str("(; ---- no framework attribution ---- ;)\n");
        }
        for d in group {
            if let Maybe::Some(fw) = &d.framework {
                out.push_str(&format!(
                    "(; {} — {} — {} :: {} ;)\n",
                    d.kind,
                    fw.name.as_str(),
                    fw.citation.as_str(),
                    d.source.display(),
                ));
            } else {
                out.push_str(&format!(
                    "(; {} :: {} ;)\n",
                    d.kind,
                    d.source.display(),
                ));
            }
            out.push_str(&format!("{} : Prop.\n\n", mangle(&d.name)));
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Coq emitter
// -----------------------------------------------------------------------------

fn emit_coq_imports(decls: &[Declaration], out: &mut String) {
    let lineages = distinct_lineages(decls);
    if lineages.is_empty() {
        return;
    }
    let mut import_lines: Vec<&'static str> = Vec::new();
    let mut unmapped: Vec<&str> = Vec::new();
    for l in &lineages {
        match lineage_import(l.as_str()).and_then(|li| li.coq) {
            Some(stanza) if !import_lines.contains(&stanza) => {
                import_lines.push(stanza)
            }
            Some(_) => {}
            None => unmapped.push(l.as_str()),
        }
    }
    for line in &import_lines {
        out.push_str(line);
        out.push('\n');
    }
    if !import_lines.is_empty() {
        out.push('\n');
    }
    for u in &unmapped {
        out.push_str(&format!(
            "(* note: framework lineage `{u}` has no Coq-library mapping \
             yet; emitted as opaque axiom. *)\n"
        ));
    }
    if !unmapped.is_empty() {
        out.push('\n');
    }
}

fn emit_coq(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("(* Exported by `verum export --to coq`. *)\n");
    out.push_str("(* Statements only — proofs are admitted. Full proof-term replay *)\n");
    out.push_str("(* through verum_kernel lands with per-backend SMT reconstruction. *)\n\n");

    // VUVA §8.5 framework-lineage → Coq-library mapping.
    emit_coq_imports(decls, &mut out);

    let by_framework = group_by_framework(decls);
    for (framework_key, group) in &by_framework {
        if let Some(fw) = framework_key {
            out.push_str(&format!(
                "(* ==== framework: {} ==== *)\n",
                fw.as_str()
            ));
        } else {
            out.push_str("(* ==== no framework attribution ==== *)\n");
        }
        for d in group {
            if let Maybe::Some(fw) = &d.framework {
                out.push_str(&format!(
                    "(* {} — {} — {} :: {} *)\n",
                    d.kind,
                    fw.name.as_str(),
                    fw.citation.as_str(),
                    d.source.display(),
                ));
            }
            match d.kind {
                "axiom" => {
                    out.push_str(&format!(
                        "Axiom {} : Prop.\n\n",
                        mangle(&d.name)
                    ));
                }
                _ => {
                    out.push_str(&format!(
                        "Theorem {} : Prop.\nProof. Admitted.\n\n",
                        mangle(&d.name)
                    ));
                }
            }
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Lean emitter
// -----------------------------------------------------------------------------

/// Emit a Metamath (.mm) certificate.
///
/// Metamath is "axiom system + proof step language". Statements are
/// declared with `$a` (axiom) / `$p ... $.` (provable), and every
/// statement must name its free variables in a `$v` line and provide
/// a constant-typing header. For a Verum export that carries only
/// statements (proofs admitted), we emit:
///
///   $c wff |- $.               — constant declarations
///   $v x y z $.                — placeholder variables
///   ax-<name> $a wff <stmt> $. — for axioms
///   th-<name> $p wff <stmt> $= ? $.   — for theorems (proof placeholder `?`)
///
/// Framework citations ride along as `$( comment $)` blocks so the
/// trusted boundary is visible. The `?` proof-step token is
/// Metamath's own placeholder for "proof not yet supplied" — tools
/// like `mmverify.py` accept it as an unchecked scaffold. This
/// mirrors the admitted-proof semantics of the Coq / Lean / Dedukti
/// emitters: the statement is authoritative, the proof step is a
/// follow-up that per-backend SMT replay will fill in.
fn emit_metamath(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("$( Exported by `verum export --to metamath`. $)\n");
    out.push_str(
        "$( Statements only — proofs are `?` placeholders. Full proof-term\n\
         replay through verum_kernel lands with per-backend SMT\n\
         reconstruction. $)\n\n",
    );

    // Constant + variable declarations. Kept minimal — Verum's
    // richer dependent statements lower to `wff` here; a faithful
    // type-preserving encoding is a follow-up that needs a Metamath
    // metatheory shared across the whole project.
    out.push_str("$c wff |- $.\n");
    out.push_str("$v x y z $.\n\n");

    let by_framework = group_by_framework(decls);
    for (framework_key, group) in &by_framework {
        if let Some(fw) = framework_key {
            out.push_str(&format!("$( ==== framework: {} ==== $)\n", fw.as_str()));
        } else {
            out.push_str("$( ==== no framework attribution ==== $)\n");
        }
        for d in group {
            if let Maybe::Some(fw) = &d.framework {
                out.push_str(&format!(
                    "$( {} — {} — {} :: {} $)\n",
                    d.kind,
                    fw.name.as_str(),
                    fw.citation.as_str(),
                    d.source.display(),
                ));
            }
            match d.kind {
                "axiom" => {
                    out.push_str(&format!(
                        "ax-{} $a wff {} $.\n\n",
                        mangle(&d.name),
                        mangle(&d.name)
                    ));
                }
                _ => {
                    out.push_str(&format!(
                        "th-{} $p wff {} $= ? $.\n\n",
                        mangle(&d.name),
                        mangle(&d.name)
                    ));
                }
            }
        }
    }
    out
}

fn emit_lean(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("-- Exported by `verum export --to lean`.\n");
    out.push_str("-- Statements only — proofs are `sorry`. Full proof-term replay\n");
    out.push_str("-- through verum_kernel lands with per-backend SMT reconstruction.\n\n");

    // VUVA §8.5: emit `import` stanzas for known framework-lineage
    // mappings so the file is ready to check against Mathlib without
    // manual editing. Unmapped lineages fall through with a comment.
    let lineages = distinct_lineages(decls);
    if !lineages.is_empty() {
        let mut import_lines: Vec<&'static str> = Vec::new();
        let mut unmapped: Vec<&str> = Vec::new();
        for l in &lineages {
            match lineage_import(l.as_str()).and_then(|li| li.lean) {
                Some(stanza) if !import_lines.contains(&stanza) => {
                    import_lines.push(stanza)
                }
                Some(_) => {}
                None => unmapped.push(l.as_str()),
            }
        }
        for line in &import_lines {
            out.push_str(line);
            out.push('\n');
        }
        if !import_lines.is_empty() {
            out.push('\n');
        }
        for u in &unmapped {
            out.push_str(&format!(
                "-- note: framework lineage `{u}` has no Lean-library \
                 mapping yet; emitted as opaque axiom.\n"
            ));
        }
        if !unmapped.is_empty() {
            out.push('\n');
        }
    }

    let by_framework = group_by_framework(decls);
    for (framework_key, group) in &by_framework {
        if let Some(fw) = framework_key {
            out.push_str(&format!(
                "-- ==== framework: {} ====\n",
                fw.as_str()
            ));
        } else {
            out.push_str("-- ==== no framework attribution ====\n");
        }
        for d in group {
            if let Maybe::Some(fw) = &d.framework {
                out.push_str(&format!(
                    "-- {} — {} — {} :: {}\n",
                    d.kind,
                    fw.name.as_str(),
                    fw.citation.as_str(),
                    d.source.display(),
                ));
            }
            match d.kind {
                "axiom" => {
                    out.push_str(&format!("axiom {} : Prop\n\n", mangle(&d.name)));
                }
                _ => {
                    out.push_str(&format!(
                        "theorem {} : Prop := sorry\n\n",
                        mangle(&d.name)
                    ));
                }
            }
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Agda emitter
// -----------------------------------------------------------------------------

/// Mangle a Verum identifier into an Agda-safe one. Agda accepts most
/// Unicode in identifiers, but mixed dashes and dots in pathnames are
/// not legal — Verum stdlib names already fit, so the MVP is a pass-
/// through that callers can tighten later.
fn agda_mangle(name: &Text) -> String {
    name.as_str().to_string()
}

/// Emit an Agda (.agda) certificate.
///
/// Each declaration becomes a single-entry `postulate` block:
///
///   -- theorem — lurie_htt — Lurie 2009 :: src/foo.vr
///   postulate
///     yoneda_full : Set
///
/// The postulate-per-decl shape (rather than one big block) keeps each
/// declaration's framework citation directly above its statement, which
/// matches the per-decl comment placement of the Coq / Lean / Dedukti
/// emitters and makes per-declaration grep / diff tractable. Agda
/// accepts arbitrarily many `postulate` blocks per module.
///
/// As with the other backends, statements are opaque (`: Set`) at the
/// MVP level. Type-preserving export through verum_kernel lands when
/// per-backend SMT proof-replay is wired in.
fn emit_agda(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("-- Exported by `verum export --to agda`.\n");
    out.push_str("-- Statements only — proofs are postulated. Full proof-term replay\n");
    out.push_str("-- through verum_kernel lands with per-backend SMT reconstruction.\n\n");
    out.push_str("module Verum.Export where\n\n");

    // VUVA §8.5 framework-lineage → Agda-library mapping. Unknown
    // lineages fall through with a comment so reviewers see the gap.
    let lineages = distinct_lineages(decls);
    if !lineages.is_empty() {
        let mut import_lines: Vec<&'static str> = Vec::new();
        let mut unmapped: Vec<&str> = Vec::new();
        for l in &lineages {
            match lineage_import(l.as_str()).and_then(|li| li.agda) {
                Some(stanza) if !import_lines.contains(&stanza) => {
                    import_lines.push(stanza)
                }
                Some(_) => {}
                None => unmapped.push(l.as_str()),
            }
        }
        for line in &import_lines {
            out.push_str(line);
            out.push('\n');
        }
        if !import_lines.is_empty() {
            out.push('\n');
        }
        for u in &unmapped {
            out.push_str(&format!(
                "-- note: framework lineage `{u}` has no Agda-library \
                 mapping yet; emitted as opaque postulate.\n"
            ));
        }
        if !unmapped.is_empty() {
            out.push('\n');
        }
    }

    let by_framework = group_by_framework(decls);
    for (framework_key, group) in &by_framework {
        if let Some(fw) = framework_key {
            out.push_str(&format!(
                "-- ==== framework: {} ====\n",
                fw.as_str()
            ));
        } else {
            out.push_str("-- ==== no framework attribution ====\n");
        }
        for d in group {
            if let Maybe::Some(fw) = &d.framework {
                out.push_str(&format!(
                    "-- {} — {} — {} :: {}\n",
                    d.kind,
                    fw.name.as_str(),
                    fw.citation.as_str(),
                    d.source.display(),
                ));
            }
            // axiom and theorem both render as a postulate block:
            // proofs are admitted at the MVP level so the rendering
            // is uniform. The `kind` is preserved in the comment line
            // above the postulate so reviewers see the original intent.
            out.push_str("postulate\n");
            out.push_str(&format!("  {} : Set\n\n", agda_mangle(&d.name)));
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Summary + helpers
// -----------------------------------------------------------------------------

fn group_by_framework(
    decls: &[Declaration],
) -> BTreeMap<Option<Text>, Vec<&Declaration>> {
    let mut map: BTreeMap<Option<Text>, Vec<&Declaration>> = BTreeMap::new();
    for d in decls {
        let key = match &d.framework {
            Maybe::Some(fw) => Some(fw.name.clone()),
            Maybe::None => None,
        };
        map.entry(key).or_default().push(d);
    }
    map
}

fn mangle(name: &Text) -> String {
    // Dedukti/Coq/Lean identifiers are ASCII-latin plus digits and
    // underscore. Stdlib names already fit this shape; nothing more
    // is needed at the MVP level.
    name.as_str().to_string()
}

fn default_output_path(manifest_dir: &Path, format: ExportFormat) -> PathBuf {
    let mut path = manifest_dir.join("certificates");
    path.push(format.as_str());
    std::fs::create_dir_all(&path).ok();
    let mut file = path.clone();
    file.push(format!("export.{}", format.extension()));
    file
}

fn print_summary(
    format: ExportFormat,
    decls: &[Declaration],
    output_path: &Path,
    skipped_files: usize,
) {
    println!();
    println!(
        "{}",
        format!(
            "Exported {} declaration(s) to {} ({})",
            decls.len(),
            output_path.display(),
            format.as_str(),
        )
        .bold()
    );

    let framework_count = decls
        .iter()
        .filter(|d| matches!(d.framework, Maybe::Some(_)))
        .count();

    if framework_count > 0 {
        println!(
            "  {} framework-axiom marker(s) carried into the certificate",
            framework_count.to_string().cyan()
        );
    }

    if skipped_files > 0 {
        println!(
            "  {} .vr file(s) skipped (parse errors)",
            skipped_files.to_string().yellow()
        );
    }

    println!();
    println!(
        "{} Full proof-term replay through verum_kernel lands with per-",
        "note:".dimmed()
    );
    println!(
        "      backend SMT reconstruction. This certificate carries"
    );
    println!("      statements + framework citations — proofs are admitted.");
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn metamath_parses_from_mm_and_metamath() {
        assert_eq!(
            ExportFormat::parse("metamath").unwrap(),
            ExportFormat::Metamath
        );
        assert_eq!(ExportFormat::parse("mm").unwrap(), ExportFormat::Metamath);
    }

    #[test]
    fn metamath_extension_is_mm() {
        assert_eq!(ExportFormat::Metamath.extension(), "mm");
        assert_eq!(ExportFormat::Metamath.as_str(), "metamath");
    }

    #[test]
    fn all_five_formats_parse_from_canonical_names() {
        assert_eq!(ExportFormat::parse("dedukti").unwrap(), ExportFormat::Dedukti);
        assert_eq!(ExportFormat::parse("coq").unwrap(), ExportFormat::Coq);
        assert_eq!(ExportFormat::parse("lean").unwrap(), ExportFormat::Lean);
        assert_eq!(ExportFormat::parse("agda").unwrap(), ExportFormat::Agda);
        assert_eq!(
            ExportFormat::parse("metamath").unwrap(),
            ExportFormat::Metamath
        );
    }

    #[test]
    fn unknown_format_error_message_lists_all_five() {
        let err = ExportFormat::parse("isabelle").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("dedukti"));
        assert!(msg.contains("coq"));
        assert!(msg.contains("lean"));
        assert!(msg.contains("agda"));
        assert!(msg.contains("metamath"));
    }

    #[test]
    fn agda_extension_and_canonical_name() {
        assert_eq!(ExportFormat::Agda.extension(), "agda");
        assert_eq!(ExportFormat::Agda.as_str(), "agda");
    }

    #[test]
    fn agda_emitter_produces_module_header_and_postulate() {
        let decls = vec![Declaration {
            kind: "theorem",
            name: Text::from("yoneda_full"),
            source: PathBuf::from("src/lib.vr"),
            framework: Maybe::None,
        }];
        let out = emit_agda(&decls);
        // Agda module declaration is mandatory for `agda --type-check`.
        assert!(out.contains("module Verum.Export where"));
        // Each declaration must appear as a postulate of type Set.
        assert!(out.contains("postulate"));
        assert!(out.contains("yoneda_full : Set"));
        // The `verum export --to agda` provenance comment must ride along.
        assert!(out.contains("`verum export --to agda`"));
    }

    #[test]
    fn metamath_emitter_produces_valid_preamble() {
        let decls: Vec<Declaration> = Vec::new();
        let out = emit_metamath(&decls);
        // Metamath verifiers require the constant/variable
        // declarations; the preamble must always be emitted.
        assert!(out.contains("$c wff |- $."));
        assert!(out.contains("$v x y z $."));
        assert!(out.contains("`verum export --to metamath`"));
    }
}
