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
            "metamath" | "mm" => Ok(Self::Metamath),
            other => Err(CliError::InvalidArgument(
                format!(
                    "unknown export format: `{}` (expected `dedukti`, \
                     `coq`, `lean`, or `metamath`)",
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
            Self::Metamath => "metamath",
        }
    }

    /// The file extension the emitted certificate uses.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Dedukti => "dk",
            Self::Coq => "v",
            Self::Lean => "lean",
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

fn emit_coq(decls: &[Declaration]) -> String {
    let mut out = String::new();
    out.push_str("(* Exported by `verum export --to coq`. *)\n");
    out.push_str("(* Statements only — proofs are admitted. Full proof-term replay *)\n");
    out.push_str("(* through verum_kernel lands with per-backend SMT reconstruction. *)\n\n");

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
    fn all_four_formats_parse_from_canonical_names() {
        assert_eq!(ExportFormat::parse("dedukti").unwrap(), ExportFormat::Dedukti);
        assert_eq!(ExportFormat::parse("coq").unwrap(), ExportFormat::Coq);
        assert_eq!(ExportFormat::parse("lean").unwrap(), ExportFormat::Lean);
        assert_eq!(
            ExportFormat::parse("metamath").unwrap(),
            ExportFormat::Metamath
        );
    }

    #[test]
    fn unknown_format_error_message_lists_all_four() {
        let err = ExportFormat::parse("isabelle").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("dedukti"));
        assert!(msg.contains("coq"));
        assert!(msg.contains("lean"));
        assert!(msg.contains("metamath"));
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
